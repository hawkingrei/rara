# Codex-Style Paste Input

RARA now enables terminal bracketed paste mode for the TUI session and disables it during teardown. This mirrors Codex's primary input contract where pasted text is delivered through a dedicated paste event instead of being replayed as ordinary key presses.

The paste handler normalizes CRLF and CR line endings to LF before inserting text into the active composer. This keeps copied multi-line text as one composer edit and avoids interpreting embedded newlines as submit key events.

Focused tests cover the crossterm paste event path and newline normalization.

One Codex fallback remains open: Codex also has a `PasteBurst` state machine for terminals that do not emit bracketed paste events and instead deliver paste as a fast stream of key events. RARA should add that separately because it touches key dispatch, tick flushing, and Enter handling.
