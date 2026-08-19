# Beefex Alpha updater contract

This is the in-app updater contract for the current Alpha line. It is not a generic release framework and it does not reuse the legacy Electron / Kivio `latest.yml` identity.

## Bootstrap truth

This source line is `0.1.0-alpha.6`. About shows `v0.1.0-alpha.6`.

Already-published Alpha 4 reports internal `0.1.0` and ships the old compile-time-disabled updater (`BEEFEX_UPDATE_REPO` unset). **Installed Alpha 4 cannot automatically discover or install Alpha 5 or later.** Alpha 5 is the one-time manual/bootstrap release that introduces the updater. Users on Alpha 4 must install Alpha 5 by hand from the download page.

Alpha 6 is the first candidate for a real in-app Alpha 5 → Alpha 6 upgrade. That upgrade is not proven until a published Alpha 6 object is discovered and installed from a running Alpha 5. Version-comparator tests may still order `0.1.0` before `0.1.0-alpha.N`; that is comparator-only and is not an auto-upgrade claim.

Versioned Alpha 4 objects remain at `beefex/releases/v0.1.0-alpha.4/` for manual rollback.

## Live objects

Download page and in-app installer use the same stable R2 names:

- `https://pub-e540a6ea6d6e4af19d7f5fc4d1f07c47.r2.dev/beefex/releases/latest/beefex-desktop-mac-arm64.dmg`
- `https://pub-e540a6ea6d6e4af19d7f5fc4d1f07c47.r2.dev/beefex/releases/latest/beefex-desktop-win-x64.exe`
- `https://pub-e540a6ea6d6e4af19d7f5fc4d1f07c47.r2.dev/beefex/releases/latest/SHA256SUMS.txt`

Do **not** consume `latest.json` or Electron `latest.yml` / `latest-mac.yml`.

## Check sequence

1. Fetch R2 `SHA256SUMS.txt`. Missing or malformed is a failed check.
2. If `beefex-updater.json` exists and matches `beefex.updater.v1` + `product=Beefex` + `identifier=com.beefapi.beefex`, and its platform SHA-256 matches `SHA256SUMS.txt`, use it.
3. Otherwise list GitHub releases on `Sunnyender-org/beefex`, pick the newest non-draft tag, read `beefex.alpha-artifact.v1`, and require that receipt SHA-256 to match R2 `SHA256SUMS.txt`.
4. Download only the R2 latest object for the current platform. Verify SHA-256 before install.

Generate metadata at release time from the real SHA256SUMS and commit. The generator accepts any `0.1.0-alpha.N`; the current example/default is `0.1.0-alpha.6`:

```bash
node scripts/build-beefex-updater-metadata.mjs \
  --sha256sums /path/to/SHA256SUMS.txt \
  --version 0.1.0-alpha.6 \
  --commit <release-commit>
```

Publishing that object to R2 is an external Owner gate.

## macOS install

Mount the DMG read-only. Require exactly one real, non-symlink `Beefex.app`. Copy it with `ditto` to a unique hidden staged app **in `/Applications`**. Verify staged `CFBundleIdentifier=com.beefapi.beefex` and the expected version before any swap. Rename the current `Beefex.app` to a unique hidden backup in the same directory, then rename staged to the target. If swap or `open` fails, move the failed target to a same-directory failed name and restore the backup. Do not run `xattr`, do not change Gatekeeper, and never touch Application Support / AppData.

Same-directory names:

- `/Applications/Beefex.app`
- `/Applications/.Beefex.staged-<id>.app`
- `/Applications/.Beefex.previous-<id>.app`
- `/Applications/.Beefex.failed-<id>.app`

## Windows compile

Mac-only install helpers are `cfg(any(target_os = "macos", test))`. Windows release builds do not compile them. Windows test builds compile the planner/identity tests. GitHub Actions `quality-windows` remains a required compile/test gate for the Windows sidecar, Pi runtime, `cargo test`, and unsigned NSIS package.

## Data preservation

Install must not write into any path containing `com.beefapi.beefex`, including a `.dmg` or `.exe` sitting under Application Support / AppData.

This document does not make Beefex Alpha Ready.
