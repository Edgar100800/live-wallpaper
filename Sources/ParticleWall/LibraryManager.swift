import AppKit
import Combine

/// CRUD over ~/Library/Application Support/ParticleWall/wallpapers/<uuid>/.
final class LibraryManager: ObservableObject {
    static let shared = LibraryManager()

    @Published private(set) var wallpapers: [Wallpaper] = []

    let rootURL: URL
    let wallpapersURL: URL
    private let fm = FileManager.default

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

    /// First run: install the bundled demo wallpaper and apply it everywhere.
    func installBundledDefaultIfNeeded() {
        guard wallpapers.isEmpty else { return }
        guard let defaultFolder = Bundle.module.url(forResource: "DefaultWallpaper", withExtension: nil),
              let threeJS = Bundle.module.url(forResource: "three.min", withExtension: "js") else {
            NSLog("ParticleWall: bundled default wallpaper missing")
            return
        }
        do {
            let id = UUID()
            let folder = wallpapersURL.appendingPathComponent(id.uuidString, isDirectory: true)
            let assets = folder.appendingPathComponent("assets", isDirectory: true)
            try fm.createDirectory(at: assets, withIntermediateDirectories: true)
            try fm.copyItem(at: defaultFolder.appendingPathComponent("index.html"),
                            to: folder.appendingPathComponent("index.html"))
            try fm.copyItem(at: threeJS, to: assets.appendingPathComponent("three.min.js"))
            let manifest = WallpaperManifest(name: "Demo Particles", source: "bundled")
            try writeManifest(manifest, to: folder)
            loadLibrary()
            if let wallpaper = wallpaper(id: id) {
                WallpaperManager.shared.apply(wallpaper, to: .allScreens)
                ThumbnailGenerator.shared.generate(for: wallpaper) { [weak self] in
                    self?.loadLibrary()
                }
            }
        } catch {
            NSLog("ParticleWall: failed to install default wallpaper: \(error)")
        }
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

    func delete(_ wallpaper: Wallpaper) {
        WallpaperManager.shared.wallpaperRemoved(wallpaper.id)
        try? fm.removeItem(at: wallpaper.folderURL)
        loadLibrary()
    }

    func regenerateThumbnail(_ wallpaper: Wallpaper) {
        ThumbnailGenerator.shared.generate(for: wallpaper) { [weak self] in
            self?.loadLibrary()
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
