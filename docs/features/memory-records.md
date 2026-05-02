# Memory Records and Storage

## Problem

RARA needs structured, agent-authored memory. The mock `VectorDB` returns empty
results, so retrieval is limited to tool-result extraction from conversation
history. Without real memory records, each session starts fresh.

## Scope

`MemoryRecord` is the durable, independently meaningful unit of memory — one
decision, insight, fact, procedure, or experience. `MemoryStore` replaces the
mock `VectorDB` with a real LanceDB backend.

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
| Storage | LanceDB structured, `memory.md` stays flat-file |
| Injection | Zero-call for human sources, budgeted vector retrieval for AI |
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

## MemoryStore API

- `insert(record) -> Uuid` — persist with auto-embedding
- `search(query, labels?, min_importance, limit) -> Vec<(MemoryRecord, f32)>`
- `get(id) -> Option<MemoryRecord>`
- `delete(id) -> ()`

Storage: `~/.rara/memories/` (LanceDB).

## Integration

| Component | Integration |
|-----------|-------------|
| `create_memory` tool | Agent persists records with auto-labels |
| `retrieve_experience` | Searches MemoryStore instead of conversation history |
| `MemorySelection` | `vector_memory_candidate` becomes `selectable: true` |
| `MemoryDistiller` | Thread → MemoryRecords with auto-labels + importance |

## Migration

1. Add `MemoryStore` alongside mock `VectorDB`.
2. Wire `memory_selection` and tools to `MemoryStore`.
3. Deprecate `VectorDB`.
4. Remove `VectorDB`.

## Source Journals

- 2026-05-03-memory-records-and-threads-spec.md
