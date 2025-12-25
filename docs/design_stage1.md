# Stage 1 Prototype Design - Bucket Streamer

## 1. Goals & Scope

### Stage 1 Objective
Build a minimal, working prototype that proves the core frame extraction pipeline:
- Load H.265 video data from storage using byte offsets
- Decode specific frames using FFmpeg with in-memory streaming
- Encode frames as JPEG and deliver via WebSocket
- Achieve ≥20 FPS for sequential frame requests

### In Scope
- Single WebSocket client connection
- Local filesystem storage (with `object_store` abstraction for S3 swap)
- MP4 container with H.265/HEVC video track
- JPEG output format
- CLI configuration with sensible defaults
- Summary benchmark statistics

### Out of Scope (Future Stages)
- Multiple concurrent clients
- Shared channels across sessions
- Frame caching / preloading
- LIFO queue optimization (using FIFO for Stage 1)
- Kubernetes deployment
- Hardware acceleration (NVDEC)

---

## 2. Technology Decisions

### Web Framework: Axum
**Version:** `0.8.x`

**Rationale:**
- Simpler mental model than Actix-web, faster team onboarding
- Native WebSocket support via `ws` feature
- HTTP/2 support included for potential future SSE streaming
- Backed by tokio-rs team with strong community momentum
- Performance comparable to Actix under typical loads
- Better testing ergonomics

**Trade-off:** Actix-web has slightly better raw throughput at extreme concurrency, but complexity cost not justified for Stage 1.

### Storage Abstraction: `object_store`
**Version:** `0.12.x` (latest stable)

**Rationale:**
- Unified API across local filesystem and S3
- Supports byte-range requests via `get_range()` - critical for IRAP-to-frame fetching
- Apache Arrow project, well-maintained
- Runtime backend switching with zero code changes

```rust
use object_store::{ObjectStore, local::LocalFileSystem, aws::AmazonS3Builder};
use std::sync::Arc;

pub fn create_store(backend: &str, config: &StorageConfig) -> Arc<dyn ObjectStore> {
    match backend {
        "local" => Arc::new(LocalFileSystem::new_with_prefix(&config.local_path).unwrap()),
        "s3" => Arc::new(
            AmazonS3Builder::new()
                .with_bucket_name(&config.s3_bucket)
                .with_region(&config.s3_region)
                .build()
                .unwrap()
        ),
        _ => panic!("Unknown storage backend: {}", backend),
    }
}
```

### Image Encoding: TurboJPEG
**Version:** `turbojpeg 1.x`

**Rationale:**
- 1.5-2x faster than alternatives (critical for 20+ FPS target)
- Can accept YUV data directly from decoder (skip RGB conversion)
- Mature crate with good documentation
- JPEG has universal browser support with hardware decode

**Alternatives considered:**
- WebP: Better compression but slower encoding
- Raw frames + WebCodecs: See `docs/idea_webcodecs_raw_frames.md`

### FFmpeg Integration: `ffmpeg-next` + `ffmpeg-sys-next`
**Rationale:**
- `ffmpeg-next` for high-level safe wrappers (Packet, Frame, Decoder)
- Drop to `ffmpeg-sys-next` for custom `AVIOContext` (in-memory streaming)
- FFmpeg handles both MP4 demuxing and H.265 decoding
- See `docs/idea_pure_rust_demux.md` for alternative demux approach

### Mock S3: MinIO
**Rationale:**
- Best S3 API compatibility (321 vs 56 tests passed vs SeaweedFS)
- Byte-range requests work correctly - critical for video streaming
- Simple single-binary Docker setup
- Good Web UI for uploading test videos

**Note:** MinIO OSS is in "maintenance mode" - fine for dev. We're storage-agnostic via `object_store` so migration is trivial.

```bash
# Local dev setup
docker run -p 9000:9000 -p 9001:9001 \
  -e MINIO_ROOT_USER=minioadmin \
  -e MINIO_ROOT_PASSWORD=minioadmin \
  minio/minio server /data --console-address ":9001"
```

### CLI Framework: Clap + Serde
**Rationale:**
- `clap` derive macros for ergonomic CLI definition
- Serde integration for future config file support
- Sensible defaults for local development

---

