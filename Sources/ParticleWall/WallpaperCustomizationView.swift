import SwiftUI
import AppKit

struct WallpaperCustomizationView: View {
    let wallpaper: Wallpaper
    let target: ScreenTarget

    @Environment(\.dismiss) private var dismiss
    @State private var descriptors: [WallpaperControlDescriptor] = []
    @State private var values: [String: Double] = [:]
    @State private var isLoading = true
    @State private var isUsingFallback = false
    @State private var hasUnsavedChanges = false
    @State private var statusMessage = "Configuración cargada"
    @State private var colorProfiles: [ColorProfile] = []
    @State private var colorProfileName = ""

    private let manager = WallpaperManager.shared
    private let colorProfileStore = ColorProfileStore.shared

    var body: some View {
        VStack(spacing: 0) {
            header
            Divider()
            content
            Divider()
            footer
        }
        .frame(width: 520, height: 560)
        .onAppear(perform: loadControls)
        .onDisappear(perform: persistValues)
    }

    private var header: some View {
        HStack(spacing: 12) {
            VStack(alignment: .leading, spacing: 3) {
                Text("Personalizar \(wallpaper.name)")
                    .font(.headline)
                Text(targetDescription)
                    .font(.caption)
                    .foregroundStyle(.secondary)
            }
            Spacer()
            Button("Cerrar") {
                persistValues()
                dismiss()
            }
            .keyboardShortcut(.defaultAction)
        }
        .padding(16)
    }

    @ViewBuilder
    private var content: some View {
        if isLoading {
            VStack(spacing: 12) {
                ProgressView()
                Text("Cargando parámetros del modelo…")
                    .font(.callout)
                    .foregroundStyle(.secondary)
            }
            .frame(maxWidth: .infinity, maxHeight: .infinity)
        } else if descriptors.isEmpty {
            ContentUnavailableView(
                "Sin controles disponibles",
                systemImage: "slider.horizontal.3",
                description: Text("Este wallpaper no expone un modelo compatible, o el renderer está en Power Save. Los módulos ES pueden declarar parámetros personalizados.")
            )
        } else {
            ScrollView {
                VStack(alignment: .leading, spacing: 20) {
                    if supportsColorProfiles {
                        colorProfileSection
                    }
                    ForEach(categories, id: \.self) { category in
                        controlSection(category)
                    }
                }
                .padding(16)
            }
        }
    }

    private var footer: some View {
        VStack(spacing: 10) {
            HStack {
                Button {
                    restoreDefaults()
                } label: {
                    Label("Restaurar valores predeterminados",
                          systemImage: "arrow.counterclockwise")
                }
                .disabled(descriptors.isEmpty)

                Spacer()

                Button("Guardar configuración") {
                    persistValues()
                }
                .keyboardShortcut("s", modifiers: .command)
                .disabled(descriptors.isEmpty || !hasUnsavedChanges)
            }

            HStack(spacing: 6) {
                Image(systemName: hasUnsavedChanges ? "circle.dashed" : "checkmark.circle.fill")
                    .foregroundStyle(hasUnsavedChanges ? Color.secondary : Color.green)
                Text(hasUnsavedChanges ? "Cambios sin guardar" : statusMessage)
                    .font(.caption)
                    .foregroundStyle(.secondary)

                Spacer()

                Text("Vista previa en vivo")
                    .font(.caption)
                    .foregroundStyle(.secondary)

                if isUsingFallback {
                    Text("· Controles básicos")
                        .font(.caption)
                        .foregroundStyle(.secondary)
                }
            }
        }
        .padding(16)
    }

    private var categories: [String] {
        descriptors.reduce(into: []) { result, descriptor in
            if !result.contains(descriptor.category) {
                result.append(descriptor.category)
            }
        }
    }

    private var supportsColorProfiles: Bool {
        let colorIDs = Set(descriptors.lazy
            .filter { $0.resolvedKind == .color }
            .map(\.id))
        return colorIDs.contains("backgroundColor") && colorIDs.contains("particleColor")
    }

    private var colorProfileSection: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("Perfiles de color")
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
                .textCase(.uppercase)

