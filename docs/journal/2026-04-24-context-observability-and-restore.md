# 2026-04-24 · Context Observability And Restore Alignment

## Summary

RARA now exposes structured prompt/context assembly details and restores enough session-scoped
runtime state for `session restore` to rebuild the same prompt/runtime surface as a direct run.

## What Changed

- Extended shared runtime context prompt views with structured source entries:
  - assembly order;
  - source kind;
  - display path / label;
  - human-readable inclusion reason.
- Updated TUI runtime snapshots and `/status` / `/context` rendering to consume those structured
  source entries instead of only showing flat status strings.
- Extended the same source-aware contract into compaction/runtime inspection so `/context` can now
  describe active compacted-history inputs such as:
  - the compaction boundary marker;
  - the structured compacted summary;
  - recent-file carry-over;
  - recent-file excerpt carry-over.
- Added a first retrieval/memory state surface to `/context` so the runtime can explain:
  - which memory sources are active now;
  - which sources are merely available for future recall;
  - which sources are currently missing.
- Extended the retrieval/memory inspection surface with explicit selected items so `/context` can
  now distinguish:
  - registered memory sources;
  - memory-like items that are actually active in the current turn.
- The first selected-item contract currently covers:
  - active workspace memory files injected into the prompt;
  - compacted thread-memory carry-over such as structured summaries, recent-file lists, and recent
    file excerpts.
- The selected-item contract now also reconstructs explicit recall results from retrieval-tool
  outputs in history so `/context` can surface:
  - selected workspace-memory candidates returned by `retrieve_experience`;
  - selected thread-memory results returned by `retrieve_session_context`.
- Persisted session-scoped prompt runtime state in the state DB:
  - append system prompt text;
  - prompt warnings.
- Restored persisted runtime state during session restore:
  - agent execution mode;
  - bash approval mode;
  - append system prompt text;
  - prompt warnings.
- Added focused tests covering:
  - structured prompt source observability;
  - session runtime state persistence;
  - restore-time runtime/context alignment.

## Why

This closes a real Phase 1 gap in the runtime/context contract:

- direct runs and restored sessions were not rebuilding the same context surface;
- `/status` could describe prompt sections, but not explain injected sources clearly;
- `/context` needed source-aware observability to be useful as a debugging surface.

## Remaining Follow-Up

- Continue shrinking `src/main.rs` so it becomes a thin CLI dispatcher over the shared bootstrap
  and runtime builder surfaces.
- Push the same structured context contract deeper into retrieval/memory layers so `/context`
  explains not only prompt sources, but also real recalled vector/thread memory selection beyond
  prompt-injected workspace memory and compacted-history carry-over.
