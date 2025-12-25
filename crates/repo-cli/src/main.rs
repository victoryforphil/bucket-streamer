use anyhow::Result;
use clap::{Args, Parser, Subcommand};
use std::process::ExitCode;
use tracing_subscriber::EnvFilter;

mod commands;
mod error;

#[derive(Parser, Debug)]
#[command(name = "repo-cli")]
#[command(about = "Development utilities for bucket-streamer")]
#[command(version)]
struct Cli {
    #[command(flatten)]
    global: GlobalOpts,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Args, Debug, Clone)]
struct GlobalOpts {
    /// JSON output format (machine-readable)
    #[arg(long, global = true)]
    json: bool,

    /// Verbosity level (-v for info, -vv for debug)
    #[arg(short, long, global = true, action = clap::ArgAction::Count)]
    verbose: u8,

    /// Disable progress bar output (useful for scripts/CI)
    #[arg(long, global = true)]
    no_progress: bool,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Convert video files to H.265 MP4 format
    Convert(commands::convert::ConvertArgs),
    /// Execute commands in Docker container
    Devshell(commands::devshell::DevshellArgs),
}

fn main() -> ExitCode {
    if let Err(e) = run() {
        eprintln!("Error: {e:#}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

#[tokio::main]
async fn run() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Convert(args) => commands::convert::run(&cli.global, args).await,
        Commands::Devshell(args) => commands::devshell::run(&cli.global, args).await,
    }
}
