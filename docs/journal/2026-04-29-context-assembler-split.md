# 2026-04-29 Context Assembler Split

## Summary

Split `src/context/assembler.rs` (1649 lines) into a submodule directory to
stay under the roughly-800-line guideline.

## New Module Layout

```
src/context/assembler/
  mod.rs              (519 lines) — core types, ContextAssembler impl, tests
  memory_selection.rs (518 lines) — full memory selection pipeline
  view.rs             (435 lines) — context view assembly, retrieval, budget helpers
  compaction.rs       (162 lines) — compaction metadata extraction
```

## Boundary

- `mod.rs` owns: `AssembledContext`, `AssembledTurnContext`, `ContextAssembler`,
  `RuntimeContextInputs`, `RuntimeInteractionInput`, tests.
- `memory_selection.rs` owns: `memory_selection()`, candidate structs,
  selection decision logic, retrieval-tool candidate extraction.
- `view.rs` owns: `estimate_text_tokens`, `active_turn_budget`,
  `assemble_context_view`, `compaction_context_entries`,
  `retrieval_context`, `latest_user_request`, `latest_tool_results`.
- `compaction.rs` owns: `compaction_source_entries()`, `CompactionSourceItem`
  struct, item summarization helpers.

No public API changes — all re-exports through `src/context/mod.rs` unchanged.
