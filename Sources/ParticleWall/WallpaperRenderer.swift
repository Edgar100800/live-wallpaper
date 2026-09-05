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
    /// Naga's reserved [[buffer(6)]] (bound, never read).
    private let bufferSizes: MTLBuffer
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
        /// Engine addition: physical drawable size in pixels, used by the
        /// shared WGSL to expand point sprites as instanced quads.
        var viewport: SIMD2<Float>
        var pad: SIMD2<Float>
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
        case .metalNoiseRain, .metalTorusKnot:
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
        case .metalSphereTorus: 8
        case .metalJellyfishPoints: 9
        case .metalNebula: 10
        case .metalTorusKnot: 11
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
        case .metalSphereTorus: 40 * 80
        case .metalJellyfishPoints: 10_000
        case .metalNebula: 9 * 512 * 200
        case .metalTorusKnot: 20_000
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

        // Shared engine source of truth: shared/backgrounds/engines/
        // particle-v1/particle.wgsl, compiled to MSL by tools/xtask.
        guard let shaderURL = Bundle.module.url(
            forResource: "particle",
            withExtension: "metal",
            subdirectory: "generated"
        ), let source = try? String(contentsOf: shaderURL, encoding: .utf8) else {
            NSLog("ParticleWall: generated particle.metal missing from bundle")
            return nil
        }

        do {
            let library = try device.makeLibrary(source: source, options: nil)
            // Entry point names come from the shared WGSL (tools/xtask).
            guard let vertex = library.makeFunction(name: "vsMain"),
                  let fragment = library.makeFunction(name: "fsMain"),
                  let flowUpdate = library.makeFunction(name: "flowUpdate"),
                  let graphVertex = library.makeFunction(name: "vsLine"),
                  let graphFragment = library.makeFunction(name: "fsLine"),
                  let graphPositionUpdate = library.makeFunction(name: "graphPositionUpdate"),
                  let graphConnectionUpdate = library.makeFunction(name: "graphConnectionUpdate")
            else { return nil }
            let descriptor = MTLRenderPipelineDescriptor()
            descriptor.vertexFunction = vertex
            descriptor.fragmentFunction = fragment
            // The shared engine expands every particle to a 6-vertex quad.
            descriptor.inputPrimitiveTopology = .triangle
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
                                                options: .storageModePrivate),
                  // Naga reserves [[buffer(6)]] for bounds-check sizes; the
                  // generated code never reads them (unchecked policies), but
                  // the argument must stay bound.
                  let bufferSizes = device.makeBuffer(length: 32,
                                                      options: .storageModeShared)
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
            bufferSizes = bufferSizes
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
                         graphConnections),
            viewport: SIMD2(Float(drawableSize.width), Float(drawableSize.height)),
            pad: SIMD2(repeating: 0)
        )
        if graphEnabled {
            encodeGraphUpdates(in: buffer, uniforms: &uniforms)
        }
        if graphEnabled {
            guard let graphEncoder = buffer.makeRenderCommandEncoder(descriptor: pass) else {
                return
            }
            graphEncoder.setRenderPipelineState(graphPipeline)
            graphEncoder.setVertexBytes(&uniforms,
                                        length: MemoryLayout<Uniforms>.stride,
                                        index: 0)
            graphEncoder.setVertexBuffer(graphEdges, offset: 0, index: 3)
            graphEncoder.setVertexBuffer(bufferSizes, offset: 0, index: 6)
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
        encoder.setVertexBuffer(bufferSizes, offset: 0, index: 6)
        // The shared engine expands each particle to a 6-vertex quad.
        encoder.drawPrimitives(type: .triangle, vertexStart: 0, vertexCount: vertexCount * 6)
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
        positionEncoder.setBuffer(graphPositions, offset: 0, index: 5)
        positionEncoder.setBuffer(bufferSizes, offset: 0, index: 6)
        positionEncoder.dispatchThreads(
            MTLSize(width: Self.graphNodeCount, height: 1, depth: 1),
            threadsPerThreadgroup: MTLSize(width: positionWidth, height: 1, depth: 1)
        )
        positionEncoder.endEncoding()

        let connectionWidth = min(graphConnectionPipeline.threadExecutionWidth,
                                  graphConnectionPipeline.maxTotalThreadsPerThreadgroup)
        guard let connectionEncoder = commandBuffer.makeComputeCommandEncoder() else { return }
        // The connection kernel reads graph parameters from the full
        // uniforms block (gu, [[buffer(0)]]).
        connectionEncoder.setComputePipelineState(graphConnectionPipeline)
        connectionEncoder.setBytes(&uniforms, length: MemoryLayout<Uniforms>.stride, index: 0)
        connectionEncoder.setBuffer(graphPositions, offset: 0, index: 5)
        connectionEncoder.setBuffer(graphEdges, offset: 0, index: 3)
        connectionEncoder.setBuffer(bufferSizes, offset: 0, index: 6)
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
            encoder.setBuffer(flowParticles, offset: 0, index: 1)
            encoder.setBuffer(flowHistory, offset: 0, index: 2)
            encoder.setBytes(&flow, length: MemoryLayout<SIMD4<UInt32>>.stride, index: 4)
            encoder.setBuffer(bufferSizes, offset: 0, index: 6)
            encoder.dispatchThreads(grid, threadsPerThreadgroup: threads)
            encoder.endEncoding()
            flowFrame &+= 1
            flowStepAccumulator -= 1
        }
    }
}
