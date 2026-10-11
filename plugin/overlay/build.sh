#!/bin/sh
# Builds "Pixlens Overlay.app" next to this script, for Apple silicon and
# Intel Macs both. The plugin ships the built app and starts it itself.
set -e
cd "$(dirname "$0")"
app="Pixlens Overlay.app"
work="$(mktemp -d)"
trap 'rm -rf "$work"' EXIT

for arch in arm64 x86_64; do
  swiftc -O -target "$arch-apple-macos12" -o "$work/$arch" Sources/main.swift
done

rm -rf "$app"
mkdir -p "$app/Contents/MacOS"
cp Info.plist "$app/Contents/Info.plist"
lipo -create "$work/arm64" "$work/x86_64" -output "$app/Contents/MacOS/pixlens-overlay"
codesign --force --sign - "$app"
echo "$PWD/$app"
