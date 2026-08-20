#!/bin/bash

set -euo pipefail

repo_root="$(cd "$(dirname "$0")/.." && pwd)"
cd "$repo_root"

matches="$(mktemp)"
unexpected="$(mktemp)"
cleanup() {
  rm -f "$matches" "$unexpected"
}
trap cleanup EXIT

rg -n -i 'kivio' \
  README.md NOTICE package.json src src-tauri/Cargo.toml src-tauri/tauri.conf.json \
  src-tauri/src scripts .github \
  --glob '!scripts/check-beefex-identity.sh' \
  --glob '!**/target/**' --glob '!**/.build/**' >"$matches" || true

while IFS= read -r match; do
  [[ -z "$match" ]] && continue
  case "$match" in
    README.md:*'modified derivative of [Kivio]'*) ;;
    NOTICE:*) ;;
    src/settings/SettingsShell.tsx:*'Kivio · GPL-3.0-or-later'*) ;;
    src/settings/AboutOpenSourceNotice.tsx:*'Kivio · GPL-3.0-or-later'*) ;;
    src/settings/settingsSurface.ts:*'Kivio · GPL-3.0-or-later'*) ;;
    src/settings/settingsSurface.test.ts:*'Kivio · GPL-3.0-or-later'*) ;;
    src/settings/AboutOpenSourceNotice.test.tsx:*'Kivio · GPL-3.0-or-later'*) ;;
    src/lens/history.ts:*"'kivio:lens-history:v1'"*) ;;
    src/chat/persistence.ts:*"'kivio-chat-"*) ;;
    src/chat/persistence.test.ts:*"'kivio-chat-"*) ;;
    src/chat/multiAnswerViewMode.ts:*"'kivio.chat.multiAnswerView'"*) ;;
    src-tauri/src/lens.rs:*'"Kivio Desktop"'*) ;;
    src-tauri/src/lens.rs:*'"Kivio"'*) ;;
    src-tauri/src/lens.rs:*'"kivio"'*) ;;
    src-tauri/src/external_agents/prompt.rs:*'assert!(!hint.contains(".kivio"))'*) ;;
    scripts/verify-clean-macos-bundle.sh:*"grep -q 'Kivio'"*) ;;
    scripts/verify-clean-macos-bundle.sh:*"'*/.kivio/skills-staged*'"*) ;;
    scripts/verify-clean-macos-bundle.sh:*'[[ ! -e "$app_path/Contents/MacOS/kivio'*) ;;
    *) printf '%s\n' "$match" >>"$unexpected" ;;
  esac
done <"$matches"

if [[ -s "$unexpected" ]]; then
  echo 'Unexpected legacy product identity:' >&2
  cat "$unexpected" >&2
  exit 1
fi

grep -q '^name = "beefex"$' src-tauri/Cargo.toml
grep -q '^default-run = "beefex"$' src-tauri/Cargo.toml
grep -q '"binaries/beefex-ocr-helper"' src-tauri/tauri.conf.json
test ! -e src-tauri/src/kivio_code
test ! -e src-tauri/src/bin/kivio-code.rs
test ! -e src-tauri/src/plugins
test ! -e src-tauri/src/external_agents/skill_stage.rs
grep -Fq 'pub const AGENT_DEFS: &[RuntimeAgentDef] = &[pi::PI_AGENT_DEF];' \
  src-tauri/src/external_agents/registry.rs

echo "BEEFEX_IDENTITY_OK allowed_legacy_lines=$(wc -l <"$matches" | tr -d ' ')"
