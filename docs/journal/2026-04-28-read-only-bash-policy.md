# Read-Only Bash Approval Policy

## Context

RARA already supports a `bash_approval=suggestion` mode, but every bash call paused for user approval. Claude Code treats clearly read-only shell commands as concurrency-safe and permission-safe, while keeping write, network, background, and complex shell commands under the normal permission flow.

## Implementation Checkpoint

- Added a conservative read-only classifier on `BashCommandInput`.
- Auto-allows read-only bash commands in suggestion mode.
- Added rules-file-backed command-prefix approvals, mirroring Codex-style exec policy amendments without opening all bash commands.
- Persists accepted prefixes in `rules/default.rules` under the user RARA config directory.
- Keeps commands with network access, background execution, environment overrides, output redirection, unparseable shell syntax, or known write-capable subcommands under approval.
- Covered the policy with focused bash classifier tests and agent-loop approval tests.

## Follow-Up

- The classifier intentionally covers the common Claude-style read-only surface first: `git`, `rg`, `grep`, `find`, `fd`, `sed`, basic file inspection commands, `docker` read-only inspection, and `pyright`.
- A future hardening pass can replace the small local tokenizer with a stricter shell parser if RARA needs broader compound-shell support.
