# Comparative Analysis: TaskStore vs Beads vs Engram

**Date:** 2026-01-13
**Purpose:** Ensure TaskStore learns from both Beads and Engram to build the right thing

## Executive Summary

**Critical Finding:** TaskStore's current design is closer to Engram than Beads, which means **we're at risk of repeating Engram's critical mistakes**.

### The Three Systems at a Glance

| System | Author | Language | LOC | Status | Philosophy |
|--------|--------|----------|-----|--------|------------|
| **Beads** | Steve Yegge | Go | ~10K+ | Production | Full-featured issue tracker with deep git integration |
| **Engram** | Scott Aidler | Rust | ~4K | v0.1.2 | Minimal task graph (accidentally minimal from removing gastown) |
| **TaskStore** | Planned | Rust | TBD | Design | Planned as foundation for TaskDaemon orchestrator |

### The Core Pattern (All Three Share)

```
SQLite (fast query cache, .gitignore)
    ↓
JSONL (git-tracked source of truth, append-only)
    ↓
Git (distribution layer, version control)
```

All three implement the "Bead Store" pattern. The differences lie in **what else** they include.

## Critical Lessons: What Engram Got Wrong

From the "Accidental Minimalism" document, Engram's agent mistakenly removed features that seemed like "orchestration" but were actually **core git integration**:

### Mistakes to Avoid (Engram's Errors)

