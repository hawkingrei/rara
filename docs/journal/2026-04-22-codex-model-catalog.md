# 2026-04-22 · Codex Model Catalog and Reasoning Picker

## Summary

RARA previously treated Codex models as a small hardcoded preset list. That had
already drifted from upstream Codex and could not represent per-model reasoning
levels.

This checkpoint switches the Codex picker to an upstream-catalog-driven flow and
adds reasoning-effort selection as first-class provider state.

## Upstream Reference

The implementation was aligned against local `codex-rs` first:

- `models-manager/src/model_presets.rs`
  - hardcoded presets were removed upstream; visible models come from the
    active catalog
- `models-manager/src/manager.rs`
  - `ModelsManager::list_models(...)` refreshes and filters model presets
- `protocol/src/openai_models.rs`
  - `ModelPreset` carries `default_reasoning_effort`,
    `supported_reasoning_efforts`, and auth-aware visibility metadata
- `tui/src/chatwidget.rs`
  - Codex opens a second reasoning-level picker only when the chosen model has
    multiple supported efforts

## Implementation

The RARA-side changes in this pass are:

- add a direct git dependency on `codex-models-manager`
- add `src/codex_model_catalog.rs` as a thin RARA adapter over:
  - `codex_models_manager`
  - `codex_login::AuthManager`
- replace the local hardcoded Codex model picker with catalog-driven entries
- add `reasoning_effort` to provider-scoped config state
- pass the selected reasoning effort through the Codex `/v1/responses` request
- add a second TUI overlay for reasoning-level selection
- skip that second overlay when the selected model only supports one effort

## Notes

- Codex model selection is now runtime-driven, but RARA still keeps a local
  fallback default model for non-picker paths.
- That fallback was updated from the old `gpt-5-codex` value to `gpt-5.4`, and
  legacy Codex model names are now migrated on load/default-application.
- Auth-mode-specific endpoint selection remains follow-up work; this checkpoint
  only addresses model catalog and reasoning metadata.

## Validation

Targeted validation for this pass:

- `cargo check`
- `cargo test -p rara-config provider_switch_restores_provider_specific_settings -- --nocapture`
- `cargo test codex_responses_request_includes_reasoning_effort_when_selected -- --nocapture`
- `cargo test codex_model_picker_opens_reasoning_level_overlay_before_rebuild -- --nocapture`
