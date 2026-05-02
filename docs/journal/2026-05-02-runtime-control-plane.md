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

Runtime provenance uses explicit `trust` and `authorship` enums rather than
independent boolean flags, so callers cannot represent contradictory source
states. Shell approval control keeps a contract-local enum but has explicit
round-trip conversions to the runtime `BashApprovalDecision`.

This checkpoint intentionally does not route ACP or Wire through the control
plane yet. The next slice should add subscriber plumbing and then move ACP
prompt/cancel/session handling onto these request types.

## Hook Context Timing Checkpoint

Clarified the context-management role of hooks in
`docs/features/context-architecture.md` and cross-linked the same boundary from
`docs/features/runtime-control-plane.md`.

The contract is:

- hook output is a structured context candidate, not final prompt text;
- final injection happens only before model request assembly completes;
- late lifecycle output becomes next-turn context or compaction metadata;
- `SessionStart`, `UserPromptSubmit`, `PostToolUse`, `PreCompact`, and `Stop`
  each have explicit timing responsibilities;
- `/context` must explain selected, available, dropped, deferred, or ignored
  hook-created context.

This keeps hook behavior compatible with ACP/Wire control, stable prompt-source
ordering, and future context-cache invalidation.

## Large Write And Composer Status Checkpoint

Clarified large-file write guidance in `docs/features/tool-transcript.md`,
`docs/features/prompt-runtime.md`, the default prompt, and the `write_file` tool
description.

The source comparison was:

- Claude Write says the tool overwrites local files and should be reserved for
  creates or complete rewrites, while Edit is preferred for existing-file
  modifications because it sends only the diff;
- Claude Bash/PowerShell alternatives route file writes through Write rather
  than `echo >`, `cat <<EOF`, `Set-Content`, or `Out-File`, and route edits
  through Edit rather than `sed` or `awk`;
- Codex uses `apply_patch` as the reviewable edit surface and supports heredoc
  parsing for `apply_patch`, but that is a patch transport rather than a general
  shell-file-write pattern.

RARA therefore treats a failed, truncated, or apparently non-persistent
`write_file` result as an edit-tool failure to diagnose with direct tools,
smaller patches, or an explicit user-visible failure report. It must not
silently degrade into PTY, shell redirection, or heredoc file writes.

Also documented and fixed the TUI boundary that long-lived query progress such
as `Working` / `Sending prompt to model` should not remain pinned in the bottom
composer area. Runtime progress belongs in transcript/status events so the
composer stays an input surface.
