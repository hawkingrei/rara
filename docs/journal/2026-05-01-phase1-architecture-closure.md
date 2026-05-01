# Phase 1 Architecture Closure Checkpoint

## Date

2026-05-01

## Context

Phase 1 of the architecture closure (per `docs/todo.md`) targeted four areas:
`/context`, `MemorySelection`, `ThreadStore`/`ThreadRecorder`, and compaction lifecycle.

## Changes

### 1. MemorySelection Focused Tests (`src/context/memory_selection.rs`)

Added 6 focused tests that lock down the non-vector selection contract:

- `thread_history_selected_when_no_compacted_history_and_budget_allows` — proves the non-vector
  thread-history path selects when no compacted history exists and budget allows.
- `thread_history_available_not_selected_when_compacted_history_exists` — proves the compacted-history
  deduplication rule moves thread_history to available.
- `vector_memory_is_available_but_not_selectable` — proves the vector-memory placeholder stays in
  available with an explicit "not implemented" reason.
- `retrieval_tool_results_from_history_are_captured_as_candidates` — proves that
  `retrieve_experience`/`retrieve_session_context` tool results from the conversation history are
  captured as discretionary candidates.
- `retrieval_tool_results_selected_when_budget_allows` — proves retrieval tool results win the
  budget-aware selection when budget is sufficient.
- `memory_selection_reports_all_three_categories` — proves that selected, available, and dropped
  items are all populated in the same call.

### 2. ThreadStore Boundary (`src/thread_store.rs`)

Added explicit lineage and provenance inspection:

- `ThreadMetadata::is_fork()` — returns true when `origin_kind == "fork"` and a source thread id exists.
- `ThreadMetadata::lineage()` — returns `(origin_kind, Option<forked_from_thread_id>)`.
- `ThreadSnapshot::provenance_description()` — human-readable string summarizing where metadata,
  history, and non-turn rollout items were sourced from.
- `ThreadMaterializationProvenance::describe()` — maps each `Thread*Source` variant to a concise
  label, making the canonical / legacy-backfill / StateDb-fallback hierarchy explicit.

### 3. Todo Updated

Updated `docs/todo.md` to reflect completed Phase 1 items:

- Marked MemorySelection non-vector path as complete.
- Marked `/context` data pipeline as solid.
- Marked ThreadStore lineage/provenance as complete.
- Left remaining items explicitly tagged with `[ ]`:
  - 1B: LanceDB-backed vector retrieval.
  - Wire actual cache hit/miss signals from `WorkspaceMemory` into `/context` cache status.
  - Tightening `CompactState` / `CompactionRecord` ownership.

## Validation

- `cargo check` passes.
- `cargo test context::memory_selection` — 7 passed.
- `cargo test context::assembler` — 7 passed.
- `cargo test thread_store::tests` — 14 passed.
- `cargo test context` (all context-filtered tests including TUI context rendering) — 37 passed.

## Follow-up

- The largest remaining Phase 1 gap is replacing the mock `VectorDB` with real Lance/LanceDB
  retrieval (`docs/todo.md` Phase 1, item 1B).
- Compaction ownership tightening (`CompactState` vs `CompactionRecord`) can be addressed
  when the durable in-turn checkpoint work begins.
- Cache hit/refresh status is now wired through `CacheStatus` enum and `/context` rendering;
  actual WorkspaceMemory cache signals will populate the field once the `cached_file_content`
  return type is promoted to return `CacheStatus` alongside content.
