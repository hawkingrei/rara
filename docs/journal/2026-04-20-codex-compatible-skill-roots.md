# Codex-Compatible Skill Roots

## Summary

Expanded `rara-skills` from the original `.rara/skills`-only loader into a Codex-compatible skill root resolver.

This phase keeps the lightweight local loader, but aligns discovery rules more closely with Codex before attempting a heavier direct dependency on `codex-core-skills`.

## Implemented

- `rara-skills` now searches:
  - `~/.rara/skills`
  - `~/.agents/skills`
  - `~/.codex/skills`
  - repo-scoped `.agents/skills` directories from the project root down to the current working directory
  - local `./.rara/skills`
- Existing support for:
  - flat legacy `*.md` skills
  - directory skills with `SKILL.md`
  remains intact.

## Design Notes

- This mirrors the most valuable part of Codex's skill loader first: root discovery and repo/user skill visibility.
- It intentionally does **not** pull in `codex-core-skills` yet because that crate currently brings a much larger config/protocol/exec dependency surface than RARA needs for this step.
- The current `rara-skills` crate remains the adaptation boundary.

## Validation

- `cargo test -p rara-skills -- --nocapture`

## Follow-up

- Keep moving more skill runtime behavior behind `rara-skills`.
- Revisit direct reuse of `codex-core-skills` once a smaller stable dependency boundary is acceptable.
