#!/usr/bin/env bash
# Download adb and scrcpy Windows binaries for bundling.
# Run this before `npm run tauri build` on Windows.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
OUTPUT_DIR="${SCRIPT_DIR}/../src-tauri/resources"
mkdir -p "$OUTPUT_DIR"

echo "Downloading Windows tools to: ${OUTPUT_DIR}"

# ── ADB (platform-tools) ──
echo ""
echo "[1/2] Downloading ADB (platform-tools)..."
ADB_ZIP="/tmp/platform-tools-latest-windows.zip"
curl -fSL -o "$ADB_ZIP" "https://dl.google.com/android/repository/platform-tools-latest-windows.zip"

echo "  Extracting adb.exe, AdbWinApi.dll, AdbWinUsbApi.dll..."
ADB_TMP="/tmp/platform-tools"
rm -rf "$ADB_TMP"
unzip -q -o "$ADB_ZIP" -d "$ADB_TMP"

cp "$ADB_TMP/platform-tools/adb.exe" "$OUTPUT_DIR/adb.exe"
cp "$ADB_TMP/platform-tools/AdbWinApi.dll" "$OUTPUT_DIR/AdbWinApi.dll"
cp "$ADB_TMP/platform-tools/AdbWinUsbApi.dll" "$OUTPUT_DIR/AdbWinUsbApi.dll"

rm -rf "$ADB_TMP"

# ── scrcpy ──
echo ""
echo "[2/2] Downloading scrcpy..."
LATEST_JSON="$(curl -fSL "https://api.github.com/repos/Genymobile/scrcpy/releases/latest")"
VERSION="$(echo "$LATEST_JSON" | grep '"tag_name":' | sed 's/.*"v\(.*\)",*/\1/')"
SCRCPY_URL="https://github.com/Genymobile/scrcpy/releases/download/v${VERSION}/scrcpy-win64-v${VERSION}.zip"
echo "  Version: v${VERSION}"

SCRCPY_ZIP="/tmp/scrcpy-win64-v${VERSION}.zip"
curl -fSL -o "$SCRCPY_ZIP" "$SCRCPY_URL"

echo "  Extracting all files..."
SCRCPY_TMP="/tmp/scrcpy-win64"
rm -rf "$SCRCPY_TMP"
unzip -q -o "$SCRCPY_ZIP" -d "$SCRCPY_TMP"

# Copy everything from the extracted folder (handle any dir name)
EXTRACTED_DIR="$(find "$SCRCPY_TMP" -maxdepth 1 -type d -name 'scrcpy-win64*' | head -1)"
if [ -n "$EXTRACTED_DIR" ]; then
  cp -r "$EXTRACTED_DIR"/* "$OUTPUT_DIR/"
else
  cp -r "$SCRCPY_TMP"/* "$OUTPUT_DIR/"
fi

rm -rf "$SCRCPY_TMP"

echo ""
echo "Done! Files in: ${OUTPUT_DIR}"
ls -lh "$OUTPUT_DIR"
