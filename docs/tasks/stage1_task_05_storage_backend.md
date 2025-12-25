# Task 05: Storage Backend Abstraction

## Goal
Implement storage abstraction using `object_store` crate with support for local filesystem and S3 backends. Provide byte-range fetching utility.

## Dependencies
- Task 01: Project Skeleton
- Task 04: Server Config (uses `Config` struct directly)

## Files to Modify

```
Cargo.toml (workspace)                          # Add 'fs' feature to object_store
crates/bucket-streamer/src/storage/mod.rs       # Module exports
crates/bucket-streamer/src/storage/backend.rs   # Implementation
```

## Steps

### 1. Add `fs` feature to object_store in workspace Cargo.toml

```toml
object_store = { version = "0.11", features = ["aws", "fs"] }
```

The `fs` feature is required for `LocalFileSystem` support.

### 2. Implement storage/mod.rs

```rust
pub mod backend;

pub use backend::{create_store, fetch_range, fetch_all, exists, get_size};
```

### 3. Implement storage/backend.rs

```rust
use anyhow::{Context, Result};
use bytes::Bytes;
use object_store::{local::LocalFileSystem, aws::AmazonS3Builder, ObjectStore, path::Path};
use std::sync::Arc;

use crate::config::{Config, StorageBackend};

/// Create an ObjectStore instance based on configuration
pub fn create_store(config: &Config) -> Result<Arc<dyn ObjectStore>> {
    match config.storage_backend {
        StorageBackend::Local => {
            let store = LocalFileSystem::new_with_prefix(&config.local_path)
                .context("Failed to create local filesystem store")?;
            Ok(Arc::new(store))
        }
        StorageBackend::S3 => {
            let mut builder = AmazonS3Builder::new()
                .with_bucket_name(&config.s3_bucket)
                .with_region(&config.s3_region)
                .with_access_key_id(&config.s3_access_key)
                .with_secret_access_key(&config.s3_secret_key);

            if let Some(endpoint) = &config.s3_endpoint {
                builder = builder.with_endpoint(endpoint);
                // For MinIO and other S3-compatible services
                builder = builder.with_allow_http(true);
            }

            let store = builder.build()
                .context("Failed to create S3 store")?;
            Ok(Arc::new(store))
        }
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
    let range = start..end;

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

### 4. Add helper functions (continued in backend.rs)

```rust
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

### 5. Add integration tests

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{Config, StorageBackend};
    use std::io::Write;
    use tempfile::TempDir;

    fn create_test_config(temp_path: &std::path::Path) -> Config {
        Config {
            storage_backend: StorageBackend::Local,
            local_path: temp_path.to_str().unwrap().to_string(),
            ..Config::default()
        }
    }

    async fn setup_local_store() -> (Arc<dyn ObjectStore>, TempDir) {
        let temp = TempDir::new().unwrap();
        
        // Create test file
        let file_path = temp.path().join("test.bin");
        let mut file = std::fs::File::create(&file_path).unwrap();
        file.write_all(b"0123456789ABCDEF").unwrap();

        let config = create_test_config(temp.path());
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

### 6. Add tempfile dev dependency

In `crates/bucket-streamer/Cargo.toml`:
```toml
[dev-dependencies]
tempfile = "3"
```

## Success Criteria

- [ ] `fs` feature added to object_store in workspace Cargo.toml
- [ ] `create_store` returns LocalFileSystem for `StorageBackend::Local`
- [ ] `create_store` returns AmazonS3 for `StorageBackend::S3`
- [ ] S3 builder includes access key and secret key from config
- [ ] `fetch_range` returns correct byte slice
- [ ] `fetch_all` returns complete file contents
- [ ] `exists` correctly detects presence/absence
- [ ] `get_size` returns correct file size
- [ ] All tests pass: `cargo test -p bucket-streamer storage`

## Context

### object_store Crate
From the Apache Arrow project. Provides unified API for:
- Local filesystem (requires `fs` feature)
- AWS S3 (requires `aws` feature)
- Azure Blob Storage
- Google Cloud Storage

Key benefit: `get_range()` maps directly to HTTP Range requests on S3.

### API Notes
- `get_range()` takes `Range<u64>`, not `Range<usize>`
- `LocalFileSystem` requires the `fs` feature flag
- `AmazonS3Builder` supports `from_env()` for automatic credential discovery
- Version 0.11.x is stable; avoid 0.13.0 which has breaking changes

### Byte Range Fetching
Critical for video streaming - we fetch only the bytes needed for specific frames:
```rust
// Fetch just the IRAP frame data
let irap_data = fetch_range(&store, "video.mp4", irap_offset, irap_offset + irap_size).await?;
```

### Path Handling
`object_store::path::Path` normalizes paths across backends. Don't use `std::path::Path` for storage paths.

### S3 Credentials
The `AmazonS3Builder` requires explicit credentials for MinIO/custom endpoints:
```rust
builder = builder
    .with_access_key_id(&config.s3_access_key)
    .with_secret_access_key(&config.s3_secret_key);
```

For AWS, you can also use `AmazonS3Builder::from_env()` which reads from:
- `AWS_ACCESS_KEY_ID`
- `AWS_SECRET_ACCESS_KEY`
- `AWS_DEFAULT_REGION`

### S3 Endpoint for MinIO
```rust
builder = builder.with_endpoint("http://localhost:9000");
builder = builder.with_allow_http(true);  // Required for non-HTTPS
```
