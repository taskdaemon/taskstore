# TaskDaemon Storage Architecture: The Bead Store Pattern

**Date:** 2026-01-13
**Author:** Claude Sonnet 4.5
**Status:** Authoritative

## Summary

TaskDaemon uses the **Bead Store pattern** from Steve Yegge's Gas Town: work items stored as one-line JSON in Git-backed JSONL files, with SQLite as a rebuildable query cache. This provides durability, version control, collaboration, and crash recovery while maintaining fast query performance.

## The Bead Store Pattern (from Gas Town)

Steve Yegge's Gas Town stores work items as "Beads":
- **Beads** = Atomic work units with persistent identity (like GitHub issues)
- **Storage** = One JSON record per line in `.jsonl` files, committed to Git
- **Query** = SQLite index rebuilt from JSONL for fast lookups
- **Updates** = Append-only (multiple lines for same ID, latest wins)

**Key insight:** Beads are **work items**, not just data. They track:
1. What needs to be done
2. Who's doing it
3. Current status
4. Dependencies

## Three Storage Layers

```
┌─────────────────────────────────────────────────────────────┐
│                    LAYER 1: JSONL Files                      │
│                  (Source of Truth, in Git)                   │
│                                                              │
│  .taskstore/                                                 │
│  ├── prds.jsonl           ← Requirements (Epics)            │
│  ├── task-specs.jsonl     ← Work units (Beads)              │
│  ├── executions.jsonl     ← Worker assignments              │
│  ├── workflows.jsonl      ← Templates (Formulas)            │
│  └── dependencies.jsonl   ← Coordination                     │
│                                                              │
│  Properties:                                                 │
│  - Append-only (never delete lines)                         │
│  - One JSON object per line                                 │
│  - Multiple lines for same ID allowed                       │
│  - Latest record wins (highest updated_at)                  │
│  - Committed to Git (version controlled)                    │
│  - Human-readable (can grep, inspect)                       │
└─────────────────────────────────────────────────────────────┘
                            │
                            │ sync() rebuilds
                            ▼
┌─────────────────────────────────────────────────────────────┐
│                  LAYER 2: SQLite Database                    │
│                  (Query Cache, ephemeral)                    │
│                                                              │
│  .taskstore/taskstore.db (in .gitignore)                    │
│                                                              │
│  Tables:                                                     │
│  - prds                                                      │
│  - task_specs                                                │
│  - executions                                                │
│  - workflows                                                 │
│  - dependencies                                              │
│                                                              │
│  Properties:                                                 │
│  - Fast indexed queries                                      │
│  - Built from JSONL on startup                              │
│  - Can be deleted and rebuilt anytime                       │
│  - NOT committed to Git                                     │
└─────────────────────────────────────────────────────────────┘
                            │
                            │ queries
                            ▼
┌─────────────────────────────────────────────────────────────┐
│                   LAYER 3: Memory (Runtime)                  │
│                                                              │
│  - Coordinator registry (exec_id → channel)                 │
│  - Pending queries (query_id → oneshot sender)              │
│  - Subscription lists (event_type → [exec_ids])             │
│  - Rate limiter counters                                     │
│  - Loop context variables                                    │
│                                                              │
│  Properties:                                                 │
│  - Ephemeral (lost on restart)                              │
│  - Rebuilt on startup (loops re-register)                   │
│  - No persistence needed                                     │
└─────────────────────────────────────────────────────────────┘
```

## What Goes Where

### In JSONL (and Git) - Work Items

These are **work items with persistent identity** - the core of the Bead Store:

#### 1. prds.jsonl - Requirements (Epics)

**Purpose:** High-level work requirements, like epics in Jira

**Example:**
```jsonl
{"id":"prd-550e8400","title":"Add User Authentication","status":"active","created_at":1704067200000,"updated_at":1704067200000,"review_passes":5,"content":"# PRD: Add User Authentication\n\n## Summary\nImplement JWT-based authentication...\n\n## Goals\n- Secure login/logout\n- Session management\n\n## Non-Goals\n- OAuth integration\n- LDAP support"}
```

**Why in Git:**
- Requirements evolve (need version history)
- Teams review/approve PRDs (collaboration)
- Compare PRDs across branches
- See how requirements changed over time

**SQLite usage:** `SELECT * FROM prds WHERE status='active'`

#### 2. task-specs.jsonl - Work Units (Beads)

**Purpose:** Atomic work items, one per phase/task

