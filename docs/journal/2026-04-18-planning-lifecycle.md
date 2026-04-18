# 2026-04-18 Planning Lifecycle

## Summary

Aligned the TUI planning flow more closely with the Claude-style lifecycle instead of a local mode toggle or a forced one-turn plan output.

## What Changed

- `/plan` now enters read-only planning mode for the current task instead of toggling between plan and execute.
- Planning mode no longer forces every turn to emit a `<plan>` block. The agent can use planning turns for exploration, clarification, and plan refinement before finalizing a concrete plan.
- Successful planning turns now stop in an `Awaiting Approval` stage when a concrete plan is available.
- The planning turn remains visible in the current turn, and the approval card offers:
  - `1. Start implementation`
  - `2. Continue planning`
- Choosing `1` starts an execution turn with the approved plan.
- Choosing `2` starts another planning turn to refine the plan.
- Plan approval now only triggers when the final turn in the current planning query actually produced a fresh plan, which prevents exploratory planning turns from dropping into approval just because an older plan is still stored in state.
- Help/footer/runtime copy was updated to remove the old `/plan toggle`, `planning pass`, and `return to execute` wording.

## Notes

- The approval state is currently TUI-local and is not yet restored from persisted session state.
- Focused render tests cover the approval card, while broader lifecycle tests remain follow-up work.
