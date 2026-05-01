# Memory Selection Spec

## Context

`MemorySelection` already exists as a Stage 1 runtime view, but its contract was
still mostly embedded inside the larger context architecture document.

This note records the follow-up specification split and the priority change for
`/context`.

## Changes

- Added `docs/features/memory-selection.md` as the canonical contract for the
  per-turn memory-selection boundary.
- Clarified that context selection is the broader model-working-set assembly
  process, while `MemorySelection` is the narrower memory-like source and recall
  classification step inside that process.
- Defined selected, available, and dropped item semantics.
- Documented the ownership boundary with `WorkspaceMemory`, `ThreadStore`,
  compaction, retrieval backends, and `ContextAssembler`.
- Raised `/context` priority in `docs/todo.md` so the inspectable debugger lands
  before broader retrieval expansion.
- Refined the spec toward the Gemini CLI documentation style: user-facing
  inspection model first, then runtime contract, display contract, safety, and
  validation.

## Follow-Up

- Add focused `MemorySelection` tests for available and dropped reasons before
  replacing placeholder retrieval.
- Extend `/context` to show cache hit or refresh status once prompt-source cache
  invalidation has an explicit runtime signal.
