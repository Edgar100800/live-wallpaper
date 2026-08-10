import AppKit
import MetalKit
import QuartzCore
import WebKit

protocol WallpaperRenderer: AnyObject {
    var kind: WallpaperRendererKind { get }
    var view: NSView { get }
    var controlDescriptors: [WallpaperControlDescriptor] { get }
    var diagnosticSummary: String { get }

    func updateFrame(_ frame: CGRect)
    func setPlayback(paused: Bool, fpsCap: Int, adaptiveQuality: Bool)
    func applyControlValues(_ values: [String: Double])
    func captureSnapshot(targetPixelSize: CGSize, completion: @escaping (NSImage?) -> Void)
    func tearDown()
}

final class WebWallpaperRenderer: WallpaperRenderer {
    let kind: WallpaperRendererKind = .web
    let webView: WKWebView
    var view: NSView { webView }
    var controlDescriptors: [WallpaperControlDescriptor] { [] }
    var diagnosticSummary: String { "renderer:web" }

    init(frame: CGRect) {
        webView = WebViewFactory.makeWebView(frame: frame)
    }

    func updateFrame(_ frame: CGRect) {
        webView.frame = frame
    }

    func setPlayback(paused: Bool, fpsCap: Int, adaptiveQuality: Bool) {
        let js = """
        window.__pwPaused = \(paused ? "true" : "false");
        window.__pwFPSCap = \(fpsCap);
        window.__pwAdaptiveEnabled = \(adaptiveQuality ? "true" : "false");
        if (!window.__pwAdaptiveEnabled) {
          window.__pwAdaptiveCap = 0;
          window.__pwQualityLevel = 1;
        }
        """
        webView.evaluateJavaScript(js, completionHandler: nil)
    }

    func applyControlValues(_ values: [String: Double]) {}

    func captureSnapshot(targetPixelSize: CGSize, completion: @escaping (NSImage?) -> Void) {
        let configuration = WKSnapshotConfiguration()
        configuration.rect = webView.bounds
        if targetPixelSize.width > 0 {
            configuration.snapshotWidth = NSNumber(value: Double(targetPixelSize.width))
        }
        configuration.afterScreenUpdates = false
        webView.takeSnapshot(with: configuration) { image, error in
            if let error {
                NSLog("ParticleWall: Web snapshot failed: \(error)")
            }
            completion(image)
        }
    }

    func tearDown() {
        webView.stopLoading()
        webView.navigationDelegate = nil
        webView.removeFromSuperview()
    }
}

/// Native renderer for the bundled particle demo. Particle positions are
/// generated procedurally in the vertex shader, so each frame only sends a
/// small uniform block instead of updating thousands of values from the CPU.
final class MetalParticleRenderer: NSObject, WallpaperRenderer, MTKViewDelegate {
    let kind: WallpaperRendererKind
    let metalView: MTKView
    var view: NSView { metalView }

    private let commandQueue: MTLCommandQueue
    private let pipeline: MTLRenderPipelineState
    private let flowPipeline: MTLComputePipelineState
    private let graphPipeline: MTLRenderPipelineState
    private let graphPositionPipeline: MTLComputePipelineState
    private let graphConnectionPipeline: MTLComputePipelineState
    private let flowParticles: MTLBuffer
    private let flowHistory: MTLBuffer
    private let graphPositions: MTLBuffer
    private let graphEdges: MTLBuffer
    private let startedAt = CACurrentMediaTime()
    private var frameCount: UInt64 = 0
    private var animationTime: Float = 0
    private var lastDrawTime: CFTimeInterval?
    private var speed: Float = 1
    private var scale: Float = 1
    private var rotation = SIMD3<Float>(repeating: 0)
    private var position = SIMD3<Float>(repeating: 0)
    private var particleSize: Float = 1.6
    private var brightness: Float = 1.5
    private var fitToScreen = false
    private var horizontalLimit: Float = 1
    private var verticalLimit: Float = 1
    private var particleColor = SIMD3<Float>(232 / 255, 1, 1)
    private var backgroundColor = SIMD3<Float>(3 / 255, 6 / 255, 9 / 255)
    private var flowFrame: UInt32 = 0
    private var flowStepAccumulator: Float = 1
    private var graphEnabled = false
    private var graphDistance: Float = 0.085
    private var graphOpacity: Float = 0.55
    private var graphConnections: Float = 2
    private var pendingSnapshots: [(NSImage?) -> Void] = []

    private static let flowParticleCount = 720 * 9
    private static let flowHistoryCount = 12
    private static let primeLimit = 999_999
    private static let primeCount = 78_498
    private static let torusPointCount = 512 + 64 * 96 * 6
    private static let chromaticRingPointCount = 6_225
    private static let chromaticRingTrailCount = 8
    private static let graphNodeCount = 768
    private static let graphMaxConnections = 3

    struct Uniforms {
        var time: Float
        var aspect: Float
        var pointSize: Float
        var scale: Float
        var rotation: SIMD3<Float>
        var position: SIMD3<Float>
        var appearance: SIMD4<Float>
        var model: SIMD4<Float>
        var screenFit: SIMD4<Float>
        var graph: SIMD4<Float>
    }

