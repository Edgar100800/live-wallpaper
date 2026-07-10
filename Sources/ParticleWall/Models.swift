import Foundation

/// Manifest stored as manifest.json inside each wallpaper folder.
struct WallpaperManifest: Codable {
    var name: String
    var createdAt: Date
    var fps: Int?
    var source: String

    init(name: String, createdAt: Date = Date(), fps: Int? = nil, source: String) {
        self.name = name
        self.createdAt = createdAt
        self.fps = fps
        self.source = source
    }
}

/// A wallpaper entry in the library.
struct Wallpaper: Identifiable, Hashable {
    let id: UUID
    var manifest: WallpaperManifest
    var folderURL: URL

    var indexURL: URL { folderURL.appendingPathComponent("index.html") }
    var thumbnailURL: URL { folderURL.appendingPathComponent("thumbnail.png") }
    var manifestURL: URL { folderURL.appendingPathComponent("manifest.json") }
    var name: String { manifest.name }

    static func == (lhs: Wallpaper, rhs: Wallpaper) -> Bool { lhs.id == rhs.id }
    func hash(into hasher: inout Hasher) { hasher.combine(id) }
}

/// Where to apply a wallpaper.
enum ScreenTarget: Hashable {
    case allScreens
    case screen(displayUUID: String)
}

enum DefaultsKey {
    static let activeWallpapers = "activeWallpapers"   // [displayUUID: wallpaperUUID]
    static let defaultWallpaper = "defaultWallpaper"   // wallpaperUUID for new/unassigned screens
    static let pauseOnBattery = "pauseOnBattery"
    static let powerSave = "powerSave"
    static let fpsCap = "fpsCap"                       // 0 = unlimited
    static let renderScale = "renderScale"             // devicePixelRatio cap: 1.0 / 1.5 / 2.0
    static let globallyPaused = "globallyPaused"
}

extension Notification.Name {
    static let pwLibraryChanged = Notification.Name("pwLibraryChanged")
    static let pwPlaybackStateChanged = Notification.Name("pwPlaybackStateChanged")
}
