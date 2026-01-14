# TaskStore Implementation Guide

**Date:** 2026-01-13
**Status:** Active

## Overview

This guide provides practical implementation details for building TaskStore, including repo structure, naming conventions, schema design, and git integration.

## 1. Repository Structure: Library + Binary

TaskStore is both a library (for use by TaskDaemon) and a CLI binary (for manual inspection and git hooks).

### Current State (from scaffold)

```
taskstore/
├── Cargo.toml
├── src/
│   ├── main.rs    # Binary only
│   ├── cli.rs
│   └── config.rs
```

### Target State (library + thin CLI)

```
taskstore/
├── Cargo.toml              # [[bin]] section added
├── src/
│   ├── lib.rs              # NEW: Main library (pub use)
│   ├── store.rs            # Store implementation
│   ├── models.rs           # PRD, TaskSpec, Execution structs
│   ├── jsonl.rs            # JSONL persistence
│   ├── sqlite.rs           # SQLite operations
│   ├── merge.rs            # Git merge driver
│   ├── cli.rs              # CLI argument parsing (keep)
│   ├── config.rs           # Config loading (keep)
│   └── bin/
│       └── taskstore.rs    # NEW: Thin CLI (calls lib)
```

### Conversion Steps

#### 1. Create src/lib.rs

```rust
// taskstore/src/lib.rs
pub mod store;
pub mod models;
pub mod jsonl;
pub mod sqlite;
pub mod merge;

pub use store::Store;
pub use models::{Prd, TaskSpec, Execution, Dependency, Workflow};
```

#### 2. Create src/bin/taskstore.rs

```rust
// taskstore/src/bin/taskstore.rs
use clap::Parser;
use taskstore::{Store, cli::Cli};
use eyre::Result;

fn main() -> Result<()> {
    let cli = Cli::parse();
    let store = Store::open(".taskstore")?;

    // CLI logic here (list, show, sync, etc.)
    match cli.command {
        Command::ListPrds { status } => {
            let prds = store.list_prds(status)?;
            for prd in prds {
                println!("{} - {} [{}]", prd.id, prd.title, prd.status);
            }
        }
        Command::Sync => {
            store.sync()?;
            println!("TaskStore synced successfully");
        }
        // ... other commands
    }

    Ok(())
}
```

#### 3. Update Cargo.toml

```toml
[package]
name = "taskstore"
version = "0.1.0"
edition = "2024"

[lib]
name = "taskstore"
path = "src/lib.rs"

[[bin]]
name = "taskstore"
path = "src/bin/taskstore.rs"

[dependencies]
rusqlite = { version = "0.32", features = ["bundled"] }
serde = { version = "1.0", features = ["derive"] }
serde_json = "1.0"
eyre = "0.6"
uuid = { version = "1.0", features = ["v7"] }
clap = { version = "4.0", features = ["derive"] }
```

#### 4. Move Logic

- **Core logic** → `src/store.rs`, `src/models.rs`, `src/jsonl.rs`, `src/sqlite.rs`
- **CLI commands** → `src/bin/taskstore.rs`
- **Keep** `cli.rs` and `config.rs` as-is (used by both lib and bin)

## 2. File Naming Conventions

### Rust Module Names

**Rule:** Use short, single-word module names to avoid underscores entirely.

```
taskstore/src/
├── lib.rs
├── store.rs      # mod store; (not store_manager)
├── models.rs     # mod models;
├── jsonl.rs      # mod jsonl;
├── sqlite.rs     # mod sqlite;
├── merge.rs      # mod merge; (git merge driver)
└── bin/
    └── taskstore.rs
```

### JSONL Field Names

**Rule:** Use snake_case in JSONL (matches Rust struct fields directly)

```jsonl
{"id":"prd-550e8400","title":"Add OAuth","status":"draft","created_at":1704067200000,"updated_at":1704067200000}
```

**Rust struct:**
```rust
#[derive(Debug, Serialize, Deserialize)]
pub struct Prd {
    pub id: String,
    pub title: String,
    pub status: PrdStatus,
    pub created_at: i64,
    pub updated_at: i64,
}
```

**Note:** No serde rename needed - JSONL uses snake_case, Rust uses snake_case.

### Markdown File Names

**Rule:** Lowercase with hyphens, sanitize special characters

