# Task 02: Repo CLI Base

## Goal
Establish the repo-cli binary with Clap subcommand structure. Create pattern for adding future commands (convert, benchmark, etc.) with async execution, global options, JSON output, error handling, and progress bar support.

## Dependencies
- Task 01: Project Skeleton

## Prerequisites

### Check and Add Dependencies

Verify all required dependencies are present in the workspace:

```bash
# Check workspace dependencies
cat Cargo.toml | grep -A 20 "\[workspace.dependencies\]"

# Expected to see:
#   indicatif = "0.17"
#   anyhow = "1"
#   thiserror = "2"
#   serde = { version = "1", features = ["derive"] }
#   serde_json = "1"
#   clap = { version = "4", features = ["derive"] }
#   tokio = { version = "1", features = ["full"] }
#   tracing = "0.1"
#   tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# If indicatif is missing, add it:
# cargo edit -p repo-cli --add indicatif

# Verify repo-cli dependencies
cat crates/repo-cli/Cargo.toml | grep -A 15 "\[dependencies\]"
```

## Files to Create/Modify

```
crates/repo-cli/src/
├── main.rs                 # CLI entry point with async/ExitCode
├── error.rs                # NEW: CliError enum with thiserror
└── commands/
    ├── mod.rs              # Command exports
    ├── convert.rs          # Convert subcommand (with validation, progress, JSON)
    └── output.rs           # NEW: JSON output structs
```

## Steps

### 1. Create CLI Error Types (`src/error.rs`)

Define domain-specific error types using `thiserror` for user-facing errors:

```rust
use thiserror::Error;

#[derive(Error, Debug)]
pub enum CliError {
    #[error("File not found: {0}")]
    FileNotFound(String),

    #[error("Invalid file extension: {0} (expected .mp4 or .mov)")]
    InvalidExtension(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Internal error: {0}")]
    Internal(String),
}
```

### 2. Create JSON Output Types (`src/commands/output.rs`)

Define strictly typed JSON output schema with `serde`:

```rust
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug)]
#[serde(tag = "status")]
pub enum CommandOutput {
    Success { data: serde_json::Value },
    Error { error: String, context: Option<String> },
}

impl CommandOutput {
    pub fn success(data: serde_json::Value) -> Self {
        Self::Success { data }
    }

    pub fn error(error: String, context: Option<String>) -> Self {
        Self::Error { error, context }
    }
}
```

### 3. Implement main.rs with Async + ExitCode + Global Options

```rust
use anyhow::{Context, Result};
use clap::{Args, Parser, Subcommand};
use std::process::ExitCode;
use tracing_subscriber::EnvFilter;

mod commands;
mod error;

use error::CliError;

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

#[derive(Args, Debug)]
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
}

fn main() -> ExitCode {
    if let Err(e) = run() {
        eprintln!("Error: {e:#}");
        return ExitCode::from(1);
    }
    ExitCode::SUCCESS
}

async fn run() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Convert(args) => commands::convert::run(&cli.global, args).await,
    }
}
```

### 4. Update commands/mod.rs

```rust
pub mod convert;
pub mod output;
```

### 5. Implement Convert Command with Validation, Progress, and JSON

Create `src/commands/convert.rs`:

```rust
use anyhow::{Context, Result};
use clap::Args;
use indicatif::{ProgressBar, ProgressStyle};
use serde_json::json;
use std::path::Path;

use super::output::CommandOutput;
use crate::error::CliError;

#[derive(Args, Debug)]
pub struct ConvertArgs {
    /// Input video file path
    #[arg(short, long)]
    pub input: String,

    /// Output file path (default: input with .mp4 extension)
    #[arg(short, long)]
    pub output: Option<String>,

    /// Extract frame byte offsets to JSON sidecar
    #[arg(long)]
    pub extract_offsets: bool,
}

/// Validates that the input file exists and has a valid extension
fn validate_input(path: &str) -> Result<()> {
    let p = Path::new(path);

    if !p.exists() {
        return Err(CliError::FileNotFound(path.to_string()).into());
    }

    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .ok_or_else(|| CliError::InvalidInput(format!("No extension found: {}", path)))?;

    if !matches!(ext.to_lowercase().as_str(), "mp4" | "mov") {
        return Err(CliError::InvalidExtension(path.to_string()).into());
    }

    Ok(())
}

/// Determines output path based on input if not specified
fn determine_output(input: &str) -> String {
    Path::new(input)
        .with_extension("mp4")
        .to_string_lossy()
        .to_string()
}

/// Creates a progress bar based on global options
fn create_progress_bar(global: &crate::GlobalOpts, total: u64) -> Option<ProgressBar> {
    if global.no_progress {
        return None;
    }

    let pb = ProgressBar::new(total);
    pb.set_style(
        ProgressStyle::with_template(
            "[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} {msg}",
        )
        .unwrap()
        .progress_chars("##-"),
    );
    Some(pb)
}

pub async fn run(global: &crate::GlobalOpts, args: ConvertArgs) -> Result<()> {
    // Validate input file
    validate_input(&args.input).context("Input validation failed")?;

    // Determine output path
    let output = args.output.unwrap_or_else(|| determine_output(&args.input));

    // Simulate conversion work (to be replaced with actual FFmpeg conversion in Task 03)
    let pb = create_progress_bar(global, 100);

    for i in 0..100 {
        tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;
        if let Some(ref pb) = pb {
            pb.set_position(i + 1);
            pb.set_message(format!("Processing {}/100", i + 1));
        }
    }

    if let Some(pb) = pb {
        pb.finish_with_message("Conversion completed");
    }

    // Generate output
    let data = json!({
        "input": args.input,
        "output": output,
        "extract_offsets": args.extract_offsets,
        "status": "not_implemented"
    });

    if global.json {
        let output = CommandOutput::success(data);
        println!("{}", serde_json::to_string_pretty(&output)?);
    } else {
        println!("Convert command not yet implemented");
        println!("  Input: {}", args.input);
        println!("  Output: {}", output);
        println!("  Extract offsets: {}", args.extract_offsets);
    }

    Ok(())
}
```

