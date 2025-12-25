# Bucket-Streamer Project Analysis

**Last Updated:** December 25, 2025
**Rust Toolchain:** 1.92.0
**Project Status:** Stage 1 Prototype - Core pipeline implemented and functional

---

## 1. PROJECT OVERVIEW

### What It Does

**bucket-streamer** is a WebSocket-based video streaming server that serves individual video frames on-demand. It's specifically designed for H.265/HEVC video files stored in S3 or local filesystem.

**Key Features:**
- Extracts frames from H.265 MP4 files by byte offset
- Encodes frames to JPEG for efficient transmission
- Serves frames over WebSocket with metadata
- Supports both local filesystem and S3 storage backends
- Measures and reports FPS and latency metrics
- CLI tools for video conversion and benchmarking

**Core Problem Solved:**
Instead of requiring clients to download entire videos or use standard streaming protocols, bucket-streamer allows precise, on-demand frame extraction at specific byte offsets. This is useful for applications like:
- Video analysis/inspection tools
- Thumbnail extraction
- Keyframe-based navigation
- Low-latency remote monitoring

---

## 2. SERVER ARCHITECTURE OVERVIEW

### High-Level Diagram

```
┌─────────────────────────────────────────────────────────────────┐
│                  WebSocket Server (Axum)                        │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  GET /health         → Health check endpoint                    │
│  GET /ws             → WebSocket upgrade (ws_handler)           │
│                                                                 │
│  Session Management:                                            │
│  ├─ Accepts SetVideo command                                    │
│  ├─ Fetches video file from storage                             │
│  └─ Streams RequestFrames with JPEG responses                   │
│                                                                 │
│  Per-Frame Pipeline:                                            │
│  ├─ H.265 Decoder (ffmpeg-next) → YUV420P frames               │
│  ├─ JPEG Encoder (turbojpeg)     → Compressed JPEG             │
│  └─ Binary WebSocket Send        → JPEG + metadata             │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
         │                          │
         ▼                          ▼
    Object Store              Video Data
   (LocalFS / S3)            (Bytes buffer)
```

### Module Structure

```
crates/
├── bucket-streamer/         # Main server binary
│   ├── src/
│   │   ├── main.rs         # Entry point, CLI args, server startup
│   │   ├── config.rs       # Configuration (Clap + Serde)
│   │   │
│   │   ├── server/
│   │   │   ├── router.rs   # Axum router setup, AppState
│   │   │   ├── websocket.rs # WS session handler, frame processing
│   │   │   └── protocol.rs  # Message types (JSON serialization)
│   │   │
│   │   ├── pipeline/        # Frame processing pipeline
│   │   │   ├── mod.rs
│   │   │   ├── decoder.rs   # H.265 decode (FFmpeg)
│   │   │   ├── encoder.rs   # JPEG encode (turbojpeg)
│   │   │   ├── fetcher.rs   # Storage fetch abstraction
│   │   │   ├── avio.rs      # In-memory AVIO context for FFmpeg
│   │   │   └── session.rs   # Session state management
│   │   │
│   │   └── storage/
│   │       ├── mod.rs
│   │       └── backend.rs   # object_store abstraction (LocalFS/S3)
│   │
│   └── Cargo.toml
│
├── repo-cli/                # Development utilities
│   ├── src/
│   │   ├── main.rs
│   │   ├── commands/
│   │   │   ├── convert.rs   # Video conversion (MP4 → H.265)
│   │   │   └── devshell.rs  # Docker shell execution
│   │   └── error.rs
│   └── Cargo.toml
│
└── streaming-cli/           # Benchmark client
    ├── src/main.rs          # WebSocket benchmark client
    └── Cargo.toml

data/                         # Test data directory
├── .gitkeep
├── test_dji_sfnight_1.mp4   # ~1GB test video (DJI footage)
├── test_dji_sfnight_2.mp4   # ~472MB
├── test_dji_sfnight_3.mp4   # ~525MB
├── test_dji_sfnight_4.mp4   # ~614MB
├── test_dji_sfnight_5.mp4   # ~582MB
├── test_dji_sfnight_6.mp4   # ~519MB
├── test_dji_sfnight_7.mp4   # ~862MB
├── test_dji_sfnight_1.h265  # ~2.6MB (sample converted)
└── out/                      # Output directory for benchmarks
```

