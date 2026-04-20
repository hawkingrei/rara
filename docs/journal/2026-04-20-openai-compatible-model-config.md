# 2026-04-20 · OpenAI-Compatible Model Config

## What changed

Added a first-class `OpenAI-compatible` provider family to the inline TUI `/model` flow.

The new family exposes a generic endpoint preset and lets the user edit:

- base URL
- API key
- model name

The backend builder now supports `provider = "openai-compatible"` through the existing `OpenAiCompatibleBackend`.

## Why

RARA already supported several hosted and local backends, but there was no generic way to point the runtime at an arbitrary OpenAI-compatible endpoint from the TUI. Users had to rely on config-file edits or provider-specific paths.

This change keeps Codex login separate while making generic OpenAI-compatible endpoints a first-class runtime choice.

## Key implementation notes

- Added `ProviderFamily::OpenAiCompatible`
- Added `Overlay::ModelNameEditor`
- Added `OPENAI_COMPATIBLE_MODEL_PRESETS`
- Added OpenAI-compatible-specific model-picker shortcuts:
  - `B` base URL
  - `A` API key
  - `N` model name
- Added generic `openai-compatible` backend construction in `build_backend_with_progress`
- Kept provider-scoped remembered state so switching providers restores the right `api_key`, `base_url`, and `model`

## Validation

Focused validation for this checkpoint:

- `cargo test tui::state::state_presets::tests -- --nocapture`
- `cargo test tui::state::tests::openai_compatible_preset_sets_default_connection_fields -- --nocapture`
- `cargo test tui::tests::openai_compatible_model_picker_exposes_connection_edit_shortcuts -- --nocapture`
- `cargo check`

## Follow-up

No new backlog item was added for this checkpoint. Remaining work stays under the broader TUI alignment and auth hardening items already tracked in `docs/todo.md`.
