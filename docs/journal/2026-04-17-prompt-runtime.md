# Prompt Runtime Refactor

## Summary

Introduced a dedicated prompt runtime for RARA so system prompt assembly, workspace instructions,
memory injection, and compaction prompts no longer depend on ad hoc agent-local string building.

## Background

RARA's agent prompt path had grown around a single inline builder inside `agent.rs`. That made it
hard to reason about prompt precedence, difficult to override from config, and inconsistent with the
prompt-management patterns used by more mature coding-agent runtimes.

## Scope

- Added `src/prompt.rs` as the shared prompt runtime.
- Added config-backed prompt override, append, and compact prompt support.
- Refactored workspace instruction discovery into structured prompt sources.
- Routed normal system prompt assembly through the prompt runtime.
- Routed compaction through a dedicated compact instruction.
- Added prompt observability to `/status`.

## Key Decisions

- Follow Claude-style prompt-runtime layering first before adopting Codex-style config/state
  integration.
- Treat workspace instructions, memory, runtime context, and mode addenda as explicit prompt
  sections rather than opaque appended strings.
- Let a custom system prompt replace the default base prompt family while still preserving dynamic
  runtime sections.
- Keep compact prompt handling separate from the main system prompt.

## Validation

- `cargo check`
- `cargo test prompt::tests -- --nocapture`
- `cargo test agent::tests -- --nocapture`

## Follow-ups

- Add more specialized prompt families for memory extraction and tool-result summarization.
- Reduce remaining agent-loop continuation heuristics in favor of structured runtime state.
- Consider a later Codex-style instruction/config/state convergence after the Claude-style prompt
  runtime is stable.
