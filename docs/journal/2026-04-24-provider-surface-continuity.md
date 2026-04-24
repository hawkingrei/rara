# 2026-04-24 · Provider Surface Continuity

## Summary

RARA's provider/model switching path now treats backend rebuilds as session-stable hot-swaps
instead of transcript/session resets, and `/status` now exposes basic provenance for the active
provider-scoped model surface.

## What Changed

- Rebuild completion now merges the new backend/agent runtime with the previous session state
  instead of replacing the whole session shell.
- In-session provider/model switching now preserves:
  - session id;
  - existing history/transcript continuity;
  - current plan state;
  - pending interactions;
  - compaction state.
- `/status` now reports source labels for:
  - `model`;
  - `base_url`;
  - `reasoning_summary`;
  - `revision`;
  - `reasoning_effort`;
  - `api_key`.
- The first provenance labels are:
  - `provider_state`;
  - `legacy_global`;
  - `built_in_default`;
  - `unset`.
- TUI now keeps a small Codex auth surface in app state:
  - `codex_auth_mode = chatgpt | api_key | -`
  - `codex_endpoint_kind = chatgpt_codex | openai_api | unknown | -`
  so `/status` can explain the active Codex connection shape without reading auth storage
  ad hoc during rendering.

## Why

Phase 2 is about making model/provider behavior explainable and safely switchable.

Without this change, applying a new model/provider looked more like restarting the session than
re-pointing the backend for the same session, which made TUI continuity and session identity
harder to reason about.

## Remaining Follow-Up

- Extend provider-surface provenance beyond the first status labels so `/status` can eventually
  distinguish user overrides, workspace overrides, and provider defaults more precisely.
- Align Codex endpoint selection with auth mode so ChatGPT/Codex login and OpenAI API key sessions
  cannot silently share the wrong endpoint.
