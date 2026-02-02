// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "VoxtypeMenubar",
    platforms: [
        .macOS(.v13)
    ],
    products: [
        .executable(name: "VoxtypeMenubar", targets: ["VoxtypeMenubar"])
    ],
    targets: [
        .executableTarget(
            name: "VoxtypeMenubar",
            path: "Sources"
        )
    ]
)
