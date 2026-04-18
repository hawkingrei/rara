# 2026-04-18 Planning And Explore Sub-Agents

This checkpoint continues the planning lifecycle work toward a Claude-style workflow and a Codex-style transcript surface.

## What Changed

- Added minimal read-only sub-agent tools:
  - `explore_agent`
  - `plan_agent`
- Both sub-agents use a restricted tool manager with only repository inspection tools:
  - `read_file`
  - `list_files`
  - `glob`
  - `grep`
- Shared prompt runtime config is now passed into sub-agent tool construction so delegated work inherits the same instruction sources and append prompts.
- Planning prompt guidance now explicitly prefers `explore_agent` and `plan_agent` for delegated read-only work.
- Removed `/search` as a local TUI slash command. Search remains available as a model tool instead of a local control-surface command.
- TUI active-turn rendering now treats delegated planning differently from generic running work:
  - `explore_agent` contributes to `Exploring`
  - `plan_agent` contributes to `Planning`

## Why

The previous planning implementation was still too prompt-driven and too close to the main agent's ordinary execution loop. This made planning chatter noisy and kept delegated repo inspection at the generic tool-call level.

These changes move RARA closer to the intended split:

- Claude-style workflow for planning and delegated exploration
- Codex-style transcript and active-cell rendering for current-turn state

## Validation

- `cargo check`
- `cargo test tools::agent::tests -- --nocapture`
- `cargo test tui::command::tests -- --nocapture`
- `cargo test tui::render::cells::tests -- --nocapture`
- `cargo test agent::tests -- --nocapture`

## Follow-Up

- Continue aligning delegated agent rendering so committed history can distinguish delegated planning/exploration results from generic tool activity.
- Decide whether `spawn_agent` should remain a minimal general sub-agent or be upgraded to a more structured parallel/delegation runtime.