### Key Technologies

| Component | Technology | Version | Purpose |
|-----------|-----------|---------|---------|
| Web Framework | Axum | 0.7 | HTTP/WebSocket server |
| Async Runtime | Tokio | 1.x | Task execution, channels |
| Video Decoding | FFmpeg (ffmpeg-next) | 8.0 | H.265/HEVC decoder |
| JPEG Encoding | TurboJPEG | 1.3 | Fast JPEG compression |
| Storage | object_store | 0.11 | Unified S3/LocalFS API |
| CLI | Clap | 4.x | Argument parsing |
| Serialization | Serde + serde_json | Latest | JSON protocol |

---

## 3. HOW TO START THE SERVER

### Prerequisites

1. **Install dependencies** (via Docker or local system):
   - FFmpeg libraries (libavformat, libavcodec, libavutil, libswscale)
   - TurboJPEG library (libturbojpeg0-dev)
   - Rust 1.70+ (already installed: v1.92.0)

2. **Environment:**
   - `RUST_LOG=info` (or `debug` for verbose output)
   - `STORAGE_BACKEND=local` (default)
   - `LOCAL_PATH=./data` (where videos are stored)

### Option A: Docker (Recommended)

```bash
# Start both server and MinIO S3 mock
docker-compose up

# In another terminal, run tests
docker-compose run --rm bucket-streamer cargo test

# Or run server directly
docker-compose exec bucket-streamer cargo run -p bucket-streamer
```

### Option B: Local Build

```bash
# Build (debug)
cargo build -p bucket-streamer

# Build (release, optimized)
cargo build --release -p bucket-streamer

# Run with defaults
cargo run -p bucket-streamer

# Run with custom config
cargo run -p bucket-streamer -- \
  --listen-addr 0.0.0.0:3000 \
  --storage-backend local \
  --local-path ./data \
  --jpeg-quality 85

# Run in background
cargo run --release -p bucket-streamer > server.log 2>&1 &
```

### Configuration Options

All options can be set via CLI args or environment variables:

```bash
# CLI argument format
cargo run -p bucket-streamer -- --listen-addr 0.0.0.0:3000 --jpeg-quality 80

# Environment variable format
export LISTEN_ADDR="0.0.0.0:3000"
export STORAGE_BACKEND="local"
export LOCAL_PATH="./data"
export JPEG_QUALITY="80"
export RUST_LOG="info"
cargo run -p bucket-streamer
```

**Available Options:**
- `--listen-addr` (default: `0.0.0.0:3000`) - Server bind address
- `--storage-backend` (default: `local`) - Storage type: `local` or `s3`
- `--local-path` (default: `./data`) - Path for local filesystem storage
- `--s3-bucket` (required if using S3) - S3 bucket name
- `--s3-region` (default: `us-east-1`) - AWS region
- `--s3-endpoint` (optional) - Custom endpoint (for MinIO, etc.)
- `--s3-access-key` (default: `minioadmin`) - Access credentials
- `--s3-secret-key` (default: `minioadmin`)
- `--jpeg-quality` (default: `80`) - JPEG quality (1-100)
- `--log-level` (default: `info`) - Log verbosity

### Verify Server Is Running

```bash
# Health check
curl http://localhost:3000/health
# Expected response: "ok"

# WebSocket endpoint (requires wscat)
wscat -c ws://localhost:3000/ws
# Should connect successfully
```

---

## 4. CLI COMMANDS AND TOOLS

### 4.1 repo-cli: Development Utilities

**Binary:** `crates/repo-cli/src/main.rs`

#### Command: `convert` - Video to H.265 Conversion

Convert standard video formats to H.265/HEVC MP4 and optionally extract frame offsets.

