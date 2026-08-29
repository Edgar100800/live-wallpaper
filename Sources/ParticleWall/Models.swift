import Foundation

enum WallpaperRendererKind: String, Codable, CaseIterable {
    case web
    case metalParticles = "metal-particles"
    case metalTwinVortex = "metal-twin-vortex"
    case metalOrbitalBloom = "metal-orbital-bloom"
    case metalHexagonalRosette = "metal-hexagonal-rosette"
    case metalNoiseRain = "metal-noise-rain"
    case metalPrimeSpiral = "metal-prime-spiral"
    case metalTorusOrbit = "metal-torus-orbit"
    case metalChromaticRings = "metal-chromatic-rings"
    case metalSphereTorus = "metal-sphere-torus"

    var isNativeMetal: Bool { self != .web }

    static var nativeMetalCases: [WallpaperRendererKind] {
        allCases.filter(\.isNativeMetal)
    }
}

/// Manifest stored as manifest.json inside each wallpaper folder.
struct WallpaperManifest: Codable {
    var name: String
    var createdAt: Date
    var fps: Int?
    var source: String
    var renderer: WallpaperRendererKind?

    init(name: String,
         createdAt: Date = Date(),
         fps: Int? = nil,
         source: String,
         renderer: WallpaperRendererKind? = nil) {
        self.name = name
        self.createdAt = createdAt
        self.fps = fps
        self.source = source
        self.renderer = renderer
    }

    /// Old manifests predate the renderer field. The bundled demo is safe to
    /// promote automatically; arbitrary imported content remains on WebKit.
    var effectiveRenderer: WallpaperRendererKind {
        renderer ?? (source == "bundled" ? .metalParticles : .web)
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
    var isBundled: Bool { manifest.source == "bundled" }

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
    static let adaptiveQuality = "adaptiveQuality"
    static let globallyPaused = "globallyPaused"
    static let wallpaperControls = "wallpaperControls" // JSON: wallpaper/display -> numeric controls
    static let colorProfiles = "colorProfiles"         // JSON: personal background/particle pairs
}

extension Notification.Name {
    static let pwLibraryChanged = Notification.Name("pwLibraryChanged")
    static let pwPlaybackStateChanged = Notification.Name("pwPlaybackStateChanged")
    static let pwGalleryDidHide = Notification.Name("pwGalleryDidHide")
}
