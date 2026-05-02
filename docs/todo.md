# TODO

Active backlog only. Keep this file small and current.

## Suggested Rollout Order

1. Runtime bootstrap and context contracts
2. Configuration and provider-surface cleanup
3. Workspace / skill observability and cache correctness
4. Memory / retrieval / thread persistence
5. TUI transcript parity and command-surface polish
6. Terminal-Bench evaluation readiness

## Configuration / Provider Surface

- [ ] Complete `reasoning_summary` rollout across backend requests, switching flows, and status surfaces; retire remaining `thinking`-only behavior outside migration fallback.
- [ ] Surface provider-scoped reasoning configuration in `/status` and provider/model switching flows.
- [ ] Study Gemini/Codex-style multi-model routing for top-tier + flash/fast model pairing.
- [ ] Deepen provider-surface continuity after hot-swap: auth-mode/endpoint alignment, provenance reporting.
- [ ] Align Codex endpoint selection with auth mode (ChatGPT/Codex login vs API key).
- [ ] Split Codex-specific persisted auth/config to `~/.codex`, keep RARA config under `~/.rara`.

## Workspace / Skills / Prompt Sources

- [ ] Tests for workspace prompt-source discovery and cache invalidation (cwd changes, git branches, nested workspaces).
- [ ] Define `WorkspaceMemory` cache invalidation rules for prompt files and environment info.
- [ ] Unify `discover_prompt_sources()` and TUI `/status` source reporting.
- [ ] New prompt inputs through structured source objects, `MemorySelection`, lifecycle events — not ad hoc text.
- [ ] Project-scoped extension surface for `.claude/agents/`, `.claude/hooks/`, `.agents/skills/` with precedence rules.
- [ ] Claude-style `verify` skill and `verifier-*` convention (see `docs/features/verify-skill.md`).
- [ ] Evolve `SkillTool` to Codex/Claude contract (see `docs/features/skill-tool.md`): frontmatter, scopes, override visibility.
- [ ] Surface skill precedence/override across home, repo, nested roots.

## Web Tools

- [ ] Replace `web_fetch` HTML-to-text with higher-fidelity markdown conversion.

## Memory / Retrieval / Persistence

- [ ] Real vector-backed retrieval via LanceDB for thread and workspace memory.
- [ ] Durable in-turn checkpoints: persist after each message/tool-result batch, atomic writes, crash-tolerant `SessionManager`.
- [ ] Define background task restart/reattach semantics.
- [ ] Compaction as first-class lifecycle event: persist summaries, token counters, metadata ownership.
- [ ] `ThreadStore` / `ThreadRecorder`: from façade over `SessionManager`+`StateDb` to true structured thread store.
- [ ] Thread-scoped and workspace-scoped `MemoryRecord` storage with promotion rules.
- [ ] Retrieval orchestration layer from `docs/features/context-architecture.md`.
- [ ] Initialize LanceDB and wire vector index for memory selection.

## TUI / Transcript

- [ ] Decouple overlays from transcript layout (pure top layer, no viewport perturbation).
- [ ] Post-exit resume hint (e.g. `rara resume --last`).
- [ ] Claude-style repo context hints beneath input area (GitHub PR link).
- [ ] Codex/Claude-style transcript role cards (`You` / `Agent` / `System`).
- [ ] Stabilize active response blocks while streaming, avoid generic transcript fallback.
- [ ] Rework built-in command TUI (`/help`, `/model`, `/status`, command palette, overlays) to match Codex/Claude.
- [ ] Refine `/status`: provider/model state, reasoning, sandbox/network, context injection, tool availability.
- [ ] Tool-action summaries more source-aware and file-aware.
- [ ] Live `bash` transcript: lifecycle framing, streamed stdout/stderr, long-output folding.
- [ ] High-fidelity render pass for `write/update`, inline diffs, approval cards, message-card hierarchy.
- [ ] Expand TUI snapshot coverage.

## Security / Reliability / Performance

- [ ] Structured command model (`program`, `args`, `cwd`, `allow_net`) in `src/tools/bash.rs` and `crates/sandbox`.
- [ ] Classifier and routing model from `docs/features/classifier-and-routing.md`.
- [ ] Auditable permission + sandbox-bypass rules (Codex/Claude-inspired).
- [ ] Structured auto-permission classifier with compact transcript projection.
- [ ] Background-task state classifier: `working` / `blocked` / `done` / `failed`.
- [ ] `secrecy::SecretString` end-to-end for API keys, audit error paths.
- [ ] Replace `.expect(...)` with structured `anyhow::Context` errors.
- [ ] Review path/command validation in `bash`, file tools, sandbox.
- [ ] Rework token accounting in `src/agent.rs` (avoid re-encoding full history).
- [ ] Replace fixed 100ms TUI event polling loop.
