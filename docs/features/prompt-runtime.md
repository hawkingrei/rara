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
- Prompt observability exists in `/status` but there is not yet a dedicated prompt inspection UI.

## Source Journals

- [2026-04-17-prompt-runtime](../journal/2026-04-17-prompt-runtime.md)
