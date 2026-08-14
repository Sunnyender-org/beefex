#!/usr/bin/env swift

import AppKit
import Foundation

let arguments = CommandLine.arguments
guard arguments.count == 2 else {
    fputs("usage: generate-dmg-background.swift <output-base.png>\n", stderr)
    exit(2)
}

let baseURL = URL(fileURLWithPath: arguments[1])
let logicalWidth: CGFloat = 584
let logicalHeight: CGFloat = 440

func drawCentered(_ text: String, y: CGFloat, font: NSFont, color: NSColor, scale: CGFloat) {
    let attributes: [NSAttributedString.Key: Any] = [
        .font: font,
        .foregroundColor: color,
        .kern: 0.15 * scale,
    ]
    let size = text.size(withAttributes: attributes)
    text.draw(
        at: NSPoint(x: (logicalWidth * scale - size.width) / 2, y: y * scale),
        withAttributes: attributes
    )
}

func render(scale: CGFloat, destination: URL) throws {
    let pixelWidth = Int(logicalWidth * scale)
    let pixelHeight = Int(logicalHeight * scale)
    guard let bitmap = NSBitmapImageRep(
        bitmapDataPlanes: nil,
        pixelsWide: pixelWidth,
        pixelsHigh: pixelHeight,
        bitsPerSample: 8,
        samplesPerPixel: 4,
        hasAlpha: true,
        isPlanar: false,
        colorSpaceName: .deviceRGB,
        bytesPerRow: 0,
        bitsPerPixel: 0
    ) else {
        throw NSError(domain: "BeefexDMG", code: 1)
    }

    NSGraphicsContext.saveGraphicsState()
    guard let context = NSGraphicsContext(bitmapImageRep: bitmap) else {
        throw NSError(domain: "BeefexDMG", code: 2)
    }
    NSGraphicsContext.current = context

    let bounds = NSRect(x: 0, y: 0, width: logicalWidth * scale, height: logicalHeight * scale)
    NSColor(calibratedRed: 0.965, green: 0.970, blue: 0.982, alpha: 1).setFill()
    bounds.fill()

    let topGlow = NSGradient(
        starting: NSColor(calibratedRed: 0.995, green: 0.997, blue: 1.0, alpha: 1),
        ending: NSColor(calibratedRed: 0.945, green: 0.953, blue: 0.974, alpha: 1)
    )
    topGlow?.draw(in: bounds, angle: -90)

    drawCentered(
        "BEEFEX ALPHA · TEST BUILD",
        y: 402,
        font: NSFont.systemFont(ofSize: 12 * scale, weight: .semibold),
        color: NSColor(calibratedWhite: 0.43, alpha: 1),
        scale: scale
    )

    let arrowColor = NSColor(calibratedRed: 0.20, green: 0.74, blue: 0.48, alpha: 1)
    arrowColor.setFill()
    let shaft = NSBezierPath(
        roundedRect: NSRect(x: 240 * scale, y: 288 * scale, width: 84 * scale, height: 16 * scale),
        xRadius: 8 * scale,
        yRadius: 8 * scale
    )
    shaft.fill()
    let arrow = NSBezierPath()
    arrow.move(to: NSPoint(x: 324 * scale, y: 278 * scale))
    arrow.line(to: NSPoint(x: 350 * scale, y: 296 * scale))
    arrow.line(to: NSPoint(x: 324 * scale, y: 314 * scale))
    arrow.close()
    arrow.fill()

    drawCentered(
        "将 Beefex 拖入 Applications",
        y: 196,
        font: NSFont.systemFont(ofSize: 16 * scale, weight: .medium),
        color: NSColor(calibratedWhite: 0.34, alpha: 1),
        scale: scale
    )
    drawCentered(
        "Drag Beefex to Applications",
        y: 171,
        font: NSFont.systemFont(ofSize: 13 * scale, weight: .regular),
        color: NSColor(calibratedWhite: 0.52, alpha: 1),
        scale: scale
    )

    context.flushGraphics()
    NSGraphicsContext.restoreGraphicsState()

    guard let png = bitmap.representation(using: .png, properties: [:]) else {
        throw NSError(domain: "BeefexDMG", code: 3)
    }
    try png.write(to: destination, options: .atomic)
}

try render(scale: 1, destination: baseURL)
let retinaURL = baseURL.deletingLastPathComponent()
    .appendingPathComponent(baseURL.deletingPathExtension().lastPathComponent + "@2x.png")
try render(scale: 2, destination: retinaURL)