```rust
fn sanitize_filename(title: &str) -> String {
    title
        .to_lowercase()
        .replace(char::is_whitespace, "-")
        .replace(|c: char| !c.is_alphanumeric() && c != '-', "")
}

// "Add OAuth Authentication" → "add-oauth-authentication.md"
```

## 3. Database Schema

### Schema Design

```sql
-- Product Requirements Documents
CREATE TABLE prds (
    id TEXT PRIMARY KEY,
    title TEXT NOT NULL,
    description TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    status TEXT NOT NULL,             -- 'draft' | 'ready' | 'in_progress' | 'complete' | 'failed' | 'cancelled'
    review_passes INTEGER NOT NULL,
    file TEXT NOT NULL,               -- Markdown filename
    UNIQUE(file)
);

-- Task Specifications
CREATE TABLE task_specs (
    id TEXT PRIMARY KEY,
    prd_id TEXT NOT NULL,
    phase_name TEXT NOT NULL,
    description TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    status TEXT NOT NULL,             -- 'pending' | 'blocked' | 'running' | 'complete' | 'failed'
    workflow_name TEXT,               -- Which AWL workflow to use
    assigned_to TEXT,                 -- Execution ID if running
    file TEXT NOT NULL,               -- Markdown filename
    content TEXT NOT NULL,            -- Full task spec markdown
    FOREIGN KEY (prd_id) REFERENCES prds(id) ON DELETE CASCADE,
    UNIQUE(file)
);

-- Executions (running loops)
CREATE TABLE executions (
    id TEXT PRIMARY KEY,
    ts_id TEXT NOT NULL,
    worktree_path TEXT NOT NULL,
    branch_name TEXT NOT NULL,
    status TEXT NOT NULL,             -- 'running' | 'paused' | 'complete' | 'failed' | 'stopped'
    started_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    completed_at INTEGER,
    current_phase TEXT,
    iteration_count INTEGER NOT NULL DEFAULT 0,
    error_message TEXT,
    FOREIGN KEY (ts_id) REFERENCES task_specs(id) ON DELETE CASCADE
);

-- Dependencies (coordination messages)
CREATE TABLE dependencies (
    id TEXT PRIMARY KEY,
    from_exec_id TEXT NOT NULL,
    to_exec_id TEXT,                  -- NULL for broadcast
    dependency_type TEXT NOT NULL,    -- 'notify' | 'query' | 'share'
    created_at INTEGER NOT NULL,
    resolved_at INTEGER,
    payload TEXT,
    FOREIGN KEY (from_exec_id) REFERENCES executions(id) ON DELETE CASCADE,
    FOREIGN KEY (to_exec_id) REFERENCES executions(id) ON DELETE CASCADE
);

-- Workflows (AWL templates)
CREATE TABLE workflows (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL UNIQUE,
    version TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL,
    content TEXT NOT NULL
);

-- Indexes
CREATE INDEX idx_prds_status ON prds(status);
CREATE INDEX idx_task_specs_prd_id ON task_specs(prd_id);
CREATE INDEX idx_task_specs_status ON task_specs(status);
CREATE INDEX idx_executions_ts_id ON executions(ts_id);
CREATE INDEX idx_executions_status ON executions(status);
```

### Schema Updates

When adding new fields:

1. Write migration function:
```rust
fn migrate_v1_to_v2(conn: &Connection) -> Result<()> {
    conn.execute(
        "ALTER TABLE task_specs ADD COLUMN workflow_name TEXT",
        [],
    )?;
    Ok(())
}
```

2. Update `.version` file:
```rust
const CURRENT_VERSION: u32 = 2;

pub fn migrate(store_path: &Path) -> Result<()> {
    let version_file = store_path.join(".version");
    let current = read_version(&version_file)?;

    if current < 2 {
        migrate_v1_to_v2(&conn)?;
        write_version(&version_file, 2)?;
    }

    Ok(())
}
```

3. Rebuild from JSONL:
```rust
store.sync()?;  // Rebuilds SQLite from JSONL
```

## 4. JSONL Patterns

### Append-Only Writes

**Every update appends a new line:**

```rust
pub fn update_execution(&mut self, exec_id: &str, exec: Execution) -> Result<()> {
    // 1. Append to JSONL (source of truth)
    let json = serde_json::to_string(&exec)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(self.base_path.join("executions.jsonl"))?;
    writeln!(file, "{}", json)?;
    file.sync_all()?;  // fsync

    // 2. Update SQLite (derived cache)
    self.db.execute(
        "UPDATE executions SET status=?1, updated_at=?2, iteration_count=?3 WHERE id=?4",
        params![exec.status.to_string(), exec.updated_at, exec.iteration_count, exec.id],
    )?;

    Ok(())
}
```

