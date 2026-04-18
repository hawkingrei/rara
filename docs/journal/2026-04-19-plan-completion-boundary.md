# 2026-04-19 Plan Completion Boundary

## Summary

Tightened planning-mode completion so automatic planning does not stop after repository inspection without producing a structured next step.

## Changes

- Strengthened the planning-mode prompt so a planning turn must end with exactly one of:
  - `<plan>`
  - `<request_user_input>`
  - `<continue_inspection/>`
- Updated planning continuation logic so:
  - inspection evidence without a finalized plan triggers one more planning continuation round;
  - a turn that already updated the plan does not keep looping just because inspection evidence is still shallow.
- Added a regression test covering:
  - inspect-first planning turns that must continue once more to synthesize a real plan.

## Rationale

This moves RARA closer to the Claude/Codex style completion boundary:

- planning is allowed to inspect first;
- planning should not end with narration alone;
- once a real plan is produced, the turn should converge instead of continuing unnecessarily.
