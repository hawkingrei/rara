# 2026-04-30 Codex-Style Shell Escalation

RARA now accepts a Codex-style explicit sandbox bypass request on the `bash`
tool:

- `sandbox_permissions = "use_default"` keeps the existing sandbox behavior.
- `sandbox_permissions = "require_escalated"` requests approval to run outside
  the sandbox and can include a `justification` string.
- `prefix_rule` can suggest the reusable approval prefix for equivalent future
  commands.

When an escalated command is approved and executed, the bash tool uses the
direct command wrapper and reports `sandboxed = false` with the existing
unsandboxed execution warning. This keeps the bypass visible in the transcript
and separates command approval from the sandbox policy.

This implements the model-requested escalation path from Codex. The next step
for full Codex parity is a structured sandbox-denial error path that can power
`on-failure` approval and retry without relying on stderr keyword matching.

Validation:

- `cargo test escalated_sandbox_request -- --nocapture`
- `cargo test suggestion_mode_uses_escalated_sandbox_justification_for_approval -- --nocapture`
- `cargo check`
