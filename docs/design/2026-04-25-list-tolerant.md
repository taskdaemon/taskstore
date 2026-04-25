# Design Document: `list_tolerant<T>` - Corruption-Aware Bulk Read

**Author:** Scott Idler
**Date:** 2026-04-25
**Status:** Implemented
**Review Passes Completed:** 5/5

## Summary

Add `Store::list_tolerant<T: Record>(&self, filters) -> eyre::Result<ListResult<T>>` that reads JSONL directly (bypassing the SQLite cache) and returns valid records plus a `Vec<CorruptionEntry>` for malformed lines. Today taskstore silently drops malformed lines: `read_jsonl_latest` logs a `warn!` and skips them, `sync()` inherits that silence, and callers using `list<T>` cannot tell that records were lost. `list_tolerant` is the additive surface that surfaces what was dropped, without changing the existing `list<T>` hot path or the on-disk format.

## Problem Statement

### Background

TaskStore's read paths compose like this today:

1. **JSONL is source of truth.** Each collection lives in `<collection>.jsonl` under `.taskstore/`. Records are appended; updates and tombstones are additional lines.
2. **SQLite is a derived cache.** `Store::sync()` reads JSONL via `jsonl::read_jsonl_latest` and rebuilds the `records` table. `is_stale()` triggers a re-sync when a JSONL mtime advances.
3. **`Store::list<T>(filters)` reads from SQLite,** for indexed-filter performance (`store.rs:340`).

`read_jsonl_latest` already tolerates malformed input - it iterates lines, calls `serde_json::from_str`, and on failure does `warn!(...) ; continue`. There is even a test for this behavior (`test_read_jsonl_malformed_line`, `jsonl.rs:181`). That tolerance is correct for boot resilience: a single bad line should not prevent the daemon from starting. But the diagnostic is thrown away. By the time `list<T>` returns, the caller sees `Ok(records_present_in_sqlite)` with no signal that anything was missing.

### Problem

A caller has no way to ask the question "did any records fail to load?" The information exists at the JSONL parsing layer but is dropped on the floor before reaching the public API. Specifically:

- A line that fails `serde_json::from_str` is logged and skipped.
- A line with valid JSON but no `id` field is logged and skipped.
- A line that parses as `serde_json::Value` but does not deserialize to `T` is currently invisible at the JSONL layer (deserialization to `T` happens later in `list<T>` via SQLite, where it propagates an `eyre::Report` for the whole call - the corruption story for typed deserialization is split across two layers).

The initiating use case is loopr's daemon-boot sweep, which wants to inspect every collection on startup and report corruption rather than silently self-heal. The same surface is useful for any audit, doctor, or repair tool that wants line-level visibility.

### Goals

- Single new method on `Store`: `list_tolerant<T: Record>(&self, filters) -> eyre::Result<ListResult<T>>`.
- Reads JSONL directly. SQLite is bypassed for this method.
- Returns line-level corruption signal: number of malformed lines and where they live.
- No change to existing `list<T>` behavior.
- No change to JSONL on-disk format.
- No automatic recovery, quarantine, or repair.
- Internal refactor of the JSONL reader so the existing tolerant behavior is reused, not duplicated.

The return type matches taskstore's existing convention (`eyre::Result` throughout). If taskstore later introduces a typed `StoreError`, this method participates in that change like every other public API; the typed-error question is out of scope for this document.

### Non-Goals

- **Per-collection variants** (`list_by_parent_id_tolerant` etc.). The existing API is generic over `T: Record`; one method covers all collections.
- **Streaming iterator.** Last-write-wins-per-id requires buffering the whole file; a "streaming" iterator that gives that up has different semantics from `list`. Not worth the surface area.
- **Audit-only helper.** `list_tolerant::<T>(&[])` with no filter is the audit. One method covers both jobs.
- **Change to `get(id)`.** Single-record reads keep their existing error behavior.
- **Change to `updated_at` defaulting.** Existing reader silently defaults missing/non-int `updated_at` to 0; preserve that. Not classified as corruption.
- **Auto-recovery, quarantine moves, JSONL format changes.** Out of scope.
- **Performance parity with `list<T>`.** `list_tolerant` re-parses JSONL on every call. Documented as a sweep/audit path, not a hot read.

