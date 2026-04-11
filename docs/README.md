# Documentation Guide

This folder contains engineering-facing documentation for RARA contributors.

## Structure

- `docs/features/`: stable feature and architecture specs
- `docs/journal/`: dated implementation checkpoints
- `docs/todo.md`: active backlog only

## When You Change Code

Use this checklist for every non-trivial change:

1. Update the canonical feature spec in `docs/features/` when contracts or
   behavior changed.
2. Add or append a dated journal note in `docs/journal/`.
3. Add a `docs/todo.md` item only when follow-up work remains.

## Journal Convention

- Filename: `YYYY-MM-DD-topic.md`
- Keep notes concise and operational:
  - Summary
  - Background
  - Scope
  - Key decisions
  - Validation
  - Follow-ups

## Compaction Rules

- `docs/features/` holds stable contracts, not a chronological changelog.
- `docs/journal/` holds dated implementation records.
- When a feature evolves, update the canonical feature doc and add a journal note.
- Remove completed TODO items after the evidence lives in a feature spec, journal, or merged PR.
