import XCTest
@testable import ParticleWall

final class WallpaperControlStoreTests: XCTestCase {
    private var suiteName: String!
    private var defaults: UserDefaults!
    private var store: WallpaperControlStore!

    override func setUp() {
        super.setUp()
        suiteName = "ParticleWallTests.\(UUID().uuidString)"
        defaults = UserDefaults(suiteName: suiteName)
        defaults.removePersistentDomain(forName: suiteName)
        store = WallpaperControlStore(defaults: defaults, defaultsKey: "controls")
    }

    override func tearDown() {
        defaults.removePersistentDomain(forName: suiteName)
        store = nil
        defaults = nil
        suiteName = nil
        super.tearDown()
    }

    func testDisplayValueOverridesAllScreensDefault() {
        let wallpaperID = UUID()
        store.setDefaultValues(["positionX": 12], for: wallpaperID)

        XCTAssertEqual(store.values(for: wallpaperID, displayUUID: "display-a"),
                       ["positionX": 12])

        store.setValues(["positionX": -8], for: wallpaperID, displayUUID: "display-a")

        XCTAssertEqual(store.values(for: wallpaperID, displayUUID: "display-a"),
                       ["positionX": -8])
        XCTAssertEqual(store.values(for: wallpaperID, displayUUID: "display-b"),
                       ["positionX": 12])
    }

    func testApplyingAllScreensClearsStaleDisplayOverrides() {
        let wallpaperID = UUID()
        store.setValues(["scale": 2], for: wallpaperID, displayUUID: "display-a")
        store.setDefaultValues(["scale": 0.75], for: wallpaperID)

        XCTAssertEqual(store.values(for: wallpaperID, displayUUID: "display-a"),
                       ["scale": 0.75])
        XCTAssertEqual(store.defaultValues(for: wallpaperID), ["scale": 0.75])
    }

    func testRemovingWallpaperDeletesEveryDisplayConfiguration() {
        let wallpaperID = UUID()
        store.setDefaultValues(["speed": 1], for: wallpaperID)
        store.setValues(["speed": 2], for: wallpaperID, displayUUID: "display-a")

        store.removeValues(for: wallpaperID)

        XCTAssertTrue(store.values(for: wallpaperID, displayUUID: "display-a").isEmpty)
        XCTAssertTrue(store.defaultValues(for: wallpaperID).isEmpty)
    }

    func testWallpapersKeepIndependentConfigurations() {
        let first = UUID()
        let second = UUID()
        store.setValues(["scale": 0.75], for: first, displayUUID: "display-a")
        store.setValues(["scale": 2.25], for: second, displayUUID: "display-a")

        XCTAssertEqual(store.values(for: first, displayUUID: "display-a"), ["scale": 0.75])
        XCTAssertEqual(store.values(for: second, displayUUID: "display-a"), ["scale": 2.25])

        store.removeValues(for: first)

        XCTAssertTrue(store.values(for: first, displayUUID: "display-a").isEmpty)
        XCTAssertEqual(store.values(for: second, displayUUID: "display-a"), ["scale": 2.25])
    }

    func testControlDescriptorDefaultsToNumberWhenKindIsMissing() throws {
        let json = """
        {
          "id": "speed",
          "label": "Velocidad",
          "category": "Animación",
          "min": 0,
          "max": 3,
          "step": 0.05,
          "defaultValue": 1
        }
        """.data(using: .utf8)!

        let descriptor = try JSONDecoder().decode(WallpaperControlDescriptor.self, from: json)

        XCTAssertEqual(descriptor.resolvedKind, .number)
    }

    func testPackedRGBRoundTripAndHexFormatting() {
        let value = PackedRGB.value(red: 0.2, green: 0.6, blue: 1)
        let components = PackedRGB.components(value)

        XCTAssertEqual(components.red, 0.2, accuracy: 0.001)
        XCTAssertEqual(components.green, 0.6, accuracy: 0.001)
        XCTAssertEqual(components.blue, 1, accuracy: 0.001)
        XCTAssertEqual(PackedRGB.hex(value), "#3399FF")
    }

