// TaskStore library - Persistent state management with SQLite+JSONL+Git

pub mod jsonl;
pub mod models;
pub mod store;

// Re-export main types for convenience
pub use models::*;
pub use store::Store;

// Keep existing CLI/config for now (will be refactored in Phase 6)
pub mod cli;
pub mod config;
