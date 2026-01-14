// Data models for TaskStore

use serde::{Deserialize, Serialize};

/// Product Requirements Document
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Prd {
    pub id: String,
    pub title: String,
    pub description: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub status: PrdStatus,
    pub review_passes: u8,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrdStatus {
    Draft,
    Ready,
    Active,
    Complete,
    Cancelled,
}

/// Task Specification (decomposed from PRDs)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskSpec {
    pub id: String,
    pub prd_id: String,
    pub phase_name: String,
    pub description: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub status: TaskSpecStatus,
    pub workflow_name: Option<String>,
    pub assigned_to: Option<String>,
    pub content: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskSpecStatus {
    Pending,
    Running,
    Complete,
    Failed,
}

/// Execution State (loop instances)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Execution {
    pub id: String,
    pub ts_id: String,
    pub worktree_path: String,
    pub branch_name: String,
    pub status: ExecStatus,
    pub started_at: i64,
    pub updated_at: i64,
    pub completed_at: Option<i64>,
    pub current_phase: Option<String>,
    pub iteration_count: u32,
    pub error_message: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExecStatus {
    Running,
    Paused,
    Complete,
    Failed,
    Stopped,
}

/// Dependency (coordination between executions)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Dependency {
    pub id: String,
    pub from_exec_id: String,
    pub to_exec_id: String,
    pub dependency_type: DependencyType,
    pub created_at: i64,
    pub resolved_at: Option<i64>,
    pub payload: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DependencyType {
    Notify,
    Query,
    Share,
}

/// AWL Workflow Definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub id: String,
    pub name: String,
    pub version: String,
    pub created_at: i64,
    pub updated_at: i64,
    pub content: String,
}

/// Repository State (per-repo metadata)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepoState {
    pub repo_path: String,
    pub last_synced_commit: String,
    pub updated_at: i64,
}

/// Helper function to get current timestamp in milliseconds
pub fn now_ms() -> i64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("System time before Unix epoch")
        .as_millis() as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_now_ms() {
        let ts = now_ms();
        assert!(ts > 0);
        // Should be reasonable timestamp (after year 2020)
        assert!(ts > 1_600_000_000_000);
    }

    #[test]
    fn test_prd_status_serialization() {
        let status = PrdStatus::Draft;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"draft\"");

        let status = PrdStatus::Active;
        let json = serde_json::to_string(&status).unwrap();
        assert_eq!(json, "\"active\"");
    }

    #[test]
    fn test_prd_serialization() {
        let prd = Prd {
            id: "test-id".to_string(),
            title: "Test PRD".to_string(),
            description: "Test description".to_string(),
            created_at: 1000,
            updated_at: 1000,
            status: PrdStatus::Draft,
            review_passes: 5,
            content: "# Test Content".to_string(),
        };

        let json = serde_json::to_string(&prd).unwrap();
        let deserialized: Prd = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.id, prd.id);
        assert_eq!(deserialized.title, prd.title);
    }
}
