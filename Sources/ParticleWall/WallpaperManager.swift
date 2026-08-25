import AppKit

/// Owns one WallpaperWindowController per connected screen, reacts to screen
/// changes, applies wallpapers and persists the assignment per display.
final class WallpaperManager {
    static let shared = WallpaperManager()

    private(set) var controllers: [String: WallpaperWindowController] = [:] // displayUUID -> controller
    private let defaults = UserDefaults.standard
    private let lastFrames = LastFrameStore.shared
    private var lastRenderScale: Double = 0
    private var snapshotTimer: Timer?
    private var snapshotCaptureTokens: [String: UUID] = [:]
    private var persistedSnapshotKeys: Set<LastFrameStore.Key> = []
    /// Coalesces desktop-picture refreshes during live color edits so macOS
    /// Space transitions / menu-bar tint settle on the latest background once.
    private var desktopResyncWorkItem: DispatchWorkItem?
    private let desktopResyncDelay: TimeInterval = 0.8

    private init() {}

    func start() {
        NotificationCenter.default.addObserver(self,
                                               selector: #selector(screensChanged),
                                               name: NSApplication.didChangeScreenParametersNotification,
                                               object: nil)
        lastRenderScale = defaults.double(forKey: DefaultsKey.renderScale)
        NotificationCenter.default.addObserver(self,
                                               selector: #selector(defaultsChanged),
                                               name: UserDefaults.didChangeNotification,
                                               object: nil)
        rebuildControllers()
        startSnapshotTimer()
    }

    /// Render scale is baked into each webview's user scripts at creation,
    /// so a change requires rebuilding them.
    @objc private func defaultsChanged() {
        let scale = defaults.double(forKey: DefaultsKey.renderScale)
        guard scale != lastRenderScale else { return }
        lastRenderScale = scale
        for controller in controllers.values {
            controller.refreshRenderScale()
        }
    }

    // MARK: - Screens

    static func displayUUID(for screen: NSScreen) -> String? {
        guard let number = screen.deviceDescription[NSDeviceDescriptionKey("NSScreenNumber")] as? NSNumber else {
            return nil
        }
        let displayID = CGDirectDisplayID(number.uint32Value)
        guard let uuidRef = CGDisplayCreateUUIDFromDisplayID(displayID)?.takeRetainedValue() else {
            return nil
        }
        return CFUUIDCreateString(nil, uuidRef) as String
    }

    var screensByUUID: [(uuid: String, screen: NSScreen)] {
        NSScreen.screens.compactMap { screen in
            guard let uuid = Self.displayUUID(for: screen) else { return nil }
            return (uuid, screen)
        }
    }

    @objc private func screensChanged() {
        rebuildControllers()
    }

    private func rebuildControllers() {
        let current = screensByUUID
        let currentUUIDs = Set(current.map(\.uuid))
        var addedUUIDs: [String] = []

        // Drop controllers for disconnected screens.
        for (uuid, controller) in controllers where !currentUUIDs.contains(uuid) {
            controller.window.orderOut(nil)
            controllers.removeValue(forKey: uuid)
        }

        // Create/update controllers for connected screens.
        for (uuid, screen) in current {
            if let controller = controllers[uuid] {
                controller.updateFrame(for: screen)
            } else {
                let controller = WallpaperWindowController(screen: screen, displayUUID: uuid)
                controllers[uuid] = controller
                controller.show()
                addedUUIDs.append(uuid)
            }
        }

        // Apply lock/sleep/Power Save before loading content into new screens.
        // A deep-asleep controller then records its assignment without creating
        // a short-lived WebContent process.
        PowerManager.shared.pushStateToAllControllers()
        for uuid in addedUUIDs {
            restoreAssignment(for: uuid)
        }
    }

    // MARK: - Applying wallpapers

