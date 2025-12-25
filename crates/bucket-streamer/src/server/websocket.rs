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
pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
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

                        if sender.send(Message::Text(json.into())).await.is_err() {
                            error!("Failed to send response");
                            break;
                        }
                    }
                    Err(e) => {
                        let error_msg = ServerMessage::Error {
                            message: format!("Invalid message: {}", e),
                        };
                        if sender
                            .send(Message::Text(error_msg.to_json().into()))
                            .await
                            .is_err()
                        {
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
        ClientMessage::RequestFrames {
            irap_offset,
            frames,
        } => {
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
