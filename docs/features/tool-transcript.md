# Tool Transcript

## Goal

Keep tool execution visible in the TUI without collapsing long-running work into a single post-hoc summary.

The transcript should move toward Codex/Claude-style tool visibility:

- tool uses should identify what they touched;
- edit tools should summarize file-level changes;
- shell execution should surface live stdout/stderr updates while the command is still running;
- queued follow-up messages should distinguish between:
  - messages waiting for the next tool/result boundary;
  - messages already queued for end-of-turn submission.

## Current Contract

### Edit tools

- `apply_patch` tool-use rows must include the touched file paths when they can be derived from the patch.
- `apply_patch` tool-result rows must summarize:
  - files changed;
  - line delta;
  - created / updated / deleted / moved files;
  - a short change preview.
- `write_file` and `replace` must render file-aware summaries instead of generic action labels.
- `replace` is an exact-match edit tool. When file read-state tracking is
  enabled, it still requires the file to have been read at least once, but it may
  proceed after a partial read because the edit re-reads the current file and
  requires `old_string` to match exactly once. Line-number-only edits such as
  `replace_lines` must continue to require a full read first.

### Shell execution

- `bash` tool execution must emit live transcript updates while stdout/stderr are still being produced.
- `bash` tool descriptions and input schema must carry the command discipline
  that the model sees at call time:
  - prefer dedicated RARA tools for file search, reads, and edits;
  - use `cwd` instead of prepending `cd`;
  - avoid newline-separated shell chaining;
  - keep commands sandboxed unless escalation is justified by user request or
    clear sandbox failure evidence;
  - use background task controls for long-running non-interactive commands.
- The final `bash` transcript row should keep the exit code and avoid
  duplicating large output that was already streamed live; when live streaming
  was shown, the rendered row should use a compact summary or truncated preview
  rather than reprinting the full output block.
- The final foreground `bash` tool-result payload must still expose `stdout`,
  `stderr`, `aggregated_output`, `model_preview_output`, `exit_code`, and
  `duration_ms`. `aggregated_output` remains the raw combined capture.
  `model_preview_output` is the model-facing head/tail preview, with failed
  commands biased toward the tail so error diagnostics remain visible without
  requiring shell-side `2>&1` redirection.
- Oversized tool results should be persisted to disk and replaced in model
  context with a `<persisted-output>` message containing the path and a bounded
  preview. The full JSON payload remains inspectable from that path.
- A single tool-result batch should enforce an aggregate model-facing budget so
  parallel tool calls cannot combine many individually acceptable results into
  one oversized follow-up turn.
- When shell execution pauses on a human approval request, the approval card should take visual priority over older live stdout/stderr progress from the same turn.
- Approval choices should describe both the action and its scope, such as:
  - allow only the current command;
  - allow commands with the matching prefix for the current session;
  - allow shell commands for the current session;
  - deny the command.
- OpenAI-compatible chat endpoints must keep approved shell command results as
  protocol-level tool messages before the runtime continuation message. DeepSeek
  v4/pro history folding for missing reasoning metadata is only valid when
  DeepSeek thinking mode is explicitly enabled; the default DeepSeek request body
  must preserve assistant tool calls and adjacent tool results so the model can
  continue after approval.
- The default TUI terminal mode should preserve native terminal text selection.
  Mouse capture may be added later only behind an explicit opt-in, because
  terminal mouse reporting steals drag and wheel events from the host terminal.
- Edit tool results should expose a diff-like preview in the transcript instead
  of rendering only `old=` and `new=` summary lines.
- background bash tasks must be inspectable without imposing a fixed task-count limit:
  - `background_task_list` lists known background tasks;
  - `background_task_status` reads status and recent output for one task;
  - `background_task_stop` stops one task, or all running background bash tasks
    when no task id is supplied.

### PTY sessions

- `pty_start` tool descriptions and input schema must frame PTY as an
  interactive-command surface. Ordinary non-interactive commands should use
  `bash`, while PTY sessions should preserve the same `cwd` guidance as shell
  execution. Runtime sandboxing is platform-dependent: with the macOS seatbelt
  backend, PTY commands currently run unsandboxed because `sandbox-exec` does
  not preserve interactive PTY stdin reliably.
- PTY sessions must be inspectable and stoppable without imposing a fixed
  session-count limit:
  - `pty_list` lists known PTY sessions;
  - `pty_status` reads status and recent output for one session;
  - `pty_stop` stops one session, or all running PTY sessions when no session id
    is supplied.
- `pty_read`, `pty_write`, and `pty_kill` remain supported for direct session
  interaction and backward compatibility.

### Queued follow-up messages

- While a turn is running, follow-up user messages are not dropped.
- If a follow-up is entered during a query turn, it first waits for the next tool/result boundary.
- Once that boundary is crossed, the message is promoted into the ordinary end-of-turn queue.
- If the turn finishes before another boundary appears, the pending follow-up is promoted at turn completion.
- The bottom pane should render the two queues separately:
  - `Messages to be submitted after next tool call`
  - `Queued follow-up messages`

### Running query cancellation

- When no overlay is open and a model query is running, `Esc` requests
  cancellation for the current query.
- Cancellation is cooperative: provider streaming loops should check the query
  cancellation token and return a local cancellation error instead of leaving the
  TUI stuck in `streaming model output`.
- Cancellation must preserve the current agent state so the user can continue
  from the same thread after the task exits.

## Non-Goals

- This does not yet implement the full Codex "interrupt and send immediately" steer path.
- This does not yet provide a fully separate command-pane widget for bash output; the current contract only guarantees live transcript visibility.