## 3. Architecture Overview

**Deployment Context:** All components run in Docker containers for Stage 1 development.

```
Docker Host
│
├─ bucket-streamer container (Rust server with FFmpeg/TurboJPEG)
│  └─ Port 3000 → Host
│
├─ minio container (S3-compatible storage)
│  └─ Ports 9000, 9001 → Host
│
└─ Shared Docker network for service discovery

┌─────────────────────────────────────────────────────────────────────┐
│                         Streaming Server                            │
├─────────────────────────────────────────────────────────────────────┤
│                                                                     │
│  ┌──────────────────┐     ┌────────────────────────────────────┐   │
│  │    Axum Server   │     │       Pipeline (tokio tasks)       │   │
│  │                  │     │                                    │   │
│  │  GET /health ────┼────▶│  Health check                      │   │
│  │                  │     │                                    │   │
│  │  WS /ws ─────────┼────▶│  Session Handler                   │   │
│  │       │          │     │       │                            │   │
│  │       │          │     │       ▼                            │   │
│  │       │          │     │  ┌─────────────────────────────┐   │   │
│  │       │          │     │  │     Frame Request Queue     │   │   │
│  │       │          │     │  │         (FIFO)              │   │   │
│  │       │          │     │  └─────────────┬───────────────┘   │   │
│  │       │          │     │                │                   │   │
│  │       │          │     │                ▼                   │   │
│  │       │          │     │  ┌─────────────────────────────┐   │   │
│  │       │          │     │  │    Storage Fetcher          │   │   │
│  │       │          │     │  │    (object_store)           │   │   │
│  │       │          │     │  │    - Byte-range GET         │   │   │
│  │       │          │     │  └─────────────┬───────────────┘   │   │
│  │       │          │     │                │                   │   │
│  │       │          │     │                ▼                   │   │
│  │       │          │     │  ┌─────────────────────────────┐   │   │
│  │       │          │     │  │    H.265 Decoder            │   │   │
│  │       │          │     │  │    (ffmpeg-next)            │   │   │
│  │       │          │     │  │    - Custom AVIOContext     │   │   │
│  │       │          │     │  │    - Persistent context     │   │   │
│  │       │          │     │  └─────────────┬───────────────┘   │   │
│  │       │          │     │                │                   │   │
│  │       │          │     │                ▼                   │   │
│  │       │          │     │  ┌─────────────────────────────┐   │   │
│  │       │          │     │  │    JPEG Encoder             │   │   │
│  │       │          │     │  │    (turbojpeg)              │   │   │
│  │       │          │     │  │    - YUV direct input       │   │   │
│  │       │          │     │  └─────────────┬───────────────┘   │   │
│  │       │          │     │                │                   │   │
│  │       ◀──────────┼─────┼────────────────┘                   │   │
│  │  WS Send         │     │                                    │   │
│  └──────────────────┘     └────────────────────────────────────┘   │
│                                                                     │
│  ┌─────────────────────────────────────────────────────────────┐   │
│  │                      Session State                           │   │
│  │  - Decoder context (persistent, flushed between seeks)      │   │
│  │  - Current video path                                        │   │
│  │  - Frame request queue                                       │   │
│  └─────────────────────────────────────────────────────────────┘   │
└─────────────────────────────────────────────────────────────────────┘
                                    │
                                    ▼
                    ┌───────────────────────────────┐
                    │   Object Store                │
                    │   - LocalFileSystem (dev)     │
                    │   - AmazonS3 (prod)           │
                    └───────────────────────────────┘
```

---

## 4. Critical Path: AVIOContext In-Memory Streaming

This is the highest-risk component and should be validated early via a spike.

### Problem
FFmpeg typically expects file paths or file descriptors. We need to feed it bytes directly from memory (fetched via `object_store`) to avoid disk I/O overhead.

### Solution: Custom AVIOContext
FFmpeg's `AVIOContext` allows custom read/seek callbacks. We implement these to read from a `Bytes` buffer.

### Implementation Pattern

