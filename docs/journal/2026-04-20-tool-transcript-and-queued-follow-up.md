# 2026-04-20 · Tool transcript and queued follow-up boundary

## Summary

This checkpoint moved RARA's TUI closer to the Codex-style tool transcript model.

## What changed

### Live bash transcript

- Extended the tool runtime so a tool can emit progress events while it is still running.
- `bash` now streams stdout/stderr chunks into the active transcript instead of only surfacing a final summary after the command exits.
- The final `bash` result keeps the exit code and marks that live output was already shown above.

### Queued follow-up boundary

- Queued follow-up messages are now split into two stages:
  - pending until the next tool/result boundary;
  - queued for end-of-turn submission.
- Query turns start with boundary counter `0`.
- Each tool/result boundary advances the counter and promotes any follow-ups that were waiting for that boundary.
- If a turn completes before the next boundary appears, pending follow-ups are promoted at task completion.

### Codex-compatible skill roots

- `rara-skills` now looks in Codex-compatible skill roots:
  - `~/.rara/skills`
  - `~/.agents/skills`
  - `~/.codex/skills`
  - repo-scoped `.agents/skills` from project root down to cwd
  - local `./.rara/skills`
- Existing legacy `*.md` skills and `skill-name/SKILL.md` directories remain supported.

## Validation

- `cargo test -p rara-skills -- --nocapture`
- `cargo test tui::state::tests -- --nocapture`
- `cargo test tui::render::bottom_pane::tests -- --nocapture`
- `cargo test tui::runtime::events::tests -- --nocapture`
- `cargo test tools::bash::tests -- --nocapture`
- `cargo test tui::tests::busy_submit_queues_follow_up_message -- --nocapture`
- `cargo check`

## Follow-up

- Add the explicit "interrupt and send immediately" steer path so queued follow-ups fully match Codex pending input behavior.
- Keep improving the live bash transcript with clearer command start/finish framing and better long-output folding.
