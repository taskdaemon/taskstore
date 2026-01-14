use clap::{Parser, Subcommand};
use colored::*;
use eyre::{Context, Result};
use std::path::PathBuf;
use taskstore::{ExecStatus, PrdStatus, Store, TaskSpecStatus};

#[derive(Parser)]
#[command(name = "taskstore")]
#[command(about = "TaskStore CLI - Persistent state management with SQLite+JSONL+Git")]
#[command(version = env!("GIT_DESCRIBE"))]
struct Cli {
    /// Path to the store directory (default: current directory)
    #[arg(short, long, default_value = ".")]
    store_path: PathBuf,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// List all PRDs
    ListPrds {
        /// Filter by status (draft, ready, active, complete, cancelled)
        #[arg(short, long)]
        status: Option<String>,
    },

    /// List task specifications for a PRD
    ListTaskSpecs {
        /// PRD ID to list task specs for
        #[arg(value_name = "PRD_ID")]
        prd_id: String,
    },

    /// List all executions
    ListExecutions {
        /// Filter by status (running, paused, complete, failed, stopped)
        #[arg(short, long)]
        status: Option<String>,
    },

    /// Show detailed information about a record
    Show {
        /// Type of record (prd, ts, execution)
        #[arg(value_name = "TYPE")]
        record_type: String,

        /// ID of the record
        #[arg(value_name = "ID")]
        id: String,
    },

    /// Sync SQLite database from JSONL files
    Sync,

    /// Install git hooks for automatic syncing
    InstallHooks,

    /// Show store statistics
    Stats,
}

