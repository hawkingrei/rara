# 2026-04-19 Planning Mode Contract

## Why

Planning mode was still too easy to treat like a read-only execute pass:

- the model could end with ordinary planning narration;
- runtime could accept that turn as complete;
- users could be left with a planning transcript but no concrete plan, question, or explicit continuation.

That behavior was weaker than both Claude Code plan mode and Codex collaboration-mode expectations.

## What Changed

- added a dedicated planning-mode feature spec in `docs/features/planning-mode.md`;
- tightened the planning-mode completion contract:
  - `<plan>`
  - `<request_user_input>`
  - `<continue_inspection/>`
- added `plan_structured_outcome_required` as a runtime continuation phase;
- if a planning turn ends with narration alone, runtime now continues the same planning turn instead of accepting it as complete;
- aligned `plan_agent` prompt instructions with the same structured end-of-turn contract.

## Validation

- focused agent tests cover:
  - planning continuation after inspection evidence;
  - narration-only planning turns forcing a structured follow-up;
  - delegated planning evidence still leading to plan synthesis.
