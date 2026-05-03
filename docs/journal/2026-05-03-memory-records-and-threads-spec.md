# Memory Records and Threads Spec Checkpoint

## Date
2026-05-03

## Context

Reviewed an external memory and thread architecture. Two gaps: no structured
memory records, no thread abstraction above SessionManager.

## Decisions

### Memory Records

- `MemoryRecord`: durable unit with title, content, labels (Insight/Decision/
  Fact/Procedure/Experience), importance (0.1–1.0), source provenance.
- `MemoryStore`: LanceDB backend replacing mock VectorDB.
- Search ranking = importance × cosine_similarity.
- Six cross-industry design laws adopted as correctness baseline.

### Threads

- `Thread`: durable conversation record with messages, source, pin.
- Save triggers: stop hook, session end, /save, /save-handoff.
- Conversation Markdown format for import/export.
- `MemoryDistiller`: Thread → 2-8 MemoryRecords.

### RARA Positioning

- **Extraction**: Automation-first (novice users), human override available.
- **Storage**: LanceDB structured, `memory.md` flat-file.
- **Injection**: Zero-call human, budgeted vector AI.
- **Forgetting**: Discrete with importance gating; UserCreated exempt.
- **Architecture**: Core built-in now, plugin deferred.

## Implementation Order

```
C.1a: MemoryRecord + MemoryStore (LanceDB, replace mock VectorDB)
C.1b: ThreadStore upgrade (save/list/get/export)
C.2:  Embedding backend + Thread → Memory distillation
C.3:  Crash-safe SessionManager + cross-session search + import
```

## Follow-Up

- Update `docs/todo.md` Phase C.
- Implement `src/memory_record.rs` and `src/memory_store.rs`.
