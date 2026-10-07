import AppKit
import SwiftUI

final class AppDelegate: NSObject, NSApplicationDelegate {
    private var statusItem: NSStatusItem!
    private var galleryWindow: NSWindow?
    private var settingsWindow: NSWindow?
    private var rendererSmokeWindows: [NSWindow] = []
    private var rendererSmokeTests: [MetalParticleRenderer] = []
    private var terminationPending = false
    private var terminationPrepared = false

    func applicationDidFinishLaunching(_ notification: Notification) {
        UserDefaults.standard.register(defaults: [
            DefaultsKey.fpsCap: 30,
            DefaultsKey.renderScale: 1.5,
            DefaultsKey.adaptiveQuality: true
        ])
        setupStatusItem()

        LibraryManager.shared.loadLibrary()
        ImportPipeline.shared.upgradeModuleWallpapers()
        LibraryManager.shared.upgradeBundledWallpapers()
        PowerManager.shared.start()
        WallpaperManager.shared.start()
        LibraryManager.shared.installBundledDefaultIfNeeded()
        handleCLIImport()
        handleCLIApplyBundledDefault()
        handleCLIDiag()
        handleCLIRendererSmokeTest()
        handleCLIPowerSaveTest()
    }

    /// `ParticleWall --apply-bundled-default` selects the native bundled
    /// animation on every screen. Useful for upgrades and automated verification.
    private func handleCLIApplyBundledDefault() {
        guard CommandLine.arguments.contains("--apply-bundled-default") else { return }
        if LibraryManager.shared.applyBundledDefault() {
            NSLog("ParticleWall: applied native bundled default")
        } else {
            NSLog("ParticleWall: bundled default is unavailable")
        }
    }

    /// `ParticleWall --renderer-smoke-test` creates every native renderer for
    /// two seconds and logs frame counts. It does not alter assignments.
    private func handleCLIRendererSmokeTest() {
        guard CommandLine.arguments.contains("--renderer-smoke-test") else { return }
        for (index, kind) in WallpaperRendererKind.nativeMetalCases.enumerated() {
            if kind == .asciiVideo {
                NSLog("ParticleWall: renderer smoke test skips ascii-video; it requires imported frame data")
                continue
            }
            let window = NSWindow(
                contentRect: NSRect(x: -400 - index * 270, y: -400, width: 256, height: 256),
                styleMask: [.borderless],
                backing: .buffered,
                defer: false
            )
            guard let renderer = MetalParticleRenderer(frame: window.contentView?.bounds ?? .zero,
                                                       kind: kind) else {
                NSLog("ParticleWall: renderer smoke test failed to create \(kind.rawValue)")
                continue
            }
            renderer.applyControlValues([
                "graphEnabled": 1,
                "graphDistance": 0.085,
                "graphConnections": 2,
                "graphOpacity": 10,
                "brightness": 10
            ])
            window.contentView?.addSubview(renderer.view)
            renderer.setPlayback(paused: false, fpsCap: 30, adaptiveQuality: false)
            rendererSmokeWindows.append(window)
            rendererSmokeTests.append(renderer)
            window.orderBack(nil)
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 1) { [weak self] in
            self?.rendererSmokeTests.forEach { renderer in
                renderer.captureSnapshot(targetPixelSize: CGSize(width: 512, height: 512)) { image in
                    if let image {
                        NSLog("ParticleWall: renderer snapshot \(renderer.kind.rawValue) " +
                              "\(Int(image.size.width))x\(Int(image.size.height))")
                    } else {
                        NSLog("ParticleWall: renderer snapshot failed \(renderer.kind.rawValue)")
                    }
                }
            }
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 2) { [weak self] in
            guard let self else { return }
            self.rendererSmokeTests.forEach {
                NSLog("ParticleWall: renderer smoke test \($0.diagnosticSummary)")
                $0.tearDown()
            }
            self.rendererSmokeWindows.forEach { $0.orderOut(nil) }
            self.rendererSmokeTests.removeAll()
            self.rendererSmokeWindows.removeAll()
        }
    }

