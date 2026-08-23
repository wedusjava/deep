mod agent;
mod clients;
mod credentials;
mod store;
mod tui;

use std::path::PathBuf;

use anyhow::{Result, anyhow};
use clap::{Parser, Subcommand};
use directories::ProjectDirs;

use credentials::CredentialStore;

#[derive(Parser)]
#[command(
    name = "deep",
    version,
    about = "Evidence-first research agent harness"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Launch the investigation console.
    Tui,
    /// Print local application paths without exposing credential contents.
    Paths,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    match cli.command.unwrap_or(Command::Tui) {
        Command::Tui => {
            let credentials = CredentialStore::load()?;
            tui::run(workspace_path()?, credentials).await
        }
        Command::Paths => {
            println!("workspace={}", workspace_path()?.display());
            println!("credentials={}", CredentialStore::path()?.display());
            Ok(())
        }
    }
}

fn workspace_path() -> Result<PathBuf> {
    let dirs = ProjectDirs::from("", "", "deep")
        .ok_or_else(|| anyhow!("unable to resolve the application data directory"))?;
    Ok(dirs.data_local_dir().join("workspace.db"))
}
