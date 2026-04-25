# Prompt Runtime Specification

## Problem

RARA originally built its system prompt inline inside the agent and treated compaction as a generic
summary request. That made prompt composition hard to reason about, hard to override, and difficult
to align with the prompt-management patterns used by mature coding agents.

## Scope

- Effective system prompt assembly for normal agent turns.
- Prompt source discovery from workspace and runtime state.
- Prompt override and append behavior from config.
- Dedicated prompt handling for context compaction.
- Shared runtime bootstrap wiring for prompt runtime inputs.

## Non-Goals

- Codex-style state DB driven instruction layering.
- Prompt caching or token-level prompt reuse.
- Full coordinator / worker prompt families.

## Architecture

### 1) Prompt Runtime Inputs

The effective prompt may draw from:

- the default built-in prompt family;
- a configured custom system prompt;
- an optional append prompt;
- workspace instruction files;
- local memory files;
- runtime context;
- plan-mode specific guidance.

### 2) Base Prompt Selection

- If `system_prompt` or `system_prompt_file` is configured, that content replaces the default base
  prompt family.
- Otherwise the default built-in prompt family is used.
- Dynamic runtime sections still apply even when a custom base prompt is configured.

### 3) Effective Prompt Composition

The effective prompt is assembled in this order:

1. base prompt;
2. dynamic instruction sources;
3. memory sources;
4. runtime context;
5. mode-specific addenda such as plan mode;
6. append prompt.

### 4) Compact Prompt

- Context compaction must not use the normal system prompt.
- Compaction uses a dedicated compact instruction.
- `compact_prompt` or `compact_prompt_file` overrides the built-in compact instruction.

## Contracts

### 1) Prompt Observability

- The TUI status view must be able to report:
  - whether the base prompt is default or custom;
  - which prompt sections are active;
  - which prompt sources participated in assembly.
- The prompt inspection surface must preserve assembly order and explain for each injected source:
  - what kind of source it was;
  - the display path or source label;
  - why it was included.
- The same source-aware inspection surface should also describe any active compacted-history inputs
  that still contribute to the current turn, including:
  - compaction boundary metadata;
  - structured compacted summaries;
  - recent-file carry-over;
  - recent-file excerpt carry-over.
- The same inspection surface should expose memory/retrieval readiness separately from active
  prompt injection so the runtime can distinguish:
  - sources that are active now;
  - sources that are available for recall;
  - sources that are not currently available.
- The same inspection surface should also show which memory-like items are actually active in the
  current turn, starting with:
  - active workspace memory files that were injected into the effective prompt;
  - compacted thread-memory carry-over such as structured summaries and recent-file carry-over.
  - selected retrieval results reconstructed from retrieval-tool outputs when the current turn has
    already performed explicit recall.
- Session restore must rebuild the same prompt/runtime surface that a direct run would produce for
  persisted session-scoped state such as execution mode, append prompt text, and prompt warnings.

### 2) Workspace Prompt Sources

- Workspace instructions and local memory are treated as explicit prompt sources instead of opaque
  text blobs.
- Prompt source discovery must remain reusable across agent runtime and TUI status reporting.

### 3) Agent Loop Integration

- `Agent::build_system_prompt()` must delegate to the prompt runtime instead of hand-building the
  prompt inline.
- Compaction must pass the dedicated compact instruction down to every backend summarization path.

### 4) Runtime Bootstrap Contract

- Runtime/bootstrap callers must initialize workspace, prompt runtime config, skills, and tools
  through one shared entrypoint instead of wiring those pieces independently in `main.rs` and TUI
  rebuild paths.
- The shared bootstrap entrypoint is `initialize_rara_context(...)` in `src/runtime_context.rs`.
- Bootstrap warnings from prompt/runtime configuration or skill loading must remain visible to the
  caller instead of being silently dropped.
- Workspace-scoped persistence paths used by bootstrap-owned tools should derive from the resolved
  workspace data directory rather than hard-coded literals such as `data/lancedb`.

## Validation Matrix

- `cargo check`
- focused prompt runtime tests for source discovery and prompt precedence
- focused agent tests for compaction instruction wiring

## Open Risks

- The current runtime is closer to Claude-style prompt management than Codex-style instruction and
  state layering.
- Prompt observability now exists in both `/status` and `/context`, including active selected
  workspace/thread memory items, but deeper memory inspection still needs to cover real recalled
  vector/thread selection instead of only prompt-injected or compacted carry-over.

## Source Journals

- [2026-04-17-prompt-runtime](../journal/2026-04-17-prompt-runtime.md)
- [2026-04-24-context-observability-and-restore](../journal/2026-04-24-context-observability-and-restore.md)
