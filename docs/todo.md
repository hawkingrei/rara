# TODO

Active backlog only. Keep this file small and current.

## Suggested Rollout Order

From higher-leverage structural work toward later UX parity work:

1. Runtime bootstrap and context contracts
2. Configuration and provider-surface cleanup
3. Workspace / skill observability and cache correctness
4. Memory / retrieval / thread persistence
5. TUI transcript parity and command-surface polish
6. Terminal-Bench evaluation readiness

## Phase 1: Architecture Closure

Goal: lock down the boundaries that are most likely to keep expanding unless they are made explicit now.

Acceptance:
- agent, TUI, and session restore share the same context/runtime assembly contract
- `/status` (or equivalent debug output) can explain which context sources were injected, where they came from, and in what order
- `src/main.rs` remains a thin startup orchestrator instead of continuing to own runtime assembly

Priority order for this phase:

1. Complete `MemorySelection` as the authoritative bounded retrieval-selection pipeline
   - The current Stage 1 skeleton exists:
     - candidate pool;
     - selected/dropped reason reporting;
     - selection budget surface;
   - The next step is to make it the real runtime path for bounded recall.
   - Rollout order:
     - 1A. promote selection logic into the primary path for thread/workspace candidates;
     - 1B. replace placeholder retrieval with real vector-backed retrieval for thread and workspace memory.

2. Deepen the `ThreadStore` / `ThreadRecorder` boundary into a real thread domain
   - The current thread boundary and lifecycle surface now exist:
     - `threads`
     - `thread`
     - `resume --last`
     - `fork`
   - The remaining work is to define:
     - authoritative thread metadata ownership;
     - rollout-item ownership;
     - lineage / fork source / latest-thread contract;
     - which legacy files remain fallback-only.

3. Continue promoting compaction into a first-class runtime lifecycle event
   - Build on the current persisted summaries, token counters, and boundary metadata.
   - Tighten ownership between compaction state and thread/runtime persistence.
   - Keep this coupled to the thread-domain work instead of treating it as a UI-only follow-up.

## Architecture / Runtime

- [x] Split `src/context/assembler.rs` into `assembler/{mod,memory_selection,view,compaction}.rs` to stay under the roughly-800-line module guideline (2026-04-29).
- [ ] Promote the current `MemorySelection` skeleton into the authoritative bounded retrieval-selection pipeline for thread and workspace recall.
- [ ] Finish the first non-vector cut of `MemorySelection` so thread memory, workspace memory, active thread state, pending interaction state, and recent tool results all flow through one selected/available/dropped explanation path.

## Configuration / Provider Surface

- [ ] Complete `reasoning_summary` rollout across backend requests, switching flows, and status surfaces; retire remaining `thinking`-only behavior outside migration fallback.
- [ ] Surface provider-scoped reasoning configuration in `/status` and provider/model switching flows, including where the effective value came from.
- [ ] Deepen provider-surface continuity after hot-swap landed: tighten auth-mode/endpoint alignment, provenance reporting, and remaining runtime continuity edge cases.
- [ ] Align Codex endpoint selection with auth mode so ChatGPT/Codex login and OpenAI API key sessions do not blindly share the same provider URL.
- [ ] Split Codex-specific persisted auth/config back out to `~/.codex` while keeping provider-agnostic RARA config and runtime/session state under `~/.rara`.

## Workspace / Skills / Prompt Sources

- [ ] Expand focused tests around workspace prompt-source discovery and cache invalidation across cwd changes, git branch changes, nested workspaces, and outside-workspace fallback.
- [ ] Define and document `WorkspaceMemory` cache invalidation rules for prompt files and environment info instead of relying on implicit behavior.
- [ ] Unify `discover_prompt_sources()` and TUI `/status` source reporting so displayed prompt sources match the actual injected sources.
- [ ] Preserve stable top-level prompt/context prefixes while adding repo context, skills, hooks, imported agents, and memory sources; new inputs should enter through structured source objects, `MemorySelection`, lifecycle events, or thread-owned agent profiles instead of ad hoc prompt text.
- [ ] Design a project-scoped extension surface that can ingest Claude/Codex-style repo customizations from `.claude/agents/`, `.claude/hooks/`, and `.agents/skills/`, with explicit precedence and compatibility rules before adding runtime execution.
- [ ] Define and surface skill precedence/override behavior across home, repo, nested repo roots, and workspace-local skill roots.
- [ ] Extend `SkillManager::list_summaries()` (or equivalent status output) with source precedence and overridden-skill visibility so conflicts are debuggable.

## Memory / Retrieval / Persistence

- [ ] Extend the local `ThreadStore` / `ThreadRecorder` boundary from a façade over `SessionManager` + `StateDb` into a true structured thread store with explicit thread metadata and rollout-item ownership.
- [ ] Complete the thread lifecycle surface around the new thread boundary: stable `threads`/`thread`/`resume --last`/`fork` flows now exist, but richer lineage metadata and a clearer `latest thread` contract still need to land.
- [ ] Add durable in-turn checkpoints for long-running agent tasks, aligned with Codex-style turn/item status tracking and Claude-style JSONL transcript replay:
  - persist after the user message is accepted, after each assistant message, after each tool-result batch, after runtime continuation messages, and before waiting for pending user approval;
  - make `SessionManager::save_session` crash-tolerant by writing through a temporary file and atomic rename instead of overwriting `history.json` directly;
  - separate resumable transcript history from transient TUI task state so interrupted turns can resume with a clear `in progress` / `interrupted` / `failed` boundary rather than only an end-of-turn save.
