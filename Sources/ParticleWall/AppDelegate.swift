import AppKit
import SwiftUI

final class AppDelegate: NSObject, NSApplicationDelegate {
    private var statusItem: NSStatusItem!
    private var galleryWindow: NSWindow?
    private var settingsWindow: NSWindow?

    func applicationDidFinishLaunching(_ notification: Notification) {
        setupStatusItem()

        LibraryManager.shared.loadLibrary()
        WallpaperManager.shared.start()
        PowerManager.shared.start()
        WallpaperManager.shared.restoreAllAssignments()
        LibraryManager.shared.installBundledDefaultIfNeeded()
        handleCLIImport()
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
