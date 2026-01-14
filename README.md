# TaskStore

**Persistent state management with SQLite+JSONL+Git integration**

TaskStore is a Rust library and CLI for managing persistent state with a unique architecture that combines:
- **SQLite** for fast queries
- **JSONL** (JSON Lines) as the git-friendly source of truth
- **Git merge driver** for conflict-free collaboration

## Features

- **Dual storage**: SQLite for speed, JSONL for git compatibility
- **Three-way merge**: Custom git merge driver for JSONL files with timestamp-based conflict resolution
- **Automatic syncing**: Git hooks automatically rebuild SQLite from JSONL
- **Schema versioning**: Automatic migrations with version tracking
- **Append-only semantics**: JSONL files track full history, latest version wins
- **Type-safe API**: Full Rust type safety with serde serialization

## Installation

### Build from source

```bash
cargo build --release
sudo cp target/release/taskstore /usr/local/bin/
sudo cp target/release/taskstore-merge /usr/local/bin/
```

### Install git integration

```bash
taskstore install-hooks
```

This installs:
- Git merge driver for JSONL files
- Pre-commit, post-merge, post-rebase, pre-push, post-checkout hooks
- `.gitattributes` configuration for `*.jsonl` files

## Usage

### Initialize a store

```bash
taskstore sync
```

This creates:
- `.taskstore/` directory
- `store.db` SQLite database
- `.version` file for schema tracking
- `*.jsonl` files for each entity type

### List PRDs

```bash
# List all PRDs
taskstore list-prds

# Filter by status
taskstore list-prds --status active
```

### List task specifications

```bash
taskstore list-task-specs <PRD_ID>
```

### List executions

```bash
# List all executions
taskstore list-executions

# Filter by status
taskstore list-executions --status running
```

### Show detailed information

```bash
# Show PRD details
taskstore show prd <ID>

# Show task spec details
taskstore show ts <ID>

# Show execution details
taskstore show execution <ID>
```

### View statistics

```bash
taskstore stats
```

### Manual sync

```bash
taskstore sync
```

Rebuilds the SQLite database from JSONL files. This happens automatically via git hooks, but you can trigger it manually.

## Architecture

### Storage Pattern: Bead Store

TaskStore uses a "Bead Store" pattern where:

1. **JSONL is the source of truth**
   - Append-only log of all changes
   - Git-friendly plain text format
   - Each record has `id`, `created_at`, `updated_at` fields
   - Multiple versions of same record can exist

2. **SQLite is a derived cache**
   - Built from JSONL files
   - Keeps only latest version per ID
   - Optimized for fast queries with indexes
   - Can be regenerated at any time

3. **Write-through pattern**
   - All writes go to JSONL first
   - Then written to SQLite
   - If SQLite fails, JSONL still has the data

### Data Model

```
PRD (Product Requirements Document)
├── id: String
├── title: String
├── description: String
├── status: draft | ready | active | complete | cancelled
├── review_passes: u8
├── content: String (markdown)
└── timestamps: created_at, updated_at

TaskSpec (Task Specification)
├── id: String
├── prd_id: String
├── phase_name: String
├── description: String
├── status: pending | running | complete | failed
├── workflow_name: Option<String>
├── assigned_to: Option<String>
├── content: String
└── timestamps: created_at, updated_at

Execution (Loop Instance)
├── id: String
├── ts_id: String (task spec ID)
├── worktree_path: String
├── branch_name: String
├── status: running | paused | complete | failed | stopped
├── current_phase: Option<String>
├── iteration_count: u32
├── error_message: Option<String>
└── timestamps: started_at, updated_at, completed_at

Dependency (Coordination)
├── id: String
├── from_exec_id: String
├── to_exec_id: String
├── dependency_type: notify | query | share
├── payload: Option<String>
└── timestamps: created_at, resolved_at

Workflow (AWL Definition)
├── id: String
├── name: String
├── description: String
├── awl_code: String
└── timestamps: created_at, updated_at

RepoState (Sync Tracking)
├── repo_path: String (primary key)
├── last_synced_commit: String
└── updated_at: i64
```

### Git Integration

#### Custom Merge Driver

The `taskstore-merge` binary implements a three-way merge algorithm:

