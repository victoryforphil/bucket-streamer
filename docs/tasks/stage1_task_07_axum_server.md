# Task 07: Axum Server & WebSocket Handler

## Goal
Set up Axum HTTP server with `/health` and `/ws` routes. Implement WebSocket upgrade and basic session loop that logs messages.

## Dependencies
- Task 04: Server Config
- Task 06: WebSocket Protocol Types

## Files to Modify

```
crates/bucket-streamer/src/server/router.rs     # Axum router setup
crates/bucket-streamer/src/server/websocket.rs  # WS handler and session loop
crates/bucket-streamer/src/main.rs              # Start server
```

## Steps

### 1. Implement server/router.rs

```rust
use axum::{
    Router,
    routing::get,
    response::IntoResponse,
    http::StatusCode,
};
use std::sync::Arc;
use tower_http::trace::TraceLayer;

use crate::config::Config;

/// Application state shared across handlers
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<Config>,
    // Storage and other shared state added in Task 11
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
```

### 2. Implement server/websocket.rs

```rust
use axum::{
    extract::{
        State,
        ws::{Message, WebSocket, WebSocketUpgrade},
    },
    response::IntoResponse,
};
use futures_util::{SinkExt, StreamExt};
use tracing::{info, warn, error, debug};

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
async fn handle_session(socket: WebSocket, state: AppState) {
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
                        let response = handle_message(client_msg, &mut video_path, &state).await;
                        let json = response.to_json();

                        if sender.send(Message::Text(json.into())).await.is_err() {
                            error!("Failed to send response");
                            break;
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
            Message::Binary(_) => {
                // Binary messages from client not expected in protocol
                warn!("Unexpected binary message from client");
            }
            Message::Ping(data) => {
                if sender.send(Message::Pong(data)).await.is_err() {
                    break;
                }
            }
            Message::Pong(_) => {}
            Message::Close(_) => {
                info!("Client initiated close");
                break;
            }
        }
    }

    info!("WebSocket client disconnected");
}

/// Handle a parsed client message
async fn handle_message(
    msg: ClientMessage,
    video_path: &mut Option<String>,
    _state: &AppState,
) -> ServerMessage {
    match msg {
        ClientMessage::SetVideo { path } => {
            info!("Setting video: {}", path);
            *video_path = Some(path.clone());
            // TODO: Verify video exists (Task 11)
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

            // TODO: Actually decode frames (Task 11)
            // For now, return stub error
            ServerMessage::FrameError {
                index: frames.first().map(|f| f.index).unwrap_or(0),
                offset: irap_offset,
                error: "not_implemented".to_string(),
            }
        }
    }
}
```

### 3. Add futures-util dependency

In workspace Cargo.toml:
```toml
[workspace.dependencies]
futures-util = "0.3"
```

In bucket-streamer Cargo.toml:
```toml
[dependencies]
futures-util.workspace = true
```

### 4. Update main.rs to start server

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

### 5. Add integration test

```rust
// tests/websocket_test.rs or inline in websocket.rs
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

## Success Criteria

- [ ] `cargo run -p bucket-streamer` starts server on port 3000
- [ ] `curl http://localhost:3000/health` returns "ok"
- [ ] WebSocket connects at `ws://localhost:3000/ws`
- [ ] SetVideo message returns VideoSet response
- [ ] RequestFrames without SetVideo returns Error
- [ ] Invalid JSON returns Error with parse message
- [ ] Clean disconnect logging on client close
- [ ] Tests pass: `cargo test -p bucket-streamer`

## Context

### Axum WebSocket Pattern
Axum 0.8 uses `WebSocketUpgrade` extractor:
```rust
pub async fn ws_handler(ws: WebSocketUpgrade) -> impl IntoResponse {
    ws.on_upgrade(handle_session)
}
```

The `handle_session` runs as a spawned task after upgrade completes.

### futures-util for Stream/Sink
`socket.split()` requires `StreamExt` and `SinkExt` from futures-util:
- `receiver.next().await` - get next message
- `sender.send(msg).await` - send message

### State Sharing
`AppState` is cloned into each handler. Use `Arc<T>` for expensive-to-clone data:
```rust
pub struct AppState {
    pub config: Arc<Config>,
    pub store: Arc<dyn ObjectStore>,  // Added in Task 11
}
```

### Testing WebSocket Connections
For integration tests with actual WS connection, use `tokio-tungstenite`:
```rust
let (ws, _) = tokio_tungstenite::connect_async("ws://localhost:3000/ws").await?;
```
This is tested more thoroughly in Task 12 (Streaming CLI).
