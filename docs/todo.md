# TODO

Active backlog only. Keep this file small and current.

## Security and Reliability

- [ ] Block unsandboxed fallback on unknown platforms in `src/sandbox.rs`: unsupported hosts should fail closed instead of returning the original command unwrapped.
- [ ] Replace the current string-based shell execution path in `src/tools/bash.rs` and `src/sandbox.rs` with a structured command model (`program`, `args`, `cwd`, `allow_net`) so `bash -c` / `sh -c` is no longer the default execution path.
- [ ] Harden sandbox profile generation in `src/sandbox.rs`: avoid rewriting a shared `sandbox.sb` on every command by using per-command profiles or explicit locking.
- [ ] Tighten Linux sandboxing in `src/sandbox.rs`: stop binding the entire filesystem by default and move to a minimal bind set for the workspace and required runtime paths.
- [ ] Move API key handling in `src/llm.rs` and config paths to `secrecy::SecretString`, and audit error/reporting paths so secrets are never echoed in logs or panic messages.
- [ ] Replace `.expect(...)` on provider credential/model setup with structured `anyhow::Context` errors that remain useful without leaking sensitive values.
- [ ] Review path and command validation around `bash`, file tools, and sandbox handoff; define a stricter validation policy instead of relying on minimal escaping.

## LLM and Networking

- [ ] Reuse shared `reqwest::Client` instances in `src/llm.rs` instead of constructing a new client per request.
- [ ] Add explicit HTTP timeouts to all networked backends in `src/llm.rs` so provider calls fail predictably instead of hanging indefinitely.
- [ ] Extract shared message-mapping helpers across OpenAI-compatible and Ollama backends to reduce duplication and keep tool-call behavior consistent.
- [ ] Add clearer config/runtime feedback when provider setup is invalid instead of silently falling back to partial behavior.

## Memory and Retrieval

- [ ] Replace the current setup-heavy TUI flow with a unified slash-command surface: `/help`, `/model`, `/status`, and `/clear` should work from the main input bar without forcing users through a separate setup screen for common actions.
- [ ] Add a first-run onboarding flow that explains workspace, provider/model selection, local model download behavior, cache location, and tool loop expectations before the user lands in a blank chat.
- [ ] Add a richer runtime status line to the TUI so provider, model, revision, workspace, download/inference state, and token usage remain visible during the session.
- [ ] Harden local model prompting contracts for Gemma 4 and Qwen3: chat template handling, stop sequences, and tool-call JSON framing should be explicit and regression-tested.
- [ ] Replace the current hash-based local embedding fallback with a real embedding backend so project memory retrieval quality is good enough for normal coding sessions.
- [ ] Replace the mock `VectorDB` implementation in `src/vectordb.rs` with real LanceDB-backed search/upsert behavior, or feature-gate the memory tools until the backend is real.
- [ ] Implement the ACP runtime path end to end: `RaraAcpAgent::prompt` and `run_acp_stdio` should execute the real tool/backend loop instead of returning placeholder responses.
- [ ] Implement the Gemini backend instead of keeping the current `Gemini pending` stub so the configured provider is a real option.
- [ ] Implement session context retrieval on top of the existing session storage and vector/session managers instead of returning `no_context_found`.
- [ ] Implement the vector memory tools against the real backend and LanceDB path instead of using placeholder save/retrieve responses.
- [ ] Implement real parallel `team_create` execution instead of the current mocked result payload.

## Performance and Runtime

- [ ] Rework token accounting in `src/agent.rs` so repeated checks do not need to re-encode the full history every time.
- [ ] Replace the fixed 100ms TUI event polling loop in `src/tui/mod.rs` with a more event-driven wakeup model when the app is idle.
- [ ] Add terminal panic/teardown guards so alternate-screen/raw-mode cleanup is more robust on unexpected failures.

## Code Organization and Docs

- [ ] Split oversized orchestration functions such as `create_full_tool_manager()` in `src/main.rs` and `dispatch_event()` in `src/tui/mod.rs` into smaller focused helpers.
- [ ] Add module-level documentation for the agent lifecycle, tool loop, plan/update flow, and sandbox model so the runtime architecture is easier to reason about.
- [ ] Add a security section to `AGENTS.md` or a dedicated security doc covering sandbox expectations, command execution rules, and secret-handling standards.
- [ ] Add comments around the non-obvious continuation / plan / current-turn rendering logic so future refactors do not regress the Codex-style workflow.
- [ ] Replace remaining magic numbers in the TUI/runtime path with named constants where the values encode policy rather than layout convenience.
- [ ] Decide whether config hot-reload is a real roadmap item; if yes, add a scoped design and file-watcher implementation plan instead of leaving it implicit.

## Maintenance Rules

- Keep only open work here.
- Remove completed items after evidence lands in a journal, PR, or canonical feature spec.
- Prefer one umbrella rollout item over many duplicated micro-items for the same surface.
