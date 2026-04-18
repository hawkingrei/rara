# 2026-04-18 Sandbox Hardening

## Summary

Continued the command-execution hardening work in `src/sandbox.rs` and `src/tools/bash.rs` by moving Linux bubblewrap wrapping away from a blanket `--ro-bind / /` mount and toward a smaller runtime filesystem view.

## What changed

- unsupported platforms now fail closed instead of silently falling back to unsandboxed execution;
- macOS sandbox profiles are generated per command and cleaned up after execution;
- Linux bubblewrap wrapping now starts from:
  - `--tmpfs /`
  - `--dev /dev`
  - `--proc /proc`
  - `--tmpfs /tmp`
- Linux then layers only the required read-only runtime roots:
  - `/bin`
  - `/sbin`
  - `/usr`
  - `/etc`
  - `/lib`
  - `/lib64`
  - `/nix/store`
  - `/run/current-system/sw`
- the active workspace path is rebound as the writable root and restored as the command cwd;
- `bash` now supports both the legacy shell-string form and the newer structured command form:
  - `command`
  - `program`
  - `args`
  - `cwd`
  - `env`
  - `allow_net`
- approval/session restore/state persistence now preserve the structured bash payload instead of flattening everything down to a single command string.

## Validation

- `cargo test sandbox::tests -- --nocapture`
- `cargo test tools::bash::tests -- --nocapture`
- `cargo test agent::tests -- --nocapture`
- `cargo test tui::state::tests -- --nocapture`
- `cargo test tui::command::tests -- --nocapture`
- `cargo test state_db::tests -- --nocapture --test-threads=1`
- `cargo check`

## Follow-up

- finish migrating callers away from the legacy `command` field so shell-string execution is no longer the default path;
- continue tightening command/path validation now that the structured bash payload exists;
- consider adopting a more complete Codex-style Linux sandbox policy with explicit unreadable carveouts and writable subpath protection if RARA needs broader filesystem policies later.
