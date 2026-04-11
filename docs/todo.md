# TODO

Active backlog only. Keep this file small and current.

- [ ] Replace the current setup-heavy TUI flow with a unified slash-command surface: `/help`, `/model`, `/status`, and `/clear` should work from the main input bar without forcing users through a separate setup screen for common actions.
- [ ] Add a first-run onboarding flow that explains workspace, provider/model selection, local model download behavior, cache location, and tool loop expectations before the user lands in a blank chat.
- [ ] Add a richer runtime status line to the TUI so provider, model, revision, workspace, download/inference state, and token usage remain visible during the session.
- [ ] Harden local model prompting contracts for Gemma 4 and Qwen3: chat template handling, stop sequences, and tool-call JSON framing should be explicit and regression-tested.
- [ ] Replace the current hash-based local embedding fallback with a real embedding backend so project memory retrieval quality is good enough for normal coding sessions.
- [ ] Implement the ACP runtime path end to end: `RaraAcpAgent::prompt` and `run_acp_stdio` should execute the real tool/backend loop instead of returning placeholder responses.
- [ ] Implement the Gemini backend instead of keeping the current `Gemini pending` stub so the configured provider is a real option.
- [ ] Implement session context retrieval on top of the existing session storage and vector/session managers instead of returning `no_context_found`.
- [ ] Implement the vector memory tools against the real backend and LanceDB path instead of using placeholder save/retrieve responses.
- [ ] Implement real parallel `team_create` execution instead of the current mocked result payload.

## Maintenance Rules

- Keep only open work here.
- Remove completed items after evidence lands in a journal, PR, or canonical feature spec.
- Prefer one umbrella rollout item over many duplicated micro-items for the same surface.
