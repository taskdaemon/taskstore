# Design Document: TaskStore - Persistent State Management

**Author:** Claude Sonnet 4.5
**Date:** 2026-01-13
**Status:** Complete
**Review Passes:** 5/5

## Summary

TaskStore is a Rust library providing durable, git-integrated persistent storage for TaskDaemon's PRDs, task specs, execution state, and coordination data. It implements the SQLite+JSONL+Git pattern: SQLite for fast queries, JSONL files for git-based truth and merge conflict resolution, and a custom git merge driver for automated conflict handling.

## Problem Statement

### Background

TaskDaemon orchestrates multiple concurrent agentic loops, each executing phases of work defined by task specifications derived from product requirements documents. This creates several persistence challenges:

1. **State Durability:** Loop executions can crash, be interrupted, or need to pause/resume. All state must survive these events.
2. **Concurrent Access:** Multiple loops query and update state simultaneously without blocking each other.
3. **Merge Conflicts:** When loops work in separate git worktrees and merge back to main, state files can conflict.
4. **Auditability:** Teams need to review what work was done, track progress, and debug failures.
5. **Version Control:** State should be versioned alongside code, allowing rollback and branch comparison.

Existing solutions fall short:
- **In-memory only:** Lost on crash
- **SQLite alone:** Not git-friendly, binary format causes merge conflicts
- **JSONL alone:** Slow queries, no indexing
- **Beads:** Overcomplicated, invasive behavior, poor user experience
- **Engram:** Lost git integration (merge driver, hooks) by accident during decoupling

### Problem

**How do we build a persistent state store that is:**
- **Fast:** Sub-millisecond queries for lookups
- **Durable:** Survives crashes, supports pause/resume
- **Git-native:** Merges cleanly, works with worktrees
- **Concurrent:** Multiple readers/writers without blocking
- **Auditable:** Human-readable state files
- **Simple:** Thin library, minimal API surface

### Goals

- Implement SQLite+JSONL+Git pattern with custom merge driver
- Provide CRUD operations for PRDs, task specs, executions, dependencies
- Support atomic transactions for multi-record updates
- Enable fast queries: "Show all running executions", "Get TS dependencies", etc.
- Store state as line-delimited JSON (one record per line) for merge-friendliness
- Include git hooks for automatic sync on commit/merge/rebase
- Support graceful degradation: If SQLite is stale, rebuild from JSONL
- Library crate with thin CLI for manual inspection

### Non-Goals

- **Not a distributed database:** Single machine, single process writes
- **Not a replacement for git:** Git remains source of truth
- **Not a query engine:** No complex SQL beyond simple lookups
- **Not a workflow engine:** TaskDaemon handles orchestration
- **Not user-facing:** Internal library consumed by TaskDaemon
- **Not a background service:** Embedded library, not daemon process

## Proposed Solution

### Overview

TaskStore is a Rust library (`taskstore` crate) that maintains two synchronized representations of state:

1. **SQLite database** (`.taskstore/taskstore.db`): Fast indexed queries
2. **JSONL files** (`.taskstore/*.jsonl`): Git-friendly line-delimited JSON records

On every write, both are updated atomically. On startup or after git operations (merge, rebase, pull), TaskStore rebuilds SQLite from JSONL if timestamps indicate staleness.

A custom git merge driver resolves conflicts in JSONL files by merging record-by-record using unique IDs.

Git hooks (pre-commit, post-merge, post-rebase) trigger sync operations to keep SQLite current.

### Architecture

```
┌─────────────────────────────────────────────────────────┐
│                      TaskDaemon                          │
│                    (orchestrator)                        │
└────────────────┬────────────────────────────────────────┘
                 │
                 │ uses
                 ▼
┌─────────────────────────────────────────────────────────┐
│                   TaskStore Library                      │
│  ┌─────────────────────────────────────────────────┐   │
│  │              Public API                          │   │
│  │  - Store::open(path)                            │   │
│  │  - store.create_prd(prd)                        │   │
│  │  - store.get_prd(id)                            │   │
│  │  - store.update_execution(id, status)           │   │
│  │  - store.list_active_executions()               │   │
│  │  - store.sync()                                  │   │
│  └─────────────┬───────────────────────────────────┘   │
│                │                                         │
│  ┌─────────────┼───────────────────────────────────┐   │
│  │             ▼                                     │   │
│  │      Internal Implementation                     │   │
│  │                                                   │   │
│  │  ┌──────────────┐      ┌──────────────────┐    │   │
│  │  │   SQLite     │◄────►│    JSONL Files   │    │   │
│  │  │   Database   │      │   (line-per-rec) │    │   │
│  │  │              │      │                   │    │   │
│  │  │  Fast query  │      │  Git-friendly     │    │   │
│  │  │  Indexed     │      │  Human-readable   │    │   │
│  │  └──────────────┘      └──────────────────┘    │   │
│  │                                                   │   │
│  │  Sync Logic: Write → both, Read → SQLite        │   │
│  │  Rebuild: If stale, rebuild SQLite from JSONL   │   │
│  └───────────────────────────────────────────────────┘ │
└─────────────────────────────────────────────────────────┘

┌─────────────────────────────────────────────────────────┐
│                    Git Integration                       │
│                                                          │
│  Custom Merge Driver (.gitattributes)                   │
│    *.jsonl merge=taskstore-merge                        │
│                                                          │
│  Git Hooks:                                             │
│    pre-commit:   sync() before commit                   │
│    post-merge:   sync() after merge                     │
│    post-rebase:  sync() after rebase                    │
└─────────────────────────────────────────────────────────┘
```

