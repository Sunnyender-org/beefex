#!/bin/bash

set -euo pipefail

dmg_path="${1:-}"
if [[ -z "$dmg_path" || ! -f "$dmg_path" ]]; then
  echo 'usage: verify-macos-alpha-dmg.sh <Beefex.dmg>' >&2
  exit 2
fi

attach_output="$(hdiutil attach -readonly -nobrowse -noautoopen "$dmg_path")"
mount_point="$(printf '%s\n' "$attach_output" | awk '/\/Volumes\// {sub(/^.*\/Volumes\//,"/Volumes/"); print; exit}')"
if [[ -z "$mount_point" ]]; then
  echo 'Unable to resolve mounted DMG volume.' >&2
  exit 1
fi

cleanup() {
  if mount | grep -Fq " on $mount_point "; then
    LC_ALL=C LANG=C hdiutil detach "$mount_point" >/dev/null 2>&1 || true
  fi
}
trap cleanup EXIT

visible_count="$(find "$mount_point" -mindepth 1 -maxdepth 1 ! -name '.*' | wc -l | tr -d ' ')"
[[ "$visible_count" == '3' ]]
[[ -d "$mount_point/Beefex.app" ]]
[[ -L "$mount_point/Applications" ]]
[[ "$(readlink "$mount_point/Applications")" == '/Applications' ]]
[[ -f "$mount_point/安装前请看我.txt" ]]
[[ -f "$mount_point/.DS_Store" ]]
[[ -f "$mount_point/.background/background.png" ]]
[[ ! -e "$mount_point/Fix-Damage.txt" ]]
[[ ! -e "$mount_point/BUILD-INFO.txt" ]]

bundle_id="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleIdentifier' "$mount_point/Beefex.app/Contents/Info.plist")"
[[ "$bundle_id" == 'com.beefapi.beefex' ]]

executable_name="$(/usr/libexec/PlistBuddy -c 'Print :CFBundleExecutable' "$mount_point/Beefex.app/Contents/Info.plist")"
file "$mount_point/Beefex.app/Contents/MacOS/$executable_name" | grep -q 'arm64'

grep -q '这是用于测试和验收的 Apple Silicon 版本' "$mount_point/安装前请看我.txt"
grep -q '交给你信任的本机 Agent' "$mount_point/安装前请看我.txt"
grep -q '/usr/bin/xattr -rd com.apple.quarantine "/Applications/Beefex.app"' "$mount_point/安装前请看我.txt"
grep -q '不得关闭全局 Gatekeeper' "$mount_point/安装前请看我.txt"
grep -q 'Do not run spctl --master-disable' "$mount_point/安装前请看我.txt"

strings "$mount_point/.DS_Store" | grep -q 'background.png'

echo "BEEFEX_ALPHA_DMG_OK bundle=$bundle_id visible_items=$visible_count layout=finder-drag-drop-v1"