    func apply(_ wallpaper: Wallpaper, to target: ScreenTarget) {
        switch target {
        case .allScreens:
            for (uuid, controller) in controllers {
                load(wallpaper, into: controller)
                syncSystemWallpaper(for: uuid, wallpaperID: wallpaper.id)
            }
            // Displays the app cannot host a wallpaper window on still need
            // their system picture updated, or they keep the old static color.
            for entry in screensByUUID where controllers[entry.uuid] == nil {
                syncSystemWallpaper(for: entry.uuid, wallpaperID: wallpaper.id)
            }
            defaults.set(wallpaper.id.uuidString, forKey: DefaultsKey.defaultWallpaper)
            var map = assignmentMap()
            for uuid in controllers.keys { map[uuid] = wallpaper.id.uuidString }
            defaults.set(map, forKey: DefaultsKey.activeWallpapers)
        case .screen(let displayUUID):
            guard let controller = controllers[displayUUID] else { return }
            load(wallpaper, into: controller)
            syncSystemWallpaper(for: displayUUID)
            var map = assignmentMap()
            map[displayUUID] = wallpaper.id.uuidString
            defaults.set(map, forKey: DefaultsKey.activeWallpapers)
        }
        // load() wakes deep-asleep controllers; re-assert Power Save if active.
        PowerManager.shared.pushStateToAllControllers()
        NotificationCenter.default.post(name: .pwPlaybackStateChanged, object: nil)

        // The system wallpaper (menu-bar tint, Space transitions, Mission
        // Control) must not keep the previous static color: once the new
        // renderer paints, point it at a fresh capture of the applied wallpaper.
        scheduleDesktopResync(for: wallpaper.id, target: target)
    }

    private func load(_ wallpaper: Wallpaper, into controller: WallpaperWindowController) {
        controller.manifestFPS = wallpaper.manifest.fps ?? 0
        let controls = WallpaperControlStore.shared.values(for: wallpaper.id,
                                                           displayUUID: controller.displayUUID)
        controller.load(indexURL: wallpaper.indexURL,
                        rootURL: wallpaper.folderURL,
                        wallpaperID: wallpaper.id,
                        rendererKind: wallpaper.manifest.effectiveRenderer,
                        controlValues: controls)
    }

    // MARK: - System wallpaper sync

    /// The macOS menu bar (and Space transitions) derive their tint from the
    /// SYSTEM desktop picture, not from our desktop-level window. Point it at
    /// the active wallpaper's most recent frame so no stale colors bleed
    /// through. `wallpaperID` lets us sync displays the app has no controller
    /// for (no wallpaper window) — their static picture must still follow the
    /// applied wallpaper.
    private func syncSystemWallpaper(for displayUUID: String, wallpaperID: UUID? = nil) {
        guard let entry = screensByUUID.first(where: { $0.uuid == displayUUID }),
              let id = wallpaperID ?? controllers[displayUUID]?.currentWallpaperID,
              let wallpaper = LibraryManager.shared.wallpaper(id: id) else { return }
        // A solid rendition of the wallpaper's palette drives the menu-bar tint:
        // a captured frame has particles/animations at its top, which would tint
        // the bar with something other than the background color.
        if let colorURL = systemColorImageURL(for: id, displayUUID: displayUUID) {
            setSystemWallpaper(colorURL, for: entry.screen)
            return
        }
        let persisted = lastFrames.persistedURL(displayUUID: displayUUID, wallpaperID: id)
        let imageURL: URL
        if FileManager.default.fileExists(atPath: persisted.path) {
            imageURL = persisted
        } else if FileManager.default.fileExists(atPath: wallpaper.thumbnailURL.path) {
            imageURL = wallpaper.thumbnailURL
        } else if let black = Self.blackFallbackImage() {
            imageURL = black
        } else {
            return
        }
        setSystemWallpaper(imageURL, for: entry.screen)
    }

