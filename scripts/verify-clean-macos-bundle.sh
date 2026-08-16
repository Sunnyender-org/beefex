#!/bin/bash

set -euo pipefail

app_path="${1:-src-tauri/target/release/bundle/macos/Beefex.app}"

if [[ ! -d "$app_path" ]]; then
  echo "Missing app bundle: $app_path" >&2
  exit 1
fi

info_plist="$app_path/Contents/Info.plist"
resources="$app_path/Contents/Resources"
plist_buddy=/usr/libexec/PlistBuddy

bundle_id="$($plist_buddy -c 'Print :CFBundleIdentifier' "$info_plist")"
bundle_name="$($plist_buddy -c 'Print :CFBundleName' "$info_plist")"
executable_name="$($plist_buddy -c 'Print :CFBundleExecutable' "$info_plist")"
executable="$app_path/Contents/MacOS/$executable_name"
ocr_helper="$app_path/Contents/MacOS/beefex-ocr-helper"

[[ "$bundle_id" == "com.beefapi.beefex" ]]
[[ "$bundle_name" == "Beefex" ]]
[[ -x "$executable" ]]
[[ "$executable_name" == "beefex" ]]
[[ -x "$ocr_helper" ]]
file "$executable" | grep -q 'arm64'
file "$ocr_helper" | grep -q 'arm64'
[[ ! -e "$app_path/Contents/MacOS/kivio" ]]
[[ ! -e "$app_path/Contents/MacOS/kivio-ocr-helper" ]]

[[ -f "$resources/LICENSE" ]]
[[ -f "$resources/NOTICE" ]]
grep -q 'GNU GENERAL PUBLIC LICENSE' "$resources/LICENSE"
grep -q 'Kivio' "$resources/NOTICE"

pi_bin="$resources/pi/bin/pi"
[[ -x "$pi_bin" ]]
[[ -f "$resources/pi/beefex-managed-provider-extension.ts" ]]
[[ -f "$resources/pi/beefex-policy-extension.ts" ]]
[[ -f "$resources/pi/beefex-client-setup-extension.ts" ]]
[[ -f "$resources/client-plugins/beefapi-codex-image2.sh" ]]
[[ -d "$resources/skills" ]]

isolated_root="$(mktemp -d)"
app_log="$isolated_root/beefex-startup.log"
cleanup() {
  if [[ -n "${app_pid:-}" ]] && kill -0 "$app_pid" 2>/dev/null; then
    kill -TERM "$app_pid" 2>/dev/null || true
    wait "$app_pid" 2>/dev/null || true
  fi
  rm -rf "$isolated_root"
}
trap cleanup EXIT

pi_version="$(env -i HOME="$isolated_root" PATH=/usr/bin:/bin TMPDIR="$isolated_root" "$pi_bin" --version)"
grep -q '0.84.1' <<<"$pi_version"

env -i \
  HOME="$isolated_root" \
  LOGNAME=runner \
  PATH=/usr/bin:/bin \
  TMPDIR="$isolated_root" \
  USER=runner \
  "$executable" >"$app_log" 2>&1 &
app_pid=$!

sleep 12
if ! kill -0 "$app_pid" 2>/dev/null; then
  echo 'Beefex exited during clean-runner startup smoke.' >&2
  sed -n '1,200p' "$app_log" >&2
  exit 1
fi

if find "$isolated_root" -path '*/.kivio/skills-staged*' -print -quit | grep -q .; then
  echo 'Managed startup created a forbidden legacy staged-skill path.' >&2
  exit 1
fi

echo "BEEFEX_CLEAN_MACOS_BUNDLE_OK bundle=$bundle_id executable=$executable_name pi=$pi_version"