**Result:** Multiple lines with same ID in JSONL:
```jsonl
{"id":"exec-001","iteration_count":1,"updated_at":1000}
{"id":"exec-001","iteration_count":2,"updated_at":1001}
{"id":"exec-001","iteration_count":3,"updated_at":1002}
```

### Sync: JSONL → SQLite

```rust
pub fn sync(&mut self) -> Result<()> {
    // 1. Read all JSONL
    let executions = self.read_jsonl::<Execution>("executions.jsonl")?;

    // 2. Deduplicate: Keep latest per ID (highest updated_at)
    let mut latest: HashMap<String, Execution> = HashMap::new();
    for exec in executions {
        match latest.get(&exec.id) {
            Some(existing) if existing.updated_at > exec.updated_at => continue,
            _ => { latest.insert(exec.id.clone(), exec); }
        }
    }

    // 3. Clear SQLite table
    self.db.execute("DELETE FROM executions", [])?;

    // 4. Insert deduplicated records
    for exec in latest.values() {
        self.db.execute(
            "INSERT INTO executions (id, ts_id, status, ...) VALUES (?1, ?2, ?3, ...)",
            params![exec.id, exec.ts_id, exec.status.to_string()],
        )?;
    }

    Ok(())
}
```

### Compaction (Optional)

Remove superseded records to reclaim space:

```bash
taskstore compact
```

```rust
pub fn compact(&mut self, filename: &str) -> Result<()> {
    // 1. Read and deduplicate
    let records = self.read_jsonl::<Execution>(filename)?;
    let latest = deduplicate_by_id(records);

    // 2. Write to temp file
    let temp = format!("{}.tmp", filename);
    let mut file = File::create(&temp)?;
    for record in latest.values() {
        writeln!(file, "{}", serde_json::to_string(record)?)?;
    }
    file.sync_all()?;

    // 3. Atomic rename
    fs::rename(temp, filename)?;

    Ok(())
}
```

## 5. Git Integration (Layer 2)

### Overview

Git integration is CRITICAL for TaskStore. This is Layer 2 of the architecture:

```
Layer 1: Core task graph (CRUD, dependencies, status, SQLite + JSONL)
    ↓
Layer 2: Git integration (merge driver, hooks, debouncing) ← THIS LAYER
    ↓
Layer 3: Orchestration (comments, assignments, federation) ← TaskDaemon provides this
```

**Why Layer 2 is critical:**
- Without custom merge driver: Concurrent PRD creation = merge conflicts requiring manual resolution
- Without git hooks: Database-JSONL inconsistencies, manual sync commands needed
- Without debouncing: Poor performance (100 creates = 100 JSONL writes)

**Lesson from Engram:** Engram mistakenly removed Layer 2 when trying to remove gastown coupling. These features are NOT orchestration - they're core git integration that makes git-backed storage actually work.

### 5.1. Custom Merge Driver (CRITICAL)

**What it does:** Automatically resolves JSONL conflicts using field-level three-way merging.

**Why it's needed:**
```
Scenario: Two developers create PRDs simultaneously

Without merge driver:
  git merge → CONFLICT (line-based merge fails) → Manual resolution required

With merge driver:
  git merge → Automatic resolution (by ID, latest wins) → No conflict
```

**Implementation:**