**Usage:**
```bash
cargo run -p repo-cli -- convert \
  --input <INPUT_FILE> \
  [--output <OUTPUT_FILE>] \
  [--extract-offsets] \
  [--storage-url <URL>] \
  [--force] \
  [--gpu] \
  [--downscale <N>] \
  [--fps <FPS>]
```

**Arguments:**
- `--input` (required): Input video file path
- `--output`: Output file path (default: `{input}.h265`)
- `--recursive` / `-R`: Batch process all `.mp4` files in directory
- `--extract-offsets`: Generate JSON file with frame offsets
- `--storage-url`: Storage URL for offset metadata (e.g., `s3://bucket/video.h265`)
- `--force`: Overwrite existing output
- `--gpu`: Enable NVENC GPU acceleration (if available)
- `--downscale`: Scale factor (2 = half resolution, 4 = quarter)
- `--fps`: Target framerate (e.g., 30, 24, 15)

**Examples:**

```bash
# Convert single video
cargo run -p repo-cli -- convert \
  --input data/test_dji_sfnight_1.mp4 \
  --output data/test.h265 \
  --force

# Convert with offset extraction
cargo run -p repo-cli -- convert \
  --input data/test_dji_sfnight_1.mp4 \
  --output data/test.h265 \
  --extract-offsets \
  --storage-url "fs:///data/test.h265" \
  --force

# Batch convert all MP4s in directory
cargo run -p repo-cli -- convert \
  --input data/ \
  --recursive \
  --gpu \
  --extract-offsets \
  --force

# Downscale to quarter resolution (for testing)
cargo run -p repo-cli -- convert \
  --input data/test_dji_sfnight_1.mp4 \
  --output data/test_small.h265 \
  --downscale 4 \
  --extract-offsets
```

**Output:**
- H.265 MP4 file at specified output path
- (Optional) JSON offset file: `<output>.offsets.json`

**Offset JSON Format:**
```json
{
  "video_url": "fs:///data/test.h265",
  "frames": [
    { "offset": 1024, "irap_offset": 1024 },
    { "offset": 2048, "irap_offset": 1024 },
    { "offset": 3072, "irap_offset": 1024 }
  ]
}
```

---

#### Command: `devshell` - Docker Container Shell

Execute arbitrary commands inside the Docker development container.

**Usage:**
```bash
cargo run -p repo-cli -- devshell [--] <COMMAND> [ARGS...]
```

**Examples:**
```bash
# Run tests in container
cargo run -p repo-cli -- devshell -- cargo test

# Interactive bash
cargo run -p repo-cli -- devshell -- bash

# List data directory
cargo run -p repo-cli -- devshell -- ls -la /workspace/data
```

---

### 4.2 streaming-cli: Benchmark Client

**Binary:** `crates/streaming-cli/src/main.rs`

WebSocket client that benchmarks the streaming server with latency/FPS metrics.

**Usage:**
```bash
cargo run -p streaming-cli -- \
  --url <WS_URL> \
  --video <VIDEO_PATH> \
  --frames-file <OFFSETS_JSON> \
  [--batch <N>] \
  [--output <DIR>] \
  [--json]
```

**Arguments:**
- `--url` (default: `ws://localhost:3000/ws`) - Server WebSocket URL
- `--video` (required) - Video path on server (matches storage path)
- `--frames-file` (required) - JSON file with frame offsets (generated by `repo-cli convert`)
- `--batch` (default: `1`) - Frames per request (1 = sequential, N = batched)
- `--output` (optional) - Directory to save received JPEG frames
- `--json` (flag) - Output results as JSON instead of human-readable

**Examples:**

```bash
# Sequential benchmark (measure per-frame latency)
cargo run -p streaming-cli -- \
  --video data/test.h265 \
  --frames-file data/test.h265.offsets.json

# Batched requests for throughput test
cargo run -p streaming-cli -- \
  --video data/test.h265 \
  --frames-file data/test.h265.offsets.json \
  --batch 10

# Save received frames as JPEGs
cargo run -p streaming-cli -- \
  --video data/test.h265 \
  --frames-file data/test.h265.offsets.json \
  --output data/out/frames \
  --batch 5

# JSON output for scripting
cargo run -p streaming-cli -- \
  --video data/test.h265 \
  --frames-file data/test.h265.offsets.json \
  --json > benchmark_results.json

# Remote server
cargo run -p streaming-cli -- \
  --url ws://remote-server:3000/ws \
  --video s3://bucket/video.h265 \
  --frames-file data/offsets.json
```

