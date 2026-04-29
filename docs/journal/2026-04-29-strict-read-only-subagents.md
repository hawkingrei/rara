# Strict Read-Only Subagents

## Context

RARA already restricts Explore and Plan subagents at the tool-manager layer: they only receive repository inspection tools and do not receive bash, PTY, editing, patching, or recursive agent tools.

Claude Code's Explore and Plan prompts also make the read-only contract explicit. They forbid file creation, file mutation, temporary files, redirects, heredocs, and other shell-based write workarounds.

## Change

Explore and Plan subagent prompts now include a shared strict read-only contract:

- no creating, modifying, deleting, moving, or copying files;
- no temporary files, including under `/tmp`;
- no shell commands, scripts, redirection, heredocs, or other state-changing workarounds;
- only `read_file`, `list_files`, `glob`, and `grep` are valid inspection tools;
- mutation requests must be answered with the limitation and an evidence-backed finding or plan.

General worker subagents are unchanged, except they still do not receive recursive agent tools.

## Validation

Added prompt regression coverage for the strict read-only clauses and kept the existing tool-manager test that excludes mutating, PTY, bash, background task, and recursive agent tools from read-only subagents.

## Follow-Up Tool Workflow Prompting

The default system prompt now also includes a dedicated tool workflow discipline section. It makes the expected execution loop explicit:

- use high-signal tool calls instead of broad repetitive searches;
- diagnose exact tool errors and try narrow safe fallbacks before asking the user;
- handle truncation by narrowing reads or searches;
- keep long-running commands observable through background task or PTY list/status/stop tools;
- inspect real GitHub review/check/branch state before summarizing PR readiness;
- inspect git status before committing and do not rewrite history unless explicitly requested;
- finish code review and diagnosis tasks with evidence-backed conclusions instead of stopping after describing the next inspection step.
