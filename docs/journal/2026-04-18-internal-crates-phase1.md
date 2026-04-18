# Internal Crates Phase 1

## Summary

Started the Codex-style workspace split for RARA without changing the top-level binary entrypoint.

This phase extracted the most stable boundaries first:

- `rara-config`
- `rara-instructions`
- `rara-skills`

The root crate now depends on these internal crates through workspace path dependencies and keeps thin compatibility shims for the old module paths.

## Implemented

### Workspace conversion

- Added a workspace root to `Cargo.toml`.
- Kept the existing `rara` binary crate as the orchestration layer.
- Moved shared crate metadata and shared dependencies into workspace-level declarations.

### `rara-config`

- Extracted config types and config persistence logic into `crates/config`.
- Re-exported the crate back through `src/config.rs` so existing call sites could remain stable during the first extraction phase.

### `rara-instructions`

- Extracted prompt runtime and workspace instruction discovery into `crates/instructions`.
- Re-exported them through `src/prompt.rs` and `src/workspace.rs`.
- Upgraded instruction discovery so project instruction files are no longer root-only:
  - the resolver now walks from the current working directory up to the workspace root;
  - nested `AGENTS.md` files can participate in prompt assembly;
  - when the current process working directory is outside the workspace, discovery falls back to the workspace root.

### `rara-skills`

- Extracted skill loading into `crates/skills`.
- Added support for both:
  - legacy flat `*.md` skills;
  - Codex-style directory skills with `SKILL.md`.
- Updated the `skill` tool to consume the new crate API instead of reaching into raw prompt fields directly.

## Validation

- `cargo test -p rara-skills -- --nocapture`
- `cargo test -p rara-instructions -- --nocapture`
- `cargo check`

## Follow-up

Phase 1 intentionally kept the root crate as a compatibility shell.

Next useful extractions:

- move more skill runtime behavior behind `rara-skills`;
- keep improving instruction resolution rules to better match Codex/Claude expectations;
- extract additional stable crates only after the dependency graph is clearer (`core`, `tui`, `llm`, or `state` should not be split all at once).