### Data Model

#### Directory Structure

```
.taskstore/
├── taskstore.db           # SQLite database (fast queries)
├── prds.jsonl             # PRD records (one per line)
├── task_specs.jsonl       # Task spec records
├── executions.jsonl       # Execution state records
├── dependencies.jsonl     # Dependency records
├── workflows.jsonl        # AWL workflow definitions
├── repo_state.jsonl       # Per-repo metadata (last_synced_commit, etc.)
├── taskstore.log          # Structured log output (rotated)
└── .version               # Schema version marker
```

#### SQLite Schema

```sql
-- Product Requirements Documents
CREATE TABLE prds (
    id TEXT PRIMARY KEY,              -- UUIDv7 (sortable)
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    created_at INTEGER NOT NULL,      -- Unix timestamp (ms)
    updated_at INTEGER NOT NULL,
    status TEXT NOT NULL,             -- 'draft' | 'active' | 'complete'
    review_passes INTEGER NOT NULL,   -- Rule of Five tracking
    content TEXT NOT NULL             -- Full PRD markdown
);

-- Task Specifications (decomposed from PRDs)
CREATE TABLE task_specs (
    id TEXT PRIMARY KEY,              -- UUIDv7 (sortable)
    prd_id TEXT NOT NULL,             -- FK to prds.id
    phase_name TEXT NOT NULL,         -- e.g., "Phase 1: Core Logic"
    description TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    status TEXT NOT NULL,             -- 'pending' | 'running' | 'complete' | 'failed'
    workflow_name TEXT,               -- Which AWL workflow to use (e.g., "rust-development")
    assigned_to TEXT,                 -- Execution ID if assigned
    content TEXT NOT NULL,            -- Task spec markdown
    FOREIGN KEY (prd_id) REFERENCES prds(id) ON DELETE CASCADE
);

-- Execution State (loop instances)
CREATE TABLE executions (
    id TEXT PRIMARY KEY,              -- UUIDv7 (sortable)
    ts_id TEXT NOT NULL,              -- FK to task_specs.id
    worktree_path TEXT NOT NULL,      -- Git worktree path
    branch_name TEXT NOT NULL,        -- Git branch name
    status TEXT NOT NULL,             -- 'running' | 'paused' | 'complete' | 'failed' | 'stopped'
    started_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    completed_at INTEGER,             -- NULL if not complete
    current_phase TEXT,               -- Current foreach iteration
    iteration_count INTEGER NOT NULL DEFAULT 0,
    error_message TEXT,               -- NULL if no error
    FOREIGN KEY (ts_id) REFERENCES task_specs(id) ON DELETE CASCADE
);

-- Dependencies (coordination)
CREATE TABLE dependencies (
    id TEXT PRIMARY KEY,              -- UUIDv7 (sortable)
    from_exec_id TEXT NOT NULL,       -- Execution that depends
    to_exec_id TEXT NOT NULL,         -- Execution depended upon
    dependency_type TEXT NOT NULL,    -- 'notify' | 'query' | 'share'
    created_at INTEGER NOT NULL,
    resolved_at INTEGER,              -- NULL if not resolved
    payload TEXT,                     -- JSON payload for share/query
    FOREIGN KEY (from_exec_id) REFERENCES executions(id) ON DELETE CASCADE,
    FOREIGN KEY (to_exec_id) REFERENCES executions(id) ON DELETE CASCADE
);

-- AWL Workflow Definitions
CREATE TABLE workflows (
    id TEXT PRIMARY KEY,              -- UUIDv7 (sortable)
    name TEXT NOT NULL UNIQUE,        -- e.g., "rust-development"
    version TEXT NOT NULL,            -- Semantic version
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    content TEXT NOT NULL             -- Full YAML workflow definition
);

-- Repository State (per-repo metadata)
CREATE TABLE repo_state (
    repo_path TEXT PRIMARY KEY,       -- Absolute path to repo root
    last_synced_commit TEXT NOT NULL, -- Git SHA of main when last synced
    updated_at INTEGER NOT NULL
);

-- Indexes for common queries
CREATE INDEX idx_prds_status ON prds(status);
CREATE INDEX idx_task_specs_prd_id ON task_specs(prd_id);
CREATE INDEX idx_task_specs_status ON task_specs(status);
CREATE INDEX idx_executions_ts_id ON executions(ts_id);
CREATE INDEX idx_executions_status ON executions(status);
CREATE INDEX idx_dependencies_from ON dependencies(from_exec_id);
CREATE INDEX idx_dependencies_to ON dependencies(to_exec_id);
CREATE INDEX idx_workflows_name ON workflows(name);
```