```rust
// src/merge.rs

use crate::types::{Prd, TaskSpec, Execution};
use eyre::{Context, Result};
use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::Path;

/// Merge JSONL files using three-way merge strategy.
///
/// Algorithm:
/// 1. Parse ancestor, ours, theirs into records
/// 2. Build ID maps (last occurrence wins per file)
/// 3. For each ID present in ours or theirs:
///    - Both modified: Pick latest by updated_at
///    - Only ours: Keep ours
///    - Only theirs: Keep theirs
/// 4. Serialize merged records back to JSONL
pub fn merge_jsonl_files(
    ancestor_path: &Path,
    ours_path: &Path,
    theirs_path: &Path,
) -> Result<String> {
    // Auto-detect record type from filename
    let filename = ours_path.file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("");

    if filename.contains("prds") {
        merge_typed::<Prd>(ancestor_path, ours_path, theirs_path)
    } else if filename.contains("task_specs") {
        merge_typed::<TaskSpec>(ancestor_path, ours_path, theirs_path)
    } else if filename.contains("executions") {
        merge_typed::<Execution>(ancestor_path, ours_path, theirs_path)
    } else if filename.contains("dependencies") {
        merge_typed::<Dependency>(ancestor_path, ours_path, theirs_path)
    } else {
        Err(eyre::eyre!("Unknown JSONL file type: {}", filename))
    }
}

/// Generic merge for any record type with id and updated_at
fn merge_typed<T>(ancestor: &Path, ours: &Path, theirs: &Path) -> Result<String>
where
    T: serde::Serialize + serde::de::DeserializeOwned + HasId + HasTimestamp + Clone,
{
    // 1. Parse all three files
    let ancestor_records = parse_jsonl::<T>(ancestor)?;
    let ours_records = parse_jsonl::<T>(ours)?;
    let theirs_records = parse_jsonl::<T>(theirs)?;

    // 2. Build ID maps (last occurrence wins - handles append-only JSONL)
    let ancestor_map = build_latest_map(ancestor_records);
    let ours_map = build_latest_map(ours_records);
    let theirs_map = build_latest_map(theirs_records);

    // 3. Three-way merge
    let all_ids: HashSet<String> = ours_map.keys()
        .chain(theirs_map.keys())
        .cloned()
        .collect();

    let mut merged = Vec::new();

    for id in all_ids {
        match (ours_map.get(&id), theirs_map.get(&id), ancestor_map.get(&id)) {
            // Both branches modified the record
            (Some(ours_rec), Some(theirs_rec), Some(_ancestor_rec)) => {
                // Pick the one with latest updated_at (last-write-wins)
                if ours_rec.updated_at() >= theirs_rec.updated_at() {
                    merged.push(ours_rec.clone());
                } else {
                    merged.push(theirs_rec.clone());
                }
            }
            // Only ours added/modified
            (Some(ours_rec), None, _) => {
                merged.push(ours_rec.clone());
            }
            // Only theirs added/modified
            (None, Some(theirs_rec), _) => {
                merged.push(theirs_rec.clone());
            }
            // Both deleted (rare, but handle gracefully)
            (None, None, Some(_)) => {
                // Don't include deleted records
            }
            // Unreachable (ID must be in at least one branch)
            (None, None, None) => unreachable!(),
        }
    }

    // 4. Sort by ID for deterministic output
    merged.sort_by(|a, b| a.id().cmp(&b.id()));

    // 5. Serialize to JSONL
    let jsonl = merged.iter()
        .map(|r| serde_json::to_string(r).unwrap())
        .collect::<Vec<_>>()
        .join("\n");

    Ok(jsonl)
}

/// Parse JSONL file, handling multiple occurrences of same ID
fn parse_jsonl<T>(path: &Path) -> Result<Vec<T>>
where
    T: serde::de::DeserializeOwned,
{
    if !path.exists() {
        return Ok(Vec::new());
    }

    let content = fs::read_to_string(path)
        .with_context(|| format!("Failed to read {}", path.display()))?;

    let mut records = Vec::new();
    for (line_num, line) in content.lines().enumerate() {
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<T>(line) {
            Ok(record) => records.push(record),
            Err(e) => {
                eprintln!("Warning: Failed to parse line {} in {}: {}", line_num + 1, path.display(), e);
            }
        }
    }

    Ok(records)
}

/// Build map of ID → Record, keeping only the latest occurrence
fn build_latest_map<T>(records: Vec<T>) -> HashMap<String, T>
where
    T: HasId + HasTimestamp,
{
    let mut map = HashMap::new();
    for record in records {
        let id = record.id();
        match map.get(&id) {
            Some(existing) if existing.updated_at() > record.updated_at() => {
                // Keep existing (it's newer)
                continue;
            }
            _ => {
                // Insert or replace with this record (it's newer or first)
                map.insert(id, record);
            }
        }
    }
    map
}

/// Trait for records with an ID field
pub trait HasId {
    fn id(&self) -> String;
}

/// Trait for records with an updated_at timestamp
pub trait HasTimestamp {
    fn updated_at(&self) -> i64;
}

// Implementations for TaskStore types
impl HasId for Prd {
    fn id(&self) -> String { self.id.clone() }
}

impl HasTimestamp for Prd {
    fn updated_at(&self) -> i64 { self.updated_at }
}

// Similar implementations for TaskSpec, Execution, Dependency...
```

