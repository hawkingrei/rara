# Memory Record Persistence and Thread Export Checkpoint

## Summary

This checkpoint advances the backend-only Memory & Retrieval stream without
touching the TUI.

RARA now keeps `MemoryRecord` as a durable domain object instead of reconstructing
records entirely from compact LanceDB rows. LanceDB remains the recall index for
FTS/vector/hybrid search, while `~/.rara/memories/records.json` stores the full
record payload: title, Markdown content, labels, importance, source, scope,
timestamps, `session_id`, `thread_id`, and source turn span.

## Design Notes

The storage split is intentional:

- LanceDB owns recall performance and score diagnostics.
- `MemoryStore` owns the memory product contract.
- `MemorySelection` remains the future prompt-injection policy boundary.

This avoids coupling public memory records to the current LanceDB table schema.
Existing compact rows can still be searched, and new records can add provenance
without forcing a table migration.

Both storage paths use local advisory locks. LanceDB writes keep the existing
index lock, and domain record writes use an adjacent lock next to
`records.json`.

## Runtime Wiring

- `MemoryStore::insert` writes the LanceDB index row and then persists the full
  domain record.
- `MemoryStore::search` runs hybrid LanceDB retrieval and rehydrates full records
  by id when sidecar records exist.
- `MemoryStore::get` reads a durable record by id.
- `retrieve_session_context` now searches the `conversations` LanceDB table
  instead of returning a stub response.
- `ThreadStore::export_thread_markdown` renders a portable markdown transcript.
- `ThreadStore::distill_thread_summary` persists one thread-linked
  `ThreadDistill` memory record through `MemoryStore`.

## Source References

Codex keeps thread metadata and memory-stage outputs separate, with thread rows
feeding later memory jobs. Claude Code uses memory hooks and background
extraction after main-thread turns, while keeping direct memory file loading
separate from the live prompt assembly path.

RARA follows the same separation at this stage: thread lifecycle state,
retrieval index rows, durable memory records, and prompt selection policy remain
separate components.

## Follow-Up

- Add update/delete/list-label operations to `MemoryStore`.
- Promote search hits into ranked `MemorySelection` candidates.
- Replace summary-only distillation with LLM-assisted 2-8 memory extraction and
  duplicate detection.
- Move raw conversation checkpoints from the global `conversations` table into
  per-session append shards.
