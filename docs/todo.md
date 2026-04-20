# TODO

Active backlog only. Keep this file small and current.

## Security and Reliability

- [ ] Replace the current string-based shell execution path in `src/tools/bash.rs` and `src/sandbox.rs` with a structured command model (`program`, `args`, `cwd`, `allow_net`) so `bash -c` / `sh -c` is no longer the default execution path.
- [ ] Move API key handling in `src/llm.rs` and config paths to `secrecy::SecretString`, and audit error/reporting paths so secrets are never echoed in logs or panic messages.
- [ ] Replace `.expect(...)` on provider credential/model setup with structured `anyhow::Context` errors that remain useful without leaking sensitive values.
- [ ] Review path and command validation around `bash`, file tools, and sandbox handoff; define a stricter validation policy instead of relying on minimal escaping.

## LLM and Networking

- [ ] Replace the current Codex-auth mirror in `src/oauth.rs` with direct reuse of the smallest useful `codex_login` primitives so browser login, device-code login, API-key login, and logout stop drifting from Codex behavior over time.
- [ ] Broaden Codex-auth validation after the first parity pass: add callback-flow, persistence, and TUI auth-picker tests so the new browser/device/API-key/logout paths stay regression-tested.
- [ ] Add an `AgentHub team mode` on top of ACP: for role-specialized worker sessions, use a small model first for intent routing and only hand the task to the larger worker model when the intent is relevant to that worker, instead of sending every ACP turn directly to the expensive model.
- [ ] Deepen the `AgentHub team mode` spec before implementation: define the ACP session metadata, worker-role contract, router prompt/output schema, and the exact `skip` / `handle` response semantics so the worker runtime can be implemented without inventing a parallel protocol later.

## Memory and Retrieval

- [ ] Add a first-run onboarding flow that explains workspace, provider/model selection, local model download behavior, cache location, and tool loop expectations before the user lands in a blank chat.
- [ ] Continue aligning the TUI status and transcript surfaces with Codex/Claude so runtime state stays visible without leaking bottom-pane chrome into transcript history.
- [ ] Add Codex-style TUI snapshot coverage for popups, status surfaces, and transcript-heavy widgets so layout/text regressions are caught by golden snapshots instead of only narrow string assertions.
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

- [ ] Continue the internal-crate rollout after `rara-config`, `rara-instructions`, and `rara-skills`: move more skill runtime behavior behind `rara-skills`, then extract the next stable boundary instead of growing the root crate back toward a monolith.
- [ ] Revisit direct reuse of `codex-core-skills` after the Codex-compatible root-discovery phase: keep `rara-skills` as the adaptation boundary, but replace more of the custom loader/render/invocation stack once the dependency surface is acceptable.
- [ ] Refine instruction resolution so `AGENTS.md` / instruction files behave more like Codex and Claude Code: keep hierarchical lookup, then define clearer precedence and merge rules for nested project instructions versus local `.rara` instructions.
- [ ] Continue splitting remaining oversized TUI files such as `src/tui/state.rs` and `src/tui/markdown_render.rs` so the 800-line guideline holds across the main interaction path.
- [ ] Add module-level documentation for the agent lifecycle, tool loop, plan/update flow, and sandbox model so the runtime architecture is easier to reason about.
- [ ] Add a security section to `AGENTS.md` or a dedicated security doc covering sandbox expectations, command execution rules, and secret-handling standards.
- [ ] Add comments around the non-obvious continuation / plan / current-turn rendering logic so future refactors do not regress the Codex-style workflow.
- [ ] Replace remaining magic numbers in the TUI/runtime path with named constants where the values encode policy rather than layout convenience.
- [ ] Decide whether config hot-reload is a real roadmap item; if yes, add a scoped design and file-watcher implementation plan instead of leaving it implicit.

## Maintenance Rules

- Keep only open work here.
- Remove completed items after evidence lands in a journal, PR, or canonical feature spec.
- Prefer one umbrella rollout item over many duplicated micro-items for the same surface.
