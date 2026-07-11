import AppKit
import SwiftUI

final class AppDelegate: NSObject, NSApplicationDelegate {
    private var statusItem: NSStatusItem!
    private var galleryWindow: NSWindow?
    private var settingsWindow: NSWindow?

    func applicationDidFinishLaunching(_ notification: Notification) {
        UserDefaults.standard.register(defaults: [
            DefaultsKey.fpsCap: 30,
            DefaultsKey.renderScale: 1.5
        ])
        setupStatusItem()

        LibraryManager.shared.loadLibrary()
        ImportPipeline.shared.upgradeModuleWallpapers()
        WallpaperManager.shared.start()
        PowerManager.shared.start()
        WallpaperManager.shared.restoreAllAssignments()
        LibraryManager.shared.installBundledDefaultIfNeeded()
        handleCLIImport()
        handleCLIDiag()
        handleCLIPowerSaveTest()
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
                    NSLog("ParticleWall: diag screen \(uuid.prefix(8)): deep asleep (no webview)")
                    continue
                }
                controller.webView.evaluateJavaScript("window.__pwFrameCount|0") { start, _ in
                    let start = start as? Int ?? 0
                    DispatchQueue.main.asyncAfter(deadline: .now() + 2) {
                        controller.webView.evaluateJavaScript(
                            "'cap:' + window.__pwFPSCap + ' dpr:' + window.devicePixelRatio" +
                            " + ' inner:' + window.innerWidth + '/' + document.documentElement.clientWidth" +
                            " + ' frames:' + ((window.__pwFrameCount|0) - \(start))" +
                            " + ' paused:' + window.__pwPaused + ' errors:' + JSON.stringify(window.__pwErrors || [])"
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
        if let window = galleryWindow, window.isVisible {
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
            window.isReleasedWhenClosed = false
            window.center()
            galleryWindow = window
        }
        NSApp.activate(ignoringOtherApps: true)
        galleryWindow?.makeKeyAndOrderFront(nil)
    }

    @objc private func openSettings() {
        if settingsWindow == nil {
            let hosting = NSHostingController(rootView: SettingsView())
            let window = NSWindow(contentViewController: hosting)
            window.title = "Ajustes de ParticleWall"
            window.styleMask = [.titled, .closable]
            window.isReleasedWhenClosed = false
            window.center()
            settingsWindow = window
        }
        NSApp.activate(ignoringOtherApps: true)
        settingsWindow?.makeKeyAndOrderFront(nil)
    }

    @objc private func quit() {
        NSApp.terminate(nil)
    }
}
