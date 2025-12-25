use anyhow::{Context, Result};
use bytes::Bytes;
use object_store::{aws::AmazonS3Builder, local::LocalFileSystem, path::Path, ObjectStore};
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

            let store = builder.build().context("Failed to create S3 store")?;
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
    let range = (start as usize)..(end as usize);

    let bytes = store
        .get_range(&path, range)
        .await
        .context("Failed to fetch byte range")?;

    Ok(bytes)
}

/// Fetch entire file from storage
pub async fn fetch_all(store: &dyn ObjectStore, path: &str) -> Result<Bytes> {
    let path = Path::from(path);
    let result = store.get(&path).await.context("Failed to get object")?;

    let bytes = result
        .bytes()
        .await
        .context("Failed to read object bytes")?;

    Ok(bytes)
}

/// Check if object exists
pub async fn exists(store: &dyn ObjectStore, path: &str) -> Result<bool> {
    let path = Path::from(path);
    match store.head(&path).await {
        Ok(_) => Ok(true),
        Err(object_store::Error::NotFound { .. }) => Ok(false),
        Err(e) => Err(e).context("Failed to check object existence"),
    }
}

/// Get object metadata (size)
pub async fn get_size(store: &dyn ObjectStore, path: &str) -> Result<u64> {
    let path = Path::from(path);
    let meta = store
        .head(&path)
        .await
        .context("Failed to get object metadata")?;

    Ok(meta.size as u64)
}

#[cfg(test)]
mod tests {
    use super::*;
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
