use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use tracing::{debug, error, info, warn};

use super::protocol::{ClientMessage, FrameRequest, ServerMessage};
use super::router::AppState;
use crate::pipeline::{decoder::Decoder, encoder::JpegEncoder, fetcher};

/// WebSocket upgrade handler
pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_session(socket, state))
}

/// Handle a WebSocket session
async fn handle_session(socket: WebSocket, state: AppState) {
    let (mut sender, mut receiver) = socket.split();

    info!("WebSocket client connected");

    // Session state
    let mut video_path: Option<String> = None;
    let mut video_data: Option<Bytes> = None;

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
                            &mut video_path,
                            &mut video_data,
                            &state,
                            &mut sender,
                        )
                        .await
                        {
                            Ok(()) => {}
                            Err(e) => {
                                let error_msg = ServerMessage::Error {
                                    message: e.to_string(),
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
    video_path: &mut Option<String>,
    video_data: &mut Option<Bytes>,
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
                sender
                    .send(Message::Text(response.to_json().into()))
                    .await?;
                return Ok(());
            }

            // Fetch video data
            let data = fetcher::fetch_video(&state.store, &path).await?;

            *video_path = Some(path.clone());
            *video_data = Some(data);

            let response = ServerMessage::VideoSet { path, ok: true };
            sender
                .send(Message::Text(response.to_json().into()))
                .await?;
        }

        ClientMessage::RequestFrames { frames } => {
            if video_path.is_none() {
                anyhow::bail!("No video set. Send SetVideo first.");
            }

            // Process frames in blocking task
            let video_data_clone = video_data.as_ref().unwrap().clone();
            let jpeg_quality = state.config.jpeg_quality;

            for request in frames {
                let video_data_inner = video_data_clone.clone();
                let request_clone = request.clone();

                // Process frame in blocking task (FFmpeg is not Send)
                let result = tokio::task::spawn_blocking(move || {
                    process_frame(video_data_inner, request_clone, jpeg_quality)
                })
                .await;

                match result {
                    Ok(Ok(jpeg_data)) => {
                        // Send frame metadata
                        let frame_msg = ServerMessage::Frame {
                            index: request.index,
                            offset: request.offset,
                            size: jpeg_data.len() as u32,
                        };
                        sender
                            .send(Message::Text(frame_msg.to_json().into()))
                            .await?;

                        // Send binary JPEG data
                        sender.send(Message::Binary(jpeg_data.into())).await?;
                    }
                    Ok(Err(e)) => {
                        let error_msg = ServerMessage::FrameError {
                            index: request.index,
                            offset: request.offset,
                            error: e.to_string(),
                        };
                        sender
                            .send(Message::Text(error_msg.to_json().into()))
                            .await?;
                    }
                    Err(e) => {
                        let error_msg = ServerMessage::FrameError {
                            index: request.index,
                            offset: request.offset,
                            error: format!("Task join error: {}", e),
                        };
                        sender
                            .send(Message::Text(error_msg.to_json().into()))
                            .await?;
                    }
                }
            }
        }
    }

    Ok(())
}

/// Process a single frame (runs in blocking context)
fn process_frame(
    video_data: Bytes,
    request: FrameRequest,
    jpeg_quality: u8,
) -> anyhow::Result<Vec<u8>> {
    // Create decoder
    let mut decoder = Decoder::new(&video_data)?;

    // Decode frame
    let frame = decoder.decode_frame(&video_data, request.offset)?;

    // Create encoder and encode
    let mut encoder = JpegEncoder::new(jpeg_quality)?;
    let jpeg = encoder.encode(&frame)?;

    Ok(jpeg)
}
