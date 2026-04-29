# Structured Inspection Continuation

RARA's agent loop now treats repository-inspection continuation as an explicit
runtime contract instead of an inferred prose intent.

## Changes

- Tightened the default and plan-mode prompts so additional repository evidence
  requires either a same-response inspection tool call or `<continue_inspection/>`.
- Changed execute-mode no-tool continuation so prior inspection evidence does
  not keep the turn open by itself.
- Added an SSE idle timeout for OpenAI-compatible and Codex streaming backends
  so a provider stream that stops producing events fails the turn instead of
  leaving the TUI in an indefinite streaming state.
- Added a timeout boundary for automatic history compaction. If automatic
  compaction times out, the query continues without compaction; manual
  `/compact` still reports the timeout as a failed compaction.
- Added automatic compaction failure backoff. After an automatic compaction
  timeout or summarization failure, RARA records the failure and skips
  automatic retry until the conversation grows by another token interval. A
  later successful compaction clears the backoff state.
- Added a DeepSeek v4 context budget override so 1M-token models do not fall
  back to the default 10k-token budget and compact too early.
- Split streaming model callbacks into text deltas and reasoning deltas. RARA
  now surfaces DeepSeek `reasoning_content` and Codex reasoning summary deltas
  as transient live Thinking output while still preserving DeepSeek
  `reasoning_content` in provider metadata for API roundtrip.
- Added DeepSeek thinking compatibility fallback for legacy histories. If an
  old assistant message has no preserved `reasoning_content`, RARA folds the
  incompatible prefix into a normal system context note and keeps later
  reasoning-compatible history intact. This keeps thinking enabled without
  fabricating provider reasoning content.
- Added Codex-style TUI input recall for the composer. Submitted inputs are
  kept in local session history, `Up` / `Down` browse those entries, and moving
  past the newest entry restores the draft that was active before browsing.
- Mapped TUI mouse wheel events to transcript scroll when the terminal sends
  mouse events, but left mouse capture disabled by default so terminal text
  selection and copy keep working normally.
- Added a focused agent planning regression test for the execute-mode
  structured continuation boundary.

## Reference Behavior

- Codex-style planning relies on explicit plan artifacts and non-mutating tool
  exploration rather than natural-language intent detection.
- Codex-style compaction treats automatic and manual compaction as explicit
  trigger kinds, emits structured lifecycle events, and keeps failures scoped to
  the compaction task instead of leaving the main query loop ambiguous.
- Claude-style query loops continue from structured `tool_use` / `tool_result`
  / follow-up state and stop when no structured continuation remains. Automatic
  compaction returns both a compaction result and consecutive failure state, so
  repeated compaction failures can be handled without blocking every turn.
- Codex and Claude both expose model thinking as runtime-visible progress
  instead of waiting for the final assistant message. RARA mirrors that shape
  with a transient TUI Thinking stream that is not committed as ordinary
  assistant transcript text.
- Codex keeps composer input recall inside the chat composer rather than using
  transcript scrolling as the arrow-key fallback. RARA mirrors the local
  session part of that behavior and leaves cross-session persistent history and
  reverse search as future extensions.

## Validation

- `cargo test execute_mode_continuation_requires_structured_inspection_marker -- --nocapture`
- `cargo test automatic_compaction_timeout_does_not_block_query -- --nocapture`
- `cargo test automatic_compaction_failure_suspends_retry_until_history_grows -- --nocapture`
- `cargo test successful_compaction_clears_auto_failure_backoff -- --nocapture`
- `cargo test derives_context_budget_for_deepseek_v4_models -- --nocapture`
- `cargo test codex_stream_reasoning_delta_is_reported_without_agent_text -- --nocapture`
- `cargo test agent_thinking_delta_updates_live_thinking_without_transcript_entry -- --nocapture`
- `cargo test active_turn_cell_shows_live_thinking_stream -- --nocapture`
- `cargo test deepseek_streaming_reasoning_content_preserves_exact_bytes -- --nocapture`
- `cargo test deepseek_tool_call_reasoning_content_roundtrips_without_trimming -- --nocapture`
- `cargo test deepseek_explicit_thinking_folds_legacy_assistant_history_without_reasoning -- --nocapture`
- `cargo test deepseek_explicit_thinking_stays_enabled_for_reasoning_compatible_history -- --nocapture`
- `cargo test deepseek_explicit_thinking_keeps_compatible_suffix_after_legacy_prefix -- --nocapture`
- `cargo test input_history -- --nocapture`
- `cargo test mouse_wheel -- --nocapture`
- `cargo check`
- `git diff --check`