```rust
use bytes::Bytes;
use ffmpeg_sys_next::{
    self as ffi,
    AVIOContext, AVFormatContext,
    avio_alloc_context, avformat_open_input, avformat_close_input,
    AVSEEK_SIZE, AVIO_FLAG_READ,
};
use std::ffi::c_void;
use std::os::raw::{c_int, c_uchar};
use std::ptr;

/// Holds the video data and current read position
pub struct InMemoryIO {
    data: Bytes,
    position: usize,
}

impl InMemoryIO {
    pub fn new(data: Bytes) -> Self {
        Self { data, position: 0 }
    }
}

/// FFmpeg read callback - called when decoder needs more data
unsafe extern "C" fn read_packet(
    opaque: *mut c_void,
    buf: *mut c_uchar,
    buf_size: c_int,
) -> c_int {
    if opaque.is_null() || buf.is_null() {
        return ffi::AVERROR_EOF;
    }

    let io = &mut *(opaque as *mut InMemoryIO);
    let remaining = io.data.len().saturating_sub(io.position);
    
    if remaining == 0 {
        return ffi::AVERROR_EOF;
    }

    let to_read = std::cmp::min(remaining, buf_size as usize);
    ptr::copy_nonoverlapping(
        io.data[io.position..].as_ptr(),
        buf,
        to_read,
    );
    io.position += to_read;
    to_read as c_int
}

/// FFmpeg seek callback - allows random access within the buffer
unsafe extern "C" fn seek(
    opaque: *mut c_void,
    offset: i64,
    whence: c_int,
) -> i64 {
    if opaque.is_null() {
        return -1;
    }

    let io = &mut *(opaque as *mut InMemoryIO);
    
    // AVSEEK_SIZE: FFmpeg asking for total size
    if whence == AVSEEK_SIZE {
        return io.data.len() as i64;
    }

    let new_pos = match whence {
        libc::SEEK_SET => offset as usize,
        libc::SEEK_CUR => io.position.wrapping_add(offset as usize),
        libc::SEEK_END => io.data.len().wrapping_add(offset as usize),
        _ => return -1,
    };

    if new_pos > io.data.len() {
        return -1;
    }

    io.position = new_pos;
    new_pos as i64
}

/// Create an AVIOContext that reads from our in-memory buffer
/// 
/// # Safety
/// - The returned AVIOContext holds a raw pointer to `io`
/// - `io` must outlive the AVIOContext
/// - Caller must call `avio_context_free` when done
pub unsafe fn create_avio_context(
    io: *mut InMemoryIO,
    buffer_size: usize,
) -> *mut AVIOContext {
    let buffer = ffi::av_malloc(buffer_size) as *mut c_uchar;
    if buffer.is_null() {
        return ptr::null_mut();
    }

    avio_alloc_context(
        buffer,
        buffer_size as c_int,
        0,                          // write_flag = 0 (read-only)
        io as *mut c_void,          // opaque pointer to our data
        Some(read_packet),          // read callback
        None,                       // write callback (not needed)
        Some(seek),                 // seek callback
    )
}
```

### Spike Task Checklist
- [ ] Create minimal binary that loads MP4 bytes into memory
- [ ] Set up custom AVIOContext with read/seek callbacks
- [ ] Successfully demux and decode one frame
- [ ] Verify no memory leaks with valgrind
- [ ] Measure latency vs file-based approach
- [ ] Document any FFmpeg version-specific issues

### Key Risks
1. **Lifetime management**: `InMemoryIO` must outlive `AVIOContext`
2. **Thread safety**: If decoder is used across tasks, need `Send`/`Sync` wrapper
3. **Error handling**: FFmpeg errors are integers, need proper translation
4. **Cleanup**: Must free AVIOContext buffer and context on drop

---

## 5. Module Structure

```
src/
├── main.rs                 # Entry point, CLI parsing, server startup
├── config.rs               # Configuration structs (Clap + Serde)
│
├── server/
│   ├── mod.rs              # Server module exports
│   ├── router.rs           # Axum router setup
│   ├── websocket.rs        # WebSocket upgrade handler, session loop
│   └── protocol.rs         # Message types (ClientMessage, ServerMessage)
│
├── pipeline/
│   ├── mod.rs              # Pipeline module exports
│   ├── session.rs          # Per-session state and frame queue
│   ├── fetcher.rs          # object_store byte-range fetching
│   ├── decoder.rs          # FFmpeg H.265 decode + AVIOContext
│   └── encoder.rs          # TurboJPEG encoding
│
└── storage/
    ├── mod.rs              # Storage module exports
    └── backend.rs          # object_store abstraction, backend switching
```

