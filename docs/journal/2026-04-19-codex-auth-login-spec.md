# 2026-04-19 Codex Auth Login Spec Checkpoint

## Summary

Added a first dedicated spec for Codex-style first-party login in
`docs/features/codex-auth-login.md`.

This checkpoint started as a specification-only note, but the first
implementation pass is now in place as well.

## What Was Clarified

- RARA's current `src/oauth.rs` flow is only a lightweight provider OAuth path.
- The primary target should be Codex's own login model:
  - browser login
  - device-code login
  - API key login
- The primary reusable Codex crate for this work is `codex_login`.
- MCP/appserver OAuth is a separate second-layer concern and should not be
  conflated with first-party login.

## What Landed

- `src/oauth.rs` now targets Codex-style first-party auth instead of the older
  gateway-specific browser-only flow.
- CLI support now includes:
  - `rara login`
  - `rara login --device-auth`
  - `rara login --with-api-key`
  - `rara logout`
- The TUI `/login` path now opens a selection-style Codex auth picker with:
  - browser login
  - device code
  - API key
  - logout
- The picker now behaves more like Codex:
  - `Up/Down + Enter` is the primary interaction path
  - number keys are only secondary shortcuts
- Focused tests now cover:
  - `/login` and `/logout` command parsing
  - auth-picker key mapping
  - authorize URL issuer/client-id correctness

## Why This Matters

Without a Codex-aligned first-party login spec, RARA would likely continue
growing a bespoke login path that drifts from Codex behavior and makes later
appserver alignment harder. The first implementation pass now closes the main
product gap, but the crate-level reuse gap remains.

## Follow-Up

- Replace the local mirror in `src/oauth.rs` with direct `codex_login`
  primitives where the dependency shape is acceptable.
- Add broader auth-path tests around callback handling and persistence.
- Keep future MCP/appserver OAuth work separate from first-party login.
