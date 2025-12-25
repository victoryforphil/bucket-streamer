# Task 02: Repo CLI Base

## Goal
Establish the repo-cli binary with Clap subcommand structure. Create pattern for adding future commands (convert, benchmark, etc.).

## Dependencies
- Task 01: Project Skeleton

## Files to Modify

```
crates/repo-cli/src/
├── main.rs              # CLI entry point with Clap
└── commands/
    ├── mod.rs           # Command exports
    └── convert.rs       # Placeholder subcommand
```

## Steps

### 1. Implement main.rs with Clap

```rust
use anyhow::Result;
use clap::{Parser, Subcommand};

mod commands;

#[derive(Parser)]
#[command(name = "repo-cli")]
#[command(about = "Development utilities for bucket-streamer")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Convert video files to H.265 MP4 format
    Convert(commands::convert::ConvertArgs),
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Convert(args) => commands::convert::run(args),
    }
}
```

### 2. Create commands/mod.rs

```rust
pub mod convert;
```

### 3. Create commands/convert.rs placeholder

```rust
use anyhow::Result;
use clap::Args;

#[derive(Args)]
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

    /// Output as JSON (for scripting)
    #[arg(long)]
    pub json: bool,
}

pub fn run(args: ConvertArgs) -> Result<()> {
    if args.json {
        println!(r#"{{"status": "not_implemented", "input": "{}"}}"#, args.input);
    } else {
        println!("Convert command not yet implemented");
        println!("  Input: {}", args.input);
        println!("  Output: {:?}", args.output);
        println!("  Extract offsets: {}", args.extract_offsets);
    }
    Ok(())
}
```

### 4. Add tracing initialization (optional but recommended)

Update main.rs:
```rust
use tracing_subscriber::EnvFilter;

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    // ...
}
```

## Success Criteria

- [ ] `cargo run -p repo-cli -- --help` shows subcommands
- [ ] `cargo run -p repo-cli -- convert --help` shows convert options
- [ ] `cargo run -p repo-cli -- convert -i test.mp4` prints placeholder message
- [ ] `cargo run -p repo-cli -- convert -i test.mp4 --json` outputs valid JSON
- [ ] Adding new subcommand requires only: new file in commands/, add to enum, add match arm

## Context

### Clap Derive Pattern
Using derive macros for ergonomic CLI definition. Each subcommand gets its own Args struct in a separate file for maintainability.

### --json Flag Convention
All commands that produce output should support `--json` for machine-readable output. This enables:
- Scripting and automation
- Integration with other tools
- Structured error reporting

### Future Subcommands (not in Stage 1)
- `benchmark` - Run performance tests
- `docker` - Docker management shortcuts  
- `tui` - Interactive terminal UI (Stage 2+)