    func testColorProfilesPersistBothColorsAndNewestComesFirst() {
        let profileStore = ColorProfileStore(defaults: defaults,
                                             defaultsKey: "colorProfiles")
        let first = profileStore.save(name: "Océano",
                                      backgroundColor: 0x00_11_22,
                                      particleColor: 0x33_AA_FF)
        let second = profileStore.save(name: "  Magenta  ",
                                       backgroundColor: 0x12_00_18,
                                       particleColor: 0xFF_44_CC)

        let reloaded = ColorProfileStore(defaults: defaults,
                                         defaultsKey: "colorProfiles").profiles()

        XCTAssertEqual(reloaded.map(\.id), [second.id, first.id])
        XCTAssertEqual(reloaded.first?.name, "Magenta")
        XCTAssertEqual(reloaded.first?.backgroundColor, 0x12_00_18)
        XCTAssertEqual(reloaded.first?.particleColor, 0xFF_44_CC)
    }

    func testColorProfileCanBeDeletedWithoutChangingOthers() {
        let profileStore = ColorProfileStore(defaults: defaults,
                                             defaultsKey: "colorProfiles")
        let keep = profileStore.save(name: "Conservar",
                                     backgroundColor: 1,
                                     particleColor: 2)
        let remove = profileStore.save(name: "Eliminar",
                                       backgroundColor: 3,
                                       particleColor: 4)

        profileStore.delete(id: remove.id)

        XCTAssertEqual(profileStore.profiles(), [keep])
    }

    func testMetalAppearanceControlsAreExposed() {
        let byID = Dictionary(uniqueKeysWithValues: MetalParticleRenderer.descriptors.map {
            ($0.id, $0)
        })

        XCTAssertEqual(byID["backgroundColor"]?.resolvedKind, .color)
        XCTAssertEqual(byID["particleColor"]?.resolvedKind, .color)
        XCTAssertEqual(byID["particleSize"]?.defaultValue, 1.6)
        XCTAssertEqual(byID["brightness"]?.defaultValue, 1.5)
        XCTAssertEqual(byID["brightness"]?.max, 10)
    }

    func testMetalScreenFitControlsAreTypedAndBounded() {
        let byID = Dictionary(uniqueKeysWithValues: MetalParticleRenderer.descriptors.map {
            ($0.id, $0)
        })
        let adaptedByID = Dictionary(
            uniqueKeysWithValues: MetalParticleRenderer.screenAdaptedDescriptors.map {
                ($0.id, $0)
            }
        )

        XCTAssertEqual(byID["fitToScreen"]?.resolvedKind, .boolean)
        XCTAssertEqual(byID["horizontalLimit"]?.min, 0.25)
        XCTAssertEqual(byID["verticalLimit"]?.max, 2.5)
        XCTAssertEqual(byID["fitToScreen"]?.defaultValue, 0)
        XCTAssertEqual(adaptedByID["fitToScreen"]?.defaultValue, 1)
    }

    func testMetalGraphControlsAreOptionalAndBounded() {
        let byID = Dictionary(uniqueKeysWithValues: MetalParticleRenderer.descriptors.map {
            ($0.id, $0)
        })

        XCTAssertEqual(byID["graphEnabled"]?.resolvedKind, .boolean)
        XCTAssertEqual(byID["graphEnabled"]?.defaultValue, 0)
        XCTAssertEqual(byID["graphDistance"]?.min, 0.02)
        XCTAssertEqual(byID["graphDistance"]?.max, 0.25)
        XCTAssertEqual(byID["graphConnections"]?.max, 3)
        XCTAssertEqual(byID["graphOpacity"]?.defaultValue, 0.55)
        XCTAssertEqual(byID["graphOpacity"]?.max, 10)
    }

    func testAllBundledRendererKindsUseTheNativeMetalPath() {
        XCTAssertEqual(WallpaperRendererKind.nativeMetalCases, [
            .asciiVideo,
            .metalParticles,
            .metalTwinVortex,
            .metalOrbitalBloom,
            .metalHexagonalRosette,
            .metalNoiseRain,
            .metalPrimeSpiral,
            .metalTorusOrbit,
            .metalChromaticRings,
            .metalSphereTorus,
            .metalJellyfishPoints,
            .metalNebula,
            .metalTorusKnot
        ])
        XCTAssertTrue(WallpaperRendererKind.nativeMetalCases.allSatisfy(\.isNativeMetal))
        XCTAssertFalse(WallpaperRendererKind.web.isNativeMetal)
    }