#### JSONL Format

Each JSONL file contains one JSON object per line. Each record has a unique `id` field.

**prds.jsonl example:**
```jsonl
{"id":"550e8400-e29b-41d4-a716-446655440000","title":"Add User Authentication","description":"Implement JWT-based auth...","created_at":1704067200000,"updated_at":1704067200000,"status":"active","review_passes":5,"content":"# PRD: Add User Authentication\n\n..."}
{"id":"660e8400-e29b-41d4-a716-446655440001","title":"Database Migration Tool","description":"CLI tool for managing migrations...","created_at":1704070800000,"updated_at":1704070800000,"status":"draft","review_passes":3,"content":"# PRD: Database Migration Tool\n\n..."}
```

**executions.jsonl example:**
```jsonl
{"id":"770e8400-e29b-41d4-a716-446655440010","ts_id":"880e8400-e29b-41d4-a716-446655440020","worktree_path":"/tmp/worktrees/exec-770e8400","branch_name":"feature/auth-phase1","status":"running","started_at":1704074400000,"updated_at":1704078000000,"completed_at":null,"current_phase":"Phase 1: Core Logic","iteration_count":3,"error_message":null}
```

### API Design

```rust
use eyre::Result;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

// Main store handle
pub struct Store {
    db: rusqlite::Connection,
    base_path: PathBuf,
}

impl Store {
    /// Open or create store at given path
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self>;

    /// Sync: Rebuild SQLite from JSONL if needed
    pub fn sync(&mut self) -> Result<()>;

    /// Flush: Ensure all writes are persisted
    pub fn flush(&mut self) -> Result<()>;
}

// PRD operations
impl Store {
    pub fn create_prd(&mut self, prd: Prd) -> Result<String>; // Returns ID
    pub fn get_prd(&self, id: &str) -> Result<Option<Prd>>;
    pub fn update_prd(&mut self, id: &str, prd: Prd) -> Result<()>;
    pub fn list_prds(&self, status: Option<PrdStatus>) -> Result<Vec<Prd>>;
}

// Task Spec operations
impl Store {
    pub fn create_task_spec(&mut self, ts: TaskSpec) -> Result<String>;
    pub fn get_task_spec(&self, id: &str) -> Result<Option<TaskSpec>>;
    pub fn update_task_spec(&mut self, id: &str, ts: TaskSpec) -> Result<()>;
    pub fn list_task_specs(&self, prd_id: &str) -> Result<Vec<TaskSpec>>;
    pub fn list_pending_task_specs(&self) -> Result<Vec<TaskSpec>>;
}

// Execution operations
impl Store {
    pub fn create_execution(&mut self, exec: Execution) -> Result<String>;
    pub fn get_execution(&self, id: &str) -> Result<Option<Execution>>;
    pub fn update_execution(&mut self, id: &str, exec: Execution) -> Result<()>;
    pub fn list_executions(&self, status: Option<ExecStatus>) -> Result<Vec<Execution>>;
    pub fn list_active_executions(&self) -> Result<Vec<Execution>>;

    /// Complete execution and cascade PRD completion if all TSs are done
    pub fn complete_execution(&mut self, exec_id: &str) -> Result<()> {
        // 1. Mark execution complete
        let exec = self.get_execution(exec_id)?.ok_or_else(|| eyre!("Execution not found"))?;
        let mut updated_exec = exec.clone();
        updated_exec.status = ExecStatus::Complete;
        updated_exec.completed_at = Some(now_ms());
        self.update_execution(exec_id, updated_exec)?;

        // 2. Mark TS complete
        let ts_id = &exec.ts_id;
        let ts = self.get_task_spec(ts_id)?.ok_or_else(|| eyre!("TaskSpec not found"))?;
        let mut updated_ts = ts.clone();
        updated_ts.status = TaskSpecStatus::Complete;
        updated_ts.updated_at = now_ms();
        self.update_task_spec(ts_id, updated_ts)?;

        // 3. Check if all TSs for PRD are complete
        let prd_id = &ts.prd_id;
        let all_ts = self.list_task_specs(prd_id)?;

        if all_ts.iter().all(|ts| ts.status == TaskSpecStatus::Complete) {
            // 4. Mark PRD complete
            let prd = self.get_prd(prd_id)?.ok_or_else(|| eyre!("PRD not found"))?;
            let mut updated_prd = prd.clone();
            updated_prd.status = PrdStatus::Complete;
            updated_prd.updated_at = now_ms();
            self.update_prd(prd_id, updated_prd)?;
        }

        Ok(())
    }
}

// Dependency operations
impl Store {
    pub fn create_dependency(&mut self, dep: Dependency) -> Result<String>;
    pub fn get_dependency(&self, id: &str) -> Result<Option<Dependency>>;
    pub fn resolve_dependency(&mut self, id: &str, payload: Option<String>) -> Result<()>;
    pub fn list_dependencies(&self, exec_id: &str) -> Result<Vec<Dependency>>;
}

// Data structures
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prd {
    pub id: String,
    pub title: String,
    pub description: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub status: PrdStatus,
    pub review_passes: u8,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrdStatus {
    Draft,
    Active,
    Complete,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSpec {
    pub id: String,
    pub prd_id: String,
    pub phase_name: String,
    pub description: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub status: TaskSpecStatus,
    pub assigned_to: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskSpecStatus {
    Pending,
    Running,
    Complete,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Execution {
    pub id: String,
    pub ts_id: String,
    pub worktree_path: String,
    pub branch_name: String,
    pub status: ExecStatus,
    pub started_at: i64,
    pub updated_at: i64,
    pub completed_at: Option<i64>,
    pub current_phase: Option<String>,
    pub iteration_count: u32,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecStatus {
    Running,
    Paused,
    Complete,
    Failed,
    Stopped,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    pub id: String,
    pub from_exec_id: String,
    pub to_exec_id: String,
    pub dependency_type: DependencyType,
    pub created_at: i64,
    pub resolved_at: Option<i64>,
    pub payload: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DependencyType {
    Notify,
    Query,
    Share,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub id: String,
    pub name: String,
    pub version: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub content: String,  // YAML workflow definition
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoState {
    pub repo_path: String,
    pub last_synced_commit: String,  // Git SHA
    pub updated_at: i64,
}
```

