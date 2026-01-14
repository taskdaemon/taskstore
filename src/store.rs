// Core Store implementation

use eyre::{Context, Result, eyre};
use rusqlite::Connection;
use std::fs;
use std::path::{Path, PathBuf};
use tracing::{info, warn};

const CURRENT_VERSION: u32 = 1;

/// Main TaskStore handle
pub struct Store {
    db: Connection,
    base_path: PathBuf,
}

impl Store {
    /// Open or create store at given path
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let base_path = path.as_ref().to_path_buf();

        info!(store_path = ?base_path, "Opening TaskStore");

        // Create .taskstore directory if it doesn't exist
        fs::create_dir_all(&base_path).context("Failed to create .taskstore directory")?;

        // Create .gitignore for SQLite and logs
        let gitignore_path = base_path.join(".gitignore");
        if !gitignore_path.exists() {
            fs::write(&gitignore_path, "taskstore.db\ntaskstore.db-*\ntaskstore.log\n")
                .context("Failed to create .gitignore")?;
        }

        // Open SQLite database
        let db_path = base_path.join("taskstore.db");
        let db = Connection::open(&db_path).context("Failed to open SQLite database")?;

        // Enable WAL mode for better concurrency
        db.execute_batch("PRAGMA journal_mode=WAL;")?;

        let mut store = Self { db, base_path };

        // Check and handle schema version
        store.ensure_schema()?;

        // Check if sync is needed
        if store.is_stale()? {
            info!("Store is stale, syncing from JSONL");
            store.sync()?;
        }

