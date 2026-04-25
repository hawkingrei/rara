# TODO

Active backlog only. Keep this file small and current.

## Suggested Rollout Order

From higher-leverage structural work toward later UX parity work:

1. Runtime bootstrap and context contracts
2. Configuration and provider-surface cleanup
3. Workspace / skill observability and cache correctness
4. Memory / retrieval / thread persistence
5. TUI transcript parity and command-surface polish

## Phase 1: Architecture Closure

Goal: lock down the boundaries that are most likely to keep expanding unless they are made explicit now.

Acceptance:
- agent, TUI, and session restore share the same context/runtime assembly contract
- `/status` (or equivalent debug output) can explain which context sources were injected, where they came from, and in what order
- `src/main.rs` remains a thin startup orchestrator instead of continuing to own runtime assembly

Priority order for this phase:

1. Split `crates/config/src/lib.rs`
   - Target modules:
     - `defaults.rs`
     - `provider_surface.rs`
     - `migration.rs`
     - `secrets.rs`
     - `serde_helpers.rs`
     - `model.rs`
   - Why first:
     - low-risk organization win
     - makes `reasoning_summary` and provider migration logic easier to test
     - gives backend hot-swap a clearer config entry surface

2. Land the Stage 1 context-architecture boundary as real objects
   - Minimum first cut:
     - define `ContextBudget`
     - define `ContextAssembler`
     - route existing prompt/context assembly through one entrypoint
     - keep current retrieval behavior, but make the assembler own the final assembled context
   - Why now:
     - makes model input assembly explainable
     - reduces the risk of TUI/runtime/backend each building overlapping prompts differently
     - sets up compaction, resume, and later memory recall work

3. Deepen the new thread persistence boundary
   - Introduce objects along the lines of:
     - `ThreadStore`
     - `ThreadRecorder`
     - `RolloutItem`
     - `CompactionRecord`
   - Why before more provider hot-swap work:
     - restore reliability
     - plan-state continuity
     - pending-interaction continuity
     - compaction fidelity all depend on this boundary being explicit

## Architecture / Runtime

- [ ] Extend the shared runtime context and `/context` from prompt-injected / compacted selected memory items into real recalled vector/thread memory selection so the runtime can explain why those items won the retrieval budget.

## Configuration / Provider Surface

- [ ] Replace provider-scoped `thinking: bool` with a Codex-style reasoning summary configuration model plus config migration.
- [ ] Surface provider-scoped reasoning configuration in `/status` and provider/model switching flows, including where the effective value came from.
- [ ] Support in-session model/provider switching via backend hot-swap without resetting the active TUI transcript, session id, plan state, pending interactions, or compacted history.
- [ ] Align Codex endpoint selection with auth mode so ChatGPT/Codex login and OpenAI API key sessions do not blindly share the same provider URL.
- [ ] Split Codex-specific persisted auth/config back out to `~/.codex` while keeping provider-agnostic RARA config and runtime/session state under `~/.rara`.

## Workspace / Skills / Prompt Sources

- [ ] Expand focused tests around workspace prompt-source discovery and cache invalidation across cwd changes, git branch changes, nested workspaces, and outside-workspace fallback.
- [ ] Define and document `WorkspaceMemory` cache invalidation rules for prompt files and environment info instead of relying on implicit behavior.
- [ ] Unify `discover_prompt_sources()` and TUI `/status` source reporting so displayed prompt sources match the actual injected sources.
- [ ] Define and surface skill precedence/override behavior across home, repo, nested repo roots, and workspace-local skill roots.
- [ ] Extend `SkillManager::list_summaries()` (or equivalent status output) with source precedence and overridden-skill visibility so conflicts are debuggable.

## Memory / Retrieval / Persistence

- [ ] Extend the new local `ThreadStore` / `ThreadRecorder` boundary from a façade over `SessionManager` + `StateDb` into a true structured thread store with explicit thread metadata and rollout-item ownership.
- [ ] Complete the thread lifecycle surface around the new thread boundary: stable `threads`/`thread`/`resume --last`/`fork` flows now exist, but richer lineage metadata and a clearer `latest thread` contract still need to land.
- [ ] Make compaction a first-class runtime event with persisted summaries, token counters, and boundary metadata.
- [ ] Define thread-scoped and workspace-scoped `MemoryRecord` storage plus promotion rules so durable findings are not mixed with transient turn context.
- [ ] Replace the current placeholder retrieval path with real vector retrieval over Lance/LanceDB, including metadata-aware ranking for thread and workspace memory selection.
- [ ] Add the retrieval orchestration layer described in `docs/features/context-architecture.md` so thread recall, vector recall, and later graph recall compose into one bounded `MemorySelection`.
- [ ] Design Graph RAG as a later retrieval layer on top of durable memory and extracted relationships instead of as prompt-only glue.

## TUI / UX Parity

- [ ] Continue improving transcript rendering stability across long and streaming sessions: reduce scroll jumps and flicker, strengthen bottom anchoring, and prevent stale transient sections from reappearing after their live phase ends.
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

- [ ] Replace the current string-based shell execution path in `src/tools/bash.rs` and `src/sandbox.rs` with a structured command model (`program`, `args`, `cwd`, `allow_net`).
- [ ] Move API key handling in config/backend paths to `secrecy::SecretString` end to end and audit error/reporting paths so secrets are never echoed.
- [ ] Replace `.expect(...)` around provider credential/model setup with structured `anyhow::Context` errors that remain useful without leaking sensitive values.
- [ ] Review path and command validation around `bash`, file tools, and sandbox handoff instead of relying on minimal escaping.
- [ ] Rework token accounting in `src/agent.rs` so repeated checks do not need to re-encode full history every time.
- [ ] Replace the fixed 100ms TUI event polling loop in `src/tui/mod.rs` with a more event-driven wakeup model when the app is idle.
- [ ] Add stronger terminal panic/teardown guards so alternate-screen/raw-mode cleanup is robust on unexpected failures.

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
