# 2026-04-24 Runtime Bootstrap Consolidation

## Summary

RARA now routes runtime/bootstrap assembly through a shared `initialize_rara_context(...)`
entrypoint in `src/runtime_context.rs` instead of duplicating backend/tool/workspace setup in
`src/main.rs` and `src/tui/runtime/tasks/builder.rs`.

## What Changed

- Added a shared runtime bootstrap path in `src/runtime_context.rs`.
- Moved backend selection, workspace/session/vector DB setup, prompt runtime config, skill loading,
  and tool registration behind the shared entrypoint.
- Updated `src/main.rs` to use helper command handlers and call `initialize_rara_context(...)`
  for `ask`, `tui`, and ACP startup.
- Updated the TUI rebuild path to reuse the same bootstrap entrypoint and surface bootstrap
  warnings back into the UI.
- Split CLI parsing and command dispatch out of `src/main.rs` into `src/app_cli.rs`, leaving the
  binary entrypoint as a thin startup wrapper.
- Moved tool-manager/skill-loading/vector-path bootstrap helpers into
  `src/runtime_context/tooling.rs` so runtime dependency wiring sits beside the shared bootstrap
  contract instead of drifting back toward the entrypoint.
- Switched vector-memory bootstrap wiring from a hard-coded `data/lancedb` path to the workspace
  data dir (`<workspace>/.rara/lancedb`).
- Stopped silently swallowing `SkillManager::load_all()` failures during bootstrap; warnings are
  now collected and surfaced to the caller.

## Validation

- `cargo check`
- `cargo test runtime_context::tests -- --nocapture`

## Follow-Up

- Continue toward a richer runtime context contract that can also carry stable instructions,
  compacted history, and context observability for `/context`.
