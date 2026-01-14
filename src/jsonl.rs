// JSONL file operations

use eyre::{Context, Result};
use serde::{Serialize, de::DeserializeOwned};
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use tracing::{info, warn};

/// Append a record to a JSONL file
pub fn append_jsonl<T: Serialize>(path: &Path, record: &T) -> Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .context("Failed to open JSONL file for appending")?;

    let json = serde_json::to_string(record)?;
    writeln!(file, "{}", json)?;
    file.sync_all()?; // Ensure data is flushed to disk

    Ok(())
}

/// Read all records from a JSONL file, returning latest version per ID
///
/// This assumes records have an "id" field and "updated_at" field.
/// For records with duplicate IDs, the one with the highest updated_at wins.
pub fn read_jsonl_latest<T>(path: &Path) -> Result<HashMap<String, T>>
where
    T: DeserializeOwned + HasId + HasUpdatedAt,
{
    if !path.exists() {
        // File doesn't exist yet, return empty map
        return Ok(HashMap::new());
    }

    let file = File::open(path).context("Failed to open JSONL file")?;
    let reader = BufReader::new(file);
    let mut records: HashMap<String, T> = HashMap::new();

    for (line_num, line) in reader.lines().enumerate() {
        let line = match line {
            Ok(l) => l,
            Err(e) => {
                warn!(
                    file = ?path,
                    line = line_num + 1,
                    error = ?e,
                    "Failed to read line, skipping"
                );
                continue;
            }
        };

        if line.trim().is_empty() {
            continue;
        }

        let record: T = match serde_json::from_str(&line) {
            Ok(r) => r,
            Err(e) => {
                warn!(
                    file = ?path,
                    line = line_num + 1,
                    error = ?e,
                    "Failed to parse JSON, skipping"
                );
                continue;
            }
        };

        let id = record.id();
        let updated_at = record.updated_at();

        // Keep the record with the latest updated_at
        if let Some(existing) = records.get(&id) {
            if updated_at > existing.updated_at() {
                records.insert(id, record);
            }
        } else {
            records.insert(id, record);
        }
    }

    info!(
        file = ?path,
        count = records.len(),
        "Loaded latest records from JSONL"
    );

    Ok(records)
}

/// Trait for types that have an ID field
pub trait HasId {
    fn id(&self) -> String;
}

/// Trait for types that have an updated_at timestamp
pub trait HasUpdatedAt {
    fn updated_at(&self) -> i64;
}

// Implement traits for our models
impl HasId for crate::models::Prd {
    fn id(&self) -> String {
        self.id.clone()
    }
}

impl HasUpdatedAt for crate::models::Prd {
    fn updated_at(&self) -> i64 {
        self.updated_at
    }
}

impl HasId for crate::models::TaskSpec {
    fn id(&self) -> String {
        self.id.clone()
    }
}

impl HasUpdatedAt for crate::models::TaskSpec {
    fn updated_at(&self) -> i64 {
        self.updated_at
    }
}

impl HasId for crate::models::Execution {
    fn id(&self) -> String {
        self.id.clone()
    }
}

impl HasUpdatedAt for crate::models::Execution {
    fn updated_at(&self) -> i64 {
        self.updated_at
    }
}

impl HasId for crate::models::Dependency {
    fn id(&self) -> String {
        self.id.clone()
    }
}

impl HasUpdatedAt for crate::models::Dependency {
    fn updated_at(&self) -> i64 {
        self.created_at // Dependencies use created_at as their timestamp
    }
}

impl HasId for crate::models::Workflow {
    fn id(&self) -> String {
        self.id.clone()
    }
}

impl HasUpdatedAt for crate::models::Workflow {
    fn updated_at(&self) -> i64 {
        self.updated_at
    }
}

impl HasId for crate::models::RepoState {
    fn id(&self) -> String {
        self.repo_path.clone() // repo_path is the primary key
    }
}

impl HasUpdatedAt for crate::models::RepoState {
    fn updated_at(&self) -> i64 {
        self.updated_at
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{Prd, PrdStatus, now_ms};
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_append_jsonl() {
        let temp = TempDir::new().unwrap();
        let jsonl_path = temp.path().join("test.jsonl");

        let prd = Prd {
            id: "test-1".to_string(),
            title: "Test".to_string(),
            description: "Test".to_string(),
            created_at: now_ms(),
            updated_at: now_ms(),
            status: PrdStatus::Draft,
            review_passes: 0,
            content: "content".to_string(),
        };

        append_jsonl(&jsonl_path, &prd).unwrap();

        let content = fs::read_to_string(&jsonl_path).unwrap();
        assert!(content.contains("\"id\":\"test-1\""));
        assert!(content.contains("\"title\":\"Test\""));
    }

    #[test]
    fn test_read_jsonl_latest() {
        let temp = TempDir::new().unwrap();
        let jsonl_path = temp.path().join("test.jsonl");

        // Write multiple versions of same record
        let prd1 = Prd {
            id: "test-1".to_string(),
            title: "Version 1".to_string(),
            description: "Test".to_string(),
            created_at: 1000,
            updated_at: 1000,
            status: PrdStatus::Draft,
            review_passes: 0,
            content: "content".to_string(),
        };

        let prd2 = Prd {
            id: "test-1".to_string(),
            title: "Version 2".to_string(),
            description: "Test".to_string(),
            created_at: 1000,
            updated_at: 2000, // Newer
            status: PrdStatus::Active,
            review_passes: 5,
            content: "content".to_string(),
        };

        append_jsonl(&jsonl_path, &prd1).unwrap();
        append_jsonl(&jsonl_path, &prd2).unwrap();

        // Read should return latest version
        let records: HashMap<String, Prd> = read_jsonl_latest(&jsonl_path).unwrap();
        assert_eq!(records.len(), 1);

        let latest = records.get("test-1").unwrap();
        assert_eq!(latest.title, "Version 2");
        assert_eq!(latest.updated_at, 2000);
        assert_eq!(latest.status, PrdStatus::Active);
    }

    #[test]
    fn test_read_jsonl_nonexistent_file() {
        let temp = TempDir::new().unwrap();
        let jsonl_path = temp.path().join("nonexistent.jsonl");

        let records: HashMap<String, Prd> = read_jsonl_latest(&jsonl_path).unwrap();
        assert_eq!(records.len(), 0);
    }

    #[test]
    fn test_read_jsonl_malformed_line() {
        let temp = TempDir::new().unwrap();
        let jsonl_path = temp.path().join("test.jsonl");

        // Write valid record, then malformed, then another valid
        fs::write(
            &jsonl_path,
            r#"{"id":"test-1","title":"Valid","description":"Test","created_at":1000,"updated_at":1000,"status":"draft","review_passes":0,"content":"content"}
{malformed json}
{"id":"test-2","title":"Also Valid","description":"Test","created_at":1000,"updated_at":1000,"status":"draft","review_passes":0,"content":"content"}
"#,
        )
        .unwrap();

        let records: HashMap<String, Prd> = read_jsonl_latest(&jsonl_path).unwrap();
        // Should skip malformed line and load the two valid records
        assert_eq!(records.len(), 2);
        assert!(records.contains_key("test-1"));
        assert!(records.contains_key("test-2"));
    }
}
