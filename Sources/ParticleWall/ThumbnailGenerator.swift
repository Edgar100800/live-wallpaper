import AppKit
import WebKit

/// Renders a wallpaper in an offscreen WKWebView for ~2s and snapshots it
/// into <wallpaper>/thumbnail.png (640×400 backing, displayed smaller).
final class ThumbnailGenerator {
    static let shared = ThumbnailGenerator()

    private struct Job {
        let wallpaper: Wallpaper
        let completion: () -> Void
    }

    private var queue: [Job] = []
    private var running = false

    // Kept alive for the duration of a job.
    private var window: NSWindow?
    private var webView: WKWebView?
    private var navigationDelegate: LocalOnlyNavigationDelegate?

    private let size = NSSize(width: 640, height: 400)

    private init() {}

    func generate(for wallpaper: Wallpaper, completion: @escaping () -> Void = {}) {
        DispatchQueue.main.async {
            self.queue.append(Job(wallpaper: wallpaper, completion: completion))
            self.runNextIfIdle()
        }
    }

    private func runNextIfIdle() {
        guard !running, !queue.isEmpty else { return }
        running = true
        let job = queue.removeFirst()

        // WebKit suspends requestAnimationFrame in occluded or offscreen windows,
        // so the render window must be on-screen and unoccluded. Near-zero alpha
        // keeps it effectively invisible while WebKit still paints.
        let screenFrame = NSScreen.main?.frame ?? NSRect(x: 0, y: 0, width: 1440, height: 900)
        let origin = NSPoint(x: screenFrame.maxX - size.width, y: screenFrame.minY)
        let window = NSWindow(contentRect: NSRect(origin: origin, size: size),
                              styleMask: [.borderless],
                              backing: .buffered,
                              defer: false)
        window.isReleasedWhenClosed = false
        window.level = .floating
        window.collectionBehavior = [.canJoinAllSpaces, .stationary, .ignoresCycle]
        window.ignoresMouseEvents = true
        window.hasShadow = false
        window.alphaValue = 0.02

        let webView = WebViewFactory.makeWebView(frame: NSRect(origin: .zero, size: size))
        let delegate = LocalOnlyNavigationDelegate(allowedRoot: job.wallpaper.folderURL)
        webView.navigationDelegate = delegate
        window.contentView?.addSubview(webView)
        window.orderBack(nil)

        self.window = window
        self.webView = webView
        self.navigationDelegate = delegate

        webView.loadFileURL(job.wallpaper.indexURL, allowingReadAccessTo: job.wallpaper.folderURL)

        DispatchQueue.main.asyncAfter(deadline: .now() + 2.5) { [weak self] in
            self?.snapshot(job: job)
        }
    }

    private func snapshot(job: Job) {
        guard let webView else { finish(job: job); return }
        webView.evaluateJavaScript(
            "document.readyState + ' frames:' + (window.__pwFrameCount|0) + ' errors:' + JSON.stringify(window.__pwErrors || [])"
        ) { result, _ in
            NSLog("ParticleWall: thumbnail diag [\(job.wallpaper.name)]: \(result ?? "nil")")
        }
        let config = WKSnapshotConfiguration()
        config.rect = CGRect(origin: .zero, size: size)
        webView.takeSnapshot(with: config) { [weak self] image, error in
            if let image, let data = Self.pngData(from: image) {
                try? data.write(to: job.wallpaper.thumbnailURL)
            } else if let error {
                NSLog("ParticleWall: thumbnail snapshot failed: \(error)")
            }
            self?.finish(job: job)
        }
    }

    private func finish(job: Job) {
        webView?.removeFromSuperview()
        window?.orderOut(nil)
        webView = nil
        window = nil
        navigationDelegate = nil
        running = false
        job.completion()
        runNextIfIdle()
    }

    private static func pngData(from image: NSImage) -> Data? {
        guard let tiff = image.tiffRepresentation,
              let rep = NSBitmapImageRep(data: tiff) else { return nil }
        return rep.representation(using: .png, properties: [:])
    }
}