**Binary for git merge driver:**

```rust
// src/bin/taskstore-merge.rs

use taskstore::merge::merge_jsonl_files;
use std::env;
use std::fs;
use std::path::Path;

fn main() {
    if let Err(e) = run() {
        eprintln!("Merge failed: {}", e);
        std::process::exit(1);
    }
}

fn run() -> eyre::Result<()> {
    let args: Vec<String> = env::args().collect();

    if args.len() != 4 {
        eyre::bail!("Usage: taskstore-merge <ancestor> <ours> <theirs>");
    }

    let ancestor = Path::new(&args[1]);
    let ours = Path::new(&args[2]);
    let theirs = Path::new(&args[3]);

    // Perform three-way merge
    let merged = merge_jsonl_files(ancestor, ours, theirs)?;

    // Write result to "ours" file (git merge driver convention)
    fs::write(ours, merged)?;

    Ok(())
}
```

**Installation:**

```rust
impl Store {
    pub fn install_merge_driver(&self) -> Result<()> {
        // 1. Configure git to use our merge driver
        Command::new("git")
            .args(["config", "merge.taskstore-merge.name", "TaskStore JSONL merge driver"])
            .output()
            .context("Failed to set merge driver name")?;

        Command::new("git")
            .args(["config", "merge.taskstore-merge.driver", "taskstore-merge %O %A %B"])
            .output()
            .context("Failed to set merge driver command")?;

        // 2. Configure .gitattributes to use merge driver for JSONL files
        let gitattributes = ".taskstore/*.jsonl merge=taskstore-merge\n";
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(".gitattributes")
            .context("Failed to open .gitattributes")?;
        file.write_all(gitattributes.as_bytes())?;

        println!("✓ Installed custom merge driver for .taskstore/*.jsonl");

        Ok(())
    }
}
```

**Testing the merge driver:**

```bash
# Create test scenario
cd /tmp
mkdir merge-test && cd merge-test
git init

# Setup
taskstore init
taskstore install-merge-driver

# Create PRD on main
taskstore create-prd "Feature A" > /dev/null
git add .taskstore
git commit -m "Add Feature A"

# Branch and create different PRD
git checkout -b branch1
taskstore create-prd "Feature B" > /dev/null
git add .taskstore
git commit -m "Add Feature B"

# Back to main, create another PRD
git checkout main
taskstore create-prd "Feature C" > /dev/null
git add .taskstore
git commit -m "Add Feature C"

# Merge - should auto-resolve with no conflicts
git merge branch1

# Verify all three PRDs exist
taskstore list-prds
# Expected: Feature A, Feature B, Feature C (no conflicts!)
```

### 5.2. Git Hooks (IMPORTANT)

**What they do:** Automate sync operations to keep SQLite and JSONL in sync.

**Why they're needed:**
- Prevent database-JSONL inconsistencies
- No manual `taskstore sync` commands needed
- Works transparently with standard git workflows

**Complete hook set:**

```rust
impl Store {
    pub fn install_git_hooks(&self) -> Result<()> {
        self.install_hook("pre-commit", "taskstore sync")?;
        self.install_hook("post-merge", "taskstore sync")?;
        self.install_hook("post-rebase", "taskstore sync")?;
        self.install_hook("pre-push", "taskstore sync")?;
        self.install_hook("post-checkout", "taskstore sync")?;

        println!("✓ Installed git hooks (pre-commit, post-merge, post-rebase, pre-push, post-checkout)");

        Ok(())
    }

    fn install_hook(&self, hook_name: &str, command: &str) -> Result<()> {
        let hook_content = format!(
            "#!/bin/bash\n\
             # TaskStore auto-sync hook\n\
             cd \"$(git rev-parse --show-toplevel)\"\n\
             {} || true  # Don't fail git operation if sync fails\n\
             exit 0\n",
            command
        );

        let hook_path = PathBuf::from(".git/hooks").join(hook_name);

        // Append if hook already exists (don't overwrite user's hooks)
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&hook_path)
            .with_context(|| format!("Failed to open hook {}", hook_name))?;

        file.write_all(hook_content.as_bytes())?;

        // Make executable (Unix only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&hook_path)?.permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&hook_path, perms)?;
        }

        Ok(())
    }
}
```

**Hook purposes:**

