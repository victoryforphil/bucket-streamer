use std::sync::Arc;

use anyhow::Result;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

mod config;
mod pipeline;
mod server;
mod storage;

use config::Config;
use server::{create_router, AppState};

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::parse_args();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.log_level)),
        )
        .init();

    config.validate()?;

    tracing::info!("Starting bucket-streamer");
    tracing::debug!(?config, "Configuration loaded");

    let state = AppState {
        config: Arc::new(config.clone()),
    };

    let app = create_router(state);

    let listener = TcpListener::bind(&config.listen_addr).await?;
    tracing::info!("Listening on {}", config.listen_addr);

    axum::serve(listener, app).await?;

    Ok(())
}