### Usage Examples

**Creating a PRD:**
```rust
let store = Store::open(".taskstore")?;

let prd = Prd {
    id: uuid::Uuid::now_v7().to_string(),
    title: "Add User Authentication".to_string(),
    description: "Implement JWT-based auth system".to_string(),
    created_at: now_ms(),
    updated_at: now_ms(),
    status: PrdStatus::Draft,
    review_passes: 0,
    content: "# PRD: Add User Authentication\n\n...".to_string(),
};

let prd_id = store.create_prd(prd)?;
println!("Created PRD: {}", prd_id);
```

**Querying active executions:**
```rust
let active = store.list_active_executions()?;
for exec in active {
    println!("Execution {} on {}: {:?}",
        exec.id, exec.branch_name, exec.status);
}
```

**Pagination:**
```rust
// Get first page (100 records)
let page1 = store.list_executions_paginated(None, 100, 0)?;

// Get second page (next 100 records)
let page2 = store.list_executions_paginated(None, 100, 100)?;
```

### Merge Driver Algorithm

The `taskstore-merge` binary implements a three-way merge for JSONL files:

1. **Parse all three versions:** base (common ancestor), ours (current branch), theirs (incoming branch)
2. **Build ID maps:** Index all records by their unique `id` field
3. **Merge logic per record:**
   - If only in ours: Keep ours
   - If only in theirs: Add theirs
   - If in both: Compare `updated_at` timestamps, keep newer version
   - If same timestamp: Mark as conflict, write both with `<<<<<<< OURS` and `>>>>>>> THEIRS` markers (user must resolve manually)
4. **Write merged result:** One record per line, sorted by ID for determinism

**Example:**
```
Base:    {"id":"A","updated_at":1000,"status":"running"}
Ours:    {"id":"A","updated_at":1001,"status":"paused"}
Theirs:  {"id":"A","updated_at":1002,"status":"complete"}
Merged:  {"id":"A","updated_at":1002,"status":"complete"}  # Theirs wins (newer)
```

**Conflict example (same timestamp):**
```
<<<<<<< OURS
{"id":"B","updated_at":1500,"status":"running"}
=======
{"id":"B","updated_at":1500,"status":"paused"}
>>>>>>> THEIRS
```

User must manually edit to resolve.

### Staleness Detection Algorithm

SQLite is considered stale if any JSONL file is newer than the database file:

```rust
fn is_stale(db_path: &Path, jsonl_files: &[PathBuf]) -> Result<bool> {
    let db_mtime = fs::metadata(db_path)?.modified()?;
    for jsonl in jsonl_files {
        let jsonl_mtime = fs::metadata(jsonl)?.modified()?;
        if jsonl_mtime > db_mtime {
            return Ok(true);
        }
    }
    Ok(false)
}
```

On startup and after git hooks, check staleness. If stale, call `sync()` to rebuild.

### JSONL Update Semantics

JSONL files are **append-only logs**. Updates work as follows:

1. **Write operation:** Append new record to JSONL (even if updating existing ID)
2. **Result:** Multiple records with same ID, differing `updated_at` timestamps
3. **Sync operation:** When rebuilding SQLite from JSONL, take the **latest** record per ID (highest `updated_at`)

