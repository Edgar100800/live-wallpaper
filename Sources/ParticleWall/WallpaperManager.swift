import AppKit

/// Owns one WallpaperWindowController per connected screen, reacts to screen
/// changes, applies wallpapers and persists the assignment per display.
final class WallpaperManager {
    static let shared = WallpaperManager()

    private(set) var controllers: [String: WallpaperWindowController] = [:] // displayUUID -> controller
    private let defaults = UserDefaults.standard
    private var lastRenderScale: Double = 0

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
    }

    /// Render scale is baked into each webview's user scripts at creation,
    /// so a change requires rebuilding them.
    @objc private func defaultsChanged() {
        let scale = defaults.double(forKey: DefaultsKey.renderScale)
        guard scale != lastRenderScale else { return }
        lastRenderScale = scale
        for controller in controllers.values {
            controller.recreateWebView()
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
                restoreAssignment(for: uuid)
            }
        }

        PowerManager.shared.pushStateToAllControllers()
    }

    // MARK: - Applying wallpapers

    func apply(_ wallpaper: Wallpaper, to target: ScreenTarget) {
        switch target {
        case .allScreens:
            for (uuid, controller) in controllers {
                load(wallpaper, into: controller)
                syncSystemWallpaper(for: uuid)
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
        NotificationCenter.default.post(name: .pwPlaybackStateChanged, object: nil)
    }

    private func load(_ wallpaper: Wallpaper, into controller: WallpaperWindowController) {
        controller.manifestFPS = wallpaper.manifest.fps ?? 0
        controller.load(indexURL: wallpaper.indexURL,
                        rootURL: wallpaper.folderURL,
                        wallpaperID: wallpaper.id)
    }

    // MARK: - System wallpaper sync

    /// The macOS menu bar (and Space transitions) derive their tint from the
    /// SYSTEM desktop picture, not from our desktop-level window. Point it at
    /// the active wallpaper's thumbnail so no stale colors bleed through.
    private func syncSystemWallpaper(for displayUUID: String) {
        guard let controller = controllers[displayUUID],
              let entry = screensByUUID.first(where: { $0.uuid == displayUUID }),
              let id = controller.currentWallpaperID,
              let wallpaper = LibraryManager.shared.wallpaper(id: id) else { return }
        let imageURL: URL
        if FileManager.default.fileExists(atPath: wallpaper.thumbnailURL.path) {
            imageURL = wallpaper.thumbnailURL
        } else if let black = Self.blackFallbackImage() {
            imageURL = black
        } else {
            return
        }
        let options: [NSWorkspace.DesktopImageOptionKey: Any] = [
            .imageScaling: NSImageScaling.scaleAxesIndependently.rawValue,
            .allowClipping: true
        ]
        do {
            try NSWorkspace.shared.setDesktopImageURL(imageURL, for: entry.screen, options: options)
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

    func restoreAllAssignments() {
        for uuid in controllers.keys {
            restoreAssignment(for: uuid)
        }
    }

    // MARK: - Playback fan-out

    func setGlobalPaused(_ paused: Bool, fpsCap: Int) {
        for controller in controllers.values {
            controller.globallyPaused = paused
            controller.fpsCap = fpsCap
        }
    }

    func kickAllAfterUnlock() {
        for controller in controllers.values {
            controller.kickAfterUnlock()
        }
    }
}