| Hook | When It Runs | Purpose |
|------|--------------|---------|
| **pre-commit** | Before `git commit` | Ensure all mutations are flushed to JSONL before commit |
| **post-merge** | After `git merge` | Rebuild SQLite cache from merged JSONL |
| **post-rebase** | After `git rebase` | Rebuild SQLite cache from rebased JSONL |
| **pre-push** | Before `git push` | Final sync to ensure everything exported |
| **post-checkout** | After `git checkout` | Rebuild SQLite cache when switching branches |

**Why all 5 hooks:**
- `pre-commit`: Prevents committing stale JSONL (if using debounced export)
- `post-merge`: Imports changes from remote
- `post-rebase`: Imports changes after rebase
- `pre-push`: Safety check before pushing
- `post-checkout`: Branch switching needs cache rebuild

**Installation:**

```bash
# One-time setup per repo
taskstore install-hooks
```

**Usage:**

```bash
# One-time setup per repo
taskstore install-hooks

# Verify installation
ls -la .git/hooks/post-merge
cat .gitattributes
```

### 5.3. Debounced Export/Sync (PERFORMANCE)

**What it does:** Batches multiple mutations into a single JSONL write to improve performance and reduce commit spam.

**Why it's needed:**
```
Without debouncing:
  50 PRD creates in 5 seconds = 50 JSONL appends = 50 fsync calls = poor performance

With debouncing:
  50 PRD creates in 5 seconds = Wait 5s → 1 JSONL export = 1 fsync call = good performance
```

**When to use:**
- Batch operations (creating many PRDs/TSs at once)
- Rapid mutations (agent creating tasks quickly)
- Not needed for single operations (overhead not worth it)

**Implementation:**

```rust
// src/sync.rs

use std::collections::HashSet;
use std::time::Duration;
use tokio::sync::mpsc;

/// Configuration for debounced sync
#[derive(Debug, Clone)]
pub struct SyncConfig {
    /// Debounce interval in milliseconds (default: 5000)
    pub debounce_ms: u64,
    /// Auto-export on mutations (default: true)
    pub auto_export: bool,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            debounce_ms: 5000,  // 5 seconds
            auto_export: true,
        }
    }
}

/// Sync manager for debounced JSONL export
pub struct SyncManager {
    dirty_tx: mpsc::UnboundedSender<DirtyRecord>,
}

#[derive(Debug, Clone)]
enum DirtyRecord {
    Prd(String),
    TaskSpec(String),
    Execution(String),
    Flush,  // Force immediate flush
}

impl SyncManager {
    pub fn new(store_path: PathBuf, config: SyncConfig) -> Self {
        let (dirty_tx, dirty_rx) = mpsc::unbounded_channel();

        // Spawn background task
        tokio::spawn(sync_worker(store_path, dirty_rx, config));

        Self { dirty_tx }
    }

    /// Mark a PRD as dirty (will be exported on next flush)
    pub fn mark_prd_dirty(&self, id: String) {
        let _ = self.dirty_tx.send(DirtyRecord::Prd(id));
    }

    /// Force immediate flush (bypass debounce)
    pub fn flush_now(&self) {
        let _ = self.dirty_tx.send(DirtyRecord::Flush);
    }
}

/// Background worker that batches and exports dirty records
async fn sync_worker(
    store_path: PathBuf,
    mut dirty_rx: mpsc::UnboundedReceiver<DirtyRecord>,
    config: SyncConfig,
) {
    let mut dirty_prds = HashSet::new();
    let mut dirty_task_specs = HashSet::new();
    let mut dirty_executions = HashSet::new();

    let mut timer = tokio::time::interval(Duration::from_millis(config.debounce_ms));

    loop {
        tokio::select! {
            Some(record) = dirty_rx.recv() => {
                match record {
                    DirtyRecord::Prd(id) => {
                        dirty_prds.insert(id);
                        timer.reset();  // Reset debounce timer
                    }
                    DirtyRecord::Flush => {
                        // Force immediate flush
                        export_dirty(&store_path, &dirty_prds, &dirty_task_specs, &dirty_executions).await.ok();
                        dirty_prds.clear();
                        dirty_task_specs.clear();
                        dirty_executions.clear();
                    }
                    // ... handle other record types
                }
            }
            _ = timer.tick() => {
                // Timer fired - export if dirty
                if !dirty_prds.is_empty() {
                    export_dirty(&store_path, &dirty_prds, &dirty_task_specs, &dirty_executions).await.ok();
                    dirty_prds.clear();
                    dirty_task_specs.clear();
                    dirty_executions.clear();
                }
            }
        }
    }
}
```

