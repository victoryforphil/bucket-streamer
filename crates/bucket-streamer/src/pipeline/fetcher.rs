use anyhow::Result;
use bytes::Bytes;
use object_store::ObjectStore;
use std::sync::Arc;

/// Fetch entire video from storage
pub async fn fetch_video(store: &Arc<dyn ObjectStore>, path: &str) -> Result<Bytes> {
    crate::storage::fetch_all(store.as_ref(), path).await
}

/// Check if video exists in storage
pub async fn video_exists(store: &Arc<dyn ObjectStore>, path: &str) -> Result<bool> {
    crate::storage::exists(store.as_ref(), path).await
}
