# Task 12: Streaming CLI (Benchmark Client)

## Goal
Create lightweight WebSocket client for benchmarking frame streaming. Measures round-trip latency per frame with configurable batching.

## Dependencies
- Task 06: WebSocket Protocol Types (with per-frame `irap_offset`)
- Task 12a: Flattened Offsets Format

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

#[derive(Debug, Serialize, Deserialize)]
struct FrameRequest {
    offset: u64,
    irap_offset: u64,
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
    let offsets_json = std::fs::read_to_string(&args.frames_file)
        .context("Failed to read frames file")?;
    let offsets: OffsetsFile = serde_json::from_str(&offsets_json)
        .context("Failed to parse frames file")?;

    // Create output directory if saving frames
    if let Some(ref out_dir) = args.output {
        std::fs::create_dir_all(out_dir)
            .context("Failed to create output directory")?;
    }

    // Connect to WebSocket
    let url = Url::parse(&args.url).context("Invalid URL")?;
    let (ws, _) = connect_async(url).await.context("Failed to connect")?;
    let (mut sender, mut receiver) = ws.split();

    // Set video
    let set_video = ClientMessage::SetVideo {
        path: args.video.clone(),
    };
    sender
        .send(Message::Text(serde_json::to_string(&set_video)?))
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
            .send(Message::Text(serde_json::to_string(&request)?))
            .await?;

        // Receive responses for this batch
        let mut pending = batch.len();
        let mut binary_queue: VecDeque<(u32, u64)> = VecDeque::new(); // (index, offset)

        while pending > 0 {
            match receiver.next().await {
                Some(Ok(Message::Text(text))) => {
                    let msg: ServerMessage = serde_json::from_str(&text)?;
                    match msg {
                        ServerMessage::Frame { index, offset, size } => {
                            binary_queue.push_back((index, offset));
                            total_bytes += size as u64;
                        }
                        ServerMessage::FrameError { index, offset, error } => {
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
```

## Success Criteria

- [ ] `streaming-cli --video test.h265 --frames-file offsets.json` runs benchmark
- [ ] Frames file is required (no synthetic offset generation)
- [ ] `--batch 1` sends frames sequentially (default)
- [ ] `--batch N` sends N frames per request
- [ ] Latency measured as round-trip (batch send → frame receive)
- [ ] Human-readable output shows FPS and latency stats
- [ ] `--json` outputs valid JSON for scripting
- [ ] `-o ./output` saves JPEGs with naming: `frame_{index}_{offset}.jpg`
- [ ] Errors (video not found, frame errors) reported cleanly
- [ ] Works with `hyperfine` for repeated benchmarks

## Usage Examples

```bash
# Sequential benchmark (precise latency)
streaming-cli --video test.h265 --frames-file data/test.h265.offsets.json

# Batched benchmark (throughput testing)
streaming-cli --video test.h265 --frames-file data/test.h265.offsets.json --batch 10

# Save frames to disk
streaming-cli --video test.h265 --frames-file data/test.h265.offsets.json -o ./frames

# JSON output for scripting
streaming-cli --video test.h265 --frames-file data/test.h265.offsets.json --json > results.json

# Connect to different server
streaming-cli --url ws://192.168.1.100:3000/ws \
  --video test.h265 --frames-file offsets.json

# Use with hyperfine
hyperfine --warmup 2 \
  'streaming-cli --video test.h265 --frames-file offsets.json'
```

## Expected Output

Human-readable:
```
=== Benchmark Results ===
Frames requested: 50
Frames received:  48
Frames errored:   2
Total time:       2100.5ms
Average FPS:      22.8
Total bytes:      2250 KB

Latency (ms):
  Avg: 42.8
  Min: 31.2
  Max: 89.4
  P50: 41.1
  P95: 67.1
  P99: 85.2

Frames saved to: ./frames
```

JSON:
```json
{
  "frames_requested": 50,
  "frames_received": 48,
  "frames_errored": 2,
  "total_time_ms": 2100.5,
  "average_fps": 22.8,
  "latency_avg_ms": 42.8,
  "latency_min_ms": 31.2,
  "latency_max_ms": 89.4,
  "latency_p50_ms": 41.1,
  "latency_p95_ms": 67.1,
  "latency_p99_ms": 85.2,
  "total_bytes": 2304000
}
```

## Context

### Why Separate Binary?
- Lightweight: No FFmpeg, TurboJPEG dependencies
- Fast compile: Quick iteration on benchmarks
- Clean measurements: Minimal overhead
- Hyperfine compatible: Single command invocation

### Latency Measurement
Latency is measured from batch send to individual frame binary receive. With `--batch 1`, this is true round-trip per frame. With larger batches, it measures time from batch start to each frame's arrival.

### Percentile Calculations
- P50 (median): Typical latency
- P95: Most requests faster than this
- P99: Tail latency, important for user experience

### Frame→Binary Pairing
Server sends `Frame` (JSON) immediately followed by binary JPEG. The CLI uses a queue to track which binary belongs to which frame index, handling them in order.

### Protocol Types Duplication
The protocol types (`ClientMessage`, `ServerMessage`, `FrameRequest`) are duplicated from the server. A TODO exists to extract these to a shared crate.

### Integration with Hyperfine

```bash
# Compare batch sizes
hyperfine --parameter-list batch 1,5,10 \
  'streaming-cli --video test.h265 --frames-file offsets.json --batch {batch}'

# Export to JSON for analysis
hyperfine --export-json perf.json \
  'streaming-cli --video test.h265 --frames-file offsets.json'
```