1. Parse ancestor, ours, and theirs JSONL files
2. Build maps of latest record per ID
3. For each ID, determine merge outcome:
   - **Added in one branch**: Use that version
   - **Deleted in one branch**: Keep deletion
   - **Modified in both**: Use newest `updated_at` timestamp
   - **Same timestamp**: Create conflict marker

Exit codes:
- `0` - Merge successful
- `1` - Conflicts require manual resolution
- `2` - Error occurred

#### Git Hooks

All hooks run `taskstore sync` to keep SQLite in sync with JSONL:

- **pre-commit**: Sync before committing
- **post-merge**: Sync after pulling/merging
- **post-rebase**: Sync after rebasing
- **pre-push**: Sync before pushing
- **post-checkout**: Sync after branch changes

Hooks are idempotent and won't break existing hooks.

### Schema Versioning

Schema version is tracked in `.version` file:

```
1  # Current schema version
```

When opening a store:
1. Check `.version` file
2. If version mismatch, run migrations
3. If database exists but schema missing, auto-sync from JSONL
4. Update `.version` file

### Sync Logic

When syncing:

1. Clear all SQLite tables
2. Read all JSONL files
3. For each record with same ID, keep latest `updated_at`
4. Insert latest versions into SQLite
5. Rebuild indexes

This ensures SQLite always reflects the current state from JSONL.

## Development

### Project Structure

```
taskstore/
├── src/
│   ├── lib.rs           # Library entry point
│   ├── models.rs        # Data models and enums
│   ├── store.rs         # Core Store implementation
│   ├── jsonl.rs         # JSONL file operations
│   ├── main.rs          # CLI application
│   ├── cli.rs           # CLI argument parsing (legacy)
│   ├── config.rs        # Configuration (legacy)
│   └── bin/
│       └── taskstore-merge.rs  # Git merge driver
├── docs/
│   └── taskstore-design.md     # Design document
├── Cargo.toml
└── build.rs             # Build script for git version
```

### Running Tests

```bash
cargo test
```

Test coverage includes:
- Model serialization
- JSONL append and read operations
- Store CRUD operations
- Sync logic
- Merge driver three-way merge scenarios

### Running Examples

Create a test PRD:

```rust
use taskstore::{Store, Prd, PrdStatus, now_ms};

let mut store = Store::open(".")?;

let prd = Prd {
    id: "prd-001".to_string(),
    title: "Example PRD".to_string(),
    description: "Test description".to_string(),
    created_at: now_ms(),
    updated_at: now_ms(),
    status: PrdStatus::Draft,
    review_passes: 0,
    content: "# Example\n\nContent here".to_string(),
};

store.create_prd(prd)?;
```

## Design Philosophy

1. **JSONL as truth**: All state lives in append-only JSONL files
2. **SQLite as cache**: Fast queries without corrupting the source
3. **Git-native**: Leverage git for version control and collaboration
4. **Automatic resolution**: Use timestamps to auto-resolve most conflicts
5. **Fail-safe**: If SQLite corrupts, regenerate from JSONL
6. **Schema evolution**: Migrations live in code, version tracked in file

## Use Cases

- **Concurrent agentic loops**: Multiple AI agents working on different tasks
- **Distributed teams**: Git-based collaboration with automatic conflict resolution
- **Audit trails**: JSONL files preserve full history of changes
- **Reproducibility**: Regenerate exact state from JSONL files
- **Offline work**: Work offline, sync when reconnected

## Performance

- **Reads**: Fast queries via SQLite indexes
- **Writes**: Append to JSONL (O(1)), then SQLite
- **Sync**: Full rebuild from JSONL (typically <100ms for 1000s of records)
- **Merge**: Three-way merge is O(n) where n = unique IDs

## Limitations

- JSONL files grow unbounded (no compaction yet)
- Full sync on every merge (no incremental updates)
- Timestamp-based conflict resolution (assumes synchronized clocks)
- No built-in data validation beyond Rust types

## Future Enhancements

- JSONL compaction (remove old versions)
- Incremental sync (only changed records)
- Vector clocks for distributed conflict resolution
- Web UI for visualization
- Metrics and monitoring
- Plugin system for custom workflows

## License

[License TBD]

## Contributing

[Contributing guidelines TBD]
