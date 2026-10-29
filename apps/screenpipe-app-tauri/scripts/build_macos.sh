#!/bin/bash
set -e

# Sign with your own Apple Development identity, e.g.
# APPLE_SIGNING_IDENTITY="Apple Development: Jane Doe (TEAMID1234)" bun run tauri:build
IDENTITY="${APPLE_SIGNING_IDENTITY:?set APPLE_SIGNING_IDENTITY to your Apple Development identity}"

# Clean up any existing bundle
rm -rf src-tauri/target/release/bundle

# Build without signing
bun tauri build --no-sign

# Strip extended attributes from all files in the bundle
APP_PATH="src-tauri/target/release/bundle/macos/Handvel Dev.app"
xattr -cr "$APP_PATH"

# Sign the app manually
codesign --force --deep --sign "$IDENTITY" "$APP_PATH"

echo "Build completed successfully!"