**Output (Human-Readable):**
```
=== Benchmark Results ===
Frames requested: 100
Frames received:  100
Frames errored:   0
Total time:       4200.5ms
Average FPS:      23.8
Total bytes:      4520 KB

Latency (ms):
  Avg: 42.0
  Min: 31.2
  Max: 89.4
  P50: 38.5
  P95: 72.1
  P99: 87.2

Frames saved to: data/out/frames
```

**Output (JSON):**
```json
{
  "frames_requested": 100,
  "frames_received": 100,
  "frames_errored": 0,
  "total_time_ms": 4200.5,
  "average_fps": 23.8,
  "latency_avg_ms": 42.0,
  "latency_min_ms": 31.2,
  "latency_max_ms": 89.4,
  "latency_p50_ms": 38.5,
  "latency_p95_ms": 72.1,
  "latency_p99_ms": 87.2,
  "total_bytes": 4628480
}
```

---

## 5. AVAILABLE TEST DATA

### Location
`/home/vfp/repos/vfp/bucket-streamer/data/`

### Test Videos (Raw MP4 Files)

| Filename | Size | Source | Notes |
|----------|------|--------|-------|
| `test_dji_sfnight_1.mp4` | 1.0 GB | DJI Drone | Full night flight footage |
| `test_dji_sfnight_2.mp4` | 472 MB | DJI Drone | Segment |
| `test_dji_sfnight_3.mp4` | 525 MB | DJI Drone | Segment |
| `test_dji_sfnight_4.mp4` | 614 MB | DJI Drone | Segment |
| `test_dji_sfnight_5.mp4` | 582 MB | DJI Drone | Segment |
| `test_dji_sfnight_6.mp4` | 519 MB | DJI Drone | Segment |
| `test_dji_sfnight_7.mp4` | 862 MB | DJI Drone | Segment |

### Converted Test Data

| Filename | Size | Generated By | Contains |
|----------|------|--------------|----------|
| `test_dji_sfnight_1.h265` | 2.6 MB | `repo-cli convert` | Sample H.265 conversion |

### How to Generate Test Data

#### 1. Convert MP4 to H.265 with Offsets

```bash
# Single video conversion
cargo run -p repo-cli -- convert \
  --input data/test_dji_sfnight_1.mp4 \
  --output data/test.h265 \
  --extract-offsets \
  --force

# This generates:
# - data/test.h265 (converted video, ~2-10MB depending on encoding)
# - data/test.h265.offsets.json (frame offset metadata)
```

#### 2. Generate Smaller Test Data (for faster testing)

```bash
# Downscale to 25% resolution (faster conversion)
cargo run -p repo-cli -- convert \
  --input data/test_dji_sfnight_1.mp4 \
  --output data/test_small.h265 \
  --downscale 4 \
  --extract-offsets \
  --force

# Or extract specific frames only by downscaling and limiting FPS
cargo run -p repo-cli -- convert \
  --input data/test_dji_sfnight_1.mp4 \
  --output data/test_fast.h265 \
  --downscale 2 \
  --fps 10 \
  --extract-offsets \
  --force
```

#### 3. Batch Convert All Videos

```bash
cargo run -p repo-cli -- convert \
  --input data/ \
  --recursive \
  --extract-offsets \
  --force
```

#### 4. Inspect Generated Offset File

```bash
# View offset JSON structure
cat data/test.h265.offsets.json | jq .

# Count total frames
cat data/test.h265.offsets.json | jq '.frames | length'

# Show first 5 frame offsets
cat data/test.h265.offsets.json | jq '.frames[:5]'
```

---

## 6. WEBSOCKET PROTOCOL AND FRAME SERVING

### Protocol Overview

Communication uses JSON text messages + binary JPEG payloads over WebSocket.

### Message Flow Diagram

