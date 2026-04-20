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

### Shell execution

- `bash` tool execution must emit live transcript updates while stdout/stderr are still being produced.
- The final `bash` tool result should keep the exit code and avoid duplicating large output that was already streamed live.

### Queued follow-up messages

- While a turn is running, follow-up user messages are not dropped.
- If a follow-up is entered during a query turn, it first waits for the next tool/result boundary.
- Once that boundary is crossed, the message is promoted into the ordinary end-of-turn queue.
- If the turn finishes before another boundary appears, the pending follow-up is promoted at turn completion.
- The bottom pane should render the two queues separately:
  - `Messages to be submitted after next tool call`
  - `Queued follow-up messages`

## Non-Goals

- This does not yet implement the full Codex "interrupt and send immediately" steer path.
- This does not yet provide a fully separate command-pane widget for bash output; the current contract only guarantees live transcript visibility.
