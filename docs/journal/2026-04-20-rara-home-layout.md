# 2026-04-20 · RARA Home Layout

## Summary

Moved the default runtime path model away from project-local `.rara` and into a
Codex-style home layout rooted at `~/.rara`.

## Implemented

- `rara-config`
  - added helpers for:
    - `~/.rara`
    - workspace-scoped data directories under `~/.rara/workspaces/...`
  - added provider-scoped remembered config state so provider/model/base URL/API
    key changes do not flatten into one shared mutable profile
- production defaults updated to use shared helpers for:
  - config manager
  - Codex auth storage
  - session storage
  - sqlite state DB
  - sandbox profile storage
  - tool-result storage
  - workspace memory
- focused tests updated so state DB persistence no longer depends on
  `current_dir().join(".rara")`

## Notes

- Explicit temporary test fixtures that create `.rara` remain acceptable when a
  test wants a self-contained fake runtime root.
- The project tree should no longer be the default place where RARA creates
  `.rara` during normal operation.
