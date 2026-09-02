import Cocoa

let size = NSSize(width: 1024, height: 1024)
let image = NSImage(size: size)
image.lockFocus()

let ctx = NSGraphicsContext.current!.cgContext

// 1. Background rounded squircle (macOS Big Sur+ icon mask)
let rect = CGRect(x: 64, y: 64, width: 896, height: 896)
let path = CGPath(roundedRect: rect, cornerWidth: 200, cornerHeight: 200, transform: nil)
ctx.addPath(path)
ctx.clip()

// 2. Deep Blue / Slate Gradient
let colorSpace = CGColorSpaceCreateDeviceRGB()
let colors = [
    NSColor(red: 0.08, green: 0.12, blue: 0.22, alpha: 1.0).cgColor,
    NSColor(red: 0.15, green: 0.25, blue: 0.45, alpha: 1.0).cgColor
] as CFArray
let gradient = CGGradient(colorsSpace: colorSpace, colors: colors, locations: [0.0, 1.0])!
ctx.drawLinearGradient(gradient, start: CGPoint(x: 512, y: 960), end: CGPoint(x: 512, y: 64), options: [])

// 3. Subtle Inner Glow border
ctx.setStrokeColor(NSColor(red: 0.35, green: 0.55, blue: 0.95, alpha: 0.35).cgColor)
ctx.setLineWidth(12)
ctx.addPath(path)
ctx.strokePath()

// 4. Server Blades
let bladeWidth: CGFloat = 660
let bladeHeight: CGFloat = 150
let bladeX: CGFloat = (1024 - bladeWidth) / 2

for i in 0..<3 {
    let bladeY: CGFloat = 250 + CGFloat(i) * 190
    let bladeRect = CGRect(x: bladeX, y: bladeY, width: bladeWidth, height: bladeHeight)
    let bladePath = CGPath(roundedRect: bladeRect, cornerWidth: 24, cornerHeight: 24, transform: nil)
    
    // Blade fill
    ctx.setFillColor(NSColor(red: 0.10, green: 0.15, blue: 0.28, alpha: 0.92).cgColor)
    ctx.addPath(bladePath)
    ctx.fillPath()
    
    // Blade outline
    ctx.setStrokeColor(NSColor(red: 0.30, green: 0.50, blue: 0.85, alpha: 0.6).cgColor)
    ctx.setLineWidth(6)
    ctx.addPath(bladePath)
    ctx.strokePath()
    
    // Status LEDs
    let ledY = bladeY + 60
    let ledColors = [
        NSColor(red: 0.10, green: 0.85, blue: 0.50, alpha: 1.0).cgColor, // Green
        NSColor(red: 0.15, green: 0.70, blue: 0.95, alpha: 1.0).cgColor, // Cyan
        NSColor(red: 0.55, green: 0.40, blue: 0.95, alpha: 1.0).cgColor  // Purple
    ]
    for j in 0..<3 {
        let ledX = bladeX + 60 + CGFloat(j) * 44
        ctx.setFillColor(ledColors[j])
        ctx.fillEllipse(in: CGRect(x: ledX, y: ledY, width: 22, height: 22))
    }
    
    // Vent slits
    ctx.setFillColor(NSColor(red: 0.20, green: 0.30, blue: 0.50, alpha: 0.5).cgColor)
    for k in 0..<14 {
        let slotX = bladeX + 240 + CGFloat(k) * 26
        ctx.fill(CGRect(x: slotX, y: bladeY + 45, width: 12, height: 60))
    }
}

image.unlockFocus()

if let tiff = image.tiffRepresentation,
   let bitmap = NSBitmapImageRep(data: tiff),
   let png = bitmap.representation(using: .png, properties: [:]) {
    let outPath = CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "AppIcon.png"
    try? png.write(to: URL(fileURLWithPath: outPath))
    print("Generated icon: \(outPath)")
}