    /// Writes a small PNG of the wallpaper's background color (+ particle glow)
    /// so the system desktop picture — and therefore the menu bar it tints —
    /// matches the on-screen background.
    private func systemColorImageURL(for wallpaperID: UUID, displayUUID: String) -> URL? {
        let values = controlValues(for: wallpaperID, target: .screen(displayUUID: displayUUID))
        guard let image = WallpaperWindowController.colorGlowImage(from: values) else { return nil }
        let url = LibraryManager.shared.rootURL
            .appendingPathComponent("color-\(displayUUID)-\(wallpaperID.uuidString).png")
        guard let tiff = image.tiffRepresentation,
              let rep = NSBitmapImageRep(data: tiff),
              let png = rep.representation(using: .png, properties: [:]) else { return nil }
        try? png.write(to: url, options: .atomic)
        return FileManager.default.fileExists(atPath: url.path) ? url : nil
    }

    private func setSystemWallpaper(_ imageURL: URL, for screen: NSScreen) {
        let options: [NSWorkspace.DesktopImageOptionKey: Any] = [
            .imageScaling: NSImageScaling.scaleAxesIndependently.rawValue,
            .allowClipping: true
        ]
        do {
            try NSWorkspace.shared.setDesktopImageURL(imageURL, for: screen, options: options)
        } catch {
            NSLog("ParticleWall: could not set system wallpaper: \(error)")
        }
    }

    /// Live-update the per-wallpaper FPS cap on screens showing this wallpaper.
    func updateManifestFPS(for wallpaperID: UUID, fps: Int?) {
        for controller in controllers.values where controller.currentWallpaperID == wallpaperID {
            controller.manifestFPS = fps ?? 0
        }
    }

    /// Re-sync after a thumbnail lands for a wallpaper that is on screen.
    func refreshSystemWallpaper(for wallpaperID: UUID) {
        for (uuid, controller) in controllers where controller.currentWallpaperID == wallpaperID {
            syncSystemWallpaper(for: uuid)
        }
    }

    private static func blackFallbackImage() -> URL? {
        let url = LibraryManager.shared.rootURL.appendingPathComponent("black.png")
        if FileManager.default.fileExists(atPath: url.path) { return url }
        let size = NSSize(width: 64, height: 64)
        let image = NSImage(size: size)
        image.lockFocus()
        NSColor.black.setFill()
        NSRect(origin: .zero, size: size).fill()
        image.unlockFocus()
        guard let tiff = image.tiffRepresentation,
              let rep = NSBitmapImageRep(data: tiff),
              let data = rep.representation(using: .png, properties: [:]) else { return nil }
        try? data.write(to: url)
        return FileManager.default.fileExists(atPath: url.path) ? url : nil
    }

    /// Called when a wallpaper is deleted from the library.
    func wallpaperRemoved(_ id: UUID) {
        var map = assignmentMap()
        for (uuid, controller) in controllers where controller.currentWallpaperID == id {
            controller.clear()
            map.removeValue(forKey: uuid)
        }
        defaults.set(map, forKey: DefaultsKey.activeWallpapers)
        if defaults.string(forKey: DefaultsKey.defaultWallpaper) == id.uuidString {
            defaults.removeObject(forKey: DefaultsKey.defaultWallpaper)
        }
        WallpaperControlStore.shared.removeValues(for: id)
        lastFrames.remove(wallpaperID: id)
    }

    /// Which wallpaper is active on a given screen (for gallery highlight).
    func activeWallpaperID(on target: ScreenTarget) -> UUID? {
        switch target {
        case .allScreens:
            let ids = Set(controllers.values.compactMap(\.currentWallpaperID))
            return ids.count == 1 ? ids.first : nil
        case .screen(let uuid):
            return controllers[uuid]?.currentWallpaperID
        }
    }

    // MARK: - Wallpaper controls

    func controlValues(for wallpaperID: UUID, target: ScreenTarget) -> [String: Double] {
        switch target {
        case .screen(let displayUUID):
            return WallpaperControlStore.shared.values(for: wallpaperID,
                                                       displayUUID: displayUUID)
        case .allScreens:
            let defaults = WallpaperControlStore.shared.defaultValues(for: wallpaperID)
            if !defaults.isEmpty { return defaults }
            guard let uuid = controllers.keys.sorted().first else { return [:] }
            return WallpaperControlStore.shared.values(for: wallpaperID, displayUUID: uuid)
        }
    }

