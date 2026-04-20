# OpenAI-Compatible Model Configuration

## Goal

RARA should expose a generic OpenAI-compatible provider path in the inline TUI model flow so a user can point the agent at any OpenAI-compatible endpoint without editing config files by hand.

This path is separate from Codex-auth login. Codex keeps its own auth flow and provider-specific runtime, while generic OpenAI-compatible endpoints use explicit connection settings.

## User Contract

When the user opens `/model`, the provider picker includes:

- `Codex`
- `OpenAI-compatible`
- `Candle Local`
- `Ollama`

When `OpenAI-compatible` is selected, the model picker must allow editing:

- `base URL`
- `API key`
- `model name`

These values are editable directly from the model picker through dedicated shortcuts instead of forcing manual config edits.

## Defaults

Selecting the OpenAI-compatible preset sets:

- `provider = "openai-compatible"`
- `model = "gpt-4.1-mini"` if no explicit model has been set for that provider
- `base_url = "https://api.openai.com/v1"` if no explicit base URL has been set for that provider
- `revision = None`

The generic OpenAI-compatible backend is then constructed from:

- `api_key`
- `base_url`
- `model`

## Remembered Provider State

RARA keeps provider-scoped remembered state in `RaraConfig.provider_states`.

Switching between providers must preserve and restore provider-local settings instead of overwriting them globally. For the OpenAI-compatible path this includes at least:

- `api_key`
- `base_url`
- `model`
- `revision`
- `thinking`
- `num_ctx`

This allows the user to switch between Codex, Ollama, local models, and generic OpenAI-compatible endpoints without losing per-provider connection details.

## TUI Behavior

In the model picker:

- `B` opens the base URL editor for OpenAI-compatible and Ollama
- `A` opens the API key editor for OpenAI-compatible
- `N` opens the model-name editor for OpenAI-compatible
- `Enter` applies the selected preset and rebuilds the backend

The base URL editor and API key editor must use generic OpenAI-compatible wording when the active provider is `openai-compatible`, not Codex-specific auth wording.

The model-name editor is a separate overlay so the user can update the remote model identifier without editing config files.

## Non-Goals

- This feature does not replace the Codex OAuth flow.
- This feature does not introduce provider-specific capability discovery.
- This feature does not validate arbitrary endpoint compatibility beyond normal backend construction errors.
