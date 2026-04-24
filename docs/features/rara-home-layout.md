# RARA Home Layout

## Summary

RARA should not create a project-local `.rara` directory during normal runtime.

Instead, runtime state should live under the user's home directory:

- global config under `~/.rara`
- workspace-scoped runtime data under `~/.rara/workspaces/<workspace-id>`

This keeps repository working trees clean while still preserving per-workspace
state.

## Goals

- Keep configuration and runtime state out of the project directory by default.
- Preserve per-workspace isolation for:
  - session history
  - sqlite state
  - sandbox profiles
  - tool-result artifacts
  - workspace memory files
- Support multiple providers/models without flattening all provider-scoped
  settings into one mutable config surface.

## Non-Goals

- Removing explicit test fixtures that intentionally create temporary `.rara`
  directories.
- Redesigning the entire persistence model into a thread-store boundary in the
  same change.
- Changing skill discovery semantics beyond runtime state placement.

## Required Layout

### Global Home

RARA home is:

- `~/.rara`

It stores:

- `config.json`
- Codex auth state (currently `codex-auth/`)
- future global caches or shared metadata

### Workspace Data

Each workspace gets a dedicated directory under:

- `~/.rara/workspaces/<workspace-id>`

The `<workspace-id>` should be stable for the same canonical workspace path and
human-scannable enough to inspect manually. A slug-plus-hash identifier is
acceptable.

Workspace data currently includes:

- `rollouts/`
- `sessions/`
- `state.sqlite3`
- `sandbox/`
- `tool-results/`
- workspace memory files such as `instructions.md` and `memory.md`

## Config Model

Because RARA supports multiple providers and model families, config should not
only behave like a single flat mutable profile.

The global config should keep:

- the currently selected provider
- prompt-related global settings
- provider-scoped remembered state for fields such as:
  - API key
  - base URL
  - model
  - revision
  - reasoning summary mode
  - provider-specific thinking/runtime toggles where applicable
  - context size

Switching providers should restore the remembered provider-scoped state when it
exists, instead of forcing every provider switch to overwrite one shared set of
fields.

## Implementation Boundary

The canonical helpers should live in `rara-config` and be reused by the runtime:

- resolve/create `~/.rara`
- derive/create a workspace data directory from a workspace root
- manage provider-scoped remembered config state

Other crates should not rebuild their own `.rara` path logic from
`current_dir().join(".rara")`.

## Validation

Minimum validation:

- config manager writes `config.json` under `~/.rara`
- workspace data helpers resolve under `~/.rara/workspaces/...`
- provider switching restores provider-scoped remembered settings
- runtime surfaces that previously created project-local `.rara` now use the
  shared helpers
