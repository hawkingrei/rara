# 2026-04-30 Codex And Claude Prompt Guidance

RARA's default system prompt now carries two complementary development contracts:

- Codex-style engineering discipline for small reviewable changes, large-change decomposition, local codebase alignment, focused abstractions, API stability, and narrow validation.
- Claude-style edit safety for full-file reads before existing-file edits, stale-file recovery, and avoiding writes from snippets or summaries.

This complements the runtime file-read state enforcement in the edit tools. The prompt explains the expected behavior to the model, while the tools reject unsafe existing-file edits when the file was not fully read or changed after reading.

The runtime context now follows Codex's structured environment-context shape for local path awareness:

- inject `cwd` as a full local path instead of only a workspace label;
- include the active shell and git branch in the same block;
- tell the model to use tool working-directory fields instead of changing directories with `cd` when possible.