**Example prds.jsonl:**
```jsonl
{"id":"A","title":"Feature X","status":"draft","updated_at":1000}
{"id":"A","title":"Feature X","status":"active","updated_at":1001}
{"id":"A","title":"Feature X","status":"complete","updated_at":1002}
```

When syncing, only the last record (complete, 1002) is inserted into SQLite.

**Benefits:**
- Merge conflicts are rare (append-only)
- Full audit trail (all versions preserved)
- Sync is idempotent (can rebuild anytime)

**Compaction (future):** Periodically rewrite JSONL files keeping only latest record per ID to reclaim space.

### Transaction Boundaries and Dual-Write Ordering

**Critical invariant:** JSONL is source of truth. SQLite is derived cache.

**Write ordering:** Always write JSONL first, then SQLite.

```rust
pub fn update_execution_and_ts(&mut self, exec_id: &str, ts_id: &str) -> Result<()> {
    // 1. Write to JSONL first (source of truth)
    self.append_jsonl("executions.jsonl", &exec)?;
    self.append_jsonl("task_specs.jsonl", &ts)?;

    // 2. Then update SQLite (derived cache)
    let tx = self.db.transaction()?;
    tx.execute("UPDATE executions SET ...", ...)?;
    tx.execute("UPDATE task_specs SET ...", ...)?;
    tx.commit()?;

    Ok(())
}
```

**Failure scenarios:**
- **JSONL write fails:** Return error immediately, nothing written, consistent state
- **SQLite write fails after JSONL succeeds:** SQLite is now stale, but next `sync()` will rebuild from JSONL and repair

**Single-record operations:** Same ordering applies to all CRUD operations.

### Error Handling Patterns

**Update non-existent record:** Return `Err(eyre!("Record not found: {}", id))`

**Create duplicate ID:** Check SQLite first, return `Err(eyre!("Record already exists: {}", id))` if found

**JSONL write failure:** Return error immediately (may indicate disk full, permission issues, or read-only filesystem)

**SQLite corruption:** Rebuild from JSONL via `sync()`

**Malformed JSONL line:** Skip line, emit `tracing::warn!()`, continue sync (graceful degradation)

**Empty JSONL file:** Treat as "no records", sync results in empty table (valid state)

**Deleted JSONL file:** Treat as "no records" if file doesn't exist, sync creates empty table

**JSONL line exceeds size limit:** Reject writes >10MB per record, return error (prevents unbounded memory usage)

**File permission denied:** Return error with context about .taskstore/ permissions

**Concurrent access:** Document as unsupported - TaskDaemon is single-process, external writes are undefined behavior

**Deleted git worktree:** Execution record becomes stale, TaskDaemon should detect and mark as failed

**Circular dependencies:** Not validated at write time (allow for flexibility), but provide query helper to detect cycles

### Logging Strategy

Use `tracing` crate for structured logging:

```rust
use tracing::{info, warn, error};

// Info: Normal operations
info!(store_path = ?self.base_path, "Opening TaskStore");

// Warn: Recoverable issues
warn!(line_no = line_num, file = "prds.jsonl", "Skipping malformed JSONL line");

// Error: Critical failures
error!(error = ?e, "Failed to write JSONL, data loss possible");
```

TaskDaemon configures tracing subscriber to write to `.taskstore/taskstore.log` for debugging.

### Edge Case: Timestamps

All timestamps are **Unix milliseconds (UTC)**. No timezone conversion logic in store.

```rust
use std::time::{SystemTime, UNIX_EPOCH};

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("System time before Unix epoch")
        .as_millis() as i64
}
```

### Scale Limits and Pagination

**Expected scale:**
- **PRDs:** 10-100 concurrent PRDs per repo
- **Task specs:** 100-1000 per PRD (10K total)
- **Executions:** Up to 50 concurrent, 10K historical per repo
- **Dependencies:** Up to 500 active coordination links

**Pagination support:**

```rust
pub fn list_executions_paginated(
    &self,
    status: Option<ExecStatus>,
    limit: usize,
    offset: usize
) -> Result<Vec<Execution>>;
```

CLI and TUI use pagination for large result sets. Default limit: 100 records.

### JSONL Compaction Strategy

**Trigger conditions:**
- JSONL file exceeds 100MB
- More than 80% of lines are superseded (older versions of same ID)
- Manual invocation via `taskstore compact` CLI command

**Compaction algorithm:**
1. Read entire JSONL file (e.g., `prds.jsonl`)
2. Build map of ID → latest record (highest updated_at)
3. Write compacted records to temporary file (e.g., `prds.jsonl.tmp`)
4. Fsync temporary file to disk
5. Atomically rename: `rename("prds.jsonl.tmp", "prds.jsonl")`
6. Trigger sync() to rebuild SQLite

**Atomicity:** `rename()` is atomic on POSIX filesystems. If compaction crashes mid-way, original file is preserved.

**Safety:** Compaction preserves all current state, only removes superseded history.

### Observability and Metrics

TaskStore exposes metrics for monitoring:

