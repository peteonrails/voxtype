// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "VoxtypeSetup",
    platforms: [
        .macOS(.v13)
    ],
    products: [
        .executable(name: "VoxtypeSetup", targets: ["VoxtypeSetup"])
    ],
    targets: [
        .executableTarget(
            name: "VoxtypeSetup",
            path: "Sources"
        )
    ]
)