    func setControlValues(_ values: [String: Double],
                          for wallpaperID: UUID,
                          target: ScreenTarget) {
        switch target {
        case .screen(let displayUUID):
            WallpaperControlStore.shared.setValues(values,
                                                   for: wallpaperID,
                                                   displayUUID: displayUUID)
        case .allScreens:
            WallpaperControlStore.shared.setDefaultValues(values, for: wallpaperID)
            for uuid in controllers.keys {
                WallpaperControlStore.shared.setValues(values,
                                                       for: wallpaperID,
                                                       displayUUID: uuid)
            }
        }
        previewControlValues(values, for: wallpaperID, target: target)
        scheduleDesktopResync(for: wallpaperID, target: target)
    }

    // MARK: - Desktop picture refresh (Space transitions / menu-bar tint)

    private func scheduleDesktopResync(for wallpaperID: UUID, target: ScreenTarget) {
        desktopResyncWorkItem?.cancel()
        let workItem = DispatchWorkItem { [weak self] in
            self?.resyncDesktopWallpaper(for: wallpaperID, target: target)
            // Keep the gallery card in sync with the latest saved palette.
            self?.refreshGalleryThumbnail(for: wallpaperID)
        }
        desktopResyncWorkItem = workItem
        DispatchQueue.main.asyncAfter(deadline: .now() + desktopResyncDelay,
                                      execute: workItem)
    }

    private func refreshGalleryThumbnail(for wallpaperID: UUID) {
        guard let wallpaper = LibraryManager.shared.wallpaper(id: wallpaperID) else { return }
        LibraryManager.shared.regenerateThumbnail(wallpaper)
    }

    private func resyncDesktopWallpaper(for wallpaperID: UUID, target: ScreenTarget) {
        switch target {
        case .screen(let displayUUID):
            resyncDisplay(displayUUID, wallpaperID: wallpaperID)
        case .allScreens:
            for (displayUUID, controller) in controllers
                where controller.currentWallpaperID == wallpaperID {
                resyncDisplay(displayUUID, wallpaperID: wallpaperID)
            }
        }
    }

    private func resyncDisplay(_ displayUUID: String, wallpaperID: UUID) {
        guard let controller = controllers[displayUUID],
              controller.currentWallpaperID == wallpaperID else { return }

        // Power Save keeps the renderer torn down, so there is no live frame to
        // capture. Push a solid rendition of the current background instead.
        if controller.isDeepAsleep {
            let values = controlValues(for: wallpaperID,
                                       target: .screen(displayUUID: displayUUID))
            if let image = controller.refreshFrozenBackground(from: values) {
                lastFrames.cache(image,
                                 displayUUID: displayUUID,
                                 wallpaperID: wallpaperID)
                _ = lastFrames.persist(image,
                                       displayUUID: displayUUID,
                                       wallpaperID: wallpaperID,
                                       targetPixelSize: controller.snapshotPixelSize)
            }
            syncSystemWallpaper(for: displayUUID)
            return
        }

        controller.captureSnapshot { [weak self, weak controller] image in
            guard let self, let controller,
                  controller.currentWallpaperID == wallpaperID else { return }
            if let image {
                self.lastFrames.cache(image,
                                      displayUUID: displayUUID,
                                      wallpaperID: wallpaperID)
                _ = self.lastFrames.persist(image,
                                            displayUUID: displayUUID,
                                            wallpaperID: wallpaperID,
                                            targetPixelSize: controller.snapshotPixelSize)
            }
            self.syncSystemWallpaper(for: displayUUID)
        }
    }

