// Corruption-detection types for tolerant bulk reads.
//
// These types are returned by `Store::list_tolerant<T>` and the underlying
// `jsonl::read_jsonl_latest_with_corruption` reader. They surface line-level
// problems that the existing strict reader silently logs and skips.

use std::path::PathBuf;

const MAX_RAW_LEN: usize = 4096;
const TRUNCATION_MARKER: &str = "...[truncated]";

/// Taskstore-owned classification of JSON-parse failures.
///
/// Mirrors `serde_json::error::Category` so that the parser type is never
/// exposed in taskstore's public API. A future serde_json semver bump
/// cannot force a taskstore semver bump.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Category {
    Io,
    Syntax,
    Data,
    Eof,
}

impl From<serde_json::error::Category> for Category {
    fn from(c: serde_json::error::Category) -> Self {
        match c {
            serde_json::error::Category::Io => Category::Io,
            serde_json::error::Category::Syntax => Category::Syntax,
            serde_json::error::Category::Data => Category::Data,
            serde_json::error::Category::Eof => Category::Eof,
        }
    }
}

/// Why a single JSONL line was rejected.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CorruptionError {
    /// The bytes did not parse as JSON.
    InvalidJson { msg: String, category: Category },
    /// The bytes parsed as JSON but had no `id` field of type string.
    MissingId,
    /// The bytes parsed as JSON, had an `id`, but did not deserialize to T.
    /// Only produced by `Store::list_tolerant<T>`, never by the JSONL reader alone.
    TypeMismatch { msg: String },
}

/// One rejected JSONL line, with enough context to diagnose it.
#[derive(Debug, Clone)]
pub struct CorruptionEntry {
    /// Absolute path to the JSONL file the line came from.
    pub file: PathBuf,
    /// 1-indexed line number within `file`.
    pub line: u64,
    /// The offending line, lossy UTF-8, truncated to 4 KB with
    /// `"...[truncated]"` appended when truncation occurred.
    /// For `TypeMismatch` only, this is the parsed Value re-serialized
    /// (the original line bytes are no longer in scope at that point).
    pub raw: String,
    /// Classification of why this line was rejected.
    pub error: CorruptionError,
}

/// Return type for `Store::list_tolerant<T>`.
///
/// `records` is the set of valid, deserialized records that survived
/// last-write-wins-per-id and tombstone filtering. `corruption` is one
/// entry per malformed line; it is never deduplicated.
#[derive(Debug, Clone)]
pub struct ListResult<T> {
    pub records: Vec<T>,
    pub corruption: Vec<CorruptionEntry>,
}

/// Truncate a string to 4 KB, appending `"...[truncated]"` when truncation
/// actually occurs. Char-boundary safe.
pub(crate) fn truncate_raw(mut s: String) -> String {
    if s.len() <= MAX_RAW_LEN {
        return s;
    }
    let mut end = MAX_RAW_LEN;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s.truncate(end);
    s.push_str(TRUNCATION_MARKER);
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_raw_short_unchanged() {
        let s = "short line".to_string();
        assert_eq!(truncate_raw(s.clone()), s);
    }

    #[test]
    fn truncate_raw_at_boundary_unchanged() {
        let s = "a".repeat(MAX_RAW_LEN);
        let result = truncate_raw(s.clone());
        assert_eq!(result, s);
        assert_eq!(result.len(), MAX_RAW_LEN);
    }

    #[test]
    fn truncate_raw_long_appends_marker() {
        let s = "a".repeat(MAX_RAW_LEN + 100);
        let result = truncate_raw(s);
        assert_eq!(result.len(), MAX_RAW_LEN + TRUNCATION_MARKER.len());
        assert!(result.ends_with(TRUNCATION_MARKER));
    }

    #[test]
    fn truncate_raw_respects_char_boundaries() {
        // Build a string whose byte at index MAX_RAW_LEN falls in the middle
        // of a multi-byte char.
        let mut s = "a".repeat(MAX_RAW_LEN - 1);
        s.push_str("ñextra"); // ñ is 2 bytes
        let result = truncate_raw(s);
        assert!(result.ends_with(TRUNCATION_MARKER));
        // Truncation must not split the multi-byte char.
        assert!(result.is_char_boundary(result.len() - TRUNCATION_MARKER.len()));
    }

    #[test]
    fn category_from_serde() {
        use serde_json::error::Category as SerdeCategory;
        assert_eq!(Category::from(SerdeCategory::Io), Category::Io);
        assert_eq!(Category::from(SerdeCategory::Syntax), Category::Syntax);
        assert_eq!(Category::from(SerdeCategory::Data), Category::Data);
        assert_eq!(Category::from(SerdeCategory::Eof), Category::Eof);
    }
}
