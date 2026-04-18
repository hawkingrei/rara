# 2026-04-18 Secret Handling Checkpoint

## Summary

Started moving provider credentials away from plain runtime strings and toward `secrecy::SecretString`, while keeping the on-disk config format backward-compatible.

## What changed

- `RaraConfig.api_key` now stores `Option<SecretString>` instead of `Option<String>`.
- config serialization/deserialization remains plain JSON on disk through explicit serde adapters, so existing `config.json` files still load.
- added `RaraConfig` helpers:
  - `api_key()`
  - `has_api_key()`
  - `set_api_key(...)`
  - `clear_api_key()`
- OpenAI-compatible backends now hold `Option<SecretString>` at runtime and only expose the secret when constructing authorization headers.
- `GeminiBackend` now stores its configured API key as `SecretString`.
- Codex API-key save flow now uses `set_api_key(...)` instead of passing raw strings through the config directly.
- opening the API key editor no longer pre-fills the stored key back into the TUI input box.
- OAuth completion in the TUI no longer copies the exchanged access token into `config.api_key`; the runtime clears that field and rebuilds the backend instead.
- provider setup for Kimi/Gemini now returns structured `anyhow::Context` errors instead of panicking through `expect(...)`.
- added a dedicated `src/redaction.rs` helper for best-effort secret and sensitive URL redaction.
- TUI `System` / `Runtime` transcript entries and notices now pass through the same redaction layer instead of formatting raw provider errors directly.
- query failure and compaction failure notices now render through sanitized error-chain text instead of `Display` on the root error alone.
- top-level fatal process exit now redacts secrets before printing the final `Error: ...` line to stderr.
- invalid provider configuration now fails explicitly instead of silently falling back to `MockLlm`, so setup errors stay visible in the TUI and CLI.

## Validation

- `cargo test config::tests -- --nocapture`
- `cargo test redaction::tests -- --nocapture`
- `cargo test main::tests -- --nocapture`
- `cargo test llm::tests -- --nocapture`
- `cargo test tui::command::tests -- --nocapture`
- `cargo check`

## Follow-up

- continue auditing error/reporting paths to ensure secrets never appear in notices, panic chains, or provider setup diagnostics;
- decide whether OAuth token structs should stay serializable in-process only or move behind a dedicated auth store abstraction;
- finish the remaining TODO item for full credential sanitization and logging review.
