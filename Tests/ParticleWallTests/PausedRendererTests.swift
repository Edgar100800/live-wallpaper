import AppKit
import XCTest
@testable import ParticleWall

final class PausedRendererTests: XCTestCase {
    func testMetalSnapshotsDoNotAdvancePausedAnimationOrFlow() throws {
        for kind in [WallpaperRendererKind.metalParticles, .metalNoiseRain] {
            let frame = NSRect(x: 0, y: 0, width: 128, height: 128)
            let renderer = try XCTUnwrap(MetalParticleRenderer(frame: frame, kind: kind))
            let window = makeWindow(frame: frame, view: renderer.view)
            defer {
                renderer.tearDown()
                window.orderOut(nil)
            }
            renderer.setPlayback(paused: true, fpsCap: 30, adaptiveQuality: false)
            renderer.applyControlValues(["speed": 3])

            capture(renderer)
            let time = renderer.animationTime
            let flowFrame = renderer.flowFrame
            if kind == .metalNoiseRain { XCTAssertGreaterThan(flowFrame, 0) }
            waitForClockTick()
            capture(renderer)
            waitForClockTick()
            capture(renderer)

            XCTAssertEqual(renderer.animationTime, time)
            XCTAssertEqual(renderer.flowFrame, flowFrame)

            renderer.setPlayback(paused: false, fpsCap: 30, adaptiveQuality: false)
            waitForClockTick()
            capture(renderer)
            XCTAssertGreaterThan(renderer.animationTime, time)
        }
    }

    func testASCIISnapshotDrawsStayFrozenAndResume() throws {
        let root = FileManager.default.temporaryDirectory
            .appendingPathComponent(UUID().uuidString, isDirectory: true)
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: root) }
        try asciiFixture().write(to: root.appendingPathComponent("clip.asciivideo"))

        let frame = NSRect(x: 0, y: 0, width: 128, height: 128)
        let renderer = try XCTUnwrap(ASCIIWallpaperRenderer(frame: frame, rootURL: root))
        let window = makeWindow(frame: frame, view: renderer.view)
        defer {
            renderer.tearDown()
            window.orderOut(nil)
        }
        renderer.setPlayback(paused: true, fpsCap: 30, adaptiveQuality: false)
        renderer.metalView.draw()
        let time = renderer.animationTime
        waitForClockTick()
        renderer.metalView.draw()
        waitForClockTick()
        renderer.metalView.draw()
        XCTAssertEqual(renderer.animationTime, time)
        XCTAssertTrue(renderer.diagnosticSummary.contains("frames:3"))

        renderer.setPlayback(paused: false, fpsCap: 30, adaptiveQuality: false)
        waitForClockTick()
        XCTAssertGreaterThan(renderer.animationTime, time)
    }

    private func capture(_ renderer: WallpaperRenderer) {
        let done = expectation(description: "Explicit snapshot completes")
        renderer.captureSnapshot(targetPixelSize: CGSize(width: 128, height: 128)) { image in
            XCTAssertNotNil(image)
            done.fulfill()
        }
        wait(for: [done], timeout: 3)
    }

    private func waitForClockTick() {
        let done = expectation(description: "Animation clock advances")
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.2) { done.fulfill() }
        wait(for: [done], timeout: 2)
    }

    private func makeWindow(frame: NSRect, view: NSView) -> NSWindow {
        let window = NSWindow(contentRect: frame, styleMask: [.borderless],
                              backing: .buffered, defer: false)
        window.isReleasedWhenClosed = false
        window.contentView?.addSubview(view)
        window.orderBack(nil)
        return window
    }

    private func asciiFixture() -> Data {
        var data = Data("PWASCII1".utf8)
        for value in [UInt16(1), 48, 8, 0] {
            var littleEndian = value.littleEndian
            withUnsafeBytes(of: &littleEndian) { data.append(contentsOf: $0) }
        }
        for value in [UInt32(8), 8, 1, 1, 2, 30, 1, 0] {
            var littleEndian = value.littleEndian
            withUnsafeBytes(of: &littleEndian) { data.append(contentsOf: $0) }
        }
        data.append(contentsOf: [3, 0b1110_0000, 3, 0b0001_1111])
        return data
    }
}