fn main() -> Result<()> {
    // Setup tracing
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    // Open store
    let mut store = Store::open(&cli.store_path).context("Failed to open store")?;

    match cli.command {
        Commands::ListPrds { status } => {
            let filter_status = status.map(|s| parse_prd_status(&s)).transpose()?;
            let prds = store.list_prds(filter_status)?;

            if prds.is_empty() {
                println!("{}", "No PRDs found".yellow());
                return Ok(());
            }

            println!("{}", format!("Found {} PRD(s)", prds.len()).cyan());
            println!();
            println!("{:<20} {:<40} {:<10} {:<8}", "ID", "Title", "Status", "Passes");
            println!("{}", "-".repeat(80));

            for prd in prds {
                let status_colored = match prd.status {
                    PrdStatus::Active => prd.status.to_string().green(),
                    PrdStatus::Draft => prd.status.to_string().yellow(),
                    PrdStatus::Complete => prd.status.to_string().blue(),
                    PrdStatus::Cancelled => prd.status.to_string().red(),
                    _ => prd.status.to_string().normal(),
                };
                println!(
                    "{:<20} {:<40} {:<10} {:<8}",
                    truncate(&prd.id, 20),
                    truncate(&prd.title, 40),
                    status_colored,
                    prd.review_passes
                );
            }
        }

        Commands::ListTaskSpecs { prd_id } => {
            let specs = store.list_task_specs(&prd_id)?;

            if specs.is_empty() {
                println!("{}", format!("No task specs found for PRD '{}'", prd_id).yellow());
                return Ok(());
            }

            println!(
                "{}",
                format!("Found {} task spec(s) for PRD '{}'", specs.len(), prd_id).cyan()
            );
            println!();
            println!("{:<20} {:<30} {:<20} {:<15}", "ID", "Phase", "PRD ID", "Status");
            println!("{}", "-".repeat(85));

            for spec in specs {
                let status_colored = match spec.status {
                    TaskSpecStatus::Running => spec.status.to_string().green(),
                    TaskSpecStatus::Pending => spec.status.to_string().yellow(),
                    TaskSpecStatus::Complete => spec.status.to_string().blue(),
                    TaskSpecStatus::Failed => spec.status.to_string().red(),
                };
                println!(
                    "{:<20} {:<30} {:<20} {:<15}",
                    truncate(&spec.id, 20),
                    truncate(&spec.phase_name, 30),
                    truncate(&spec.prd_id, 20),
                    status_colored
                );
            }
        }

        Commands::ListExecutions { status } => {
            let filter_status = status.map(|s| parse_exec_status(&s)).transpose()?;
            let executions = store.list_executions(filter_status)?;

            if executions.is_empty() {
                println!("{}", "No executions found".yellow());
                return Ok(());
            }

            println!("{}", format!("Found {} execution(s)", executions.len()).cyan());
            println!();
            println!(
                "{:<20} {:<30} {:<15} {:<20}",
                "ID", "Task Spec ID", "Status", "Started At"
            );
            println!("{}", "-".repeat(85));

            for exec in executions {
                let status_colored = match exec.status {
                    ExecStatus::Running => exec.status.to_string().green(),
                    ExecStatus::Paused => exec.status.to_string().yellow(),
                    ExecStatus::Complete => exec.status.to_string().blue(),
                    ExecStatus::Failed => exec.status.to_string().red(),
                    ExecStatus::Stopped => exec.status.to_string().red(),
                };
                println!(
                    "{:<20} {:<30} {:<15} {:<20}",
                    truncate(&exec.id, 20),
                    truncate(&exec.ts_id, 30),
                    status_colored,
                    format_timestamp(exec.started_at)
                );
            }
        }

        Commands::Show { record_type, id } => match record_type.as_str() {
            "prd" => {
                if let Some(prd) = store.get_prd(&id)? {
                    println!("{}", "PRD Details".cyan().bold());
                    println!("{}", "=".repeat(80));
                    println!("{:<15} {}", "ID:", prd.id);
                    println!("{:<15} {}", "Title:", prd.title);
                    println!("{:<15} {}", "Description:", prd.description);
                    println!("{:<15} {}", "Status:", format_prd_status(prd.status));
                    println!("{:<15} {}", "Review Passes:", prd.review_passes);
                    println!("{:<15} {}", "Created At:", format_timestamp(prd.created_at));
                    println!("{:<15} {}", "Updated At:", format_timestamp(prd.updated_at));
                    println!();
                    println!("{}", "Content:".cyan());
                    println!("{}", "-".repeat(80));
                    println!("{}", prd.content);
                } else {
                    println!("{}", format!("PRD '{}' not found", id).red());
                }
            }
            "ts" => {
                if let Some(spec) = store.get_task_spec(&id)? {
                    println!("{}", "Task Spec Details".cyan().bold());
                    println!("{}", "=".repeat(80));
                    println!("{:<15} {}", "ID:", spec.id);
                    println!("{:<15} {}", "Phase:", spec.phase_name);
                    println!("{:<15} {}", "Description:", spec.description);
                    println!("{:<15} {}", "PRD ID:", spec.prd_id);
                    println!("{:<15} {}", "Status:", format_task_spec_status(spec.status));
                    if let Some(workflow) = &spec.workflow_name {
                        println!("{:<15} {}", "Workflow:", workflow);
                    }
                    if let Some(assigned) = &spec.assigned_to {
                        println!("{:<15} {}", "Assigned To:", assigned);
                    }
                    println!("{:<15} {}", "Created At:", format_timestamp(spec.created_at));
                    println!("{:<15} {}", "Updated At:", format_timestamp(spec.updated_at));
                    println!();
                    println!("{}", "Content:".cyan());
                    println!("{}", "-".repeat(80));
                    println!("{}", spec.content);
                } else {
                    println!("{}", format!("Task spec '{}' not found", id).red());
                }
            }
            "execution" => {
                if let Some(exec) = store.get_execution(&id)? {
                    println!("{}", "Execution Details".cyan().bold());
                    println!("{}", "=".repeat(80));
                    println!("{:<15} {}", "ID:", exec.id);
                    println!("{:<15} {}", "Task Spec ID:", exec.ts_id);
                    println!("{:<15} {}", "Worktree Path:", exec.worktree_path);
                    println!("{:<15} {}", "Branch Name:", exec.branch_name);
                    println!("{:<15} {}", "Status:", format_exec_status(exec.status));
                    println!("{:<15} {}", "Started At:", format_timestamp(exec.started_at));
                    println!("{:<15} {}", "Updated At:", format_timestamp(exec.updated_at));
                    if let Some(completed_at) = exec.completed_at {
                        println!("{:<15} {}", "Completed At:", format_timestamp(completed_at));
                    }
                    if let Some(phase) = &exec.current_phase {
                        println!("{:<15} {}", "Current Phase:", phase);
                    }
                    println!("{:<15} {}", "Iteration Count:", exec.iteration_count);
                    if let Some(error) = &exec.error_message {
                        println!();
                        println!("{}", "Error Message:".red());
                        println!("{}", "-".repeat(80));
                        println!("{}", error);
                    }
                } else {
                    println!("{}", format!("Execution '{}' not found", id).red());
                }
            }
            _ => {
                println!(
                    "{}",
                    format!("Unknown record type '{}'. Valid types: prd, ts, execution", record_type).red()
                );
            }
        },

        Commands::Sync => {
            println!("{}", "Syncing SQLite from JSONL files...".cyan());
            store.sync()?;
            println!("{}", "✓ Sync complete".green());
        }

        Commands::InstallHooks => {
            println!("{}", "Installing git hooks...".cyan());
            store.install_git_hooks()?;
            println!("{}", "✓ Git hooks installed successfully".green());
            println!("  - pre-commit: taskstore sync");
            println!("  - post-merge: taskstore sync");
            println!("  - post-rebase: taskstore sync");
            println!("  - pre-push: taskstore sync");
            println!("  - post-checkout: taskstore sync");
        }

        Commands::Stats => {
            let all_prds = store.list_prds(None)?;

            println!("{}", "Store Statistics".cyan().bold());
            println!("{}", "=".repeat(40));
            println!("{:<20} {}", "Total PRDs:", all_prds.len());
            println!();

            // PRD status breakdown
            if !all_prds.is_empty() {
                println!("{}", "PRD Status Breakdown:".cyan());
                for status in [
                    PrdStatus::Draft,
                    PrdStatus::Ready,
                    PrdStatus::Active,
                    PrdStatus::Complete,
                    PrdStatus::Cancelled,
                ] {
                    let count = all_prds.iter().filter(|p| p.status == status).count();
                    if count > 0 {
                        println!("  {:<12} {}", format!("{}:", status), count);
                    }
                }
            }
        }
    }

    Ok(())
}

