#!/usr/bin/env bash
set -e

PROJECT_DIR="$(cd "$(dirname "$0")/.." && pwd)"
cd "$PROJECT_DIR"

echo "==> [1/5] Compiling Rust release binary..."
cargo build --release

echo "==> [2/5] Rendering macOS App Icon..."
ICONSET_DIR="/tmp/AppIcon.iconset"
rm -rf "$ICONSET_DIR" /tmp/AppIcon.png /tmp/AppIcon.icns
mkdir -p "$ICONSET_DIR"

swift "$PROJECT_DIR/macos/make_icon.swift" /tmp/AppIcon.png

# Generate standard iconset sizes
sips -z 16 16     /tmp/AppIcon.png --out "$ICONSET_DIR/icon_16x16.png" > /dev/null
sips -z 32 32     /tmp/AppIcon.png --out "$ICONSET_DIR/icon_16x16@2x.png" > /dev/null
sips -z 32 32     /tmp/AppIcon.png --out "$ICONSET_DIR/icon_32x32.png" > /dev/null
sips -z 64 64     /tmp/AppIcon.png --out "$ICONSET_DIR/icon_32x32@2x.png" > /dev/null
sips -z 128 128   /tmp/AppIcon.png --out "$ICONSET_DIR/icon_128x128.png" > /dev/null
sips -z 256 256   /tmp/AppIcon.png --out "$ICONSET_DIR/icon_128x128@2x.png" > /dev/null
sips -z 256 256   /tmp/AppIcon.png --out "$ICONSET_DIR/icon_256x256.png" > /dev/null
sips -z 512 512   /tmp/AppIcon.png --out "$ICONSET_DIR/icon_256x256@2x.png" > /dev/null
sips -z 512 512   /tmp/AppIcon.png --out "$ICONSET_DIR/icon_512x512.png" > /dev/null
sips -z 1024 1024 /tmp/AppIcon.png --out "$ICONSET_DIR/icon_512x512@2x.png" > /dev/null

iconutil -c icns "$ICONSET_DIR" -o /tmp/AppIcon.icns
rm -rf "$ICONSET_DIR" /tmp/AppIcon.png

echo "==> [3/5] Compiling Swift Native App..."
swiftc -O -framework Cocoa -framework WebKit "$PROJECT_DIR/macos/AppDelegate.swift" -o /tmp/PXE_Server_Bin

echo "==> [4/5] Constructing PXE Server.app bundle..."
APP_DIR="$PROJECT_DIR/PXE Server.app"
rm -rf "$APP_DIR"
mkdir -p "$APP_DIR/Contents/MacOS"
mkdir -p "$APP_DIR/Contents/Resources"

mv /tmp/PXE_Server_Bin "$APP_DIR/Contents/MacOS/PXE Server"
chmod +x "$APP_DIR/Contents/MacOS/PXE Server"
cp "$PROJECT_DIR/target/release/pxe" "$APP_DIR/Contents/MacOS/pxe"
chmod +x "$APP_DIR/Contents/MacOS/pxe"
cp /tmp/AppIcon.icns "$APP_DIR/Contents/Resources/AppIcon.icns"

cat << 'EOF' > "$APP_DIR/Contents/Info.plist"
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key>
    <string>PXE Server</string>
    <key>CFBundleIconFile</key>
    <string>AppIcon</string>
    <key>CFBundleIdentifier</key>
    <string>com.yocan.pxe-server</string>
    <key>CFBundleName</key>
    <string>PXE Server</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>1.0.0</string>
    <key>LSMinimumSystemVersion</key>
    <string>12.0</string>
    <key>NSHighResolutionCapable</key>
    <true/>
    <key>NSPrincipalClass</key>
    <string>NSApplication</string>
</dict>
</plist>
EOF

echo "==> [5/5] Deploying app shortcuts..."
# Make accessible in Applications or user's Desktop
ln -sf "$APP_DIR" ~/Desktop/ 2>/dev/null || true
mkdir -p ~/Applications
cp -R "$APP_DIR" ~/Applications/ 2>/dev/null || true

echo "=========================================================="
echo " ✅ Successfully built: $APP_DIR"
echo " 📍 Also available in: ~/Applications/PXE Server.app"
echo " 🖥️ Desktop shortcut: ~/Desktop/PXE Server.app"
echo "=========================================================="