### Key Types

```rust
// config.rs
#[derive(Parser, Debug, Serialize, Deserialize)]
#[command(name = "bucket-streamer")]
pub struct Config {
    /// Server listen address
    #[arg(long, default_value = "0.0.0.0:3000")]
    pub listen_addr: String,

    /// Storage backend: "local" or "s3"
    #[arg(long, default_value = "local")]
    pub storage_backend: String,

    /// Local storage path (when using local backend)
    #[arg(long, default_value = "./data")]
    pub local_path: String,

    /// S3 bucket name (when using s3 backend)
    #[arg(long, default_value = "")]
    pub s3_bucket: String,

    /// S3 region (when using s3 backend)
    #[arg(long, default_value = "us-east-1")]
    pub s3_region: String,

    /// JPEG encoding quality (1-100)
    #[arg(long, default_value = "80")]
    pub jpeg_quality: u8,
}

// server/protocol.rs
#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ClientMessage {
    SetVideo { path: String },
    RequestFrames {
        irap_offset: u64,
        frames: Vec<FrameRequest>,
    },
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FrameRequest {
    pub offset: u64,
    pub index: u32,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum ServerMessage {
    VideoSet { path: String, ok: bool },
    Frame { index: u32, offset: u64, size: u32 },
    FrameError { index: u32, offset: u64, error: String },
    Error { message: String },
}

// pipeline/session.rs
pub struct Session {
    pub video_path: Option<String>,
    pub decoder: Option<DecoderContext>,
    pub frame_queue: VecDeque<FrameRequest>,
}
```

---

## 6. WebSocket Protocol

### Connection Flow

```
Client                                  Server
   │                                       │
   │──── WS Connect /ws ──────────────────▶│
   │                                       │
   │◀─── Connection Established ───────────│
   │                                       │
   │──── SetVideo { path: "..." } ────────▶│
   │                                       │
   │◀─── VideoSet { ok: true } ────────────│
   │                                       │
   │──── RequestFrames { ... } ───────────▶│
   │                                       │
   │◀─── Frame { index, offset, size } ────│
   │◀─── [Binary: JPEG data] ──────────────│
   │                                       │
   │◀─── Frame { index, offset, size } ────│
   │◀─── [Binary: JPEG data] ──────────────│
   │                                       │
   │       ... or on decode failure ...    │
   │                                       │
   │◀─── FrameError { index, error } ──────│
   │                                       │
```

### Message Types

#### Client → Server

```json
// Set video source (required before requesting frames)
{
    "type": "SetVideo",
    "path": "videos/robot_cam_001.mp4"
}

// Request frames by byte offset
{
    "type": "RequestFrames",
    "irap_offset": 1000,
    "frames": [
        { "offset": 1500, "index": 0 },
        { "offset": 2100, "index": 1 },
        { "offset": 2800, "index": 2 }
    ]
}
```

#### Server → Client

```json
// Video source acknowledged
{
    "type": "VideoSet",
    "path": "videos/robot_cam_001.mp4",
    "ok": true
}

// Frame metadata (binary JPEG follows immediately)
{
    "type": "Frame",
    "index": 0,
    "offset": 1500,
    "size": 45230
}

// Frame decode/encode error (client can show placeholder)
{
    "type": "FrameError",
    "index": 5,
    "offset": 2800,
    "error": "decode_failed"
}

// General error
{
    "type": "Error",
    "message": "Video not found: videos/missing.mp4"
}
```

### Binary Frame Delivery
After each `Frame` JSON message, the server sends a binary WebSocket message containing the raw JPEG bytes. The client matches this to the preceding `Frame` message by order.

---

## 7. Development Environment Setup

### Recommended: Docker Container Development

**Rationale:** FFmpeg and TurboJPEG have complex system dependencies. Running in Docker ensures:
- Consistent environment across all developers
- No manual library installation or version conflicts
- Same environment for development and eventual deployment
- Lightweight base image (Debian slim with only required libs)

### Dockerfile

