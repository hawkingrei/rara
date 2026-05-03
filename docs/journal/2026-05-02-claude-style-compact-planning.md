# 2026-05-02 Claude-Style Compact Planning

## Summary

Aligned RARA's compaction split decision with Claude Code's API-round boundary model.

## Changes

- Replaced the raw `history.len() * 0.8` split with a compact planning step that groups history by
  assistant API rounds.
- Kept assistant `tool_use` messages together with their matching user `tool_result` messages in
  the retained suffix.
- Added a token-budget-aware retained suffix target while always preserving at least the newest API
  round.
- Extracted post-compact carry-over assembly so future memory, hook, skill, MCP, or runtime-state
  reinjection can plug into one ordered stage.
- Fixed manual compaction to keep only the newest API round by default, so on-demand compaction
  actually summarizes the recent inspected context instead of retaining almost the whole suffix.
- Documented the context-cache prefix stability rule for post-compact history assembly.
- Added the auxiliary-model routing contract for compact workers: auxiliary models are reserved for
  low-risk helper reasoning such as compression or routing, use an explicit or conservatively
  inferred lite model when available, and fall back to the main model if no lite model exists.
- Updated the context compression spec to record the new boundary and carry-over contract.

## Validation

- Focused compact unit tests cover API-round grouping and retained-suffix planning.
- Agent compact tests cover retaining a recent tool-use/tool-result pair after manual compaction.

## Follow-Up

- Add explicit memory and hook carry-over stages once those runtime records have stable source
  descriptors.
- Add prompt-too-long retry logic that drops oldest API-round groups from the compaction request.
- Add partial compact support around a selected message boundary.
