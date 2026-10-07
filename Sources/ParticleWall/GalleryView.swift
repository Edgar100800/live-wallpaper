import SwiftUI
import WebKit
import UniformTypeIdentifiers

struct GalleryView: View {
    @EnvironmentObject var library: LibraryManager
    @State private var searchText = ""
    @State private var target: ScreenTarget = .allScreens
    @State private var isDropTargeted = false
    @State private var importError: String?
    @State private var renaming: Wallpaper?
    @State private var renameText = ""
    @State private var activeRefresh = 0
    @State private var customizing: Wallpaper?

    private var filteredWallpapers: [Wallpaper] {
        guard !searchText.isEmpty else { return library.wallpapers }
        return library.wallpapers.filter {
            $0.name.localizedCaseInsensitiveContains(searchText)
        }
    }

    var body: some View {
        VStack(spacing: 0) {
            toolbar
            Divider()
            if filteredWallpapers.isEmpty {
                emptyState
            } else {
                grid
            }
        }
        .frame(minWidth: 520, minHeight: 360)
        .onDrop(of: [.fileURL], isTargeted: $isDropTargeted, perform: handleDrop)
        .overlay(dropOverlay)
        .onReceive(NotificationCenter.default.publisher(for: .pwPlaybackStateChanged)) { _ in
            activeRefresh += 1
        }
        .alert("Error al importar", isPresented: Binding(
            get: { importError != nil },
            set: { if !$0 { importError = nil } }
        )) {
            Button("OK", role: .cancel) {}
        } message: {
            Text(importError ?? "")
        }
        .alert("Renombrar wallpaper", isPresented: Binding(
            get: { renaming != nil },
            set: { if !$0 { renaming = nil } }
        )) {
            TextField("Nombre", text: $renameText)
            Button("Renombrar") {
                if let wallpaper = renaming, !renameText.isEmpty {
                    library.rename(wallpaper, to: renameText)
                }
                renaming = nil
            }
            Button("Cancelar", role: .cancel) { renaming = nil }
        }
        .sheet(item: $customizing) { wallpaper in
            WallpaperCustomizationView(wallpaper: wallpaper, target: target)
        }
    }

    // MARK: - Toolbar

    private var toolbar: some View {
        HStack(spacing: 12) {
            HStack(spacing: 6) {
                Image(systemName: "magnifyingglass").foregroundStyle(.secondary)
                TextField("Buscar", text: $searchText)
                    .textFieldStyle(.plain)
            }
            .padding(6)
            .background(RoundedRectangle(cornerRadius: 8).fill(.quaternary.opacity(0.5)))
            .frame(maxWidth: 220)

            Spacer()

            screenPicker

            Button {
                openImportPanel()
            } label: {
                Label("Importar", systemImage: "plus")
            }
            .keyboardShortcut("i", modifiers: .command)
        }
        .padding(12)
    }

    private var screenPicker: some View {
        let screens = WallpaperManager.shared.screensByUUID
        return Picker("Aplicar en:", selection: $target) {
            Text("Todas las pantallas").tag(ScreenTarget.allScreens)
            ForEach(Array(screens.enumerated()), id: \.element.uuid) { index, entry in
                Text(entry.screen.localizedName.isEmpty ? "Pantalla \(index + 1)" : entry.screen.localizedName)
                    .tag(ScreenTarget.screen(displayUUID: entry.uuid))
            }
        }
        .pickerStyle(.menu)
        .fixedSize()
        .opacity(screens.count > 1 ? 1 : 0)     // hide selector on single-monitor setups
        .frame(maxWidth: screens.count > 1 ? nil : 0)
    }

    // MARK: - Grid

    private var grid: some View {
        ScrollView {
            LazyVGrid(columns: [GridItem(.adaptive(minimum: 180), spacing: 16)], spacing: 16) {
                ForEach(filteredWallpapers) { wallpaper in
                    WallpaperCard(
                        wallpaper: wallpaper,
                        isActive: WallpaperManager.shared.activeWallpaperID(on: target) == wallpaper.id,
                        onApply: { WallpaperManager.shared.apply(wallpaper, to: target) },
                        onCustomize: {
                            WallpaperManager.shared.apply(wallpaper, to: target)
                            customizing = wallpaper
                        },
                        onRename: { renaming = wallpaper; renameText = wallpaper.name },
                        onRegenerate: { library.regenerateThumbnail(wallpaper) },
                        onReveal: { library.revealInFinder(wallpaper) },
                        onDelete: { library.delete(wallpaper) },
                        onSetFPS: { library.setFPS(wallpaper, fps: $0) }
                    )
                    .id("\(wallpaper.id)-\(activeRefresh)")
                }
            }
            .padding(16)
        }
    }

