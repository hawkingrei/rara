# 2026-04-28 · Background Task Controls

## Summary

RARA now makes the Claude-style sub-agent recursion boundary explicit and adds
Codex/Claude-style management tools for long-running background bash tasks and
PTY sessions.

## What Changed

- Sub-agent prompts now explicitly say that sub-agents must complete work
  directly instead of delegating to another agent.
- Sub-agent tool managers keep recursive agent tools unavailable by contract:
  read-only sub-agents receive only read-only repository inspection tools, and
  general worker sub-agents still receive no tools.
- Background bash tasks now have:
  - `background_task_list`
  - `background_task_status`
  - `background_task_stop`
- PTY sessions now have:
  - `pty_list`
  - `pty_status`
  - `pty_stop`
- Background task and PTY management intentionally adds observability and stop
  controls without adding task-count or session-count caps.
- Running TUI query tasks now carry a cooperative cancellation token. Pressing
  `Esc` with no overlay open requests cancellation for the active model query,
  and OpenAI-compatible, Codex Responses, and Ollama streaming loops check that
  token while reading provider output.

## Why

Claude Code avoids ordinary sub-agent re-delegation, which keeps sidecar work
bounded and understandable. Codex and Claude both expose ways to inspect and
stop long-running terminal work. RARA now follows that split: keep recursive
sub-agent creation disabled for now, but make terminal work observable and
stoppable so long-running tasks are manageable.

## Validation

- `cargo test tools::agent::tests -- --nocapture`
- `cargo test background_tasks_can_be_listed_and_stopped_without_count_limit -- --nocapture`
- `cargo test tools::pty::tests -- --nocapture`
- `cargo test esc_cancels_busy_query_without_overlay -- --nocapture`
- `cargo test query_cancellation_sets_running_task_token -- --nocapture`
- `cargo check`

## Notes

The existing full `tools::bash::tests` target still depends on macOS sandbox
streaming behavior in `streaming_call_reports_stdout_and_stderr_chunks`; in the
current local environment that test returned no streamed output under
`macos-seatbelt`. The new background task list/stop regression test passed
independently.
