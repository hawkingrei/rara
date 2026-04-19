# 2026-04-19 Context Compression

## Summary

Started aligning RARA compaction with Claude-style structured context compression.

This phase tightens the compression contract so compacted history is schema-oriented rather than a
generic free-form summary blob, and it carries recent file paths plus basic compact observability
into the runtime status view.

## Changes

- Added a stable feature spec for context compression.
- Upgraded the default compact prompt to require a fixed sectioned markdown schema.
- Changed the stored compaction marker in history to a structured-summary label.
- Added a machine-readable compact boundary record ahead of the stored summary.
- Carried recent inspected file paths forward after compaction.
- Carried a small set of recent `read_file` excerpts forward with line-range metadata when available.
- Exposed recent compact carry-over in `/status`.
- Switched the common history append/replace paths to keep `estimated_history_tokens` incrementally
  updated instead of waiting for the next full compact pass.
- Mirrored compact-boundary metadata into persisted session state so restore flows can recover the
  latest compaction boundary and compact counters.

## Why

This gives long-running coding sessions a more stable compression boundary and makes future
follow-up work easier:

- compact observability;
- stronger plan / approval preservation across long sessions.

## Validation

- `cargo check`
- compact prompt unit tests
- compact history unit tests

## Follow-Up

- Add richer recent-file carry-over such as snippets or line references.
- Persist richer compact carry-over artifacts beyond boundary metadata, especially for recent file
  excerpts.
- Reduce repeated full-history token estimation during long sessions.
