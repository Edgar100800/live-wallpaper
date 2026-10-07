import AppKit
import MetalKit
import QuartzCore

final class ASCIIWallpaperRenderer: NSObject, WallpaperRenderer, MTKViewDelegate {
    let kind: WallpaperRendererKind = .asciiVideo
    let metalView: MTKView
    var view: NSView { metalView }
    let controlDescriptors: [WallpaperControlDescriptor] = [
        .init(id: "speed", label: "Velocidad", category: "Animación",
              min: 0, max: 3, step: 0.05, defaultValue: 1)
    ]

    private struct Uniforms {
        var columns: UInt32
        var rows: UInt32
        var viewport: SIMD2<Float>
    }

    private let commandQueue: MTLCommandQueue
    private let pipeline: MTLRenderPipelineState
    private let fillTexture: MTLTexture
    private let edgeTexture: MTLTexture
    private let frameStore: ASCIIFrameStore
    private let cellBuffers: [MTLBuffer]
    private var lastDrawTime: CFTimeInterval?
    private(set) var animationTime: Double = 0
    private var speed: Float = 1
    private var lastFrame = -1
    private var activeCellBuffer = 0
    private var frameCount: UInt64 = 0

    private static let shader = """
    #include <metal_stdlib>
    using namespace metal;

    struct Uniforms {
        uint columns;
        uint rows;
        float2 viewport;
    };

    struct VertexOut {
        float4 position [[position]];
        float2 uv;
        float4 color;
        uint glyph;
    };

    vertex VertexOut asciiVertex(
        uint vertexID [[vertex_id]],
        uint instanceID [[instance_id]],
        constant Uniforms& uniforms [[buffer(1)]],
        device const uint* cells [[buffer(0)]]) {
        constexpr float2 quad[6] = {
            float2(0, 0), float2(1, 0), float2(0, 1),
            float2(0, 1), float2(1, 0), float2(1, 1)
        };
        float2 local = quad[vertexID];
        uint packed = cells[instanceID];
        uint glyph = packed & 255u;
        uint rgb = (packed >> 8u) & 255u;
        uint column = instanceID % uniforms.columns;
        uint row = instanceID / uniforms.columns;
        float2 grid = (float2(column, row) + local)
            / float2(uniforms.columns, uniforms.rows);

        VertexOut out;
        out.position = float4(grid.x * 2.0 - 1.0, 1.0 - grid.y * 2.0, 0, 1);
        if (glyph < 10u) {
            out.uv = float2(float(glyph) * 8.0 + local.x * 8.0 + 0.5,
                            local.y * 8.0 + 0.5) / float2(80, 8);
        } else {
            float edge = float(glyph - 9u);
            out.uv = float2(edge * 8.0 + local.x * 8.0 + 0.5,
                            local.y * 8.0 + 0.5) / float2(40, 8);
        }
        out.glyph = glyph;
        out.color = float4(
            float((rgb >> 5u) & 7u) / 7.0,
            float((rgb >> 2u) & 7u) / 7.0,
            float(rgb & 3u) / 3.0,
            1.0
        );
        return out;
    }

    fragment float4 asciiFragment(
        VertexOut in [[stage_in]],
        texture2d<float> fillTexture [[texture(0)]],
        texture2d<float> edgeTexture [[texture(1)]]) {
        constexpr sampler nearest(address::clamp_to_edge, filter::nearest);
        float mask = in.glyph < 10u
            ? fillTexture.sample(nearest, in.uv).r
            : edgeTexture.sample(nearest, in.uv).r;
        if (mask < 0.05) discard_fragment();
        return float4(in.color.rgb, mask);
    }
    """

