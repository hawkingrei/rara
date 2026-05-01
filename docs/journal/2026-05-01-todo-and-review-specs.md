# Todo And Review Specs

## Summary

Added stable specifications for two upcoming runtime surfaces:

- a session-scoped Todo runtime modeled after Claude Code's `TodoWrite` working-set concept;
- a `/review` slash command that starts a read-only code-review turn through the normal agent loop.

## Background

RARA already separates implementation plans from execution through `plan.md`, but todo progress still
needs a dedicated artifact and tool contract. RARA also has review-oriented prompt standards, but no
direct slash command for review workflows.

## Scope

- Defined `.rara/sessions/<session_id>/todo.json` as the future session-scoped todo artifact.
- Defined `todo_write` as the initial complete-replacement todo tool.
- Defined `/review` command shapes and the generated review prompt contract.
- Kept both specs implementation-ready without changing runtime code in this checkpoint.

## Key Decisions

- Todo state is mutable execution state and must not be stored in `plan.md`.
- `/review` is read-only by default and should not enter plan mode automatically.
- The `/review` command handler should only generate a prompt and start a normal query; the agent
  remains responsible for inspecting git, GitHub, diffs, tests, and comments through existing tools.

## Validation

- Documentation-only checkpoint; no runtime tests were run.

## Follow-ups

- Implement `todo_write`, todo persistence, context/status surfacing, and TUI update cards.
- Implement `/review` parser, help registration, prompt generation, and command runtime tests.
