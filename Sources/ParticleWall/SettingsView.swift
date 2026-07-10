import SwiftUI
import ServiceManagement

struct SettingsView: View {
    @AppStorage(DefaultsKey.pauseOnBattery) private var pauseOnBattery = false
    @AppStorage(DefaultsKey.powerSave) private var powerSave = false
    @AppStorage(DefaultsKey.fpsCap) private var fpsCap = 30
    @AppStorage(DefaultsKey.renderScale) private var renderScale = 1.5
    @State private var launchAtLogin = SMAppService.mainApp.status == .enabled
    @State private var loginItemError: String?

    var body: some View {
        Form {
            Section {
                Toggle("Iniciar al arrancar sesión", isOn: $launchAtLogin)
                    .onChange(of: launchAtLogin) { _, enabled in
                        updateLaunchAtLogin(enabled)
                    }
                if let loginItemError {
                    Text(loginItemError)
                        .font(.caption)
                        .foregroundStyle(.red)
                }
            }

            Section("Energía") {
                Toggle("Pausar con batería", isOn: $pauseOnBattery)
                Toggle("Power Save (congela un frame)", isOn: $powerSave)
                Picker("Límite de FPS", selection: $fpsCap) {
                    Text("Sin límite").tag(0)
                    Text("15 fps").tag(15)
                    Text("30 fps").tag(30)
                    Text("60 fps").tag(60)
                    Text("120 fps").tag(120)
                }
                Picker("Resolución de render", selection: $renderScale) {
                    Text("Baja (1x)").tag(1.0)
                    Text("Media (1.5x)").tag(1.5)
                    Text("Nativa (2x)").tag(2.0)
                }
            }

            Section {
                Button("Abrir carpeta de wallpapers") {
                    NSWorkspace.shared.open(LibraryManager.shared.wallpapersURL)
                }
            }
        }
        .formStyle(.grouped)
        .frame(width: 380)
        .fixedSize(horizontal: false, vertical: true)
    }

    private func updateLaunchAtLogin(_ enabled: Bool) {
        loginItemError = nil
        do {
            if enabled {
                try SMAppService.mainApp.register()
            } else {
                try SMAppService.mainApp.unregister()
            }
        } catch {
            loginItemError = "No se pudo cambiar el inicio de sesión: \(error.localizedDescription). " +
                             "Mueve la app a /Applications e inténtalo de nuevo."
            launchAtLogin = SMAppService.mainApp.status == .enabled
        }
    }
}
