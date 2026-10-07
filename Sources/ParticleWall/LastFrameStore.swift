import AppKit
import ImageIO
import UniformTypeIdentifiers

/// Keeps recent renderer frames in memory and persists the last usable frame for
/// each display/wallpaper pair. Persisted files remain valid after the app exits.
final class LastFrameStore {
    static let colorPipelineVersion = 2
    static let shared = LastFrameStore(
        rootURL: LibraryManager.shared.rootURL.appendingPathComponent(
            "LastFrames-v\(colorPipelineVersion)",
            isDirectory: true
        )
    )

    struct Key: Hashable {
        let displayUUID: String
        let wallpaperID: UUID
    }

    private let rootURL: URL
    private let fileManager: FileManager
    private var memory: [Key: NSImage] = [:]

    init(rootURL: URL, fileManager: FileManager = .default) {
        self.rootURL = rootURL
        self.fileManager = fileManager
    }

    func cache(_ image: NSImage, displayUUID: String, wallpaperID: UUID) {
        // Keep only the current frame for this display, not every wallpaper
        // visited during the session. Older persisted frames remain on disk.
        memory = memory.filter { $0.key.displayUUID != displayUUID }
        memory[Key(displayUUID: displayUUID, wallpaperID: wallpaperID)] = image
    }

    func removeCachedImages(displayUUID: String) {
        memory = memory.filter { $0.key.displayUUID != displayUUID }
    }

    func cachedImage(displayUUID: String, wallpaperID: UUID) -> NSImage? {
        memory[Key(displayUUID: displayUUID, wallpaperID: wallpaperID)]
    }

    func bestImage(displayUUID: String, wallpaperID: UUID) -> NSImage? {
        cachedImage(displayUUID: displayUUID, wallpaperID: wallpaperID)
            ?? NSImage(contentsOf: persistedURL(displayUUID: displayUUID,
                                                wallpaperID: wallpaperID))
    }

    func persistedURL(displayUUID: String, wallpaperID: UUID) -> URL {
        rootURL.appendingPathComponent(
            "\(safeComponent(displayUUID))--\(wallpaperID.uuidString).png"
        )
    }

    @discardableResult
    func persist(_ image: NSImage,
                 displayUUID: String,
                 wallpaperID: UUID,
                 targetPixelSize: CGSize? = nil) -> URL? {
        guard let data = Self.pngData(from: image, targetPixelSize: targetPixelSize) else { return nil }
        do {
            try fileManager.createDirectory(at: rootURL, withIntermediateDirectories: true)
            let url = persistedURL(displayUUID: displayUUID, wallpaperID: wallpaperID)
            try data.write(to: url, options: .atomic)
            removeStaleFiles(displayUUID: displayUUID, keeping: url)
            cache(image, displayUUID: displayUUID, wallpaperID: wallpaperID)
            if let representation = NSBitmapImageRep(data: data) {
                NSLog("ParticleWall: persisted color pipeline v%d frame %dx%d sRGB at %@",
                      Self.colorPipelineVersion,
                      representation.pixelsWide,
                      representation.pixelsHigh,
                      url.path)
            }
            return url
        } catch {
            NSLog("ParticleWall: could not persist last frame: \(error)")
            return nil
        }
    }

    func remove(wallpaperID: UUID) {
        memory = memory.filter { $0.key.wallpaperID != wallpaperID }
        guard let files = try? fileManager.contentsOfDirectory(at: rootURL,
                                                               includingPropertiesForKeys: nil) else {
            return
        }
        let suffix = "--\(wallpaperID.uuidString).png"
        for file in files where file.lastPathComponent.hasSuffix(suffix) {
            try? fileManager.removeItem(at: file)
        }
    }

    private func removeStaleFiles(displayUUID: String, keeping keptURL: URL) {
        guard let files = try? fileManager.contentsOfDirectory(at: rootURL,
                                                               includingPropertiesForKeys: nil) else {
            return
        }
        let prefix = "\(safeComponent(displayUUID))--"
        for file in files where file.lastPathComponent != keptURL.lastPathComponent
                && file.lastPathComponent.hasPrefix(prefix) {
            try? fileManager.removeItem(at: file)
        }
    }

    private func safeComponent(_ value: String) -> String {
        let allowed = CharacterSet.alphanumerics.union(CharacterSet(charactersIn: "-_"))
        return value.unicodeScalars.map { allowed.contains($0) ? String($0) : "_" }.joined()
    }

    private static func pngData(from image: NSImage, targetPixelSize: CGSize?) -> Data? {
        guard let source = bestCGImage(from: image) else { return nil }
        let requestedWidth = targetPixelSize.map { Int($0.width.rounded()) } ?? source.width
        let requestedHeight = targetPixelSize.map { Int($0.height.rounded()) } ?? source.height
        let width = max(1, requestedWidth)
        let height = max(1, requestedHeight)
        guard let sRGB = CGColorSpace(name: CGColorSpace.sRGB),
              let context = CGContext(data: nil,
                                      width: width,
                                      height: height,
                                      bitsPerComponent: 8,
                                      bytesPerRow: 0,
                                      space: sRGB,
                                      bitmapInfo: CGBitmapInfo.byteOrder32Big.rawValue
                                        | CGImageAlphaInfo.premultipliedLast.rawValue)
        else { return nil }
        context.interpolationQuality = .high
        context.setRenderingIntent(.relativeColorimetric)
        context.setFillColor(CGColor(srgbRed: 0, green: 0, blue: 0, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: width, height: height))
        context.draw(source, in: CGRect(x: 0, y: 0, width: width, height: height))
        guard let output = context.makeImage() else { return nil }

        let data = NSMutableData()
        guard let destination = CGImageDestinationCreateWithData(
            data,
            UTType.png.identifier as CFString,
            1,
            nil
        ) else { return nil }
        CGImageDestinationAddImage(destination, output, nil)
        guard CGImageDestinationFinalize(destination) else { return nil }
        return data as Data
    }

    private static func bestCGImage(from image: NSImage) -> CGImage? {
        let bitmap = image.representations
            .compactMap { ($0 as? NSBitmapImageRep)?.cgImage }
            .max { lhs, rhs in lhs.width * lhs.height < rhs.width * rhs.height }
        if let bitmap { return bitmap }
        var proposedRect = NSRect(origin: .zero, size: image.size)
        return image.cgImage(forProposedRect: &proposedRect, context: nil, hints: nil)
    }
}
