// swift-tools-version: 6.2
import PackageDescription

let package = Package(
    name: "Rocket",
    platforms: [.macOS(.v26)],
    products: [
        .executable(name: "Rocket", targets: ["Rocket"]),
        .library(name: "RocketKit", targets: ["RocketKit"]),
    ],
    targets: [
        .target(
            name: "RocketKit",
            swiftSettings: [.swiftLanguageMode(.v6)]
        ),
        .executableTarget(
            name: "Rocket",
            dependencies: ["RocketKit"],
            swiftSettings: [.swiftLanguageMode(.v6)]
        ),
        .testTarget(
            name: "RocketKitTests",
            dependencies: ["RocketKit"],
            resources: [.copy("Fixtures")],
            swiftSettings: [.swiftLanguageMode(.v6)]
        ),
    ]
)
