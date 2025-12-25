# Bucket-Streamer Quick Reference

## Project Structure at a Glance

```
bucket-streamer/
├── crates/
│   ├── bucket-streamer/          Server binary
│   │   └── src/
│   │       ├── main.rs           → server startup
│   │       ├── config.rs         → CLI configuration
│   │       ├── server/           → HTTP/WebSocket handlers
│   │       ├── pipeline/         → Decode/encode pipeline
│   │       └── storage/          → S3/LocalFS abstraction
│   ├── repo-cli/                 Development utilities
│   │   └── src/
│   │       ├── main.rs
│   │       └── commands/convert  → MP4 to H.265 converter
│   └── streaming-cli/            Benchmark client
│       └── src/main.rs          → WebSocket benchmark
├── data/                          Test data (7x MP4 files, ~5.5GB)
├── docs/                          Design documentation
├── Cargo.toml                     Workspace config
├── docker-compose.yml            Dev environment setup
├── ANALYSIS.md                    Comprehensive analysis (THIS PROJECT)
├── TEST_PLAN.md                  Integration test plan
└── AGENTS.md                      Development guidelines
```

## Start Server (3 ways)

### 1. Docker (Recommended)
```bash
docker-compose up
```
Starts: server on :3000, MinIO S3 mock on :9000

### 2. Local Debug Build
```bash
cargo run -p bucket-streamer
```
Uses: local storage (./data), debug logging

### 3. Local Release Build (Optimized)
```bash
cargo build --release
./target/release/bucket-streamer \
  --listen-addr 0.0.0.0:3000 \
  --local-path ./data \
  --jpeg-quality 85
```

## CLI Commands

### Convert Video (repo-cli)
```bash
# Basic conversion
cargo run -p repo-cli -- convert \
  --input data/test_dji_sfnight_1.mp4 \
  --output data/test.h265 \
  --force

# With offset extraction (for streaming-cli)
cargo run -p repo-cli -- convert \
  --input data/test_dji_sfnight_1.mp4 \
  --output data/test.h265 \
  --extract-offsets \
  --force
```

### Benchmark Streaming (streaming-cli)
```bash
# Sequential (per-frame latency)
cargo run -p streaming-cli -- \
  --video data/test.h265 \
  --frames-file data/test.h265.offsets.json

# Batched (throughput test)
cargo run -p streaming-cli -- \
  --video data/test.h265 \
  --frames-file data/test.h265.offsets.json \
  --batch 10

# Save frames as JPEGs
cargo run -p streaming-cli -- \
  --video data/test.h265 \
  --frames-file data/test.h265.offsets.json \
  --output data/out/frames

# JSON output
cargo run -p streaming-cli -- \
  --video data/test.h265 \
  --frames-file data/test.h265.offsets.json \
  --json > results.json
```

## WebSocket Protocol

### Client → Server
```json
// Set video source
{"type": "SetVideo", "path": "data/test.h265"}

// Request frames
{"type": "RequestFrames", "frames": [
  {"offset": 1024, "irap_offset": 1024, "index": 0},
  {"offset": 2048, "irap_offset": 1024, "index": 1}
]}
```

### Server → Client
```json
// Video acknowledged
{"type": "VideoSet", "path": "data/test.h265", "ok": true}

// Frame metadata + binary JPEG follows
{"type": "Frame", "index": 0, "offset": 1024, "size": 45230}

// Error response
{"type": "FrameError", "index": 0, "offset": 1024, "error": "decode_failed"}
```

## Key Configuration

```bash
# Environment Variables (or --flag arguments)
LISTEN_ADDR=0.0.0.0:3000          # Server bind address
STORAGE_BACKEND=local              # local or s3
LOCAL_PATH=./data                  # Local directory
JPEG_QUALITY=80                    # 1-100
RUST_LOG=info                      # Logging level

# S3 Configuration
STORAGE_BACKEND=s3
S3_BUCKET=my-bucket
S3_REGION=us-east-1
S3_ENDPOINT=http://localhost:9000  # For MinIO
S3_ACCESS_KEY=minioadmin
S3_SECRET_KEY=minioadmin
```

## Performance Expectations

| Scenario | FPS | Latency | Notes |
|----------|-----|---------|-------|
| 480p Sequential | 30-50 | 20-40ms | Per-frame |
| 720p Sequential | 20-30 | 35-50ms | Per-frame |
| 1080p Sequential | 10-20 | 50-100ms | Per-frame |
| Batch Mode | 2-3x higher | Varies | Depends on batch size |