    /// Restores the renderer's declared defaults. For one display, keep an
    /// explicit default-valued configuration so a custom "All screens" profile
    /// cannot leak back into it. For all displays, remove every saved override.
    func resetControlValues(to modelDefaults: [String: Double],
                            for wallpaperID: UUID,
                            target: ScreenTarget) {
        switch target {
        case .screen(let displayUUID):
            WallpaperControlStore.shared.setValues(modelDefaults,
                                                   for: wallpaperID,
                                                   displayUUID: displayUUID)
        case .allScreens:
            WallpaperControlStore.shared.removeValues(for: wallpaperID)
        }
        previewControlValues(modelDefaults, for: wallpaperID, target: target)
    }

    /// Applies slider changes immediately without writing UserDefaults for every
    /// intermediate mouse event. The editor persists once dragging ends.
    func previewControlValues(_ values: [String: Double],
                              for wallpaperID: UUID,
                              target: ScreenTarget) {
        switch target {
        case .screen(let displayUUID):
            if let controller = controllers[displayUUID],
               controller.currentWallpaperID == wallpaperID {
                controller.applyControlValues(values)
            }
        case .allScreens:
            for controller in controllers.values where controller.currentWallpaperID == wallpaperID {
                controller.applyControlValues(values)
            }
        }
    }

    func fetchControlDescriptors(for wallpaperID: UUID,
                                 target: ScreenTarget,
                                 completion: @escaping ([WallpaperControlDescriptor]) -> Void) {
        let candidates: [WallpaperWindowController]
        switch target {
        case .screen(let displayUUID):
            candidates = controllers[displayUUID].map { [$0] } ?? []
        case .allScreens:
            candidates = controllers.keys.sorted().compactMap { controllers[$0] }
        }
        guard let controller = candidates.first(where: { $0.currentWallpaperID == wallpaperID }) else {
            completion([])
            return
        }
        controller.fetchControlDescriptors(completion: completion)
    }

    // MARK: - Persistence

    private func assignmentMap() -> [String: String] {
        (defaults.dictionary(forKey: DefaultsKey.activeWallpapers) as? [String: String]) ?? [:]
    }

    private func restoreAssignment(for displayUUID: String) {
        let map = assignmentMap()
        let idString = map[displayUUID] ?? defaults.string(forKey: DefaultsKey.defaultWallpaper)
        guard let idString,
              let id = UUID(uuidString: idString),
              let wallpaper = LibraryManager.shared.wallpaper(id: id),
              let controller = controllers[displayUUID] else { return }
        load(wallpaper, into: controller)
        syncSystemWallpaper(for: displayUUID)
    }

    // MARK: - Playback fan-out

    func setGlobalPaused(_ paused: Bool,
                         fpsCap: Int,
                         deepSleep: Bool = false,
                         preservingFrame: Bool = true,
                         persistSnapshots: Bool = false) {
        for (displayUUID, controller) in controllers {
            if deepSleep {
                let wallpaperID = controller.currentWallpaperID
                let fallback = wallpaperID.flatMap {
                    lastFrames.bestImage(displayUUID: displayUUID, wallpaperID: $0)
                }
                if persistSnapshots {
                    persistBestFrame(for: displayUUID)
                }
                controller.enterDeepSleep(
                    preservingFrame: preservingFrame,
                    fallbackImage: fallback
                ) { [weak self, weak controller] image in
                    guard let self, let controller,
                          let wallpaperID = controller.currentWallpaperID else { return }
                    self.lastFrames.cache(image,
                                          displayUUID: displayUUID,
                                          wallpaperID: wallpaperID)
                    if persistSnapshots {
                        self.persistBestFrame(for: displayUUID)
                    }
                }
            } else if controller.isDeepAsleep {
                controller.exitDeepSleep()
            }
            controller.globallyPaused = paused
            controller.fpsCap = fpsCap
        }
    }

    func kickAllAfterUnlock() {
        for controller in controllers.values {
            controller.kickAfterUnlock()
        }
    }

    // MARK: - Last-frame capture

