import Foundation

enum ASCIIFrameStoreError: LocalizedError {
    case missingFile
    case invalidMagic
    case unsupportedVersion(UInt16)
    case invalidHeader
    case truncated

    var errorDescription: String? {
        switch self {
        case .missingFile: return "No se encontró un archivo .asciivideo."
        case .invalidMagic: return "El archivo no es un asciivideo de ParticleWall."
        case .unsupportedVersion(let version): return "Versión asciivideo no soportada: \(version)."
        case .invalidHeader: return "Cabecera asciivideo inválida."
        case .truncated: return "El archivo asciivideo está truncado."
        }
    }
}

/// Reads the compact, precomputed cell stream used by the native ASCII renderer.
final class ASCIIFrameStore {
    struct Header {
        let cellSize: Int
        let sourceWidth: Int
        let sourceHeight: Int
        let columns: Int
        let rows: Int
        let frameCount: Int
        let fps: Double
    }

    let header: Header
    private let data: Data
    private let frameOffset = 48
    private let bytesPerCell = 2
    private let bytesPerFrame: Int

    var cellCount: Int { header.columns * header.rows }

    convenience init(rootURL: URL) throws {
        let files = (try? FileManager.default.contentsOfDirectory(
            at: rootURL,
            includingPropertiesForKeys: [.isRegularFileKey],
            options: [.skipsHiddenFiles]
        )) ?? []
        guard let url = files.first(where: { $0.pathExtension.lowercased() == "asciivideo" }) else {
            throw ASCIIFrameStoreError.missingFile
        }
        try self.init(fileURL: url)
    }

    init(fileURL: URL) throws {
        data = try Data(contentsOf: fileURL, options: [.mappedIfSafe])
        guard data.count >= frameOffset else { throw ASCIIFrameStoreError.truncated }
        guard Array(data.prefix(8)) == Array("PWASCII1".utf8) else {
            throw ASCIIFrameStoreError.invalidMagic
        }
        let version = Self.u16(data, at: 8)
        guard version == 1 else { throw ASCIIFrameStoreError.unsupportedVersion(version) }
        guard Self.u16(data, at: 10) == 48 else { throw ASCIIFrameStoreError.invalidHeader }

        let cellSize = Int(Self.u16(data, at: 12))
        let sourceWidth = Int(Self.u32(data, at: 16))
        let sourceHeight = Int(Self.u32(data, at: 20))
        let columns = Int(Self.u32(data, at: 24))
        let rows = Int(Self.u32(data, at: 28))
        let frameCount = Int(Self.u32(data, at: 32))
        let fpsNumerator = Self.u32(data, at: 36)
        let fpsDenominator = Self.u32(data, at: 40)
        guard cellSize > 0, sourceWidth > 0, sourceHeight > 0,
              columns > 0, rows > 0, frameCount > 0,
              fpsNumerator > 0, fpsDenominator > 0 else {
            throw ASCIIFrameStoreError.invalidHeader
        }
        let bytesPerFrame = columns * rows * bytesPerCell
        let expectedSize = frameOffset + frameCount * bytesPerFrame
        guard expectedSize == data.count else { throw ASCIIFrameStoreError.truncated }

        header = Header(
            cellSize: cellSize,
            sourceWidth: sourceWidth,
            sourceHeight: sourceHeight,
            columns: columns,
            rows: rows,
            frameCount: frameCount,
            fps: Double(fpsNumerator) / Double(fpsDenominator)
        )
        self.bytesPerFrame = bytesPerFrame
    }

    /// Expands compact cells to uint values: low byte glyph, next byte RGB332.
    func expandedFrame(at index: Int) -> [UInt32] {
        let safeIndex = ((index % header.frameCount) + header.frameCount) % header.frameCount
        let start = frameOffset + safeIndex * bytesPerFrame
        return (0..<cellCount).map { cell in
            let offset = start + cell * bytesPerCell
            return UInt32(data[offset]) | (UInt32(data[offset + 1]) << 8)
        }
    }

    private static func u16(_ data: Data, at offset: Int) -> UInt16 {
        UInt16(data[offset]) | (UInt16(data[offset + 1]) << 8)
    }

    private static func u32(_ data: Data, at offset: Int) -> UInt32 {
        UInt32(data[offset])
            | (UInt32(data[offset + 1]) << 8)
            | (UInt32(data[offset + 2]) << 16)
            | (UInt32(data[offset + 3]) << 24)
    }
}