```rust
pub struct StoreMetrics {
    pub write_latency_ms: f64,       // Exponential moving average of last 100 writes
    pub sync_duration_ms: f64,       // Duration of last sync() call
    pub prd_count: usize,            // COUNT(*) from prds table
    pub task_spec_count: usize,
    pub execution_count: usize,
    pub prds_jsonl_size_mb: f64,     // fs::metadata().len() / 1MB
    pub executions_jsonl_size_mb: f64,
}

impl Store {
    pub fn metrics(&self) -> Result<StoreMetrics>;
}
```

**Collection timing:**
- Write latency: Measured on each write, EMA updated
- Sync duration: Recorded at end of each sync() call
- Counts: Queried on-demand when metrics() is called
- File sizes: Queried on-demand via fs::metadata()

**Usage:**
- TaskDaemon calls `store.metrics()` every 5 seconds for TUI status bar
- Optional: Expose via Prometheus endpoint for monitoring

### Backup and Restore Strategy

**Backup:**
- JSONL files are the source of truth
- To backup: `cp -r .taskstore/ backup/taskstore-$(date +%s)/`
- Or commit .taskstore/ to git (recommended)

**Restore:**
- Copy JSONL files to .taskstore/
- Delete taskstore.db (or let it be stale)
- Call `taskstore sync` or `Store::open()` (auto-syncs)
- SQLite rebuilds from JSONL

**Git-based backup (recommended):**
- .gitignore: `taskstore.db` and `taskstore.log`
- Commit JSONL files: `prds.jsonl`, `executions.jsonl`, etc.
- Push to remote for off-machine backup

### TaskDaemon Integration and Coordination

**Store ownership:**
- TaskDaemon creates single Store instance at startup
- Actor pattern: State manager task owns Store, other tasks send messages
- Store lives for entire TaskDaemon lifetime

**Actor pattern detail:**
```rust
// State manager task (owns Store)
tokio::spawn(async move {
    let mut store = Store::open(".taskstore")?;
    loop {
        match rx.recv().await {
            StoreMessage::CreateExecution(exec, reply_tx) => {
                let result = store.create_execution(exec);
                reply_tx.send(result).await;
            }
            StoreMessage::UpdateExecution(id, exec, reply_tx) => {
                let result = store.update_execution(&id, exec);
                reply_tx.send(result).await;
            }
            // ... other message types
        }
    }
});

// Loop tasks send messages to state manager
let (store_tx, store_rx) = mpsc::channel(100);
let (reply_tx, reply_rx) = oneshot::channel();
store_tx.send(StoreMessage::CreateExecution(exec, reply_tx)).await?;
let exec_id = reply_rx.await??;
```

**Change notification:**
- TaskStore is passive (no pub/sub, no watchers)
- TaskDaemon polls Store periodically or after known writes
- For real-time coordination, TaskDaemon uses in-memory channels, Store provides durable backup

**Multi-repo support:**
- Each repo has its own `.taskstore/` directory at repo root
- TaskDaemon can manage multiple repos, opens one Store per repo
- Store path: `<repo_root>/.taskstore/`

**Proactive rebase detection:**
- repo_state table tracks `last_synced_commit` (main branch HEAD SHA)
- Periodically query `git rev-parse main`, compare to stored SHA
- If different, trigger proactive rebase flow (notify all executions)

### Schema Migration Strategy

The `.version` file contains current schema version as integer (e.g., `1`).

**Migration process:**
1. On `Store::open()`, read `.version` file
2. Compare to `CURRENT_VERSION` constant in code
3. If behind, run migration functions sequentially
4. Update `.version` file after successful migration

**Example migrations:**
```rust
const CURRENT_VERSION: u32 = 2;

fn migrate_1_to_2(store: &mut Store) -> Result<()> {
    // Add new column to executions table
    store.db.execute("ALTER TABLE executions ADD COLUMN priority INTEGER DEFAULT 0", [])?;
    // Rebuild from JSONL to populate new column with defaults
    store.sync()?;
    Ok(())
}
```

**JSONL schema evolution:** JSON is flexible, missing fields default via serde. For breaking changes, write migration that rewrites JSONL files.

### Implementation Plan

#### Phase 1: Core Library Structure
- Create `taskstore` crate with library + binary structure:
  - `src/lib.rs` - Main library exports (pub use Store, models, etc.)
  - `src/store.rs` - Store implementation
  - `src/models.rs` - Data structures (Prd, TaskSpec, Execution, Dependency)
  - `src/jsonl.rs` - JSONL read/write operations
  - `src/sqlite.rs` - SQLite operations
  - `src/bin/taskstore.rs` - Thin CLI binary that uses the library
- Update Cargo.toml with `[lib]` and `[[bin]]` sections
- Define all data structures with proper serde derives
- Implement basic Store::open() with staleness check and sync()
- Add directory structure creation (.taskstore/ with .gitignore for .db file)
- Implement schema version tracking and migration framework

