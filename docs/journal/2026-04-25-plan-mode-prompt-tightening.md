# 2026-04-25 Plan Mode Prompt Tightening

## Why

RARA planning mode already enforced a structured end-of-turn contract, but the
prompt still left too much room for verbose "I will inspect ..." narration and
plain-prose plan-approval asks.

That was weaker than the interaction patterns we want to mirror from Codex and
Claude Code:

- short, grounded planning progress;
- tool transcript carrying inspection detail instead of prose narration;
- explicit structured plan approval instead of ordinary-language approval asks.

## What Changed

- tightened the plan-mode prompt in `crates/instructions/src/prompt.rs`;
- reorganized the prompt into explicit sections:
  - read-only execution boundary;
  - short planning-progress style;
  - structured planning outcomes;
  - structured plan-approval contract;
- added an explicit rule that plan approval must not be requested in ordinary
  prose;
- kept the existing runtime contract unchanged:
  - `<plan>`
  - `<request_user_input>`
  - `<continue_inspection/>`

## Validation

- added a focused prompt test covering:
  - short progress wording;
  - no prose plan approval;
  - exact structured end-of-turn outcomes.