    static let descriptors: [WallpaperControlDescriptor] = [
        .init(id: "positionX", label: "Posición X", category: "Transformación",
              min: -3, max: 3, step: 0.05, defaultValue: 0),
        .init(id: "positionY", label: "Posición Y", category: "Transformación",
              min: -3, max: 3, step: 0.05, defaultValue: 0),
        .init(id: "positionZ", label: "Posición Z", category: "Transformación",
              min: -3, max: 3, step: 0.05, defaultValue: 0),
        .init(id: "rotationX", label: "Rotación X", category: "Transformación",
              min: -60, max: 60, step: 1, defaultValue: 0),
        .init(id: "rotationY", label: "Perspectiva Y", category: "Transformación",
              min: -60, max: 60, step: 1, defaultValue: 0),
        .init(id: "rotationZ", label: "Rotación", category: "Transformación",
              min: -180, max: 180, step: 1, defaultValue: 0),
        .init(id: "scale", label: "Escala", category: "Transformación",
              min: 0.2, max: 2.5, step: 0.05, defaultValue: 1),
        .init(id: "speed", label: "Velocidad", category: "Animación",
              min: 0, max: 3, step: 0.05, defaultValue: 1),
        .init(id: "fitToScreen", label: "Adaptar al tamaño de pantalla", category: "Pantalla",
              min: 0, max: 1, step: 1, defaultValue: 0, kind: .boolean),
        .init(id: "horizontalLimit", label: "Límite horizontal", category: "Pantalla",
              min: 0.25, max: 2.5, step: 0.05, defaultValue: 1),
        .init(id: "verticalLimit", label: "Límite vertical", category: "Pantalla",
              min: 0.25, max: 2.5, step: 0.05, defaultValue: 1),
        .init(id: "backgroundColor", label: "Color del fondo", category: "Apariencia",
              min: 0, max: 0xFF_FF_FF, step: 1, defaultValue: 0x03_06_09,
              kind: .color),
        .init(id: "particleColor", label: "Color de partículas", category: "Apariencia",
              min: 0, max: 0xFF_FF_FF, step: 1, defaultValue: 0xE8_FF_FF,
              kind: .color),
        .init(id: "particleSize", label: "Tamaño", category: "Apariencia",
              min: 0.5, max: 4, step: 0.05, defaultValue: 1.6),
        .init(id: "brightness", label: "Intensidad de puntos", category: "Apariencia",
              min: 0.25, max: 10, step: 0.05, defaultValue: 1.5),
        .init(id: "graphEnabled", label: "Conectar puntos cercanos", category: "Grafo",
              min: 0, max: 1, step: 1, defaultValue: 0, kind: .boolean),
        .init(id: "graphDistance", label: "Distancia de conexión", category: "Grafo",
              min: 0.02, max: 0.25, step: 0.005, defaultValue: 0.085),
        .init(id: "graphConnections", label: "Conexiones por nodo", category: "Grafo",
              min: 1, max: 3, step: 1, defaultValue: 2),
        .init(id: "graphOpacity", label: "Intensidad de líneas", category: "Grafo",
              min: 0.05, max: 10, step: 0.05, defaultValue: 0.55)
    ]

    static let flowDescriptors: [WallpaperControlDescriptor] = descriptors.map { descriptor in
        let defaultValue: Double
        switch descriptor.id {
        case "backgroundColor": defaultValue = 0x00_00_00
        case "particleColor": defaultValue = 0xFF_FF_FF
        default: defaultValue = descriptor.defaultValue
        }
        return WallpaperControlDescriptor(
            id: descriptor.id,
            label: descriptor.label,
            category: descriptor.category,
            min: descriptor.min,
            max: descriptor.max,
            step: descriptor.step,
            defaultValue: defaultValue,
            kind: descriptor.kind
        )
    }

    static let screenAdaptedDescriptors: [WallpaperControlDescriptor] = descriptors.map { descriptor in
        WallpaperControlDescriptor(
            id: descriptor.id,
            label: descriptor.label,
            category: descriptor.category,
            min: descriptor.min,
            max: descriptor.max,
            step: descriptor.step,
            defaultValue: descriptor.id == "fitToScreen" ? 1 : descriptor.defaultValue,
            kind: descriptor.kind
        )
    }

    static let torusDescriptors: [WallpaperControlDescriptor] = screenAdaptedDescriptors.map {
        descriptor in
        WallpaperControlDescriptor(
            id: descriptor.id,
            label: descriptor.label,
            category: descriptor.category,
            min: descriptor.min,
            max: descriptor.max,
            step: descriptor.step,
            defaultValue: descriptor.id == "particleSize" ? 0.9 : descriptor.defaultValue,
            kind: descriptor.kind
        )
    }

    static let chromaticRingDescriptors: [WallpaperControlDescriptor] =
        screenAdaptedDescriptors.map { descriptor in
            let defaultValue: Double
            switch descriptor.id {
            case "backgroundColor": defaultValue = 0x00_00_00
            case "particleColor": defaultValue = 0xFF_FF_FF
            case "particleSize": defaultValue = 1.75
            default: defaultValue = descriptor.defaultValue
            }
            return WallpaperControlDescriptor(
                id: descriptor.id,
                label: descriptor.label,
                category: descriptor.category,
                min: descriptor.min,
                max: descriptor.max,
                step: descriptor.step,
                defaultValue: defaultValue,
                kind: descriptor.kind
            )
        }

    var controlDescriptors: [WallpaperControlDescriptor] {
        switch kind {
        case .metalNoiseRain:
            Self.flowDescriptors
        case .metalPrimeSpiral:
            Self.screenAdaptedDescriptors
        case .metalTorusOrbit:
            Self.torusDescriptors
        case .metalChromaticRings:
            Self.chromaticRingDescriptors
        default:
            Self.descriptors
        }
    }
    var diagnosticSummary: String {
        let elapsed = max(0.001, CACurrentMediaTime() - startedAt)
        return "renderer:\(kind.rawValue) vertices:\(vertexCount) frames:\(frameCount) avgFPS:\(String(format: "%.1f", Double(frameCount) / elapsed))"
    }

    private var modelIndex: Float {
        switch kind {
        case .metalParticles: 0
        case .metalTwinVortex: 1
        case .metalOrbitalBloom: 2
        case .metalHexagonalRosette: 3
        case .metalNoiseRain: 4
        case .metalPrimeSpiral: 5
        case .metalTorusOrbit: 6
        case .metalChromaticRings: 7
        case .web: 0
        }
    }

    private var vertexCount: Int {
        switch kind {
        case .metalParticles: 10_000
        case .metalTwinVortex, .metalOrbitalBloom: 30_000
        case .metalHexagonalRosette: 19_993 * 6
        case .metalNoiseRain: Self.flowParticleCount * Self.flowHistoryCount
        case .metalPrimeSpiral: Self.primeCount
        case .metalTorusOrbit: Self.torusPointCount
        case .metalChromaticRings:
            Self.chromaticRingPointCount * Self.chromaticRingTrailCount
        case .web: 0
        }
    }

