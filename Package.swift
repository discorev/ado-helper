// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "ado-helper",
    platforms: [.macOS(.v13)],
    products: [.executable(name: "ado", targets: ["ado"])],
    targets: [
        .target(name: "ADOCore"),
        .executableTarget(name: "ado", dependencies: ["ADOCore"]),
        .testTarget(name: "ADOCoreTests", dependencies: ["ADOCore"])
    ]
)