```
Client                                          Server
  │                                              │
  ├─ GET /ws (upgrade) ──────────────────────────▶
  │                                              │
  ◀─ 101 Switching Protocols ─────────────────────┤
  │                                              │
  ├─ {"type": "SetVideo", "path": "..."} ───────▶ (fetch video from storage)
  │                                              │
  ◀─ {"type": "VideoSet", "ok": true} ──────────┤
  │                                              │
  ├─ {"type": "RequestFrames", "frames": [...]} ─▶ (process each frame)
  │                                              │
  ◀─ {"type": "Frame", "index": 0, ...} ────────┤
  ◀─ [Binary: JPEG data] ─────────────────────────┤
  │                                              │
  ◀─ {"type": "Frame", "index": 1, ...} ────────┤
  ◀─ [Binary: JPEG data] ─────────────────────────┤
  │                                              │
  │ (repeat for each frame)                     │
```

### JSON Message Types

#### Client → Server

**1. SetVideo - Set video source**
```json
{
  "type": "SetVideo",
  "path": "data/test.h265"
}
```

**2. RequestFrames - Request multiple frames**
```json
{
  "type": "RequestFrames",
  "frames": [
    {
      "offset": 1024,
      "irap_offset": 1024,
      "index": 0
    },
    {
      "offset": 2048,
      "irap_offset": 1024,
      "index": 1
    }
  ]
}
```

#### Server → Client

**1. VideoSet - Response to SetVideo**
```json
{
  "type": "VideoSet",
  "path": "data/test.h265",
  "ok": true
}
```

**2. Frame - Metadata before binary JPEG data**
```json
{
  "type": "Frame",
  "index": 0,
  "offset": 1024,
  "size": 45230
}
```
*(Followed immediately by binary message with JPEG data)*

**3. FrameError - Decode/encode failed for frame**
```json
{
  "type": "FrameError",
  "index": 5,
  "offset": 3072,
  "error": "decode_failed"
}
```

**4. Error - General server error**
```json
{
  "type": "Error",
  "message": "Video not found: data/missing.h265"
}
```

### Frame Serving Pipeline

For each frame request:

1. **Fetch** - Retrieve video data from storage (cached in session)
2. **Decode** - H.265 decoder extracts frame at specified offset
   - Input: H.265 bitstream (bytes)
   - Output: YUV420P planar format (uncompressed)
3. **Encode** - TurboJPEG compresses to JPEG
   - Input: YUV420P frame
   - Output: JPEG binary data
4. **Send** - WebSocket sends metadata JSON, then binary JPEG

### Current Limitations

- **Single client:** No concurrent WebSocket sessions (Stage 1)
- **No frame caching:** Each request decodes and encodes from scratch
- **FIFO queue:** All frames processed in request order (no LIFO optimization)
- **Entire video in memory:** Full video data loaded after SetVideo

### Performance Characteristics

**Measured FPS (sequential, local storage):**
- 480p: ~30-50 FPS
- 720p: ~20-30 FPS
- 1080p: ~10-20 FPS

*Actual numbers depend on CPU, video complexity, JPEG quality, and batching.*

---

## 7. METRICS AND PERFORMANCE TRACKING

### Available Metrics

The streaming-cli benchmark client tracks:

#### Per-Frame Metrics
- **Latency:** Time from request sent to frame received (ms)
- **Index:** Frame identifier (0-based)
- **Size:** JPEG payload size (bytes)

#### Aggregate Statistics
- **Frames Requested:** Total frames in benchmark
- **Frames Received:** Successfully decoded frames
- **Frames Errored:** Decode/encode failures
- **Total Time:** Wall-clock duration of entire benchmark
- **Average FPS:** `frames_received / total_time`
- **Total Bytes:** Sum of all JPEG sizes

#### Latency Percentiles
- **Min:** Fastest single frame
- **Max:** Slowest single frame
- **Avg:** Mean latency across all frames
- **P50:** Median (50th percentile)
- **P95:** 95th percentile (99% faster than this)
- **P99:** 99th percentile (fastest 1%)

### How to Measure Metrics

