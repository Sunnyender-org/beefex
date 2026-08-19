# Beefex Repository Rules

## Product contract

Beefex is BeefAPI's open-source desktop coding agent. React + Tauri owns the product shell; the pinned official Pi coding-agent runtime owns cognition, tools, skills, extensions, sessions, compaction, and recovery.

- BeefAPI owns account, entitlement, model availability, balance, and payment truth.
- Beefex stores Task metadata and Pi session references; Pi owns the agent transcript and runtime session.
- Project trust is separate from scoped file and shell approval.
- The selectable legacy runtime, generic runtime adapter, standalone coding CLI, and inherited product plugin surface remain retired.

## Current platform boundary

- macOS arm64 is the active development and packaging target.
- Published Alpha artifacts are unsigned and unnotarized.
- Windows packaging and runtime support are not ready.
- Do not describe a local build or unsigned DMG as a production-ready release.

## Source and identity rules

- Start with `README.md`, then inspect the nearest code and tests.
- React/TypeScript UI lives under `src/`; the Tauri/Rust host lives under `src-tauri/`; build and contract helpers live under `scripts/`.
- Preserve Beefex product identity and the required Kivio, BF Labs, and Pi notices in `LICENSE` and `NOTICE`.
- Use the BF Labs screen language without creating an unrelated generic component platform.
- BeefAPI credentials remain backend-owned and must not be written to renderer state or Pi configuration files.

## Verification

```bash
npm run lint
npm run typecheck
npm test
cargo test --manifest-path src-tauri/Cargo.toml
```

Use the nearest additional Pi, identity, BF Labs source, or packaging check when changing those surfaces. Signing, notarization, release publication, and deployment require separate authorization.

