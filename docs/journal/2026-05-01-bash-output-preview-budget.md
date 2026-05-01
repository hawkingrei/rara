# Bash Output Preview Budget

## Context

The foreground Bash result already captured `stdout`, `stderr`, and
`aggregated_output`, but long model-facing output still used a head-only preview.
That made failing commands easy to misread when the useful error appeared near the
tail.

## Upstream Alignment

- Codex formats exec output from aggregated output and truncates model-facing
  content with head/tail retention.
- Claude Code persists oversized tool results to a session `tool-results`
  directory and replaces model context with a bounded preview plus the full output
  path. It also applies an aggregate tool-result budget per model-visible message.

## Implementation Checkpoint

- Kept `aggregated_output` as the raw combined Bash capture and added
  `model_preview_output` as an independent head/tail preview field.
- Biased failed Bash previews toward the tail so compiler and test errors remain
  visible. Missing or unknown exit status now uses the same error-oriented
  preview instead of being treated as success.
- Centralized Bash preview shaping in the tool-result layer so foreground Bash
  execution and persisted-result fallback use the same head/tail behavior.
- Replaced oversized model-facing tool results with a `<persisted-output>` block
  that includes the saved full-result path and bounded preview.
- Added a lightweight tool-result batch budget before feeding parallel tool
  results back to the model. The final compacted batch is constrained to the
  aggregate budget even when many large tool results arrive together.
- Strengthened Bash tool descriptions with Codex/Claude-aligned command
  discipline: prefer dedicated file tools, use `cwd` instead of `cd`, split
  independent validation commands into separate tool calls, and avoid shell-side
  `2>&1`/`head`/`tail` trimming only to manage model-visible output.
- Relaxed `replace` read-state validation for partial reads. This follows the
  Codex apply-patch safety shape: the edit re-reads the current file and must
  match the expected old text, so a large-file preview should not deadlock exact
  string replacement. `replace_lines` remains full-read-only because it is based
  on line numbers instead of old text context.

## Validation

- `cargo test tool_result`
- `cargo test model_preview_bash_output_preserves_error_tail`
- `cargo test agent::tests::planning`
- `cargo test tools::file::tests`
- `cargo check`
