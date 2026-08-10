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

/// One controller per screen. Owns the window + selected renderer and playback state.
final class WallpaperWindowController: NSObject {
    let window: WallpaperWindow
    let displayUUID: String
    private(set) var snapshotPixelSize: CGSize
    /// Created only while a wallpaper is actively rendering. Keeping this
    /// optional lets deep sleep release WebKit instead of retaining a detached
    /// placeholder view.
    private var renderer: WallpaperRenderer?
    var webView: WKWebView? { (renderer as? WebWallpaperRenderer)?.webView }
    private var navigationDelegate: LocalOnlyNavigationDelegate?
    private(set) var currentWallpaperID: UUID?
    private var currentIndexURL: URL?
    private var currentRootURL: URL?
    private var currentRendererKind: WallpaperRendererKind = .web
    private var controlValues: [String: Double] = [:]
    private var contentReady = false
    private var pendingControlRequests: [([WallpaperControlDescriptor]) -> Void] = []

    /// Set by PowerManager (global) and occlusion (local); effective pause is the OR.
    var globallyPaused = false { didSet { pushPlaybackState() } }
    private var occluded = false { didSet { if oldValue != occluded { pushPlaybackState() } } }
    var fpsCap: Int = 0 { didSet { if oldValue != fpsCap { pushPlaybackState() } } }
    /// Per-wallpaper cap from manifest.json; effective cap is the lowest non-zero.
    var manifestFPS: Int = 0 { didSet { if oldValue != manifestFPS { pushPlaybackState() } } }

    init(screen: NSScreen, displayUUID: String) {
        self.window = WallpaperWindow(screen: screen)
        self.displayUUID = displayUUID
        self.snapshotPixelSize = Self.pixelSize(for: screen)
        super.init()
        NotificationCenter.default.addObserver(self,
                                               selector: #selector(occlusionChanged),
                                               name: NSWindow.didChangeOcclusionStateNotification,
                                               object: window)
    }

    deinit {
        NotificationCenter.default.removeObserver(self)
        tearDownRenderer()
    }

    func show() {
        window.orderBack(nil)
    }

    func updateFrame(for screen: NSScreen) {
        window.setFrame(screen.frame, display: true)
        snapshotPixelSize = Self.pixelSize(for: screen)
        renderer?.updateFrame(window.contentView?.bounds ?? .zero)
    }

    func load(indexURL: URL,
              rootURL: URL,
              wallpaperID: UUID?,
              rendererKind: WallpaperRendererKind = .web,
              controlValues: [String: Double]? = nil) {
        finishPendingControlRequests(with: [])
        if currentRendererKind != rendererKind {
            tearDownRenderer()
        }
        currentWallpaperID = wallpaperID
        currentIndexURL = indexURL
        currentRootURL = rootURL
        currentRendererKind = rendererKind
        if let controlValues { self.controlValues = controlValues }
        contentReady = false

        // Applying while asleep updates the pending wallpaper and frozen image,
        // but must not briefly respawn WebContent just to tear it down again.
        guard !isDeepAsleep else {
            installSleepImage(fallbackSleepImage())
            return
        }

        if rendererKind.isNativeMetal, makeMetalRendererIfNeeded(kind: rendererKind) {
            contentReady = true
            pushPlaybackState()
            pushControlValues()
            flushPendingControlRequests()
            return
        }

        let webView = makeWebViewIfNeeded()
        let delegate = LocalOnlyNavigationDelegate(allowedRoot: rootURL)
        // A fresh document resets __pwPaused/__pwFPSCap to defaults; re-push once loaded.
        delegate.onDidFinish = { [weak self] in
            guard let self else { return }
            self.contentReady = true
            self.pushPlaybackState()
            self.pushControlValues()
            self.flushPendingControlRequests()
        }
        navigationDelegate = delegate
        webView.navigationDelegate = delegate
        webView.loadFileURL(indexURL, allowingReadAccessTo: rootURL)
    }

    /// Rebuild the WKWebView (new user scripts, e.g. after a render-scale change)
    /// and reload the current wallpaper.
    func recreateWebView() {
        guard !isDeepAsleep, currentRendererKind == .web else { return }
        tearDownRenderer()
        if let indexURL = currentIndexURL, let rootURL = currentRootURL {
            load(indexURL: indexURL, rootURL: rootURL, wallpaperID: currentWallpaperID,
                 rendererKind: currentRendererKind)
        }
    }

    func refreshRenderScale() {
        if currentRendererKind == .web {
            recreateWebView()
        } else {
            renderer?.updateFrame(window.contentView?.bounds ?? .zero)
        }
    }

    // MARK: - Deep sleep (Power Save)

    /// Power Save: freeze the last frame in a plain NSImageView and tear the
    /// WKWebView down entirely — its WebContent/GPU work drops to zero.
    private var sleepImageView: NSImageView?
    private var sleepGeneration = 0
    private(set) var isDeepAsleep = false

