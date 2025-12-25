# Task 12: Streaming CLI (Benchmark Client)

## Goal
Create lightweight WebSocket client for benchmarking frame streaming. Outputs latency metrics in human-readable and JSON formats.

## Dependencies
- Task 06: WebSocket Protocol Types

## Files to Modify

```
crates/streaming-cli/src/main.rs    # Full implementation
crates/streaming-cli/Cargo.toml     # Ensure deps are correct
```

## Steps

### 1. Verify streaming-cli Cargo.toml

```toml
[package]
name = "streaming-cli"
version.workspace = true
edition.workspace = true

[dependencies]
tokio.workspace = true
clap.workspace = true
serde.workspace = true
serde_json.workspace = true
tokio-tungstenite.workspace = true
anyhow.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
futures-util = "0.3"
url = "2"
```

### 2. Implement main.rs

```rust
use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};
use tokio_tungstenite::{connect_async, tungstenite::Message};
use url::Url;

#[derive(Parser)]
#[command(name = "streaming-cli")]
#[command(about = "Benchmark client for bucket-streamer")]
#[command(version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run frame request benchmark
    Bench(BenchArgs),

    /// Send a single request (for testing)
    Request(RequestArgs),
}

#[derive(clap::Args)]
struct BenchArgs {
    /// WebSocket server URL
    #[arg(short, long, default_value = "ws://localhost:3000/ws")]
    url: String,

    /// Video path on server
    #[arg(short, long)]
    video: String,

    /// Number of frames to request
    #[arg(short, long, default_value = "100")]
    frames: u32,

    /// IRAP offset for all frames
    #[arg(long, default_value = "0")]
    irap_offset: u64,

    /// Starting frame offset
    #[arg(long, default_value = "0")]
    start_offset: u64,

    /// Offset increment between frames
    #[arg(long, default_value = "1000")]
    offset_step: u64,

    /// Output as JSON
    #[arg(long)]
    json: bool,
}

#[derive(clap::Args)]
struct RequestArgs {
    /// WebSocket server URL
    #[arg(short, long, default_value = "ws://localhost:3000/ws")]
    url: String,

    /// Video path on server
    #[arg(short, long)]
    video: String,

    /// Frame offset to request
    #[arg(short, long)]
    offset: u64,

    /// IRAP offset
    #[arg(long, default_value = "0")]
    irap_offset: u64,

    /// Output as JSON
    #[arg(long)]
    json: bool,
}

// Protocol types (matching server)
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum ClientMessage {
    SetVideo { path: String },
    RequestFrames { irap_offset: u64, frames: Vec<FrameRequest> },
}

#[derive(Debug, Serialize, Deserialize)]
struct FrameRequest {
    offset: u64,
    index: u32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
enum ServerMessage {
    VideoSet { path: String, ok: bool },
    Frame { index: u32, offset: u64, size: u32 },
    FrameError { index: u32, offset: u64, error: String },
    Error { message: String },
}

// Benchmark results
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

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Commands::Bench(args) => run_benchmark(args).await,
        Commands::Request(args) => run_single_request(args).await,
    }
}

async fn run_benchmark(args: BenchArgs) -> Result<()> {
    let url = Url::parse(&args.url).context("Invalid URL")?;
    let (ws, _) = connect_async(url).await.context("Failed to connect")?;
    let (mut sender, mut receiver) = ws.split();

    // Set video
    let set_video = ClientMessage::SetVideo {
        path: args.video.clone(),
    };
    sender.send(Message::Text(serde_json::to_string(&set_video)?)).await?;

    // Wait for VideoSet response
    match receiver.next().await {
        Some(Ok(Message::Text(text))) => {
            let msg: ServerMessage = serde_json::from_str(&text)?;
            if let ServerMessage::VideoSet { ok: false, .. } = msg {
                anyhow::bail!("Video not found: {}", args.video);
            }
            if let ServerMessage::Error { message } = msg {
                anyhow::bail!("Server error: {}", message);
            }
        }
        _ => anyhow::bail!("Unexpected response to SetVideo"),
    }

    // Build frame requests
    let frames: Vec<FrameRequest> = (0..args.frames)
        .map(|i| FrameRequest {
            offset: args.start_offset + (i as u64 * args.offset_step),
            index: i,
        })
        .collect();

    let request = ClientMessage::RequestFrames {
        irap_offset: args.irap_offset,
        frames,
    };

    // Start timing
    let start = Instant::now();
    let mut latencies: Vec<f64> = Vec::with_capacity(args.frames as usize);
    let mut received = 0u32;
    let mut errored = 0u32;
    let mut total_bytes = 0u64;
    let mut frame_start = Instant::now();

    // Send request
    sender.send(Message::Text(serde_json::to_string(&request)?)).await?;

    // Receive responses
    let mut expecting_binary = false;
    while received + errored < args.frames {
        match receiver.next().await {
            Some(Ok(Message::Text(text))) => {
                let msg: ServerMessage = serde_json::from_str(&text)?;
                match msg {
                    ServerMessage::Frame { size, .. } => {
                        expecting_binary = true;
                        total_bytes += size as u64;
                    }
                    ServerMessage::FrameError { .. } => {
                        errored += 1;
                        latencies.push(frame_start.elapsed().as_secs_f64() * 1000.0);
                        frame_start = Instant::now();
                    }
                    ServerMessage::Error { message } => {
                        anyhow::bail!("Server error: {}", message);
                    }
                    _ => {}
                }
            }
            Some(Ok(Message::Binary(_))) => {
                if expecting_binary {
                    received += 1;
                    latencies.push(frame_start.elapsed().as_secs_f64() * 1000.0);
                    frame_start = Instant::now();
                    expecting_binary = false;
                }
            }
            Some(Err(e)) => anyhow::bail!("WebSocket error: {}", e),
            None => break,
            _ => {}
        }
    }

    let total_time = start.elapsed();

    // Calculate statistics
    latencies.sort_by(|a, b| a.partial_cmp(b).unwrap());

    let result = BenchmarkResult {
        frames_requested: args.frames,
        frames_received: received,
        frames_errored: errored,
        total_time_ms: total_time.as_secs_f64() * 1000.0,
        average_fps: received as f64 / total_time.as_secs_f64(),
        latency_avg_ms: latencies.iter().sum::<f64>() / latencies.len() as f64,
        latency_min_ms: *latencies.first().unwrap_or(&0.0),
        latency_max_ms: *latencies.last().unwrap_or(&0.0),
        latency_p50_ms: percentile(&latencies, 50),
        latency_p95_ms: percentile(&latencies, 95),
        latency_p99_ms: percentile(&latencies, 99),
        total_bytes,
    };

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
    }

    Ok(())
}

async fn run_single_request(args: RequestArgs) -> Result<()> {
    let url = Url::parse(&args.url).context("Invalid URL")?;
    let (ws, _) = connect_async(url).await.context("Failed to connect")?;
    let (mut sender, mut receiver) = ws.split();

    // Set video
    let set_video = ClientMessage::SetVideo {
        path: args.video.clone(),
    };
    sender.send(Message::Text(serde_json::to_string(&set_video)?)).await?;

    // Wait for VideoSet
    if let Some(Ok(Message::Text(text))) = receiver.next().await {
        let msg: ServerMessage = serde_json::from_str(&text)?;
        if let ServerMessage::VideoSet { ok: false, .. } = msg {
            anyhow::bail!("Video not found");
        }
    }

    // Request single frame
    let request = ClientMessage::RequestFrames {
        irap_offset: args.irap_offset,
        frames: vec![FrameRequest { offset: args.offset, index: 0 }],
    };

    let start = Instant::now();
    sender.send(Message::Text(serde_json::to_string(&request)?)).await?;

    // Receive response
    let mut frame_size = 0u32;
    let mut jpeg_data: Option<Vec<u8>> = None;

    for _ in 0..2 {  // Expect Frame + Binary
        match receiver.next().await {
            Some(Ok(Message::Text(text))) => {
                let msg: ServerMessage = serde_json::from_str(&text)?;
                match msg {
                    ServerMessage::Frame { size, .. } => frame_size = size,
                    ServerMessage::FrameError { error, .. } => {
                        anyhow::bail!("Frame error: {}", error);
                    }
                    _ => {}
                }
            }
            Some(Ok(Message::Binary(data))) => {
                jpeg_data = Some(data);
            }
            _ => {}
        }
    }

    let elapsed = start.elapsed();

    if args.json {
        #[derive(Serialize)]
        struct SingleResult {
            offset: u64,
            size: u32,
            latency_ms: f64,
        }
        let result = SingleResult {
            offset: args.offset,
            size: frame_size,
            latency_ms: elapsed.as_secs_f64() * 1000.0,
        };
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("Frame received:");
        println!("  Offset: {}", args.offset);
        println!("  Size: {} bytes", frame_size);
        println!("  Latency: {:.2}ms", elapsed.as_secs_f64() * 1000.0);
    }

    // Optionally save JPEG
    if let Some(data) = jpeg_data {
        let path = format!("frame_{}.jpg", args.offset);
        std::fs::write(&path, &data)?;
        println!("  Saved: {}", path);
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
```

