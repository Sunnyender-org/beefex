# Beefex

Beefex is BeefAPI's open-source desktop coding agent. React + Tauri owns the
product shell while the pinned official Pi coding agent owns cognition, tools,
skills, extensions, sessions, compaction, and recovery.

The core workflow is:

> Sign in to BeefAPI → open a local project → start a durable Task → review scoped file and shell actions → inspect the real diff and command output → quit and recover the same Pi session after relaunch.

## Status

Beefex is under active development as a macOS Alpha. An unsigned Apple Silicon
test DMG is available from GitHub Releases, but it is not a signed, notarized, or
generally supported production release.

- macOS arm64 is the current development and packaging target.
- Windows packaging and runtime support are not ready.
- Published Alpha artifacts are unsigned and not notarized.
- BeefAPI credentials remain backend-owned and are not written to renderer state or Pi configuration files.

## Architecture

- **Desktop shell:** React 18 + TypeScript + Tauri 2
- **Agent runtime:** `@earendil-works/pi-coding-agent` 0.84.1 over Pi RPC
- **Account and model authority:** BeefAPI
- **Persistence:** Beefex stores Task metadata and Pi session references; Pi owns the agent transcript and runtime session
- **Approvals:** project trust is separate from scoped file and shell approval
- **Design source:** BFLabs screen language, applied screen-first without a generic component platform

The selectable legacy runtime, generic runtime adapter, standalone coding CLI,
and inherited product plugin surface have been retired. The app, Rust crate,
main executable, and OCR helper use Beefex identity. Explicit compatibility
aliases remain only where required to read current Beefex data, and upstream
legal attribution remains in [`NOTICE`](NOTICE).

## Development

Requirements:

- macOS
- Node.js 20+
- npm
- Rust stable
- Tauri 2 platform prerequisites
- Bun, used by `npm run build:pi-runtime` to compile the pinned Pi runtime for packaging

Install dependencies and run the app:

```bash
npm ci
npm run dev
```

Run the main checks:

```bash
npm run lint
npm run typecheck
npm test
cargo test --manifest-path src-tauri/Cargo.toml
```

Build the macOS app locally:

```bash
npm run build
```

No deployment, signing, notarization, or release is performed by the public CI workflow.
The manually triggered clean macOS bundle workflow builds an unsigned app on a fresh
GitHub-hosted Apple Silicon runner, verifies the bundled Pi runtime and notices, and
performs an isolated-home startup smoke without uploading the app as an artifact.

## Security

Please do not open public issues containing credentials, private source code, account information, or unredacted logs. Use GitHub's private vulnerability reporting for security-sensitive reports when it is available for the repository.

## License

Beefex is a modified derivative of [Kivio](https://github.com/ZMGID/kivio) and is licensed under `GPL-3.0-or-later`.

See [`LICENSE`](LICENSE) and [`NOTICE`](NOTICE) for the complete source and attribution obligations. BF Labs-derived UI code and Pi coding-agent integration retain their respective notices described in `NOTICE`.

## 中文

Beefex 是一个开源的 BeefAPI 登录即用桌面 coding agent。当前产品执行链使用官方 Pi coding-agent runtime；React + Tauri 负责桌面产品壳、Task、审批、diff、命令回执和恢复界面。

目前只把 macOS arm64 作为开发与验收目标。GitHub Releases 提供未签名、未公证的
Apple Silicon 测试包，但它不是正式生产发布，Windows 版本也尚未完成。产品身份、
主执行文件和 OCR helper 已收敛为 Beefex；Pi 通过原生 RPC、Skills、Extensions 和
Packages 提供 coding-agent 能力。
