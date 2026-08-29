// swift-tools-version:5.10
import PackageDescription

let package = Package(
    name: "ParticleWall",
    platforms: [.macOS(.v14)],
    targets: [
        .executableTarget(
            name: "ParticleWall",
            path: "Sources/ParticleWall",
            resources: [
                .copy("Resources/three.min.js"),
                .copy("Resources/three-esm"),
                .copy("Resources/template.html"),
                .copy("Resources/template-module.html"),
                .copy("Resources/DefaultWallpaper"),
                .copy("Resources/TwinVortexWallpaper"),
                .copy("Resources/OrbitalBloomWallpaper"),
                .copy("Resources/HexagonalRosetteWallpaper"),
                .copy("Resources/NoiseRainWallpaper"),
                .copy("Resources/PrimeSpiralWallpaper"),
                .copy("Resources/TorusOrbitWallpaper"),
                .copy("Resources/ChromaticRingsWallpaper"),
                .copy("Resources/SphereTorusWallpaper"),
                .copy("Resources/JellyfishPointsWallpaper"),
                .copy("Resources/generated")
            ]
        ),
        .testTarget(
            name: "ParticleWallTests",
            dependencies: ["ParticleWall"]
        )
    ]
)
