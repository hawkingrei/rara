# Approval UI Copy

## Context

Shell and plan approval cards used terse option labels such as `yes`, `prefix`,
`on`, and `no`. Those labels exposed implementation shorthand instead of the
decision scope, and active tool progress could compete with the approval card
when a command paused for review.

## Decision

- Plan approval choices now use action-oriented labels:
  - start implementation now;
  - continue planning and refine the plan.
- Shell approval choices now describe the scope:
  - allow once;
  - allow matching prefix for this session;
  - allow shell commands for this session;
  - deny the command.
- Active turn rendering suppresses older explicit tool progress while a pending
  interaction is active, keeping the approval request as the focused surface.
- DeepSeek v4/pro default chat-completion requests keep approved shell results
  as adjacent protocol tool messages before the runtime continuation. Legacy
  reasoning-history folding now applies only when DeepSeek thinking mode is
  explicitly enabled.

## Validation

- `cargo test interaction_text -- --nocapture`
- `cargo test active_turn_cell_renders_shell_approval_as_interaction_card -- --nocapture`
- `cargo test deepseek_v4_defaults_keep_tool_results_as_protocol_messages -- --nocapture`
- `cargo check`