**Example:**
```jsonl
{"id":"ts-660e8400","prd_id":"prd-550e8400","phase_name":"Phase 1: Core Logic","status":"pending","workflow_name":"rust-development","dependencies":[],"content":"# Task: Implement Auth Core\n\nImplement the core authentication logic:\n- JWT generation\n- Token validation\n- Session storage"}
{"id":"ts-770e8400","prd_id":"prd-550e8400","phase_name":"Phase 2: Tests","status":"pending","workflow_name":"rust-development","dependencies":["ts-660e8400"],"content":"# Task: Write Auth Tests\n\nTest coverage for:\n- Login flow\n- Token expiry\n- Invalid credentials"}
```

**Why in Git:**
- Work breakdown changes (refine task definitions)
- Dependencies shift as work progresses
- Multiple devs need to see the work queue
- Track what's assigned to whom

**SQLite usage:**
```sql
SELECT * FROM task_specs
WHERE status='pending'
  AND NOT EXISTS (
    SELECT 1 FROM dependencies
    WHERE to_exec_id = task_specs.assigned_to
  )
```

#### 3. workflows.jsonl - Templates (Formulas)

**Purpose:** Reusable AWL workflow definitions

**Example:**
```jsonl
{"id":"wf-880e8400","name":"rust-development","version":"1.0","created_at":1704067200000,"updated_at":1704067200000,"content":"workflow:\n  name: rust-development\n  version: \"1.0\"\n  before:\n    - action: shell\n      command: \"cargo init --bin\"\n  foreach:\n    items: \"{ts.phases}\"\n    steps:\n      - action: prompt-agent\n        model: \"claude-opus-4.5\"\n        prompt: \"Implement {foreach.item}...\"\n      - action: shell\n        command: \"cargo check\"\n      - action: shell\n        command: \"cargo test\""}
```

**Why in Git:**
- Workflows are code (need version control)
- Teams iterate on workflow templates
- Different projects customize workflows
- Compare workflow versions

**SQLite usage:** `SELECT content FROM workflows WHERE name='rust-development'`

#### 4. executions.jsonl - Worker Assignments

**Purpose:** Track which loop (worker) is executing which task

**Example:**
```jsonl
{"id":"exec-990e8400","ts_id":"ts-660e8400","worktree_path":"/tmp/taskdaemon/worktrees/exec-990e8400","branch_name":"feature/auth-phase1","status":"running","started_at":1704074400000,"updated_at":1704078000000,"completed_at":null,"current_phase":"Phase 1","iteration_count":7,"error_message":null}
{"id":"exec-990e8400","ts_id":"ts-660e8400","worktree_path":"/tmp/taskdaemon/worktrees/exec-990e8400","branch_name":"feature/auth-phase1","status":"running","started_at":1704074400000,"updated_at":1704079000000,"completed_at":null,"current_phase":"Phase 1","iteration_count":8,"error_message":null}
```

**Note:** Multiple lines for same ID (exec-990e8400) - this is normal and expected!

**Why in Git:**
- **Crash recovery:** Know what was running, can resume
- **Status visibility:** Team sees in-progress work
- **Audit trail:** Who did what, when
- **Resume capability:** Restart from last iteration

**SQLite usage:** `SELECT * FROM executions WHERE status='running'`

#### 5. dependencies.jsonl - Coordination Messages

**Purpose:** Record inter-task communication (notify/query/share)

**Example:**
```jsonl
{"id":"dep-aa0e8400","from_exec_id":"exec-990e8400","to_exec_id":null,"dependency_type":"notify","created_at":1704078000000,"resolved_at":1704078000000,"payload":"{\"event_type\":\"phase_complete\",\"phase\":\"Phase 1\"}"}
{"id":"dep-bb0e8400","from_exec_id":"exec-990e8400","to_exec_id":"exec-cc0e8400","dependency_type":"query","created_at":1704078100000,"resolved_at":1704078105000,"payload":"{\"query_id\":\"q-123\",\"question\":\"What is the API URL?\",\"answer\":\"http://localhost:8080\"}"}
```

**Why in Git:**
- **Coordination history:** See how tasks interacted
- **Debugging:** Why did task X wait for task Y?
- **Audit:** Track all inter-task communication

**SQLite usage:** `SELECT * FROM dependencies WHERE resolved_at IS NULL`

### In SQLite Only - Query Cache

**Purpose:** Fast indexed lookups

**Contents:** Exact copy of JSONL data, but structured in tables with indexes

**Examples:**
```sql
-- Find all pending tasks ready to run
SELECT * FROM task_specs ts
WHERE ts.status = 'pending'
  AND NOT EXISTS (
    SELECT 1 FROM task_specs dep
    JOIN JSONL_to_list(ts.dependencies) d ON dep.id = d.value
    WHERE dep.status != 'complete'
  );

-- Find all running executions
SELECT * FROM executions WHERE status = 'running';

-- Find execution by worktree path
SELECT * FROM executions WHERE worktree_path = '/tmp/taskdaemon/worktrees/exec-990e8400';
```