#### Phase 2: SQLite Implementation
- Write SQL schema creation
- Implement PRD CRUD operations
- Implement TaskSpec CRUD operations
- Implement Execution CRUD operations
- Implement Dependency CRUD operations
- Add indexes for common queries

#### Phase 3: JSONL Persistence
- Implement write-through: Every SQL write appends to JSONL
- Implement sync: Rebuild SQLite from JSONL if stale
- Add timestamp-based staleness detection
- Handle JSONL file rotation if files grow too large (future)

#### Phase 4: Git Merge Driver
- Write `taskstore-merge` binary that takes 3 args: base, ours, theirs file paths
- Implement line-by-line merge using unique IDs and updated_at timestamps
- Handle conflict resolution: manual markers if timestamps equal
- Exit 0 on success, non-zero on unresolvable conflict

**Git configuration:**
```bash
# In .git/config or ~/.gitconfig
[merge "taskstore-merge"]
    name = TaskStore JSONL merge driver
    driver = taskstore-merge %O %A %B
```

**In .gitattributes:**
```
.taskstore/*.jsonl merge=taskstore-merge
```

**Installation helper:**
```rust
impl Store {
    pub fn install_git_integration(&self) -> Result<()> {
        // Write .gitattributes
        // Configure git merge driver
        // Install git hooks
    }
}
```

#### Phase 5: Git Hooks
- Write hook scripts that shell out to `taskstore sync`
- Install hooks to `.git/hooks/` (make executable)
- Hook content:

**post-merge hook:**
```bash
#!/bin/bash
# Auto-sync TaskStore after merge
cd "$(git rev-parse --show-toplevel)"
taskstore sync || echo "Warning: TaskStore sync failed"
exit 0  # Don't block merge on sync failure
```

Same pattern for post-rebase and pre-commit.

Installation via `store.install_git_integration()` or manual:
```bash
taskstore install-hooks
```

#### Phase 6: CLI Tool
- Add `taskstore` binary for manual inspection
- Commands:
  - `taskstore list-prds [--status STATUS]` - List PRDs with optional status filter
  - `taskstore list-task-specs [--prd-id ID]` - List task specs, optionally for one PRD
  - `taskstore list-executions [--status STATUS] [--limit N] [--offset N]` - Paginated execution list
  - `taskstore show <id>` - Show detailed record by ID (auto-detects table)
  - `taskstore sync` - Force rebuild SQLite from JSONL
  - `taskstore compact` - Compact JSONL files (remove superseded records)
  - `taskstore metrics` - Display store metrics (record counts, file sizes, latencies)
  - `taskstore backup <dest>` - Copy JSONL files to destination directory
  - `taskstore install-hooks` - Install git hooks and merge driver
- Pretty-printed output using colored terminal output (colored crate)

**Example CLI output:**

```
$ taskstore list-executions --status running
ID                                      TS                 Branch                 Status    Phase
────────────────────────────────────────────────────────────────────────────────────────────────────
01928374-abcd-7000-a123-456789012345   01928374-...       feature/auth-phase1    running   Phase 2
01928375-beef-7000-a456-789012345678   01928375-...       feature/db-migration   running   Phase 1

$ taskstore metrics
TaskStore Metrics
─────────────────────────────────────
PRDs:              42
Task Specs:        387
Executions:        12 (8 running, 4 complete)
Dependencies:      23

Write Latency:     0.8ms (avg)
Sync Duration:     127ms (last)

JSONL File Sizes:
  prds.jsonl:        8.4 MB
  executions.jsonl:  2.1 MB
  task_specs.jsonl:  15.7 MB
```

#### Phase 7: Testing & Docs
- Unit tests for all CRUD operations
- Integration tests for sync behavior
- Test merge driver with conflicting JSONL files
- Write comprehensive README.md
- Add rustdoc comments to all public APIs

## Alternatives Considered

### Alternative 1: Pure SQLite (No JSONL)
- **Description:** Store everything in SQLite only, commit the .db file to git
- **Pros:** Simpler implementation, no dual-write overhead
- **Cons:** Binary files cause merge conflicts, not human-readable, git diffs useless
- **Why not chosen:** Git integration is critical for auditability and team collaboration

### Alternative 2: Pure JSONL (No SQLite)
- **Description:** Store everything as JSONL, parse on every query
- **Pros:** Git-native, human-readable, simple implementation
- **Cons:** Slow queries (O(n) scans), no indexing, poor performance at scale
- **Why not chosen:** Unacceptable performance for "list all active executions" queries

### Alternative 3: Use Beads
- **Description:** Adopt Beads as the datastore
- **Pros:** Already exists, proven merge driver, community support
- **Cons:** Slow, invasive (auto-push), overcomplicated API, poor UX, one-per-line beads clutter git
- **Why not chosen:** TaskDaemon needs cleaner, simpler, less invasive storage

### Alternative 4: Use Engram
- **Description:** Fork Engram and restore git integration
- **Pros:** Close to our needs, already has SQLite+JSONL pattern
- **Cons:** Lost merge driver and hooks (would need to re-add), codebase unfamiliar, licensing unclear
- **Why not chosen:** Starting fresh is cleaner than fixing someone else's accidental minimalism