    init?(frame: CGRect, kind: WallpaperRendererKind = .metalParticles) {
        guard kind.isNativeMetal else { return nil }
        self.kind = kind
        guard let device = MTLCreateSystemDefaultDevice(),
              let queue = device.makeCommandQueue() else { return nil }
        metalView = MTKView(frame: frame, device: device)
        commandQueue = queue

        let source = """
        #include <metal_stdlib>
        using namespace metal;

        struct Uniforms {
          float time;
          float aspect;
          float pointSize;
          float scale;
          float3 rotation;
          float3 position;
          float4 appearance;
          float4 model;
          float4 screenFit;
          float4 graph;
        };
        struct VertexOut {
          float4 position [[position]];
          float pointSize [[point_size]];
          float4 color;
        };
        struct ParticleSample {
          float4 position;
          float pointSize;
          float4 color;
        };
        struct LineOut {
          float4 position [[position]];
          float4 color;
        };

        float flowHash(float3 p) {
          p = fract(p * 0.1031);
          p += dot(p, p.yzx + 33.33);
          return fract((p.x + p.y) * p.z);
        }

        float flowNoise(float3 p) {
          float3 cell = floor(p);
          float3 fraction = fract(p);
          fraction = fraction * fraction * (3.0 - 2.0 * fraction);
          float x00 = mix(flowHash(cell + float3(0, 0, 0)),
                          flowHash(cell + float3(1, 0, 0)), fraction.x);
          float x10 = mix(flowHash(cell + float3(0, 1, 0)),
                          flowHash(cell + float3(1, 1, 0)), fraction.x);
          float x01 = mix(flowHash(cell + float3(0, 0, 1)),
                          flowHash(cell + float3(1, 0, 1)), fraction.x);
          float x11 = mix(flowHash(cell + float3(0, 1, 1)),
                          flowHash(cell + float3(1, 1, 1)), fraction.x);
          return mix(mix(x00, x10, fraction.y),
                     mix(x01, x11, fraction.y), fraction.z);
        }

        float3 hsvToRGB(float3 hsv) {
          float4 constants = float4(1.0, 0.66666667, 0.33333333, 3.0);
          float3 p = abs(fract(hsv.xxx + constants.xyz) * 6.0 - constants.www);
          return hsv.z * mix(constants.xxx, clamp(p - constants.xxx, 0.0, 1.0),
                             hsv.y);
        }

        kernel void flowUpdate(uint id [[thread_position_in_grid]],
                               device float4 *particles [[buffer(0)]],
                               device float4 *history [[buffer(1)]],
                               constant uint4 &flow [[buffer(2)]]) {
          const uint particleCount = 6480;
          const uint historyCount = 12;
          if (id >= particleCount) return;

          uint frame = flow.x;
          uint first = (frame * 9) % particleCount;
          uint insertionOffset = (id + particleCount - first) % particleCount;
          bool spawned = insertionOffset < 9;
          float4 state = particles[id];

          if (spawned) {
            uint newTick = frame * 9 + insertionOffset + 1;
            state = float4(fmod(float(newTick) * 99.0, 720.0), 0.0, 0.0, 3.0);
            for (uint slot = 0; slot < historyCount; slot++) {
              history[id * historyCount + slot] = float4(0.0);
            }
          } else if (state.w <= 0.0) {
            return;
          }

          state.w *= 0.997;
          float globalTick = float((frame + 1) * 9);
          float n = flowNoise(float3(state.x / 720.0, state.y / 9.0,
                                     globalTick / 720.0));
          if (n > 0.4) {
            state.z += 0.5;
            state.y += state.z;
          } else {
            state.x += fmod(n, 0.1) > 0.05 ? 1.0 : -1.0;
            state.z = 0.0;
            state.y += 0.5;
          }

          particles[id] = state;
          uint historySlot = frame % historyCount;
          history[id * historyCount + historySlot] =
              float4(state.xy, state.w, 1.0);
        }

        ParticleSample particleSample(uint id,
                                      constant Uniforms &u,
                                      device const float4 *flowParticles,
                                      device const float4 *flowHistory) {
          float style = u.model.x;
          float2 pixel;
          float canvasSize = 400.0;
          float pointScale = 1.0;
          float trailAlpha = 1.0;
          float3 renderColor = u.appearance.rgb;

          if (style < 0.5) {
            // Ondas Paramétricas: PI/80 per p5 frame at a 30 FPS baseline.
            float t = u.time * 1.17809725;
            float i = 9999.0 - float(id);
            float y = i / 235.0;
            float k = (4.0 + cos(i / 9.0 - t * 2.0)) * cos(i / 35.0);
            float e = y / 7.0 - 13.0;
            float d = length(float2(k, e)) + sin(e / 9.0 + t / 2.0) - 4.0;
            float q = 2.0 * sin(k * 3.0)
                    - y / 35.0 * k * (9.0 + k * sin(cos(e) * 9.0 - d * 2.0 + t));
            float c = d - t;
            pixel = float2(q + 40.0 * cos(c) + 200.0,
                           q * sin(c) + d * 35.0);
          } else if (style < 1.5) {
            // Vórtice Gemelo: direct translation of the supplied 30k-point dweet.
            float t = u.time * 2.09439510;
            float i = 29999.0 - float(id);
            float m = fmod(i, 2.0) * 3.0;
            float k = 14.0 * cos(i / 39.0);
            float e = i / 1200.0 - 13.0;
            float d = dot(float2(k, e), float2(k, e)) / 59.0 + 1.0;
            float q = 89.0 - sin(k) * d
                    + k * (8.0 / d + sin(d * 3.0 + e / 9.0 - t));
            float c = d * 0.45 - sin(t - d) / 8.0 - t / 8.0 + m;
            pixel = float2(q * sin(c) + 200.0,
                           (q + 40.0 + 30.0 * sin(c * 2.0 + m)) * cos(c) + 200.0);
          } else if (style < 2.5) {
            // Flor Orbital: the bitwise JS term becomes 80 or 160 by parity.
            float t = u.time * 1.57079633;
            float i = 29999.0 - float(id);
            float y = i / 799.0;
            float k = 5.0 * cos(i / 48.0);
            float e = 5.0 * cos(y / 9.0);
            float divisor = 6.0 + fmod(i, 4.0);
            float d = pow(length(float2(k, e)) / divisor, 4.0) + 4.0;
            float parityOffset = 80.0 * (1.0 + fmod(i, 2.0));
            float q = k * (3.0 + e / 2.0 * sin(d * 8.0 + k / 9.0 - t))
                    - 3.0 * sin(k * d / 3.0) + parityOffset;
            float c = d - t / 9.0 + fmod(i, 5.0);
            pixel = float2(q * sin(c) + 200.0,
                           q * cos(c - fmod(i, 2.0) + fmod(i, 5.0) * 3.0 + 7.0)
                           + 200.0);
          } else if (style < 3.5) {
            // Roseta Hexagonal: generate the six feedback rotations directly
            // instead of copying the previous framebuffer six times.
            uint sample = id / 6;
            uint copy = id % 6;
            float t = u.time * 0.39269908;
            float i = 19999.0 - float(sample);
            float k = fmod(i, 25.0) - 12.0;
            float e = i / 800.0;
            float d = 7.0 * cos(length(float2(k, e)) / 3.0 + t / 2.0);
            float2 centered = float2(k * 4.0 + d * k * sin(d + e / 9.0 + t),
                                     e * 2.0 - d * 9.0 - d * 9.0 * cos(d + t));
            float angle = float(copy) * 1.04719755;
            float ca = cos(angle), sa = sin(angle);
            centered = float2(centered.x * ca - centered.y * sa,
                              centered.x * sa + centered.y * ca);
            pixel = centered + 200.0;
          } else if (style < 4.5) {
            // Lluvia de Ruido: positions come from the persistent compute
            // buffer. Twelve history slots approximate p5's BLUR feedback
            // without copying and filtering the entire framebuffer.
            const uint historyCount = 12;
            uint particleID = id / historyCount;
            uint trailID = id % historyCount;
            uint latestSlot = uint(u.model.y + 0.5);
            uint slot = (latestSlot + historyCount - trailID) % historyCount;
            float4 sample = flowHistory[particleID * historyCount + slot];
            pixel = sample.xy;
            canvasSize = 720.0;
            pointScale = max(0.16, sample.z / 3.0);
            trailAlpha = sample.w * exp(-float(trailID) * 0.2);
          } else if (style < 5.5) {
            // Espiral Prima: upload only the 78,498 primes represented by the
            // million-entry Processing sieve, not the composite placeholders.
            float prime = flowParticles[id].x;
            float t = 1.0 + u.time * 0.000003;
            pixel = float2(prime * sin(prime * t) / 99.0 + 400.0,
                           prime * cos(prime * t) / 99.0 + 400.0);
            canvasSize = 800.0;
            pointScale = 0.38;
          } else if (style < 6.5) {
            // Órbita Toroidal: 64 toruses represented by a balanced 36,864
            // point cloud, plus a 512-point light sphere.
            float f = u.time * 0.3;
            float3 world;
            if (id < 512) {
              float sample = float(id) + 0.5;
              float sphereY = 1.0 - 2.0 * sample / 512.0;
              float sphereRadius = sqrt(max(0.0, 1.0 - sphereY * sphereY));
              float sphereAngle = sample * 2.39996323;
              float3 unitSphere = float3(cos(sphereAngle) * sphereRadius,
                                         sphereY,
                                         sin(sphereAngle) * sphereRadius);
              world = float3(60.0 * sin(f) + 99.0 * sin(-f),
                             -20.0,
                             170.0 + 99.0 * cos(f)) + unitSphere * 4.0;
              pointScale = 0.75;
            } else {
              uint localID = id - 512;
              uint torusID = localID / (96 * 6);
              uint torusPoint = localID % (96 * 6);
              uint majorID = torusPoint / 6;
              uint tubeID = torusPoint % 6;
              float major = float(majorID) * 6.28318531 / 96.0;
              float tube = float(tubeID) * 6.28318531 / 6.0;
              float radius = 80.0 + 4.0 * cos(tube);
              float3 local = float3(radius * cos(major),
                                    radius * sin(major),
                                    4.0 * sin(tube));

              float cx = cos(0.8), sx = sin(0.8);
              local = float3(local.x,
                             local.y * cx - local.z * sx,
                             local.y * sx + local.z * cx);
              local.x += 120.0;

              float ringAngle = float(torusID) * 3.14159265 / 32.0 + f;
              float cy = cos(ringAngle), sy = sin(ringAngle);
              world = float3(local.x * cy + local.z * sy,
                             local.y,
                             -local.x * sy + local.z * cy);
              world += float3(60.0 * sin(f), -20.0, 170.0);
              pointScale = 0.42;
            }

            float eyeZ = 346.41016;
            float perspective = eyeZ / max(72.0, eyeZ - world.z);
            pixel = float2(200.0 + world.x * perspective,
                           200.0 + world.y * perspective);
          } else {
            // Anillos Cromáticos: the source's eleven HSB rings contain 6,225
            // points. Eight nearby time samples reproduce the soft BLUR trail
            // without filtering or retaining a full 720x720 framebuffer.
            const uint trailCount = 8;
            uint baseID = id / trailCount;
            uint trailID = id % trailCount;
            uint localID = baseID;
            float d = 330.0;
            uint ringPointCount = 0;
            for (uint ring = 0; ring < 11; ring++) {
              ringPointCount = uint(ceil(3.14159265 * d));
              if (localID < ringPointCount) break;
              localID -= ringPointCount;
              d -= 30.0;
            }

            float t = max(0.0, u.time * 30.0 - float(trailID) * 0.85);
            float r = float(localID) * 2.0 / d;
            float tangent = tan(d / 199.0 - t / 99.0);
            float energy = min(tangent * tangent, 64.0);
            float noiseValue = flowNoise(float3(r * 99.0, d, 0.0));
            float radius = d
                + sin(r * 9.0 + t / 9.0 * d / 720.0)
                * d * 0.25 * noiseValue * energy;
            float angle = r - 1.57079633;
            pixel = float2(cos(angle) * radius + 360.0,
                           sin(angle) * radius + 360.0);
            canvasSize = 720.0;
            pointScale = 0.9;

            // p5's default HSB range is 0...255. The tint control multiplies
            // the generated hue, so white keeps the original palette.
            float hue = clamp(d, 0.0, 255.0) / 255.0;
            float3 hsbColor = hsvToRGB(float3(hue, 50.0 / 255.0, 1.0));
            renderColor = hsbColor * u.appearance.rgb;
            float sourceAlpha = clamp(0.7 / max(energy, 0.025), 0.018, 0.82);
            trailAlpha = sourceAlpha * exp(-float(trailID) * 0.32);
          }

          // Convert the source canvas into aspect-correct clip space.
          float center = canvasSize * 0.5;
          float2 p = float2((pixel.x - center) / center,
                            (center - pixel.y) / center);
          float cz = cos(u.rotation.z), sz = sin(u.rotation.z);
          p = float2(p.x * cz - p.y * sz, p.x * sz + p.y * cz);
          p *= u.scale * exp(u.position.z * 0.08);
          // X/Y rotations become a subtle 2D perspective compression.
          p *= float2(cos(u.rotation.y), cos(u.rotation.x));
          p += u.position.xy * 0.25;
          p *= max(float2(0.05), u.screenFit.yz);
          if (u.screenFit.x < 0.5) {
            p.x /= max(0.1, u.aspect);
          }

          ParticleSample out;
          out.position = float4(p, 0.0, 1.0);
          out.pointSize = max(1.0, u.pointSize * pointScale);
          float brightness = max(0.05, u.appearance.w);
          out.color = float4(renderColor * brightness,
                             clamp(0.38 * brightness, 0.08, 1.0) * trailAlpha);
          return out;
        }

        vertex VertexOut particleVertex(uint id [[vertex_id]],
                                        constant Uniforms &u [[buffer(0)]],
                                        device const float4 *flowParticles [[buffer(1)]],
                                        device const float4 *flowHistory [[buffer(2)]]) {
          ParticleSample sample = particleSample(id, u, flowParticles, flowHistory);
          VertexOut out;
          out.position = sample.position;
          out.pointSize = sample.pointSize;
          out.color = sample.color;
          return out;
        }

        kernel void graphPositionUpdate(
            uint id [[thread_position_in_grid]],
            constant Uniforms &u [[buffer(0)]],
            device const float4 *flowParticles [[buffer(1)]],
            device const float4 *flowHistory [[buffer(2)]],
            device float4 *positions [[buffer(3)]]) {
          const uint nodeCount = 768;
          if (id >= nodeCount) return;
          uint sourceCount = max(1u, uint(u.model.w + 0.5));
          uint sourceID = min(sourceCount - 1,
                              uint(float(id) * float(sourceCount) / float(nodeCount)));
          ParticleSample sample = particleSample(sourceID, u, flowParticles, flowHistory);
          bool visible = all(abs(sample.position.xy) <= float2(1.15))
              && sample.color.a > 0.005;
          positions[id] = float4(sample.position.xy,
                                 visible ? sample.color.a : 0.0,
                                 float(sourceID));
        }

        kernel void graphConnectionUpdate(
            uint id [[thread_position_in_grid]],
            device const float4 *positions [[buffer(0)]],
            device float4 *edges [[buffer(1)]],
            constant float4 &graph [[buffer(2)]]) {
          const uint nodeCount = 768;
          const uint maxConnections = 3;
          if (id >= nodeCount) return;

          float4 source = positions[id];
          float threshold = max(0.001, graph.y);
          float thresholdSquared = threshold * threshold;
          float bestDistance0 = thresholdSquared;
          float bestDistance1 = thresholdSquared;
          float bestDistance2 = thresholdSquared;
          uint bestID0 = 0xffffffffu;
          uint bestID1 = 0xffffffffu;
          uint bestID2 = 0xffffffffu;

          if (source.z > 0.0) {
            for (uint candidate = id + 1; candidate < nodeCount; candidate++) {
              float4 target = positions[candidate];
              if (target.z <= 0.0) continue;
              float2 delta = source.xy - target.xy;
              float distanceSquared = dot(delta, delta);
              if (distanceSquared >= bestDistance2) continue;
              if (distanceSquared < bestDistance0) {
                bestDistance2 = bestDistance1;
                bestID2 = bestID1;
                bestDistance1 = bestDistance0;
                bestID1 = bestID0;
                bestDistance0 = distanceSquared;
                bestID0 = candidate;
              } else if (distanceSquared < bestDistance1) {
                bestDistance2 = bestDistance1;
                bestID2 = bestID1;
                bestDistance1 = distanceSquared;
                bestID1 = candidate;
              } else {
                bestDistance2 = distanceSquared;
                bestID2 = candidate;
              }
            }
          }

          uint requestedConnections = min(maxConnections, uint(graph.w + 0.5));
          for (uint slot = 0; slot < maxConnections; slot++) {
            uint targetID = slot == 0 ? bestID0 : (slot == 1 ? bestID1 : bestID2);
            float distanceSquared = slot == 0
                ? bestDistance0
                : (slot == 1 ? bestDistance1 : bestDistance2);
            uint edge = (id * maxConnections + slot) * 2;
            if (slot < requestedConnections && targetID != 0xffffffffu) {
              float4 target = positions[targetID];
              float proximity = 1.0 - sqrt(distanceSquared) / threshold;
              float alpha = clamp(graph.z * proximity * min(source.z, target.z),
                                  0.0, 1.0);
              edges[edge] = float4(source.xy, alpha, 0.0);
              edges[edge + 1] = float4(target.xy, alpha, 0.0);
            } else {
              edges[edge] = float4(2.0, 2.0, 0.0, 0.0);
              edges[edge + 1] = float4(2.0, 2.0, 0.0, 0.0);
            }
          }
        }

        vertex LineOut graphVertex(uint id [[vertex_id]],
                                   device const float4 *edges [[buffer(0)]],
                                   constant Uniforms &u [[buffer(1)]]) {
          float4 edge = edges[id];
          LineOut out;
          out.position = float4(edge.xy, 0.0, 1.0);
          out.color = float4(u.appearance.rgb * max(0.05, u.appearance.w), edge.z);
          return out;
        }

        fragment float4 graphFragment(LineOut in [[stage_in]]) {
          return in.color;
        }

        fragment float4 particleFragment(VertexOut in [[stage_in]],
                                         float2 pointCoord [[point_coord]]) {
          float2 centered = pointCoord * 2.0 - 1.0;
          float alpha = smoothstep(1.0, 0.15, length(centered)) * in.color.a;
          return float4(in.color.rgb, alpha);
        }
        """

        do {
            let library = try device.makeLibrary(source: source, options: nil)
            guard let vertex = library.makeFunction(name: "particleVertex"),
                  let fragment = library.makeFunction(name: "particleFragment"),
                  let flowUpdate = library.makeFunction(name: "flowUpdate"),
                  let graphVertex = library.makeFunction(name: "graphVertex"),
                  let graphFragment = library.makeFunction(name: "graphFragment"),
                  let graphPositionUpdate = library.makeFunction(name: "graphPositionUpdate"),
                  let graphConnectionUpdate = library.makeFunction(name: "graphConnectionUpdate")
            else { return nil }
            let descriptor = MTLRenderPipelineDescriptor()
            descriptor.vertexFunction = vertex
            descriptor.fragmentFunction = fragment
            descriptor.colorAttachments[0].pixelFormat = .bgra8Unorm
            descriptor.colorAttachments[0].isBlendingEnabled = true
            descriptor.colorAttachments[0].sourceRGBBlendFactor = .sourceAlpha
            descriptor.colorAttachments[0].destinationRGBBlendFactor = .oneMinusSourceAlpha
            descriptor.colorAttachments[0].sourceAlphaBlendFactor = .sourceAlpha
            descriptor.colorAttachments[0].destinationAlphaBlendFactor = .oneMinusSourceAlpha
            pipeline = try device.makeRenderPipelineState(descriptor: descriptor)
            flowPipeline = try device.makeComputePipelineState(function: flowUpdate)
            graphPositionPipeline =
                try device.makeComputePipelineState(function: graphPositionUpdate)
            graphConnectionPipeline =
                try device.makeComputePipelineState(function: graphConnectionUpdate)

            let graphDescriptor = MTLRenderPipelineDescriptor()
            graphDescriptor.vertexFunction = graphVertex
            graphDescriptor.fragmentFunction = graphFragment
            graphDescriptor.inputPrimitiveTopology = .line
            graphDescriptor.colorAttachments[0].pixelFormat = .bgra8Unorm
            graphDescriptor.colorAttachments[0].isBlendingEnabled = true
            graphDescriptor.colorAttachments[0].sourceRGBBlendFactor = .sourceAlpha
            graphDescriptor.colorAttachments[0].destinationRGBBlendFactor = .oneMinusSourceAlpha
            graphDescriptor.colorAttachments[0].sourceAlphaBlendFactor = .sourceAlpha
            graphDescriptor.colorAttachments[0].destinationAlphaBlendFactor = .oneMinusSourceAlpha
            graphPipeline = try device.makeRenderPipelineState(descriptor: graphDescriptor)

            let usesFlowState = kind == .metalNoiseRain
            let usesPrimeData = kind == .metalPrimeSpiral
            let particleElementCount = usesFlowState
                ? Self.flowParticleCount
                : (usesPrimeData ? Self.primeCount : 1)
            let particleLength = particleElementCount * MemoryLayout<SIMD4<Float>>.stride
            let historyLength = usesFlowState
                ? Self.flowParticleCount * Self.flowHistoryCount * MemoryLayout<SIMD4<Float>>.stride
                : MemoryLayout<SIMD4<Float>>.stride
            let graphPositionLength =
                Self.graphNodeCount * MemoryLayout<SIMD4<Float>>.stride
            let graphEdgeLength =
                Self.graphNodeCount * Self.graphMaxConnections * 2
                * MemoryLayout<SIMD4<Float>>.stride
            guard let particles = device.makeBuffer(length: particleLength,
                                                    options: .storageModeShared),
                  let history = device.makeBuffer(length: historyLength,
                                                  options: .storageModeShared),
                  let positions = device.makeBuffer(length: graphPositionLength,
                                                    options: .storageModePrivate),
                  let edges = device.makeBuffer(length: graphEdgeLength,
                                                options: .storageModePrivate)
            else { return nil }
            particles.contents().initializeMemory(as: UInt8.self,
                                                  repeating: 0,
                                                  count: particleLength)
            history.contents().initializeMemory(as: UInt8.self,
                                                repeating: 0,
                                                count: historyLength)
            if usesPrimeData {
                let primes = Self.generatePrimes(upTo: Self.primeLimit)
                guard primes.count == Self.primeCount else { return nil }
                let destination = particles.contents()
                    .bindMemory(to: SIMD4<Float>.self, capacity: primes.count)
                for (index, prime) in primes.enumerated() {
                    destination[index] = SIMD4(Float(prime), 0, 0, 0)
                }
            }
            flowParticles = particles
            flowHistory = history
            graphPositions = positions
            graphEdges = edges
        } catch {
            NSLog("ParticleWall: Metal pipeline failed: \(error)")
            return nil
        }

        if kind == .metalNoiseRain || kind == .metalPrimeSpiral
            || kind == .metalTorusOrbit || kind == .metalChromaticRings {
            particleColor = SIMD3<Float>(repeating: 1)
            backgroundColor = SIMD3<Float>(repeating: 0)
        }
        if kind == .metalPrimeSpiral || kind == .metalTorusOrbit
            || kind == .metalChromaticRings {
            fitToScreen = true
        }
        if kind == .metalTorusOrbit {
            particleSize = 0.9
        } else if kind == .metalChromaticRings {
            particleSize = 1.75
        }

        super.init()
        metalView.delegate = self
        metalView.autoresizingMask = [.width, .height]
        metalView.colorPixelFormat = .bgra8Unorm
        metalView.colorspace = CGColorSpace(name: CGColorSpace.sRGB)
        metalView.clearColor = MTLClearColorMake(Double(backgroundColor.x),
                                                 Double(backgroundColor.y),
                                                 Double(backgroundColor.z), 1)
        // Last-frame capture blits the drawable into a CPU-readable buffer.
        metalView.framebufferOnly = false
        metalView.autoResizeDrawable = false
        metalView.enableSetNeedsDisplay = false
        metalView.preferredFramesPerSecond = 30
        metalView.isPaused = false
        updateDrawableSize()
    }