    init?(frame: CGRect, rootURL: URL) {
        guard let device = MTLCreateSystemDefaultDevice(),
              let queue = device.makeCommandQueue(),
              let store = try? ASCIIFrameStore(rootURL: rootURL),
              let library = try? device.makeLibrary(source: Self.shader, options: nil),
              let vertex = library.makeFunction(name: "asciiVertex"),
              let fragment = library.makeFunction(name: "asciiFragment"),
              let fillURL = Bundle.module.url(forResource: "fillASCII", withExtension: "png",
                                              subdirectory: "ASCII"),
              let edgeURL = Bundle.module.url(forResource: "edgesASCII", withExtension: "png",
                                              subdirectory: "ASCII") else {
            return nil
        }

        let loader = MTKTextureLoader(device: device)
        guard let fill = try? loader.newTexture(URL: fillURL, options: nil),
              let edge = try? loader.newTexture(URL: edgeURL, options: nil) else {
            return nil
        }
        let descriptor = MTLRenderPipelineDescriptor()
        descriptor.vertexFunction = vertex
        descriptor.fragmentFunction = fragment
        descriptor.colorAttachments[0].pixelFormat = .bgra8Unorm
        descriptor.colorAttachments[0].isBlendingEnabled = true
        descriptor.colorAttachments[0].sourceRGBBlendFactor = .sourceAlpha
        descriptor.colorAttachments[0].destinationRGBBlendFactor = .oneMinusSourceAlpha
        descriptor.colorAttachments[0].sourceAlphaBlendFactor = .sourceAlpha
        descriptor.colorAttachments[0].destinationAlphaBlendFactor = .oneMinusSourceAlpha
        guard let pipeline = try? device.makeRenderPipelineState(descriptor: descriptor) else {
            return nil
        }
        let cellBuffers = (0..<3).compactMap { _ in
            device.makeBuffer(length: store.cellCount * MemoryLayout<UInt32>.stride,
                              options: .storageModeShared)
        }
        guard cellBuffers.count == 3 else { return nil }

        frameStore = store
        commandQueue = queue
        self.pipeline = pipeline
        fillTexture = fill
        edgeTexture = edge
        self.cellBuffers = cellBuffers
        metalView = MTKView(frame: frame, device: device)
        super.init()
        metalView.delegate = self
        metalView.autoresizingMask = [.width, .height]
        metalView.colorPixelFormat = .bgra8Unorm
        metalView.colorspace = CGColorSpace(name: CGColorSpace.sRGB)
        metalView.clearColor = MTLClearColorMake(0, 0, 0, 1)
        metalView.framebufferOnly = false
        metalView.autoResizeDrawable = false
        metalView.enableSetNeedsDisplay = false
        metalView.preferredFramesPerSecond = min(60, max(1, Int(store.header.fps.rounded())))
        updateDrawableSize()
        uploadFrame(0)
    }

    var diagnosticSummary: String {
        "renderer:ascii-video cells:\(frameStore.cellCount) frames:\(frameCount)"
    }

    func updateFrame(_ frame: CGRect) {
        metalView.frame = frame
        updateDrawableSize()
    }

    func setPlayback(paused: Bool, fpsCap: Int, adaptiveQuality: Bool) {
        metalView.preferredFramesPerSecond = fpsCap > 0
            ? fpsCap
            : min(60, max(1, Int(frameStore.header.fps.rounded())))
        metalView.isPaused = paused
        lastDrawTime = nil
    }

    func applyControlValues(_ values: [String: Double]) {
        if let value = values["speed"] { speed = Float(max(0, min(3, value))) }
    }

    func captureSnapshot(targetPixelSize: CGSize, completion: @escaping (NSImage?) -> Void) {
        let previousSize = metalView.drawableSize
        metalView.drawableSize = CGSize(width: max(1, targetPixelSize.width.rounded()),
                                        height: max(1, targetPixelSize.height.rounded()))
        metalView.draw()
        let image = currentImage()
        metalView.drawableSize = previousSize
        completion(image)
    }

    func tearDown() {
        metalView.isPaused = true
        metalView.delegate = nil
        metalView.removeFromSuperview()
    }

