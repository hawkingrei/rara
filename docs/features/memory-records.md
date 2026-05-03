# Memory Records and Storage

## Problem

RARA needs structured, agent-authored memory. The mock `VectorDB` returns empty
results, so retrieval is limited to tool-result extraction from conversation
history. Without real memory records, each session starts fresh.

## Scope

`MemoryRecord` is the durable, independently meaningful unit of memory — one
decision, insight, fact, procedure, or experience. The storage path uses
LanceDB as a unified local memory index: raw text, metadata, full-text search,
and vector search live in one table, while context assembly still goes through
`MemorySelection`.

This spec describes the target product contract. The first implementation
slices provide the LanceDB-backed index, retrieval tools, and a runtime
`MemoryStore` facade; update/delete, filtering, distillation, and direct
`MemorySelection` integration remain follow-up work.

## Six Design Laws (Cross-Industry Consensus)

### Law 1: Non-Derivable Principle
Don't persist what can be retrieved live. Stale memory > no memory.

### Law 2: Human Memory Priority
`memory.md` unconditional; AI memories compete for budget.

### Law 3: Multi-Layer Isolation
User scope (`~/.rara/memories/`), project scope (`memory.md`), session scope.

### Law 4: Path as Primary Signal
`MemoryStore::search` accepts `scope_path`; local results rank above global.

### Law 5: Human Memories Immune to Forgetting
`UserCreated` records exempt from automatic cleanup.

### Law 6: Negative Space First
`create_memory` prompt includes what-NOT-to-save section.

## RARA Positioning (Five Axes)

| Axis | Position |
|------|----------|
| Extraction | Automation-first (novice users), human override available |
| Storage | LanceDB structured with FTS + vector columns, `memory.md` stays flat-file |
| Injection | Zero-call for human sources, budgeted hybrid retrieval for AI |
| Forgetting | Discrete with importance gating; `UserCreated` exempt |
| Architecture | Core built-in now, plugin surface deferred |

## Data Model

```rust
pub struct MemoryRecord {
    pub id: Uuid,
    pub title: String,
    pub content: String,       // Markdown
    pub labels: Vec<MemoryLabel>,
    pub importance: f32,       // 0.1–1.0
    pub source: MemorySource,
    pub created_at: DateTime,
    pub embedding: Option<Vec<f32>>,
}

pub enum MemoryLabel { Insight, Decision, Fact, Procedure, Experience }
pub enum MemorySource { AgentTurn, UserCreated, ThreadDistill, FileImport }
```

## Product Contract

RARA memory should eventually behave like a durable knowledge object, not just a
retrieval row.

Each memory owns:

- `title`: short human-readable summary.
- `content`: Markdown body containing the durable knowledge.
- `labels`: reusable classification tags for filtering and routing.
- `importance`: ranking signal from `0.1` to `1.0`.
- `created_at` and `updated_at`: temporal search and evolution metadata.
- `source`: provenance such as user-created, agent turn, thread distillation,
  file import, or protocol write.
- `scope`: user, workspace, project, thread, or session visibility boundary.
- `embedding`: optional vector representation for semantic retrieval.

Standard labels:

| Label | Intended Use |
|-------|--------------|
| `insight` | Durable lessons and realizations. |
| `decision` | Choices with rationale and trade-offs. |
| `fact` | Reference information and stable data points. |
| `procedure` | Repeatable workflows and steps. |
| `experience` | Events, conversations, outcomes, and incident notes. |

Importance scale:

| Range | Meaning |
|-------|---------|
| `0.8..=1.0` | Critical architectural decisions, incidents, or high-value procedures. |
| `0.5..0.8` | Useful project learnings and ordinary decisions. |
| `0.1..0.5` | Background reference and low-priority notes. |

## Product Capability Matrix

| Capability | Target Behavior | Current Runtime Status |
|------------|-----------------|------------------------|
| Memory record anatomy | Title, Markdown content, labels, importance, timestamps, source, scope, embedding. | Partial. `MemoryRecord` exists at the runtime facade; LanceDB rows still store the compact index shape. |
| Memory creation | Agent or user creates a durable `MemoryRecord`; title, labels, and importance can be generated or explicit. | Partial. `remember_experience` is now a compatibility adapter over `MemoryStore::insert`. |
| Memory search | Hybrid semantic + keyword search with metadata filters and explainable scores. | Partial. LanceDB vector, FTS, and hybrid helpers exist behind the current `VectorDB` façade. |
| Memory update | Existing records can be edited without creating duplicates. | Not implemented as a public memory capability. |
| Memory delete | User or control-plane request can delete records with audit-safe semantics. | Not implemented as a public memory capability. |
| Thread distillation | Thread history can be distilled into 2-8 durable memory records. | Spec only. |
| Context injection | Ranked memory candidates pass through `MemorySelection` before prompt injection. | Partial. `MemorySelection` exists, but LanceDB search results are not yet direct ranked candidates. |
| Graph retrieval | Entity and relationship traversal complements vector recall. | Future work. |
| Working memory | Daily or session briefing summarizes recent and important memories. | Future work. |
| MCP / ACP / Wire memory APIs | Protocol clients can query and mutate memory through the runtime control plane. | Future work over the `MemoryStore` boundary. |

