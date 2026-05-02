# Runtime Control Plane Spec

## Context

RARA needs to support ACP and Wire-style integrations where third-party
applications can control or observe skills, memory, prompt sources, hooks,
input, and output.

The existing roadmap already includes skills, memory selection, prompt-source
observability, hook discovery, ACP, and AgentHub team mode. Without a shared
runtime control boundary, those features risk becoming TUI-only or protocol-
specific implementations.

## Decision

Added `docs/features/runtime-control-plane.md` as the canonical architecture
spec for adapter-neutral control requests and structured runtime events.

The new control-plane-ready rule says future work touching skills, memory,
prompt sources, hooks, planning, approvals, tool output, `/context`, or
`/status` must keep core behavior in runtime/domain layers and expose structured
request/event contracts that ACP, Wire, TUI, CLI, and future appserver adapters
can reuse.

## Scope

This checkpoint is documentation-only.

It updates:

- `docs/todo.md` with a new runtime control plane rollout section;
- `docs/features/README.md` with the control-plane readiness rule;
- `docs/features/prompt-runtime.md` with protocol prompt-source constraints;
- `docs/features/repo-extension-surface.md` with protocol adapter boundaries;
- `docs/features/agenthub-team-mode.md` with ACP-as-control-plane-adapter
  constraints.

## Follow-Up

Next implementation work should start with adapter-neutral request/event types
and a structured output event bridge before attempting full ACP or Wire feature
coverage.
