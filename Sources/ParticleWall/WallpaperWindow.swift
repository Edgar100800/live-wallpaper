import AppKit
import WebKit

/// Borderless window pinned at desktop level: behind Finder icons,
/// in front of the system wallpaper. Clicks pass through.
final class WallpaperWindow: NSWindow {
    init(screen: NSScreen) {
        super.init(contentRect: screen.frame,
                   styleMask: [.borderless],
                   backing: .buffered,
                   defer: false)
        level = NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.desktopWindow)))
        collectionBehavior = [.canJoinAllSpaces, .stationary, .ignoresCycle]
        isOpaque = true
        backgroundColor = .black
        ignoresMouseEvents = true
        hasShadow = false
        isReleasedWhenClosed = false
        animationBehavior = .none
    }

    override var canBecomeKey: Bool { false }
    override var canBecomeMain: Bool { false }
}

/// One controller per screen. Owns the window + WKWebView and its playback state.
final class WallpaperWindowController: NSObject {
    let window: WallpaperWindow
    let displayUUID: String
    private(set) var webView: WKWebView
    private var navigationDelegate: LocalOnlyNavigationDelegate?
    private(set) var currentWallpaperID: UUID?
    private var currentIndexURL: URL?
    private var currentRootURL: URL?

    /// Set by PowerManager (global) and occlusion (local); effective pause is the OR.
    var globallyPaused = false { didSet { pushPlaybackState() } }
    private var occluded = false { didSet { if oldValue != occluded { pushPlaybackState() } } }
    var fpsCap: Int = 0 { didSet { if oldValue != fpsCap { pushPlaybackState() } } }
    /// Per-wallpaper cap from manifest.json; effective cap is the lowest non-zero.
    var manifestFPS: Int = 0 { didSet { if oldValue != manifestFPS { pushPlaybackState() } } }

    init(screen: NSScreen, displayUUID: String) {
        self.window = WallpaperWindow(screen: screen)
        self.displayUUID = displayUUID
        self.webView = WebViewFactory.makeWebView(frame: window.contentView?.bounds ?? .zero)
        super.init()
        window.contentView?.addSubview(webView)
        NotificationCenter.default.addObserver(self,
                                               selector: #selector(occlusionChanged),
                                               name: NSWindow.didChangeOcclusionStateNotification,
                                               object: window)
    }

    deinit {
        NotificationCenter.default.removeObserver(self)
    }

    func show() {
        window.orderBack(nil)
    }

    func updateFrame(for screen: NSScreen) {
        window.setFrame(screen.frame, display: true)
        webView.frame = window.contentView?.bounds ?? .zero
    }

    func load(indexURL: URL, rootURL: URL, wallpaperID: UUID?) {
        // Applying a wallpaper wakes a deep-asleep controller; PowerManager
        // re-asserts deep sleep afterwards if Power Save is still on.
        if isDeepAsleep {
            isDeepAsleep = false
            sleepImageView?.removeFromSuperview()
            sleepImageView = nil
        }
        currentWallpaperID = wallpaperID
        currentIndexURL = indexURL
        currentRootURL = rootURL
        let delegate = LocalOnlyNavigationDelegate(allowedRoot: rootURL)
        // A fresh document resets __pwPaused/__pwFPSCap to defaults; re-push once loaded.
        delegate.onDidFinish = { [weak self] in self?.pushPlaybackState() }
        navigationDelegate = delegate
        webView.navigationDelegate = delegate
        webView.loadFileURL(indexURL, allowingReadAccessTo: rootURL)
    }

    /// Rebuild the WKWebView (new user scripts, e.g. after a render-scale change)
    /// and reload the current wallpaper.
    func recreateWebView() {
        webView.removeFromSuperview()
        webView = WebViewFactory.makeWebView(frame: window.contentView?.bounds ?? .zero)
        window.contentView?.addSubview(webView)
        if let indexURL = currentIndexURL, let rootURL = currentRootURL {
            load(indexURL: indexURL, rootURL: rootURL, wallpaperID: currentWallpaperID)
        }
    }

    // MARK: - Deep sleep (Power Save)

    /// Power Save: freeze the last frame in a plain NSImageView and tear the
    /// WKWebView down entirely — its WebContent/GPU work drops to zero.
    private var sleepImageView: NSImageView?
    private(set) var isDeepAsleep = false

    func enterDeepSleep() {
        guard !isDeepAsleep else { return }
        isDeepAsleep = true
        let config = WKSnapshotConfiguration()
        webView.takeSnapshot(with: config) { [weak self] image, _ in
            guard let self, self.isDeepAsleep else { return }
            let imageView = NSImageView(frame: self.window.contentView?.bounds ?? .zero)
            imageView.autoresizingMask = [.width, .height]
            imageView.imageScaling = .scaleAxesIndependently
            imageView.image = image   // nil leaves the black window background
            self.window.contentView?.addSubview(imageView)
            self.sleepImageView = imageView
            self.webView.removeFromSuperview()
            self.webView = WebViewFactory.makeWebView(frame: .zero) // detached placeholder
        }
    }

    func exitDeepSleep() {
        guard isDeepAsleep else { return }
        isDeepAsleep = false
        recreateWebView()
        // Keep the frozen frame visible until the reloaded wallpaper paints.
        if let delegate = navigationDelegate {
            let previousFinish = delegate.onDidFinish
            delegate.onDidFinish = { [weak self] in
                previousFinish?()
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) {
                    self?.sleepImageView?.removeFromSuperview()
                    self?.sleepImageView = nil
                }
            }
        } else {
            sleepImageView?.removeFromSuperview()
            sleepImageView = nil
        }
    }

    func clear() {
        currentWallpaperID = nil
        currentIndexURL = nil
        currentRootURL = nil
        webView.loadHTMLString("<html><body style='background:#000'></body></html>", baseURL: nil)
    }

    @objc private func occlusionChanged() {
        occluded = !window.occlusionState.contains(.visible)
    }

    private var effectivePaused: Bool { globallyPaused || occluded }

    private var effectiveFPSCap: Int {
        let caps = [fpsCap, manifestFPS].filter { $0 > 0 }
        return caps.min() ?? 0
    }

    func pushPlaybackState() {
        guard !isDeepAsleep else { return } // no webview to talk to; would respawn WebContent
        let js = "window.__pwPaused = \(effectivePaused ? "true" : "false"); window.__pwFPSCap = \(effectiveFPSCap);"
        webView.evaluateJavaScript(js, completionHandler: nil)
    }

    /// Kick after session unlock: some WebKit renders stay frozen. Re-push state;
    /// if the JS context is gone, reload as fallback.
    func kickAfterUnlock() {
        guard !isDeepAsleep else { return }
        webView.evaluateJavaScript("window.__pwInstalled === true") { [weak self] result, error in
            guard let self else { return }
            if error != nil || (result as? Bool) != true {
                self.reload()
            } else {
                self.pushPlaybackState()
            }
        }
    }

    func reload() {
        if let indexURL = currentIndexURL, let rootURL = currentRootURL {
            webView.loadFileURL(indexURL, allowingReadAccessTo: rootURL)
        } else {
            webView.reload()
        }
    }
}
