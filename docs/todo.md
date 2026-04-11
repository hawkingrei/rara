# TODO

Active backlog only. Keep this file small and current.

- [ ] Replace the current setup-heavy TUI flow with a unified slash-command surface: `/help`, `/model`, `/status`, and `/clear` should work from the main input bar without forcing users through a separate setup screen for common actions.
- [ ] Add a first-run onboarding flow that explains workspace, provider/model selection, local model download behavior, cache location, and tool loop expectations before the user lands in a blank chat.
- [ ] Add a richer runtime status line to the TUI so provider, model, revision, workspace, download/inference state, and token usage remain visible during the session.
- [ ] Harden local model prompting contracts for Gemma 4 and Qwen3: chat template handling, stop sequences, and tool-call JSON framing should be explicit and regression-tested.
- [ ] Replace the current hash-based local embedding fallback with a real embedding backend so project memory retrieval quality is good enough for normal coding sessions.

## Maintenance Rules

- Keep only open work here.
- Remove completed items after evidence lands in a journal, PR, or canonical feature spec.
- Prefer one umbrella rollout item over many duplicated micro-items for the same surface.
