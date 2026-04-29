# Codex-Style Active Progress Segments

## Context

Codex keeps reasoning and execution progress as transcript history cells instead of continuously overwriting one shared status area. Reasoning deltas are accumulated for the current block, finalized into a reasoning summary cell, and command execution is shown through separate execution cells.

## Change

RARA now keeps an ordered live event log for active TUI progress updates across Thinking, Exploring, Planning, and Running. Adjacent events with the same role are grouped and compacted, while interleaved events stay flat in their original order.

This makes long turns render progress like `Thinking -> Running -> Thinking -> Running` as separate transcript segments instead of merging all updates into one Thinking or Running area.

## Validation

- Added a focused active turn rendering test for interleaved Thinking and Running events.
- Updated the existing active plan snapshot to preserve progress event order.
