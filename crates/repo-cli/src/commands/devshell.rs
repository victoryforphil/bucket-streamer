use anyhow::Result;
use clap::Args;
use std::process::Command;

#[derive(Args, Debug)]
pub struct DevshellArgs {
    /// Command or keyword to execute (build, test, fmt, check, clippy, clean)
    /// or use '--' to pass through custom command
    #[arg(trailing_var_arg = true, allow_hyphen_values = true)]
    pub args: Vec<String>,
}

/// Maps keywords to full commands
fn map_keyword(cmd: &str) -> String {
    match cmd {
        "build" => "docker-compose build".to_string(),
        other => format!("cargo {}", other),
    }
}

pub async fn run(_global: &crate::GlobalOpts, args: DevshellArgs) -> Result<()> {
    if args.args.is_empty() {
        eprintln!("Usage: repo-cli devshell [KEYWORD|--] [ARGS...]");
        eprintln!("  Keywords: build, test, fmt, check, clippy, clean");
        eprintln!("  Example: repo-cli devshell build");
        eprintln!("  Example: repo-cli devshell -- cargo build --release");
        return Ok(());
    }

    let mut cmd_parts = args.args;

    // Check if first arg is '--' for custom command
    let full_command = if cmd_parts[0] == "--" {
        // Remove the '--' and join rest as raw command
        cmd_parts.remove(0);
        cmd_parts.join(" ")
    } else {
        // First arg is keyword, map it and append remaining args
        let keyword = cmd_parts.remove(0);
        let base = map_keyword(&keyword);
        if cmd_parts.is_empty() {
            base
        } else {
            format!("{} {}", base, cmd_parts.join(" "))
        }
    };

    // Execute via docker-compose exec
    let status = Command::new("docker-compose")
        .arg("exec")
        .arg("-T")
        .arg("bucket-streamer")
        .arg("sh")
        .arg("-c")
        .arg(&full_command)
        .status()?;

    // Exit with propagated exit code
    std::process::exit(status.code().unwrap_or(1));
}
