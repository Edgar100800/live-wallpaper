import Foundation
import XCTest
@testable import ParticleWall

final class ASCIIFrameStoreTests: XCTestCase {
    func testReadsCompactFrameAndExpandsCells() throws {
        var data = Data("PWASCII1".utf8)
        appendUInt16(1, to: &data)
        appendUInt16(48, to: &data)
        appendUInt16(8, to: &data)
        appendUInt16(0, to: &data)
        appendUInt32(16, to: &data)
        appendUInt32(8, to: &data)
        appendUInt32(2, to: &data)
        appendUInt32(1, to: &data)
        appendUInt32(1, to: &data)
        appendUInt32(30, to: &data)
        appendUInt32(1, to: &data)
        appendUInt32(0, to: &data)
        data.append(contentsOf: [3, 0b1110_0000, 10, 0b0001_1111])

        let folder = FileManager.default.temporaryDirectory
            .appendingPathComponent("ascii-store-\(UUID().uuidString)", isDirectory: true)
        try FileManager.default.createDirectory(at: folder, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: folder) }
        try data.write(to: folder.appendingPathComponent("clip.asciivideo"))

        let store = try ASCIIFrameStore(rootURL: folder)
        XCTAssertEqual(store.header.columns, 2)
        XCTAssertEqual(store.header.rows, 1)
        XCTAssertEqual(store.header.frameCount, 1)
        XCTAssertEqual(store.expandedFrame(at: 0), [
            UInt32(3) | (UInt32(0b1110_0000) << 8),
            UInt32(10) | (UInt32(0b0001_1111) << 8)
        ])
    }

    private func appendUInt16(_ value: UInt16, to data: inout Data) {
        data.append(UInt8(value & 0xff))
        data.append(UInt8(value >> 8))
    }

    private func appendUInt32(_ value: UInt32, to data: inout Data) {
        data.append(UInt8(value & 0xff))
        data.append(UInt8((value >> 8) & 0xff))
        data.append(UInt8((value >> 16) & 0xff))
        data.append(UInt8((value >> 24) & 0xff))
    }
}
