# Cache Status In /context

## Date

2026-05-01

## Summary

Added a `CacheStatus` enum to `src/context/runtime.rs` and threaded it through
the `/context` debugger so assembly entries can expose whether a source was
served from the in-memory workspace cache once that signal is wired.

## Changes

- Added `CacheStatus` (`Hit` / `Miss` / `NoCache`) to `src/context/runtime.rs`.
- Added `cache_status: Option<CacheStatus>` to `ContextAssemblyEntry`.
- Updated `assemble_context_view` in `src/context/assembly_view.rs` to
  populate `cache_status: None` on every entry (reserving the field for
  future wiring when the `WorkspaceMemory` cached_file_content signal is
  propagated).
- Updated the `/context` render in `src/tui/command.rs` to show cache state as
  compact markers for entries with a known status: `●` for hit, `○` for miss,
  and `-` for no-cache. Unknown or not-yet-wired status renders no marker.
- Re-exported `CacheStatus` from `src/context/mod.rs`.

## Key Decision

`PromptSource` and `PromptSourceContextEntry` do not carry transient cache
signals directly. The cache status field is reserved for the display layer
(`ContextAssemblyEntry`) and the retrieval layer (`retrieval_view.rs` already
reports workspace memory availability). This avoids breaking the equality
contracts that `assemble_turn_context == shared_runtime_context` depends on.

## Validation

- `cargo test context` — 37 passed
- `cargo test thread_store` — 14 passed
