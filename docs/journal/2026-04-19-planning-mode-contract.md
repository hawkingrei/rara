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

## Follow-up Alignment

- tightened the planning prompt so plan-mode turns do not claim `apply_patch`, file writes, or implementation as if it is already happening;
- folded non-structured planning narration into planning/exploration sidecars in the TUI instead of rendering it as a standalone responding block;
- added focused tests for planning-sidecar rendering and chatter filtering.
- moved plan rendering onto a dedicated `Updated Plan` formatter so transcript cells, status panels, and overlays now show the same checklist-style plan object instead of separate `steps:/note:` text blocks;
- reduced bottom-pane noise so planning and pending-interaction states behave more like Codex's single-status source instead of duplicating transcript information.
- stopped injecting runtime heartbeat text into planning/exploration/running transcript sidecars once those sections already exist; heartbeat now stays in the activity/status area instead of showing up as fake plan content.
- stopped rehydrating stale completed plan/question/approval interactions from snapshot state into each new active turn; only explicit transcript completion entries now render as completion cards.
- tightened execute-mode plan progression so successful turn completion only closes the active in-progress step rather than pessimistically marking every remaining pending step as completed.
