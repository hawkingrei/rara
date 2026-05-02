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

## Implementation Checkpoint

Added `src/runtime_control.rs` with the first adapter-neutral runtime control
contract:

- `RuntimeControlRequest` covers session, input, output subscription, prompt
  source, skill source, memory, hook, and approval request families;
- `RuntimeEvent` covers the event families named by the spec, including
  session status, assistant, tool, context, warning, and error events;
- `RuntimeProvenance` records controller, adapter, session, source, trust, and
  authorship metadata;
- `agent_event_to_runtime_event()` maps existing `AgentEvent` output into the
  shared event shape so protocol subscribers can later reuse the same stream.

The serialized contract uses explicit `type` fields and `snake_case` enum
names instead of Rust's default externally tagged enum representation. Event
counts use fixed-width integer fields where they may cross protocol boundaries.

Wire mode remains a transport adapter over this contract. It should own the
JSON-RPC 2.0 envelope, Wire method names, PascalCase Wire message names, and
`event` / `request` framing rather than exposing runtime-control enums directly
as Wire messages.

This checkpoint intentionally does not route ACP or Wire through the control
plane yet. The next slice should add subscriber plumbing and then move ACP
prompt/cancel/session handling onto these request types.