    func updateFrame(_ frame: CGRect) {
        metalView.frame = frame
        updateDrawableSize()
    }

    func setPlayback(paused: Bool, fpsCap: Int, adaptiveQuality: Bool) {
        metalView.preferredFramesPerSecond = fpsCap > 0 ? fpsCap : 60
        metalView.isPaused = paused
        lastDrawTime = nil
    }

    func applyControlValues(_ values: [String: Double]) {
        func float(_ key: String, _ fallback: Float) -> Float {
            values[key].map(Float.init) ?? fallback
        }
        position = SIMD3(float("positionX", position.x),
                         float("positionY", position.y),
                         float("positionZ", position.z))
        let radians = Float.pi / 180
        rotation = SIMD3(float("rotationX", rotation.x / radians) * radians,
                         float("rotationY", rotation.y / radians) * radians,
                         float("rotationZ", rotation.z / radians) * radians)
        scale = max(0.05, float("scale", scale))
        speed = max(0, float("speed", speed))
        fitToScreen = float("fitToScreen", fitToScreen ? 1 : 0) >= 0.5
        horizontalLimit = max(0.05, float("horizontalLimit", horizontalLimit))
        verticalLimit = max(0.05, float("verticalLimit", verticalLimit))
        particleSize = max(0.1, float("particleSize", particleSize))
        brightness = max(0.05, float("brightness", brightness))
        graphEnabled = float("graphEnabled", graphEnabled ? 1 : 0) >= 0.5
        graphDistance = max(0.005, float("graphDistance", graphDistance))
        graphConnections = min(Float(Self.graphMaxConnections),
                               max(1, float("graphConnections", graphConnections)))
        graphOpacity = min(10, max(0.01, float("graphOpacity", graphOpacity)))
        if let value = values["particleColor"] {
            particleColor = Self.rgb(value)
        }
        if let value = values["backgroundColor"] {
            backgroundColor = Self.rgb(value)
            metalView.clearColor = MTLClearColorMake(Double(backgroundColor.x),
                                                     Double(backgroundColor.y),
                                                     Double(backgroundColor.z), 1)
        }
    }