| Feature | Engram Removed It? | Why Wrong | Impact | TaskStore Status |
|---------|-------------------|-----------|--------|------------------|
| **Custom merge driver** | ❌ YES | Seemed like orchestration, but it's pure function for field-level merging | Manual conflict resolution, line-based git fails | ⚠️ **NOT IN PLAN** |
| **Git hooks** | ❌ YES | Seemed like gastown intercept, but it's workflow automation | Manual `sync` before every commit, inconsistencies | ⚠️ **MENTIONED BUT UNCLEAR** |
| **Incremental export** | ❌ YES | FlushManager looked complex, but it's performance optimization | 100 creates = 100 JSONL appends, commit spam | ⚠️ **NOT IN PLAN** |
| **Comments** | ❌ YES | Inter-agent communication, but also useful for humans | No way to annotate tasks | ✅ **Correctly excluded** |
| **Assignments** | ❌ YES | Work distribution (orchestrator's job) | No ownership tracking | ✅ **Correctly excluded** |

### What This Means for TaskStore

**Current TaskStore design matches Engram's mistakes:**
- ✅ Has JSONL + SQLite + Git (good)
- ⚠️ No mention of custom merge driver in implementation guide
- ⚠️ Git hooks mentioned but not detailed
- ❌ No incremental export/debouncing mentioned
- ✅ Correctly excludes comments/assignments (TaskDaemon handles these)

**We need to add the git integration features that Engram wrongly removed.**

---

## The Three Layers of Features

### Layer 1: Core Task Graph (All Three Have This)

**What it is:** Pure data structure operations

| Feature | Beads | Engram | TaskStore (Planned) |
|---------|-------|--------|---------------------|
| Item CRUD | ✅ | ✅ | ✅ |
| Dependency graph (blocks, parent-child, related) | ✅ | ✅ | ✅ |
| Status transitions and validation | ✅ | ✅ | ✅ |
| SQLite + JSONL persistence | ✅ | ✅ | ✅ |
| Query filters and `ready()` | ✅ | ✅ | ✅ |
| Cycle detection | ✅ | ✅ | ✅ |
| Hash-based IDs | ✅ | ✅ | ✅ |

**Verdict:** TaskStore has Layer 1 covered. ✅

### Layer 2: Git Integration (CRITICAL - Engram Got This Wrong)

**What it is:** Making git-backed storage actually work well

| Feature | Beads | Engram | TaskStore (Planned) | **Correct Answer** |
|---------|-------|--------|---------------------|--------------------|
| **Custom merge driver** | ✅ `beads-merge` | ❌ No | ❌ **NOT MENTIONED** | **✅ MUST HAVE** |
| **Git hooks** | ✅ pre-commit, post-merge, pre-push, post-checkout | ❌ No | ⚠️ Mentioned vaguely | **✅ MUST HAVE** |
| **Incremental export/debouncing** | ✅ FlushManager | ❌ No | ❌ **NOT MENTIONED** | **✅ SHOULD HAVE** |
| **Event-driven daemon** | ✅ inotify/FSEvents | ❌ Basic | ❌ **NOT MENTIONED** | **⚠️ NICE TO HAVE** |

**Verdict:** TaskStore currently missing critical Layer 2 features. ❌

### Layer 3: Orchestration (TaskDaemon's Job, Not TaskStore's)

**What it is:** Multi-agent coordination

| Feature | Beads | Engram | TaskStore (Planned) | **Correct Answer** |
|---------|-------|--------|---------------------|--------------------|
| Comments (inter-agent messaging) | ✅ | ❌ | ❌ | **❌ Exclude (TaskDaemon)** |
| Assignments (work distribution) | ✅ | ❌ | ❌ | **❌ Exclude (TaskDaemon)** |
| Federation (multi-repo) | ✅ | ❌ | ❌ | **❌ Exclude (TaskDaemon)** |
| Semantic compaction (LLM context) | ✅ | ❌ | ❌ | **❌ Exclude (TaskDaemon)** |

**Verdict:** TaskStore correctly excludes Layer 3. ✅

---

## Detailed Comparison: Feature by Feature

### 1. Custom Merge Driver (CRITICAL GAP)

#### What Beads Does

```bash
# .git/config
[merge "beads-merge"]
    name = Beads JSONL merge driver
    driver = bd merge %O %A %B

# .gitattributes
.beads/issues.jsonl merge=beads-merge
```

**The merge driver:**
- Parses JSONL from ancestor, ours, theirs
- Builds ID maps for each
- Three-way merge: For each ID, pick latest by `updated_at`
- Field-level merging (not line-based)
- Outputs merged JSONL
- **Result:** Zero conflict commits, automatic resolution

#### What Engram Does

❌ **Nothing.** Relies on line-based git merging. Conflicts require manual resolution.

#### What TaskStore Plans

**From `taskstore-design.md`:**
> "Custom git merge driver handles JSONL conflicts automatically"

**From `storage-architecture.md`:**
> "Git merge driver: Three-way merge for JSONL files (by ID, latest wins)"

**From `implementation-guide.md`:**
> ⚠️ **NOT COVERED**

**The Problem:** Design mentions it, but **no implementation detail, no algorithms, no examples**.

#### What We Must Do

**Add to `implementation-guide.md`:**

```rust
// taskstore/src/merge.rs

pub fn merge_jsonl_files(
    ancestor_path: &Path,
    ours_path: &Path,
    theirs_path: &Path,
) -> Result<String> {
    // 1. Parse all three files
    let ancestor_records: Vec<Prd> = parse_jsonl(ancestor_path)?;
    let ours_records: Vec<Prd> = parse_jsonl(ours_path)?;
    let theirs_records: Vec<Prd> = parse_jsonl(theirs_path)?;

    // 2. Build ID maps (last occurrence wins)
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
            // Both modified: Pick latest by updated_at
            (Some(ours), Some(theirs), _) => {
                if ours.updated_at >= theirs.updated_at {
                    merged.push(ours.clone());
                } else {
                    merged.push(theirs.clone());
                }
            }
            // Only ours modified
            (Some(ours), None, _) => merged.push(ours.clone()),
            // Only theirs modified
            (None, Some(theirs), _) => merged.push(theirs.clone()),
            // Both deleted (shouldn't happen, but handle gracefully)
            (None, None, _) => {}
        }
    }

    // 4. Serialize
    Ok(merged.iter()
        .map(|r| serde_json::to_string(r).unwrap())
        .collect::<Vec<_>>()
        .join("\n"))
}

fn build_latest_map<T: HasId + HasTimestamp>(records: Vec<T>) -> HashMap<String, T> {
    let mut map = HashMap::new();
    for record in records {
        match map.get(&record.id()) {
            Some(existing) if existing.updated_at() > record.updated_at() => continue,
            _ => { map.insert(record.id(), record); }
        }
    }
    map
}
```

**Install command:**
```rust
impl Store {
    pub fn install_merge_driver(&self) -> Result<()> {
        // 1. Configure git
        Command::new("git")
            .args(["config", "merge.taskstore-merge.name", "TaskStore JSONL merge"])
            .output()?;

        Command::new("git")
            .args(["config", "merge.taskstore-merge.driver", "taskstore merge %O %A %B"])
            .output()?;

        // 2. Write .gitattributes
        let gitattributes = ".taskstore/*.jsonl merge=taskstore-merge\n";
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(".gitattributes")?;
        file.write_all(gitattributes.as_bytes())?;

        Ok(())
    }
}
```

**Why this matters:**
- Without merge driver: Concurrent PRD creation = merge conflicts
- With merge driver: Concurrent PRD creation = automatic merge
- **This is the whole point of "git-backed" vs "git-annoying"**

---

### 2. Git Hooks (IMPORTANT GAP)

#### What Beads Does

```bash
# .git/hooks/pre-commit
#!/bin/bash
bd sync  # Flush pending changes to JSONL

# .git/hooks/post-merge
#!/bin/bash
bd import  # Rebuild SQLite from updated JSONL

# .git/hooks/pre-push
#!/bin/bash
bd sync  # Ensure everything exported

# .git/hooks/post-checkout
#!/bin/bash
bd import  # Rebuild after branch switch
```

**Result:** SQLite always in sync, no manual commands needed.

#### What Engram Does

❌ **Nothing.** User must run `eg sync` manually before every commit.

#### What TaskStore Plans

**From `implementation-guide.md`:**
> "Git hooks: post-merge and post-rebase call taskstore sync"

**The Problem:** Only mentions 2 hooks, missing pre-commit and pre-push. No implementation detail.

#### What We Must Do

**Add complete hook set to `implementation-guide.md`:**

```rust
impl Store {
    pub fn install_hooks(&self) -> Result<()> {
        self.install_hook("pre-commit", "taskstore sync")?;
        self.install_hook("post-merge", "taskstore sync")?;
        self.install_hook("post-rebase", "taskstore sync")?;
        self.install_hook("pre-push", "taskstore sync")?;
        self.install_hook("post-checkout", "taskstore sync")?;
        Ok(())
    }

    fn install_hook(&self, hook_name: &str, command: &str) -> Result<()> {
        let hook_content = format!(
            "#!/bin/bash\ncd \"$(git rev-parse --show-toplevel)\"\n{}\nexit 0\n",
            command
        );

        let hook_path = PathBuf::from(".git/hooks").join(hook_name);

        // Append if exists, create if not
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&hook_path)?;
        file.write_all(hook_content.as_bytes())?;

        // Make executable (Unix)
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

**Why this matters:**
- Prevents database-JSONL inconsistencies
- Automates git best practices
- No manual `sync` commands needed

---

### 3. Incremental Export / Debouncing (PERFORMANCE GAP)

#### What Beads Does

**FlushManager pattern:**
```go
type FlushManager struct {
    markDirtyCh   chan string       // Buffer dirty IDs
    timerFiredCh  chan struct{}     // Debounce timer
    flushNowCh    chan struct{}     // Force flush
    shutdownCh    chan struct{}
}

func (fm *FlushManager) Run() {
    dirty := make(map[string]bool)
    timer := time.NewTimer(5 * time.Second)

    for {
        select {
        case id := <-fm.markDirtyCh:
            dirty[id] = true
            timer.Reset(5 * time.Second)  // Debounce

        case <-timer.C:
            if len(dirty) > 0 {
                exportToJSONL(dirty)
                dirty = make(map[string]bool)
            }

        case <-fm.flushNowCh:
            exportToJSONL(dirty)
            dirty = make(map[string]bool)

        case <-fm.shutdownCh:
            exportToJSONL(dirty)  // Final flush
            return
        }
    }
}
```

**Result:**
- 100 creates in 5 seconds = 1 JSONL export
- No commit spam
- Batched I/O

#### What Engram Does

```rust
pub fn append_item(&mut self, item: &Item) -> Result<()> {
    // Write to JSONL immediately
    let mut file = OpenOptions::new()
        .append(true)
        .open(&items_path)?;
    writeln!(file, "{}", serde_json::to_string(item)?)?;
    file.sync_all()?;  // fsync every time

    // Update SQLite immediately
    self.insert_item_to_db(item)?;

    Ok(())
}
```

**Result:**
- 100 creates = 100 JSONL appends
- 100 fsync calls
- Poor performance

#### What TaskStore Plans

**From design docs:**
❌ **NOT MENTIONED ANYWHERE**

Current design matches Engram's immediate-write approach.

#### What We Should Do

**Add to `implementation-guide.md`:**

```rust
// Simple debouncing (much simpler than Beads FlushManager)

pub struct SyncConfig {
    pub debounce_ms: u64,
    pub auto_export: bool,
}

impl Store {
    pub fn new_with_sync(root: &Path, config: SyncConfig) -> Result<Self> {
        let mut store = Self::open(root)?;

        if config.auto_export {
            let (dirty_tx, dirty_rx) = mpsc::channel(100);
            store.dirty_tx = Some(dirty_tx);
            store.spawn_sync_task(dirty_rx, config.debounce_ms);
        }

        Ok(store)
    }

    fn spawn_sync_task(&self, mut dirty_rx: mpsc::Receiver<String>, debounce_ms: u64) {
        let store_path = self.base_path.clone();

        tokio::spawn(async move {
            let mut dirty = HashSet::new();
            let mut timer = tokio::time::interval(Duration::from_millis(debounce_ms));

            loop {
                tokio::select! {
                    Some(id) = dirty_rx.recv() => {
                        dirty.insert(id);
                        timer.reset();
                    }
                    _ = timer.tick() => {
                        if !dirty.is_empty() {
                            // Export dirty items to JSONL
                            export_dirty(&store_path, &dirty).await?;
                            dirty.clear();
                        }
                    }
                }
            }
        });
    }
}
```

**Why this matters:**
- TaskDaemon will create many PRDs/TSs in rapid succession
- Without debouncing: 50 PRDs = 50 JSONL writes = commit spam
- With debouncing: 50 PRDs in 5s = 1 JSONL write = clean history

---

### 4. Storage Architecture Differences

#### JSONL Organization

| System | Structure | Rationale | Merge Strategy |
|--------|-----------|-----------|----------------|
| **Beads** | Single file (`issues.jsonl`) | All entities together, custom merge driver handles it | Field-level 3-way merge via `beads-merge` |
| **Engram** | Split files (`items.jsonl`, `edges.jsonl`) | Separate concerns, simpler to parse | Line-based git merge (conflicts likely) |
| **TaskStore** | Split files (`prds.jsonl`, `task_specs.jsonl`, `executions.jsonl`) | One file per entity type | **UNSPECIFIED** ⚠️ |

**The Question:** Should TaskStore use single-file (like Beads) or split-files (like Engram)?

**Analysis:**

**Single-file (Beads approach):**
- ✅ Merge driver can do intelligent field-level merging
- ✅ Atomic commits (all entities in one file)
- ❌ Larger files (all entities together)
- ❌ More complex parsing

**Split-files (Engram/TaskStore approach):**
- ✅ Simpler file structure
- ✅ Clearer separation of concerns
- ❌ Merge conflicts more likely (line-based)
- ❌ Need merge driver for EACH file

**Recommendation:** **Keep split files, but add merge driver for each**.

```bash
# .gitattributes
.taskstore/prds.jsonl merge=taskstore-merge
.taskstore/task_specs.jsonl merge=taskstore-merge
.taskstore/executions.jsonl merge=taskstore-merge
.taskstore/dependencies.jsonl merge=taskstore-merge
```

**Why:** Split files match TaskStore's entity model better (PRDs ≠ TSs ≠ Executions). We can still have intelligent merging.

---

### 5. ID Generation

| System | Format | Length | Collision Handling |
|--------|--------|--------|-------------------|
| **Beads** | `bd-a1b2` | Progressive (4-6 chars) | Detect collision, grow length |
| **Engram** | `eg-0a1b2c3d4e` | Fixed 10 hex chars | Virtually impossible |
| **TaskStore** | `prd-550e8400` (UUIDv7) | 8 hex chars | Virtually impossible |

**Verdict:** TaskStore's UUIDv7 approach is good. ✅

Short IDs (like Beads) are nice for humans but add complexity. TaskStore's approach is fine.

---

### 6. Daemon Architecture

| System | Auto-start? | Event-driven? | Monitoring? | Purpose |
|--------|-------------|---------------|-------------|---------|
| **Beads** | ✅ Yes | ✅ Yes (inotify) | ✅ Health checks | Production reliability |
| **Engram** | ❌ No | ❌ No | ❌ No | Basic RPC |
| **TaskStore** | ⚠️ Unknown | ⚠️ Unknown | ⚠️ Unknown | **NOT SPECIFIED** |

**What we should do:**
- Daemon is optional for TaskStore (not mentioned in design)
- TaskDaemon owns the Store directly (not via daemon)
- **Decision:** Skip daemon complexity for TaskStore. TaskDaemon's state manager owns the Store directly.

**Rationale:**
- Beads needs daemon because it's a CLI tool (multiple `bd` commands in parallel)
- TaskStore is a library embedded in TaskDaemon (single process owns Store)
- No concurrent CLI access = no need for daemon

**Verdict:** Correctly excluded. ✅

---

## Critical Gaps in TaskStore Design

### Gap 1: Custom Merge Driver (CRITICAL)

**Status:** Mentioned in design, **not in implementation guide**

**Action Required:**
1. Add `merge.rs` module to implementation guide
2. Provide complete merge algorithm
3. Show installation process
4. Include in Phase 1 implementation

**Risk if not addressed:** Concurrent PRD/TS creation = merge conflicts = manual resolution = defeats purpose of git-backed storage.

### Gap 2: Git Hooks (IMPORTANT)

**Status:** Mentioned vaguely, **incomplete**

**Action Required:**
1. Expand git hooks section in implementation guide
2. Add all 5 hooks (pre-commit, post-merge, post-rebase, pre-push, post-checkout)
3. Show complete implementation
4. Include in Phase 1 implementation

**Risk if not addressed:** Database-JSONL inconsistencies, manual sync commands needed, poor UX.

### Gap 3: Incremental Export / Debouncing (PERFORMANCE)

**Status:** **Not mentioned anywhere**

**Action Required:**
1. Add sync/flush section to implementation guide
2. Design simple debouncing (simpler than Beads FlushManager)
3. Make it optional (not required for correctness)
4. Include in Phase 2 implementation

**Risk if not addressed:** 50 PRD creates = 50 JSONL writes = poor performance, commit spam.

### Gap 4: Markdown File Storage Integration

**Status:** Mentioned, **not detailed**

**Questions to answer:**
1. When are `.md` files written? (On create? On demand? Both?)
2. What happens if JSONL exists but `.md` is missing?
3. How do we keep JSONL metadata and `.md` content in sync?
4. Do we need a merge driver for `.md` files too?

**Action Required:**
1. Clarify markdown file lifecycle in implementation guide
2. Add error handling for missing files
3. Document sync process between JSONL and `.md`

---

## Recommendations: What to Add to TaskStore

### Phase 1: Critical Git Integration (Must Have)

**Add these immediately:**

1. **Custom merge driver** (`src/merge.rs`)
   - Three-way merge algorithm for JSONL
   - Support all entity types (PRDs, TS, Executions, Dependencies)
   - Install command: `taskstore install-merge-driver`

2. **Complete git hooks**
   - pre-commit: Ensure everything flushed to JSONL
   - post-merge: Rebuild SQLite from updated JSONL
   - post-rebase: Rebuild SQLite
   - pre-push: Ensure everything exported
   - post-checkout: Rebuild SQLite after branch switch
   - Install command: `taskstore install-hooks`

3. **Markdown file lifecycle**
   - Clarify when `.md` files are written
   - Error handling for missing files
   - Sync mechanism between JSONL and `.md`

### Phase 2: Performance Optimization (Should Have)

**Add after Phase 1:**

1. **Debounced export** (`src/sync.rs`)
   - Optional feature (default: immediate write)
   - Simple timer-based debouncing (5s default)
   - Configurable via `SyncConfig`
   - Much simpler than Beads FlushManager

2. **Batch operations**
   - Already in design (good!)
   - Ensure it uses transactions
   - Ensure it debounces exports

### Phase 3: Optional Enhancements (Nice to Have)

**Add if time permits:**

1. **Blocked cache** (like Beads)
   - Materialized view for `ready()` query
   - 25x speedup for large datasets
   - Only needed if >1K items

2. **Event-driven sync** (like Beads)
   - File watching (inotify/FSEvents)
   - Auto-import on external changes
   - Only needed if multiple processes

### What NOT to Add (Correctly Excluded)

These belong in TaskDaemon, not TaskStore:

- ❌ Comments (inter-loop communication)
- ❌ Assignments (work distribution)
- ❌ Federation (multi-repo coordination)
- ❌ Semantic compaction (LLM context management)
- ❌ Daemon with monitoring (TaskDaemon is the daemon)

---

## Updated Architecture: The Right Layering

```
┌─────────────────────────────────────────────────────────────┐
│                   LAYER 3: Orchestration                     │
│                      (TaskDaemon)                            │
│  - Multi-loop coordination                                   │
│  - PRD/TS lifecycle management                               │
│  - Inter-loop messaging (Notify/Query/Share)                 │
│  - Proactive rebase                                          │
│  - TUI (monitoring and control)                              │
└──────────────────────────┬──────────────────────────────────┘
                           │
                           v
┌─────────────────────────────────────────────────────────────┐
│                 LAYER 2: Git Integration                     │
│                   (TaskStore Library)                        │
│  ✅ Custom merge driver (field-level merging)                │
│  ✅ Git hooks (pre-commit, post-merge, etc.)                 │
│  ✅ Incremental export with debouncing                       │
│  ✅ Markdown file integration                                │
│  ⚠️ Event-driven daemon (optional, probably skip)            │
└──────────────────────────┬──────────────────────────────────┘
                           │
                           v
┌─────────────────────────────────────────────────────────────┐
│                  LAYER 1: Task Graph Core                    │
│                   (TaskStore Library)                        │
│  ✅ PRD/TS/Execution CRUD                                    │
│  ✅ Dependency graph (blocks, parent-child, related)         │
│  ✅ Status transitions and validation                        │
│  ✅ SQLite + JSONL persistence                               │
│  ✅ Query filters and ready() calculation                    │
│  ✅ Cycle detection                                          │
└─────────────────────────────────────────────────────────────┘
```

**Key insight:** TaskStore needs Layer 1 + Layer 2. It currently only has Layer 1.

---

## Comparison Summary Table

| Feature | Beads | Engram | TaskStore (Planned) | **Should Be** |
|---------|-------|--------|---------------------|---------------|
| **Layer 1: Core** |
| Item CRUD | ✅ | ✅ | ✅ | ✅ |
| Dependency graph | ✅ | ✅ | ✅ | ✅ |
| Status transitions | ✅ | ✅ | ✅ | ✅ |
| SQLite + JSONL | ✅ | ✅ | ✅ | ✅ |
| Query/ready/blocked | ✅ | ✅ | ✅ | ✅ |
| Hash-based IDs | ✅ | ✅ | ✅ | ✅ |
| **Layer 2: Git Integration** |
| Custom merge driver | ✅ | ❌ | ⚠️ Mentioned | **✅ ADD** |
| Git hooks (all 5) | ✅ | ❌ | ⚠️ Partial | **✅ ADD** |
| Incremental export | ✅ | ❌ | ❌ Missing | **✅ ADD** |
| Markdown integration | ✅ Via TOML | ❌ No | ⚠️ Vague | **✅ CLARIFY** |
| Event-driven daemon | ✅ | ❌ | ❌ Missing | **❌ SKIP** |
| **Layer 3: Orchestration** |
| Comments | ✅ | ❌ | ❌ | **❌ Exclude** |
| Assignments | ✅ | ❌ | ❌ | **❌ Exclude** |
| Federation | ✅ | ❌ | ❌ | **❌ Exclude** |
| Compaction | ✅ | ❌ | ❌ | **❌ Exclude** |

**Legend:**
- ✅ = Has it
- ❌ = Doesn't have it
- ⚠️ = Mentioned but not detailed

---

## Action Items for TaskStore

### Immediate (Before Implementation Starts)

1. **Update `implementation-guide.md`:**
   - Add complete merge driver section with code
   - Expand git hooks section (all 5 hooks)
   - Add debounced export section
   - Clarify markdown file lifecycle

2. **Update `taskstore-design.md`:**
   - Add Layer 2 (Git Integration) as explicit section
   - Show clear separation between Layer 1, Layer 2, Layer 3
   - Update implementation phases to include git integration

3. **Update `storage-architecture.md`:**
   - Emphasize merge driver is REQUIRED, not optional
   - Show concrete examples of merge conflicts and resolution
   - Document debouncing strategy

### During Phase 1 Implementation

4. **Implement merge driver first:**
   - Critical path item
   - Test with concurrent modifications
   - Verify automatic conflict resolution

5. **Implement git hooks:**
   - Install as part of `Store::init()`
   - Make it work automatically
   - Test pre-commit and post-merge flows

6. **Test markdown file integration:**
   - Create PRD with `.md` file
   - Verify JSONL and `.md` stay in sync
   - Handle missing file cases

### During Phase 2 Implementation

7. **Add debounced export:**
   - Make it optional (default off for simplicity)
   - Test with rapid mutations
   - Verify batching works

---

## Conclusion

**TaskStore is currently missing critical git integration features that Engram mistakenly removed.**

The "Accidental Minimalism" analysis reveals that Engram's agent incorrectly classified git integration (merge driver, hooks, debouncing) as "orchestration" when they're actually **foundational to git-backed storage**.

**Current state:**
- ✅ Layer 1 (Core): Complete
- ❌ Layer 2 (Git Integration): Missing
- ✅ Layer 3 (Orchestration): Correctly excluded

**What we must do:**
1. Add custom merge driver (CRITICAL)
2. Add complete git hooks (IMPORTANT)
3. Add debounced export (PERFORMANCE)
4. Clarify markdown file lifecycle (CLARITY)

**What we must NOT do:**
- Don't add comments/assignments/federation (TaskDaemon's job)
- Don't add daemon monitoring (TaskDaemon is the daemon)
- Don't add semantic compaction (TaskDaemon's job)

**If we don't fix this:** TaskStore will repeat Engram's mistakes, and we'll have a git-backed storage system that's annoying to use (manual conflict resolution, manual sync, poor performance).

**If we do fix this:** TaskStore will be like Beads' core (solid git integration) without the orchestration baggage (which TaskDaemon provides).

---

## References

- [Beads Research](~/.config/pais/tech/researcher/steve-yegge-gas-town-2026-01-13.md)
- [Engram vs Beads Comparison](~/.config/pais/research/tech/engram-vs-beads/2026-01-12-comparison.md)
- [Engram's Accidental Minimalism](~/.config/pais/research/tech/engram-vs-beads/2026-01-12-accidental-minimalism.md)
- [Engram Source Code](~/repos/neuraphage/engram/src/)
- [TaskStore Design Docs](./taskstore-design.md, ./storage-architecture.md, ./implementation-guide.md)
