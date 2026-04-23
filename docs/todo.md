# TODO

Active backlog only. Keep this file small and current.

## Suggested Rollout Order

From easier / lower-risk work toward harder / more structural work:

1. Transcript and command-surface polish
   - [ ] Add Claude-style repository context hints beneath the input area, especially the current GitHub PR link (when the workspace is on a PR branch or can be mapped to an open PR), so the active review context stays visible without manual lookup.
   - [ ] Add Codex/Claude-style transcript role cards for `You` / `Agent` / `System`: use clearer card/background separation without mixing status chrome into committed transcript history.
   - [ ] Bring the main response UI closer to Codex / Claude Code: tighten the `You` / `Agent` message-card hierarchy, keep active response blocks visually stable while streaming, and avoid falling back to generic transcript rows for states that should render as dedicated response cards.
   - [ ] Rework the built-in command TUI (`/help`, `/model`, `/status`, command palette, setup overlays) to more closely match Codex / Claude Code: better information density, clearer keyboard affordances, stronger selection states, and less modal friction.

2. Transcript stability and compactness
   - [ ] Improve transcript rendering stability across long and streaming sessions: reduce scroll jumps and flicker, keep bottom anchoring stable while new content streams in, and prevent stale transient sections (`Exploring`, `Updated Plan`, busy chrome) from reappearing after their live phase has ended.
   - [ ] Rework long `Exploring` / `Explored` handling to follow Codex more closely: keep live exploration compact, summarize committed exploration into a small source-aware digest (prefer actual `read`/inspection actions over noisy `list`/`glob`/search chatter), and avoid dumping long raw action traces into the main transcript.
   - [ ] Decouple setup/help/model overlays from transcript layout so popups do not perturb history rendering: overlays should render as a pure top layer instead of changing transcript viewport sizing or causing history reflow/flicker when opened and closed.
   - [ ] Expand the new Codex-style TUI snapshot coverage beyond the first auth-picker / queued-follow-up / Updated Plan snapshots so more popups, status surfaces, and transcript-heavy widgets are protected by golden tests.

3. Rich Codex/Claude transcript parity
   - [ ] Continue making tool-action transcript summaries more source-aware and file-aware so edit tools such as `write_file` / `replace` / `apply_patch` consistently show what they touched instead of only generic action labels.
   - [ ] Continue refining the live `bash` transcript path so command execution behaves more like Codex: keep the streamed stdout/stderr surface, then add richer lifecycle details such as clearer command-start/finish framing and better long-output folding.
   - [ ] Add a high-fidelity Claude Code / Codex transcript rendering pass: mirror the structured `write/update` tool presentation, inline diff display, approval cards, and message-card hierarchy as closely as practical instead of only loosely borrowing the style, and explicitly use Codex as the primary reference for `bash` / command lifecycle framing, stdout/stderr streaming, completion summaries, and output folding.

4. Reasoning and model-surface alignment
   - [ ] Add a Codex-style configurable `model_reasoning_summary` surface instead of a boolean thinking toggle: support model/provider-scoped configuration, show reasoning summaries only when the backend emits them, and keep the transcript/status behavior aligned with Codex (`none` / `auto` / richer summary modes) rather than exposing raw chain-of-thought.
   - [ ] Continue refining queued follow-up steering toward the full Codex contract: keep the new next-tool-boundary queue, then add the explicit interrupt/send-now path and clearer separation between pending steers and ordinary queued follow-ups.

5. Deeper runtime and architecture work
   - [ ] Unify runtime assembly and startup boundaries across `src/main.rs` and `src/tui/runtime/tasks.rs`: introduce one shared runtime/builder entry point for backend/tool/session/workspace initialization, stop silently swallowing `SkillManager::load_all()` failures, move hard-coded persistence paths such as `data/lancedb` behind config/workspace path resolution, and keep the root crate split between thin entrypoints and explicit runtime-context assembly instead of letting `main.rs` keep parser/setup/dispatch responsibilities together.
   - [ ] Implement the Stage 1 context-architecture boundary from `docs/features/context-architecture.md`: introduce explicit `ContextBudget` and `ContextAssembler` layers so stable instructions, workspace context, active turn context, memory selections, and compacted history stop being assembled implicitly across unrelated modules.
   - [ ] Implement a local `ThreadStore` / `ThreadRecorder` boundary so thread metadata, rollout history, plan state, pending interactions, and future sub-agent lineage are persisted as structured items instead of reconstructed from flattened transcript text.
   - [ ] Introduce a dedicated transcript viewport model that renders history from stable turn data plus scroll state instead of letting overlays and transient runtime sections implicitly drive layout; this should become the basis for future virtualization, stronger scroll anchoring, and less flicker under streaming updates.

## Security and Reliability