    func captureSnapshot(targetPixelSize: CGSize, completion: @escaping (NSImage?) -> Void) {
        pendingSnapshots.append(completion)
        let previousSize = metalView.drawableSize
        let width = max(1, targetPixelSize.width.rounded())
        let height = max(1, targetPixelSize.height.rounded())
        metalView.drawableSize = CGSize(width: width, height: height)
        metalView.draw()
        metalView.drawableSize = previousSize
    }

    func tearDown() {
        let requests = pendingSnapshots
        pendingSnapshots.removeAll()
        requests.forEach { $0(nil) }
        metalView.isPaused = true
        metalView.delegate = nil
        metalView.removeFromSuperview()
    }

    func mtkView(_ view: MTKView, drawableSizeWillChange size: CGSize) {}

    private func updateDrawableSize() {
        let configured = UserDefaults.standard.double(forKey: DefaultsKey.renderScale)
        let scale = max(0.75, min(2, configured > 0 ? configured : 1.5))
        metalView.drawableSize = CGSize(width: max(1, metalView.bounds.width * scale),
                                        height: max(1, metalView.bounds.height * scale))
    }

    private static func rgb(_ packed: Double) -> SIMD3<Float> {
        let components = PackedRGB.components(packed)
        return SIMD3(Float(components.red), Float(components.green), Float(components.blue))
    }

