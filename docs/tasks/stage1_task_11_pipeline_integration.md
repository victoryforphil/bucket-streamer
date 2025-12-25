# Task 11: Pipeline Integration

## Goal
Wire all components together: WebSocket → Storage → Decoder → Encoder → WebSocket response. Implement session state with FIFO queue and proper error handling.

## Dependencies
- Task 05: Storage Backend
- Task 07: Axum Server
- Task 09: Decoder Module
- Task 10: JPEG Encoder

## Files to Modify

```
crates/bucket-streamer/src/pipeline/session.rs   # Session state
crates/bucket-streamer/src/pipeline/fetcher.rs   # Storage to decoder bridge
crates/bucket-streamer/src/server/router.rs      # Add storage to AppState
crates/bucket-streamer/src/server/websocket.rs   # Full message handling
crates/bucket-streamer/src/main.rs               # Wire up storage
```

## Steps

### 1. Implement pipeline/session.rs

```rust
use std::collections::VecDeque;
use bytes::Bytes;

use super::decoder::Decoder;
use super::encoder::JpegEncoder;
use crate::server::protocol::FrameRequest;

/// Per-session state for frame processing
pub struct Session {
    pub video_path: Option<String>,
    pub video_data: Option<Bytes>,
    pub decoder: Option<Decoder>,
    pub encoder: JpegEncoder,
    pub frame_queue: VecDeque<FrameRequest>,
}

impl Session {
    pub fn new(jpeg_quality: u8) -> anyhow::Result<Self> {
        Ok(Self {
            video_path: None,
            video_data: None,
            decoder: None,
            encoder: JpegEncoder::new(jpeg_quality)?,
            frame_queue: VecDeque::new(),
        })
    }

    /// Set video source, initializing decoder
    pub fn set_video(&mut self, path: String, data: Bytes) -> anyhow::Result<()> {
        let decoder = Decoder::new(&data)?;
        
        self.video_path = Some(path);
        self.video_data = Some(data);
        self.decoder = Some(decoder);
        self.frame_queue.clear();

        Ok(())
    }

    /// Queue frames for processing
    pub fn queue_frames(&mut self, frames: Vec<FrameRequest>) {
        self.frame_queue.extend(frames);
    }

    /// Process next frame in queue
    ///
    /// Returns (FrameRequest, JPEG bytes) or error
    pub fn process_next(&mut self, irap_offset: u64) -> Option<ProcessResult> {
        let request = self.frame_queue.pop_front()?;

        let result = self.process_frame(&request, irap_offset);
        Some(ProcessResult { request, result })
    }

    fn process_frame(
        &mut self,
        request: &FrameRequest,
        irap_offset: u64,
    ) -> anyhow::Result<Vec<u8>> {
        let decoder = self.decoder.as_mut()
            .ok_or_else(|| anyhow::anyhow!("No decoder initialized"))?;
        
        let video_data = self.video_data.as_ref()
            .ok_or_else(|| anyhow::anyhow!("No video data loaded"))?;

        // Decode frame at offset
        let frame = decoder.decode_at_offset(
            video_data,
            irap_offset,
            request.offset,
        )?;

        // Encode to JPEG
        let jpeg = self.encoder.encode(&frame)?;

        Ok(jpeg)
    }

    pub fn has_pending_frames(&self) -> bool {
        !self.frame_queue.is_empty()
    }

    pub fn clear_queue(&mut self) {
        self.frame_queue.clear();
    }
}

pub struct ProcessResult {
    pub request: FrameRequest,
    pub result: anyhow::Result<Vec<u8>>,
}
```

### 2. Implement pipeline/fetcher.rs

```rust
use anyhow::Result;
use bytes::Bytes;
use object_store::ObjectStore;
use std::sync::Arc;

/// Fetch video data from storage
pub async fn fetch_video(
    store: &Arc<dyn ObjectStore>,
    path: &str,
) -> Result<Bytes> {
    crate::storage::fetch_all(store.as_ref(), path).await
}

/// Check if video exists in storage
pub async fn video_exists(
    store: &Arc<dyn ObjectStore>,
    path: &str,
) -> Result<bool> {
    crate::storage::exists(store.as_ref(), path).await
}
```

### 3. Update server/router.rs

```rust
use axum::{
    Router,
    routing::get,
    response::IntoResponse,
    http::StatusCode,
};
use object_store::ObjectStore;
use std::sync::Arc;
use tower_http::trace::TraceLayer;

use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    pub store: Arc<dyn ObjectStore>,
}

pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/ws", get(super::websocket::ws_handler))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}
```

### 4. Update server/websocket.rs

