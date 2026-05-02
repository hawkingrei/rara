# AgentHub Team Mode

## Problem

RARA already has delegated planning and exploration tools, but it does not have an ACP-backed
AgentHub worker runtime that can cheaply reject irrelevant work before handing every turn to a
larger and more expensive model.

For multi-worker or role-specialized AgentHub sessions, this means every incoming turn currently
pays the full worker-model cost even when the request is clearly meant for a different worker.

This spec is intentionally written for the future `hawkingrei/agenthub` integration path and is
intentionally distinct from any future local-only team mode:

- RARA acts as the worker runtime;
- AgentHub provides the higher-level leader / team orchestration;
- RARA AgentHub team mode decides whether the local worker should spend its expensive model budget
  on the current request.
- The integration boundary is ACP, not a RARA-specific mailbox or custom RPC layer.

## Scope

- An ACP-oriented `AgentHub team mode` for worker sessions.
- A lightweight router model that classifies whether the current request is relevant to the worker.
- A worker-model handoff when the router says the request is relevant.

## Non-Goals

- Full multi-worker orchestration.
- A custom leader / worker mailbox transport outside ACP.
- Automatic worker spawning or balancing.
- TUI-specific visualization of router decisions.
- Full AgentHub protocol integration in the first phase.

## Architecture

### 0) ACP Boundary First

- AgentHub should talk to RARA through ACP session and prompt requests.
- AgentHub team mode is therefore a worker-runtime concern inside ACP request handling, not a
  parallel CLI surface with a separate protocol.
- ACP should be an adapter over RARA's runtime control plane. AgentHub team mode
  must not create a separate skills, memory, prompt, hook, approval, or output
  path.
- The first implementation target is:
  - ACP prompt arrives;
  - team router evaluates worker relevance;
  - relevant prompts go to the expensive worker backend;
  - irrelevant prompts return a structured worker decline through ACP.

### 1) AgentHub Team Runtime Wrapper

- AgentHub team mode wraps the normal worker backend with a routing backend.
- The routing backend runs before the expensive worker backend on:
  - `ask`
  - `ask_streaming`
- If the router says the request is relevant, the worker backend handles the turn normally.
- If the router says the request is irrelevant, the wrapper returns a short assistant response
  without calling the expensive worker backend.

### 2) Worker Identity

- Team mode requires a worker role string.
- The worker role must be available from ACP session/runtime configuration.
- The router prompt evaluates the latest user request against that role.
- The first phase assumes a single worker role per process.

### 3) Router Backend Selection

- AgentHub team mode is enabled by CLI/runtime configuration and must also be representable in ACP
  session initialization.
- The router backend uses the same provider and credentials as the worker backend where possible.
- The router model may be provided explicitly by CLI.
- If not provided, RARA derives a provider-specific small-model default when one is known.
- Providers without a safe default must require an explicit router model.

## Contracts

### 1) Routing Contract

The router must return strict JSON:

```json
{
  "decision": "handle" | "skip",
  "reason": "short explanation"
}
```

- `handle` means the worker backend should continue.
- `skip` means the worker should decline the turn without calling the expensive worker backend.

### 2) Failure Mode

- If the router output is malformed or unusable, team mode must fail open and let the worker backend
  handle the request.
- This avoids losing valid work because of a cheap-model routing mistake.

### 3) ACP Response Semantics

- AgentHub team-mode routing decisions must be reflected in ACP prompt handling rather than hidden
  in local CLI-only state.
- `skip` should produce a structured worker decline/deferral response suitable for AgentHub to
  re-route or ignore.
- `handle` should continue the normal ACP-backed worker turn.

## Validation Matrix

- `cargo check`
- focused routing tests:
  - router says `skip` -> worker backend is not called
  - router says `handle` -> worker backend is called
  - malformed router output -> worker backend still handles the request
- CLI/backend wiring tests for router model selection
- ACP-level tests:
  - ACP prompt path respects team routing
  - irrelevant ACP worker prompt returns a structured decline
  - relevant ACP worker prompt reaches the worker backend

## Follow-Up

- Define and implement the runtime control-plane boundary that ACP uses for
  session, prompt, cancel, output, skills, memory, prompt-source, and hook
  interactions.
- Add TUI/router observability.
- Route sub-agent or worker approvals through the same team runtime boundary.
- Complete the ACP runtime path so AgentHub can actually use RARA as a worker runtime.
- Integrate the ACP-backed AgentHub team runtime with AgentHub leader / worker orchestration.
