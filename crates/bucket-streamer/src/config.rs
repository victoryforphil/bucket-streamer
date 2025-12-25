use clap::{Parser, ValueEnum};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "lowercase")]
pub enum StorageBackend {
    Local,
    S3,
}

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
    pub storage_backend: StorageBackend,

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

    /// S3 access key (for MinIO or explicit credentials)
    #[arg(long, env = "S3_ACCESS_KEY", default_value = "minioadmin")]
    pub s3_access_key: String,

    /// S3 secret key (for MinIO or explicit credentials)
    #[arg(long, env = "S3_SECRET_KEY", default_value = "minioadmin")]
    pub s3_secret_key: String,

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
        if self.storage_backend == StorageBackend::S3 && self.s3_bucket.is_empty() {
            return Err(ConfigError::MissingS3Bucket);
        }

        Ok(())
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_addr: "0.0.0.0:3000".to_string(),
            storage_backend: StorageBackend::Local,
            local_path: "./data".to_string(),
            s3_bucket: "".to_string(),
            s3_region: "us-east-1".to_string(),
            s3_endpoint: None,
            s3_access_key: "minioadmin".to_string(),
            s3_secret_key: "minioadmin".to_string(),
            jpeg_quality: 80,
            log_level: "info".to_string(),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    #[error("S3 bucket name required when using s3 backend")]
    MissingS3Bucket,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_s3_requires_bucket() {
        let mut config = Config::default();
        config.storage_backend = StorageBackend::S3;
        config.s3_bucket = "".to_string();
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_s3_with_bucket_valid() {
        let mut config = Config::default();
        config.storage_backend = StorageBackend::S3;
        config.s3_bucket = "my-bucket".to_string();
        assert!(config.validate().is_ok());
    }
}
