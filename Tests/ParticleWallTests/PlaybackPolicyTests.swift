import XCTest
@testable import ParticleWall

final class PlaybackPolicyTests: XCTestCase {
    func testNormalPlayback() {
        let policy = PlaybackPolicy.resolve(userPaused: false,
                                            powerSave: false,
                                            systemUnavailable: false,
                                            batteryPause: false)

        XCTAssertEqual(policy, PlaybackPolicy(paused: false,
                                              deepSleep: false,
                                              preservingFrame: true,
                                              persistSnapshot: false))
    }

    func testManualPauseIsLightweight() {
        let policy = PlaybackPolicy.resolve(userPaused: true,
                                            powerSave: false,
                                            systemUnavailable: false,
                                            batteryPause: false)

        XCTAssertTrue(policy.paused)
        XCTAssertFalse(policy.deepSleep)
        XCTAssertTrue(policy.preservingFrame)
        XCTAssertFalse(policy.persistSnapshot)
    }

    func testEnergyModesDeepSleep() {
        for inputs in [(true, false), (false, true)] {
            let policy = PlaybackPolicy.resolve(userPaused: false,
                                                powerSave: inputs.0,
                                                systemUnavailable: false,
                                                batteryPause: inputs.1)

            XCTAssertTrue(policy.paused)
            XCTAssertTrue(policy.deepSleep)
            XCTAssertTrue(policy.preservingFrame)
            XCTAssertFalse(policy.persistSnapshot)
        }
    }

    func testSystemUnavailablePreservesAndPersistsFrame() {
        let policy = PlaybackPolicy.resolve(userPaused: false,
                                            powerSave: false,
                                            systemUnavailable: true,
                                            batteryPause: false)

        XCTAssertTrue(policy.paused)
        XCTAssertTrue(policy.deepSleep)
        XCTAssertTrue(policy.preservingFrame)
        XCTAssertTrue(policy.persistSnapshot)
    }
}