## Proposed Solution

### Overview

Refactor `read_jsonl_latest` so that its existing tolerant behavior is callable by both `sync()` (which keeps swallowing the diagnostic by design - boot resilience) and the new `list_tolerant<T>` (which surfaces it). Add three public types - `ListResult<T>`, `CorruptionEntry`, `CorruptionError` - and one new method that ties them together.

### API Design

```rust
// All types live in the taskstore crate root (re-exported from src/lib.rs).
// Filter is the existing taskstore::Filter (crate::filter::Filter).

impl Store {
    pub fn list_tolerant<T: Record>(
        &self,
        filters: &[Filter],
    ) -> eyre::Result<ListResult<T>>;
}

pub struct ListResult<T> {
    pub records:    Vec<T>,
    pub corruption: Vec<CorruptionEntry>,
}

pub struct CorruptionEntry {
    pub file:  PathBuf,
    pub line:  u64,            // 1-indexed within the JSONL
    pub raw:   String,         // original line bytes, lossy UTF-8, truncated to 4 KB with "...[truncated]" appended
    pub error: CorruptionError,
}

pub enum CorruptionError {
    // `category` is the taskstore-owned mirror of serde_json's classification;
    // see "Category Re-Export" below.
    InvalidJson { msg: String, category: Category },
    MissingId,
    TypeMismatch { msg: String },   // valid JSON, has id, but T deserialize failed
}
```

The signature mirrors `Store::list<T>`: same `&self` receiver, same `&[Filter]` parameter, same generic constraint. The only differences are the return type and the read path.

### Read Path: JSONL-Direct, Committed

`list_tolerant` always reads JSONL directly. Reasons:

- **Detection at source of truth.** SQLite is the silenced view; reading from it would mask the very condition we are trying to surface.
- **No race window.** A clean SQLite that was last synced before today's corruption would lie to the caller.
- **No new `Store` state.** No `last_sync_corruption()` to maintain across calls.
- **Hot path unaffected.** `list<T>` continues to read SQLite. Callers who want speed use `list`; callers who want truth use `list_tolerant`.

A future `list_tolerant_cached` (SQLite + a remembered `last_sync_corruption()`) can be added later as a separate method without touching this one.

### Counting Contract: Line-Level

