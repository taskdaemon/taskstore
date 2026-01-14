# TaskStore Documentation

TaskStore is a Rust library providing durable, git-integrated persistent storage using the SQLite+JSONL+Git pattern (Bead Store pattern from Gas Town).

## Documentation Structure

### Core Design Documents

| Document | Description |
|----------|-------------|
| [storage-architecture.md](./storage-architecture.md) | Bead Store pattern: what goes where and why |
| [taskstore-design.md](./taskstore-design.md) | Full API design, schema, implementation plan |
| [implementation-guide.md](./implementation-guide.md) | Practical implementation details, patterns, examples |

### Read This First

If you're implementing TaskStore, read in this order:

1. **[storage-architecture.md](./storage-architecture.md)** - Understand the Bead Store pattern
2. **[taskstore-design.md](./taskstore-design.md)** - Full design (API, schema, alternatives)
3. **[implementation-guide.md](./implementation-guide.md)** - How to actually build it

### Quick Reference

**What is TaskStore?**
- Library + binary for persistent state management
- SQLite (fast queries) + JSONL (git-friendly) + Custom merge driver
- Stores PRDs, Task Specs, Executions, Dependencies, Workflows

**Key Features:**
- Append-only JSONL (source of truth)
- Rebuildable SQLite cache
- Custom git merge driver for conflict resolution
- Git hooks for automatic sync

**API Overview:**
```rust
use taskstore::{Store, Prd, TaskSpec, Execution};

let mut store = Store::open(".taskstore")?;

// Create PRD
let prd_id = store.create_prd(prd)?;

// Query
let prds = store.list_prds(Some(PrdStatus::Active))?;
let prd = store.get_prd(&prd_id)?;

// Sync (rebuild SQLite from JSONL)
store.sync()?;
```

**CLI Usage:**
```bash
# List operations
taskstore list-prds --status ready
taskstore list-executions --status running

# Maintenance
taskstore sync
taskstore compact

# Git integration
taskstore install-hooks
```

## Relationship to TaskDaemon

TaskStore is consumed by [TaskDaemon](../../taskdaemon/docs/taskdaemon-design.md) as a library:

```
taskdaemon/
├── Cargo.toml              # Depends on: taskstore = { path = "../taskstore" }
└── src/
    └── lib.rs              # use taskstore::{Store, Prd, TaskSpec};
```

TaskDaemon orchestrates concurrent agentic loops that read/write state via TaskStore.

## Status

All design documents are complete (5/5 Rule of Five review passes) and ready for implementation.