    func mtkView(_ view: MTKView, drawableSizeWillChange size: CGSize) {}

    func draw(in view: MTKView) {
        guard let drawable = view.currentDrawable,
              let pass = view.currentRenderPassDescriptor,
              let commandBuffer = commandQueue.makeCommandBuffer(),
              let encoder = commandBuffer.makeRenderCommandEncoder(descriptor: pass) else {
            return
        }
        let now = CACurrentMediaTime()
        if !view.isPaused, let lastDrawTime {
            animationTime += min(0.1, now - lastDrawTime) * Double(speed)
        }
        lastDrawTime = view.isPaused ? nil : now
        let frame = Int(animationTime * frameStore.header.fps) % frameStore.header.frameCount
        if frame != lastFrame {
            uploadFrame(frame)
            lastFrame = frame
        }

        var uniforms = Uniforms(columns: UInt32(frameStore.header.columns),
                                rows: UInt32(frameStore.header.rows),
                                viewport: SIMD2(Float(view.drawableSize.width),
                                                Float(view.drawableSize.height)))
        encoder.setRenderPipelineState(pipeline)
        encoder.setVertexBuffer(cellBuffers[activeCellBuffer], offset: 0, index: 0)
        encoder.setVertexBytes(&uniforms, length: MemoryLayout<Uniforms>.stride, index: 1)
        encoder.setFragmentTexture(fillTexture, index: 0)
        encoder.setFragmentTexture(edgeTexture, index: 1)
        encoder.drawPrimitives(type: .triangle, vertexStart: 0, vertexCount: 6,
                               instanceCount: frameStore.cellCount)
        encoder.endEncoding()
        commandBuffer.present(drawable)
        commandBuffer.commit()
        frameCount += 1
    }

    private func uploadFrame(_ index: Int) {
        let values = frameStore.expandedFrame(at: index)
        activeCellBuffer = (activeCellBuffer + 1) % cellBuffers.count
        let cellBuffer = cellBuffers[activeCellBuffer]
        values.withUnsafeBufferPointer { buffer in
            cellBuffer.contents().copyMemory(
                from: buffer.baseAddress!,
                byteCount: buffer.count * MemoryLayout<UInt32>.stride
            )
        }
    }

    private func updateDrawableSize() {
        let configured = UserDefaults.standard.double(forKey: DefaultsKey.renderScale)
        let scale = max(0.75, min(2, configured > 0 ? configured : 1.5))
        metalView.drawableSize = CGSize(width: max(1, metalView.bounds.width * scale),
                                        height: max(1, metalView.bounds.height * scale))
    }

    private func currentImage() -> NSImage? {
        guard let texture = metalView.currentDrawable?.texture else { return nil }
        let width = texture.width
        let height = texture.height
        let bytesPerRow = width * 4
        var data = Data(count: bytesPerRow * height)
        data.withUnsafeMutableBytes { buffer in
            texture.getBytes(buffer.baseAddress!, bytesPerRow: bytesPerRow,
                             from: MTLRegionMake2D(0, 0, width, height), mipmapLevel: 0)
        }
        guard let provider = CGDataProvider(data: data as CFData),
              let image = CGImage(width: width, height: height,
                                  bitsPerComponent: 8, bitsPerPixel: 32,
                                  bytesPerRow: bytesPerRow,
                                  space: CGColorSpace(name: CGColorSpace.sRGB)
                                    ?? CGColorSpaceCreateDeviceRGB(),
                                  bitmapInfo: CGBitmapInfo.byteOrder32Little.union(
                                    CGBitmapInfo(rawValue: CGImageAlphaInfo.premultipliedFirst.rawValue)),
                                  provider: provider, decode: nil,
                                  shouldInterpolate: false, intent: .defaultIntent) else {
            return nil
        }
        return NSImage(cgImage: image, size: NSSize(width: width, height: height))
    }
}
