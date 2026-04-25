// JSONL file operations

use crate::corruption::{CorruptionEntry, CorruptionError, truncate_raw};
use eyre::{Context, Result};
use fs2::FileExt;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use tracing::{info, warn};

/// Result of a tolerant JSONL read: id-keyed records carrying their LWW-winning
/// line number, plus one corruption entry per malformed line.
pub type TolerantRead = (HashMap<String, (u64, Value)>, Vec<CorruptionEntry>);

/// Append a record to a JSONL file
pub fn append_jsonl<T: Serialize>(path: &Path, record: &T) -> Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .context("Failed to open JSONL file for appending")?;

    // Acquire exclusive lock before writing
    file.lock_exclusive().context("Failed to acquire file lock")?;

    let json = serde_json::to_string(record)?;
    writeln!(file, "{}", json)?;
    file.sync_all()?; // Ensure data is flushed to disk

    // Lock is automatically released when file is dropped
    Ok(())
}

/// Read all records from a JSONL file, returning latest version per ID.
///
/// Malformed lines are silently skipped with a `warn!` log entry, preserving
/// the long-standing boot-resilience behavior callers rely on. To surface
/// malformed lines to the caller instead of swallowing them, use
/// [`read_jsonl_latest_with_corruption`].
pub fn read_jsonl_latest(path: &Path) -> Result<HashMap<String, Value>> {
    let (map, corruption) = read_jsonl_inner(path)?;
    for entry in &corruption {
        warn!(
            file = ?entry.file,
            line = entry.line,
            error = ?entry.error,
            "Failed to parse JSONL line, skipping"
        );
    }
    Ok(map.into_iter().map(|(k, (_, v))| (k, v)).collect())
}

/// Read all records from a JSONL file, returning latest version per ID
/// alongside one [`CorruptionEntry`] per malformed line.
///
/// The returned map preserves the line number of each record's last-write-wins
/// winning line. Callers that need to attribute later failures (e.g. typed
/// deserialization mismatches) back to a specific line use this variant.
pub fn read_jsonl_latest_with_corruption(path: &Path) -> Result<TolerantRead> {
    read_jsonl_inner(path)
}