## Test Workflows

### Unit Tests
```bash
cargo test                              # All tests
cargo test -p bucket-streamer          # Single crate
cargo test -- --ignored --nocapture    # Benchmarks
cargo test -- --nocapture              # Show output
```

### Full Integration Test
```bash
# 1. Convert video
cargo run -p repo-cli -- convert \
  --input data/test_dji_sfnight_1.mp4 \
  --output data/test.h265 \
  --extract-offsets --force

# 2. Start server
cargo run -p bucket-streamer &
sleep 2

# 3. Run benchmark
cargo run -p streaming-cli -- \
  --video data/test.h265 \
  --frames-file data/test.h265.offsets.json \
  --batch 5

# 4. Verify results
file data/out/frames/frame_*.jpg  # Check JPEG format
```

## Code Quality

```bash
cargo fmt              # Format all code
cargo fmt --check     # Verify formatting
cargo clippy          # Lint check
cargo clippy --fix    # Auto-fix warnings
```

## Endpoints

| Endpoint | Method | Purpose |
|----------|--------|---------|
| `/health` | GET | Server health check |
| `/ws` | GET/WebSocket | Frame streaming WebSocket |

Health check:
```bash
curl http://localhost:3000/health
# Response: "ok"
```

## Test Data

**Location:** `./data/`

**Available:**
- 7 large DJI drone MP4 files (5.5GB total)
- 1 sample H.265 converted file (2.6MB)

**Generate more:**
```bash
cargo run -p repo-cli -- convert \
  --input data/test_dji_sfnight_2.mp4 \
  --output data/test2.h265 \
  --extract-offsets --force

# For faster testing (quarter resolution)
cargo run -p repo-cli -- convert \
  --input data/test_dji_sfnight_1.mp4 \
  --output data/test_small.h265 \
  --downscale 4 \
  --extract-offsets --force
```

## Troubleshooting

| Problem | Solution |
|---------|----------|
| Port 3000 in use | `lsof -i :3000` or use `--listen-addr` |
| FFmpeg not found | Use Docker or install dev libraries |
| Can't convert video | Ensure input is MP4, not H.265 |
| Server won't connect | Check `curl http://localhost:3000/health` |
| Frames decode failed | Verify video is H.265 and offset file is valid |
| Out of memory | Use smaller video or downscale option |

## Useful Commands

```bash
# Check server is responding
curl -I http://localhost:3000/health

# Test WebSocket connection
wscat -c ws://localhost:3000/ws

# View offset file format
cat data/test.h265.offsets.json | jq .

# Count frames in video
cat data/test.h265.offsets.json | jq '.frames | length'

# Extract benchmark results
cargo run -p streaming-cli -- \
  --video data/test.h265 \
  --frames-file data/test.h265.offsets.json \
  --json | jq '{fps: .average_fps, p95_latency: .latency_p95_ms, bytes: .total_bytes}'

# Find all H.265 files
find data -name "*.h265" -ls

# Monitor server logs (Docker)
docker-compose logs -f bucket-streamer

# Interactive Docker shell
docker-compose exec bucket-streamer bash
```

## Important Files Reference

| File | Purpose |
|------|---------|
| `ANALYSIS.md` | Comprehensive project analysis (11 sections) |
| `TEST_PLAN.md` | Integration test plan with expected results |
| `AGENTS.md` | Development guidelines and conventions |
| `docs/design_stage1.md` | Architecture design document |
| `crates/bucket-streamer/src/main.rs` | Server startup code |
| `crates/bucket-streamer/src/server/websocket.rs` | Frame processing |
| `crates/bucket-streamer/src/pipeline/decoder.rs` | H.265 decoding |
| `crates/repo-cli/src/commands/convert.rs` | Video conversion |
| `crates/streaming-cli/src/main.rs` | Benchmark client |

## Next Steps

1. Read `ANALYSIS.md` for comprehensive documentation
2. Run `cargo test` to verify project builds
3. Convert a video: `cargo run -p repo-cli -- convert --input data/test_dji_sfnight_1.mp4 --output data/test.h265 --extract-offsets --force`
4. Start server: `cargo run -p bucket-streamer`
5. Run benchmark: `cargo run -p streaming-cli -- --video data/test.h265 --frames-file data/test.h265.offsets.json`

---

For full details, see `ANALYSIS.md`
