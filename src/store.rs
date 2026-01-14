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
}
