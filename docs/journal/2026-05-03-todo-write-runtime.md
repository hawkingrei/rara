# Todo Write Runtime

## Context

RARA already had approved plan state through `plan.md`, but execution progress was still implicit in
transcript text. Claude Code separates those concepts with a mutable todo list. RARA now follows that
shape while keeping the artifact structured and protocol-ready.

## Implemented

- Added `todo_write` as a complete-list replacement tool.
- Validated the todo contract with normalized ids, timestamps, supported statuses, and at most one
  `in_progress` item.
- Persisted todo state at `.rara/sessions/<session_id>/todo.json` through `SessionManager` atomic
  replacement.
- Restored todo state into `Agent.todo_state` on session resume and preserved it across runtime
  rebuilds.
- Added structured `AgentEvent::TodoUpdated` and runtime-control `todo.updated` events so TUI and
  Wire/ACP subscribers consume the same update.
- Rendered `Todo Updated` as a compact transcript card while suppressing raw `todo_write` tool JSON.
- Increased queued-follow-up title contrast by using the planning phase badge color instead of a
  low-contrast secondary surface color.

## Deferred

- Add first-class `/context` and `/status` todo sections using the existing `TodoContextView`.
- Decide whether compaction summaries should include the active item only or a bounded pending-list
  projection.
- Define multi-agent todo ownership before allowing more than one `in_progress` item.

## Validation

- `cargo test todo_write -- --nocapture`

