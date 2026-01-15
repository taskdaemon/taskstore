use clap::{Parser, Subcommand};
use eyre::Result;
use std::path::PathBuf;
use taskstore::Store;

#[derive(Parser)]
#[command(name = "taskstore")]
#[command(about = "TaskStore CLI - Generic persistent state management with SQLite+JSONL+Git")]
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
    /// Sync SQLite database from JSONL files
    Sync,

    /// Install git hooks for automatic syncing
    InstallHooks,
}

fn main() -> Result<()> {
    // Setup tracing
    tracing_subscriber::fmt::init();

    let cli = Cli::parse();

    // Open store
    let mut store = Store::open(&cli.store_path)?;

    match cli.command {
        Commands::Sync => {
            println!("Syncing database from JSONL files...");
            store.sync()?;
            println!("Sync complete");
        }
        Commands::InstallHooks => {
            println!("Installing git hooks...");
            store.install_git_hooks()?;
            println!("Git hooks installed successfully");
        }
    }

    Ok(())
}
