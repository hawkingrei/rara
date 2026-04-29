# Classifier and Routing

This document defines where RARA should use classifier-style side decisions and
where it should rely on the main agent model plus tool descriptions.

## Goals

- Keep safety-sensitive approval decisions auditable and separate from normal
  agent reasoning.
- Make long-running agent state visible without requiring the main agent to
  summarize itself perfectly.
- Keep plan-mode entry aligned with Codex and Claude Code: the main agent should
  decide to enter plan mode through an explicit tool, not through a hidden
  keyword or intent classifier.

## Reference Patterns

Claude Code uses several specialized classifier paths rather than one global
task classifier:

- Auto-mode permission classification: a side query classifies whether a tool
  action should be blocked or allowed. Its input is a compact transcript built
  from user text and assistant tool-use blocks, while ordinary assistant text is
  excluded to avoid model-authored content influencing the security decision.
- Bash allow classification: a speculative classifier can run while permission
  UI is pending and auto-approve only high-confidence allow matches.
- Background-agent state classification: a classifier reads the tail of a
  background-agent transcript and returns structured state such as `working`,
  `blocked`, `done`, or `failed`.
- Plan-mode entry: `EnterPlanMode` is a normal tool with strong tool
  instructions. The main model decides when to call it for ambiguous or
  non-trivial implementation work.

RARA should mirror the separation of concerns, not collapse these into one
generic natural-language router.

## Classifier Surfaces

### Auto Permission Classifier

Use a side classifier for tool permission decisions when static rules are
insufficient.

The classifier input must be a structured, compact projection:

- include user messages that express intent;
- include assistant tool calls as actions being evaluated;
- exclude assistant prose by default;
- include applicable repository guidance only as user-provided configuration;
- include the current candidate action explicitly.

The classifier output must be structured:

- `should_block: bool`;
- `reason: string`;
- optional `matched_rule` or `matched_description`;
- optional confidence when the policy supports speculative allow.

Static deny rules must still win over classifier allow results. Classifier allow
must not bypass sandbox policy unless a separate sandbox-bypass rule permits it
and the transcript/status surface reports that bypass.

### Background Task State Classifier

Use a separate state classifier for long-running background agents and detached
tasks.

The input should be the recent assistant-message and tool-status tail for one
task. The output should be:

- `state`: `working`, `blocked`, `done`, or `failed`;
- `tempo`: `active`, `idle`, or `blocked`;
- `detail`: one concise status line;
- `needs`: present only when user action can unblock the task;
- `result`: present only when a durable deliverable exists.

This classifier is observational. It must not approve tools or mutate task
state directly. Runtime code should record the classifier result as derived
status and keep the raw task transcript as the source of truth.

### Plan-Mode Entry

RARA should not add a hidden keyword classifier for plan mode.

Plan mode should be entered through an explicit `EnterPlanMode`-style tool
description and system reminder:

- the main agent can call the tool proactively for genuinely ambiguous,
  architectural, or multi-file implementation work;
- simple or clearly specified implementation work should proceed directly;
- pure analysis/report requests should produce conclusions without entering an
  approval-gated implementation plan;
- when an agent-initiated planning phase is part of an autonomous workflow, the
  plan should be generated and reviewed by the agent without requiring an extra
  user approval unless the workflow explicitly asks for one.

## Non-Goals

- Do not implement keyword matching to detect planning, safety, or task state.
- Do not use one classifier prompt for unrelated concerns.
- Do not let classifier decisions replace deterministic deny rules, sandbox
  boundaries, or explicit user approvals for high-risk actions.

## Implementation Notes

The first implementation should be additive:

1. Define typed request and response structs for each classifier surface.
2. Add transcript projection helpers that are testable without an LLM backend.
3. Add provider-agnostic side-query plumbing with timeout and error reporting.
4. Start with observational status classification for background tasks before
   using classifiers in permission decisions.
5. Surface classifier decisions in the transcript or status view so users can
   debug why an action was allowed, blocked, or marked as waiting.

## Validation

- Unit-test transcript projection so assistant prose cannot affect permission
  classification input.
- Unit-test static deny precedence over classifier allow.
- Unit-test background-state transitions (state: working/blocked/done/failed; tempo: active/idle/blocked) and verify that detail, needs, and result fields are correctly derived.
- Add TUI/status snapshot coverage for visible classifier decisions before
  enabling auto-approval behavior by default.
