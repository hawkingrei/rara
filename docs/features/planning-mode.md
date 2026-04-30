# Planning Mode

RARA planning mode is a read-only collaboration mode for non-trivial tasks.

## Goals

- let the agent inspect repository context before editing;
- converge toward a concrete implementation plan or a structured clarification;
- preserve planning mode until an explicit runtime action exits it.

## Contract

- planning mode is read-only;
- the agent enters planning mode by calling `enter_plan_mode`; the TUI must not infer planning mode from prompt keywords;
- planning mode persists across turns until the runtime explicitly switches back to execute mode, such as after plan approval;
- decision-complete implementation plans are persisted to `.rara/sessions/<session_id>/plan.md`;
- `exit_plan_mode` submits the persisted proposed plan to the approval flow; it does not directly grant editing permission;
- user imperative wording like "continue" or "implement" does not exit planning mode by itself;
- planning turns may use read-only repository tools, read-only shell commands, and delegated read-only sub-agents;
- planning turns may end with a normal final answer for research, review, or planning-advice tasks;
- when the agent entered planning mode automatically from execute mode without calling `exit_plan_mode`, a decision-complete implementation plan is auto-approved by the runtime and resumes execution without a human approval card;
- manual `/plan` sessions still require explicit human approval before execution resumes from `<proposed_plan>`;
- structured planning artifacts are reserved for explicit runtime actions:
  - `<proposed_plan>` when the implementation plan is ready to persist as the plan artifact;
  - `exit_plan_mode` when the persisted plan is ready for approval;
  - `<request_user_input>` when a key decision still needs user input;
  - `<continue_inspection/>` when more repository inspection is still required.
- a model message with no tool call and no `<continue_inspection/>` is a final answer for the current turn, not an implicit request for runtime continuation;
- planning-mode prose should stay in inspected findings and concise progress updates;
- planning-mode progress updates should stay short and grounded in inspected code instead of narrating each next file-by-file action;
- planning mode must not describe file edits, patches, or implementation steps as if they are already happening.
- plan approval must not be requested in ordinary prose; the model must use `<proposed_plan>` followed by `exit_plan_mode` when the plan is ready, or `<request_user_input>` when a key decision still blocks it.

Plain narration or status updates are valid only when they are the final answer to a research, review, or planning-advice task. They must not be treated as approval requests.

## Sub-Agents

- `explore_agent` is a read-only exploration sidecar for repository inspection;
- `plan_agent` is a read-only planning sidecar for delegated sub-planning;
- sub-agents must not recursively create sub-agents;
- general worker sub-agents do not receive tool access until RARA has an explicit
  nested-agent depth and observability contract;
- planning sub-agents follow the same completion contract:
  - `<proposed_plan>`
  - `<request_user_input>`
  - `<continue_inspection/>`

## Runtime Continuation

When planning mode needs to continue, runtime records a structured continuation message with one of these phases:

- `plan_continuation_required`
- `plan_approved`

`plan_continuation_required` means the agent explicitly requested another read-only inspection pass before answering or requesting implementation approval.

Execute-mode repository inspection follows the same structured continuation boundary: prior inspection evidence is not enough to keep a turn open after a no-tool model response. The model must either call another inspection tool or emit `<continue_inspection/>`.

## TUI Expectations

- planning mode should render as a dedicated planning workflow, not as ordinary execution with extra chatter;
- non-structured planning narration should be folded into planning/exploration sidecars instead of rendering as a separate responding block;
- structured plans should render as an `Updated Plan` checklist object with a separate note/explanation and stable step status markers;
- runtime heartbeat text such as `waiting for model response` or elapsed-time notices should stay in the activity/status surfaces instead of being repeated inside planning, exploration, or updated-plan transcript cells;
- when execution resumes from an approved plan, only the currently active plan step may be auto-completed at successful turn end; pending later steps must not be marked complete optimistically;
- pending plan approval and planning questions should appear as interaction cards;
- status panels, overlays, and transcript cells should reuse the same plan formatting instead of diverging into separate text-only summaries;
- approval or answers should resume the same workflow rather than starting a brand-new free-form chat turn.