- [ ] Define background task restart/reattach semantics before persisting background task metadata across process restarts; the durable index should make completed logs discoverable without pretending killed parent processes are still attachable.
- [ ] Make compaction a first-class runtime lifecycle event with persisted summaries, token counters, and boundary metadata ownership aligned with the thread domain.
- [ ] Define thread-scoped and workspace-scoped `MemoryRecord` storage plus promotion rules so durable findings are not mixed with transient turn context.
- [ ] Replace the current placeholder retrieval path with real vector retrieval over Lance/LanceDB, including metadata-aware ranking for thread and workspace memory selection.
- [ ] Add the retrieval orchestration layer described in `docs/features/context-architecture.md` so thread recall, vector recall, and later graph recall compose into one bounded `MemorySelection`.
- [ ] Design Graph RAG as a later retrieval layer on top of durable memory and extracted relationships instead of as prompt-only glue.

## TUI / UX Parity

- [ ] Continue improving transcript rendering stability across long and streaming sessions: reduce scroll jumps and flicker, strengthen bottom anchoring, and prevent stale transient sections from reappearing after their live phase ends.
- [ ] Improve long-running task progress reporting so the TUI heartbeat reflects the active phase (`sending prompt`, `streaming response`, `running tool`, `waiting for approval`, `checkpointing`) instead of resetting to a generic prompt-sending notice during long tool execution.
- [ ] Rework long `Exploring` / `Explored` handling to follow Codex more closely: keep live exploration compact and summarize committed exploration into a source-aware digest instead of dumping long raw traces.
- [ ] Decouple setup/help/model overlays from transcript layout so overlays behave as a pure top layer and do not perturb history viewport sizing.
- [ ] After exit, print a Codex/Claude-style resume hint that tells the user how to restore the current thread quickly (for example the exact `rara resume <THREAD_ID>` or `rara resume --last` command to use).
- [ ] Add Claude-style repository context hints beneath the input area, especially the current GitHub PR link when the workspace maps to an open PR.
- [ ] Add Codex/Claude-style transcript role cards for `You` / `Agent` / `System` without mixing status chrome into committed transcript history.
- [ ] Bring the main response UI closer to Codex / Claude Code: stabilize active response blocks while streaming and avoid falling back to generic transcript rows for states that should stay in dedicated response cards.
- [ ] Rework the built-in command TUI (`/help`, `/model`, `/status`, command palette, setup overlays) to more closely match Codex / Claude Code.
- [ ] Continue making tool-action transcript summaries more source-aware and file-aware so edit tools (`write_file`, `replace`, `apply_patch`) consistently show what they touched.
- [ ] Continue refining the live `bash` transcript path so command execution behaves more like Codex: clearer lifecycle framing, streamed stdout/stderr handling, and better long-output folding.
- [ ] Add a high-fidelity Claude Code / Codex transcript rendering pass for `write/update`, inline diff display, approval cards, and message-card hierarchy.
- [ ] Expand TUI snapshot coverage for transcript-heavy widgets, overlays, status surfaces, and auth/model picker flows.

## Security / Reliability / Performance

- [ ] Replace the current string-based shell execution path in `src/tools/bash.rs` and `crates/sandbox` with a structured command model (`program`, `args`, `cwd`, `allow_net`).
- [ ] Design an auditable command permission and sandbox-bypass rule model inspired by Codex and Claude Code:
  - keep approval rules separate from sandbox rules;
  - make deny rules take precedence over allow / bypass rules;
  - support explicit command-prefix or command-pattern rules for trusted commands;
  - allow unsandboxed execution only when policy permits it and the transcript/status surface makes the bypass visible;
  - document that convenience exclusions are not a security boundary.
- [ ] Move API key handling in config/backend paths to `secrecy::SecretString` end to end and audit error/reporting paths so secrets are never echoed.
- [ ] Replace `.expect(...)` around provider credential/model setup with structured `anyhow::Context` errors that remain useful without leaking sensitive values.
- [ ] Review path and command validation around `bash`, file tools, and sandbox handoff instead of relying on minimal escaping.
- [ ] Rework token accounting in `src/agent.rs` so repeated checks do not need to re-encode full history every time.
- [ ] Replace the fixed 100ms TUI event polling loop in `src/tui/mod.rs` with a more event-driven wakeup model when the app is idle.
- [ ] Add stronger terminal panic/teardown guards so alternate-screen/raw-mode cleanup is robust on unexpected failures.

## Evaluation / Benchmarks

- [ ] Make Terminal-Bench compatibility an explicit product-quality target: add a headless Harbor/Terminal-Bench adapter, preserve structured trajectories, and keep benchmark data out of prompts, memories, fixtures, and training-oriented artifacts. See `docs/features/terminal-bench-evaluation.md`.
- [ ] Add a small Terminal-Bench smoke run target before attempting full benchmark runs, with recorded RARA revision, dataset version, provider/model, sandbox mode, and failure taxonomy.

## Code Organization / Docs

- [ ] Continue splitting the remaining near-limit runtime/TUI files so the 800-line guideline keeps holding in practice, especially `src/tui/runtime/events/helpers.rs`, `src/tui/custom_terminal.rs`, `src/tui/command.rs`, and `src/agent/planning.rs`.
- [ ] Add module-level documentation for the agent lifecycle, tool loop, context assembly, plan/update flow, and sandbox model.
- [ ] Add comments around non-obvious continuation / plan / current-turn rendering logic so future refactors do not regress the Codex-style workflow.
- [ ] Replace remaining policy-like magic numbers in the TUI/runtime path with named constants.
- [ ] Split backlog planning between architecture/runtime work and TUI/UX parity work if this file starts accumulating duplicate or umbrella-incompatible items again.

## Maintenance Rules

- Keep only open work here.
- Remove completed items after evidence lands in a journal, PR, or canonical feature spec.
- Prefer one umbrella rollout item over many duplicated micro-items for the same surface.