## Success Criteria

- [ ] `streaming-cli bench --video test.mp4 --frames 10` runs benchmark
- [ ] Human-readable output shows FPS and latency stats
- [ ] `--json` outputs valid JSON for scripting
- [ ] `streaming-cli request --video test.mp4 --offset 1000` fetches single frame
- [ ] Single request saves JPEG to disk
- [ ] Errors (video not found, server down) reported cleanly
- [ ] Works with `hyperfine` for repeated benchmarks

## Usage Examples

```bash
# Basic benchmark
streaming-cli bench --video test.h265.mp4 --frames 100

# Benchmark with JSON output
streaming-cli bench --video test.h265.mp4 --frames 100 --json > results.json

# Single frame request
streaming-cli request --video test.h265.mp4 --offset 5000

# Use with hyperfine for statistical analysis
hyperfine --warmup 2 \
  'streaming-cli bench --video test.h265.mp4 --frames 50'

# Connect to different server
streaming-cli bench --url ws://192.168.1.100:3000/ws \
  --video test.h265.mp4 --frames 100
```

## Expected Output

Human-readable:
```
=== Benchmark Results ===
Frames requested: 100
Frames received:  98
Frames errored:   2
Total time:       4200.5ms
Average FPS:      23.3
Total bytes:      4523 KB

Latency (ms):
  Avg: 42.8
  Min: 31.2
  Max: 89.4
  P50: 41.1
  P95: 67.1
  P99: 85.2
```

