# 2026-04-27 · Tool Loop Final Fallback

## Summary

RARA now treats the per-turn tool-round limit as a bounded stop condition
instead of a hard query failure. When the model keeps requesting tools after the
limit, the runtime closes the transcript with explicit skipped-tool results and
returns a local final fallback message.

## What Changed

- A final-answer continuation now tells the model to state limitations instead
  of emitting another tool call.
- If the over-limit model turn contains tool calls, RARA records synthetic
  error `tool_result` messages for those calls before asking for a final answer.
- If the forced final-answer turn still emits tool calls or no text, RARA:
  - records synthetic error `tool_result` messages for any skipped calls;
  - emits a bounded assistant fallback instead of returning `Query failed`;
  - preserves plan state and any partial model text in the fallback.
- Agent tests now cover the case where the model keeps emitting tool calls even
  after tools are disabled for the final-answer attempt.

## Why

Claude Code's loop design keeps the core model/tool cycle simple, but protects
the surrounding transcript invariants: tool-use blocks that reach the transcript
must be paired with tool-result blocks, even when execution is interrupted or
blocked. Applying the same boundary in RARA prevents orphan tool calls from
leaking into OpenAI-compatible follow-up requests and keeps the user-facing TUI
from ending on a low-level loop-limit error.

## Validation

- `cargo fmt --check`
- `cargo test agent::tests::planning -- --nocapture`
- `cargo check`
