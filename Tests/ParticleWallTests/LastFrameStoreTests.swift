import AppKit
import XCTest
@testable import ParticleWall

final class LastFrameStoreTests: XCTestCase {
    private var temporaryURL: URL!
    private var store: LastFrameStore!

    override func setUpWithError() throws {
        temporaryURL = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        store = LastFrameStore(rootURL: temporaryURL)
    }

    override func tearDownWithError() throws {
        if let temporaryURL {
            try? FileManager.default.removeItem(at: temporaryURL)
        }
    }

    func testPersistsAndReloadsMatchingFrame() throws {
        let wallpaperID = UUID()
        let image = makeImage(color: .systemPink)

        let url = try XCTUnwrap(store.persist(image,
                                              displayUUID: "display/one",
                                              wallpaperID: wallpaperID))

        XCTAssertTrue(FileManager.default.fileExists(atPath: url.path))
        XCTAssertNotNil(store.bestImage(displayUUID: "display/one", wallpaperID: wallpaperID))
        XCTAssertNil(store.bestImage(displayUUID: "display/one", wallpaperID: UUID()))
    }

    func testNewWallpaperRemovesStaleFrameForDisplay() throws {
        let firstID = UUID()
        let secondID = UUID()
        let image = makeImage(color: .black)
        let firstURL = try XCTUnwrap(store.persist(image,
                                                   displayUUID: "display-one",
                                                   wallpaperID: firstID))
        let secondURL = try XCTUnwrap(store.persist(image,
                                                    displayUUID: "display-one",
                                                    wallpaperID: secondID))

        XCTAssertFalse(FileManager.default.fileExists(atPath: firstURL.path))
        XCTAssertTrue(FileManager.default.fileExists(atPath: secondURL.path))
    }

    func testRemovingWallpaperDeletesItsFrames() throws {
        let wallpaperID = UUID()
        let url = try XCTUnwrap(store.persist(makeImage(color: .white),
                                              displayUUID: "display-one",
                                              wallpaperID: wallpaperID))

        store.remove(wallpaperID: wallpaperID)

        XCTAssertFalse(FileManager.default.fileExists(atPath: url.path))
        XCTAssertNil(store.cachedImage(displayUUID: "display-one", wallpaperID: wallpaperID))
    }

    func testSwitchingWallpaperEvictsOnlyThatDisplaysPreviousImage() throws {
        let firstID = UUID()
        let otherDisplayID = UUID()
        let image = makeImage(color: .black)
        let persisted = try XCTUnwrap(store.persist(image,
                                                    displayUUID: "display-one",
                                                    wallpaperID: firstID))
        store.cache(image, displayUUID: "display-two", wallpaperID: otherDisplayID)

        var previousID = firstID
        for _ in 0..<20 {
            let nextID = UUID()
            store.cache(image, displayUUID: "display-one", wallpaperID: nextID)
            XCTAssertNil(store.cachedImage(displayUUID: "display-one", wallpaperID: previousID))
            XCTAssertNotNil(store.cachedImage(displayUUID: "display-one", wallpaperID: nextID))
            previousID = nextID
        }

        XCTAssertNotNil(store.cachedImage(displayUUID: "display-two", wallpaperID: otherDisplayID))
        XCTAssertTrue(FileManager.default.fileExists(atPath: persisted.path))
        XCTAssertNotNil(store.bestImage(displayUUID: "display-one", wallpaperID: firstID))
    }

    func testDisconnectingDisplayClearsItsMemoryButKeepsPersistedFrame() throws {
        let wallpaperID = UUID()
        let image = makeImage(color: .white)
        let url = try XCTUnwrap(store.persist(image,
                                              displayUUID: "display-one",
                                              wallpaperID: wallpaperID))
        store.cache(image, displayUUID: "display-two", wallpaperID: wallpaperID)

        store.removeCachedImages(displayUUID: "display-one")

        XCTAssertNil(store.cachedImage(displayUUID: "display-one", wallpaperID: wallpaperID))
        XCTAssertNotNil(store.cachedImage(displayUUID: "display-two", wallpaperID: wallpaperID))
        XCTAssertTrue(FileManager.default.fileExists(atPath: url.path))
        XCTAssertNotNil(store.bestImage(displayUUID: "display-one", wallpaperID: wallpaperID))
    }

    func testPersistsRequestedPixelResolutionAndColorProfile() throws {
        let url = try XCTUnwrap(store.persist(makeImage(color: .systemRed),
                                              displayUUID: "retina-display",
                                              wallpaperID: UUID(),
                                              targetPixelSize: CGSize(width: 64, height: 48)))
        let data = try Data(contentsOf: url)
        let representation = try XCTUnwrap(NSBitmapImageRep(data: data))

        XCTAssertEqual(representation.pixelsWide, 64)
        XCTAssertEqual(representation.pixelsHigh, 48)
        XCTAssertNotNil(representation.colorSpace)
        XCTAssertNotNil(representation.value(forProperty: .colorSyncProfileData))
    }

    func testSRGBPaletteSurvivesPNGPixelForPixel() throws {
        let wallpaperID = UUID()
        let image = try makePaletteImage(width: 16, height: 8)
        let url = try XCTUnwrap(store.persist(image,
                                              displayUUID: "color-display",
                                              wallpaperID: wallpaperID,
                                              targetPixelSize: CGSize(width: 16, height: 8)))
        let representation = try XCTUnwrap(NSBitmapImageRep(data: Data(contentsOf: url)))

        assertPixel(in: representation, x: 2, y: 4, hex: 0x0060C3)
        assertPixel(in: representation, x: 13, y: 4, hex: 0xFAFF00)
    }

    private func makeImage(color: NSColor) -> NSImage {
        let image = NSImage(size: NSSize(width: 8, height: 8))
        image.lockFocus()
        color.setFill()
        NSRect(x: 0, y: 0, width: 8, height: 8).fill()
        image.unlockFocus()
        return image
    }

    private func makePaletteImage(width: Int, height: Int) throws -> NSImage {
        let colorSpace = try XCTUnwrap(CGColorSpace(name: CGColorSpace.sRGB))
        let context = try XCTUnwrap(CGContext(data: nil,
                                              width: width,
                                              height: height,
                                              bitsPerComponent: 8,
                                              bytesPerRow: 0,
                                              space: colorSpace,
                                              bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue))
        context.setFillColor(CGColor(srgbRed: 0, green: 96 / 255, blue: 195 / 255, alpha: 1))
        context.fill(CGRect(x: 0, y: 0, width: width, height: height))
        context.setFillColor(CGColor(srgbRed: 250 / 255, green: 1, blue: 0, alpha: 1))
        context.fill(CGRect(x: width / 2, y: 0, width: width / 2, height: height))
        let cgImage = try XCTUnwrap(context.makeImage())
        return NSImage(cgImage: cgImage, size: NSSize(width: width, height: height))
    }

    private func assertPixel(in representation: NSBitmapImageRep,
                             x: Int,
                             y: Int,
                             hex: Int,
                             file: StaticString = #filePath, line: UInt = #line) {
        var pixel = [Int](repeating: 0, count: 4)
        representation.getPixel(&pixel, atX: x, y: y)
        XCTAssertEqual(pixel[0], (hex >> 16) & 0xFF, accuracy: 2, file: file, line: line)
        XCTAssertEqual(pixel[1], (hex >> 8) & 0xFF, accuracy: 2, file: file, line: line)
        XCTAssertEqual(pixel[2], hex & 0xFF, accuracy: 2, file: file, line: line)
        XCTAssertEqual(pixel[3], 255, accuracy: 1, file: file, line: line)
    }
}
