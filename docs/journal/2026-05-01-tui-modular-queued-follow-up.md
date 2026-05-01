# 2026-05-01 · TUI modular queued follow-up and compact shell results

## Summary

This checkpoint records two UI lessons from the shell approval and queued
follow-up path:

- final compact tool output should be fixed at the source contract, not cleaned
  up later by renderer-specific string patches;
- queued follow-up status should live in the active transcript, while the
  composer remains only the input and hint surface.

## Source-level compact results

The previous persisted shell result path could render as:

```text
bash: bash finished.
full result stored on disk
```

That was a source contract problem. The `bash` compactor emitted a tool-specific
sentence, then the TUI renderer prepended the tool name again. The oversized
result marker also exposed an internal key/value field that the UI had to
translate into human copy.

The corrected contract is composable:

- `bash` compact output starts with process status only:
  - `finished with exit code 0`
  - `failed with exit code <code>`
  - `finished with unknown exit status`
- oversized compact output uses:
  - `[tool_result truncated]`
  - `full result: <path>`
- TUI rendering can prepend `bash:` once without knowing how the shell compactor
  phrased its status.

This keeps model-facing output and transcript output aligned. It also avoids a
fragile renderer helper that has to recognize every historical bash sentence.

## Queued follow-up rendering

The earlier bottom-pane preview made queued follow-up messages compete with
approval options. In practice, a pending shell approval card could be present
while the composer area displayed queued text, making the option surface feel
hidden or blocked.

The new boundary is:

- `queued_input` builds renderer-neutral `QueuedFollowUpSection` values.
- `ActiveTurnCell` decides where queued state belongs in the active transcript.
- `QueuedFollowUpCell` renders that state as transcript status.
- `bottom_pane` renders the composer text and a short hint only.

The ordering rule is explicit: pending interaction cards are rendered before
queued follow-up status. This keeps approval options visible and places queued
input after the newest relevant active cell, matching the append-oriented
transcript behavior used elsewhere in the TUI.

## Submission routing

Plain text submitted while a pending approval is active is now queued as a
follow-up. It does not start a second model turn and does not steal focus from
the approval decision.

The exceptions are intentionally narrow:

- numeric option shortcuts still answer the pending approval;
- request-input prompts still consume plain text as the requested answer.

This gives queued messages more chances to be captured without weakening the
structured approval path.

## TUI modularity rule

Future TUI work should preserve these layers:

- state owns facts;
- small builders shape facts into display data;
- cells render semantic transcript units;
- the bottom pane owns only composer layout and hints;
- active-turn composition owns ordering and priority.

When a new UI behavior needs to be visible during a turn, prefer adding a cell
or a display-data section. Avoid adding durable status bodies to the composer,
because the composer has to stay predictable for approval shortcuts, text input,
cursor placement, and terminal selection.

## Validation

- `cargo test tui::render::bottom_pane::tests:: -- --nocapture`
- `cargo test tool_result::tests:: -- --nocapture`
- `cargo test tui::runtime::events::tests::formats_ -- --nocapture`
- `cargo test plain_submit_queues_while_shell_approval_is_pending -- --nocapture`
- `cargo test active_turn_cell_renders_queued_follow_up_without_hiding_shell_approval -- --nocapture`
