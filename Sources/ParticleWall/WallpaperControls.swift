import Foundation

enum WallpaperControlKind: String, Codable {
    case number
    case color
    case boolean
}

/// Numeric or color control exposed by a wallpaper renderer.
struct WallpaperControlDescriptor: Codable, Identifiable, Equatable {
    let id: String
    let label: String
    let category: String
    let min: Double
    let max: Double
    let step: Double
    let defaultValue: Double
    let kind: WallpaperControlKind?

    init(id: String,
         label: String,
         category: String,
         min: Double,
         max: Double,
         step: Double,
         defaultValue: Double,
         kind: WallpaperControlKind? = nil) {
        self.id = id
        self.label = label
        self.category = category
        self.min = min
        self.max = max
        self.step = step
        self.defaultValue = defaultValue
        self.kind = kind
    }

    var resolvedKind: WallpaperControlKind { kind ?? .number }

    /// Safe fallback while an ES module is still initializing or if WebKit
    /// cannot serialize its dynamically detected descriptor list.
    static let standardTransformControls: [WallpaperControlDescriptor] = [
        .init(id: "positionX", label: "Posición X", category: "Transformación",
              min: -100, max: 100, step: 0.5, defaultValue: 0),
        .init(id: "positionY", label: "Posición Y", category: "Transformación",
              min: -100, max: 100, step: 0.5, defaultValue: 0),
        .init(id: "positionZ", label: "Posición Z", category: "Transformación",
              min: -100, max: 100, step: 0.5, defaultValue: 0),
        .init(id: "rotationX", label: "Rotación X", category: "Transformación",
              min: -180, max: 180, step: 1, defaultValue: 0),
        .init(id: "rotationY", label: "Rotación Y", category: "Transformación",
              min: -180, max: 180, step: 1, defaultValue: 0),
        .init(id: "rotationZ", label: "Rotación Z", category: "Transformación",
              min: -180, max: 180, step: 1, defaultValue: 0),
        .init(id: "scale", label: "Escala", category: "Transformación",
              min: 0.1, max: 4, step: 0.05, defaultValue: 1)
    ]
}

enum PackedRGB {
    static func components(_ value: Double) -> (red: Double, green: Double, blue: Double) {
        let packed = max(0, min(0xFF_FF_FF, Int(value.rounded())))
        return (Double((packed >> 16) & 0xFF) / 255,
                Double((packed >> 8) & 0xFF) / 255,
                Double(packed & 0xFF) / 255)
    }

    static func value(red: Double, green: Double, blue: Double) -> Double {
        let r = Int((max(0, min(1, red)) * 255).rounded())
        let g = Int((max(0, min(1, green)) * 255).rounded())
        let b = Int((max(0, min(1, blue)) * 255).rounded())
        return Double((r << 16) | (g << 8) | b)
    }

    static func hex(_ value: Double) -> String {
        String(format: "#%06X", max(0, min(0xFF_FF_FF, Int(value.rounded()))))
    }
}

/// Persists control values per wallpaper and display. A wildcard display stores
/// the defaults used by "All screens" and by monitors connected in the future.
final class WallpaperControlStore {
    static let shared = WallpaperControlStore()

    private struct Storage: Codable {
        var configurations: [String: [String: Double]] = [:]
    }

    private let defaults: UserDefaults
    private let defaultsKey: String
    private let wildcardDisplay = "*"

    init(defaults: UserDefaults = .standard,
         defaultsKey: String = DefaultsKey.wallpaperControls) {
        self.defaults = defaults
        self.defaultsKey = defaultsKey
    }

    func values(for wallpaperID: UUID, displayUUID: String) -> [String: Double] {
        let storage = readStorage()
        return storage.configurations[key(wallpaperID, displayUUID)]
            ?? storage.configurations[key(wallpaperID, wildcardDisplay)]
            ?? [:]
    }

    func defaultValues(for wallpaperID: UUID) -> [String: Double] {
        readStorage().configurations[key(wallpaperID, wildcardDisplay)] ?? [:]
    }

    func setValues(_ values: [String: Double],
                   for wallpaperID: UUID,
                   displayUUID: String) {
        var storage = readStorage()
        storage.configurations[key(wallpaperID, displayUUID)] = values
        writeStorage(storage)
    }

    func setDefaultValues(_ values: [String: Double], for wallpaperID: UUID) {
        var storage = readStorage()
        let prefix = wallpaperID.uuidString + "|"
        storage.configurations = storage.configurations.filter { !$0.key.hasPrefix(prefix) }
        storage.configurations[key(wallpaperID, wildcardDisplay)] = values
        writeStorage(storage)
    }

    func removeValues(for wallpaperID: UUID) {
        var storage = readStorage()
        let prefix = wallpaperID.uuidString + "|"
        storage.configurations = storage.configurations.filter { !$0.key.hasPrefix(prefix) }
        writeStorage(storage)
    }

    private func key(_ wallpaperID: UUID, _ displayUUID: String) -> String {
        wallpaperID.uuidString + "|" + displayUUID
    }

    private func readStorage() -> Storage {
        guard let data = defaults.data(forKey: defaultsKey),
              let storage = try? JSONDecoder().decode(Storage.self, from: data) else {
            return Storage()
        }
        return storage
    }

    private func writeStorage(_ storage: Storage) {
        guard let data = try? JSONEncoder().encode(storage) else { return }
        defaults.set(data, forKey: defaultsKey)
    }
}

/// A reusable pair of background and particle colors. Profiles are global so
/// the same palette can be applied to any compatible wallpaper.
struct ColorProfile: Codable, Identifiable, Equatable {
    let id: UUID
    var name: String
    let backgroundColor: Double
    let particleColor: Double
    let createdAt: Date

    init(id: UUID = UUID(),
         name: String,
         backgroundColor: Double,
         particleColor: Double,
         createdAt: Date = Date()) {
        self.id = id
        self.name = name
        self.backgroundColor = backgroundColor
        self.particleColor = particleColor
        self.createdAt = createdAt
    }
}

final class ColorProfileStore {
    static let shared = ColorProfileStore()

    private struct Storage: Codable {
        var profiles: [ColorProfile] = []
    }

    private let defaults: UserDefaults
    private let defaultsKey: String

    init(defaults: UserDefaults = .standard,
         defaultsKey: String = DefaultsKey.colorProfiles) {
        self.defaults = defaults
        self.defaultsKey = defaultsKey
    }

    func profiles() -> [ColorProfile] {
        readStorage().profiles
    }

    @discardableResult
    func save(name: String,
              backgroundColor: Double,
              particleColor: Double) -> ColorProfile {
        let profile = ColorProfile(
            name: name.trimmingCharacters(in: .whitespacesAndNewlines),
            backgroundColor: backgroundColor,
            particleColor: particleColor
        )
        var storage = readStorage()
        storage.profiles.insert(profile, at: 0)
        writeStorage(storage)
        return profile
    }

    func delete(id: UUID) {
        var storage = readStorage()
        storage.profiles.removeAll { $0.id == id }
        writeStorage(storage)
    }

    private func readStorage() -> Storage {
        guard let data = defaults.data(forKey: defaultsKey),
              let storage = try? JSONDecoder().decode(Storage.self, from: data) else {
            return Storage()
        }
        return storage
    }

    private func writeStorage(_ storage: Storage) {
        guard let data = try? JSONEncoder().encode(storage) else { return }
        defaults.set(data, forKey: defaultsKey)
    }
}
