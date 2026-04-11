# Local Model Bootstrap

## Summary

Bootstrapped RARA's first real local-model path with Candle-backed Gemma 4 and Qwen3 support,
download progress, persistent model caching, and initial TUI model switching.

## Background

RARA already had an agent loop, tool manager, and TUI shell, but local inference support was not a
first-class path. The new work introduced a local backend while keeping the existing `LlmBackend`
abstraction intact.

## Scope

- Added a shared local backend based on upstream Hugging Face Candle.
- Added Gemma 4 E4B / E2B and Qwen3 8B preset support.
- Added provider/model alias resolution for local usage.
- Added persistent Hugging Face cache configuration and visible download progress.
- Added initial TUI preset switching for local models.
- Removed the API-key requirement for local providers inside the TUI flow.
- Added GitHub Actions build and clippy workflows.

## Key Decisions

- Use Candle from upstream `main` rather than crates.io releases for faster model-family support.
- Keep local inference behind `LocalLlmBackend` so the agent loop does not gain model-specific branches.
- Use a constrained JSON shim for local tool-calling until a stronger native function-calling path exists.
- Store model cache data under a user-global cache root instead of per-project state.
- Treat the current setup screen as a transitional surface, not the final TUI interaction model.

## Validation

- `cargo check`
- focused local backend tests for alias resolution and parsing behavior
- `cargo clippy --locked --all-targets --no-deps`

## Follow-ups

- Move common model/runtime controls into slash commands.
- Add first-run onboarding instead of relying on the setup screen alone.
- Replace the hash embedding fallback with a real embedding backend.
- Add stronger prompt-format and stop-sequence coverage for Gemma 4 and Qwen3.