    private var emptyState: some View {
        VStack(spacing: 10) {
            Image(systemName: "sparkles.rectangle.stack")
                .font(.system(size: 44))
                .foregroundStyle(.secondary)
            Text("Sin wallpapers")
                .font(.title3)
            Text("Arrastra un .html, .zip, carpeta o snippet .js aquí,\no usa el botón Importar.")
                .font(.callout)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .frame(maxWidth: .infinity, maxHeight: .infinity)
    }

    private var dropOverlay: some View {
        RoundedRectangle(cornerRadius: 12)
            .strokeBorder(Color.accentColor, style: StrokeStyle(lineWidth: 3, dash: [8]))
            .padding(6)
            .opacity(isDropTargeted ? 1 : 0)
            .allowsHitTesting(false)
    }

    // MARK: - Import

    private func openImportPanel() {
        let panel = NSOpenPanel()
        panel.canChooseFiles = true
        panel.canChooseDirectories = true
        panel.allowsMultipleSelection = true
        var types: [UTType] = [.html, .zip, .folder]
        if let js = UTType(filenameExtension: "js") { types.append(js) }
        if let asciiVideo = UTType(filenameExtension: "asciivideo") { types.append(asciiVideo) }
        panel.allowedContentTypes = types
        panel.message = "Elige un .html, .zip, carpeta, .js o .asciivideo"
        if panel.runModal() == .OK {
            importURLs(panel.urls)
        }
    }

    private func handleDrop(providers: [NSItemProvider]) -> Bool {
        var handled = false
        for provider in providers where provider.hasItemConformingToTypeIdentifier(UTType.fileURL.identifier) {
            handled = true
            provider.loadItem(forTypeIdentifier: UTType.fileURL.identifier, options: nil) { item, _ in
                guard let data = item as? Data,
                      let url = URL(dataRepresentation: data, relativeTo: nil) else { return }
                DispatchQueue.main.async { importURLs([url]) }
            }
        }
        return handled
    }

    private func importURLs(_ urls: [URL]) {
        for url in urls {
            do {
                try ImportPipeline.shared.importItem(at: url)
            } catch {
                importError = error.localizedDescription
            }
        }
    }
}

// MARK: - Card

struct WallpaperCard: View {
    let wallpaper: Wallpaper
    let isActive: Bool
    let onApply: () -> Void
    let onCustomize: () -> Void
    let onRename: () -> Void
    let onRegenerate: () -> Void
    let onReveal: () -> Void
    let onDelete: () -> Void
    let onSetFPS: (Int?) -> Void

    @State private var hovering = false
    @State private var showLivePreview = false

    var body: some View {
        VStack(spacing: 6) {
            ZStack {
                thumbnail
                if showLivePreview {
                    if wallpaper.manifest.effectiveRenderer == .asciiVideo {
                        ASCIIViewPreview(wallpaper: wallpaper).transition(.opacity)
                    } else {
                        WebViewPreview(wallpaper: wallpaper).transition(.opacity)
                    }
                }
                if hovering && !showLivePreview {
                    previewButton
                }
            }
            .frame(height: 110)
            .clipShape(RoundedRectangle(cornerRadius: 10))
            .overlay(
                RoundedRectangle(cornerRadius: 10)
                    .strokeBorder(isActive ? Color.accentColor : Color.clear, lineWidth: 3)
            )
            .overlay(alignment: .bottomTrailing) {
                if hovering {
                    optionsMenu
                        .padding(8)
                        .transition(.opacity.combined(with: .scale(
                            scale: 0.94,
                            anchor: .bottomTrailing
                        )))
                }
            }

            Text(wallpaper.name)
                .font(.callout)
                .lineLimit(1)
        }
        .contentShape(Rectangle())
        .onTapGesture(perform: onApply)
        .onHover { inside in
            hovering = inside
            if !inside { showLivePreview = false }
        }
        .onReceive(NotificationCenter.default.publisher(for: .pwGalleryDidHide)) { _ in
            showLivePreview = false
        }
        .animation(.easeOut(duration: 0.14), value: hovering)
    }