**Why not in Git:**
- Derived data (can rebuild from JSONL anytime)
- Binary file (git diffs useless)
- Changes constantly (every write)

**Rebuild strategy:**
```bash
rm .taskstore/taskstore.db
taskstore sync  # Rebuilds from JSONL
```

### In Memory Only - Runtime State

**Not persisted anywhere:**

1. **Coordinator registry:** `HashMap<ExecId, Sender<CoordMessage>>`
   - Changes every time a loop spawns/dies
   - Rebuilt on startup (loops re-register)

2. **Pending queries:** `HashMap<QueryId, Oneshot<Result<String>>>`
   - Resolved within seconds
   - Outstanding queries timeout on restart

3. **Subscription lists:** `HashMap<EventType, Vec<ExecId>>`
   - Runtime registration
   - Loops re-subscribe on startup

4. **Rate limiter counters:** `HashMap<ExecId, VecDeque<Instant>>`
   - Sliding window, resets on restart

5. **Loop context variables:** `HashMap<String, Value>` (per loop)
   - Temporary during iteration
   - Discarded after iteration completes

## Write Operations

### Creating a Work Item

```rust
pub fn create_task_spec(&mut self, ts: TaskSpec) -> Result<String> {
    // 1. Append to JSONL (durability first)
    let json = serde_json::to_string(&ts)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(".taskstore/task-specs.jsonl")?;
    writeln!(file, "{}", json)?;
    file.sync_all()?;  // fsync to disk

    // 2. Insert into SQLite (query performance)
    self.db.execute(
        "INSERT INTO task_specs (id, prd_id, phase_name, status, ...)
         VALUES (?1, ?2, ?3, ?4, ...)",
        params![ts.id, ts.prd_id, ts.phase_name, ts.status.to_string()],
    )?;

    Ok(ts.id)
}
```

### Updating a Work Item (Append-Only)

```rust
pub fn update_execution(&mut self, exec_id: &str, exec: Execution) -> Result<()> {
    // 1. Append to JSONL (yes, duplicate ID - that's the pattern!)
    let json = serde_json::to_string(&exec)?;
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(".taskstore/executions.jsonl")?;
    writeln!(file, "{}", json)?;
    file.sync_all()?;

    // 2. Update SQLite (single record)
    self.db.execute(
        "UPDATE executions
         SET status = ?1, updated_at = ?2, iteration_count = ?3, current_phase = ?4
         WHERE id = ?5",
        params![exec.status.to_string(), exec.updated_at, exec.iteration_count, exec.current_phase, exec.id],
    )?;

    Ok(())
}
```

**Result:** JSONL has multiple lines for same exec_id, each with different iteration_count and updated_at.

## Read Operations

### Fast Queries (SQLite)

```rust
pub fn list_active_executions(&self) -> Result<Vec<Execution>> {
    let mut stmt = self.db.prepare(
        "SELECT * FROM executions WHERE status = 'running' ORDER BY started_at"
    )?;

    let executions = stmt.query_map([], |row| {
        Ok(Execution {
            id: row.get(0)?,
            ts_id: row.get(1)?,
            status: row.get(4)?.parse().unwrap(),
            // ... map all fields
        })
    })?
    .collect::<Result<Vec<_>, _>>()?;

    Ok(executions)
}
```

### Rebuild from JSONL (Sync)

```rust
pub fn sync(&mut self) -> Result<()> {
    // 1. Read all JSONL files
    let executions = self.read_jsonl::<Execution>("executions.jsonl")?;

    // 2. Deduplicate: keep latest record per ID (highest updated_at)
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

fn read_jsonl<T: DeserializeOwned>(&self, filename: &str) -> Result<Vec<T>> {
    let path = PathBuf::from(".taskstore").join(filename);
    let file = File::open(path)?;
    let reader = BufReader::new(file);

    let mut records = Vec::new();
    for line in reader.lines() {
        let line = line?;
        let record: T = serde_json::from_str(&line)?;
        records.push(record);
    }

    Ok(records)
}
```

## Git Integration

### Committing Work State

```bash
# Developer checkpoint
cd /path/to/repo
git add .taskstore/*.jsonl
git commit -m "Checkpoint: 3 loops running, auth core complete"

# Executions.jsonl might have 50 lines for 3 executions
# (each execution updated ~15-20 times)
# That's fine - it's append-only history
```

### Pulling Updates

```bash
git pull origin main

# post-merge hook automatically runs:
#!/bin/bash
cd "$(git rev-parse --show-toplevel)"
taskstore sync  # Rebuilds SQLite from merged JSONL
```

