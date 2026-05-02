# Kimi Provider Preset

## Summary

Kimi is now treated as a first-class OpenAI-compatible endpoint profile instead
of relying on users to fill every custom endpoint field manually.

## Implementation Notes

- Updated the built-in Kimi model default to `kimi-k2-0905-preview`.
- Kept Kimi on the existing OpenAI-compatible backend path.
- Added `MOONSHOT_API_KEY` and `KIMI_API_KEY` as runtime-only credential sources
  for active Kimi profiles.
- Exposed environment-sourced credentials through `/status` without serializing
  them into `config.json`.
- Refreshed the OpenAI-compatible model picker examples and Kimi preset model.
- Parsed OpenAI-compatible usage cache counters into cumulative runtime state
  and surfaced `cache_hit_rate = hit / (hit + miss)` in `/status` and the footer
  when providers return cache usage data.

## Validation

Focused configuration tests cover:

- Kimi profile default base URL and model selection.
- Runtime-only `MOONSHOT_API_KEY` use.
- Explicit profile API keys overriding environment defaults.
- OpenAI-compatible usage parsing for cache hit/miss tokens.
