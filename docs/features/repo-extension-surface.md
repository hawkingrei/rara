# Repository Extension Surface

## Goal

RARA should recognize repository-scoped customizations from multiple agent
ecosystems without immediately coupling discovery to execution.

The initial target is compatibility with:

- native RARA skills under `.agents/skills/`;
- Claude-style agent definitions under `.claude/agents/`;
- Claude-style lifecycle hooks under `.claude/hooks/`.

This document defines discovery, precedence, compatibility boundaries, and a
staged rollout order.

It does **not** require full runtime execution support in the first cut.

## Related Claude Code Capabilities

This extension surface is intended to align with the Claude Code capabilities
that are already meaningful at repository scope:

- repo-local agent definitions under `.claude/agents/`;
- repo-local lifecycle hooks under `.claude/hooks/`;
- a sub-agent model where imported agent roles are more structured than plain
  prompt snippets;
- hookable runtime phases around prompt submission, tool execution, and session
  lifecycle.

For RARA, the compatibility target is the **shape of the extension surface**,
not a byte-for-byte clone of Claude Code runtime semantics.

That means:

- RARA may normalize Claude agent files into RARA-owned imported-agent objects;
- RARA may normalize Claude hook declarations into RARA-owned hook-definition
  objects;
- actual execution still has to respect RARA thread, context, sandbox, and tool
  boundaries.

The Claude-related runtime phases that map most cleanly onto current RARA
surfaces are:

- `SessionStart`
- `UserPromptSubmit`
- `PreToolUse`
- `PostToolUse`
- `Stop`

Later follow-up phases may include:

- `SubagentStop`
- `PreCompact`

## Non-Goals

The first rollout should not:

- execute arbitrary hook files on discovery;
- treat imported Claude agents as first-class RARA child threads immediately;
- merge all extension types into one generic opaque plugin loader;
- introduce a marketplace or remote-install protocol.

## Core Principle

RARA should separate:

1. **discovery**
   - what repo-local extensions exist;
2. **normalization**
   - how discovered files are mapped into RARA-owned objects;
3. **execution**
   - when and how those normalized objects affect runtime behavior.

The first milestone should complete discovery and normalization before adding
runtime execution.

## Extension Types

### Native Skills

RARA-native reusable skills live under:

- `.agents/skills/<skill-name>/SKILL.md`

They remain the authoritative skill format for direct runtime use.

### Imported Agent Definitions

Claude-style repo-local agents live under:

- `.claude/agents/*.md`

RARA should treat these as **importable agent profiles**, not as raw prompt
files.

Each discovered agent definition should normalize into a structured object such
as:

- `ImportedAgentProfile`
  - `id`
  - `label`
  - `source_path`
  - `source_kind = "claude_agent"`
  - `prompt_body`
  - `tools_policy`
  - `description`

The first cut may leave `tools_policy` partially inferred or unknown, but the
object boundary should exist.

### Imported Hook Definitions

Claude-style lifecycle hooks live under:

- `.claude/hooks/`

RARA should treat them as **declared hook candidates**, not as executable logic
until the hook runtime contract exists.

Each discovered hook definition should normalize into a structured object such
as:

- `ImportedHookDefinition`
  - `id`
  - `source_path`
  - `declared_event`
  - `handler_kind`
  - `handler_target`
  - `parse_status`

If a hook file cannot be fully parsed, RARA should still surface it as a known
repo extension with a parse/error status instead of silently ignoring it.

## Discovery Roots

RARA should distinguish:

1. user/home-level extension roots;
2. workspace-level extension roots;
3. nested workspace roots when supported by current prompt/workspace discovery.

The initial repository-scoped roots are:

- `<workspace>/.agents/skills/`
- `<workspace>/.claude/agents/`
- `<workspace>/.claude/hooks/`

## Precedence Rules

Precedence should be explicit and source-aware.

### Skills

For skill name conflicts:

1. workspace-local `.agents/skills`
2. nested workspace skill roots, if applicable
3. home/global skills

RARA should surface overridden skills in status/debug output instead of hiding
them.

### Imported Agents

Imported Claude agents should remain in their own namespace and should not
silently override native RARA skills or built-in sub-agent kinds.

That means:

- a Claude agent named `planner` should not replace RARA's built-in planning
  mode;
- collisions should be surfaced as compatibility warnings or namespace-qualified
  entries.

### Imported Hooks

Imported hooks should never implicitly override built-in safety/runtime rules.

If a hook later becomes executable, built-in runtime invariants still win:

1. safety and policy constraints;
2. persisted thread/runtime continuity rules;
3. hook execution.

## Status / Explainability Requirements

Once discovery lands, `/status` or equivalent debug surfaces should be able to
show:

- discovered native skills;
- discovered imported Claude agents;
- discovered imported Claude hooks;
- source path;
- precedence or override status;
- parse status;
- whether runtime execution is currently supported.

This is required before adding hook execution so the extension surface stays
debuggable.

## Runtime Compatibility Plan

### Stage 1: Discovery Only

Deliver:

- discovery of `.agents/skills/`, `.claude/agents/`, `.claude/hooks/`;
- normalized metadata objects;
- source-aware status reporting;
- precedence/override visibility.

Do not yet:

- run hooks;
- spawn imported agents automatically.

### Stage 2: Imported Agent Profiles

Deliver:

- map imported Claude agents into explicit RARA agent profiles;
- allow opt-in invocation through a RARA-owned delegation/runtime surface;
- keep parent/child thread contracts owned by `ThreadStore`, not by imported
  file formats.

Imported agent files should adapt into RARA's thread/sub-agent model rather than
define a separate execution path.

### Stage 3: Minimal Hook Runtime

Deliver a minimal hook contract with a small event set, for example:

- `SessionStart`
- `UserPromptSubmit`
- `PreToolUse`
- `PostToolUse`
- `Stop`

The first runtime cut should prefer:

- explicit command hooks;
- explicit prompt-injection hooks;
- deterministic failure/reporting rules.

RARA should avoid broad "run any repo script at any time" semantics.

### Stage 4: Richer Hook / Agent Interop

Possible later work:

- imported agent-specific hook policies;
- MCP- or HTTP-backed hooks;
- repo-local approval or formatting workflows;
- richer compatibility with external agent ecosystems.

This stage should only happen after the core thread/runtime lifecycle remains
stable under Stage 2 and Stage 3.

## Thread and Context Constraints

Imported extensions must not bypass RARA-owned runtime boundaries.

That means:

- imported Claude agents still run through RARA sub-agent/thread contracts;
- imported hooks still operate through RARA lifecycle events;
- context injection still flows through `ContextAssembler` and
  `MemorySelection`;
- thread persistence still flows through `ThreadStore` / `ThreadRecorder`.

Compatibility should adapt external formats into RARA-owned objects instead of
letting external directory conventions define runtime semantics directly.

## Open Questions

- How much of Claude agent frontmatter or metadata should be preserved vs
  normalized away?
- Should imported hooks live in a separate "compatibility" status section until
  executable support lands?
- Should nested workspace discovery allow multiple `.claude/` roots, or only
  the active workspace root?
- How should imported agent names appear in `/help` or command surfaces without
  colliding with built-ins?

## Immediate Follow-Up

The next implementation slice should stay small:

1. add source-aware discovery objects for repo-local native skills, Claude
   agents, and Claude hooks;
2. expose them in `/status`;
3. record precedence and parse status;
4. defer runtime execution to a later milestone.
