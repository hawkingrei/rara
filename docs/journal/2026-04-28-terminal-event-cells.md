# 2026-04-28 · Terminal Event Cells

## Summary

RARA now has a Codex-style typed terminal event layer for PTY sessions and
background bash tasks, plus a dedicated TUI history cell for terminal command
results.

## What Changed

- Added `TerminalEvent` with typed begin, output-delta, end, list, and stop
  variants for PTY sessions and background tasks.
- Routed terminal tool uses, progress, and results through typed TUI events
  before rendering them into the existing transcript representation.
- Added `TerminalCell` so committed and active history can render terminal
  activity as a command/result block instead of a generic tool-result row.
- Added an internal `Terminal Event` transcript role that stores serialized
  terminal events for the TUI renderer while preserving the existing transcript
  storage shape.
- Moved terminal output tailing and ANSI/control-sequence cleanup into the
  terminal event layer so runtime event formatting and TUI rendering share the
  same behavior.

## Why

Codex keeps command execution as structured lifecycle events and lets the TUI
render those events through dedicated history cells. RARA still persists plain
transcript entries today, but introducing a typed terminal event boundary gives
future app-server and third-party consumers a stable event shape without
requiring them to parse UI text.

## Current Boundary

- The typed terminal event exists inside the TUI runtime event path.
- Persistence remains transcript-compatible, but terminal entries now keep the
  typed event payload in an internal transcript role.
- `TerminalCell` renders typed terminal events first and keeps text-summary
  parsing only as a restore/backward-compatibility fallback. A later
  thread-domain migration can persist typed terminal rollout items directly.

## Validation

- `cargo fmt --check`
- `cargo test terminal_event -- --nocapture`
- `cargo test terminal_result_as_terminal_cell -- --nocapture`
- `cargo test tui::runtime::events::tests -- --nocapture`
- `cargo check`