- `corruption.len() == K` where K is the number of malformed lines.
- `records.len()` is **not** asserted as N - K. Last-write-wins-per-id and tombstone filtering still apply on top; the relationship between line count and record count is not specifiable without inspecting ids, and a corrupt line by definition does not have a usable id.
- `CorruptionEntry`s are **not** deduplicated. Two malformed lines that happen to share content (or where one is a malformed copy of another's id) each produce their own `CorruptionEntry`. There is no key by which they could be coalesced, and a sweep caller wants line-by-line visibility regardless.

### Behavior Specifications

| Input | Output |
|-------|--------|
| Well-formed JSONL, mix of records and tombstones | `Ok(ListResult { records: live_records_after_lww_and_tombstone_filter, corruption: [] })` |
| Line fails `serde_json::from_str` | Surfaces as `CorruptionEntry { error: InvalidJson { msg, category }, .. }`. Skipped from `records`. |
| Line is valid JSON but lacks `id` | Surfaces as `CorruptionEntry { error: MissingId, .. }`. Skipped from `records`. |
| Line is valid JSON, has `id`, but does not deserialize to `T` | Surfaces as `CorruptionEntry { error: TypeMismatch { msg }, .. }`. Skipped from `records`. |
| Line is `{"deleted": true, ...}` and well-formed | Filtered from `records` (matching `list<T>` semantics). Not counted as corruption. |
| Empty JSONL file | `Ok(ListResult { records: [], corruption: [] })` |
| Missing JSONL file (collection never written) | Same: `Ok(ListResult { records: [], corruption: [] })` |
| Unreadable JSONL (perms, fs error) | `Err(eyre::Report)` with file-context. No `ListResult` produced. |
| `updated_at` missing or non-int on an otherwise valid line | Existing default-to-0 behavior preserved. Not corruption. |
| Line bytes are not valid UTF-8 | Treated as `InvalidJson` (the parse will fail). `raw` is populated via `String::from_utf8_lossy` so callers always get a printable form. |
| Tombstone line that fails JSON parse | Surfaces as `InvalidJson` corruption. The id it was meant to tombstone is unknown, so the prior live record (if any) stays in `records`. Documented consequence: a corrupt tombstone leaves the target record alive in `list_tolerant` output. |
| Filter references a field absent from `T::indexed_fields()` | `match_filter` returns `false` for that record (no match). Mirrors the SQL path's `EXISTS` semantics. |
| Filter `value` type does not match the indexed field's `IndexValue` variant | `match_filter` returns `false`. Mirrors the SQL path, which keys off the typed columns (`field_value_str` / `_int` / `_bool`). |
| Filter `op` is `Contains` | `match_filter` performs an ASCII-case-insensitive substring match (lowercase both sides, then `str::contains`). This mirrors SQLite's `LIKE` semantics (ASCII-case-insensitive by default), so an identical filter returns the same set of records via `list<T>` and `list_tolerant<T>`. Non-ASCII case-folding intentionally not handled - SQLite's `LIKE` does not handle it either, and parity is more valuable than completeness. |
| Two valid lines for id X: line 10 valid as `T`, line 20 valid JSON with later `updated_at` but invalid as `T` | Last-write-wins selects line 20's `Value`. Typed deserialization fails. id X surfaces as `TypeMismatch` corruption; the older valid record is **not** in `records`. This matches `sync()`'s existing behavior (sync would also overwrite SQLite with line 20 and fail at typed read time). The "latest write wins, even if it breaks the type" rule is the same one taskstore applies everywhere; `list_tolerant` does not introduce a new fallback path. |

`list<T>` behavior is unchanged: still reads SQLite, still returns `Ok(records_present_in_sqlite)` with no corruption signal. The new signal lives only in `list_tolerant`.

### `Category` Type: Decided

`CorruptionError::InvalidJson::category` is a taskstore-owned `enum Category { Syntax, Eof, Data, Io }`, defined alongside the other corruption types in `src/corruption.rs`. The conversion from `serde_json::error::Category` is implemented as `impl From<serde_json::error::Category> for Category` so that the parser type never appears in taskstore's public surface; a future serde_json semver bump cannot force a taskstore semver bump.

### Internal Refactor

Today `jsonl::read_jsonl_latest` parses lines, dedupes by id (last-write-wins-per-id), and returns `HashMap<String, serde_json::Value>`. It is **already tolerant** of malformed lines (logs `warn!` and skips). The diagnostics are emitted to the log and discarded.

The internal change is to share the iteration loop between two callers - one that keeps today's silent behavior, one that returns the diagnostics:

```rust
// Public: today's behavior, callers that don't care about corruption.
// sync() keeps calling this; no behavior change there.
pub fn read_jsonl_latest(path: &Path) -> eyre::Result<HashMap<String, Value>>;

// Public: same iteration, returns the entries that would otherwise be warn!-logged.
// The winning line's number is preserved alongside its Value so that downstream
// typed deserialization can attribute a TypeMismatch back to a specific line.
pub fn read_jsonl_latest_with_corruption(
    path: &Path,
) -> eyre::Result<(HashMap<String, (u64, Value)>, Vec<CorruptionEntry>)>;
```

The line-number-bearing return type is unique to the with-corruption variant. `read_jsonl_latest`'s public signature is unchanged - `sync()` does not learn about line numbers and pays no extra memory cost. The 8 bytes-per-record overhead is paid only by `list_tolerant` callers.

Implementation strategy: the existing parse-and-dedup loop is extracted into a private helper that, given a path, yields `(line_no, original_line_text, Result<(id, Value), CorruptionError>)` per non-empty line. The `path` and `line_no` from the helper become the `file` and `line` fields of any `CorruptionEntry`; the original line text becomes `raw` (so `raw` always reflects on-disk bytes, never re-serialized JSON). The dedup wrapper for the with-corruption variant carries the line number through into the returned `HashMap`'s value side. Both public functions consume the helper - `read_jsonl_latest` keeps `warn!`-and-discard, `read_jsonl_latest_with_corruption` materialises the `Err` arm into a `CorruptionEntry` and pushes it to the returned `Vec`. This avoids two parallel parsers drifting out of sync.

`list_tolerant<T>`'s body is then:

1. Build the JSONL path from `T::collection_name()` and `self.base_path`.
2. Call `read_jsonl_latest_with_corruption(&path)`. Propagate I/O errors directly via `?`.
3. Drop tombstones from the returned `HashMap<String, (u64, Value)>` (matching `sync()`'s tombstone filter). Tombstone detection runs against the `Value`; the `(line_no, Value)` tuple is preserved for the rest.
4. For each remaining `(id, (line_no, Value))`, deserialize to `T`. On failure, build a `CorruptionEntry { error: TypeMismatch { msg }, raw: <Value::to_string truncated>, line: line_no, file: path.clone(), }` and push it to `corruption`; on success, keep the `T`. Note: `raw` for `TypeMismatch` is the parsed Value re-serialized (the original line text is no longer in scope at this point - the helper yielded a `Value`). This is the only `CorruptionError` variant where `raw` is a re-serialization rather than original bytes; document on the field.
5. Apply `filters` over the deserialized records. The existing SQL path (`store.rs:340`) compiles `FilterOp::to_sql()` into a SQL `WHERE`; the in-Rust path needs a small parallel helper - `fn match_filter(fields: &HashMap<String, IndexValue>, f: &Filter) -> bool` - that performs the same comparison in code, including ASCII-case-insensitive matching for `FilterOp::Contains` (mirrors SQLite's `LIKE`). Records survive iff every filter matches. This helper is new code; place it in `src/filter.rs` next to the existing types, with tests that pin the SQL-vs-Rust parity for each `FilterOp` variant.
6. Return `Ok(ListResult { records, corruption })`.

`list_tolerant` does **not** trigger `sync()` and does not write to SQLite. It is read-only against JSONL.

Concurrency: the JSONL reader takes a shared `fs2` file lock, identical to the existing `read_jsonl_latest`. Concurrent writers (which take an exclusive lock) block `list_tolerant` and vice versa. This matches the existing read discipline; no new contention shape.

### Implementation Plan

#### Phase 1: Public types and refactored JSONL reader
**Model:** sonnet

- Add a new module `src/corruption.rs` containing `CorruptionEntry`, `CorruptionError`, `Category` (taskstore-owned mirror with `From<serde_json::error::Category>`), and `ListResult<T>`. Placing them in their own module keeps the type surface discoverable and avoids growing `jsonl.rs` or `store.rs` unnecessarily.
- Re-export the new types from `src/lib.rs`.
- Extract the parse-and-dedup loop from `read_jsonl_latest` into a private helper that yields `(line_no, original_line_text, Result<(id, Value), CorruptionError>)`. The existing `read_jsonl_latest` becomes a thin wrapper that consumes the helper and warn-logs failures (preserving today's behavior). Add a new `read_jsonl_latest_with_corruption` wrapper that materialises failures into `CorruptionEntry`.
- `sync()` is not edited. It continues to call `read_jsonl_latest`. The existing `test_read_jsonl_malformed_line` still passes unchanged.

#### Phase 2: `match_filter` helper and `Store::list_tolerant`
**Model:** sonnet

- Add `pub(crate) fn match_filter(fields: &HashMap<String, IndexValue>, f: &Filter) -> bool` in `src/filter.rs`. Mirrors the comparisons that `FilterOp::to_sql` produces in `store.rs:340`, with one explicit semantic carry-over: `FilterOp::Contains` is ASCII-case-insensitive (matching SQLite `LIKE`'s default). Tests must pin SQL-vs-Rust parity for every `FilterOp` variant.
- Add `Store::list_tolerant<T: Record>` in `src/store.rs`. Build the JSONL path, call `read_jsonl_latest_with_corruption`, drop tombstones, deserialize-each with `TypeMismatch` capture, apply filters via `match_filter`, return `ListResult`.
- Method-level rustdoc states: re-parses JSONL on every call (not a hot read), filters are applied in Rust without SQLite index pushdown, does not trigger `sync()`.

#### Phase 3: Test contract
**Model:** sonnet

The PR must include tests covering:

- 100 lines / 3 deliberately corrupted (one Syntax, one Eof-like truncation, one MissingId): `corruption.len() == 3`; each entry carries `file`, `line`, `raw`, `error`.
- 100 lines / 1 line that is valid JSON with an `id` but a field-shape that fails to deserialize to `T`: surfaces as `TypeMismatch`. The `line` field of the `CorruptionEntry` matches the on-disk line number of the LWW-winning entry for that id (not necessarily the first line for that id).
- LWW masking test: line 10 valid as `T` for id X, line 20 same id with a later `updated_at` and a field-shape invalid for `T`. `list_tolerant` reports id X as `TypeMismatch` (not present in `records`); `corruption[].line == 20`.
- `Contains` parity test: an identical `Filter { op: Contains, value: "ACTIVE" }` returns the same set of records via `list<T>` (post-sync, SQLite path) and via `list_tolerant<T>` (post-deserialize, `match_filter` path), including for records whose indexed value differs only in case.
- After `sync()` runs over the same corrupted JSONL, the existing `list<T>` returns `Ok(...)` containing whatever survived sync's silent-skip (no new error path introduced). The corruption signal is unique to `list_tolerant`.
- Unreadable JSONL (chmod 000 in test, or simulate via a temp dir): `list_tolerant` returns `Err(eyre::Report)`, no `ListResult`.
- Empty JSONL: `Ok(ListResult { records: [], corruption: [] })`.
- Missing JSONL (collection never written): same as empty.
- 100 lines including 5 tombstones for already-present ids: tombstones filtered from `records`, do not appear in `corruption`.
- Filter pushdown happens post-deserialize: `list_tolerant::<T>(&[Filter::eq("status", "active")])` returns the same set as `list::<T>(&[same filter])` would on the post-sync SQLite (when no corruption is present).
- `raw` truncation: a line longer than 4 KB has its `raw` truncated to 4 KB and the literal string `"...[truncated]"` appended.

## Alternatives Considered

### Alternative 1: Mutate `list<T>` to return corruption alongside records

- **Description:** Change `list<T>` to return `(Vec<T>, Vec<CorruptionEntry>)` or a wrapping type.
- **Pros:** One method instead of two; callers always see the signal.
- **Cons:** Breaking change to a hot-path API. Forces every caller to handle a corruption vec they may not care about. Forces `list` to read JSONL directly, losing the SQLite index path.
- **Why not chosen:** The detection signal is needed by a small number of audit callers; the hot path should not pay for it. Additive method is the right shape.

### Alternative 2: `last_sync_corruption()` accessor on `Store`

- **Description:** Have `sync()` remember what it skipped; expose `Store::last_sync_corruption() -> &[CorruptionEntry]`. `list<T>` is unchanged; auditors check the accessor.
- **Pros:** Auditor reads SQLite (fast); corruption signal is decoupled from any single read.
- **Cons:** Stateful. `sync()` only runs when `is_stale()` returns true, so the accessor reflects "what was skipped at last sync," not "what is corrupt right now." A caller that wants ground truth would have to force a sync first. Adds `Store` state that has to be invalidated on every `sync()` call.
- **Why not chosen:** Adds complexity for a use case that JSONL-direct read covers more directly. Could still be added later as `list_tolerant_cached` if profiling shows the JSONL re-parse is hurting.

### Alternative 3: Per-collection methods (`list_plans_tolerant` etc.)

- **Description:** One method per collection type.
- **Pros:** Loopr's original framing.
- **Cons:** TaskStore's API is generic over `T: Record`; per-collection variants do not exist. Adding them would invert the API shape.
- **Why not chosen:** Wrong shape for taskstore. One generic method covers it.

### Alternative 4: Return a streaming iterator

- **Description:** `iter_records_tolerant() -> impl Iterator<Item = (u64, Result<T, CorruptionError>)>`.
- **Pros:** Avoids buffering for very large JSONL.
- **Cons:** Last-write-wins-per-id requires seeing every line before yielding any record - a streaming iterator either gives that up (different semantics from `list`) or buffers internally (not really streaming).
- **Why not chosen:** No callers want the alternative semantics. Bulk method covers the use case.

## Technical Considerations

### Dependencies

No new external dependencies. Uses existing `serde_json`, `fs2`, `std::path::PathBuf`.

### Performance

`list_tolerant` re-parses the full JSONL file on every call. For collections that grow unboundedly (e.g., a tick stream), the cost scales linearly with file size. This is intentional: callers opt into the cost when they want detection. The hot path (`list<T>`) is untouched and continues to use the SQLite index.

Memory: like the existing `read_jsonl_latest`, the tolerant variant materialises a `HashMap<String, (u64, Value)>` of every live record plus a `Vec<CorruptionEntry>` (typically empty). Peak memory is proportional to collection size, not pipeline-streamed. Callers running audits on extremely large collections should expect a slightly larger allocation profile than `sync()` does today (8 extra bytes per record for the line number); not a regression but worth being aware of.

Per-record allocator pressure during filtering: `match_filter` is called once per surviving record, and each call invokes `Record::indexed_fields()` (`src/record.rs:22`), which allocates an owned `HashMap<String, IndexValue>`. The SQL path avoids this allocation by pushing the comparison into SQLite. For an audit sweep this is the cost of "JSONL-direct, no SQL pushdown" - we accepted O(N) deserialize, and per-record HashMap allocation is in the same bucket. If a future profiling pass shows this is the dominant cost on a real audit, we can extend `Record` with a `match_field(&self, field: &str, value: &IndexValue, op: FilterOp) -> bool` default method that avoids the HashMap intermediate; that change is additive and out of scope for this design.

If profiling later shows audit calls are too slow on a particular collection, two follow-ups are available without changing this method's contract:

1. Add `list_tolerant_cached` (SQLite + remembered `last_sync_corruption`).
2. Cache the JSONL file mtime + parsed result inside `list_tolerant` calls within a single `Store` lifetime.

### Security

`raw` field returns the bytes of a corrupt line, truncated to 4 KB. If a JSONL collection ever contains user-supplied content, callers logging `CorruptionEntry` should treat `raw` with the same care as any other user-content field. Documented on the type.

### Testing Strategy

Covered in Phase 3. Round-trip tests use `tempfile::TempDir` and `Store::open`; corrupt-line tests write JSONL bytes directly to bypass the writer's validation.

### Rollout Plan

Additive change. New method, new public types, no migration. Released in the next taskstore minor version. Loopr pins to that version when it consumes the new surface.

## Risks and Mitigations

| Risk | Likelihood | Impact | Mitigation |
|------|------------|--------|------------|
| `serde_json::error::Category` semver coupling if re-exported directly | Medium | Low | Use taskstore-owned `Category` mirror with `From<serde_json::error::Category>` |
| Two parallel JSONL parsers drift over time | Low | Medium | Share a single iteration loop; the strict and tolerant variants are thin wrappers |
| Caller misuses `list_tolerant` as a hot path | Low | Medium | Document on the method that this re-parses JSONL on every call; provide `list<T>` as the hot-path alternative |
| `raw` field exposes sensitive content to logs | Low | Low | Document on the type; truncate to 4 KB; let callers redact |
| Tombstone-only line at the end of a file is mis-classified | Low | Low | Tombstone path is a positive check on `deleted == true` after JSON parse succeeds; mis-classification would require a JSON-parse failure on a tombstone line, which lands in `corruption` correctly |

## Open Questions

None. The three implementation-time decisions originally listed here (`Category` type, `CorruptionEntry` module placement, `raw` truncation marker) are resolved in their respective sections above.

## References

- `src/jsonl.rs:36` - existing `read_jsonl_latest` (the function being refactored)
- `src/jsonl.rs:181` - existing `test_read_jsonl_malformed_line` (current tolerant behavior)
- `src/store.rs:340` - existing `Store::list<T>` (the read path being supplemented, not modified)
- `src/store.rs:601` - existing `Store::sync` (continues to call `read_jsonl_latest`; no behavior change)
- `docs/design/2026-04-13-create-many.md` - prior additive method on `Store` (style reference)