    private static func generatePrimes(upTo limit: Int) -> [Int] {
        guard limit >= 2 else { return [] }
        var composite = [Bool](repeating: false, count: limit + 1)
        let squareRoot = Int(Double(limit).squareRoot())
        if squareRoot >= 2 {
            for candidate in 2...squareRoot where !composite[candidate] {
                var multiple = candidate * candidate
                while multiple <= limit {
                    composite[multiple] = true
                    multiple += candidate
                }
            }
        }
        return (2...limit).filter { !composite[$0] }
    }

    func draw(in view: MTKView) {
        guard let drawable = view.currentDrawable,
              let pass = view.currentRenderPassDescriptor,
              let buffer = commandQueue.makeCommandBuffer() else { return }
        let drawableSize = view.drawableSize
        let now = CACurrentMediaTime()
        var delta: Float = 0
        if let lastDrawTime {
            delta = Float(min(0.1, now - lastDrawTime))
            animationTime += delta * speed
        }
        lastDrawTime = now
        if kind == .metalNoiseRain {
            encodeFlowUpdates(in: buffer, delta: delta)
        }

        let latestFlowSlot = flowFrame == 0
            ? 0
            : Float((flowFrame - 1) % UInt32(Self.flowHistoryCount))
        var uniforms = Uniforms(
            time: animationTime,
            aspect: Float(max(1, drawableSize.width) / max(1, drawableSize.height)),
            pointSize: Float(max(1.25, min(3, view.drawableSize.height /
                                          max(1, view.bounds.height)) * 1.25)) * particleSize,
            scale: scale,
            rotation: rotation,
            position: position,
            appearance: SIMD4(particleColor.x, particleColor.y, particleColor.z, brightness),
            model: SIMD4(modelIndex, latestFlowSlot, Float(Self.flowHistoryCount),
                         Float(vertexCount)),
            screenFit: SIMD4(fitToScreen ? 1 : 0, horizontalLimit, verticalLimit, 0),
            graph: SIMD4(graphEnabled ? 1 : 0, graphDistance, graphOpacity,
                         graphConnections)
        )
        if graphEnabled {
            encodeGraphUpdates(in: buffer, uniforms: &uniforms)
        }
        if graphEnabled {
            guard let graphEncoder = buffer.makeRenderCommandEncoder(descriptor: pass) else {
                return
            }
            graphEncoder.setRenderPipelineState(graphPipeline)
            graphEncoder.setVertexBuffer(graphEdges, offset: 0, index: 0)
            graphEncoder.setVertexBytes(&uniforms,
                                        length: MemoryLayout<Uniforms>.stride,
                                        index: 1)
            graphEncoder.drawPrimitives(
                type: .line,
                vertexStart: 0,
                vertexCount: Self.graphNodeCount * Self.graphMaxConnections * 2
            )
            graphEncoder.endEncoding()
            pass.colorAttachments[0].loadAction = .load
        }

        guard let encoder = buffer.makeRenderCommandEncoder(descriptor: pass) else { return }
        encoder.setRenderPipelineState(pipeline)
        encoder.setVertexBytes(&uniforms, length: MemoryLayout<Uniforms>.stride, index: 0)
        encoder.setVertexBuffer(flowParticles, offset: 0, index: 1)
        encoder.setVertexBuffer(flowHistory, offset: 0, index: 2)
        encoder.drawPrimitives(type: .point, vertexStart: 0, vertexCount: vertexCount)
        encoder.endEncoding()

        let snapshotRequests = pendingSnapshots
        pendingSnapshots.removeAll()
        if !snapshotRequests.isEmpty {
            encodeSnapshot(of: drawable.texture,
                           in: buffer,
                           requests: snapshotRequests)
        }
        buffer.present(drawable)
        buffer.commit()
        frameCount += 1
    }

