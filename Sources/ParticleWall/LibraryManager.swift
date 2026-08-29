import AppKit
import Combine

/// CRUD over ~/Library/Application Support/ParticleWall/wallpapers/<uuid>/.
final class LibraryManager: ObservableObject {
    static let shared = LibraryManager()

    @Published private(set) var wallpapers: [Wallpaper] = []

    let rootURL: URL
    let wallpapersURL: URL
    private let fm = FileManager.default

    private struct BundledWallpaperSpec {
        let name: String
        let resourceName: String
        let renderer: WallpaperRendererKind
    }

    private static let bundledWallpaperSpecs: [BundledWallpaperSpec] = [
        .init(name: "Ondas Paramétricas",
              resourceName: "DefaultWallpaper",
              renderer: .metalParticles),
        .init(name: "Vórtice Gemelo",
              resourceName: "TwinVortexWallpaper",
              renderer: .metalTwinVortex),
        .init(name: "Flor Orbital",
              resourceName: "OrbitalBloomWallpaper",
              renderer: .metalOrbitalBloom),
        .init(name: "Roseta Hexagonal",
              resourceName: "HexagonalRosetteWallpaper",
              renderer: .metalHexagonalRosette),
        .init(name: "Lluvia de Ruido",
              resourceName: "NoiseRainWallpaper",
              renderer: .metalNoiseRain),
        .init(name: "Espiral Prima",
              resourceName: "PrimeSpiralWallpaper",
              renderer: .metalPrimeSpiral),
        .init(name: "Órbita Toroidal",
              resourceName: "TorusOrbitWallpaper",
              renderer: .metalTorusOrbit),
        .init(name: "Anillos Cromáticos",
              resourceName: "ChromaticRingsWallpaper",
              renderer: .metalChromaticRings),
        .init(name: "Toro de Esferas",
              resourceName: "SphereTorusWallpaper",
              renderer: .metalSphereTorus),
        .init(name: "Medusa de Puntos",
              resourceName: "JellyfishPointsWallpaper",
              renderer: .metalJellyfishPoints)
    ]

    private init() {
        let appSupport = fm.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
        rootURL = appSupport.appendingPathComponent("ParticleWall", isDirectory: true)
        wallpapersURL = rootURL.appendingPathComponent("wallpapers", isDirectory: true)
        try? fm.createDirectory(at: wallpapersURL, withIntermediateDirectories: true)
    }

    func wallpaper(id: UUID) -> Wallpaper? {
        wallpapers.first { $0.id == id }
    }

    // MARK: - Loading

    func loadLibrary() {
        var found: [Wallpaper] = []
        let contents = (try? fm.contentsOfDirectory(at: wallpapersURL,
                                                    includingPropertiesForKeys: nil,
                                                    options: [.skipsHiddenFiles])) ?? []
        for folder in contents {
            guard let id = UUID(uuidString: folder.lastPathComponent) else { continue }
            let manifestURL = folder.appendingPathComponent("manifest.json")
            guard let data = try? Data(contentsOf: manifestURL),
                  let manifest = try? Self.decoder.decode(WallpaperManifest.self, from: data),
                  fm.fileExists(atPath: folder.appendingPathComponent("index.html").path) else {
                continue
            }
            found.append(Wallpaper(id: id, manifest: manifest, folderURL: folder))
        }
        wallpapers = found.sorted { $0.manifest.createdAt > $1.manifest.createdAt }
    }

    /// Install every missing built-in wallpaper. Identity is based on renderer,
    /// so renaming one does not create duplicates and deleting old installations
    /// is repaired on the next launch.
    func installBundledDefaultIfNeeded() {
        let missing = Self.bundledWallpaperSpecs.filter { spec in
            !wallpapers.contains {
                $0.isBundled && $0.manifest.effectiveRenderer == spec.renderer
            }
        }
        guard !missing.isEmpty else { return }
        let libraryWasEmpty = wallpapers.isEmpty
        var installedIDs: [UUID] = []

        for spec in missing {
            guard let resourceFolder = Bundle.module.url(forResource: spec.resourceName,
                                                         withExtension: nil) else {
                NSLog("ParticleWall: bundled wallpaper resource missing: \(spec.resourceName)")
                continue
            }
            let id = UUID()
            let folder = wallpapersURL.appendingPathComponent(id.uuidString, isDirectory: true)
            do {
                try fm.createDirectory(at: folder, withIntermediateDirectories: true)
                try fm.copyItem(at: resourceFolder.appendingPathComponent("index.html"),
                                to: folder.appendingPathComponent("index.html"))
                let manifest = WallpaperManifest(name: spec.name,
                                                 source: "bundled",
                                                 renderer: spec.renderer)
                try writeManifest(manifest, to: folder)
                installedIDs.append(id)
            } catch {
                try? fm.removeItem(at: folder)
                NSLog("ParticleWall: failed to install \(spec.name): \(error)")
            }
        }

        guard !installedIDs.isEmpty else { return }
        loadLibrary()
        let hasActiveWallpaper = WallpaperManager.shared.controllers.values.contains {
            $0.currentWallpaperID != nil
        }
        if (libraryWasEmpty || !hasActiveWallpaper),
           let defaultWallpaper = wallpapers.first(where: {
               $0.isBundled && $0.manifest.effectiveRenderer == .metalParticles
           }) {
            WallpaperManager.shared.apply(defaultWallpaper, to: .allScreens)
        }

        for id in installedIDs {
            guard let wallpaper = wallpaper(id: id) else { continue }
            ThumbnailGenerator.shared.generate(for: wallpaper) { [weak self] in
                self?.loadLibrary()
                WallpaperManager.shared.refreshSystemWallpaper(for: id)
            }
        }
    }