            VStack(alignment: .leading, spacing: 12) {
                if colorProfiles.isEmpty {
                    Label("Guarda ambos colores para crear tu biblioteca personal.",
                          systemImage: "paintpalette")
                        .font(.callout)
                        .foregroundStyle(.secondary)
                } else {
                    ScrollView(.horizontal) {
                        HStack(spacing: 8) {
                            ForEach(colorProfiles) { profile in
                                colorProfileCard(profile)
                            }
                        }
                    }
                    .scrollIndicators(.hidden)
                }

                HStack(spacing: 8) {
                    TextField("Nombre opcional", text: $colorProfileName)
                        .textFieldStyle(.roundedBorder)

                    Button {
                        saveCurrentColorProfile()
                    } label: {
                        Label("Guardar ambos colores", systemImage: "plus")
                    }
                    .disabled(!supportsColorProfiles)
                    .help("Guarda juntos el color del fondo y el de las partículas")
                }
            }
            .padding(12)
            .background(.quaternary.opacity(0.35),
                        in: RoundedRectangle(cornerRadius: 10))
        }
    }

    private func colorProfileCard(_ profile: ColorProfile) -> some View {
        HStack(spacing: 6) {
            Button {
                applyColorProfile(profile)
            } label: {
                HStack(spacing: 8) {
                    ZStack {
                        Circle()
                            .fill(color(for: profile.backgroundColor))
                            .overlay(Circle().stroke(.white.opacity(0.25), lineWidth: 1))
                            .frame(width: 24, height: 24)
                            .offset(x: -5)
                        Circle()
                            .fill(color(for: profile.particleColor))
                            .overlay(Circle().stroke(.black.opacity(0.18), lineWidth: 1))
                            .frame(width: 24, height: 24)
                            .offset(x: 5)
                    }
                    .frame(width: 36)

                    VStack(alignment: .leading, spacing: 1) {
                        Text(profile.name)
                            .font(.callout)
                            .lineLimit(1)
                        Text("\(PackedRGB.hex(profile.backgroundColor)) · \(PackedRGB.hex(profile.particleColor))")
                            .font(.system(.caption2, design: .monospaced))
                            .foregroundStyle(.secondary)
                    }
                }
                .contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .help("Aplicar \(profile.name)")

            Button {
                deleteColorProfile(profile)
            } label: {
                Image(systemName: "trash")
                    .foregroundStyle(.secondary)
            }
            .buttonStyle(.borderless)
            .help("Eliminar \(profile.name)")
        }
        .padding(.horizontal, 9)
        .padding(.vertical, 7)
        .background(.background.opacity(0.55),
                    in: RoundedRectangle(cornerRadius: 8))
        .overlay {
            RoundedRectangle(cornerRadius: 8)
                .stroke(.separator.opacity(0.35), lineWidth: 1)
        }
    }

    private func controlSection(_ category: String) -> some View {
        VStack(alignment: .leading, spacing: 10) {
            Text(category)
                .font(.caption.weight(.semibold))
                .foregroundStyle(.secondary)
                .textCase(.uppercase)

            VStack(spacing: 12) {
                ForEach(descriptors.filter { $0.category == category }) { descriptor in
                    controlRow(descriptor)
                }
            }
            .padding(12)
            .background(.quaternary.opacity(0.35), in: RoundedRectangle(cornerRadius: 10))
        }
    }

    @ViewBuilder
    private func controlRow(_ descriptor: WallpaperControlDescriptor) -> some View {
        if descriptor.resolvedKind == .color {
            colorControlRow(descriptor)
        } else if descriptor.resolvedKind == .boolean {
            Toggle(
                isOn: Binding(
                    get: { (values[descriptor.id] ?? descriptor.defaultValue) >= 0.5 },
                    set: { updateValue($0 ? 1 : 0, for: descriptor.id) }
                )
            ) {
                Text(descriptor.label)
                    .font(.callout)
            }
            .toggleStyle(.switch)
        } else {
            HStack(spacing: 12) {
                Text(descriptor.label)
                    .font(.callout)
                    .frame(width: 112, alignment: .leading)

                Slider(
                    value: Binding(
                        get: { values[descriptor.id] ?? descriptor.defaultValue },
                        set: { updateValue($0, for: descriptor.id) }
                    ),
                    in: descriptor.min...descriptor.max,
                    step: descriptor.step
                )

                Text(formattedValue(values[descriptor.id] ?? descriptor.defaultValue,
                                    step: descriptor.step))
                    .font(.system(.caption, design: .monospaced))
                    .foregroundStyle(.secondary)
                    .frame(width: 58, alignment: .trailing)
            }
        }
    }

    private func colorControlRow(_ descriptor: WallpaperControlDescriptor) -> some View {
        HStack(spacing: 12) {
            Text(descriptor.label)
                .font(.callout)
                .frame(width: 142, alignment: .leading)

            Spacer()

            Text(PackedRGB.hex(values[descriptor.id] ?? descriptor.defaultValue))
                .font(.system(.caption, design: .monospaced))
                .foregroundStyle(.secondary)

            ColorPicker(
                descriptor.label,
                selection: Binding(
                    get: {
                        let components = PackedRGB.components(
                            values[descriptor.id] ?? descriptor.defaultValue
                        )
                        return Color(red: components.red,
                                     green: components.green,
                                     blue: components.blue)
                    },
                    set: { color in
                        let converted = NSColor(color).usingColorSpace(.sRGB) ?? NSColor(color)
                        updateValue(
                            PackedRGB.value(red: converted.redComponent,
                                            green: converted.greenComponent,
                                            blue: converted.blueComponent),
                            for: descriptor.id
                        )
                    }
                ),
                supportsOpacity: false
            )
            .labelsHidden()
        }
    }

    private func updateValue(_ value: Double, for id: String) {
        values[id] = value
        hasUnsavedChanges = true
        manager.previewControlValues(values, for: wallpaper.id, target: target)
    }

    private var targetDescription: String {
        switch target {
        case .allScreens:
            return "Aplicando cambios a todas las pantallas"
        case .screen(let displayUUID):
            let name = manager.screensByUUID.first(where: { $0.uuid == displayUUID })?.screen.localizedName
            return "Aplicando cambios a \(name ?? "la pantalla seleccionada")"
        }
    }

    private func loadControls() {
        colorProfiles = colorProfileStore.profiles()
        values = manager.controlValues(for: wallpaper.id, target: target)
        manager.fetchControlDescriptors(for: wallpaper.id, target: target) { loaded in
            let resolved: [WallpaperControlDescriptor]
            if loaded.isEmpty && wallpaper.manifest.source == "es-module" {
                resolved = WallpaperControlDescriptor.standardTransformControls
                isUsingFallback = true
            } else {
                resolved = loaded
                isUsingFallback = false
            }
            descriptors = resolved
            for descriptor in resolved where values[descriptor.id] == nil {
                values[descriptor.id] = descriptor.defaultValue
            }
            isLoading = false
            if !resolved.isEmpty {
                manager.previewControlValues(values, for: wallpaper.id, target: target)
            }
        }
    }

    private func persistValues() {
        guard !descriptors.isEmpty, hasUnsavedChanges else { return }
        manager.setControlValues(values, for: wallpaper.id, target: target)
        hasUnsavedChanges = false
        statusMessage = "Guardado para \(wallpaper.name)"
    }

    private func restoreDefaults() {
        guard !descriptors.isEmpty else { return }
        let defaults = Dictionary(uniqueKeysWithValues: descriptors.map {
            ($0.id, $0.defaultValue)
        })
        values = defaults
        manager.resetControlValues(to: defaults, for: wallpaper.id, target: target)
        hasUnsavedChanges = false
        statusMessage = "Valores predeterminados restaurados"
    }

    private func saveCurrentColorProfile() {
        guard let background = value(for: "backgroundColor"),
              let particle = value(for: "particleColor") else { return }
        let trimmedName = colorProfileName.trimmingCharacters(in: .whitespacesAndNewlines)
        let resolvedName = trimmedName.isEmpty
            ? "Perfil \(colorProfiles.count + 1)"
            : String(trimmedName.prefix(48))
        let profile = colorProfileStore.save(
            name: resolvedName,
            backgroundColor: background,
            particleColor: particle
        )
        colorProfiles = colorProfileStore.profiles()
        colorProfileName = ""
        statusMessage = "\(profile.name) guardado en tu biblioteca"
    }

    private func applyColorProfile(_ profile: ColorProfile) {
        values["backgroundColor"] = profile.backgroundColor
        values["particleColor"] = profile.particleColor
        hasUnsavedChanges = true
        manager.previewControlValues(values, for: wallpaper.id, target: target)
        statusMessage = "\(profile.name) aplicado"
    }

    private func deleteColorProfile(_ profile: ColorProfile) {
        colorProfileStore.delete(id: profile.id)
        colorProfiles = colorProfileStore.profiles()
        statusMessage = "\(profile.name) eliminado"
    }

    private func value(for id: String) -> Double? {
        guard let descriptor = descriptors.first(where: { $0.id == id }) else { return nil }
        return values[id] ?? descriptor.defaultValue
    }

    private func color(for packedValue: Double) -> Color {
        let components = PackedRGB.components(packedValue)
        return Color(red: components.red,
                     green: components.green,
                     blue: components.blue)
    }

    private func formattedValue(_ value: Double, step: Double) -> String {
        if step >= 1 { return String(format: "%.0f", value) }
        if step >= 0.1 { return String(format: "%.1f", value) }
        if step >= 0.01 { return String(format: "%.2f", value) }
        return String(format: "%.3f", value)
    }
}