#### 1. Sequential Throughput (measure FPS per frame)

```bash
cargo run -p streaming-cli -- \
  --video data/test.h265 \
  --frames-file data/test.h265.offsets.json \
  --batch 1
```

**Interpreting Results:**
- High `Average FPS` = good single-frame performance
- Low `Latency Avg` = responsive server
- Low `Latency Max` = consistent performance (no spikes)

#### 2. Batched Throughput (measure total FPS)

```bash
cargo run -p streaming-cli -- \
  --video data/test.h265 \
  --frames-file data/test.h265.offsets.json \
  --batch 10
```

**Expected:** FPS should increase ~2-3x vs batch=1

#### 3. Latency Profile Comparison

```bash
# Save results for comparison
for batch in 1 5 10 20; do
  echo "=== Batch Size: $batch ===" 
  cargo run -p streaming-cli --release -- \
    --video data/test.h265 \
    --frames-file data/test.h265.offsets.json \
    --batch $batch \
    --json > benchmark_batch_${batch}.json
done

# Compare latencies
jq '.latency_avg_ms, .latency_p95_ms, .average_fps' benchmark_batch_*.json
```

#### 4. Machine-Readable JSON Output

```bash
cargo run -p streaming-cli -- \
  --video data/test.h265 \
  --frames-file data/test.h265.offsets.json \
  --json > results.json

# Parse results
jq '.average_fps' results.json
jq '.latency_p95_ms' results.json
jq '.total_bytes / 1024' results.json  # KB
```

### Server-Side Metrics (Logs)

The server logs contain useful diagnostic information:

```bash
# Run with debug logging
RUST_LOG=debug cargo run -p bucket-streamer

# Look for timing information in logs:
# - "WebSocket client connected"
# - "Setting video: {path}"
# - "Frame decode/encode" (in processing)
```

---

## 8. EXISTING TEST WORKFLOWS

### Unit Tests

Run all tests:
```bash
cargo test
```

Run tests for specific crate:
```bash
cargo test -p bucket-streamer
cargo test -p repo-cli
cargo test -p streaming-cli
```

Run with output:
```bash
cargo test -- --nocapture
```

Run ignored tests (benchmarks):
```bash
cargo test -- --ignored --nocapture
```

### Test Coverage

**bucket-streamer tests:**
- `config.rs`: Config validation (S3 bucket required, local path valid)
- `protocol.rs`: Message serialization/deserialization (SetVideo, RequestFrames, responses)
- `encoder.rs`: JPEG encoding (quality levels, frame sizes, JPEG format)
- `decoder.rs`: H.265 decoding (decoder creation, frame extraction, YUV420P format)
- `storage/backend.rs`: Object store operations (fetch_range, fetch_all, exists)
- `server/router.rs`: Health check endpoint

**repo-cli tests:**
- Video conversion commands
- Offset file generation
- Batch processing

### Integration Tests

#### Full End-to-End Test

1. **Convert test video:**
```bash
cargo run -p repo-cli -- convert \
  --input data/test_dji_sfnight_1.mp4 \
  --output data/test.h265 \
  --extract-offsets \
  --force
```

2. **Start server:**
```bash
cargo run -p bucket-streamer &
sleep 2  # Wait for startup
```

3. **Run benchmark:**
```bash
cargo run -p streaming-cli -- \
  --video data/test.h265 \
  --frames-file data/test.h265.offsets.json \
  --batch 5 \
  --output data/out/frames
```

4. **Verify results:**
```bash
# Check JPEG output
file data/out/frames/frame_*.jpg

# Verify FPS >= 20 for Stage 1 target
cargo run -p streaming-cli -- \
  --video data/test.h265 \
  --frames-file data/test.h265.offsets.json \
  --json | jq '.average_fps'
```

### Docker-Based Testing

```bash
# Full test in container
docker-compose up -d
docker-compose exec bucket-streamer cargo test

# Interactive testing
docker-compose exec bucket-streamer bash

# View logs
docker-compose logs -f bucket-streamer
```

### Performance Testing

#### Benchmark: Decoder Speed