## Memories vs Threads

Threads preserve conversation history. Memories preserve durable knowledge.

RARA should not treat every thread message as a memory. Raw turn checkpoints are
useful for crash recovery, browsing, and future distillation, but a
`MemoryRecord` must be independently useful without the full thread.

The runtime should therefore keep three separate objects:

- `Thread`: full or summarized conversation record.
- `MemoryRecord`: distilled durable knowledge unit.
- `MemorySelectionItem`: per-turn context candidate selected from prompt files,
  thread recall, memory retrieval, or future protocol sources.

This separation prevents a storage backend from bypassing context policy:
LanceDB may store and retrieve candidates, but `MemorySelection` decides whether
they enter the model context.

## Session Shards and Global Memory

Session-level history should stay local to the session. The target storage
shape is one append-oriented shard per session, so active agent turns can write
without contending on the global memory index. A shard may be a LanceDB table,
state-db artifact, or another append-friendly file format, but it must be
addressed by session id and remain cheap to restore, compact, or delete.

Global memory has a different contract. It should contain durable
`MemoryRecord`s promoted from explicit memory tools, user actions, protocol
writes, and periodic distillation of session shards. Raw turn checkpoints should
not be written to the global memory index by default. Promotion into global
memory should be scheduled or batched so cross-session recall stays useful
without turning every active turn into a global write.

The current `conversations` LanceDB table is an interim checkpoint path. It is
not the final session-storage contract.

## MemoryStore API

- `insert(record) -> MemoryRecord` — persist with auto-embedding
- `search(query, labels?, min_importance, scope?, limit) -> Vec<(MemoryRecord, f32)>`
- `update(id, patch) -> MemoryRecord`
- `get(id) -> Option<MemoryRecord>`
- `delete(id) -> ()`
- `list_labels(scope?) -> Vec<(MemoryLabel, usize)>`

Storage: `~/.rara/lancedb/` (LanceDB).

Local write coordination: RARA uses an adjacent advisory lock file
(`~/.rara/lancedb.lock`) for LanceDB mutations. Reads remain lock-free, while
table creation, index creation, upsert, and future update/delete paths must
serialize through this lock so multiple RARA processes can share the same
workspace memory directory without racing initialization or commits.

## LanceDB Index Contract

The first runtime slice keeps the existing `VectorDB` façade but backs it with
LanceDB instead of a mock.

The current table shape is intentionally small:

- `id`: stable memory id.
- `session_id`: source session or scope.
- `turn_index`: source turn index or deterministic tool-write id.
- `text`: raw memory text; indexed with LanceDB FTS / BM25.
- `vector`: embedding column; searched with LanceDB vector search.

Search modes:

- vector search via `search_with_metadata`;
- FTS search via `full_text_search_with_metadata`;
- hybrid search via `hybrid_search_with_metadata`, combining LanceDB FTS and
  vector search while returning debug scores (`fts_score`, `vector_distance`).

Search must not create tables. Only write paths create tables using the real
embedding dimension. This avoids fixing an empty table to a guessed vector
dimension before the first memory write.

## Integration

| Component | Integration |
|-----------|-------------|
| `remember_experience` | Current compatibility tool; should become a thin adapter over `MemoryStore::insert` |
| `memory_add` / `memory_update` / `memory_delete` | Future protocol-safe memory mutation tools |
| `retrieve_experience` | Current compatibility retrieval tool; should delegate to `MemoryStore::search` |
| `memory_search` | Future protocol-safe search tool with labels, scope, and importance filters |
| `MemorySelection` | `vector_memory_candidate` becomes `selectable: true` |
| `MemoryDistiller` | Thread → MemoryRecords with auto-labels + importance |

Current implementation checkpoint:

- `MemoryStore` owns the memory-domain runtime facade over the LanceDB-backed
  index.
- `remember_experience` writes through `MemoryStore::insert`.
- `retrieve_experience` searches through `MemoryStore::search` and returns both
  `relevant_experiences` and memory diagnostics.
- Agent turn checkpoints continue writing to the `conversations` table.
- `MemorySelection` is not yet switched to direct ranked memory candidates;
  retrieved memories still enter through retrieval-tool results.

## Migration

1. Replace mock `VectorDB` with LanceDB-backed FTS/vector/hybrid index.
2. Wire retrieval tools to the LanceDB-backed index.
3. Add the `MemoryRecord` domain model with title, labels, importance, source,
   scope, and timestamps. Done.
4. Add the `MemoryStore` domain façade over `VectorDB`. Done.
5. Make `remember_experience` and `retrieve_experience` compatibility adapters
   over `MemoryStore`. Done.
6. Wire `MemorySelection` to ranked memory candidates.
7. Add update/delete/list-label control-plane scaffolding without exposing
   storage internals.
8. Add thread distillation into `MemoryRecord`.
9. Move raw session checkpoints out of the global `conversations` LanceDB table
   into per-session append shards.
10. Add periodic promotion from session shards into global `MemoryRecord`s.
11. Deprecate `VectorDB`.
12. Remove `VectorDB`.

## Source Journals

- 2026-05-03-memory-records-and-threads-spec.md
- 2026-05-03-lancedb-memory-index.md
