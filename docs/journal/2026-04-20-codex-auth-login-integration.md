# 2026-04-20 Codex Auth Login Integration

## Summary

RARA now uses `codex_login` directly for the main Codex auth paths instead of
maintaining a parallel bespoke OAuth implementation in `src/oauth.rs`.

This turns the earlier "Codex-style mirror" into a thin adaptation layer around
Codex's own browser login, device-code login, API-key login, and logout
primitives.

## What Landed

- `Cargo.toml`
  - adds a direct `codex-login` dependency
  - mirrors the upstream tungstenite workspace patches required for dependency
    resolution
- `src/oauth.rs`
  - wraps `codex_login::run_login_server(...)`
  - wraps `codex_login::request_device_code(...)`
  - wraps `codex_login::complete_device_code_login(...)`
  - wraps `codex_login::login_with_api_key(...)`
  - wraps `codex_login::logout(...)`
  - keeps a small RARA-local bridge that converts saved Codex auth state back
    into the provider credential that RARA stores in its config
- `src/main.rs`
  - CLI `login`, `login --device-auth`, `login --with-api-key`, and `logout`
    now all run through the new auth bridge
- `src/tui/runtime/tasks.rs`, `src/tui/runtime/commands.rs`, `src/tui/mod.rs`
  - TUI browser/device/API-key/login/logout flows now use the same bridge
  - logout clears both the local RARA credential and the underlying Codex auth
    storage

## Validation

Focused regression coverage now includes:

- `oauth::tests`
  - API-key persistence
  - saved-credential loading precedence
  - logout cleanup
  - browser-login bridge metadata
- `tui::tests::auth_mode_picker_prefers_selection_navigation`

Initial `insta` snapshot coverage now exists for:

- auth-mode picker popup
- queued follow-up preview
- Updated Plan active-turn rendering

## Follow-Up

- add broader callback-path validation if RARA needs more confidence around the
  browser-login bridge itself
- expand snapshot coverage across more popups and status-heavy transcript views
- keep MCP/appserver auth as a separate later feature instead of folding it
  back into first-party Codex login
