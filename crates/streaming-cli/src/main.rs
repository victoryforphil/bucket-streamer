use anyhow::{Context, Result};
use clap::Parser;
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::path::PathBuf;
use std::time::Instant;
use tokio_tungstenite::{connect_async, tungstenite::Message};
use url::Url;

#[derive(Parser)]
#[command(name = "streaming-cli")]
#[command(about = "Benchmark client for bucket-streamer")]
#[command(version)]
struct Cli {
    /// WebSocket server URL
    #[arg(short, long, default_value = "ws://localhost:3000/ws")]
    url: String,

    /// Video path on server
    #[arg(short, long)]
    video: String,

    /// Path to JSON file with frame offsets
    #[arg(short, long)]
    frames_file: PathBuf,

    /// Frames per request batch (1 = sequential, N = batched)
    #[arg(short, long, default_value = "1")]
    batch: u32,

    /// Output as JSON
    #[arg(long)]
    json: bool,

    /// Directory to save received JPEGs
    #[arg(short, long)]
    output: Option<PathBuf>,
}

//=============================================================================
// Offsets File Format (from Task 12a)
//=============================================================================

#[derive(Debug, Deserialize)]
struct OffsetsFile {
    video_url: String,
    frames: Vec<FrameEntry>,
}

#[derive(Debug, Deserialize)]
struct FrameEntry {
    offset: u64,
    irap_offset: u64,
}