### Merge Conflicts

Custom merge driver (`.gitattributes`):
```
.taskstore/*.jsonl merge=taskstore-merge
```

**taskstore-merge algorithm:**
1. Parse base, ours, theirs (all JSONL)
2. Combine all records from both sides
3. Deduplicate by ID (keep record with highest updated_at)
4. Write merged JSONL
5. Exit 0 (success)

**Example conflict:**
```
Base:    {"id":"exec-123","iteration_count":5,"updated_at":1000}
Ours:    {"id":"exec-123","iteration_count":7,"updated_at":1001}
Theirs:  {"id":"exec-123","iteration_count":6,"updated_at":1002}

Merged:  {"id":"exec-123","iteration_count":6,"updated_at":1002}  # Theirs wins (newer)
```

## Compaction (Optional)

Over time, JSONL files grow (multiple lines per ID). Compaction removes superseded records:

```bash
taskstore compact

# Or automatically when file > 100MB
```

**Algorithm:**
```rust
pub fn compact(&mut self, filename: &str) -> Result<()> {
    // 1. Read all records
    let records = self.read_jsonl::<Execution>(filename)?;

    // 2. Keep only latest per ID
    let mut latest: HashMap<String, Execution> = HashMap::new();
    for record in records {
        match latest.get(&record.id) {
            Some(existing) if existing.updated_at > record.updated_at => continue,
            _ => { latest.insert(record.id.clone(), record); }
        }
    }

    // 3. Write to temp file
    let temp_path = format!("{}.tmp", filename);
    let mut temp = File::create(&temp_path)?;
    for record in latest.values() {
        let json = serde_json::to_string(record)?;
        writeln!(temp, "{}", json)?;
    }
    temp.sync_all()?;

    // 4. Atomic rename
    fs::rename(temp_path, filename)?;

    Ok(())
}
```

**Result:** JSONL shrinks from 5000 lines to 50 lines (100 executions × 50 updates = 5000, compacted to 100 latest)

## Crash Recovery Example

### Scenario:

```
12:00 PM - TaskDaemon starts
12:01 PM - Spawn 3 execution loops (exec-001, exec-002, exec-003)
          → Written to executions.jsonl
12:05 PM - exec-001 iteration 5 completes
          → Appended to executions.jsonl
12:10 PM - exec-002 iteration 3 completes
          → Appended to executions.jsonl
12:15 PM - exec-001 iteration 6 completes
          → Appended to executions.jsonl
12:16 PM - **CRASH** (power failure)
```

### On Restart:

```
12:20 PM - TaskDaemon restarts
          → Calls store.sync()
          → Reads executions.jsonl (6 lines)
          → Deduplicates to 3 latest records
          → Rebuilds SQLite

12:21 PM - Queries SQLite: "SELECT * FROM executions WHERE status='running'"
          → Finds 3 running executions

12:22 PM - For each running execution:
          → Checks if worktree exists
          → If yes: Resume from last iteration
          → If no: Mark as failed (cleanup issue)

12:23 PM - exec-001 resumes from iteration 6
          - exec-002 resumes from iteration 3
          - exec-003 resumes from iteration 1

Work continues without data loss!
```

## Benefits of the Bead Store Pattern

1. **Durability:** JSONL writes are fsync'd, survive crashes
2. **Version Control:** Full git history of all work items
3. **Collaboration:** Multiple devs see same work queue
4. **Audit Trail:** Every status change recorded
5. **Crash Recovery:** Resume exactly where we left off
6. **Fast Queries:** SQLite indexes for performance
7. **Simple Consistency:** JSONL is truth, SQLite is cache
8. **Human Readable:** Can grep/inspect JSONL files directly
9. **Merge Capability:** Git merges handled automatically
10. **Flexible:** Append-only means never delete history

## Anti-Patterns to Avoid

❌ **Don't put SQLite in Git**
- Binary file, git diffs useless
- Changes constantly, huge repo bloat
- Can rebuild from JSONL anytime

❌ **Don't update JSONL in-place**
- Append only! Multiple lines per ID is the pattern
- Compaction is a separate operation

❌ **Don't query JSONL directly for speed**
- Use SQLite for queries
- JSONL is for durability and git

❌ **Don't persist ephemeral state**
- Coordinator registry, subscriptions → memory only
- Rebuild on restart (loops re-register)

❌ **Don't trust SQLite as source of truth**
- If JSONL and SQLite disagree, JSONL wins
- Always rebuild from JSONL on conflict

## References

- Gas Town Beads: `~/.config/pais/tech/researcher/steve-yegge-gas-town-2026-01-13.md`
- TaskStore Design: `./taskstore-design.md`
- JSONL Format: https://jsonlines.org/
