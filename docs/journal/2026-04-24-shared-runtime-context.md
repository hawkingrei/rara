# 2026-04-24 Shared Runtime Context Contract

## Summary

RARA now has a shared runtime-context view that the agent can expose to other
surfaces instead of forcing TUI state to reconstruct prompt, plan, and
compaction details field by field.

## What Changed

- Added `SharedRuntimeContext` and related view structs in
  `src/context/runtime.rs`.
- Added `Agent::shared_runtime_context()` in `src/agent/context_view.rs`.
- Updated TUI snapshot sync to consume the shared runtime-context contract
  instead of rebuilding prompt/plan/compaction fields directly from scattered
  agent state.

## Why

This is the first concrete step toward the context-architecture spec:

- one runtime-owned object describes the effective prompt view;
- plan state and compaction state are grouped with that runtime context;
- TUI status surfaces now depend on a shared contract instead of open-coded
  extraction logic.

## Validation

- `cargo check`
- `cargo test agent::tests::context_view -- --nocapture`

## Follow-Up

- Extend the shared runtime context so session restore can consume the same
  contract instead of reconstructing adjacent state independently.
- Add a dedicated `/context` surface on top of this shared contract.
