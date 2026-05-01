# Shell Approval Shortcuts

## Context

Shell command approval was being represented through both `pending_approval`
and `pending_user_input`. That made the TUI vulnerable to rendering a bash
approval as a generic request-input card, which exposed the wrong shortcut hint
and could make numeric approval shortcuts appear unresponsive.

## Implementation Checkpoint

- Kept bash command approval on the structured `ShellApproval` path instead of
  also creating a generic pending user-input question.
- Centralized pending-option counts so key handling knows the real number of
  valid options for plan approval, shell approval, and request-input cards.
- Added the missing `4` shortcut for shell approval denial.
- Added exact numeric submit handling as a fallback for terminals where a digit
  lands in the composer before submission.
- Covered both local and SSH shortcut mapping in tests.

## Validation

- `cargo fmt --check`
- `cargo test pending_shell_approval_number_shortcuts_work_in_local_and_ssh -- --nocapture`
- `cargo test pending_shell_approval_does_not_render_as_request_input -- --nocapture`
- `cargo test suggestion_mode_uses_escalated_sandbox_justification_for_approval -- --nocapture`
- `cargo check`
