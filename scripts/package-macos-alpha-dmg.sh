#!/bin/bash

set -euo pipefail

if [[ $# -ne 3 ]]; then
  echo 'usage: package-macos-alpha-dmg.sh <Beefex.app> <output.dmg> <release-tag>' >&2
  exit 2
fi

app_path="$1"
output_path="$2"
release_tag="$3"

if [[ ! -d "$app_path" ]]; then
  echo "Missing app bundle: $app_path" >&2
  exit 1
fi

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
staging_root="$(mktemp -d)"
cleanup() {
  rm -rf "$staging_root"
}
trap cleanup EXIT

if ! command -v create-dmg >/dev/null 2>&1; then
  echo 'Missing create-dmg. Install the pinned Homebrew release before packaging.' >&2
  exit 1
fi
if [[ "$(create-dmg --version)" != 'create-dmg 1.3.0' ]]; then
  echo 'Expected create-dmg 1.3.0.' >&2
  exit 1
fi

mkdir -p "$(dirname "$output_path")" "$staging_root/source" "$staging_root/assets"
ditto "$app_path" "$staging_root/source/Beefex.app"
swift "$repo_root/scripts/generate-dmg-background.swift" "$staging_root/assets/background.png"

rm -f "$output_path"
LC_ALL=C LANG=C create-dmg \
  --volname "Beefex ${release_tag}" \
  --volicon "$repo_root/src-tauri/icons/icon.icns" \
  --background "$staging_root/assets/background.png" \
  --window-pos 400 300 \
  --window-size 584 440 \
  --text-size 12 \
  --icon-size 80 \
  --icon 'Beefex.app' 145 145 \
  --hide-extension 'Beefex.app' \
  --app-drop-link 439 145 \
  --add-file '安装前请看我.txt' \
    "$repo_root/docs/UNSIGNED_MACOS_ALPHA_INSTALL.txt" 292 320 \
  --format UDZO \
  --filesystem APFS \
  --no-internet-enable \
  --hdiutil-retries 10 \
  --overwrite \
  "$output_path" \
  "$staging_root/source"
hdiutil verify "$output_path"
