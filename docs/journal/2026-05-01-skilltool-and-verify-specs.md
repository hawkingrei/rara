# SkillTool And Verify Specs

## Summary

Added specs for:

- a Claude-style `verify` skill and `verifier-*` project verifier skills;
- a Codex/Claude-aligned `SkillTool` contract for discovery, invocation, metadata, precedence,
  budgets, and observability.

## Background

RARA already has a basic skill loader and `skill` tool. It supports home/repo/cwd skill roots and can
list or invoke markdown skills. Compared with Codex and Claude Code, the missing parts are structured
metadata, source scopes, parse errors, overridden-skill visibility, invocation tracking, and
budget-aware prompt rendering.

## Scope

- Documented `verify` as a file-backed skill, not a runtime verifier tool.
- Documented `verifier-*` skills as project-specific evidence-capture protocols.
- Documented how `SkillTool` should evolve from `list`/`invoke` into a source-aware and
  budget-aware skill surface.
- Kept this checkpoint documentation-only.

## Key Decisions

- Learn `verify` from Claude Code: use ordinary skills and project-local verifier skills first.
- Learn skill loading and presentation from Codex and Claude where it matches RARA roots: explicit
  available-skill summaries, source-aware metadata, full body injection only after invocation, and
  no invented skill names.
- Do not adopt `~/.codex/skills` as a default compatibility root.
- Do not support `/commit` as a RARA built-in command.

## Validation

- Documentation-only checkpoint; no runtime tests were run.

## Follow-ups

- Add `.agents/skills/verify/SKILL.md`.
- Extend `rara-skills` with frontmatter metadata, scope, errors, and overridden-skill reporting.
- Keep skill metadata structured in the runtime, following Codex's model-visible
  `name`, `description`, and path list. Markdown headings are a legacy fallback
  parsed by the loader, not something the model should infer from raw Markdown.
- Extend `skill` tool responses with source metadata and optional args.
- Surface skill roots, overrides, parse errors, and invoked skills in `/context` or `/status`.
