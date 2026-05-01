# Proposed Plan Close Tag Guard

## Checkpoint

- Tightened the plan-mode prompt contract so `exit_plan_mode` is only valid after the same assistant message contains both `<proposed_plan>` and the exact closing `</proposed_plan>` tag.
- Added runtime detection for an opened but unclosed `<proposed_plan>` block. The runtime now reports a specific malformed-plan error instead of the generic missing-plan error.
- Added focused regression coverage for malformed plan exit handling and the plan-mode prompt requirement.

## Validation

- `cargo test -p rara-instructions plan_mode_prompt_requires_short_progress_and_structured_approval -- --nocapture`
- `cargo test exit_plan_mode_with_unclosed_proposed_plan_reports_specific_error -- --nocapture`
- `cargo test detects_unclosed_proposed_plan_block -- --nocapture`
