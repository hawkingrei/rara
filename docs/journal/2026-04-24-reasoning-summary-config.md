# 2026-04-24 Reasoning Summary Config Migration

## Summary

RARA now has a provider-scoped `reasoning_summary` config field with a default
of `auto`.

This is the first step away from the older generic `thinking: bool` toggle for
hosted-provider/runtime surfaces.

## What Changed

- Added `reasoning_summary` to provider-scoped config state in `rara-config`.
- Default config now sets `reasoning_summary = "auto"`.
- Legacy configs that only had `thinking: true/false` are migrated on load:
  - `true` -> `auto`
  - `false` -> `none`
- `/status` now reports the effective `reasoning_summary` value explicitly.

## Scope Boundary

This checkpoint only establishes the configuration contract and migration.

It does not yet:

- expose a dedicated picker/editor for reasoning-summary modes;
- wire reasoning-summary requests into each backend;
- remove provider-specific `thinking` behavior used by Ollama runtime paths.

## Validation

- `cargo test -p rara-config -- --nocapture`
- `cargo check`

## Follow-Up

- Surface `reasoning_summary` in provider/model switching flows.
- Add a dedicated runtime/UI contract for reasoning summaries in transcript and
  status surfaces.
- Finish retiring the old generic `thinking` toggle from provider-agnostic
  config surfaces.