```rust
use axum::{
    extract::{State, ws::{Message, WebSocket, WebSocketUpgrade}},
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use tracing::{info, warn, error, debug};

use super::protocol::{ClientMessage, ServerMessage};
use super::router::AppState;
use crate::pipeline::{fetcher, session::Session};

pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_session(socket, state))
}

async fn handle_session(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    info!("WebSocket client connected");

    let mut session = match Session::new(state.config.jpeg_quality) {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to create session: {}", e);
            return;
        }
    };

    let mut current_irap_offset: u64 = 0;

    while let Some(msg_result) = receiver.next().await {
        let msg = match msg_result {
            Ok(m) => m,
            Err(e) => {
                warn!("WebSocket error: {}", e);
                break;
            }
        };

        match msg {
            Message::Text(text) => {
                debug!("Received: {}", text);

                match ClientMessage::from_json(&text) {
                    Ok(client_msg) => {
                        match handle_message(
                            client_msg,
                            &mut session,
                            &mut current_irap_offset,
                            &state,
                            &mut sender,
                        ).await {
                            Ok(()) => {}
                            Err(e) => {
                                let error_msg = ServerMessage::Error {
                                    message: e.to_string(),
                                };
                                if sender.send(Message::Text(error_msg.to_json().into())).await.is_err() {
                                    break;
                                }
                            }
                        }
                    }
                    Err(e) => {
                        let error_msg = ServerMessage::Error {
                            message: format!("Invalid message: {}", e),
                        };
                        if sender.send(Message::Text(error_msg.to_json().into())).await.is_err() {
                            break;
                        }
                    }
                }
            }
            Message::Ping(data) => {
                if sender.send(Message::Pong(data)).await.is_err() {
                    break;
                }
            }
            Message::Pong(_) => {}
            Message::Close(_) => {
                info!("Client closed connection");
                break;
            }
            _ => {}
        }
    }

    info!("WebSocket client disconnected");
}

async fn handle_message(
    msg: ClientMessage,
    session: &mut Session,
    current_irap: &mut u64,
    state: &AppState,
    sender: &mut futures_util::stream::SplitSink<WebSocket, Message>,
) -> anyhow::Result<()> {
    match msg {
        ClientMessage::SetVideo { path } => {
            info!("Setting video: {}", path);

            // Check if video exists
            if !fetcher::video_exists(&state.store, &path).await? {
                let response = ServerMessage::VideoSet {
                    path: path.clone(),
                    ok: false,
                };
                sender.send(Message::Text(response.to_json().into())).await?;
                return Ok(());
            }

            // Fetch video data
            let data = fetcher::fetch_video(&state.store, &path).await?;

            // Initialize session with video
            session.set_video(path.clone(), data)?;

            let response = ServerMessage::VideoSet { path, ok: true };
            sender.send(Message::Text(response.to_json().into())).await?;
        }

        ClientMessage::RequestFrames { irap_offset, frames } => {
            if session.video_path.is_none() {
                anyhow::bail!("No video set. Send SetVideo first.");
            }

            *current_irap = irap_offset;
            session.queue_frames(frames);

            // Process all queued frames
            while let Some(result) = session.process_next(*current_irap) {
                match result.result {
                    Ok(jpeg_data) => {
                        // Send frame metadata
                        let frame_msg = ServerMessage::Frame {
                            index: result.request.index,
                            offset: result.request.offset,
                            size: jpeg_data.len() as u32,
                        };
                        sender.send(Message::Text(frame_msg.to_json().into())).await?;

                        // Send binary JPEG data
                        sender.send(Message::Binary(jpeg_data.into())).await?;
                    }
                    Err(e) => {
                        let error_msg = ServerMessage::FrameError {
                            index: result.request.index,
                            offset: result.request.offset,
                            error: e.to_string(),
                        };
                        sender.send(Message::Text(error_msg.to_json().into())).await?;
                    }
                }
            }
        }
    }

    Ok(())
}
```

### 5. Update main.rs

```rust
use anyhow::Result;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing_subscriber::EnvFilter;

mod config;
mod pipeline;
mod server;
mod storage;

use config::Config;
use server::router::{create_router, AppState};
use storage::{create_store, StorageConfig};

#[tokio::main]
async fn main() -> Result<()> {
    let config = Config::parse_args();

    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new(&config.log_level)),
        )
        .init();

    config.validate()?;

    // Create storage backend
    let storage_config = StorageConfig::from_config(&config);
    let store = create_store(&storage_config)?;

    let state = AppState {
        config: Arc::new(config.clone()),
        store,
    };

    let app = create_router(state);

    let listener = TcpListener::bind(&config.listen_addr).await?;
    tracing::info!("Listening on {}", config.listen_addr);

    axum::serve(listener, app).await?;

    Ok(())
}
```

## Success Criteria

- [ ] Server starts and creates storage backend
- [ ] SetVideo checks file existence, returns ok: false if missing
- [ ] SetVideo loads video and initializes decoder
- [ ] RequestFrames decodes and returns JPEG frames
- [ ] Each Frame message followed by binary JPEG data
- [ ] Failed frames return FrameError (don't crash session)
- [ ] Multiple frame requests work in sequence
- [ ] Tests pass: `cargo test -p bucket-streamer`

## Integration Test

```rust
#[tokio::test]
async fn test_end_to_end_frame_request() {
    // This test requires a running server and test video
    // Better suited for streaming-cli tests (Task 12)
}
```

Manual test with websocat:
```bash
# Terminal 1: Start server
cargo run -p bucket-streamer -- --local-path ./data

# Terminal 2: Connect and test
websocat ws://localhost:3000/ws
> {"type":"SetVideo","path":"test.h265.mp4"}
< {"type":"VideoSet","path":"test.h265.mp4","ok":true}
> {"type":"RequestFrames","irap_offset":0,"frames":[{"offset":1000,"index":0}]}
< {"type":"Frame","index":0,"offset":1000,"size":45230}
< [binary JPEG data]
```

## Context

### Frame Processing Flow

```
ClientMessage::RequestFrames
         │
         ▼
    session.queue_frames()
         │
         ▼
    session.process_next()
         │
    ┌────┴────┐
    ▼         ▼
 Decoder   Encoder
    │         │
    └────┬────┘
         ▼
 ServerMessage::Frame + Binary
```

### Error Isolation
Each frame is processed independently. A decode error on frame 5 shouldn't prevent frame 6 from being processed. The `FrameError` message tells the client which frame failed.

### FIFO vs LIFO
Stage 1 uses FIFO (first-in, first-out). For scrubbing optimization in Stage 2, switch to LIFO so the most recently requested frame is processed first.

### Memory Considerations
- Full video data is loaded into memory per session
- For large videos, consider streaming/chunked approach in Stage 2
- Decoder and encoder are reused to avoid allocation overhead
