# Codex-Style Progress Append

RARA's active TUI progress events were already stored in order, but rendering
still grouped adjacent `Exploring`, `Planning`, and `Running` events into one
summary cell. Long tasks therefore looked like one mutable block instead of a
Codex-style appended transcript.

This checkpoint changes the active and committed transcript path to preserve
event boundaries:

- active live events render one cell per event in original order;
- active live events materialize into one transcript entry per event when the
  turn commits;
- committed progress rendering no longer merges adjacent entries with the same
  role;
- focused tests cover long active exploration/planning/running sequences and
  committed adjacent progress entries.

Validation:

- `cargo test active_turn_cell -- --nocapture`
- `cargo test committed_turn_cell -- --nocapture`
- `cargo check --message-format=short`
