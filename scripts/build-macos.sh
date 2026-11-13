#!/bin/bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release --locked
app="$PWD/build/RustKaleidoRadio.app"
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp target/release/kaleido-radio "$app/Contents/MacOS/kaleido-radio"
cp assets/KaleidoRadio.icns "$app/Contents/Resources/"
# MilkDrop: bundle the projectM library and presets if present, so the
# visualizer can find them at Contents/Resources/. Missing files do
# not fail the build; the app then reports projectM unavailable.
if [[ -f libprojectM-4.dylib ]]; then
    cp libprojectM-4.dylib "$app/Contents/Resources/"
fi
if [[ -d presets ]]; then
    mkdir -p "$app/Contents/Resources/presets"
    cp -R presets/. "$app/Contents/Resources/presets/"
fi
cat > "$app/Contents/Info.plist" <<'PLIST'
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>CFBundleName</key><string>KaleidoRadio Rust</string>
<key>CFBundleDisplayName</key><string>KaleidoRadio Rust</string>
<key>CFBundleIdentifier</key><string>local.shadow.KaleidoRadioRust</string>
<key>CFBundleExecutable</key><string>kaleido-radio</string>
<key>CFBundlePackageType</key><string>APPL</string>
<key>CFBundleShortVersionString</key><string>0.1.0</string>
<key>CFBundleVersion</key><string>1</string>
<key>CFBundleIconFile</key><string>KaleidoRadio.icns</string>
<key>LSMinimumSystemVersion</key><string>12.0</string>
<key>NSPrincipalClass</key><string>NSApplication</string>
<key>NSHighResolutionCapable</key><true/>
<key>NSLocalNetworkUsageDescription</key><string>KaleidoRadio connects to your WebDAV or SMB server to browse and play media on your local network.</string>
<key>NSDocumentsFolderUsageDescription</key><string>KaleidoRadio reads music folders and favorites files you select.</string>
<key>NSDownloadsFolderUsageDescription</key><string>KaleidoRadio reads music folders and favorites files you select.</string>
<key>NSDesktopFolderUsageDescription</key><string>KaleidoRadio reads music folders and favorites files you select.</string>
<key>NSNetworkVolumesUsageDescription</key><string>KaleidoRadio reads music folders you select on network volumes.</string>
<key>NSRemovableVolumesUsageDescription</key><string>KaleidoRadio reads music folders you select on removable volumes.</string>
</dict></plist>
PLIST
plutil -lint "$app/Contents/Info.plist"
codesign --force --sign - "$app"
codesign --verify --deep --strict "$app"
printf 'Built %s\n' "$app"
if [[ "${1:-}" == "--install" ]]; then
    ditto "$app" /Applications/RustKaleidoRadio.app
    codesign --verify --deep --strict /Applications/RustKaleidoRadio.app
    printf 'Installed /Applications/RustKaleidoRadio.app\n'
elif [[ "${1:-}" == "--run" ]]; then
    open -n "$app"
fi
