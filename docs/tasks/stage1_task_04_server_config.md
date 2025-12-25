# Task 04: Server Configuration

## Goal
Implement `config.rs` for bucket-streamer with Clap CLI parsing, Serde serialization, and environment variable support.

## Dependencies
- Task 01: Project Skeleton

## Files to Modify

```
crates/bucket-streamer/src/config.rs    # Full implementation
crates/bucket-streamer/src/main.rs      # Wire up config parsing
```

## Steps

### 1. Implement config.rs

```rust
use clap::Parser;
use serde::{Deserialize, Serialize};

#[derive(Parser, Debug, Clone, Serialize, Deserialize)]
#[command(name = "bucket-streamer")]
#[command(about = "Video frame streaming server")]
#[command(version)]
pub struct Config {
    /// Server listen address
    #[arg(long, env = "LISTEN_ADDR", default_value = "0.0.0.0:3000")]
    pub listen_addr: String,

    /// Storage backend: "local" or "s3"
    #[arg(long, env = "STORAGE_BACKEND", default_value = "local")]
    pub storage_backend: String,

    /// Local storage path (when using local backend)
    #[arg(long, env = "LOCAL_PATH", default_value = "./data")]
    pub local_path: String,

    /// S3 bucket name (when using s3 backend)
    #[arg(long, env = "S3_BUCKET", default_value = "")]
    pub s3_bucket: String,

    /// S3 region (when using s3 backend)
    #[arg(long, env = "S3_REGION", default_value = "us-east-1")]
    pub s3_region: String,

    /// S3 endpoint URL (for MinIO or custom S3)
    #[arg(long, env = "S3_ENDPOINT")]
    pub s3_endpoint: Option<String>,

    /// JPEG encoding quality (1-100)
    #[arg(long, env = "JPEG_QUALITY", default_value = "80")]
    pub jpeg_quality: u8,

    /// Log level (trace, debug, info, warn, error)
    #[arg(long, env = "RUST_LOG", default_value = "info")]
    pub log_level: String,
}

impl Config {
    /// Parse from CLI args and environment
    pub fn parse_args() -> Self {
        Config::parse()
    }

    /// Validate configuration values
    pub fn validate(&self) -> Result<(), ConfigError> {
        if self.jpeg_quality == 0 || self.jpeg_quality > 100 {
            return Err(ConfigError::InvalidJpegQuality(self.jpeg_quality));
        }

        if self.storage_backend == "s3" && self.s3_bucket.is_empty() {
            return Err(ConfigError::MissingS3Bucket);
        }

        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("JPEG quality must be 1-100, got {0}")]
    InvalidJpegQuality(u8),

    #[error("S3 bucket name required when using s3 backend")]
    MissingS3Bucket,
}
```

### 2. Update main.rs to use config

```rust
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
        .with_env_filter(EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new(&config.log_level)))
        .init();

    config.validate()?;

    tracing::info!("Starting bucket-streamer");
    tracing::debug!(?config, "Configuration loaded");

    // Server startup will be added in Task 07
    println!("bucket-streamer listening on {}", config.listen_addr);

    Ok(())
}
```

### 3. Add unit tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config {
            listen_addr: "0.0.0.0:3000".to_string(),
            storage_backend: "local".to_string(),
            local_path: "./data".to_string(),
            s3_bucket: "".to_string(),
            s3_region: "us-east-1".to_string(),
            s3_endpoint: None,
            jpeg_quality: 80,
            log_level: "info".to_string(),
        };
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_invalid_jpeg_quality() {
        let config = Config {
            jpeg_quality: 0,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_s3_requires_bucket() {
        let config = Config {
            storage_backend: "s3".to_string(),
            s3_bucket: "".to_string(),
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }
}
```

For tests, implement Default:
```rust
impl Default for Config {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0:3000".to_string(),
            storage_backend: "local".to_string(),
            local_path: "./data".to_string(),
            s3_bucket: "".to_string(),
            s3_region: "us-east-1".to_string(),
            s3_endpoint: None,
            jpeg_quality: 80,
            log_level: "info".to_string(),
        }
    }
}
```

## Success Criteria

- [ ] `cargo run -p bucket-streamer -- --help` shows all options
- [ ] Default values work without any args
- [ ] Environment variables override defaults: `LISTEN_ADDR=:8080 cargo run -p bucket-streamer`
- [ ] CLI args override environment: `LISTEN_ADDR=:8080 cargo run -- --listen-addr :9000`
- [ ] Invalid jpeg_quality (0 or >100) returns error
- [ ] S3 backend without bucket returns error
- [ ] `cargo test -p bucket-streamer` passes

## Context

### Clap + Environment Variables
Using `#[arg(env = "...")]` enables automatic environment variable fallback. Priority: CLI arg > env var > default.

### Why Serde on Config?
- Future: load config from TOML/JSON file
- Debug: serialize config to logs
- Testing: easy construction in tests

### S3 Endpoint Option
The `s3_endpoint` field supports MinIO and other S3-compatible services:
```bash
bucket-streamer --storage-backend s3 \
  --s3-bucket videos \
  --s3-endpoint http://localhost:9000
```
