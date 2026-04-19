# Context Compression

## Problem

RARA already compacts long conversations, but the current compaction output is still a generic
summary blob. That makes the result less stable than Claude Code style context compression, where
important state is preserved through a predictable structure instead of depending on free-form
summaries.

For long coding sessions, this increases the risk of losing:

- the exact user objective;
- concrete file paths already inspected or edited;
- current plan state;
- pending approvals or questions;
- unresolved risks and the immediate next action.

## Scope

- The compact prompt contract and default compression schema.
- The compacted history marker that gets written back into `Agent.history`.
- Recent-file carry-over and compact observability in `/status`.
- Limited recent-file excerpt carry-over for the most recent `read_file` results.
- Compact-boundary metadata persistence across session save/restore.
- Focused tests for compaction prompt and stored summary shape.

## Non-Goals

- Full recent-file snippet reattachment after compaction.
- Token-cache aware prompt reuse.
- Provider-specific remote compaction APIs.

## Architecture

### 1) Structured Compact Prompt

- The default compact prompt should require a stable markdown schema instead of a generic prose
  summary.
- The first phase keeps the schema simple and directly usable by the next turn.

### 2) Required Compression Sections

The default compact output should preserve, in order:

1. `User Intent`
2. `Constraints`
3. `Repository Findings`
4. `Files Touched Or Inspected`
5. `Plan State`
6. `Pending Interactions`
7. `Unresolved Risks`
8. `Next Best Action`

### 3) Stored History Shape

- After compaction, RARA should store a clearly labeled structured summary in history instead of a
  generic `"SUMMARY OF PREVIOUS CONVERSATION"` marker.
- The stored marker should make it obvious to both runtime and future debugging that this is a
  compaction artifact with a stable schema.
- RARA should also write a compact boundary record ahead of the summary so later tooling can detect
  compaction boundaries without scraping free-form summary text.
- Compact boundary metadata should also be mirrored into persisted session state so resume flows and
  status views can recover the latest compaction boundary without reparsing full history.

## Contracts

### 1) Preservation Rules

- Preserve the current objective as close to the user's wording as practical.
- Preserve concrete file paths when they were already inspected or edited.
- Preserve a small amount of recent `read_file` content so the next turn does not depend only on file
  names.
- Preserve the current plan and any pending approval or request-user-input state.
- Preserve the immediate next useful action instead of ending with a vague recap.

### 2) Failure Tolerance

- If the model returns imperfect formatting, compaction still succeeds.
- The structure is a prompt contract, not a hard parser contract in the first phase.

## Validation Matrix

- `cargo check`
- focused prompt tests for the default compact schema
- focused agent tests ensuring manual compaction stores the structured marker

## Open Risks

- Recent-file excerpt carry-over is still limited to recent `read_file` results; `grep` and
  `search` evidence are not yet restored the same way.
- Token accounting still relies on full-history re-estimation at some boundaries.
- Session restore mirrors compact boundary metadata, but it does not yet persist the full recent-file
  excerpt payload separately from history.

## Source Journals

- [2026-04-19-context-compression](../journal/2026-04-19-context-compression.md)