    /// `ParticleWall --powersave-test`: toggles Power Save on at +8s and off at
    /// +16s, logging deep-sleep state so the teardown/restore path is testable
    /// without UI interaction.
    private func handleCLIPowerSaveTest() {
        guard CommandLine.arguments.contains("--powersave-test") else { return }
        DispatchQueue.main.asyncAfter(deadline: .now() + 8) {
            PowerManager.shared.powerSave = true
            DispatchQueue.main.asyncAfter(deadline: .now() + 3) {
                let asleep = WallpaperManager.shared.controllers.values.map(\.isDeepAsleep)
                NSLog("ParticleWall: powersave-test deep sleep states: \(asleep)")
            }
        }
        DispatchQueue.main.asyncAfter(deadline: .now() + 16) {
            PowerManager.shared.powerSave = false
            DispatchQueue.main.asyncAfter(deadline: .now() + 3) {
                let asleep = WallpaperManager.shared.controllers.values.map(\.isDeepAsleep)
                NSLog("ParticleWall: powersave-test restored, deep sleep states: \(asleep)")
            }
        }
    }

    /// `ParticleWall --diag` logs the effective FPS cap and measured FPS of every
    /// wallpaper window a few seconds after launch.
    private func handleCLIDiag() {
        guard CommandLine.arguments.contains("--diag") else { return }
        DispatchQueue.main.asyncAfter(deadline: .now() + 6) {
            for (uuid, controller) in WallpaperManager.shared.controllers {
                if controller.isDeepAsleep {
                    NSLog("ParticleWall: diag screen \(uuid.prefix(8)): deep asleep (no renderer)")
                    continue
                }
                guard let webView = controller.webView else {
                    NSLog("ParticleWall: diag screen \(uuid.prefix(8)): \(controller.diagnosticSummary)")
                    continue
                }
                webView.evaluateJavaScript("window.__pwFrameCount|0") { start, _ in
                    let start = start as? Int ?? 0
                    DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
                        webView.evaluateJavaScript(
                            "'cap:' + window.__pwFPSCap + ' dpr:' + window.devicePixelRatio" +
                            " + ' inner:' + window.innerWidth + '/' + document.documentElement.clientWidth" +
                            " + ' frames:' + ((window.__pwFrameCount|0) - \(start))" +
                            " + ' paused:' + window.__pwPaused" +
                            " + ' adaptiveCap:' + (window.__pwAdaptiveCap || 0)" +
                            " + ' frameCost:' + Number(window.__pwAverageFrameCost || 0).toFixed(2)" +
                            " + ' controls:' + JSON.stringify(window.__pwGetControls" +
                            " ? window.__pwGetControls().map(function(c){return c.id;}) : [])" +
                            " + ' errors:' + JSON.stringify(window.__pwErrors || [])"
                        ) { result, _ in
                            NSLog("ParticleWall: diag screen \(uuid.prefix(8)): \(result ?? "nil") in 2s")
                        }
                    }
                }
            }
        }
    }

    /// `ParticleWall --import <path>` imports a wallpaper from the command line.
    private func handleCLIImport() {
        let args = CommandLine.arguments
        guard let index = args.firstIndex(of: "--import"), args.count > index + 1 else { return }
        let url = URL(fileURLWithPath: args[index + 1])
        do {
            let wallpaper = try ImportPipeline.shared.importItem(at: url)
            NSLog("ParticleWall: imported \"\(wallpaper.name)\" (\(wallpaper.id))")
        } catch {
            NSLog("ParticleWall: import failed: \(error.localizedDescription)")
        }
    }

    // MARK: - Status item

    private func setupStatusItem() {
        statusItem = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        if let button = statusItem.button {
            button.image = NSImage(systemSymbolName: "sparkles",
                                   accessibilityDescription: "ParticleWall")
            button.action = #selector(statusItemClicked)
            button.target = self
            button.sendAction(on: [.leftMouseUp, .rightMouseUp])
        }
    }

    @objc private func statusItemClicked() {
        if NSApp.currentEvent?.type == .rightMouseUp {
            showQuickMenu()
        } else {
            toggleGallery()
        }
    }

    private func showQuickMenu() {
        let menu = NSMenu()

        let power = PowerManager.shared
        let playPause = NSMenuItem(title: power.userPaused ? "Reanudar" : "Pausar",
                                   action: #selector(togglePlayPause), keyEquivalent: "")
        playPause.target = self
        menu.addItem(playPause)

        let powerSave = NSMenuItem(title: "Power Save",
                                   action: #selector(togglePowerSave), keyEquivalent: "")
        powerSave.target = self
        powerSave.state = power.powerSave ? .on : .off
        menu.addItem(powerSave)

        let fpsItem = NSMenuItem(title: "Límite de FPS", action: nil, keyEquivalent: "")
        let fpsMenu = NSMenu()
        let currentCap = UserDefaults.standard.integer(forKey: DefaultsKey.fpsCap)
        for (title, value) in [("Sin límite", 0), ("15 fps", 15), ("30 fps", 30),
                               ("60 fps", 60), ("120 fps", 120)] {
            let item = NSMenuItem(title: title, action: #selector(setGlobalFPS(_:)), keyEquivalent: "")
            item.target = self
            item.tag = value
            item.state = currentCap == value ? .on : .off
            fpsMenu.addItem(item)
        }
        fpsItem.submenu = fpsMenu
        menu.addItem(fpsItem)

        menu.addItem(.separator())

        let gallery = NSMenuItem(title: "Abrir galería", action: #selector(openGallery), keyEquivalent: "g")
        gallery.target = self
        menu.addItem(gallery)

        let settings = NSMenuItem(title: "Ajustes…", action: #selector(openSettings), keyEquivalent: ",")
        settings.target = self
        menu.addItem(settings)

        menu.addItem(.separator())

        let quit = NSMenuItem(title: "Salir de ParticleWall", action: #selector(quit), keyEquivalent: "q")
        quit.target = self
        menu.addItem(quit)

        // Transient menu: attach, click, detach — keeps left-click free for the gallery.
        statusItem.menu = menu
        statusItem.button?.performClick(nil)
        statusItem.menu = nil
    }

    // MARK: - Actions

    @objc private func togglePlayPause() {
        PowerManager.shared.userPaused.toggle()
    }

    @objc private func togglePowerSave() {
        PowerManager.shared.powerSave.toggle()
    }

    @objc private func setGlobalFPS(_ sender: NSMenuItem) {
        UserDefaults.standard.set(sender.tag, forKey: DefaultsKey.fpsCap)
        // PowerManager observes UserDefaults changes and fans the new cap out.
    }

    @objc private func openGallery() {
        showGallery()
    }

    @objc private func toggleGallery() {
        if let window = galleryWindow,
           window.isVisible,
           window.isOnActiveSpace,
           !window.isMiniaturized {
            NotificationCenter.default.post(name: .pwGalleryDidHide, object: nil)
            window.orderOut(nil)
        } else {
            showGallery()
        }
    }

    private func showGallery() {
        if galleryWindow == nil {
            let hosting = NSHostingController(rootView: GalleryView()
                .environmentObject(LibraryManager.shared))
            let window = NSWindow(contentViewController: hosting)
            window.title = "ParticleWall"
            window.styleMask = [.titled, .closable, .miniaturizable, .resizable]
            window.setContentSize(NSSize(width: 720, height: 480))
            window.minSize = NSSize(width: 520, height: 360)
            window.collectionBehavior = [.moveToActiveSpace, .fullScreenAuxiliary]
            window.isReleasedWhenClosed = false
            window.delegate = self
            window.center()
            galleryWindow = window
        }
        if let window = galleryWindow {
            presentOnCurrentSpace(window)
        }
    }

    @objc private func openSettings() {
        if settingsWindow == nil {
            let hosting = NSHostingController(rootView: SettingsView())
            let window = NSWindow(contentViewController: hosting)
            window.title = "Ajustes de ParticleWall"
            window.styleMask = [.titled, .closable]
            window.collectionBehavior = [.moveToActiveSpace, .fullScreenAuxiliary]
            window.isReleasedWhenClosed = false
            window.center()
            settingsWindow = window
        }
        if let window = settingsWindow {
            presentOnCurrentSpace(window)
        }
    }

    private func presentOnCurrentSpace(_ window: NSWindow) {
        if window.isMiniaturized {
            window.deminiaturize(nil)
        }
        // Move/order the requested window before activating the app, so its
        // previously active window does not take us back to another Space.
        window.makeKeyAndOrderFront(nil)
        NSApp.activate()
    }

    @objc private func quit() {
        NSApp.terminate(nil)
    }

    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        if terminationPrepared { return .terminateNow }
        if terminationPending { return .terminateLater }

        terminationPending = true
        WallpaperManager.shared.prepareForTermination { [weak self, weak sender] in
            guard let self else { return }
            self.terminationPrepared = true
            self.terminationPending = false
            sender?.reply(toApplicationShouldTerminate: true)
        }
        return .terminateLater
    }
}

extension AppDelegate: NSWindowDelegate {
    func windowWillClose(_ notification: Notification) {
        guard let window = notification.object as? NSWindow,
              window === galleryWindow else { return }
        NotificationCenter.default.post(name: .pwGalleryDidHide, object: nil)
    }
}
