# 2026-04-25 · Context Assembly Stage 1

## Summary

RARA now has a real Stage 1 context-assembly boundary: one assembler-owned
runtime view can explain the effective context, budget decisions, and restored
thread state across agent runtime, `/status`, and `/context`.

## What Changed

- Expanded the shared runtime context with an explicit `ContextAssemblyView`
  made of ordered assembly entries.
- Each assembly entry now records:
  - layer;
  - kind;
  - display label / source path;
  - injection status;
  - inclusion reason;
  - budget impact;
  - dropped reason where applicable.
- Expanded the context-budget surface from generic compaction counters into a
  Stage 1 budget breakdown:
  - model context window;
  - reserved output budget;
  - stable-instructions budget;
  - workspace prompt-source budget;
  - active-turn budget;
  - compacted-history budget;
  - retrieved-memory budget;
  - remaining input budget.
- `ContextAssembler` now owns one runtime assembly entrypoint that combines:
  - stable instructions;
  - workspace prompt sources;
  - active memory inputs;
  - compacted history;
  - active turn state;
  - retrieval-ready but not injected items.
- The assembler now also exposes one turn-level assembled result so agent
  callers can read:
  - the prompt/system view;
  - the runtime/debug view;
  from one shared turn contract instead of parallel helper paths.
- Agent runtime context construction now passes pending approvals and request
  inputs through this same assembly path.
- TUI runtime snapshots now store the same assembly entries and budget
  breakdown, and `/context` renders those layers directly instead of rebuilding
  a parallel explanation surface.
- Session restore now proves the same assembled view can be rebuilt after
  restoring:
  - compaction state;
  - plan state;
  - pending interactions.

## Why

Before this checkpoint, RARA already had prompt/runtime specs, shared runtime
context, and a thread-store boundary, but there was still no single object with
final ownership of the "effective context" explanation.

That left four drift risks:

- agent runtime context and TUI debug surfaces could explain different sources;
- restore could recover state without rebuilding the same context view;
- compacted history and active turn state were visible, but not through one
  ordered assembly contract;
- future retrieval injection would have had to bolt onto multiple partially
  overlapping context surfaces.

This Stage 1 cut closes those ownership gaps without forcing a larger rewrite
of the model-send path.

## Validation

- `cargo test agent::tests::context_view::shared_runtime_context_collects_prompt_plan_and_compaction_state -- --nocapture`
- `cargo test tui::command::tests::status_context_text_includes_prompt_sources_and_plan_state -- --nocapture`
- `cargo test tui::session_restore::tests::restore_session_keeps_runtime_context_and_snapshot_aligned -- --nocapture`
- `cargo test tui::session_restore::tests::restore_session_surfaces_pending_interactions_in_assembled_context -- --nocapture`
- `cargo check`

## Remaining Follow-Up

- Added a first `MemorySelection` contract on top of Stage 1 assembly so the
  runtime can distinguish:
  - selected memory-like inputs;
  - dropped-but-considered inputs;
  - a bounded selection budget for the current turn.
- The retrieval side now uses an explicit candidate pool:
  - fixed inputs stay selected because they are already injected elsewhere;
  - the fixed side now explicitly includes the active thread working set:
    - plan explanation / plan steps;
    - pending interactions;
    - latest user request;
    - recent tool results;
  - discretionary retrieval candidates compete for the remaining
    memory-selection budget;
  - dropped candidates carry concrete reasons such as:
    - compacted history already covering the thread need;
    - a more focused retrieved thread-context candidate winning;
    - the remaining memory-selection budget being exhausted.
- Retrieval tool outputs now normalize into more semantic candidate kinds:
  - `retrieved_workspace_memory`
  - `retrieved_thread_context`
  instead of only echoing raw tool names.
- This still stops short of real vector candidate ranking. The current dropped
  side can therefore include readiness-only items such as the configured vector
  memory store.

- Push this Stage 1 assembly contract deeper into real retrieval selection so
  `/context` can explain why vector/thread candidates won or lost the retrieval
  budget.
- Decide later whether the model-send path itself should be rebuilt directly
  from the same assembled-context object instead of the current prompt/history
  split.
