# Terminal Tool Schema Prompts

## Context

RARA already had global command guidance in the instruction prompt, but the
terminal tool schemas still exposed only minimal descriptions. That left the
model without the same call-site guidance that Codex and Claude Code provide for
shell execution.

## Upstream Alignment

- Codex keeps shell workdir as an explicit tool parameter and resolves it
  against the turn cwd. It also routes patch application through a dedicated
  apply-patch path instead of encouraging shell-based patch execution.
- Claude Code puts Bash-specific discipline directly in the Bash tool
  description: keep the working directory stable, prefer dedicated tools, avoid
  shell-based file edits, avoid newline-separated command chaining, and keep
  commands sandboxed unless escalation is justified.

## Implementation Checkpoint

- Strengthened the `bash` tool description and input schema with command
  discipline for dedicated tools, `cwd`, sandbox escalation, background tasks,
  and shell-edit avoidance.
- Strengthened background task tool descriptions so models know to inspect or
  stop long-running work instead of starting duplicates.
- Strengthened `pty_start` and PTY control descriptions so PTY is reserved for
  interactive terminal sessions while ordinary commands stay on `bash`.
- Added focused schema-description tests for Bash, background tasks, and PTY.

## Validation

- `cargo test bash_tool_schema_guides_command_discipline -- --nocapture`
- `cargo test background_task_tool_descriptions_point_to_run_in_background -- --nocapture`
- `cargo test pty_tool_schema_guides_interactive_command_discipline -- --nocapture`
- `cargo check`
