#!/usr/bin/env bash
set -euo pipefail

NOTES_FILE="${1:-}"

VERSION=$(node -p "require('./package.json').version")
TAG="v${VERSION}"

KEY_FILE="src-tauri/.tauri/flashback.key"
PASS_FILE="src-tauri/.tauri/flashback.key.pass"
[ -f "$KEY_FILE" ] || { echo "missing $KEY_FILE"; exit 1; }
[ -f "$PASS_FILE" ] || { echo "missing $PASS_FILE"; exit 1; }

export TAURI_SIGNING_PRIVATE_KEY="$(cat "$KEY_FILE")"
export TAURI_SIGNING_PRIVATE_KEY_PASSWORD="$(cat "$PASS_FILE")"

pnpm tauri build

SETUP="src-tauri/target/release/bundle/nsis/Flashback_${VERSION}_x64-setup.exe"
MSI="src-tauri/target/release/bundle/msi/Flashback_${VERSION}_x64_en-US.msi"
SIG="${SETUP}.sig"
[ -f "$SIG" ] || { echo "missing signature $SIG (createUpdaterArtifacts?)"; exit 1; }

if [ -n "$NOTES_FILE" ] && [ -f "$NOTES_FILE" ]; then
  export RELEASE_NOTES="$(cat "$NOTES_FILE")"
fi

node scripts/gen-latest-json.mjs "$VERSION" "$SIG" > latest.json

if [ -n "$NOTES_FILE" ] && [ -f "$NOTES_FILE" ]; then
  gh release create "$TAG" --title "Flashback ${VERSION}" --notes-file "$NOTES_FILE" \
    "$SETUP" "$MSI" latest.json
else
  gh release create "$TAG" --title "Flashback ${VERSION}" --generate-notes \
    "$SETUP" "$MSI" latest.json
fi

echo "Published $TAG"
