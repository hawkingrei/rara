# Non-Interactive Bash Cancellation

## Context

A bare `git commit` can start the user's editor and produce fullscreen terminal
control sequences inside the non-PTY `bash` tool. That exposes two separate
problems:

- the model needs call-site guidance to avoid interactive shell flows;
- the runtime must be able to interrupt a foreground tool, not only provider
  streaming.

Codex and Claude Code both treat shell execution as cancellable turn work. Codex
has explicit interrupt tests for long-running tools and shell commands, while
Claude Code threads an abort controller through query/tool execution.

## Implementation Checkpoint

- Strengthened the `bash` tool description and command schema with
  non-interactive command discipline.
- Added explicit guidance to use `git commit -m` or `git commit -F` instead of
  bare `git commit`.
- Added a shared `ToolCallContext` that can carry the active turn cancellation
  token into tool execution.
- Wired the context into normal tool calls and approved pending bash execution.
- Made foreground `bash` poll the cancellation token, emit a cancellation
  diagnostic, kill the child process group, and return a local
  `cancelled by user` error.

## Validation

- `cargo test bash_tool_schema_guides_command_discipline -- --nocapture`
- `cargo test foreground_bash_can_be_cancelled -- --nocapture`
- `cargo check`
