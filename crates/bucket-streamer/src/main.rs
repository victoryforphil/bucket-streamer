use anyhow::Result;
use tracing_subscriber::EnvFilter;

mod config;
mod pipeline;
mod server;
mod storage;

use config::Config;

fn main() -> Result<()> {
    let config = Config::parse_args();

    // Initialize tracing with configured level
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.log_level)),
        )
        .init();

    config.validate()?;

    tracing::info!("Starting bucket-streamer");
    tracing::debug!(?config, "Configuration loaded");

    // Server startup will be added in Task 07
    println!("bucket-streamer listening on {}", config.listen_addr);

    Ok(())
}
