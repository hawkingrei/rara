# Plan Mode Exit Approval

RARA now mirrors Claude-style plan exit semantics for implementation plans.

- `exit_plan_mode` is registered as the explicit tool for submitting a completed proposed plan to the TUI approval flow.
- Parsed `<proposed_plan>` content is persisted to `.rara/sessions/<session_id>/plan.md`.
- `exit_plan_mode` does not directly grant edit permission. It pauses the turn until the user approves or continues planning.
- When the user approves, the agent receives a tool result containing the approved plan and the plan file path, then resumes in execute mode.
- Existing agent-driven planning without `exit_plan_mode` keeps the previous auto-resume compatibility path.

Validation:

- `cargo fmt --check`
- `cargo test exit_plan_mode -- --nocapture`
- `cargo test plan_turn_completion_keeps_plan_mode_after_plain_answer -- --nocapture`
- `cargo test agent_driven_plan_mode_auto_approves_and_resumes_execution -- --nocapture`
- `cargo check`
