# Threads

## Problem

Sessions saved as flat JSON by `SessionManager` — no browse, search, pin,
export, or distillation surface. Cannot recall past conversations or distill
durable memories.

## Scope

`Thread` is a durable conversation record. Threads are the raw material from
which `MemoryRecord`s are distilled.

Threads are not memories. A thread can be long, contextual, and useful only when
read with its surrounding messages. A memory must stand on its own as one
durable decision, insight, fact, procedure, or experience.

## Data Model

```rust
pub struct Thread {
    pub id: Uuid,
    pub title: String,
    pub source: ThreadSource,
    pub created_at: DateTime,
    pub updated_at: DateTime,
    pub messages: Vec<ThreadMessage>,
    pub distilled_memory_ids: Vec<Uuid>,
    pub pinned: bool,
}

pub enum ThreadSource {
    RaraSession { session_id: String },
    CodexImport { source_session_id: String },
    FileImport { path: String, format: ImportFormat },
    ManualCapture,
}
```

## Lifecycle

```
Session Active
  ├─ stop hook → save_thread(auto)
  ├─ /save → save_thread(manual)
  ├─ /save-handoff → summary only
  ▼
Thread stored (LanceDB)
  ├─ /distill → MemoryRecords
  ├─ /export → Conversation Markdown
  ├─ /threads → browse, search, pin
```

## Save Triggers

| Trigger | Behavior |
|---------|----------|
| Stop hook | Auto-save after each response |
| Session end | Final turn batch on clean exit |
| `/save` | Explicit full-session save |
| `/save-handoff` | Concise continuation summary (not full thread) |

## ThreadStore API (LanceDB)

- `save(thread) -> Uuid`
- `get(id) -> Option<Thread>`
- `list(pinned_only?, source_filter?, limit, offset) -> Vec<ThreadSummary>`
- `search(query, limit) -> Vec<ThreadSummary>`
- `search_messages(thread_id, query) -> Vec<ThreadMessage>`
- `pin(id, pinned) -> ()`
- `delete(id) -> ()`

Storage: `~/.rara/threads/`.

## Conversation Markdown Format

```markdown
---
title: Python Async Patterns
source: rara
date: 2026-05-03
---

## User
How does async/await work?

## Assistant
Python's async/await lets you write concurrent code...

## Tool
read_file path="src/main.rs"
```

Headers: `## User`, `## Assistant`, `## System`, `## Tool`, `## Tool Result`.
Optional YAML frontmatter with `title`, `source`, `date`.

## Import Paths

| Source | Format |
|--------|--------|
| Conversation Markdown | `.md` with `## User`/`## Assistant` |
| ChatGPT | `chat.html` |
| Claude | `conversations.json` |
| DeepSeek | `deepseek_conversations.json` |

## Thread Distillation

`MemoryDistiller`: Thread → 2-8 MemoryRecords.
- Read full thread messages.
- Identify independently meaningful units.
- Auto-generate title, labels, importance.
- For >50 messages: chunked Smart Background Distillation.

Distillation rules:

- Do not persist every message as a memory.
- Prefer fewer, independently useful records over many thin summaries.
- Preserve source provenance: thread id, message span, session id, and
  workspace scope.
- Deduplicate against existing memories before insert.
- Use `MemorySelection` for any immediate context carry-over; do not inject
  distilled memories directly into prompts.

## Source Journals

- 2026-05-03-memory-records-and-threads-spec.md
