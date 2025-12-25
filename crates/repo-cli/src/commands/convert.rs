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