**Usage:**

```rust
// Default (no debouncing)
let store = Store::open(path)?;

// With debouncing (5s default)
let store = Store::with_sync_config(path, SyncConfig::default())?;
```

## 6. Excluded Features (Layer 3 - Orchestration)

### Overview

The following features are **intentionally excluded** from TaskStore because they belong in Layer 3 (Orchestration), which TaskDaemon provides.

This section documents these features for future reference, in case we need to bring them back or understand why they were excluded.

**Key principle:** TaskStore is a **library** (Layer 1 + Layer 2), not an **application** (Layer 1 + Layer 2 + Layer 3).

### 6.1. Comments (Inter-Loop Communication)

**Why Beads has it:** Multi-agent teams need coordination, async communication channel

**Why TaskStore excludes it (CORRECT):** TaskDaemon provides inter-loop messaging via Coordinator (Notify/Query/Share)

**Data model (for reference):**
```sql
CREATE TABLE comments (
    id TEXT PRIMARY KEY,
    item_id TEXT NOT NULL,
    author TEXT NOT NULL,
    content TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
```

### 6.2. Assignments (Work Distribution)

**Why Beads has it:** Multi-user collaboration, clear ownership of work

**Why TaskStore excludes it (CORRECT):** TaskDaemon provides work distribution via Scheduler. Dynamic assignment is better than static.

**Note:** `executions.assigned_to` exists for "who is currently working on it" (runtime), not pre-assignment (orchestration).

### 6.3. Federation (Multi-Repo Coordination)

**Why Beads has it:** Large projects span multiple repos, cross-repo dependencies

**Why TaskStore excludes it (CORRECT):** TaskDaemon can manage multiple TaskStore instances. Federation is orchestration, not storage.

### 6.4. Semantic Compaction (LLM Context Management)

**Why Beads has it:** Automatically summarize old/closed tasks to reduce token count for LLM context

**Why TaskStore excludes it (CORRECT):** Context management is TaskDaemon's responsibility. TaskStore just stores data.

### 6.5. Daemon Monitoring (Health Checks, Observability)

**Why Beads has it:** Production reliability, event-driven daemon needs monitoring

**Why TaskStore excludes it (CORRECT):** TaskDaemon IS the daemon. TaskStore is a library embedded in TaskDaemon. Monitoring is TaskDaemon's responsibility (via TUI).

### 6.6. Time Tracking & Estimates

**Why Beads has it:** Project planning, agent performance tracking

**Why TaskStore excludes it (CORRECT):** Not core to execution model. TaskDaemon can add if needed.

**Note:** We already have `started_at` and `completed_at` in executions (sufficient for duration calculation).

### 6.7. Protected Branches & Git Workflows

**Why Beads has it:** Production safety, protected main branch, work on sync-branch

**Why TaskStore excludes it (CORRECT):** Git workflows are repository policy, not storage concern. Configure at GitHub/GitLab level.

### 6.8. Event-Driven Daemon (File Watching)

**Why Beads has it:** Multi-user real-time collaboration, sub-500ms sync latency

**Why TaskStore excludes it (OPTIONAL - MAY SKIP):** TaskDaemon is single-process (state manager owns Store). No concurrent external access. Git hooks handle post-merge sync.

**Decision:** Skip for now. Revisit if multi-process access needed.

### Summary of Exclusions

| Feature | Beads | Engram | TaskStore | Owner |
|---------|-------|--------|-----------|-------|
| Comments | ✅ | ❌ | ❌ | TaskDaemon Coordinator |
| Assignments | ✅ | ❌ | ❌ | TaskDaemon Scheduler |
| Federation | ✅ | ❌ | ❌ | TaskDaemon (multi-store) |
| Semantic Compaction | ✅ | ❌ | ❌ | TaskDaemon (prompt building) |
| Daemon Monitoring | ✅ | ❌ | ❌ | TaskDaemon TUI |
| Time Tracking | ✅ | ❌ | ❌ | TaskDaemon (optional) |
| Protected Branches | ✅ | ❌ | ❌ | Git/GitHub config |
| Event-Driven Daemon | ✅ | ❌ | ❌ | Probably skip |

