#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
cd "$ROOT"

command -v rustup >/dev/null || {
  echo "rustup is required for a universal build" >&2
  exit 1
}
cargo tauri --version >/dev/null 2>&1 || {
  echo "Tauri CLI is required: cargo install tauri-cli --version '^2' --locked" >&2
  exit 1
}

ARM_TARGET="aarch64-apple-darwin"
INTEL_TARGET="x86_64-apple-darwin"
UNIVERSAL_DIR="$ROOT/target/universal-apple-darwin/release"
VERSION="$(awk -F'"' '/^version = / { print $2; exit }' Cargo.toml)"

rustup target add "$ARM_TARGET" "$INTEL_TARGET"
cargo build --release --workspace --exclude cng-desktop --target "$ARM_TARGET"
cargo build --release --workspace --exclude cng-desktop --target "$INTEL_TARGET"

(
  cd apps/desktop
  cargo tauri build --target universal-apple-darwin --bundles app
)

APP="$UNIVERSAL_DIR/bundle/macos/Codex Network Guard.app"
test -d "$APP" || {
  echo "Tauri app bundle was not found at $APP" >&2
  exit 1
}

for binary in cng cngd cng-codex; do
  lipo -create \
    "$ROOT/target/$ARM_TARGET/release/$binary" \
    "$ROOT/target/$INTEL_TARGET/release/$binary" \
    -output "$APP/Contents/MacOS/$binary"
  chmod 755 "$APP/Contents/MacOS/$binary"
done

if [[ -n "${APPLE_SIGNING_IDENTITY:-}" ]]; then
  for binary in cng cngd cng-codex; do
    codesign --force --options runtime --timestamp --sign "$APPLE_SIGNING_IDENTITY" \
      "$APP/Contents/MacOS/$binary"
  done
  codesign --force --deep --options runtime --timestamp --sign "$APPLE_SIGNING_IDENTITY" "$APP"
  BUILD_KIND="signed"
else
  codesign --force --deep --sign - "$APP"
  BUILD_KIND="development"
fi

DMG_DIR="$UNIVERSAL_DIR/bundle/dmg"
mkdir -p "$DMG_DIR"
DMG="$DMG_DIR/Codex.Network.Guard_${VERSION}_universal_${BUILD_KIND}.dmg"
STAGING="$(mktemp -d "${TMPDIR:-/tmp}/cng-dmg.XXXXXX")"
trap 'rm -rf "$STAGING"' EXIT
cp -R "$APP" "$STAGING/"
ln -s /Applications "$STAGING/Applications"
hdiutil create -volname "Codex Network Guard" -srcfolder "$STAGING" -ov -format UDZO "$DMG"

if [[ -n "${APPLE_NOTARY_PROFILE:-}" ]]; then
  xcrun notarytool submit "$DMG" --keychain-profile "$APPLE_NOTARY_PROFILE" --wait
  xcrun stapler staple "$DMG"
fi

shasum -a 256 "$DMG" > "$DMG.sha256"
echo "Built $DMG"