/// Shared parse-and-dedup loop used by both `read_jsonl_latest` and
/// `read_jsonl_latest_with_corruption`.
///
/// Returns a `(map, corruption)` pair where:
/// - `map` is keyed by record id and valued by `(line_no, Value)`. The line
///   number is the 1-indexed line of the LWW-winning entry.
/// - `corruption` is one entry per malformed line, in file order, never
///   deduplicated.
fn read_jsonl_inner(path: &Path) -> Result<TolerantRead> {
    if !path.exists() {
        return Ok((HashMap::new(), Vec::new()));
    }

    let file = File::open(path).context("Failed to open JSONL file")?;
    file.lock_shared().context("Failed to acquire shared file lock")?;

    let mut reader = BufReader::new(file);
    let mut map: HashMap<String, (u64, Value)> = HashMap::new();
    let mut corruption: Vec<CorruptionEntry> = Vec::new();
    let mut line_no: u64 = 0;
    let mut buf: Vec<u8> = Vec::new();

    loop {
        buf.clear();
        let bytes = reader
            .read_until(b'\n', &mut buf)
            .context("Failed to read JSONL line")?;
        if bytes == 0 {
            break;
        }
        line_no += 1;

        // Strip trailing newline (LF or CRLF) for parsing and for `raw`.
        let mut end = buf.len();
        if end > 0 && buf[end - 1] == b'\n' {
            end -= 1;
            if end > 0 && buf[end - 1] == b'\r' {
                end -= 1;
            }
        }
        let line_bytes = &buf[..end];

        // Lossy UTF-8 conversion: invalid bytes become U+FFFD. JSON parse
        // will fail consistently on the lossy form, surfacing as InvalidJson
        // with a printable `raw`.
        let line_str = String::from_utf8_lossy(line_bytes);

        if line_str.trim().is_empty() {
            continue;
        }

        match serde_json::from_str::<Value>(&line_str) {
            Ok(value) => {
                let id = match value.get("id").and_then(|v| v.as_str()) {
                    Some(id) => id.to_string(),
                    None => {
                        corruption.push(CorruptionEntry {
                            file: path.to_path_buf(),
                            line: line_no,
                            raw: truncate_raw(line_str.into_owned()),
                            error: CorruptionError::MissingId,
                        });
                        continue;
                    }
                };

                let updated_at = value.get("updated_at").and_then(|v| v.as_i64()).unwrap_or(0);

                if let Some((_, existing)) = map.get(&id) {
                    let existing_updated_at = existing.get("updated_at").and_then(|v| v.as_i64()).unwrap_or(0);
                    if updated_at > existing_updated_at {
                        map.insert(id, (line_no, value));
                    }
                } else {
                    map.insert(id, (line_no, value));
                }
            }
            Err(e) => {
                let category = e.classify().into();
                corruption.push(CorruptionEntry {
                    file: path.to_path_buf(),
                    line: line_no,
                    raw: truncate_raw(line_str.into_owned()),
                    error: CorruptionError::InvalidJson {
                        msg: e.to_string(),
                        category,
                    },
                });
            }
        }
    }

    info!(
        file = ?path,
        count = map.len(),
        corruption = corruption.len(),
        "Loaded latest records from JSONL"
    );

    Ok((map, corruption))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn test_append_jsonl() {
        let temp = TempDir::new().unwrap();
        let jsonl_path = temp.path().join("test.jsonl");

        let record = json!({
            "id": "test-1",
            "name": "Test",
            "updated_at": 1000
        });

        append_jsonl(&jsonl_path, &record).unwrap();

        let content = fs::read_to_string(&jsonl_path).unwrap();
        assert!(content.contains("\"id\":\"test-1\""));
        assert!(content.contains("\"name\":\"Test\""));
    }

    #[test]
    fn test_read_jsonl_latest() {
        let temp = TempDir::new().unwrap();
        let jsonl_path = temp.path().join("test.jsonl");

        // Write multiple versions of same record
        let record1 = json!({
            "id": "test-1",
            "name": "Version 1",
            "updated_at": 1000
        });

        let record2 = json!({
            "id": "test-1",
            "name": "Version 2",
            "updated_at": 2000
        });

        append_jsonl(&jsonl_path, &record1).unwrap();
        append_jsonl(&jsonl_path, &record2).unwrap();

        // Read should return latest version
        let records = read_jsonl_latest(&jsonl_path).unwrap();
        assert_eq!(records.len(), 1);

        let latest = records.get("test-1").unwrap();
        assert_eq!(latest.get("name").and_then(|v| v.as_str()), Some("Version 2"));
        assert_eq!(latest.get("updated_at").and_then(|v| v.as_i64()), Some(2000));
    }

    #[test]
    fn test_read_jsonl_nonexistent_file() {
        let temp = TempDir::new().unwrap();
        let jsonl_path = temp.path().join("nonexistent.jsonl");

        let records = read_jsonl_latest(&jsonl_path).unwrap();
        assert_eq!(records.len(), 0);
    }

    #[test]
    fn test_read_jsonl_malformed_line() {
        let temp = TempDir::new().unwrap();
        let jsonl_path = temp.path().join("test.jsonl");

        // Write valid record, then malformed, then another valid
        fs::write(
            &jsonl_path,
            r#"{"id":"test-1","name":"Valid","updated_at":1000}
{malformed json}
{"id":"test-2","name":"Also Valid","updated_at":1000}
"#,
        )
        .unwrap();

        let records = read_jsonl_latest(&jsonl_path).unwrap();
        // Should skip malformed line and load the two valid records
        assert_eq!(records.len(), 2);
        assert!(records.contains_key("test-1"));
        assert!(records.contains_key("test-2"));
    }

    #[test]
    fn test_read_jsonl_with_corruption_surfaces_malformed() {
        let temp = TempDir::new().unwrap();
        let jsonl_path = temp.path().join("test.jsonl");

        fs::write(
            &jsonl_path,
            r#"{"id":"test-1","name":"Valid","updated_at":1000}
{malformed json}
{"id":"test-2","name":"Also Valid","updated_at":1000}
"#,
        )
        .unwrap();

        let (map, corruption) = read_jsonl_latest_with_corruption(&jsonl_path).unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(corruption.len(), 1);
        assert_eq!(corruption[0].line, 2);
        match &corruption[0].error {
            CorruptionError::InvalidJson { .. } => {}
            other => panic!("expected InvalidJson, got {:?}", other),
        }
    }

    #[test]
    fn test_read_jsonl_with_corruption_surfaces_missing_id() {
        let temp = TempDir::new().unwrap();
        let jsonl_path = temp.path().join("test.jsonl");

        fs::write(
            &jsonl_path,
            r#"{"id":"test-1","updated_at":1000}
{"name":"no id here","updated_at":2000}
{"id":"test-2","updated_at":3000}
"#,
        )
        .unwrap();

        let (map, corruption) = read_jsonl_latest_with_corruption(&jsonl_path).unwrap();
        assert_eq!(map.len(), 2);
        assert_eq!(corruption.len(), 1);
        assert_eq!(corruption[0].line, 2);
        assert_eq!(corruption[0].error, CorruptionError::MissingId);
    }

    #[test]
    fn test_read_jsonl_with_corruption_records_lww_line_no() {
        let temp = TempDir::new().unwrap();
        let jsonl_path = temp.path().join("test.jsonl");

        // Two updates for the same id; line 2 has the later updated_at.
        fs::write(
            &jsonl_path,
            r#"{"id":"test-1","name":"older","updated_at":1000}
{"id":"test-1","name":"newer","updated_at":2000}
"#,
        )
        .unwrap();

        let (map, _) = read_jsonl_latest_with_corruption(&jsonl_path).unwrap();
        let (line_no, value) = map.get("test-1").unwrap();
        assert_eq!(*line_no, 2);
        assert_eq!(value.get("name").and_then(|v| v.as_str()), Some("newer"));
    }

    #[test]
    fn test_read_jsonl_with_corruption_empty_file() {
        let temp = TempDir::new().unwrap();
        let jsonl_path = temp.path().join("empty.jsonl");
        fs::write(&jsonl_path, "").unwrap();

        let (map, corruption) = read_jsonl_latest_with_corruption(&jsonl_path).unwrap();
        assert!(map.is_empty());
        assert!(corruption.is_empty());
    }

    #[test]
    fn test_read_jsonl_with_corruption_missing_file() {
        let temp = TempDir::new().unwrap();
        let jsonl_path = temp.path().join("does-not-exist.jsonl");

        let (map, corruption) = read_jsonl_latest_with_corruption(&jsonl_path).unwrap();
        assert!(map.is_empty());
        assert!(corruption.is_empty());
    }
}
