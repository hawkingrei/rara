# 2026-05-03 Plan Exit Repair

## Summary

Plan mode now treats an `exit_plan_mode` call without a complete same-turn
`<proposed_plan>...</proposed_plan>` block as a recoverable submission error.
Instead of only displaying the tool error to the TUI and ending the turn, the
runtime records a structured `plan_exit_repair_required` continuation message
and gives the model one immediate repair attempt.

## Runtime Contract

- The repair message is written into the model-visible history as structured
  runtime context.
- `exit_plan_mode` now exposes a structured `proposed_plan` tool argument with
  `summary`, `steps`, and `validation` fields. This is the preferred transport
  because it uses the provider's function/tool-call argument channel instead of
  relying on free-form assistant text.
- OpenAI-native Chat Completions and Responses requests mark strict-compatible
  tool schemas with `strict: true`, allowing providers with Structured Outputs
  support to enforce the plan argument shape at the API layer.
- The model is told that Markdown headings, plain bullets, and ordinary prose
  are not substitutes for `<proposed_plan>`.
- The preferred `<proposed_plan>` artifact uses `summary:`, `steps:`, and
  `validation:` fields. Runtime parses executable plan steps only from the
  `steps:` field so validation commands do not become plan items.
- The repair attempt must either emit a complete `<proposed_plan>` block and
  call `exit_plan_mode`, or answer normally without calling `exit_plan_mode`.
- RARA still rejects stale plan state: the accepted plan must come from the
  same assistant response as the `exit_plan_mode` call.
- RARA still reports a specific malformed-plan error when the opening
  `<proposed_plan>` tag has no matching `</proposed_plan>` closing tag.

## Validation

- `cargo test exit_plan_mode -- --nocapture`
- `cargo test -p rara-instructions plan_mode_prompt_requires_short_progress_and_structured_approval -- --nocapture`
- `cargo check`