JSON:
```json
{
  "frames_requested": 100,
  "frames_received": 98,
  "frames_errored": 2,
  "total_time_ms": 4200.5,
  "average_fps": 23.3,
  "latency_avg_ms": 42.8,
  "latency_min_ms": 31.2,
  "latency_max_ms": 89.4,
  "latency_p50_ms": 41.1,
  "latency_p95_ms": 67.1,
  "latency_p99_ms": 85.2,
  "total_bytes": 4631552
}
```

## Context

### Why Separate Binary?
- Lightweight: No FFmpeg, TurboJPEG dependencies
- Fast compile: Quick iteration on benchmarks
- Clean measurements: Minimal overhead
- Hyperfine compatible: Single command invocation

### Latency Measurement
Latency is measured from just before sending request to receiving complete JPEG. This includes:
- Network round-trip
- Server decode time
- Server encode time
- Response transmission

### Percentile Calculations
- P50 (median): Typical latency
- P95: Most requests faster than this
- P99: Tail latency, important for user experience

### Integration with Hyperfine

```bash
# Compare different quality settings
hyperfine --parameter-list quality 60,80,95 \
  'streaming-cli bench --video test.mp4 --frames 50'

# Export to JSON for analysis
hyperfine --export-json perf.json \
  'streaming-cli bench --video test.mp4 --frames 100'
```