Create a multi-stage `Dockerfile` at the project root:

```dockerfile
# Stage 1: Development base with all dependencies
FROM rust:1.75-slim-bookworm AS dev

# Install FFmpeg and TurboJPEG development libraries
RUN apt-get update && apt-get install -y \
    libavcodec-dev \
    libavformat-dev \
    libavutil-dev \
    libswscale-dev \
    libturbojpeg0-dev \
    pkg-config \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /workspace

# Development mode: source is mounted as volume
CMD ["cargo", "run"]

# Stage 2: Production builder
FROM dev AS builder

COPY Cargo.toml Cargo.lock ./
COPY src ./src

RUN cargo build --release

# Stage 3: Production runtime (minimal)
FROM debian:bookworm-slim AS prod

# Install only runtime libraries (no -dev packages)
RUN apt-get update && apt-get install -y \
    libavcodec59 \
    libavformat59 \
    libavutil57 \
    libswscale6 \
    libturbojpeg0 \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /workspace/target/release/bucket-streamer /usr/local/bin/

EXPOSE 3000

CMD ["bucket-streamer"]
```

### Docker Compose Setup

Create `docker-compose.yaml` at the project root:

```yaml
version: '3.8'

services:
  bucket-streamer:
    build:
      context: .
      target: dev
    ports:
      - "3000:3000"
    volumes:
      # Mount source code for hot reload
      - .:/workspace
      # Mount local video files
      - ./data:/workspace/data
      # Cache Cargo dependencies
      - cargo-cache:/usr/local/cargo/registry
    environment:
      - RUST_LOG=debug
      - STORAGE_BACKEND=local
      - LOCAL_PATH=/workspace/data
    depends_on:
      - minio
    stdin_open: true
    tty: true

  minio:
    image: minio/minio:latest
    ports:
      - "9000:9000"
      - "9001:9001"
    environment:
      MINIO_ROOT_USER: minioadmin
      MINIO_ROOT_PASSWORD: minioadmin
    command: server /data --console-address ":9001"
    # Ephemeral storage (no named volume - resets on restart)

volumes:
  cargo-cache:
```

### Quick Start

```bash
# 1. Create data directory for local video files
mkdir -p data

# 2. Build and start all services
docker-compose up

# 3. In another terminal: build the project
docker-compose run --rm bucket-streamer cargo build

# 4. Run the server with custom args
docker-compose run --rm -p 3000:3000 bucket-streamer \
  cargo run -- --storage-backend local --local-path /workspace/data

# 5. Run tests
docker-compose run --rm bucket-streamer cargo test

# 6. Shell into container for debugging
docker-compose run --rm bucket-streamer bash

# 7. Production build (creates optimized binary)
docker build --target prod -t bucket-streamer:latest .
docker run -p 3000:3000 bucket-streamer:latest

# 8. Stop all services
docker-compose down
```

### Uploading Test Videos to MinIO

```bash
# MinIO web console available at http://localhost:9001
# Login: minioadmin / minioadmin
# Create a bucket and upload your MP4 files

# Or use MinIO CLI
docker run --rm --network host minio/mc \
  alias set local http://localhost:9000 minioadmin minioadmin
docker run --rm --network host minio/mc \
  mb local/videos
docker run --rm --network host -v $(pwd)/data:/data minio/mc \
  cp /data/test.mp4 local/videos/
```

### Alternative: Local Installation (Optional)

<details>
<summary><b>Ubuntu/Debian Setup</b> (click to expand)</summary>

Use this if Docker isn't available or you prefer local development.

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# FFmpeg and TurboJPEG
sudo apt-get update
sudo apt-get install -y \
    libavcodec-dev libavformat-dev libavutil-dev libswscale-dev \
    libturbojpeg0-dev pkg-config

# Verify FFmpeg
pkg-config --modversion libavcodec

# Start MinIO (optional, for S3 testing)
docker run -d --name minio \
    -p 9000:9000 -p 9001:9001 \
    -e MINIO_ROOT_USER=minioadmin \
    -e MINIO_ROOT_PASSWORD=minioadmin \
    minio/minio server /data --console-address ":9001"

