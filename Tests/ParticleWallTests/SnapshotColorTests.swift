import AppKit
import MetalKit
import WebKit
import XCTest
@testable import ParticleWall

final class SnapshotColorTests: XCTestCase {
    func testWebSnapshotPreservesBackgroundAndParticleColors() throws {
        let frame = NSRect(x: 0, y: 0, width: 64, height: 64)
        let window = makeWindow(frame: frame)
        let renderer = WebWallpaperRenderer(frame: frame)
        let navigation = NavigationProbe(expectation: expectation(description: "Web content loaded"))
        renderer.webView.navigationDelegate = navigation
        window.contentView?.addSubview(renderer.view)
        window.orderBack(nil)
        defer {
            renderer.tearDown()
            window.orderOut(nil)
        }

        renderer.webView.loadHTMLString(
            """
            <html><body style="margin:0;background:#0060C3;overflow:hidden">
            <div style="position:absolute;right:0;top:0;width:50%;height:100%;background:#FAFF00"></div>
            </body></html>
            """,
            baseURL: nil
        )
        wait(for: [navigation.expectation], timeout: 3)

        let captured = expectation(description: "Web snapshot captured")
        var image: NSImage?
        renderer.captureSnapshot(targetPixelSize: CGSize(width: 64, height: 64)) {
            image = $0
            captured.fulfill()
        }
        wait(for: [captured], timeout: 3)

        let representation = try persistAndLoad(try XCTUnwrap(image), size: CGSize(width: 64, height: 64))
        assertPixel(in: representation, x: 8, y: 32, hex: 0x0060C3)
        assertPixel(in: representation, x: 56, y: 32, hex: 0xFAFF00)
    }

    func testMetalSnapshotKeepsConfiguredPalette() throws {
        let frame = NSRect(x: 0, y: 0, width: 128, height: 128)
        let window = makeWindow(frame: frame)
        let renderer = try XCTUnwrap(MetalParticleRenderer(frame: frame, kind: .metalParticles))
        window.contentView?.addSubview(renderer.view)
        window.orderBack(nil)
        renderer.applyControlValues([
            "backgroundColor": Double(0x0060C3),
            "particleColor": Double(0xFAFF00),
            "particleSize": 6,
            "brightness": 1
        ])
        renderer.setPlayback(paused: true, fpsCap: 30, adaptiveQuality: false)
        defer {
            renderer.tearDown()
            window.orderOut(nil)
        }

        let captured = expectation(description: "Metal snapshot captured")
        var image: NSImage?
        renderer.captureSnapshot(targetPixelSize: CGSize(width: 128, height: 128)) {
            image = $0
            captured.fulfill()
        }
        wait(for: [captured], timeout: 3)

        let representation = try persistAndLoad(try XCTUnwrap(image),
                                                size: CGSize(width: 128, height: 128))
        var backgroundPixels = 0
        var particlePixels = 0
        for y in stride(from: 0, to: representation.pixelsHigh, by: 2) {
            for x in stride(from: 0, to: representation.pixelsWide, by: 2) {
                var pixel = [Int](repeating: 0, count: 4)
                representation.getPixel(&pixel, atX: x, y: y)
                if pixel[2] > 153 && pixel[1] > 64 && pixel[0] < 38 {
                    backgroundPixels += 1
                }
                if pixel[0] > 166 && pixel[1] > 166 && pixel[2] < 89 {
                    particlePixels += 1
                }
            }
        }
        XCTAssertGreaterThan(backgroundPixels, 500)
        XCTAssertGreaterThan(particlePixels, 0)
    }

    private func makeWindow(frame: NSRect) -> NSWindow {
        let window = NSWindow(contentRect: frame,
                              styleMask: [.borderless],
                              backing: .buffered,
                              defer: false)
        window.isReleasedWhenClosed = false
        window.level = NSWindow.Level(rawValue: Int(CGWindowLevelForKey(.desktopWindow)))
        window.ignoresMouseEvents = true
        return window
    }

    private func persistAndLoad(_ image: NSImage, size: CGSize) throws -> NSBitmapImageRep {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        defer { try? FileManager.default.removeItem(at: root) }
        let store = LastFrameStore(rootURL: root)
        let url = try XCTUnwrap(store.persist(image,
                                              displayUUID: "test-display",
                                              wallpaperID: UUID(),
                                              targetPixelSize: size))
        return try XCTUnwrap(NSBitmapImageRep(data: Data(contentsOf: url)))
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

private final class NavigationProbe: NSObject, WKNavigationDelegate {
    let expectation: XCTestExpectation

    init(expectation: XCTestExpectation) {
        self.expectation = expectation
    }

    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
        expectation.fulfill()
    }
}