        info!("TaskStore opened successfully");
        Ok(store)
    }

    /// Get the base path of the store
    pub fn base_path(&self) -> &Path {
        &self.base_path
    }

    /// Ensure schema is initialized and up to date
    fn ensure_schema(&mut self) -> Result<()> {
        let version_file = self.base_path.join(".version");

        let current_version = if version_file.exists() {
            fs::read_to_string(&version_file)
                .context("Failed to read .version file")?
                .trim()
                .parse::<u32>()
                .unwrap_or(0)
        } else {
            0
        };

        if current_version == 0 {
            // Fresh install, initialize schema
            info!("Initializing schema version {}", CURRENT_VERSION);
            self.create_schema()?;
            fs::write(&version_file, CURRENT_VERSION.to_string()).context("Failed to write .version file")?;
        } else if current_version < CURRENT_VERSION {
            // Migration needed
            info!("Migrating schema from v{} to v{}", current_version, CURRENT_VERSION);
            self.migrate_schema(current_version, CURRENT_VERSION)?;
            fs::write(&version_file, CURRENT_VERSION.to_string()).context("Failed to update .version file")?;
        } else if current_version > CURRENT_VERSION {
            return Err(eyre!(
                "Database version ({}) is newer than supported version ({}). Please update taskstore.",
                current_version,
                CURRENT_VERSION
            ));
        }

        Ok(())
    }

    /// Create initial schema
    fn create_schema(&self) -> Result<()> {
        self.db.execute_batch(
            r#"
            -- Product Requirements Documents
            CREATE TABLE IF NOT EXISTS prds (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                description TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                status TEXT NOT NULL,
                review_passes INTEGER NOT NULL,
                content TEXT NOT NULL
            );

            -- Task Specifications
            CREATE TABLE IF NOT EXISTS task_specs (
                id TEXT PRIMARY KEY,
                prd_id TEXT NOT NULL,
                phase_name TEXT NOT NULL,
                description TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                status TEXT NOT NULL,
                workflow_name TEXT,
                assigned_to TEXT,
                content TEXT NOT NULL,
                FOREIGN KEY (prd_id) REFERENCES prds(id) ON DELETE CASCADE
            );

            -- Execution State
            CREATE TABLE IF NOT EXISTS executions (
                id TEXT PRIMARY KEY,
                ts_id TEXT NOT NULL,
                worktree_path TEXT NOT NULL,
                branch_name TEXT NOT NULL,
                status TEXT NOT NULL,
                started_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                completed_at INTEGER,
                current_phase TEXT,
                iteration_count INTEGER NOT NULL DEFAULT 0,
                error_message TEXT,
                FOREIGN KEY (ts_id) REFERENCES task_specs(id) ON DELETE CASCADE
            );

            -- Dependencies
            CREATE TABLE IF NOT EXISTS dependencies (
                id TEXT PRIMARY KEY,
                from_exec_id TEXT NOT NULL,
                to_exec_id TEXT NOT NULL,
                dependency_type TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                resolved_at INTEGER,
                payload TEXT,
                FOREIGN KEY (from_exec_id) REFERENCES executions(id) ON DELETE CASCADE,
                FOREIGN KEY (to_exec_id) REFERENCES executions(id) ON DELETE CASCADE
            );

            -- AWL Workflow Definitions
            CREATE TABLE IF NOT EXISTS workflows (
                id TEXT PRIMARY KEY,
                name TEXT NOT NULL UNIQUE,
                version TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL,
                content TEXT NOT NULL
            );

            -- Repository State
            CREATE TABLE IF NOT EXISTS repo_state (
                repo_path TEXT PRIMARY KEY,
                last_synced_commit TEXT NOT NULL,
                updated_at INTEGER NOT NULL
            );

            -- Indexes for common queries
            CREATE INDEX IF NOT EXISTS idx_prds_status ON prds(status);
            CREATE INDEX IF NOT EXISTS idx_task_specs_prd_id ON task_specs(prd_id);
            CREATE INDEX IF NOT EXISTS idx_task_specs_status ON task_specs(status);
            CREATE INDEX IF NOT EXISTS idx_executions_ts_id ON executions(ts_id);
            CREATE INDEX IF NOT EXISTS idx_executions_status ON executions(status);
            CREATE INDEX IF NOT EXISTS idx_dependencies_from ON dependencies(from_exec_id);
            CREATE INDEX IF NOT EXISTS idx_dependencies_to ON dependencies(to_exec_id);
            CREATE INDEX IF NOT EXISTS idx_workflows_name ON workflows(name);
            "#,
        )?;

        info!("Schema created successfully");
        Ok(())
    }

    /// Migrate schema from one version to another
    fn migrate_schema(&self, _from: u32, _to: u32) -> Result<()> {
        // Future migrations will be implemented here
        // For now, no migrations needed (version 1 is initial)
        warn!("Schema migration requested but no migrations defined yet");
        Ok(())
    }

    /// Check if SQLite is stale compared to JSONL files
    fn is_stale(&self) -> Result<bool> {
        let db_path = self.base_path.join("taskstore.db");

        // If database doesn't exist, it's stale
        if !db_path.exists() {
            return Ok(true);
        }

        let db_mtime = fs::metadata(&db_path)?.modified()?;

        // Check all JSONL files
        let jsonl_files = vec![
            "prds.jsonl",
            "task_specs.jsonl",
            "executions.jsonl",
            "dependencies.jsonl",
            "workflows.jsonl",
            "repo_state.jsonl",
        ];

        for file in jsonl_files {
            let jsonl_path = self.base_path.join(file);
            if jsonl_path.exists() {
                let jsonl_mtime = fs::metadata(&jsonl_path)?.modified()?;
                if jsonl_mtime > db_mtime {
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    /// Sync: Rebuild SQLite from JSONL if needed
    pub fn sync(&mut self) -> Result<()> {
        info!("Syncing store from JSONL files");
        // Implementation will be in Phase 3
        // For now, just a placeholder
        Ok(())
    }

    /// Flush: Ensure all writes are persisted
    pub fn flush(&mut self) -> Result<()> {
        // SQLite auto-commits, JSONL writes are sync
        // This is a no-op for now, but provides API for future optimization
        Ok(())
    }

    // ===== PRD Operations =====

    /// Create a new PRD
    pub fn create_prd(&mut self, prd: crate::models::Prd) -> Result<String> {
        use crate::models::PrdStatus;
        let status_str = match prd.status {
            PrdStatus::Draft => "draft",
            PrdStatus::Ready => "ready",
            PrdStatus::Active => "active",
            PrdStatus::Complete => "complete",
            PrdStatus::Cancelled => "cancelled",
        };

        self.db.execute(
            "INSERT INTO prds (id, title, description, created_at, updated_at, status, review_passes, content)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            (
                &prd.id,
                &prd.title,
                &prd.description,
                prd.created_at,
                prd.updated_at,
                status_str,
                prd.review_passes,
                &prd.content,
            ),
        )?;

        Ok(prd.id.clone())
    }

    /// Get a PRD by ID
    pub fn get_prd(&self, id: &str) -> Result<Option<crate::models::Prd>> {
        use crate::models::{Prd, PrdStatus};
        let mut stmt = self.db.prepare(
            "SELECT id, title, description, created_at, updated_at, status, review_passes, content
             FROM prds WHERE id = ?1",
        )?;

        let prd = stmt.query_row([id], |row| {
            let status_str: String = row.get(5)?;
            let status = match status_str.as_str() {
                "draft" => PrdStatus::Draft,
                "ready" => PrdStatus::Ready,
                "active" => PrdStatus::Active,
                "complete" => PrdStatus::Complete,
                "cancelled" => PrdStatus::Cancelled,
                _ => PrdStatus::Draft,
            };

            Ok(Prd {
                id: row.get(0)?,
                title: row.get(1)?,
                description: row.get(2)?,
                created_at: row.get(3)?,
                updated_at: row.get(4)?,
                status,
                review_passes: row.get(6)?,
                content: row.get(7)?,
            })
        });

        match prd {
            Ok(p) => Ok(Some(p)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Update an existing PRD
    pub fn update_prd(&mut self, id: &str, prd: crate::models::Prd) -> Result<()> {
        use crate::models::PrdStatus;
        let status_str = match prd.status {
            PrdStatus::Draft => "draft",
            PrdStatus::Ready => "ready",
            PrdStatus::Active => "active",
            PrdStatus::Complete => "complete",
            PrdStatus::Cancelled => "cancelled",
        };

        let rows = self.db.execute(
            "UPDATE prds SET title = ?1, description = ?2, updated_at = ?3, status = ?4,
                            review_passes = ?5, content = ?6 WHERE id = ?7",
            (
                &prd.title,
                &prd.description,
                prd.updated_at,
                status_str,
                prd.review_passes,
                &prd.content,
                id,
            ),
        )?;

        if rows == 0 {
            return Err(eyre!("PRD not found: {}", id));
        }

        Ok(())
    }

    /// List PRDs, optionally filtered by status
    pub fn list_prds(&self, status: Option<crate::models::PrdStatus>) -> Result<Vec<crate::models::Prd>> {
        use crate::models::{Prd, PrdStatus};

        let query = if let Some(status_filter) = status {
            let status_str = match status_filter {
                PrdStatus::Draft => "draft",
                PrdStatus::Ready => "ready",
                PrdStatus::Active => "active",
                PrdStatus::Complete => "complete",
                PrdStatus::Cancelled => "cancelled",
            };
            format!(
                "SELECT id, title, description, created_at, updated_at, status, review_passes, content
                 FROM prds WHERE status = '{}' ORDER BY created_at DESC",
                status_str
            )
        } else {
            "SELECT id, title, description, created_at, updated_at, status, review_passes, content
             FROM prds ORDER BY created_at DESC"
                .to_string()
        };

        let mut stmt = self.db.prepare(&query)?;
        let prds = stmt
            .query_map([], |row| {
                let status_str: String = row.get(5)?;
                let status = match status_str.as_str() {
                    "draft" => PrdStatus::Draft,
                    "ready" => PrdStatus::Ready,
                    "active" => PrdStatus::Active,
                    "complete" => PrdStatus::Complete,
                    "cancelled" => PrdStatus::Cancelled,
                    _ => PrdStatus::Draft,
                };

                Ok(Prd {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    description: row.get(2)?,
                    created_at: row.get(3)?,
                    updated_at: row.get(4)?,
                    status,
                    review_passes: row.get(6)?,
                    content: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(prds)
    }

    // ===== TaskSpec Operations =====

    /// Create a new TaskSpec
    pub fn create_task_spec(&mut self, ts: crate::models::TaskSpec) -> Result<String> {
        use crate::models::TaskSpecStatus;
        let status_str = match ts.status {
            TaskSpecStatus::Pending => "pending",
            TaskSpecStatus::Running => "running",
            TaskSpecStatus::Complete => "complete",
            TaskSpecStatus::Failed => "failed",
        };

        self.db.execute(
            "INSERT INTO task_specs (id, prd_id, phase_name, description, created_at, updated_at,
                                    status, workflow_name, assigned_to, content)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            (
                &ts.id,
                &ts.prd_id,
                &ts.phase_name,
                &ts.description,
                ts.created_at,
                ts.updated_at,
                status_str,
                &ts.workflow_name,
                &ts.assigned_to,
                &ts.content,
            ),
        )?;

        Ok(ts.id.clone())
    }

    /// Get a TaskSpec by ID
    pub fn get_task_spec(&self, id: &str) -> Result<Option<crate::models::TaskSpec>> {
        use crate::models::{TaskSpec, TaskSpecStatus};
        let mut stmt = self.db.prepare(
            "SELECT id, prd_id, phase_name, description, created_at, updated_at, status,
                    workflow_name, assigned_to, content
             FROM task_specs WHERE id = ?1",
        )?;

        let ts = stmt.query_row([id], |row| {
            let status_str: String = row.get(6)?;
            let status = match status_str.as_str() {
                "pending" => TaskSpecStatus::Pending,
                "running" => TaskSpecStatus::Running,
                "complete" => TaskSpecStatus::Complete,
                "failed" => TaskSpecStatus::Failed,
                _ => TaskSpecStatus::Pending,
            };

            Ok(TaskSpec {
                id: row.get(0)?,
                prd_id: row.get(1)?,
                phase_name: row.get(2)?,
                description: row.get(3)?,
                created_at: row.get(4)?,
                updated_at: row.get(5)?,
                status,
                workflow_name: row.get(7)?,
                assigned_to: row.get(8)?,
                content: row.get(9)?,
            })
        });

        match ts {
            Ok(t) => Ok(Some(t)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Update an existing TaskSpec
    pub fn update_task_spec(&mut self, id: &str, ts: crate::models::TaskSpec) -> Result<()> {
        use crate::models::TaskSpecStatus;
        let status_str = match ts.status {
            TaskSpecStatus::Pending => "pending",
            TaskSpecStatus::Running => "running",
            TaskSpecStatus::Complete => "complete",
            TaskSpecStatus::Failed => "failed",
        };

        let rows = self.db.execute(
            "UPDATE task_specs SET prd_id = ?1, phase_name = ?2, description = ?3, updated_at = ?4,
                                  status = ?5, workflow_name = ?6, assigned_to = ?7, content = ?8
             WHERE id = ?9",
            (
                &ts.prd_id,
                &ts.phase_name,
                &ts.description,
                ts.updated_at,
                status_str,
                &ts.workflow_name,
                &ts.assigned_to,
                &ts.content,
                id,
            ),
        )?;

        if rows == 0 {
            return Err(eyre!("TaskSpec not found: {}", id));
        }

        Ok(())
    }

    /// List all TaskSpecs for a PRD
    pub fn list_task_specs(&self, prd_id: &str) -> Result<Vec<crate::models::TaskSpec>> {
        use crate::models::{TaskSpec, TaskSpecStatus};

        let mut stmt = self.db.prepare(
            "SELECT id, prd_id, phase_name, description, created_at, updated_at, status,
                    workflow_name, assigned_to, content
             FROM task_specs WHERE prd_id = ?1 ORDER BY created_at ASC",
        )?;

        let specs = stmt
            .query_map([prd_id], |row| {
                let status_str: String = row.get(6)?;
                let status = match status_str.as_str() {
                    "pending" => TaskSpecStatus::Pending,
                    "running" => TaskSpecStatus::Running,
                    "complete" => TaskSpecStatus::Complete,
                    "failed" => TaskSpecStatus::Failed,
                    _ => TaskSpecStatus::Pending,
                };

                Ok(TaskSpec {
                    id: row.get(0)?,
                    prd_id: row.get(1)?,
                    phase_name: row.get(2)?,
                    description: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                    status,
                    workflow_name: row.get(7)?,
                    assigned_to: row.get(8)?,
                    content: row.get(9)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(specs)
    }

    /// List all pending TaskSpecs
    pub fn list_pending_task_specs(&self) -> Result<Vec<crate::models::TaskSpec>> {
        use crate::models::{TaskSpec, TaskSpecStatus};

        let mut stmt = self.db.prepare(
            "SELECT id, prd_id, phase_name, description, created_at, updated_at, status,
                    workflow_name, assigned_to, content
             FROM task_specs WHERE status = 'pending' ORDER BY created_at ASC",
        )?;

        let specs = stmt
            .query_map([], |row| {
                Ok(TaskSpec {
                    id: row.get(0)?,
                    prd_id: row.get(1)?,
                    phase_name: row.get(2)?,
                    description: row.get(3)?,
                    created_at: row.get(4)?,
                    updated_at: row.get(5)?,
                    status: TaskSpecStatus::Pending,
                    workflow_name: row.get(7)?,
                    assigned_to: row.get(8)?,
                    content: row.get(9)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(specs)
    }

    // ===== Execution Operations =====

    /// Create a new Execution
    pub fn create_execution(&mut self, exec: crate::models::Execution) -> Result<String> {
        use crate::models::ExecStatus;
        let status_str = match exec.status {
            ExecStatus::Running => "running",
            ExecStatus::Paused => "paused",
            ExecStatus::Complete => "complete",
            ExecStatus::Failed => "failed",
            ExecStatus::Stopped => "stopped",
        };

        self.db.execute(
            "INSERT INTO executions (id, ts_id, worktree_path, branch_name, status, started_at,
                                    updated_at, completed_at, current_phase, iteration_count, error_message)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
            (
                &exec.id,
                &exec.ts_id,
                &exec.worktree_path,
                &exec.branch_name,
                status_str,
                exec.started_at,
                exec.updated_at,
                exec.completed_at,
                &exec.current_phase,
                exec.iteration_count,
                &exec.error_message,
            ),
        )?;

        Ok(exec.id.clone())
    }

    /// Get an Execution by ID
    pub fn get_execution(&self, id: &str) -> Result<Option<crate::models::Execution>> {
        use crate::models::{ExecStatus, Execution};
        let mut stmt = self.db.prepare(
            "SELECT id, ts_id, worktree_path, branch_name, status, started_at, updated_at,
                    completed_at, current_phase, iteration_count, error_message
             FROM executions WHERE id = ?1",
        )?;

        let exec = stmt.query_row([id], |row| {
            let status_str: String = row.get(4)?;
            let status = match status_str.as_str() {
                "running" => ExecStatus::Running,
                "paused" => ExecStatus::Paused,
                "complete" => ExecStatus::Complete,
                "failed" => ExecStatus::Failed,
                "stopped" => ExecStatus::Stopped,
                _ => ExecStatus::Running,
            };

            Ok(Execution {
                id: row.get(0)?,
                ts_id: row.get(1)?,
                worktree_path: row.get(2)?,
                branch_name: row.get(3)?,
                status,
                started_at: row.get(5)?,
                updated_at: row.get(6)?,
                completed_at: row.get(7)?,
                current_phase: row.get(8)?,
                iteration_count: row.get(9)?,
                error_message: row.get(10)?,
            })
        });

        match exec {
            Ok(e) => Ok(Some(e)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Update an existing Execution
    pub fn update_execution(&mut self, id: &str, exec: crate::models::Execution) -> Result<()> {
        use crate::models::ExecStatus;
        let status_str = match exec.status {
            ExecStatus::Running => "running",
            ExecStatus::Paused => "paused",
            ExecStatus::Complete => "complete",
            ExecStatus::Failed => "failed",
            ExecStatus::Stopped => "stopped",
        };

        let rows = self.db.execute(
            "UPDATE executions SET ts_id = ?1, worktree_path = ?2, branch_name = ?3, status = ?4,
                                  updated_at = ?5, completed_at = ?6, current_phase = ?7,
                                  iteration_count = ?8, error_message = ?9
             WHERE id = ?10",
            (
                &exec.ts_id,
                &exec.worktree_path,
                &exec.branch_name,
                status_str,
                exec.updated_at,
                exec.completed_at,
                &exec.current_phase,
                exec.iteration_count,
                &exec.error_message,
                id,
            ),
        )?;

        if rows == 0 {
            return Err(eyre!("Execution not found: {}", id));
        }

        Ok(())
    }

    /// List executions, optionally filtered by status
    pub fn list_executions(&self, status: Option<crate::models::ExecStatus>) -> Result<Vec<crate::models::Execution>> {
        use crate::models::{ExecStatus, Execution};

        let query = if let Some(status_filter) = status {
            let status_str = match status_filter {
                ExecStatus::Running => "running",
                ExecStatus::Paused => "paused",
                ExecStatus::Complete => "complete",
                ExecStatus::Failed => "failed",
                ExecStatus::Stopped => "stopped",
            };
            format!(
                "SELECT id, ts_id, worktree_path, branch_name, status, started_at, updated_at,
                        completed_at, current_phase, iteration_count, error_message
                 FROM executions WHERE status = '{}' ORDER BY started_at DESC",
                status_str
            )
        } else {
            "SELECT id, ts_id, worktree_path, branch_name, status, started_at, updated_at,
                    completed_at, current_phase, iteration_count, error_message
             FROM executions ORDER BY started_at DESC"
                .to_string()
        };

        let mut stmt = self.db.prepare(&query)?;
        let execs = stmt
            .query_map([], |row| {
                let status_str: String = row.get(4)?;
                let status = match status_str.as_str() {
                    "running" => ExecStatus::Running,
                    "paused" => ExecStatus::Paused,
                    "complete" => ExecStatus::Complete,
                    "failed" => ExecStatus::Failed,
                    "stopped" => ExecStatus::Stopped,
                    _ => ExecStatus::Running,
                };

                Ok(Execution {
                    id: row.get(0)?,
                    ts_id: row.get(1)?,
                    worktree_path: row.get(2)?,
                    branch_name: row.get(3)?,
                    status,
                    started_at: row.get(5)?,
                    updated_at: row.get(6)?,
                    completed_at: row.get(7)?,
                    current_phase: row.get(8)?,
                    iteration_count: row.get(9)?,
                    error_message: row.get(10)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(execs)
    }

    /// List all active (running or paused) executions
    pub fn list_active_executions(&self) -> Result<Vec<crate::models::Execution>> {
        use crate::models::{ExecStatus, Execution};

        let mut stmt = self.db.prepare(
            "SELECT id, ts_id, worktree_path, branch_name, status, started_at, updated_at,
                    completed_at, current_phase, iteration_count, error_message
             FROM executions WHERE status IN ('running', 'paused') ORDER BY started_at DESC",
        )?;

        let execs = stmt
            .query_map([], |row| {
                let status_str: String = row.get(4)?;
                let status = match status_str.as_str() {
                    "running" => ExecStatus::Running,
                    "paused" => ExecStatus::Paused,
                    _ => ExecStatus::Running,
                };

                Ok(Execution {
                    id: row.get(0)?,
                    ts_id: row.get(1)?,
                    worktree_path: row.get(2)?,
                    branch_name: row.get(3)?,
                    status,
                    started_at: row.get(5)?,
                    updated_at: row.get(6)?,
                    completed_at: row.get(7)?,
                    current_phase: row.get(8)?,
                    iteration_count: row.get(9)?,
                    error_message: row.get(10)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;

        Ok(execs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_store_open_creates_directory() {
        let temp = TempDir::new().unwrap();
        let store_path = temp.path().join(".taskstore");

        let _store = Store::open(&store_path).unwrap();
        assert!(store_path.exists());
        assert!(store_path.join("taskstore.db").exists());
        assert!(store_path.join(".gitignore").exists());
        assert!(store_path.join(".version").exists());
    }

    #[test]
    fn test_gitignore_contents() {
        let temp = TempDir::new().unwrap();
        let store_path = temp.path().join(".taskstore");

        Store::open(&store_path).unwrap();

        let gitignore = fs::read_to_string(store_path.join(".gitignore")).unwrap();
        assert!(gitignore.contains("taskstore.db"));
        assert!(gitignore.contains("taskstore.log"));
    }

    #[test]
    fn test_version_file_created() {
        let temp = TempDir::new().unwrap();
        let store_path = temp.path().join(".taskstore");

        Store::open(&store_path).unwrap();

        let version = fs::read_to_string(store_path.join(".version")).unwrap();
        assert_eq!(version.trim(), CURRENT_VERSION.to_string());
    }

    #[test]
    fn test_store_reopen() {
        let temp = TempDir::new().unwrap();
        let store_path = temp.path().join(".taskstore");

        // Open first time
        {
            let _store = Store::open(&store_path).unwrap();
        }

        // Reopen should work
        let store = Store::open(&store_path).unwrap();
        assert_eq!(store.base_path(), store_path);
    }

    #[test]
    fn test_is_stale_fresh_db() {
        let temp = TempDir::new().unwrap();
        let store_path = temp.path().join(".taskstore");

        let _store = Store::open(&store_path).unwrap();
        // Fresh database with no JSONL files should not be stale
        assert!(!_store.is_stale().unwrap());
    }

    #[test]
    fn test_prd_crud() {
        use crate::models::{Prd, PrdStatus, now_ms};
        let temp = TempDir::new().unwrap();
        let store_path = temp.path().join(".taskstore");
        let mut store = Store::open(&store_path).unwrap();

        // Create
        let prd = Prd {
            id: "test-prd-1".to_string(),
            title: "Test PRD".to_string(),
            description: "Test description".to_string(),
            created_at: now_ms(),
            updated_at: now_ms(),
            status: PrdStatus::Draft,
            review_passes: 5,
            content: "# Test Content".to_string(),
        };

        let id = store.create_prd(prd.clone()).unwrap();
        assert_eq!(id, "test-prd-1");

        // Read
        let retrieved = store.get_prd(&id).unwrap();
        assert!(retrieved.is_some());
        let retrieved = retrieved.unwrap();
        assert_eq!(retrieved.title, "Test PRD");
        assert_eq!(retrieved.status, PrdStatus::Draft);

        // Update
        let mut updated_prd = retrieved.clone();
        updated_prd.status = PrdStatus::Active;
        updated_prd.updated_at = now_ms();
        store.update_prd(&id, updated_prd).unwrap();

        let retrieved = store.get_prd(&id).unwrap().unwrap();
        assert_eq!(retrieved.status, PrdStatus::Active);

        // List
        let prds = store.list_prds(None).unwrap();
        assert_eq!(prds.len(), 1);

        let draft_prds = store.list_prds(Some(PrdStatus::Draft)).unwrap();
        assert_eq!(draft_prds.len(), 0);

        let active_prds = store.list_prds(Some(PrdStatus::Active)).unwrap();
        assert_eq!(active_prds.len(), 1);
    }

    #[test]
    fn test_task_spec_crud() {
        use crate::models::{Prd, PrdStatus, TaskSpec, TaskSpecStatus, now_ms};
        let temp = TempDir::new().unwrap();
        let store_path = temp.path().join(".taskstore");
        let mut store = Store::open(&store_path).unwrap();

        // Create PRD first
        let prd = Prd {
            id: "prd-1".to_string(),
            title: "Test PRD".to_string(),
            description: "Test".to_string(),
            created_at: now_ms(),
            updated_at: now_ms(),
            status: PrdStatus::Active,
            review_passes: 5,
            content: "content".to_string(),
        };
        store.create_prd(prd).unwrap();

        // Create TaskSpec
        let ts = TaskSpec {
            id: "ts-1".to_string(),
            prd_id: "prd-1".to_string(),
            phase_name: "Phase 1".to_string(),
            description: "Test task".to_string(),
            created_at: now_ms(),
            updated_at: now_ms(),
            status: TaskSpecStatus::Pending,
            workflow_name: Some("rust-development".to_string()),
            assigned_to: None,
            content: "# Task Content".to_string(),
        };

        let id = store.create_task_spec(ts.clone()).unwrap();
        assert_eq!(id, "ts-1");

        // Read
        let retrieved = store.get_task_spec(&id).unwrap().unwrap();
        assert_eq!(retrieved.phase_name, "Phase 1");
        assert_eq!(retrieved.status, TaskSpecStatus::Pending);

        // Update
        let mut updated_ts = retrieved.clone();
        updated_ts.status = TaskSpecStatus::Running;
        updated_ts.assigned_to = Some("exec-1".to_string());
        store.update_task_spec(&id, updated_ts).unwrap();

        let retrieved = store.get_task_spec(&id).unwrap().unwrap();
        assert_eq!(retrieved.status, TaskSpecStatus::Running);
        assert_eq!(retrieved.assigned_to, Some("exec-1".to_string()));

        // List by PRD
        let specs = store.list_task_specs("prd-1").unwrap();
        assert_eq!(specs.len(), 1);

        // List pending
        let pending = store.list_pending_task_specs().unwrap();
        assert_eq!(pending.len(), 0); // We updated it to running
    }

    #[test]
    fn test_execution_crud() {
        use crate::models::{ExecStatus, Execution, Prd, PrdStatus, TaskSpec, TaskSpecStatus, now_ms};
        let temp = TempDir::new().unwrap();
        let store_path = temp.path().join(".taskstore");
        let mut store = Store::open(&store_path).unwrap();

        // Create PRD and TaskSpec first
        let prd = Prd {
            id: "prd-1".to_string(),
            title: "Test PRD".to_string(),
            description: "Test".to_string(),
            created_at: now_ms(),
            updated_at: now_ms(),
            status: PrdStatus::Active,
            review_passes: 5,
            content: "content".to_string(),
        };
        store.create_prd(prd).unwrap();

        let ts = TaskSpec {
            id: "ts-1".to_string(),
            prd_id: "prd-1".to_string(),
            phase_name: "Phase 1".to_string(),
            description: "Test task".to_string(),
            created_at: now_ms(),
            updated_at: now_ms(),
            status: TaskSpecStatus::Pending,
            workflow_name: None,
            assigned_to: None,
            content: "content".to_string(),
        };
        store.create_task_spec(ts).unwrap();

        // Create Execution
        let exec = Execution {
            id: "exec-1".to_string(),
            ts_id: "ts-1".to_string(),
            worktree_path: "/tmp/worktree".to_string(),
            branch_name: "feature/test".to_string(),
            status: ExecStatus::Running,
            started_at: now_ms(),
            updated_at: now_ms(),
            completed_at: None,
            current_phase: Some("Phase 1".to_string()),
            iteration_count: 0,
            error_message: None,
        };

        let id = store.create_execution(exec.clone()).unwrap();
        assert_eq!(id, "exec-1");

        // Read
        let retrieved = store.get_execution(&id).unwrap().unwrap();
        assert_eq!(retrieved.status, ExecStatus::Running);
        assert_eq!(retrieved.iteration_count, 0);

        // Update
        let mut updated_exec = retrieved.clone();
        updated_exec.iteration_count = 5;
        updated_exec.status = ExecStatus::Complete;
        updated_exec.completed_at = Some(now_ms());
        store.update_execution(&id, updated_exec).unwrap();

        let retrieved = store.get_execution(&id).unwrap().unwrap();
        assert_eq!(retrieved.status, ExecStatus::Complete);
        assert_eq!(retrieved.iteration_count, 5);
        assert!(retrieved.completed_at.is_some());

        // List all
        let execs = store.list_executions(None).unwrap();
        assert_eq!(execs.len(), 1);

        // List by status
        let running = store.list_executions(Some(ExecStatus::Running)).unwrap();
        assert_eq!(running.len(), 0);

        let complete = store.list_executions(Some(ExecStatus::Complete)).unwrap();
        assert_eq!(complete.len(), 1);

        // List active (should be empty since we completed it)
        let active = store.list_active_executions().unwrap();
        assert_eq!(active.len(), 0);
    }

    #[test]
    fn test_update_nonexistent_returns_error() {
        use crate::models::{Prd, PrdStatus, now_ms};
        let temp = TempDir::new().unwrap();
        let store_path = temp.path().join(".taskstore");
        let mut store = Store::open(&store_path).unwrap();

        let prd = Prd {
            id: "nonexistent".to_string(),
            title: "Test".to_string(),
            description: "Test".to_string(),
            created_at: now_ms(),
            updated_at: now_ms(),
            status: PrdStatus::Draft,
            review_passes: 0,
            content: "content".to_string(),
        };

        let result = store.update_prd("nonexistent", prd);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("PRD not found"));
    }
}