    func testBundledWallpaperIsMarkedAsProtected() {
        let bundled = Wallpaper(
            id: UUID(),
            manifest: WallpaperManifest(name: "Default",
                                        source: "bundled",
                                        renderer: .metalParticles),
            folderURL: URL(fileURLWithPath: "/tmp/default")
        )
        let imported = Wallpaper(
            id: UUID(),
            manifest: WallpaperManifest(name: "Imported", source: "es-module"),
            folderURL: URL(fileURLWithPath: "/tmp/imported")
        )

        XCTAssertTrue(bundled.isBundled)
        XCTAssertFalse(imported.isBundled)
    }

    func testModuleBootstrapInstallsCustomizationBridge() {
        let bootstrap = ImportPipeline.moduleBootstrap(for: """
        export default class Demo {
          animate() {
            const scale = addControl("scale", "Model Scale", 10, 100, 50);
          }
        }
        """)

        XCTAssertTrue(bootstrap.contains("window.__pwGetControls"))
        XCTAssertTrue(bootstrap.contains("window.__pwApplySettings"))
        XCTAssertTrue(bootstrap.contains("setParticleWallParameter"))
        XCTAssertTrue(bootstrap.contains("pw-user-module.js"))
        XCTAssertTrue(bootstrap.contains(#""id":"model.scale""#))
        XCTAssertTrue(bootstrap.contains(#""parameterID":"scale""#))
    }

    func testModuleBootstrapAddsAppearanceColorControls() {
        let bootstrap = ImportPipeline.moduleBootstrap(for: """
        export default class Demo {
          constructor() {
            this.renderer = null;
            this.mesh = null;
          }
        }
        """)

        // The bridge detects a three.js renderer/material and advertises the
        // same background/particle appearance controls the Metal models have.
        XCTAssertTrue(bootstrap.contains("kind: kind || undefined"))
        XCTAssertTrue(bootstrap.contains("'color'"))
        XCTAssertTrue(bootstrap.contains("particleColor"))
        XCTAssertTrue(bootstrap.contains("backgroundColor"))
        XCTAssertTrue(bootstrap.contains("getClearColor"))
        XCTAssertTrue(bootstrap.contains("setHex(pc)"))
    }

    func testGeneratedControlsAreExtractedAndTyped() {
        let code = """
        const size = addControl("size", "Block Size", 0.5, 10, 2.5);
        const count = Math.floor(addControl("blocks", "Num Blocks", 10, 800, 350));
        """

        let controls = ImportPipeline.generatedControlDescriptors(in: code)

        XCTAssertEqual(controls.count, 2)
        XCTAssertEqual(controls[0]["id"] as? String, "size")
        XCTAssertEqual(controls[0]["step"] as? Double, 0.05)
        XCTAssertEqual(controls[1]["id"] as? String, "blocks")
        XCTAssertEqual(controls[1]["step"] as? Double, 1)
    }

    func testGeneratedModuleAdapterPreservesOriginalDefaultsAndHoistsControls() {
        let code = """
        export default class Demo {
          constructor() {
            for (let i = 0; i < 10; i++) this.seed(i);
          }
          animate() {
            const PARAMS = {"size":2.5};
            const addControl = (id, label, min, max, value) =>
              PARAMS[id] !== undefined ? PARAMS[id] : value;
            for (let i = 0; i < this.count; i++) {
              const size = addControl("size", "Size", 0.5, 10, 2.5);
              this.values[i] = size;
            }
          }
        }
        """

        let adapted = ImportPipeline.adaptGeneratedModule(code)

        XCTAssertTrue(adapted.contains("particlewall-generated-adapter-v1"))
        XCTAssertTrue(adapted.contains("this.__pwParams = Object.assign"))
        let control = adapted.range(of: #"const size = addControl"#)!.lowerBound
        let animateLoop = adapted.range(of: #"for (let i"#,
                                        options: [],
                                        range: adapted.range(of: #"const PARAMS"#)!.lowerBound..<adapted.endIndex)!.lowerBound
        XCTAssertLessThan(control, animateLoop)
        XCTAssertGreaterThan(control, adapted.range(of: #"const PARAMS"#)!.lowerBound)
        XCTAssertEqual(ImportPipeline.adaptGeneratedModule(adapted), adapted)
    }
}
