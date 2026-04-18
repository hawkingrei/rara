# 2026-04-18 Completion Lifecycle

## Summary

RARA's TUI could treat a successful query as "finished" even when the model had only produced an intermediate repository-inspection update. That caused turns to stop after exploratory text and routine runtime notices like `prompt finished` to leak into the transcript. The viewport logic could also shrink back to a compact composer-only layout after history existed, which made older bottom-pane frames show up inside scrollback on the next expansion.

## Changes

- Added a structured `<continue_inspection/>` control tag for execute/planning turns that still need more repository inspection before producing a final answer or final plan.
- Updated the default prompt runtime so the model uses `<continue_inspection/>` instead of prose-only "I will inspect ..." signals.
- Stripped the control tag from assistant transcript content before saving/rendering it.
- Switched execute/planning continuation decisions to use the structured continuation signal rather than natural-language heuristics.
- Filtered routine runtime/system notices out of transcript rendering so only failure-like system messages remain visible in the conversation.
- Kept a larger viewport once transcript history exists, preventing old composer/footer frames from being inserted into scrollback between turns.

## Validation

- `cargo check`
- `cargo test agent::tests -- --nocapture`
- `cargo test tui::render::tests -- --nocapture`
- `cargo test tui::render::cells::tests -- --nocapture`
