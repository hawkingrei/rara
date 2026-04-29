# 2026-04-29 Codex-Style Skill Context

## Summary

- Added Codex-style skill metadata injection to the runtime system prompt.
- RARA now lists available skill names, descriptions, and `SKILL.md` paths in a
  dedicated prompt section when skills are loaded.
- Skill bodies remain progressive-disclosure content: the model should use the
  `skill` tool to invoke a matching skill before following its full
  instructions.

## Notes

- This keeps the prompt lightweight and avoids injecting every skill body into
  the base context.
- Skill metadata is rendered as escaped structured data and explicitly marked as
  untrusted labels so repository-provided descriptions are not treated as
  instructions.
- The runtime still uses the existing `SkillManager` discovery roots and the
  existing `skill` tool for full instruction retrieval.