    private func startSnapshotTimer() {
        snapshotTimer?.invalidate()
        let timer = Timer(timeInterval: 5, repeats: true) { [weak self] _ in
            self?.refreshSnapshotCache()
        }
        timer.tolerance = 0.5
        RunLoop.main.add(timer, forMode: .common)
        snapshotTimer = timer
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak self] in
            self?.refreshSnapshotCache()
        }
    }

    private func refreshSnapshotCache() {
        for (displayUUID, controller) in controllers {
            guard let wallpaperID = controller.currentWallpaperID,
                  !controller.isDeepAsleep,
                  snapshotCaptureTokens[displayUUID] == nil else { continue }
            let token = UUID()
            snapshotCaptureTokens[displayUUID] = token
            controller.captureSnapshot { [weak self, weak controller] image in
                guard let self, self.snapshotCaptureTokens[displayUUID] == token else { return }
                self.snapshotCaptureTokens.removeValue(forKey: displayUUID)
                guard let image, controller?.currentWallpaperID == wallpaperID else { return }
                self.lastFrames.cache(image,
                                      displayUUID: displayUUID,
                                      wallpaperID: wallpaperID)
                let key = LastFrameStore.Key(displayUUID: displayUUID,
                                             wallpaperID: wallpaperID)
                if self.persistedSnapshotKeys.insert(key).inserted,
                   let controller {
                    _ = self.lastFrames.persist(image,
                                                displayUUID: displayUUID,
                                                wallpaperID: wallpaperID,
                                                targetPixelSize: controller.snapshotPixelSize)
                    self.syncSystemWallpaper(for: displayUUID)
                }
            }
            DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) { [weak self] in
                guard self?.snapshotCaptureTokens[displayUUID] == token else { return }
                self?.snapshotCaptureTokens.removeValue(forKey: displayUUID)
            }
        }
    }

    private func persistBestFrame(for displayUUID: String) {
        guard let controller = controllers[displayUUID],
              let wallpaperID = controller.currentWallpaperID else { return }
        if let image = lastFrames.cachedImage(displayUUID: displayUUID,
                                              wallpaperID: wallpaperID) {
            _ = lastFrames.persist(image,
                                   displayUUID: displayUUID,
                                   wallpaperID: wallpaperID,
                                   targetPixelSize: controller.snapshotPixelSize)
        }
        syncSystemWallpaper(for: displayUUID)
    }

    /// Captures every active display before AppKit completes termination. The
    /// timeout guarantees that a suspended WebContent/GPU process cannot hang Quit.
    func prepareForTermination(completion: @escaping () -> Void) {
        let startedAt = CFAbsoluteTimeGetCurrent()
        snapshotTimer?.invalidate()
        snapshotTimer = nil

        let candidates = controllers.compactMap { displayUUID, controller -> (String, WallpaperWindowController, UUID)? in
            guard let wallpaperID = controller.currentWallpaperID else { return nil }
            return (displayUUID, controller, wallpaperID)
        }
        guard !candidates.isEmpty else {
            completion()
            return
        }

        var pending = Set(candidates.map { $0.0 })
        var didFinish = false
        let finishIfReady: (Bool) -> Void = { [weak self] timedOut in
            guard let self, !didFinish else { return }
            if !timedOut && !pending.isEmpty { return }
            didFinish = true
            for (displayUUID, _, _) in candidates {
                self.persistBestFrame(for: displayUUID)
            }
            let duration = CFAbsoluteTimeGetCurrent() - startedAt
            NSLog("ParticleWall: prepared last frames for termination in %.3fs", duration)
            completion()
        }

        for (displayUUID, controller, wallpaperID) in candidates {
            controller.captureSnapshot { [weak self] image in
                guard let self, !didFinish else { return }
                if let image, controller.currentWallpaperID == wallpaperID {
                    self.lastFrames.cache(image,
                                          displayUUID: displayUUID,
                                          wallpaperID: wallpaperID)
                }
                pending.remove(displayUUID)
                finishIfReady(false)
            }
        }

        DispatchQueue.main.asyncAfter(deadline: .now() + 1.5) {
            finishIfReady(true)
        }
    }
}