//=============================================================================
// Protocol Types (matching server - Task 06)
// TODO: Extract to shared crate
//=============================================================================

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum ClientMessage {
    SetVideo { path: String },
    RequestFrames { frames: Vec<FrameRequest> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct FrameRequest {
    offset: u64,
    irap_offset: u64,
    index: u32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum ServerMessage {
    VideoSet {
        path: String,
        ok: bool,
    },
    Frame {
        index: u32,
        offset: u64,
        size: u32,
    },
    FrameError {
        index: u32,
        offset: u64,
        error: String,
    },
    Error {
        message: String,
    },
}

//=============================================================================
// Benchmark Results
//=============================================================================

#[derive(Debug, Serialize)]
struct BenchmarkResult {
    frames_requested: u32,
    frames_received: u32,
    frames_errored: u32,
    total_time_ms: f64,
    average_fps: f64,
    latency_avg_ms: f64,
    latency_min_ms: f64,
    latency_max_ms: f64,
    latency_p50_ms: f64,
    latency_p95_ms: f64,
    latency_p99_ms: f64,
    total_bytes: u64,
}

//=============================================================================
// Main
//=============================================================================

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    run_benchmark(cli).await
}

async fn run_benchmark(args: Cli) -> Result<()> {
    // Load frames from file
    let offsets_json =
        std::fs::read_to_string(&args.frames_file).context("Failed to read frames file")?;
    let offsets: OffsetsFile =
        serde_json::from_str(&offsets_json).context("Failed to parse frames file")?;

    // Create output directory if saving frames
    if let Some(ref out_dir) = args.output {
        std::fs::create_dir_all(out_dir).context("Failed to create output directory")?;
    }

    // Connect to WebSocket
    let url = Url::parse(&args.url).context("Invalid URL")?;
    let (ws, _) = connect_async(url.as_str())
        .await
        .context("Failed to connect")?;
    let (mut sender, mut receiver) = ws.split();

    // Set video
    let set_video = ClientMessage::SetVideo {
        path: args.video.clone(),
    };
    sender
        .send(Message::Text(serde_json::to_string(&set_video)?.into()))
        .await?;

    // Wait for VideoSet response
    match receiver.next().await {
        Some(Ok(Message::Text(text))) => {
            let msg: ServerMessage = serde_json::from_str(&text)?;
            match msg {
                ServerMessage::VideoSet { ok: false, path } => {
                    anyhow::bail!("Video not found: {}", path);
                }
                ServerMessage::Error { message } => {
                    anyhow::bail!("Server error: {}", message);
                }
                ServerMessage::VideoSet { ok: true, .. } => {}
                _ => anyhow::bail!("Unexpected response to SetVideo"),
            }
        }
        Some(Ok(_)) => anyhow::bail!("Unexpected response type to SetVideo"),
        Some(Err(e)) => anyhow::bail!("WebSocket error: {}", e),
        None => anyhow::bail!("Connection closed"),
    }

    // Build frame requests with indices
    let all_frames: Vec<FrameRequest> = offsets
        .frames
        .iter()
        .enumerate()
        .map(|(i, f)| FrameRequest {
            offset: f.offset,
            irap_offset: f.irap_offset,
            index: i as u32,
        })
        .collect();

    let total_frames = all_frames.len() as u32;

    // Process in batches
    let mut latencies: Vec<f64> = Vec::with_capacity(total_frames as usize);
    let mut received = 0u32;
    let mut errored = 0u32;
    let mut total_bytes = 0u64;

    let overall_start = Instant::now();

    for batch in all_frames.chunks(args.batch as usize) {
        let batch_start = Instant::now();

        // Send batch request
        let request = ClientMessage::RequestFrames {
            frames: batch.to_vec(),
        };
        sender
            .send(Message::Text(serde_json::to_string(&request)?.into()))
            .await?;

        // Receive responses for this batch
        let mut pending = batch.len();
        let mut binary_queue: VecDeque<(u32, u64)> = VecDeque::new(); // (index, offset)

        while pending > 0 {
            match receiver.next().await {
                Some(Ok(Message::Text(text))) => {
                    let msg: ServerMessage = serde_json::from_str(&text)?;
                    match msg {
                        ServerMessage::Frame {
                            index,
                            offset,
                            size,
                        } => {
                            binary_queue.push_back((index, offset));
                            total_bytes += size as u64;
                        }
                        ServerMessage::FrameError {
                            index,
                            offset,
                            error,
                        } => {
                            errored += 1;
                            pending -= 1;
                            latencies.push(batch_start.elapsed().as_secs_f64() * 1000.0);
                            if !args.json {
                                eprintln!(
                                    "Frame error: index={}, offset={}, error={}",
                                    index, offset, error
                                );
                            }
                        }
                        ServerMessage::Error { message } => {
                            anyhow::bail!("Server error: {}", message);
                        }
                        _ => {}
                    }
                }
                Some(Ok(Message::Binary(data))) => {
                    if let Some((index, offset)) = binary_queue.pop_front() {
                        received += 1;
                        pending -= 1;
                        latencies.push(batch_start.elapsed().as_secs_f64() * 1000.0);

                        // Save frame if output directory specified
                        if let Some(ref out_dir) = args.output {
                            let path = out_dir.join(format!("frame_{:06}_{}.jpg", index, offset));
                            std::fs::write(&path, &data)?;
                        }
                    }
                }
                Some(Err(e)) => anyhow::bail!("WebSocket error: {}", e),
                None => anyhow::bail!("Connection closed unexpectedly"),
                _ => {}
            }
        }
    }

    let total_time = overall_start.elapsed();

    // Calculate statistics
    latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let latency_avg = if latencies.is_empty() {
        0.0
    } else {
        latencies.iter().sum::<f64>() / latencies.len() as f64
    };

    let result = BenchmarkResult {
        frames_requested: total_frames,
        frames_received: received,
        frames_errored: errored,
        total_time_ms: total_time.as_secs_f64() * 1000.0,
        average_fps: if total_time.as_secs_f64() > 0.0 {
            received as f64 / total_time.as_secs_f64()
        } else {
            0.0
        },
        latency_avg_ms: latency_avg,
        latency_min_ms: *latencies.first().unwrap_or(&0.0),
        latency_max_ms: *latencies.last().unwrap_or(&0.0),
        latency_p50_ms: percentile(&latencies, 50),
        latency_p95_ms: percentile(&latencies, 95),
        latency_p99_ms: percentile(&latencies, 99),
        total_bytes,
    };

    // Output results
    if args.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("\n=== Benchmark Results ===");
        println!("Frames requested: {}", result.frames_requested);
        println!("Frames received:  {}", result.frames_received);
        println!("Frames errored:   {}", result.frames_errored);
        println!("Total time:       {:.1}ms", result.total_time_ms);
        println!("Average FPS:      {:.1}", result.average_fps);
        println!("Total bytes:      {} KB", result.total_bytes / 1024);
        println!();
        println!("Latency (ms):");
        println!("  Avg: {:.2}", result.latency_avg_ms);
        println!("  Min: {:.2}", result.latency_min_ms);
        println!("  Max: {:.2}", result.latency_max_ms);
        println!("  P50: {:.2}", result.latency_p50_ms);
        println!("  P95: {:.2}", result.latency_p95_ms);
        println!("  P99: {:.2}", result.latency_p99_ms);

        if let Some(ref out_dir) = args.output {
            println!();
            println!("Frames saved to: {}", out_dir.display());
        }
    }

    Ok(())
}

fn percentile(sorted: &[f64], p: u32) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = (sorted.len() as f64 * p as f64 / 100.0) as usize;
    sorted[idx.min(sorted.len() - 1)]
}
