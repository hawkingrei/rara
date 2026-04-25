# Thread Store Boundary

## Summary

RARA now has a first-pass local thread persistence boundary instead of having
session restore and TUI persistence read and write `SessionManager` and
`StateDb` directly in parallel.

This change does not replace the current storage format yet. It adds a minimal
boundary that adapts the existing local JSON + SQLite persistence into
structured thread-facing objects.

## What Landed

- Added `src/thread_store.rs` with:
  - `ThreadStore`
  - `ThreadRecorder`
  - `ThreadSnapshot`
  - `RolloutItem`
  - `CompactionRecord`
- `tui/session_restore.rs` now restores through `ThreadStore::load_thread(...)`
  instead of rebuilding the session from `SessionManager` and `StateDb`
  separately.
- `tui/state/persistence.rs` now writes runtime state, plan state, interaction
  state, and committed turns through `ThreadRecorder`.
- Added a first-pass unified structured rollout-event log:
  - `events.jsonl`
  - used for `compaction`, `plan_state`, and `interaction` items
  - preferred by `ThreadStore` over older specialized persistence files
  - now also the only new write target for non-turn rollout items

## Why This Matters

This creates an explicit boundary for:

- thread lifecycle reads;
- thread-scoped rollout persistence writes;
- compaction metadata loading;
- plan and pending-interaction continuity.

That in turn reduces the risk that:

- session restore and live persistence drift apart;
- future compaction or resume work needs to touch unrelated TUI code paths;
- thread history remains spread across unrelated persistence entrypoints.

## Current Scope

This is intentionally only the first pass.

The local thread boundary still adapts the existing storage model:

- `SessionManager` remains the owner of `history.json`;
- `StateDb` remains the owner of structured session metadata, committed
  transcript turns, and compatibility fallbacks for runtime state;
- `events.jsonl` now acts as the preferred structured rollout-event surface for
  non-turn thread items.
- older compatibility files such as `events.json`, `runtime.json`, and
  `compactions.json` are now fallback read sources instead of the primary write
  path.

The new boundary is therefore a façade, not yet a storage redesign.

## Additional Checkpoint

The recent-session and resume-picker surface now consumes thread-facing
summaries instead of reading raw SQLite session-summary rows directly from the
TUI layer.

- `ThreadStore` now exposes `ThreadSummary` for recent-thread listing.
- `ThreadSummary` carries:
  - `ThreadMetadata`
  - preview text
  - compaction summary state
- the TUI resume picker now renders from that thread-facing summary instead of
  depending on `PersistedSessionSummary`.

This keeps the UI on the thread boundary and makes it easier to expand the
resume surface later with richer thread metadata without reintroducing direct
`StateDb` coupling into TUI code.

## Resume Behavior Checkpoint

The resume surface is now starting to move from session-shaped behavior toward
thread-shaped behavior:

- `rara resume [thread_id]` is again a first-class CLI entrypoint.
- `rara resume --last` is now an explicit CLI entrypoint for "continue the newest
  persisted thread" instead of relying only on the default `rara tui` startup
  behavior.
- plain `rara` / `rara tui` now starts a fresh thread by default instead of
  implicitly restoring the latest persisted thread.
- TUI startup restore now attaches the state DB before attempting a restore, so
  startup resume can actually hydrate a persisted thread instead of silently
  skipping the restore path.
- user-facing TUI copy now refers to recent local threads rather than recent
  local sessions on the resume path.

This is still only a partial thread-lifecycle surface, but it restores the
basic CLI/TUI resume contract and removes one important source of restore
drift.

## Thread CLI Checkpoint

The thread boundary now has a first explicit read/list CLI surface in addition
to TUI resume:

- `rara threads [--limit N]`
  - lists recent persisted threads from the thread-facing summary surface
  - keeps the CLI on `ThreadStore` instead of SQLite-shaped session summaries
- `rara thread <THREAD_ID>`
  - reads one persisted thread and prints runtime metadata, rollout counts, and
    compaction metadata
  - avoids depending on backend initialization so thread inspection remains a
    persistence concern rather than a model/provider concern

This establishes a cleaner thread lifecycle split:

- `threads` for listing
- `thread` for inspection
- `resume` for continuation
- `fork` for branching the current materialized thread state into a new thread

## Fork Checkpoint

The thread lifecycle now includes a minimal persisted fork path:

- `rara fork <THREAD_ID>` creates a new thread id instead of mutating the
  source thread
- the forked thread preserves:
  - canonicalized history
  - current plan/interactions runtime state
  - committed turns
  - structured compaction events
- the forked thread writes lineage metadata:
  - `origin_kind = "fork"`
  - `forked_from_thread_id = <source thread>`

The current fork is intentionally minimal:

- it clones the source thread's current materialized state
- it does not yet add TUI-native fork flows
- it does not yet preserve richer lineage such as parent/child indexes or
  resume-derived origins

## Internal Naming Checkpoint

The persistence read surface now starts using thread-shaped names internally as
well:

- `latest_session_id` -> `latest_thread_id`
- `list_recent_sessions` -> `list_recent_thread_summaries`
- `PersistedSessionSummary` -> `PersistedRecentThreadSummary`
- `load_session_record` / `PersistedSessionRecord` -> `load_thread_record` / `PersistedThreadRecord`

This does not change storage or schema yet. It only reduces the chance that new
thread-lifecycle work keeps growing on top of stale session-shaped read APIs.

## Rollout Migration Checkpoint

The non-turn rollout boundary now treats old JSON files as migration sources
instead of active write targets:

- `events.jsonl` is the only new write target for structured non-turn rollout
  items.
- `events.json`, `runtime.json`, and `compactions.json` are still readable for
  compatibility, but they are now read through migration/backfill paths.
- reading legacy rollout state now backfills the canonical `events.jsonl` log so
  subsequent thread loads can stay on the append-only path.
- `ThreadStore::load_thread(...)` now consumes one unified non-turn migration
  result instead of stitching runtime fallback and compaction fallback together
  at separate call sites.

This is still not a full storage redesign, but it does move the runtime closer
to the Codex/Claude-style model where append-only rollout logs are the primary
source of truth and compatibility files become import-only legacy paths.

## History Migration Checkpoint

Thread history is now starting to follow the same contract:

- the canonical history path is `rollouts/<thread>/history.json`
- older `sessions/<thread>.json` files are still readable for compatibility
- reading legacy history now backfills the canonical history file under the
  rollout root
- `ThreadStore::load_thread(...)` now restores history through that explicit
  history-migration helper instead of reading raw history files directly

This keeps thread restore on a more uniform "read through migration, then stay
on canonical paths" model instead of mixing one-off file lookups into the
thread boundary.

## Materialization Checkpoint

`ThreadStore` now has an explicit thread-materialization step instead of doing
all history + metadata + rollout assembly inline in `load_thread(...)`:

- `load_thread(...)` is now a thin wrapper
- `materialize_thread_state(...)` is the single place that assembles:
  - canonicalized history
  - thread metadata
  - compaction state
  - plan/interactions
  - ordered rollout items

This is still an internal object boundary, not a new persisted format, but it
gives later thread-lifecycle work a clearer place to add lineage, fork, or
other thread-scoped materialization rules without bloating the restore entry
point again.

## Follow-Up

Still open:

- add richer lineage metadata beyond the minimal `forked_from_thread_id`
  surface;
- separate thread lifecycle objects from TUI-specific restore logic further.