```bash
# Run decoder benchmarks (ignored tests)
cargo test -p bucket-streamer --release -- \
  --ignored --nocapture \
  decoder

# Expected: Single frame decode: 15-30ms (50-65 FPS)
```

#### Benchmark: JPEG Encoding

```bash
cargo test -p bucket-streamer --release -- \
  --ignored --nocapture \
  encoder

# Expected: 480p @ q80: 5-10ms (100+ FPS)
```

#### Full Pipeline Benchmark

```bash
# Measure end-to-end performance
cargo run --release -p streaming-cli -- \
  --video data/test.h265 \
  --frames-file data/test.h265.offsets.json \
  --batch 1 \
  --json | jq .
```

---

## 9. QUICK START CHECKLIST

### First-Time Setup

- [ ] Clone repository
- [ ] Verify Rust installed: `rustc --version` (need 1.70+)
- [ ] Check test videos exist: `ls -lh data/test_dji_*.mp4`

### Build and Test

```bash
# 1. Format and lint
cargo fmt
cargo clippy

# 2. Build all crates
cargo build --release

# 3. Run unit tests
cargo test

# 4. Convert test video
cargo run -p repo-cli -- convert \
  --input data/test_dji_sfnight_1.mp4 \
  --output data/test.h265 \
  --extract-offsets \
  --force

# 5. Start server
cargo run --release -p bucket-streamer &

# 6. Run benchmark
cargo run -p streaming-cli -- \
  --video data/test.h265 \
  --frames-file data/test.h265.offsets.json
```

### Troubleshooting

**Build fails with FFmpeg errors:**
- Ensure FFmpeg dev libraries installed (see Dockerfile)
- Docker: `docker-compose up`

**Server won't start:**
- Check port 3000 not in use: `lsof -i :3000`
- Verify data directory exists: `mkdir -p data`

**Client can't connect:**
- Verify server running: `curl http://localhost:3000/health`
- Check WebSocket: `wscat -c ws://localhost:3000/ws`

**Frames decode failed:**
- Ensure H.265 video used: `ffprobe data/test.h265`
- Check offset file format: `cat data/test.h265.offsets.json | jq .`

---

## 10. REFERENCE: IMPORTANT FILES

| File | Purpose | Key Info |
|------|---------|----------|
| `crates/bucket-streamer/src/main.rs` | Server entry point | Config parsing, store creation, server startup |
| `crates/bucket-streamer/src/server/websocket.rs` | Session handler | Frame processing loop, message handling |
| `crates/bucket-streamer/src/pipeline/decoder.rs` | H.265 decoder | FFmpeg integration, YUV420P output |
| `crates/bucket-streamer/src/pipeline/encoder.rs` | JPEG encoder | TurboJPEG wrapper, quality control |
| `crates/bucket-streamer/src/pipeline/avio.rs` | In-memory I/O | FFmpeg AVIOContext for memory streaming |
| `crates/repo-cli/src/commands/convert.rs` | Video conversion | FFmpeg H.265 encoding, offset extraction |
| `crates/streaming-cli/src/main.rs` | Benchmark client | WebSocket client, latency tracking, FPS calculation |
| `docs/design_stage1.md` | Architecture design | High-level design, technology rationale |
| `TEST_PLAN.md` | Test suite | Comprehensive testing guide with expected results |
| `docker-compose.yml` | Docker setup | Development environment configuration |

---

## 11. NEXT STEPS FOR DEVELOPMENT

### Immediate (Stage 1 completion)
- [ ] Verify all unit tests pass
- [ ] Convert test videos with offset extraction
- [ ] Run end-to-end benchmark
- [ ] Document actual performance numbers

### Short-term (Stage 2)
- [ ] Multiple concurrent WebSocket clients
- [ ] Frame caching (LRU)
- [ ] LIFO queue optimization for scrubbing
- [ ] Connection authentication

### Future (Stage 3+)
- [ ] Kubernetes deployment
- [ ] Hardware acceleration (NVDEC)
- [ ] Prometheus metrics
- [ ] Multiple decoder pool
- [ ] WebP/AVIF output formats

---

**End of Analysis Document**