    private func encodeSnapshot(of texture: MTLTexture,
                                in commandBuffer: MTLCommandBuffer,
                                requests: [(NSImage?) -> Void]) {
        let width = texture.width
        let height = texture.height
        let unalignedBytesPerRow = width * 4
        let bytesPerRow = (unalignedBytesPerRow + 255) & ~255
        let byteCount = bytesPerRow * height
        guard width > 0, height > 0,
              let readback = metalView.device?.makeBuffer(length: byteCount,
                                                          options: .storageModeShared),
              let blit = commandBuffer.makeBlitCommandEncoder() else {
            requests.forEach { $0(nil) }
            return
        }

        blit.copy(from: texture,
                  sourceSlice: 0,
                  sourceLevel: 0,
                  sourceOrigin: .init(x: 0, y: 0, z: 0),
                  sourceSize: .init(width: width, height: height, depth: 1),
                  to: readback,
                  destinationOffset: 0,
                  destinationBytesPerRow: bytesPerRow,
                  destinationBytesPerImage: byteCount)
        blit.endEncoding()

        commandBuffer.addCompletedHandler { commandBuffer in
            let image: NSImage?
            if commandBuffer.status == .completed {
                let data = Data(bytes: readback.contents(), count: byteCount)
                image = Self.image(fromBGRA: data,
                                   width: width,
                                   height: height,
                                   bytesPerRow: bytesPerRow)
            } else {
                image = nil
                if let error = commandBuffer.error {
                    NSLog("ParticleWall: Metal snapshot failed: \(error)")
                }
            }
            DispatchQueue.main.async {
                requests.forEach { $0(image) }
            }
        }
    }

