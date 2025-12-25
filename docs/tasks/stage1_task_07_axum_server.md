# Task 07: Axum Server & WebSocket Handler

## Goal
Set up Axum HTTP server with `/health` and `/ws` routes. Implement WebSocket upgrade and basic session loop that logs messages and handles protocol types from Task 06.

## Dependencies
- Task 04: Server Config (complete)
- Task 06: WebSocket Protocol Types (complete)

## Files to Modify

```
crates/bucket-streamer/Cargo.toml              # Add futures-util
Cargo.toml                                      # Add futures-util to workspace
crates/bucket-streamer/src/server/mod.rs       # Export router types
crates/bucket-streamer/src/server/router.rs    # Axum router setup
crates/bucket-streamer/src/server/websocket.rs # WS handler and session loop
crates/bucket-streamer/src/main.rs             # Start server with tokio
```

## Steps

### 1. Add futures-util dependency

In workspace `Cargo.toml`, add to `[workspace.dependencies]`:
```toml
futures-util = "0.3"
```

In `crates/bucket-streamer/Cargo.toml`, add to `[dependencies]`:
```toml
futures-util.workspace = true
```

### 2. Update server/mod.rs

```rust
pub mod protocol;
pub mod router;
pub mod websocket;

pub use protocol::{ClientMessage, FrameRequest, ServerMessage};
pub use router::{create_router, AppState};
```

### 3. Implement server/router.rs

```rust
use std::sync::Arc;

use axum::{http::StatusCode, response::IntoResponse, routing::get, Router};
use tower_http::trace::TraceLayer;

use crate::config::Config;

/// Application state shared across handlers
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    // Storage and pipeline components added in Task 11
}

/// Create the Axum router with all routes
pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/ws", get(super::websocket::ws_handler))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Health check endpoint
async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, "ok")
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    #[tokio::test]
    async fn test_health_check() {
        let state = AppState {
            config: Arc::new(Config::default()),
        };
        let app = create_router(state);

        let response = app
            .oneshot(Request::get("/health").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(response.status(), StatusCode::OK);
    }
}
```

### 4. Implement server/websocket.rs

```rust
use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use tracing::{debug, error, info, warn};

use super::protocol::{ClientMessage, ServerMessage};
use super::router::AppState;

/// WebSocket upgrade handler
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    State(state): State<AppState>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_session(socket, state))
}

/// Handle a WebSocket session
async fn handle_session(socket: WebSocket, _state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    info!("WebSocket client connected");

    // Session state (expanded in Task 11)
    let mut video_path: Option<String> = None;

    while let Some(msg_result) = receiver.next().await {
        let msg = match msg_result {
            Ok(m) => m,
            Err(e) => {
                warn!("WebSocket receive error: {}", e);
                break;
            }
        };

        match msg {
            Message::Text(text) => {
                debug!("Received: {}", text);

                match ClientMessage::from_json(&text) {
                    Ok(client_msg) => {
                        let response = handle_message(client_msg, &mut video_path).await;
                        let json = response.to_json();

                        if sender.send(Message::text(json)).await.is_err() {
                            error!("Failed to send response");
                            break;
                        }
                    }
                    Err(e) => {
                        let error_msg = ServerMessage::Error {
                            message: format!("Invalid message: {}", e),
                        };
                        if sender.send(Message::text(error_msg.to_json())).await.is_err() {
                            break;
                        }
                    }
                }
            }
            Message::Binary(_) => {
                warn!("Unexpected binary message from client");
            }
            Message::Ping(_) => {
                // Axum automatically responds to pings with pongs
            }
            Message::Pong(_) => {
                // Pong responses, no action needed
            }
            Message::Close(_) => {
                info!("Client initiated close");
                break;
            }
        }
    }

    info!("WebSocket client disconnected");
}

/// Handle a parsed client message
async fn handle_message(msg: ClientMessage, video_path: &mut Option<String>) -> ServerMessage {
    match msg {
        ClientMessage::SetVideo { path } => {
            info!("Setting video: {}", path);
            *video_path = Some(path.clone());
            // Video validation added in Task 11
            ServerMessage::VideoSet { path, ok: true }
        }
        ClientMessage::RequestFrames { irap_offset, frames } => {
            if video_path.is_none() {
                return ServerMessage::Error {
                    message: "No video set. Send SetVideo first.".to_string(),
                };
            }

            info!(
                "Frame request: irap_offset={}, frame_count={}",
                irap_offset,
                frames.len()
            );

            // Frame decoding implemented in Task 11
            ServerMessage::FrameError {
                index: frames.first().map(|f| f.index).unwrap_or(0),
                offset: irap_offset,
                error: "not_implemented".to_string(),
            }
        }
    }
}
```

### 5. Update main.rs

```rust
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
```

## Success Criteria

- [ ] `cargo build -p bucket-streamer` compiles without errors
- [ ] `cargo test -p bucket-streamer` passes (including health check test)
- [ ] `cargo run -p bucket-streamer` starts server on port 3000
- [ ] `curl http://localhost:3000/health` returns "ok" with 200 status
- [ ] WebSocket connects at `ws://localhost:3000/ws` (manual test with websocat or browser)
- [ ] Server logs "WebSocket client connected/disconnected" on connect/close

## Manual WebSocket Testing

Use `websocat` or similar tool:
```bash
# Install websocat if needed: cargo install websocat
websocat ws://localhost:3000/ws

# Send SetVideo message:
{"type":"SetVideo","path":"test.mp4"}
# Expected response: {"type":"VideoSet","path":"test.mp4","ok":true}

# Send RequestFrames without SetVideo:
{"type":"RequestFrames","irap_offset":0,"frames":[{"offset":100,"index":0}]}
# Expected response: {"type":"Error","message":"No video set. Send SetVideo first."}
```

## Context

### Axum 0.7 WebSocket Pattern
Axum 0.7 uses `WebSocketUpgrade` extractor. The `on_upgrade` callback runs as a spawned task after the HTTP upgrade completes:
```rust
pub async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_session)
}
```

### futures-util for Stream/Sink
`socket.split()` returns types that implement `Stream` and `Sink`. Import traits from futures-util:
- `StreamExt::next()` - receive next message
- `SinkExt::send()` - send message

### Message Type
In Axum 0.7.9+, `Message::Text(Utf8Bytes)` contains `Utf8Bytes`, not `String`. Use `Message::text(s)` helper for sending text, which accepts any stringable type. When receiving, `Utf8Bytes` can be used like a string via `Deref<Target=str>`.

### State Sharing
`AppState` is cloned into each handler. Use `Arc<T>` for expensive-to-clone data:
```rust
pub struct AppState {
    pub config: Arc<Config>,
    // Future: Arc<dyn ObjectStore>, pipeline channels, etc.
}
```

### Ping/Pong Handling
Axum's WebSocket implementation automatically responds to Ping frames with Pong frames. Explicit handling is optional.