## Key Patterns Established

### 1. Global Options Pattern
- Use `#[command(flatten)]` to apply options to all subcommands
- Use `global = true` to ensure availability across commands
- Options: `--json` (output format), `--verbose` (logging), `--no-progress` (UI control)

### 2. Error Handling Pattern
- **User-facing errors**: Define `CliError` enum with `thiserror` (clear, actionable messages)
- **Internal errors**: Use `anyhow::Error` for wrapping library errors with `.context()`
- **Error propagation**: Use `?` operator throughout, convert at CLI boundary

### 3. JSON Output Pattern
- All commands support `--json` flag for machine-readable output
- Use `#[serde(tag = "status")]` for discriminated union pattern
- Standard schema: `{status: "success|error", data: {...}|error: {...}}`
- Strictly typed with `#[derive(Serialize)]` for maintainability

### 4. Async Entry Point Pattern
- Use `fn main() -> ExitCode` for proper exit codes
- Use `async fn run() -> Result<()>` for async logic
- Tracing initialization at top of `run()`
- Wrap tokio runtime for all async operations

### 5. Progress Bar Pattern
- Check `--no-progress` flag before creating bars
- Use `indicatif` with `Option<ProgressBar>` for conditional display
- Update position and message in async loops
- Finish with completion message

### 6. File Validation Pattern
- Basic validation: file existence (`Path::exists()`)
- Extension validation: check for supported formats (`.mp4`, `.mov`)
- Return structured `CliError` for user-friendly messages

## Success Criteria

- [ ] Workspace Cargo.toml contains `indicatif = "0.17"` in `[workspace.dependencies]`
- [ ] repo-cli Cargo.toml references `indicatif.workspace = true`
- [ ] `cargo run -p repo-cli -- --help` shows global flags and subcommands
- [ ] `cargo run -p repo-cli -- convert --help` shows convert options
- [ ] `cargo run -p repo-cli -- convert -i test.mp4` prints placeholder with progress bar
- [ ] `cargo run -p repo-cli -- convert -i test.mp4 --json` outputs valid JSON:
  ```json
  {
    "status": "success",
    "data": {
      "input": "test.mp4",
      "output": "test.mp4",
      "extract_offsets": false,
      "status": "not_implemented"
    }
  }
  ```
- [ ] `cargo run -p repo-cli -- convert -i nonexistent.mp4` returns exit code 1 and prints:
  ```
  Error: File not found: nonexistent.mp4
  ```
- [ ] `cargo run -p repo-cli -- convert -i test.txt` returns error with extension validation
- [ ] `cargo run -p repo-cli -- convert -i test.mp4 --no-progress` suppresses progress bar
- [ ] `RUST_LOG=debug cargo run -p repo-cli -- convert -i test.mp4` shows debug logs
- [ ] Adding new subcommand requires only:
  1. New file in `commands/`
  2. Add variant to `Commands` enum
  3. Add match arm in `run()`

## Context

### Why Global Options
Global options ensure consistency across all commands and reduce repetition. The `#[command(flatten)]` pattern is idiomatic in Clap 4.x and automatically propagates options to all subcommands.

### Why anyhow + thiserror Together
- **thiserror**: For library boundaries and custom error types that need to be handled specifically
- **anyhow**: For application-level error handling with context enrichment
- **Pattern**: Libraries expose `thiserror` types, CLI wraps them with `anyhow` for reporting

### Why Async from Start
While Task 02 is a placeholder, subsequent tasks (FFmpeg conversion, S3 operations) will be async. Establishing the async pattern now prevents refactoring later.

### Why Progress Bars Foundation
Long-running operations (video conversion, frame extraction, benchmarking) benefit from visual feedback. The conditional pattern respects both interactive and non-interactive (CI/scripting) environments.

### Why Strict JSON Schema
Typed JSON output enables:
- Documentation generation from code
- Language-independent client code generation
- Compile-time validation of output structure
- Easy evolution with backward compatibility

## Future Subcommands (not in Stage 1)
- `benchmark` - Run performance tests
- `docker` - Docker management shortcuts
- `tui` - Interactive terminal UI (Stage 2+)

All future subcommands will inherit the patterns established here.