// Helper functions

fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len.saturating_sub(3)])
    }
}

fn format_timestamp(ms: i64) -> String {
    use chrono::{DateTime, Utc};
    let dt = DateTime::<Utc>::from_timestamp(ms / 1000, ((ms % 1000) * 1_000_000) as u32);
    match dt {
        Some(dt) => dt.format("%Y-%m-%d %H:%M:%S").to_string(),
        None => "Invalid timestamp".to_string(),
    }
}

fn parse_prd_status(s: &str) -> Result<PrdStatus> {
    match s.to_lowercase().as_str() {
        "draft" => Ok(PrdStatus::Draft),
        "ready" => Ok(PrdStatus::Ready),
        "active" => Ok(PrdStatus::Active),
        "complete" => Ok(PrdStatus::Complete),
        "cancelled" => Ok(PrdStatus::Cancelled),
        _ => Err(eyre::eyre!("Invalid PRD status: {}", s)),
    }
}

fn parse_exec_status(s: &str) -> Result<ExecStatus> {
    match s.to_lowercase().as_str() {
        "running" => Ok(ExecStatus::Running),
        "paused" => Ok(ExecStatus::Paused),
        "complete" => Ok(ExecStatus::Complete),
        "failed" => Ok(ExecStatus::Failed),
        "stopped" => Ok(ExecStatus::Stopped),
        _ => Err(eyre::eyre!("Invalid execution status: {}", s)),
    }
}

fn format_prd_status(status: PrdStatus) -> ColoredString {
    match status {
        PrdStatus::Active => status.to_string().green(),
        PrdStatus::Draft => status.to_string().yellow(),
        PrdStatus::Complete => status.to_string().blue(),
        PrdStatus::Cancelled => status.to_string().red(),
        _ => status.to_string().normal(),
    }
}

fn format_task_spec_status(status: TaskSpecStatus) -> ColoredString {
    match status {
        TaskSpecStatus::Running => status.to_string().green(),
        TaskSpecStatus::Pending => status.to_string().yellow(),
        TaskSpecStatus::Complete => status.to_string().blue(),
        TaskSpecStatus::Failed => status.to_string().red(),
    }
}

fn format_exec_status(status: ExecStatus) -> ColoredString {
    match status {
        ExecStatus::Running => status.to_string().green(),
        ExecStatus::Paused => status.to_string().yellow(),
        ExecStatus::Complete => status.to_string().blue(),
        ExecStatus::Failed => status.to_string().red(),
        ExecStatus::Stopped => status.to_string().red(),
    }
}