    private var optionsMenu: some View {
        Menu {
            Button("Aplicar", action: onApply)
            Button("Personalizar…", action: onCustomize)
            Menu("Límite de FPS") {
                fpsOption("Global", fps: nil)
                fpsOption("15 fps", fps: 15)
                fpsOption("30 fps", fps: 30)
                fpsOption("60 fps", fps: 60)
                fpsOption("120 fps", fps: 120)
            }
            Divider()
            Button("Renombrar…", action: onRename)
            Button("Regenerar thumbnail", action: onRegenerate)
            Button("Mostrar en Finder", action: onReveal)
            Divider()
            if wallpaper.isBundled {
                Button {} label: {
                    Label("Fondo incluido · Protegido", systemImage: "lock.fill")
                }
                .disabled(true)
            } else {
                Button("Eliminar", role: .destructive, action: onDelete)
            }
        } label: {
            Image(systemName: "ellipsis")
                .font(.system(size: 14, weight: .semibold))
                .foregroundStyle(.white)
                .frame(width: 28, height: 28)
                .background(.black.opacity(0.58), in: RoundedRectangle(cornerRadius: 8))
                .overlay {
                    RoundedRectangle(cornerRadius: 8)
                        .strokeBorder(.white.opacity(0.18))
                }
                .shadow(color: .black.opacity(0.28), radius: 4, y: 2)
        }
        .menuStyle(.borderlessButton)
        .menuIndicator(.hidden)
        .frame(width: 28, height: 28)
        .contentShape(RoundedRectangle(cornerRadius: 8))
        .help("Opciones")
    }

    private func fpsOption(_ label: String, fps: Int?) -> some View {
        Button {
            onSetFPS(fps)
        } label: {
            if wallpaper.manifest.fps == fps {
                Label(label, systemImage: "checkmark")
            } else {
                Text(label)
            }
        }
    }

    private var thumbnail: some View {
        Group {
            if let image = NSImage(contentsOf: wallpaper.thumbnailURL) {
                Image(nsImage: image)
                    .resizable()
                    .aspectRatio(contentMode: .fill)
            } else {
                ZStack {
                    Color.black
                    Image(systemName: "sparkles")
                        .font(.title)
                        .foregroundStyle(.green)
                }
            }
        }
    }

    private var previewButton: some View {
        Button {
            withAnimation(.easeIn(duration: 0.15)) { showLivePreview = true }
        } label: {
            Image(systemName: "play.circle.fill")
                .font(.system(size: 30))
                .foregroundStyle(.white.opacity(0.9))
                .shadow(radius: 4)
        }
        .buttonStyle(.plain)
    }
}

// MARK: - Live preview

struct WebViewPreview: NSViewRepresentable {
    let wallpaper: Wallpaper

    func makeNSView(context: Context) -> WKWebView {
        let webView = WebViewFactory.makeWebView(frame: .zero)
        let delegate = LocalOnlyNavigationDelegate(allowedRoot: wallpaper.folderURL)
        context.coordinator.delegate = delegate
        webView.navigationDelegate = delegate
        webView.loadFileURL(wallpaper.indexURL, allowingReadAccessTo: wallpaper.folderURL)
        return webView
    }

    func updateNSView(_ nsView: WKWebView, context: Context) {}

    static func dismantleNSView(_ nsView: WKWebView, coordinator: Coordinator) {
        nsView.stopLoading()
        nsView.navigationDelegate = nil
        nsView.removeFromSuperview()
        coordinator.delegate?.onDidFinish = nil
        coordinator.delegate = nil
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    final class Coordinator {
        var delegate: LocalOnlyNavigationDelegate?
    }
}

struct ASCIIViewPreview: NSViewRepresentable {
    let wallpaper: Wallpaper

    func makeNSView(context: Context) -> NSView {
        guard let renderer = ASCIIWallpaperRenderer(frame: .zero, rootURL: wallpaper.folderURL) else {
            return NSView()
        }
        context.coordinator.renderer = renderer
        return renderer.view
    }

    func updateNSView(_ nsView: NSView, context: Context) {}

    static func dismantleNSView(_ nsView: NSView, coordinator: Coordinator) {
        coordinator.renderer?.tearDown()
        coordinator.renderer = nil
    }

    func makeCoordinator() -> Coordinator { Coordinator() }

    final class Coordinator {
        var renderer: ASCIIWallpaperRenderer?
    }
}
