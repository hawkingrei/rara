# 2026-04-22 Codex Responses Backend

## Summary

RARA's `codex` provider had still been routed through the generic
OpenAI-compatible `chat/completions` path even after Codex auth/login had been
aligned to `codex_login`.

That mismatch meant:

- Codex auth could succeed,
- the TUI could route into Codex model selection,
- but actual prompt execution still hit `/v1/chat/completions` instead of the
  upstream Codex Responses API contract.

This checkpoint switches the Codex provider to a dedicated `/v1/responses`
request path, aligned with local `codex-rs`.

## Upstream Alignment

The local `codex-rs` inspection showed:

- `codex-rs/model-provider-info` only supports `wire_api = "responses"`
- the old `chat` wire API is explicitly removed
- `codex-rs/cli/src/responses_cmd.rs` uses `codex_api::ResponsesClient`

RARA now mirrors that protocol boundary instead of treating Codex as a plain
OpenAI-compatible chat-completions provider.

## What Landed

- `src/llm/openai_compatible.rs`
  - `CodexBackend` no longer delegates `ask()` and `summarize()` to the generic
    `OpenAiCompatibleBackend`
  - Codex requests now target `POST /v1/responses`
  - RARA history is converted into Responses-style input items:
    - user/system messages -> `message` with `input_text`
    - assistant text -> `message` with `output_text`
    - assistant tool uses -> `function_call`
    - tool results -> `function_call_output`
  - Responses output is converted back into RARA's internal content model:
    - output text -> `ContentBlock::Text`
    - function calls -> `ContentBlock::ToolUse`
- `src/llm/tests.rs`
  - added focused regression coverage for:
    - message history -> Responses input conversion
    - Responses output -> RARA content parsing

## Validation

Focused verification for this checkpoint:

- `cargo test converts_history_to_codex_responses_input_items -- --nocapture`
- `cargo test parses_codex_responses_output_into_text_and_tool_use_blocks -- --nocapture`
- `cargo check`