    private static func image(fromBGRA data: Data,
                              width: Int,
                              height: Int,
                              bytesPerRow: Int) -> NSImage? {
        guard let provider = CGDataProvider(data: data as CFData),
              let image = CGImage(width: width,
                                  height: height,
                                  bitsPerComponent: 8,
                                  bitsPerPixel: 32,
                                  bytesPerRow: bytesPerRow,
                                  space: CGColorSpace(name: CGColorSpace.sRGB)
                                    ?? CGColorSpaceCreateDeviceRGB(),
                                  bitmapInfo: CGBitmapInfo.byteOrder32Little.union(
                                    CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedFirst.rawValue)
                                  ),
                                  provider: provider,
                                  decode: nil,
                                  shouldInterpolate: true,
                                  intent: .defaultIntent) else { return nil }
        return NSImage(cgImage: image, size: NSSize(width: width, height: height))
    }

    private func encodeGraphUpdates(in commandBuffer: MTLCommandBuffer,
                                    uniforms: inout Uniforms) {
        let positionWidth = min(graphPositionPipeline.threadExecutionWidth,
                                graphPositionPipeline.maxTotalThreadsPerThreadgroup)
        guard let positionEncoder = commandBuffer.makeComputeCommandEncoder() else { return }
        positionEncoder.setComputePipelineState(graphPositionPipeline)
        positionEncoder.setBytes(&uniforms, length: MemoryLayout<Uniforms>.stride, index: 0)
        positionEncoder.setBuffer(flowParticles, offset: 0, index: 1)
        positionEncoder.setBuffer(flowHistory, offset: 0, index: 2)
        positionEncoder.setBuffer(graphPositions, offset: 0, index: 3)
        positionEncoder.dispatchThreads(
            MTLSize(width: Self.graphNodeCount, height: 1, depth: 1),
            threadsPerThreadgroup: MTLSize(width: positionWidth, height: 1, depth: 1)
        )
        positionEncoder.endEncoding()

        let connectionWidth = min(graphConnectionPipeline.threadExecutionWidth,
                                  graphConnectionPipeline.maxTotalThreadsPerThreadgroup)
        guard let connectionEncoder = commandBuffer.makeComputeCommandEncoder() else { return }
        var graph = uniforms.graph
        connectionEncoder.setComputePipelineState(graphConnectionPipeline)
        connectionEncoder.setBuffer(graphPositions, offset: 0, index: 0)
        connectionEncoder.setBuffer(graphEdges, offset: 0, index: 1)
        connectionEncoder.setBytes(&graph, length: MemoryLayout<SIMD4<Float>>.stride, index: 2)
        connectionEncoder.dispatchThreads(
            MTLSize(width: Self.graphNodeCount, height: 1, depth: 1),
            threadsPerThreadgroup: MTLSize(width: connectionWidth, height: 1, depth: 1)
        )
        connectionEncoder.endEncoding()
    }

    private func encodeFlowUpdates(in commandBuffer: MTLCommandBuffer, delta: Float) {
        flowStepAccumulator += delta * 30 * speed
        let stepCount = min(6, Int(flowStepAccumulator))
        guard stepCount > 0 else { return }

        let threads = MTLSize(
            width: min(flowPipeline.threadExecutionWidth,
                       flowPipeline.maxTotalThreadsPerThreadgroup),
            height: 1,
            depth: 1
        )
        let grid = MTLSize(width: Self.flowParticleCount, height: 1, depth: 1)
        for _ in 0..<stepCount {
            guard let encoder = commandBuffer.makeComputeCommandEncoder() else { break }
            var flow = SIMD4<UInt32>(
                flowFrame,
                flowFrame % UInt32(Self.flowHistoryCount),
                UInt32(Self.flowParticleCount),
                UInt32(Self.flowHistoryCount)
            )
            encoder.setComputePipelineState(flowPipeline)
            encoder.setBuffer(flowParticles, offset: 0, index: 0)
            encoder.setBuffer(flowHistory, offset: 0, index: 1)
            encoder.setBytes(&flow, length: MemoryLayout<SIMD4<UInt32>>.stride, index: 2)
            encoder.dispatchThreads(grid, threadsPerThreadgroup: threads)
            encoder.endEncoding()
            flowFrame &+= 1
            flowStepAccumulator -= 1
        }
    }
}