- [ ] Replace the current string-based shell execution path in `src/tools/bash.rs` and `src/sandbox.rs` with a structured command model (`program`, `args`, `cwd`, `allow_net`) so `bash -c` / `sh -c` is no longer the default execution path.
- [ ] Move API key handling in `src/llm.rs` and config paths to `secrecy::SecretString`, and audit error/reporting paths so secrets are never echoed in logs or panic messages.
- [ ] Replace `.expect(...)` on provider credential/model setup with structured `anyhow::Context` errors that remain useful without leaking sensitive values.
- [ ] Review path and command validation around `bash`, file tools, and sandbox handoff; define a stricter validation policy instead of relying on minimal escaping.

## LLM and Networking

- [ ] Broaden Codex-auth validation beyond the current bridge tests: add stronger callback-flow coverage and more end-to-end persistence checks so the browser/device/API-key/logout paths stay regression-tested even as `codex_login` evolves.
- [ ] Split Codex-specific persisted auth/config back out to `~/.codex` while keeping provider-agnostic RARA config, workspace runtime state, and session data under `~/.rara`.
- [ ] Align Codex endpoint selection with auth mode: ChatGPT/Codex login and OpenAI API key sessions should not share the same provider URL blindly, even though both currently reuse the Responses-shaped backend contract.
- [ ] Add an `AgentHub team mode` on top of ACP: for role-specialized worker sessions, use a small model first for intent routing and only hand the task to the larger worker model when the intent is relevant to that worker, instead of sending every ACP turn directly to the expensive model.
- [ ] Deepen the `AgentHub team mode` spec before implementation: define the ACP session metadata, worker-role contract, router prompt/output schema, and the exact `skip` / `handle` response semantics so the worker runtime can be implemented without inventing a parallel protocol later.

## Memory and Retrieval

- [ ] Implement the Stage 1 context-architecture boundary from `docs/features/context-architecture.md`: introduce explicit `ContextBudget` and `ContextAssembler` layers so stable instructions, workspace context, active turn context, memory selections, and compacted history stop being assembled implicitly across unrelated modules.
- [ ] Implement a local `ThreadStore` / `ThreadRecorder` boundary so thread metadata, rollout history, plan state, pending interactions, and future sub-agent lineage are persisted as structured items instead of reconstructed from flattened transcript text.
- [ ] Make compaction a first-class runtime event: persist compacted summary items, before/after token counts, and replacement metadata so long threads can resume cleanly without replaying unbounded raw history.
- [ ] Define thread-scoped and workspace-scoped `MemoryRecord` storage plus promotion rules so durable findings, preferences, and repo facts stop being mixed with transient turn context.
- [ ] Replace the current placeholder retrieval path with real vector retrieval over Lance/LanceDB, including metadata-aware ranking for thread memory and workspace memory selections before injection into context.
- [ ] Add the retrieval orchestration layer described in `docs/features/context-architecture.md`: a `Retriever` boundary that can merge thread recall, workspace vector recall, and later graph-based recall into one bounded `MemorySelection`.
- [ ] Design Graph RAG as a later retrieval layer on top of durable memory and extracted relationships instead of as a prompt hack: define graph nodes/edges, traversal outputs, and how graph results compose with vector retrieval.
- [ ] Add a first-run onboarding flow that explains workspace, provider/model selection, local model download behavior, cache location, and tool loop expectations before the user lands in a blank chat.
- [ ] Continue aligning the TUI status and transcript surfaces with Codex/Claude so runtime state stays visible without leaking bottom-pane chrome into transcript history.
- [ ] Improve transcript rendering stability across long and streaming sessions: reduce scroll jumps and flicker, keep bottom anchoring stable while new content streams in, and prevent stale transient sections (`Exploring`, `Updated Plan`, busy chrome) from reappearing after their live phase has ended.
- [ ] Add Claude-style repository context hints beneath the input area, especially the current GitHub PR link (when the workspace is on a PR branch or can be mapped to an open PR), so the active review context stays visible without manual lookup.
- [ ] Rework the built-in command TUI (`/help`, `/model`, `/status`, command palette, setup overlays) to more closely match Codex / Claude Code: better information density, clearer keyboard affordances, stronger selection states, and less modal friction.
- [ ] Add a Codex-style configurable `model_reasoning_summary` surface instead of a boolean thinking toggle: support model/provider-scoped configuration, show reasoning summaries only when the backend emits them, and keep the transcript/status behavior aligned with Codex (`none` / `auto` / richer summary modes) rather than exposing raw chain-of-thought.
- [ ] Add Codex/Claude-style transcript role cards for `You` / `Agent` / `System`: use clearer card/background separation without mixing status chrome into committed transcript history.
- [ ] Add a high-fidelity Claude Code / Codex transcript rendering pass: mirror the structured `write/update` tool presentation, inline diff display, approval cards, and message-card hierarchy as closely as practical instead of only loosely borrowing the style, and explicitly use Codex as the primary reference for `bash` / command lifecycle framing, stdout/stderr streaming, completion summaries, and output folding.
- [ ] Rework long `Exploring` / `Explored` handling to follow Codex more closely: keep live exploration compact, summarize committed exploration into a small source-aware digest (prefer actual `read`/inspection actions over noisy `list`/`glob`/search chatter), and avoid dumping long raw action traces into the main transcript.
- [ ] Expand the new Codex-style TUI snapshot coverage beyond the first auth-picker / queued-follow-up / Updated Plan snapshots so more popups, status surfaces, and transcript-heavy widgets are protected by golden tests.
- [ ] Continue making tool-action transcript summaries more source-aware and file-aware so edit tools such as `write_file` / `replace` / `apply_patch` consistently show what they touched instead of only generic action labels.
- [ ] Continue refining the live `bash` transcript path so command execution behaves more like Codex: keep the streamed stdout/stderr surface, then add richer lifecycle details such as clearer command-start/finish framing and better long-output folding.
- [ ] Continue refining queued follow-up steering toward the full Codex contract: keep the new next-tool-boundary queue, then add the explicit interrupt/send-now path and clearer separation between pending steers and ordinary queued follow-ups.
- [ ] Harden local model prompting contracts for Gemma 4 and Qwen3: chat template handling, stop sequences, and tool-call JSON framing should be explicit and regression-tested.
- [ ] Replace the current hash-based local embedding fallback with a real embedding backend so project memory retrieval quality is good enough for normal coding sessions.
- [ ] Replace the mock `VectorDB` implementation in `src/vectordb.rs` with real LanceDB-backed search/upsert behavior, or feature-gate the memory tools until the backend is real.
- [ ] Implement the ACP runtime path end to end: `RaraAcpAgent::prompt` and `run_acp_stdio` should execute the real tool/backend loop instead of returning placeholder responses, because AgentHub integration will use ACP as the worker boundary.
- [ ] Implement the Gemini backend instead of keeping the current `Gemini pending` stub so the configured provider is a real option.
- [ ] Implement session context retrieval on top of the existing session storage and vector/session managers instead of returning `no_context_found`.
- [ ] Implement the vector memory tools against the real backend and LanceDB path instead of using placeholder save/retrieve responses.
- [ ] Implement real parallel `team_create` execution instead of the current mocked result payload.
- [ ] Evaluate and add a `thread-store`-style persistence boundary so thread/session metadata, rollout history, resume, archive, and future remote-backed thread storage do not stay split across unrelated local state surfaces.