    /// Keep bundled fallbacks and renderer identifiers current.
    /// User-imported wallpapers are never rewritten.
    func upgradeBundledWallpapers() {
        var manifestChanged = false
        for wallpaper in wallpapers where wallpaper.isBundled {
            let renderer = wallpaper.manifest.effectiveRenderer
            guard let spec = Self.bundledWallpaperSpecs.first(where: {
                $0.renderer == renderer
            }), let bundledFolder = Bundle.module.url(forResource: spec.resourceName,
                                                      withExtension: nil),
                let bundledHTML = try? String(
                    contentsOf: bundledFolder.appendingPathComponent("index.html"),
                    encoding: .utf8
                ) else { continue }

            if wallpaper.manifest.renderer == nil ||
                (renderer == .metalParticles && wallpaper.manifest.name == "Demo Particles") {
                var manifest = wallpaper.manifest
                if manifest.name == "Demo Particles" {
                    manifest.name = spec.name
                }
                manifest.renderer = renderer
                try? writeManifest(manifest, to: wallpaper.folderURL)
                manifestChanged = true
            }

            if (try? String(contentsOf: wallpaper.indexURL, encoding: .utf8)) != bundledHTML {
                do {
                    try bundledHTML.write(to: wallpaper.indexURL, atomically: true, encoding: .utf8)
                    regenerateThumbnail(wallpaper)
                } catch {
                    NSLog("ParticleWall: could not upgrade bundled wallpaper: \(error)")
                }
            }
        }
        if manifestChanged { loadLibrary() }
    }

    /// Applies the built-in native animation without changing imported content.
    @discardableResult
    func applyBundledDefault() -> Bool {
        guard let wallpaper = wallpapers.first(where: {
            $0.isBundled && $0.manifest.effectiveRenderer == .metalParticles
        }) else {
            return false
        }
        WallpaperManager.shared.apply(wallpaper, to: .allScreens)
        return true
    }

    // MARK: - Mutations

    func add(folderWithContents sourceFolder: URL, name: String, source: String) throws -> Wallpaper {
        let id = UUID()
        let folder = wallpapersURL.appendingPathComponent(id.uuidString, isDirectory: true)
        try fm.copyItem(at: sourceFolder, to: folder)
        let manifest = WallpaperManifest(name: name, source: source)
        try writeManifest(manifest, to: folder)
        let wallpaper = Wallpaper(id: id, manifest: manifest, folderURL: folder)
        loadLibrary()
        ThumbnailGenerator.shared.generate(for: wallpaper) { [weak self] in
            self?.loadLibrary()
        }
        return wallpaper
    }

    func rename(_ wallpaper: Wallpaper, to newName: String) {
        var manifest = wallpaper.manifest
        manifest.name = newName
        try? writeManifest(manifest, to: wallpaper.folderURL)
        loadLibrary()
    }

    /// Per-wallpaper FPS cap (nil = follow the global setting). Applies live
    /// on any screen currently showing this wallpaper.
    func setFPS(_ wallpaper: Wallpaper, fps: Int?) {
        var manifest = wallpaper.manifest
        manifest.fps = fps
        try? writeManifest(manifest, to: wallpaper.folderURL)
        loadLibrary()
        WallpaperManager.shared.updateManifestFPS(for: wallpaper.id, fps: fps)
    }

    func delete(_ wallpaper: Wallpaper) {
        guard !wallpaper.isBundled else {
            NSLog("ParticleWall: ignored deletion request for bundled wallpaper \(wallpaper.id)")
            return
        }
        WallpaperManager.shared.wallpaperRemoved(wallpaper.id)
        try? fm.removeItem(at: wallpaper.folderURL)
        loadLibrary()
    }

    func regenerateThumbnail(_ wallpaper: Wallpaper) {
        let values = WallpaperManager.shared.controlValues(for: wallpaper.id, target: .allScreens)
        ThumbnailGenerator.shared.generate(for: wallpaper,
                                           controlValues: values.isEmpty ? nil : values) { [weak self] in
            self?.loadLibrary()
            WallpaperManager.shared.refreshSystemWallpaper(for: wallpaper.id)
        }
    }

    func revealInFinder(_ wallpaper: Wallpaper) {
        NSWorkspace.shared.activateFileViewerSelecting([wallpaper.folderURL])
    }

    // MARK: - Manifest IO

    private static let encoder: JSONEncoder = {
        let e = JSONEncoder()
        e.dateEncodingStrategy = .iso8601
        e.outputFormatting = [.prettyPrinted, .sortedKeys]
        return e
    }()

    private static let decoder: JSONDecoder = {
        let d = JSONDecoder()
        d.dateDecodingStrategy = .iso8601
        return d
    }()

    private func writeManifest(_ manifest: WallpaperManifest, to folder: URL) throws {
        let data = try Self.encoder.encode(manifest)
        try data.write(to: folder.appendingPathComponent("manifest.json"))
    }
}
