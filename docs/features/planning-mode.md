# Planning Mode

RARA planning mode is a read-only collaboration mode for non-trivial tasks.

## Goals

- let the agent inspect repository context before editing;
- converge toward a concrete implementation plan or a structured clarification;
- avoid narration-only planning turns that leave the user without a next action.

## Contract

- planning mode is read-only;
- planning turns may use read-only repository tools and delegated read-only sub-agents;
- a planning turn must end with exactly one structured artifact:
  - `<plan>` when the implementation plan is ready for approval;
  - `<request_user_input>` when a key decision still needs user input;
  - `<continue_inspection/>` when more repository inspection is still required.
- planning-mode prose should stay in inspected findings and concise progress updates;
- planning-mode progress updates should stay short and grounded in inspected code instead of narrating each next file-by-file action;
- planning mode must not describe file edits, patches, or implementation steps as if they are already happening.
- plan approval must not be requested in ordinary prose; the model must use `<plan>` when the plan is ready, or `<request_user_input>` when a key decision still blocks it.

Plain narration or status updates are not valid terminal planning artifacts.
If the model ends a planning turn with narration alone, runtime must continue the same planning turn instead of treating it as a successful completion.

## Sub-Agents

- `explore_agent` is a read-only exploration sidecar for repository inspection;
- `plan_agent` is a read-only planning sidecar for delegated sub-planning;
- planning sub-agents follow the same completion contract:
  - `<plan>`
  - `<request_user_input>`
  - `<continue_inspection/>`

## Runtime Continuation

When planning mode needs to continue, runtime records a structured continuation message with one of these phases:

- `plan_continuation_required`
- `plan_structured_outcome_required`
- `plan_approved`

`plan_structured_outcome_required` means the previous planning turn ended with narration alone and must continue until it produces a valid planning artifact.

## TUI Expectations

- planning mode should render as a dedicated planning workflow, not as ordinary execution with extra chatter;
- non-structured planning narration should be folded into planning/exploration sidecars instead of rendering as a separate responding block;
- structured plans should render as an `Updated Plan` checklist object with a separate note/explanation and stable step status markers;
- runtime heartbeat text such as `waiting for model response` or elapsed-time notices should stay in the activity/status surfaces instead of being repeated inside planning, exploration, or updated-plan transcript cells;
- when execution resumes from an approved plan, only the currently active plan step may be auto-completed at successful turn end; pending later steps must not be marked complete optimistically;
- pending plan approval and planning questions should appear as interaction cards;
- status panels, overlays, and transcript cells should reuse the same plan formatting instead of diverging into separate text-only summaries;
- approval or answers should resume the same workflow rather than starting a brand-new free-form chat turn.
