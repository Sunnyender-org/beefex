// swift-tools-version: 6.0
import PackageDescription

let package = Package(
  name: "beefex-ocr-helper",
  platforms: [.macOS("14.0")],
  targets: [
    .executableTarget(
      name: "beefex-ocr-helper",
      path: "Sources"
    )
  ]
)
