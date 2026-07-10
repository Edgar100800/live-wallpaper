import AppKit
import IOKit.ps

/// Central pause policy. Combines session lock, screen sleep, battery state,
/// user Power Save / Play-Pause toggles and pushes the result to every
/// wallpaper window. Per-window occlusion is handled by each controller.
final class PowerManager {
    static let shared = PowerManager()

    private let defaults = UserDefaults.standard
    private var screenLocked = false
    private var screensAsleep = false
    private var sessionInactive = false
    private(set) var onBattery = false

    private init() {}

    func start() {
        let dnc = DistributedNotificationCenter.default()
        dnc.addObserver(self, selector: #selector(screenDidLock),
                        name: Notification.Name("com.apple.screenIsLocked"), object: nil)
        dnc.addObserver(self, selector: #selector(screenDidUnlock),
                        name: Notification.Name("com.apple.screenIsUnlocked"), object: nil)

        let wnc = NSWorkspace.shared.notificationCenter
        wnc.addObserver(self, selector: #selector(screensDidSleep),
                        name: NSWorkspace.screensDidSleepNotification, object: nil)
        wnc.addObserver(self, selector: #selector(screensDidWake),
                        name: NSWorkspace.screensDidWakeNotification, object: nil)
        wnc.addObserver(self, selector: #selector(sessionDidResignActive),
                        name: NSWorkspace.sessionDidResignActiveNotification, object: nil)
        wnc.addObserver(self, selector: #selector(sessionDidBecomeActive),
                        name: NSWorkspace.sessionDidBecomeActiveNotification, object: nil)

        NotificationCenter.default.addObserver(self, selector: #selector(settingsChanged),
                                               name: UserDefaults.didChangeNotification, object: nil)
        NotificationCenter.default.addObserver(self, selector: #selector(settingsChanged),
                                               name: .NSProcessInfoPowerStateDidChange, object: nil)

        // IOKit power source change callback (plug/unplug). C callback, no context:
        // route through the singleton.
        if let source = IOPSNotificationCreateRunLoopSource({ _ in
            DispatchQueue.main.async {
                PowerManager.shared.refreshPowerState()
            }
        }, nil)?.takeRetainedValue() {
            CFRunLoopAddSource(CFRunLoopGetMain(), source, .defaultMode)
        }

        refreshPowerState()
    }

    // MARK: - State inputs

    @objc private func screenDidLock() { screenLocked = true; apply() }
    @objc private func screenDidUnlock() {
        screenLocked = false
        apply()
        // Known WebKit issue: renders can stay frozen after unlock.
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
            WallpaperManager.shared.kickAllAfterUnlock()
        }
    }
    @objc private func screensDidSleep() { screensAsleep = true; apply() }
    @objc private func screensDidWake() { screensAsleep = false; apply() }
    @objc private func sessionDidResignActive() { sessionInactive = true; apply() }
    @objc private func sessionDidBecomeActive() {
        sessionInactive = false
        apply()
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.5) {
            WallpaperManager.shared.kickAllAfterUnlock()
        }
    }
    @objc private func settingsChanged() { apply() }

    func refreshPowerState() {
        onBattery = Self.isOnBatteryPower()
        apply()
    }

    private static func isOnBatteryPower() -> Bool {
        guard let snapshot = IOPSCopyPowerSourcesInfo()?.takeRetainedValue(),
              let sources = IOPSCopyPowerSourcesList(snapshot)?.takeRetainedValue() as? [CFTypeRef] else {
            return false
        }
        for source in sources {
            if let info = IOPSGetPowerSourceDescription(snapshot, source)?.takeUnretainedValue() as? [String: Any],
               let state = info[kIOPSPowerSourceStateKey] as? String {
                return state == kIOPSBatteryPowerValue
            }
        }
        return false
    }

    // MARK: - Policy

    var userPaused: Bool {
        get { defaults.bool(forKey: DefaultsKey.globallyPaused) }
        set { defaults.set(newValue, forKey: DefaultsKey.globallyPaused) }
    }

    var powerSave: Bool {
        get { defaults.bool(forKey: DefaultsKey.powerSave) }
        set { defaults.set(newValue, forKey: DefaultsKey.powerSave) }
    }

    var shouldPause: Bool {
        if userPaused || powerSave { return true }
        if screenLocked || screensAsleep || sessionInactive { return true }
        if defaults.bool(forKey: DefaultsKey.pauseOnBattery) {
            if onBattery || ProcessInfo.processInfo.isLowPowerModeEnabled { return true }
        }
        return false
    }

    var fpsCap: Int {
        let value = defaults.integer(forKey: DefaultsKey.fpsCap)
        return value > 0 ? value : 0
    }

    private var lastPushedPaused: Bool?
    private var lastPushedCap: Int?

    private func apply() {
        let paused = shouldPause
        let cap = fpsCap
        guard paused != lastPushedPaused || cap != lastPushedCap else { return }
        lastPushedPaused = paused
        lastPushedCap = cap
        WallpaperManager.shared.setGlobalPaused(paused, fpsCap: cap)
        NotificationCenter.default.post(name: .pwPlaybackStateChanged, object: nil)
    }

    func pushStateToAllControllers() {
        lastPushedPaused = nil
        lastPushedCap = nil
        apply()
    }
}
