# 2026-05-01 MemorySelection Non-Vector First Cut

## Summary

Completed the first non-vector cut of the `MemorySelection` pipeline and
enhanced the `/context` TUI display.

## Changes

### ExperienceStore (`src/experience_store.rs`)

- Added a simple JSON-based persistent store for agent experiences.
- Lives at `rara_dir / "experiences.json"`.
- Supports keyword-based retrieval (word overlap scoring) without any
  vector/embedding dependency.
- Loads full store on construction, saves on mutation.

### Retrieval Tools Wired

- `remember_experience` — persists insights to `ExperienceStore`.
- `retrieve_experience` — keyword-matches against stored experiences and
  returns up to 5 results.
- `retrieve_session_context` — same keyword retrieval path (previously
  always returned `no_context_found`).

All three tools replaced their mock/no-op implementations with real I/O.

### `/context` Display Enhancement

The `/context` modal now opens with a compact **Context Overview** header:

- total window tokens, budgeted tokens, utilization percentage, remaining
  input budget;
- per-layer rollup showing entry count and combined token impact for each
  assembly layer (stable_instructions, workspace_prompt_sources,
  active_memory_inputs, compacted_history, active_turn_state,
  retrieval_ready);
- retrieval digest: selected/available/dropped counts, workspace memory
  activation status, compacted carryover status.

Additionally, the current session line now includes the agent execution mode
(execute/plan), and memory selection now appears after retrieval-ready items
to group the discretionary path more clearly.

## Contracts Preserved

- `MemorySelection` pipeline (selected/available/dropped + budget surface)
  unchanged — the selection logic already reads retrieval tool results from
  agent history; the tools now return real data.
- `SharedRuntimeContext` and `TuiSnapshot` data flow unchanged.
- All 542 existing tests pass.

## Validation

- `cargo check` — clean.
- `cargo test` — 542 passed, 0 failed.
- ExperienceStore unit tests cover: remember/retrieve, keyword scoring,
  empty query, limit, persistence round trip.

## Follow-up

- The retrieval path is keyword-based; vector-backed retrieval (1B in the
  Phase 1 rollout) remains deferred.
- `/context` display is complete for this milestone; further `status_runtime`
  enhancements remain in the provider-surface cleanup phase.