    func enterDeepSleep(preservingFrame: Bool = true,
                        fallbackImage: NSImage? = nil,
                        onSnapshot: ((NSImage) -> Void)? = nil) {
        if isDeepAsleep {
            // Escalating from a visible Power Save snapshot to lock/screen sleep
            // must not wait for the pending snapshot timeout.
            if !preservingFrame, renderer != nil {
                sleepGeneration += 1
                installSleepImage(fallbackImage ?? fallbackSleepImage())
                tearDownRenderer()
            }
            return
        }
        isDeepAsleep = true
        sleepGeneration += 1
        let generation = sleepGeneration

        guard let renderer else {
            installSleepImage(fallbackImage ?? fallbackSleepImage())
            return
        }

        // Lock/screen sleep do not need an exact last frame. Releasing WebKit
        // immediately is more important and the thumbnail prevents a black flash.
        guard preservingFrame else {
            installSleepImage(fallbackImage ?? fallbackSleepImage())
            tearDownRenderer()
            return
        }

        installSleepImage(fallbackImage ?? fallbackSleepImage())
        let rendererID = ObjectIdentifier(renderer)
        renderer.captureSnapshot(targetPixelSize: snapshotPixelSize) { [weak self] image in
            guard let self,
                  self.isDeepAsleep,
                  self.sleepGeneration == generation,
                  self.renderer.map(ObjectIdentifier.init) == rendererID else { return }
            if let image {
                onSnapshot?(image)
                self.installSleepImage(image)
            }
            self.tearDownRenderer()
        }

        // A suspended renderer may never answer a snapshot request. Keep the
        // cached/thumbnail image and bound teardown time.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.75) { [weak self] in
            guard let self,
                  self.isDeepAsleep,
                  self.sleepGeneration == generation,
                  self.renderer.map(ObjectIdentifier.init) == rendererID else { return }
            self.tearDownRenderer()
        }
    }

    func captureSnapshot(completion: @escaping (NSImage?) -> Void) {
        guard !isDeepAsleep, let renderer, currentWallpaperID != nil else {
            completion(nil)
            return
        }
        let rendererID = ObjectIdentifier(renderer)
        renderer.captureSnapshot(targetPixelSize: snapshotPixelSize) { [weak self] image in
            guard let self,
                  self.renderer.map(ObjectIdentifier.init) == rendererID else {
                completion(nil)
                return
            }
            completion(image)
        }
    }

    func exitDeepSleep() {
        guard isDeepAsleep else { return }
        isDeepAsleep = false
        sleepGeneration += 1

        guard let indexURL = currentIndexURL, let rootURL = currentRootURL else {
            removeSleepImage()
            return
        }

        load(indexURL: indexURL, rootURL: rootURL, wallpaperID: currentWallpaperID,
             rendererKind: currentRendererKind)
        // Keep the frozen frame visible until the reloaded wallpaper paints.
        if let delegate = navigationDelegate, sleepImageView != nil {
            let previousFinish = delegate.onDidFinish
            delegate.onDidFinish = { [weak self] in
                previousFinish?()
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.3) {
                    self?.removeSleepImage()
                }
            }
        } else {
            removeSleepImage()
        }
    }

    func clear() {
        currentWallpaperID = nil
        currentIndexURL = nil
        currentRootURL = nil
        sleepGeneration += 1
        controlValues = [:]
        removeSleepImage()
        tearDownRenderer()
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
        guard !isDeepAsleep, let renderer else { return }
        let adaptive = UserDefaults.standard.object(forKey: DefaultsKey.adaptiveQuality) as? Bool ?? true
        renderer.setPlayback(paused: effectivePaused,
                             fpsCap: effectiveFPSCap,
                             adaptiveQuality: adaptive)
    }

    func applyControlValues(_ values: [String: Double]) {
        controlValues = values
        renderer?.applyControlValues(values)
        pushControlValues()
    }

    func fetchControlDescriptors(completion: @escaping ([WallpaperControlDescriptor]) -> Void) {
        if let renderer, renderer.kind.isNativeMetal {
            completion(renderer.controlDescriptors)
            return
        }
        guard !isDeepAsleep, webView != nil else {
            completion([])
            return
        }
        guard contentReady else {
            pendingControlRequests.append(completion)
            return
        }
        fetchReadyControlDescriptors(completion: completion)
    }

    /// Kick after session unlock: some WebKit renders stay frozen. Re-push state;
    /// if the JS context is gone, reload as fallback.
    func kickAfterUnlock() {
        guard !isDeepAsleep, let webView else { return }
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
        guard !isDeepAsleep,
              let indexURL = currentIndexURL,
              let rootURL = currentRootURL else { return }
        if let webView {
            contentReady = false
            webView.loadFileURL(indexURL, allowingReadAccessTo: rootURL)
        } else {
            load(indexURL: indexURL, rootURL: rootURL, wallpaperID: currentWallpaperID,
                 rendererKind: currentRendererKind)
        }
    }

    // MARK: - WebKit lifecycle

    private func makeWebViewIfNeeded() -> WKWebView {
        if let webView { return webView }

        tearDownRenderer()
        let webRenderer = WebWallpaperRenderer(frame: window.contentView?.bounds ?? .zero)
        let webView = webRenderer.webView
        if let sleepImageView {
            window.contentView?.addSubview(webView, positioned: .below, relativeTo: sleepImageView)
        } else {
            window.contentView?.addSubview(webView)
        }
        renderer = webRenderer
        return webView
    }

    @discardableResult
    private func makeMetalRendererIfNeeded(kind: WallpaperRendererKind) -> Bool {
        if let renderer = renderer as? MetalParticleRenderer, renderer.kind == kind {
            renderer.applyControlValues(controlValues)
            return true
        }
        tearDownRenderer()
        guard let metal = MetalParticleRenderer(frame: window.contentView?.bounds ?? .zero,
                                                kind: kind) else {
            NSLog("ParticleWall: Metal unavailable; falling back to WebKit")
            currentRendererKind = .web
            return false
        }
        if let sleepImageView {
            window.contentView?.addSubview(metal.view, positioned: .below, relativeTo: sleepImageView)
        } else {
            window.contentView?.addSubview(metal.view)
        }
        renderer = metal
        metal.applyControlValues(controlValues)
        return true
    }

    private func tearDownRenderer() {
        guard let renderer else { return }
        renderer.tearDown()
        navigationDelegate?.onDidFinish = nil
        navigationDelegate = nil
        self.renderer = nil
        contentReady = false
        finishPendingControlRequests(with: [])
    }

    var diagnosticSummary: String {
        renderer?.diagnosticSummary ?? "renderer:none"
    }

    private func pushControlValues() {
        guard contentReady, !isDeepAsleep, let webView,
              JSONSerialization.isValidJSONObject(controlValues),
              let data = try? JSONSerialization.data(withJSONObject: controlValues, options: [.sortedKeys]),
              let json = String(data: data, encoding: .utf8) else { return }
        webView.evaluateJavaScript("window.__pwApplySettings && window.__pwApplySettings(\(json));",
                                   completionHandler: nil)
    }

    private func flushPendingControlRequests() {
        let requests = pendingControlRequests
        pendingControlRequests.removeAll()
        for request in requests {
            fetchReadyControlDescriptors(completion: request)
        }
    }

    private func finishPendingControlRequests(with descriptors: [WallpaperControlDescriptor]) {
        let requests = pendingControlRequests
        pendingControlRequests.removeAll()
        requests.forEach { $0(descriptors) }
    }

    private func fetchReadyControlDescriptors(
        attempt: Int = 0,
        completion: @escaping ([WallpaperControlDescriptor]) -> Void
    ) {
        guard let webView else {
            completion([])
            return
        }
        let script = "window.__pwGetControls ? window.__pwGetControls() : []"
        webView.evaluateJavaScript(script) { [weak self, weak webView] result, error in
            guard let self, let webView, self.webView === webView else {
                completion([])
                return
            }

            var descriptors: [WallpaperControlDescriptor] = []
            if let result,
               JSONSerialization.isValidJSONObject(result),
               let data = try? JSONSerialization.data(withJSONObject: result),
               let decoded = try? JSONDecoder().decode([WallpaperControlDescriptor].self,
                                                       from: data) {
                descriptors = decoded
            }

            // Module scripts can finish shortly after WKNavigationDelegate's
            // didFinish callback. Retry briefly instead of reporting a false
            // incompatibility while __pwGetControls is being installed.
            if descriptors.isEmpty, error == nil, attempt < 10 {
                DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) { [weak self] in
                    self?.fetchReadyControlDescriptors(attempt: attempt + 1,
                                                       completion: completion)
                }
                return
            }

            if let error {
                NSLog("ParticleWall: control bridge query failed: \(error)")
            } else if descriptors.isEmpty {
                webView.evaluateJavaScript("window.__pwErrors || []") { errors, _ in
                    NSLog("ParticleWall: control bridge exposed no descriptors; JS errors: \(String(describing: errors))")
                }
            }
            completion(descriptors)
        }
    }

    private func fallbackSleepImage() -> NSImage? {
        guard let currentWallpaperID,
              let wallpaper = LibraryManager.shared.wallpaper(id: currentWallpaperID) else { return nil }
        return NSImage(contentsOf: wallpaper.thumbnailURL)
    }

    private func installSleepImage(_ image: NSImage?) {
        removeSleepImage()
        guard let image else { return }

        let imageView = NSImageView(frame: window.contentView?.bounds ?? .zero)
        imageView.autoresizingMask = [.width, .height]
        imageView.imageScaling = .scaleAxesIndependently
        imageView.image = image
        window.contentView?.addSubview(imageView)
        sleepImageView = imageView
    }

    private func removeSleepImage() {
        sleepImageView?.removeFromSuperview()
        sleepImageView = nil
    }

    private static func pixelSize(for screen: NSScreen) -> CGSize {
        if let number = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")]
            as? NSNumber {
            let displayID = CGDirectDisplayID(number.uint32Value)
            let width = CGDisplayPixelsWide(displayID)
            let height = CGDisplayPixelsHigh(displayID)
            if width > 0, height > 0 {
                return CGSize(width: width, height: height)
            }
        }
        return CGSize(width: screen.frame.width * screen.backingScaleFactor,
                      height: screen.frame.height * screen.backingScaleFactor)
    }
}
