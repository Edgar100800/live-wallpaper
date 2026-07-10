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
                .copy("Resources/DefaultWallpaper")
            ]
        )
    ]
)
