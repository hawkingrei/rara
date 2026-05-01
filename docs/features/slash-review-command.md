# Slash Review Command

## Problem

RARA already has general repository-review prompting rules, but the TUI has no dedicated `/review`
entrypoint. Users currently need to describe review intent in natural language every time, and the
agent must infer whether to inspect local diffs, a GitHub pull request, or both.

A first-class `/review` command should provide a predictable review workflow without adding a
separate review engine.

## Scope

- A local slash command named `/review`.
- Argument parsing for common review targets.
- A generated agent prompt that asks for a code-review style answer.
- Integration with existing agent tools, GitHub CLI usage, and read-only inspection behavior.
- TUI help, command palette, and command history support.

## Non-Goals

- Building a separate static-analysis tool.
- Automatically applying fixes during review.
- Automatically approving or merging pull requests.
- Replacing normal natural-language review requests.
- Guaranteeing that GitHub metadata exists for every local branch.

## Architecture

### 1) Command Shape

Supported forms:

```text
/review
/review .
/review pr <number-or-url>
/review <number-or-url>
/review <path-or-rev>
```

Initial target interpretation:

- no argument or `.`: review the current local working tree and branch context;
- `pr <number-or-url>`: review the specified GitHub pull request;
- bare GitHub PR URL: review that pull request;
- bare number: treat as a PR number when the current repository has a GitHub remote;
- anything else: pass through as a local review target such as a path, revision, or range.

Ambiguous input should be preserved in the generated prompt instead of rejected early. The agent can
then inspect repository state and ask for clarification only if the ambiguity blocks a useful review.

### 2) Runtime Flow

`/review` is a local command that starts an ordinary query task with a structured review prompt.
It should not open plan mode automatically and should not require plan approval, because review is
read-only by default.

The prompt should instruct the agent to:

- inspect the relevant current repository state before concluding;
- prefer `gh pr view`, `gh pr diff`, and review-thread APIs for PR targets when available;
- inspect local `git status`, `git diff`, and nearby tests for local targets;
- prioritize correctness, behavioral regressions, missing tests, security, and maintainability;
- return findings first, ordered by severity, with file/line references when available;
- say clearly when no actionable findings are found;
- avoid making code changes unless the user separately asks to fix the findings.

### 3) Target Prompt Generation

The command handler should generate a stable prompt rather than hard-coding the entire review policy
in the TUI:

```text
Review the current repository changes.

Target: <target>

Use a code-review stance. Inspect the relevant current files, diffs, tests, and PR metadata before
concluding. Findings should lead the response, ordered by severity, with concrete file/line evidence.
Do not modify files during review unless the user explicitly asks for fixes.
```

The default prompt can be refined over time, but the behavior contract is that `/review` means
"perform a read-only code review now", not "enter planning mode".

### 4) Tooling Expectations

The command itself should not call `gh` or `git` directly. It only starts the agent with a review
prompt. The agent then uses normal tools so transcript, approvals, sandbox policy, and provider
behavior stay consistent with ordinary turns.

For PR targets, the agent should verify the current PR state instead of trusting stale conversation
context. The preferred evidence path is:

1. inspect PR metadata and review decision;
2. inspect the PR diff;
3. inspect review threads when the task asks about comments;
4. inspect CI only when the review question includes readiness or CI state.

## Contracts

### Command Registration

- `/review` appears in built-in help and the command palette.
- `/review` accepts optional arguments.
- `/review` is allowed when the app is idle.
- While a task is busy, `/review` follows the existing slash-command busy behavior and is not queued
  as ordinary user text.

### Review Output

- Findings must be first when findings exist.
- Findings should include severity and evidence.
- The answer must distinguish verified facts from inferred concerns.
- If no findings are found, the agent must state that directly and mention any unverified areas.

### Safety

- `/review` must not switch the agent to execute mode from plan mode as a side effect.
- `/review` must not mutate files, resolve GitHub comments, rerun CI, or push branches by itself.
- If a follow-up asks to fix review findings, that is a separate implementation task and may enter
  plan/execute flow as usual.

## Validation Matrix

- Parser tests for `/review`, `/review pr 123`, `/review <url>`, and `/review <path>`.
- Command help tests proving `/review` appears in help and command palette results.
- Runtime command test proving `/review` starts a query task with the generated review prompt.
- Busy-state submit test proving `/review` is treated like other slash commands while a task runs.

## Open Risks

- Bare numeric arguments depend on GitHub repository context. The first implementation should avoid
  resolving that in the command parser and let the agent verify it.
- A later dedicated review command may support submodes such as `comments`, `ci`, or `base`. Those
  should be added only after the basic review entrypoint is stable.
- Provider-specific tool-call behavior may affect how reliably the agent inspects PR metadata before
  answering; the prompt should stay evidence-oriented but the runtime should not fake evidence.

## Source Journals

- [2026-05-01-todo-and-review-specs](../journal/2026-05-01-todo-and-review-specs.md)
