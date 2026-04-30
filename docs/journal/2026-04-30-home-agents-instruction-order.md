# Home AGENTS Instruction Order

## Checkpoint

RARA now includes a user-level `~/.rara/AGENTS.md` prompt source when the
workspace runtime data directory is under the normal `~/.rara/workspaces/...`
layout.

The stable instruction source order is:

1. `~/.rara/AGENTS.md`
2. project instruction files discovered from the workspace root toward the
   current focus directory
3. local workspace memory and configured runtime prompt sources

This keeps the user-level instruction set as a stable prefix while preserving
the existing root-to-focus ordering for repository instructions.

## Validation

- Added a focused workspace discovery test that asserts user, root, and nested
  instruction sources are returned in prefix-stable order.