# Build and run
cargo build
cargo run
```

</details>

<details>
<summary><b>macOS Setup</b> (click to expand)</summary>

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# FFmpeg and TurboJPEG
brew install ffmpeg jpeg-turbo pkg-config

# Set library paths if needed
export PKG_CONFIG_PATH="/opt/homebrew/opt/jpeg-turbo/lib/pkgconfig:$PKG_CONFIG_PATH"

# Start MinIO (optional)
docker run -d --name minio \
    -p 9000:9000 -p 9001:9001 \
    -e MINIO_ROOT_USER=minioadmin \
    -e MINIO_ROOT_PASSWORD=minioadmin \
    minio/minio server /data --console-address ":9001"

# Build and run
cargo build
cargo run
```

</details>

### Initial Cargo.toml Dependencies
```toml
[package]
name = "bucket-streamer"
version = "0.1.0"
edition = "2024"

[dependencies]
# Async runtime
tokio = { version = "1", features = ["full"] }

# Web framework
axum = { version = "0.8", features = ["ws"] }
tower = "0.5"
tower-http = { version = "0.6", features = ["trace"] }

# Serialization
serde = { version = "1", features = ["derive"] }
serde_json = "1"

# CLI
clap = { version = "4", features = ["derive"] }

# Storage
object_store = { version = "0.12", features = ["aws"] }
bytes = "1"

# FFmpeg
ffmpeg-next = "7"
ffmpeg-sys-next = "7"

# JPEG encoding
turbojpeg = "1"

# Error handling
anyhow = "1"
thiserror = "2"

# Logging
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

[dev-dependencies]
tokio-tungstenite = "0.24"
```

---

## 8. Success Criteria

### Functional Requirements
1. [ ] Client connects via WebSocket to `/ws`
2. [ ] Client sets video source with `SetVideo` message
3. [ ] Client requests frames with `RequestFrames` message
4. [ ] Server returns JPEG frames via WebSocket binary messages
5. [ ] Failed frames return `FrameError` (don't crash session)
6. [ ] Decoder context is reused across frame requests (not recreated)

### Performance Requirements
1. [ ] Achieve ≥20 FPS for sequential frame requests on local storage
2. [ ] Decoder initialization happens once per video, not per frame
3. [ ] No intermediate disk writes (in-memory AVIOContext working)

### Benchmark Output
The streaming-cli should output summary stats:
```
=== Benchmark Results ===
Frames requested: 100
Frames received:  98
Frames errored:   2
Total time:       4.2s
Average FPS:      23.3
Avg latency:      42.8ms
Min latency:      31.2ms
Max latency:      89.4ms
P95 latency:      67.1ms
```

---

## 9. Known Limitations

### Stage 1 Constraints
- **Single client only**: No concurrent WebSocket sessions
- **No caching**: Every frame request hits storage + decode + encode
- **FIFO queue**: No priority/LIFO optimization for scrubbing
- **Local storage focus**: S3 works but not load-tested
- **No auth**: WebSocket endpoint is open

### Technical Debt for Future
- Error types are stringly-typed in protocol (should be enums)
- No graceful shutdown handling
- No connection timeout/keepalive configuration
- Decoder pool not implemented (1:1 session:decoder)

---

## 10. Future Work

### Stage 2 Candidates
- Multiple concurrent clients with shared channels
- Frame caching (in-memory LRU)
- LIFO queue for scrubbing optimization
- Connection authentication

### Stage 3 Candidates  
- Kubernetes deployment with horizontal scaling
- Hardware acceleration (NVDEC) - see research doc
- Metrics and observability (Prometheus)

### Research Parking Lot
- `docs/idea_webcodecs_raw_frames.md` - Client-side decode with WebCodecs
- `docs/idea_pure_rust_demux.md` - Pure Rust MP4 demuxing trade-offs

---

## Appendix: References

- [FFmpeg AVIOContext Documentation](https://ffmpeg.org/doxygen/trunk/structAVIOContext.html)
- [object_store crate](https://docs.rs/object_store)
- [turbojpeg crate](https://docs.rs/turbojpeg)
- [Axum WebSocket example](https://github.com/tokio-rs/axum/tree/main/examples/websockets)
- [Research: Fast FFmpeg](./research_fast_ffmpeg.md)