**Key takeaway:** All Layer 3 features belong in TaskDaemon, not TaskStore. TaskStore focuses on Layer 1 (core) + Layer 2 (git integration).

## 7. Testing Strategy

### Unit Tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_create_prd() {
        let mut store = Store::open_temp()?;
        let prd = Prd {
            id: "prd-001".to_string(),
            title: "Test PRD".to_string(),
            // ...
        };

        let id = store.create_prd(prd)?;
        assert_eq!(id, "prd-001");

        let retrieved = store.get_prd(&id)?.unwrap();
        assert_eq!(retrieved.title, "Test PRD");
    }

    #[test]
    fn test_sync_deduplicates() {
        let mut store = Store::open_temp()?;

        // Create multiple versions of same record
        store.append_jsonl("executions.jsonl", &exec_v1)?;
        store.append_jsonl("executions.jsonl", &exec_v2)?;
        store.append_jsonl("executions.jsonl", &exec_v3)?;

        // Sync should keep only latest
        store.sync()?;

        let exec = store.get_execution("exec-001")?.unwrap();
        assert_eq!(exec.iteration_count, 3);  // v3 is latest
    }
}
```

### Integration Tests

```rust
#[test]
fn test_merge_driver() {
    // Create conflicting JSONL files
    let base = "base.jsonl";
    let ours = "ours.jsonl";
    let theirs = "theirs.jsonl";

    // Simulate merge
    let result = merge_jsonl_files(base, ours, theirs)?;

    // Verify correct resolution
    assert!(result.contains("latest version"));
}

#[test]
fn test_git_hook_sync() {
    // Initialize repo with taskstore
    let repo = init_test_repo()?;
    store.install_git_integration()?;

    // Simulate post-merge
    run_hook("post-merge")?;

    // Verify sync was called
    assert!(store.is_synced()?);
}
```

## 7. CLI Commands

**Full command set:**

```bash
# List operations
taskstore list-prds [--status STATUS]
taskstore list-task-specs [--prd-id ID]
taskstore list-executions [--status STATUS] [--limit N]

# Show operations
taskstore show <id>           # Auto-detects type
taskstore describe <id>       # Show full markdown content

# Maintenance
taskstore sync                # Rebuild SQLite from JSONL
taskstore compact             # Remove superseded JSONL records
taskstore check               # Validate consistency

# Git integration
taskstore install-hooks       # Install git hooks and merge driver
taskstore merge <base> <ours> <theirs>  # Merge driver (internal)

# Export
taskstore backup <dest>       # Copy JSONL files
taskstore export-json         # Export all data as JSON
```

## 8. Performance Considerations

### Optimization Strategies

1. **SQLite WAL mode:**
```rust
conn.execute_batch("PRAGMA journal_mode=WAL")?;
```

2. **Batch writes:**
```rust
let tx = conn.transaction()?;
for record in records {
    tx.execute("INSERT INTO ...", ...)?;
}
tx.commit()?;
```

3. **Streaming JSONL reads:**
```rust
// Don't load entire file into memory
let file = BufReader::new(File::open(path)?);
for line in file.lines() {
    let record: Record = serde_json::from_str(&line?)?;
    process(record)?;
}
```

4. **Lazy markdown loading:**
```rust
// Don't load .md files until requested
pub fn get_prd_content(&self, prd_id: &str) -> Result<String> {
    let prd = self.get_prd(prd_id)?.unwrap();
    let path = self.base_path.join("prds").join(&prd.file);
    fs::read_to_string(path)
}
```

## 9. Error Handling Patterns

```rust
// Use eyre for context
pub fn create_prd(&mut self, prd: Prd) -> Result<String> {
    // Validate
    if prd.title.is_empty() {
        return Err(eyre!("PRD title cannot be empty"));
    }

    // Write JSONL first (source of truth)
    self.append_jsonl("prds.jsonl", &prd)
        .wrap_err("Failed to write PRD to JSONL")?;

    // Then SQLite
    self.db.execute(
        "INSERT INTO prds (id, title, ...) VALUES (?1, ?2, ...)",
        params![prd.id, prd.title],
    ).wrap_err("Failed to insert PRD into SQLite")?;

    Ok(prd.id)
}
```

## 10. References

- [Storage Architecture](./storage-architecture.md) - Bead Store pattern
- [TaskStore Design](./taskstore-design.md) - Full API and schema
- SQLite documentation: https://www.sqlite.org/
- JSONL format: https://jsonlines.org/