### Alternative 5: Temporal-style Event Sourcing
- **Description:** Store all state changes as append-only event log
- **Pros:** Full history, time-travel debugging, audit trail
- **Cons:** Complexity overkill, slower queries, much larger storage footprint
- **Why not chosen:** TaskDaemon doesn't need full event sourcing; current state is sufficient

## Technical Considerations

### Dependencies

**Rust crates:**
- `rusqlite` - SQLite bindings
- `serde` + `serde_json` - Serialization
- `uuid` - Unique ID generation (UUIDv7)
- `eyre` - Error handling
- `clap` - CLI argument parsing (for binary)
- `tracing` - Structured logging

**External tools:**
- Git (for merge driver and hooks)

### Performance

**Expected characteristics:**
- **Writes:** ~1ms per record (SQLite write + JSONL append)
- **Reads:** <1ms for indexed lookups
- **Sync:** ~100ms for 10,000 records (rebuild SQLite from JSONL)
- **Merge:** ~10ms per conflicting JSONL line

**Optimization strategies:**
- SQLite WAL mode for concurrent reads during writes
- Batch writes: Transaction wrapping multiple records
- Lazy sync: Only rebuild if stale (timestamp check)
- JSONL streaming: Don't load entire file into memory

### Security

**Threat model:**
- TaskStore runs on local machine with trusted operators
- No network exposure, no authentication needed
- File permissions inherit from git repo

**Considerations:**
- **SQL injection:** Use parameterized queries (rusqlite handles this)
- **Path traversal:** Validate all file paths stay within .taskstore/
- **Malicious JSONL:** Validate records during sync, reject malformed JSON

### Testing Strategy

**Unit tests:**
- Test each CRUD operation in isolation
- Test sync logic with stale/fresh SQLite
- Test JSONL append and read
- Test timestamp staleness detection

**Integration tests:**
- Create store, write records, close, reopen, verify persistence
- Simulate git merge conflict, test merge driver
- Trigger git hooks, verify sync is called

**Performance tests:**
- Benchmark CRUD operations with 10K, 100K, 1M records
- Measure sync time for various JSONL file sizes
- Profile memory usage during sync

### Rollout Plan

**Phase 1: Library development**
- Build taskstore as standalone crate
- Test in isolation with unit/integration tests

**Phase 2: TaskDaemon integration**
- Add taskstore as git dependency in TaskDaemon Cargo.toml
- Replace in-memory state with Store calls

**Phase 3: Git integration setup**
- Document merge driver installation steps
- Provide hook installation helper
- Test with multiple concurrent git worktrees

**Phase 4: Production hardening**
- Add comprehensive error handling
- Implement graceful degradation (e.g., if JSONL corrupt, rebuild from SQLite)
- Add logging for debugging

## Risks and Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| Merge driver conflicts not handled correctly | Medium | High | Thorough testing with real merge scenarios, fall back to manual conflict markers if tie |
| JSONL files grow too large (GBs) | Low | Medium | Implement file rotation/archiving after threshold, keep only recent N records |
| SQLite corruption | Low | High | Always rebuild from JSONL (source of truth), keep SQLite in .gitignore |
| Dual-write inconsistency (JSONL succeeds, SQLite fails) | Low | Medium | SQLite becomes stale, next sync() rebuilds and repairs automatically |
| Hook installation issues on different platforms | Medium | Low | Provide clear installation instructions, test on Linux/Mac/Windows |
| Performance degradation at scale | Medium | Medium | Profile early, add caching layer if needed, consider archiving old data |
| Concurrent writes from multiple processes | Low | High | Document: TaskDaemon is single-process, external writes are undefined behavior |
| Concurrent git operations during write | Medium | Medium | Document: Avoid git operations while TaskDaemon is running, or pause executions first |
| UUIDv7 collision | Very Low | High | Check for duplicates before insert, return error if collision detected |
| Record size exceeds memory limits | Low | Medium | Enforce 10MB per-record limit, reject writes that exceed threshold |

## Open Questions

- [ ] Should we archive old completed PRDs/executions automatically?
- [ ] How large can JSONL files grow before we need rotation?
- [ ] Should TaskStore support multi-process access (e.g., file locking)?
- [ ] Do we need a vacuum/cleanup operation for SQLite?
- [ ] Should we support JSON schema validation for JSONL records?

## References

- Engram architecture: `~/.config/pais/research/tech/engram-vs-beads/2026-01-12-comparison.md`
- Accidental minimalism lesson: `~/.config/pais/research/tech/engram-vs-beads/2026-01-12-accidental-minimalism.md`
- TaskDaemon design: `../../taskdaemon/docs/taskdaemon-design.md`
- AWL Schema design: `../../taskdaemon/docs/awl-schema-design.md`
- SQLite WAL mode: https://www.sqlite.org/wal.html
- JSONL format: https://jsonlines.org/