## Performance and Runtime

- [ ] Rework token accounting in `src/agent.rs` so repeated checks do not need to re-encode the full history every time.
- [ ] Replace the fixed 100ms TUI event polling loop in `src/tui/mod.rs` with a more event-driven wakeup model when the app is idle.
- [ ] Add terminal panic/teardown guards so alternate-screen/raw-mode cleanup is more robust on unexpected failures.
- [ ] Reduce per-frame TUI render allocations in `src/tui/render/cells.rs`: avoid rebuilding `Vec`s and boxed trait objects on every `display_lines()` call, likely by letting history/active cells append into an existing buffer instead of returning freshly allocated collections.

## Code Organization and Docs

- [ ] Unify runtime assembly and startup boundaries across `src/main.rs` and `src/tui/runtime/tasks.rs`: introduce one shared runtime/builder entry point for backend/tool/session/workspace initialization, stop silently swallowing `SkillManager::load_all()` failures, move hard-coded persistence paths such as `data/lancedb` behind config/workspace path resolution, and keep the root crate split between thin entrypoints and explicit runtime-context assembly instead of letting `main.rs` keep parser/setup/dispatch responsibilities together.
- [ ] Continue the internal-crate rollout after `rara-config`, `rara-instructions`, and `rara-skills`: move more skill runtime behavior behind `rara-skills`, then extract the next stable boundary instead of growing the root crate back toward a monolith.
- [ ] Revisit direct reuse of `codex-core-skills` after the Codex-compatible root-discovery phase: keep `rara-skills` as the adaptation boundary, but replace more of the custom loader/render/invocation stack once the dependency surface is acceptable.
- [ ] Refine instruction resolution so `AGENTS.md` / instruction files behave more like Codex and Claude Code: keep hierarchical lookup, then define clearer precedence and merge rules for nested project instructions versus workspace-local RARA instructions.
- [ ] Continue splitting the remaining near-limit runtime/TUI files so the 800-line guideline keeps holding in practice, especially `src/tui/runtime/events/helpers.rs`, `src/tui/custom_terminal.rs`, `src/tui/command.rs`, and `src/agent/planning.rs`.
- [ ] Add module-level documentation for the agent lifecycle, tool loop, plan/update flow, and sandbox model so the runtime architecture is easier to reason about.
- [ ] Add a security section to `AGENTS.md` or a dedicated security doc covering sandbox expectations, command execution rules, and secret-handling standards.
- [ ] Add comments around the non-obvious continuation / plan / current-turn rendering logic so future refactors do not regress the Codex-style workflow.
- [ ] Replace remaining magic numbers in the TUI/runtime path with named constants where the values encode policy rather than layout convenience.
- [ ] Decide whether config hot-reload is a real roadmap item; if yes, add a scoped design and file-watcher implementation plan instead of leaving it implicit.

## Maintenance Rules

- Keep only open work here.
- Remove completed items after evidence lands in a journal, PR, or canonical feature spec.
- Prefer one umbrella rollout item over many duplicated micro-items for the same surface.
