# Task 05: Storage Backend Abstraction

## Goal
Implement storage abstraction using `object_store` crate with support for local filesystem and S3 backends. Provide byte-range fetching utility.

## Dependencies
- Task 01: Project Skeleton

## Files to Modify

```
crates/bucket-streamer/src/storage/mod.rs       # Module exports
crates/bucket-streamer/src/storage/backend.rs   # Implementation
```

## Steps

### 1. Implement storage/mod.rs

```rust
pub mod backend;

pub use backend::{create_store, fetch_range, StorageConfig};
```

### 2. Implement storage/backend.rs

```rust
use anyhow::{Context, Result};
use bytes::Bytes;
use object_store::{local::LocalFileSystem, aws::AmazonS3Builder, ObjectStore, path::Path};
use std::sync::Arc;

/// Configuration for storage backend
#[derive(Debug, Clone)]
pub struct StorageConfig {
    pub backend: String,
    pub local_path: String,
    pub s3_bucket: String,
    pub s3_region: String,
    pub s3_endpoint: Option<String>,
}

impl StorageConfig {
    pub fn from_config(config: &crate::config::Config) -> Self {
        Self {
            backend: config.storage_backend.clone(),
            local_path: config.local_path.clone(),
            s3_bucket: config.s3_bucket.clone(),
            s3_region: config.s3_region.clone(),
            s3_endpoint: config.s3_endpoint.clone(),
        }
    }
}

/// Create an ObjectStore instance based on configuration
pub fn create_store(config: &StorageConfig) -> Result<Arc<dyn ObjectStore>> {
    match config.backend.as_str() {
        "local" => {
            let store = LocalFileSystem::new_with_prefix(&config.local_path)
                .context("Failed to create local filesystem store")?;
            Ok(Arc::new(store))
        }
        "s3" => {
            let mut builder = AmazonS3Builder::new()
                .with_bucket_name(&config.s3_bucket)
                .with_region(&config.s3_region);

            if let Some(endpoint) = &config.s3_endpoint {
                builder = builder.with_endpoint(endpoint);
                // For MinIO and other S3-compatible services
                builder = builder.with_allow_http(true);
            }

            let store = builder.build()
                .context("Failed to create S3 store")?;
            Ok(Arc::new(store))
        }
        other => anyhow::bail!("Unknown storage backend: {}", other),
    }
}

/// Fetch a byte range from storage
///
/// # Arguments
/// * `store` - The object store instance
/// * `path` - Path to the object (relative to store root)
/// * `start` - Start byte offset (inclusive)
/// * `end` - End byte offset (exclusive)
///
/// # Returns
/// The requested byte range as `Bytes`
pub async fn fetch_range(
    store: &dyn ObjectStore,
    path: &str,
    start: u64,
    end: u64,
) -> Result<Bytes> {
    let path = Path::from(path);
    let range = std::ops::Range {
        start: start as usize,
        end: end as usize,
    };

    let bytes = store
        .get_range(&path, range)
        .await
        .context("Failed to fetch byte range")?;

    Ok(bytes)
}

/// Fetch entire file from storage
pub async fn fetch_all(
    store: &dyn ObjectStore,
    path: &str,
) -> Result<Bytes> {
    let path = Path::from(path);
    let result = store
        .get(&path)
        .await
        .context("Failed to get object")?;

    let bytes = result
        .bytes()
        .await
        .context("Failed to read object bytes")?;

    Ok(bytes)
}

/// Check if object exists
pub async fn exists(
    store: &dyn ObjectStore,
    path: &str,
) -> Result<bool> {
    let path = Path::from(path);
    match store.head(&path).await {
        Ok(_) => Ok(true),
        Err(object_store::Error::NotFound { .. }) => Ok(false),
        Err(e) => Err(e).context("Failed to check object existence"),
    }
}

/// Get object metadata (size)
pub async fn get_size(
    store: &dyn ObjectStore,
    path: &str,
) -> Result<u64> {
    let path = Path::from(path);
    let meta = store
        .head(&path)
        .await
        .context("Failed to get object metadata")?;

    Ok(meta.size as u64)
}
```

### 3. Add integration tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    async fn setup_local_store() -> (Arc<dyn ObjectStore>, TempDir) {
        let temp = TempDir::new().unwrap();
        
        // Create test file
        let file_path = temp.path().join("test.bin");
        let mut file = std::fs::File::create(&file_path).unwrap();
        file.write_all(b"0123456789ABCDEF").unwrap();

        let config = StorageConfig {
            backend: "local".to_string(),
            local_path: temp.path().to_str().unwrap().to_string(),
            s3_bucket: String::new(),
            s3_region: String::new(),
            s3_endpoint: None,
        };

        let store = create_store(&config).unwrap();
        (store, temp)
    }

    #[tokio::test]
    async fn test_fetch_range() {
        let (store, _temp) = setup_local_store().await;
        
        let bytes = fetch_range(&*store, "test.bin", 0, 4).await.unwrap();
        assert_eq!(&bytes[..], b"0123");

        let bytes = fetch_range(&*store, "test.bin", 10, 16).await.unwrap();
        assert_eq!(&bytes[..], b"ABCDEF");
    }

    #[tokio::test]
    async fn test_fetch_all() {
        let (store, _temp) = setup_local_store().await;
        
        let bytes = fetch_all(&*store, "test.bin").await.unwrap();
        assert_eq!(&bytes[..], b"0123456789ABCDEF");
    }

    #[tokio::test]
    async fn test_exists() {
        let (store, _temp) = setup_local_store().await;
        
        assert!(exists(&*store, "test.bin").await.unwrap());
        assert!(!exists(&*store, "nonexistent.bin").await.unwrap());
    }

    #[tokio::test]
    async fn test_get_size() {
        let (store, _temp) = setup_local_store().await;
        
        let size = get_size(&*store, "test.bin").await.unwrap();
        assert_eq!(size, 16);
    }
}
```

### 4. Add tempfile dev dependency

In `crates/bucket-streamer/Cargo.toml`:
```toml
[dev-dependencies]
tempfile = "3"
```

## Success Criteria

- [ ] `create_store` returns LocalFileSystem for "local" backend
- [ ] `create_store` returns AmazonS3 for "s3" backend
- [ ] `fetch_range` returns correct byte slice
- [ ] `fetch_all` returns complete file contents
- [ ] `exists` correctly detects presence/absence
- [ ] `get_size` returns correct file size
- [ ] All tests pass: `cargo test -p bucket-streamer storage`

## Context

### object_store Crate
From the Apache Arrow project. Provides unified API for:
- Local filesystem
- AWS S3
- Azure Blob Storage
- Google Cloud Storage

Key benefit: `get_range()` maps directly to HTTP Range requests on S3.

### Byte Range Fetching
Critical for video streaming - we fetch only the bytes needed for specific frames:
```rust
// Fetch just the IRAP frame data
let irap_data = fetch_range(&store, "video.mp4", irap_offset, irap_offset + irap_size).await?;
```

### Path Handling
`object_store::path::Path` normalizes paths across backends. Don't use `std::path::Path` for storage paths.

### S3 Endpoint for MinIO
```rust
builder = builder.with_endpoint("http://localhost:9000");
builder = builder.with_allow_http(true);  // Required for non-HTTPS
```
