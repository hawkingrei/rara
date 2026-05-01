# Git Conflict Prompt Guidance

## Context

RARA already had prompt guidance for factual verification, edit-tool safety, command-output
inspection, and reviewable implementation workflow. Merge-conflict handling still relied on those
general rules, which left too much room for models to choose one side by habit or claim resolution
without checking for remaining markers.

## Implementation Checkpoint

- Added a built-in `Git Conflict Resolution` prompt section to the default instruction family.
- The guidance is source-grounded rather than attributed to a fixed upstream protocol:
  - inspect git state and the conflicted file before editing;
  - preserve complementary changes and remove obsolete code only with inspected evidence;
  - prefer structured edits or `apply_patch` over whole-file rewrites;
  - scan for remaining conflict markers after resolving;
  - run the narrowest relevant formatter, test, build, or check before claiming completion.
- Updated the prompt runtime spec to record Git conflict handling as part of the built-in
  engineering workflow guidance.
- Added a focused prompt test to lock the section into the default prompt.

## Validation

- `cargo test -p rara-instructions git_conflict_resolution -- --nocapture`
