# Agent-Driven Planning Mode

RARA now lets the agent enter planning mode by calling the `enter_plan_mode` tool instead of relying on TUI prompt keyword heuristics.

Implementation notes:

- Added a read-only `enter_plan_mode` tool and registered it in the default tool manager.
- Switched the agent loop into `Plan` mode when the tool is called, so the next model turn receives the planning prompt and read-only tool surface.
- Removed automatic TUI planning suggestions from normal prompt submission.
- Allowed planning mode to end with a normal final answer for research, review, or planning-advice tasks.
- Kept planning mode active across turns until an explicit runtime action exits it.
- Switched the implementation-approval signal to Codex-style `<proposed_plan>` while keeping legacy `<plan>` parsing compatibility.
- Allowed read-only shell commands in planning mode while rejecting mutating shell commands without opening the normal approval flow.
- Tightened pending plan approval text handling so generic confirmations do not start implementation.

This aligns RARA with Codex-style agent-selected planning and Claude Code's distinction between entering planning mode and explicitly requesting plan approval.
