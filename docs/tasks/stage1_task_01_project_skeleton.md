# Task 01: Project Skeleton & Docker Setup

## Goal
Create the initial Cargo workspace (3 binaries) plus a Docker-based dev environment (FFmpeg + TurboJPEG deps) and a persistent MinIO service via `docker-compose.yml`.

This task is intentionally **agent-executable**: it includes exact file contents for all stubs and a single phased command script.

## Dependencies
None — this is the foundation task.

## Policy (Stage 1)
- Rust toolchain is pinned to `stable` via `rust-toolchain.toml`.
- Dependency versions are **not** hardcoded in docs. Use `cargo add` (latest) instead.
- Keep `streaming-cli` lightweight: **no FFmpeg/TurboJPEG** deps.
- Use `docker compose` (space) as the canonical command.
- If your system only has the legacy plugin, `docker-compose` should also work.

## Files to Create

```text
bucket-streamer/
├── Cargo.toml                      # Workspace root
├── rust-toolchain.toml             # Rust toolchain pin (stable)
├── Dockerfile
├── docker-compose.yml
├── .dockerignore
├── .gitignore                      # Append data/ ignores
├── data/                           # Local dev videos (gitignored)
│   └── .gitkeep
└── crates/
    ├── bucket-streamer/
    │   ├── Cargo.toml
    │   └── src/
    │       ├── main.rs
    │       ├── config.rs
    │       ├── server/
    │       │   ├── mod.rs
    │       │   ├── router.rs
    │       │   ├── websocket.rs
    │       │   └── protocol.rs
    │       ├── pipeline/
    │       │   ├── mod.rs
    │       │   ├── session.rs
    │       │   ├── fetcher.rs
    │       │   ├── decoder.rs
    │       │   └── encoder.rs
    │       └── storage/
    │           ├── mod.rs
    │           └── backend.rs
    ├── streaming-cli/
    │   ├── Cargo.toml
    │   └── src/main.rs
    └── repo-cli/
        ├── Cargo.toml
        └── src/
            ├── main.rs
            └── commands/
                ├── mod.rs
                └── convert.rs
```

## Steps

### Phase 1: Workspace + toolchain

#### 1) Create workspace root `Cargo.toml`

Create `Cargo.toml` at repo root:

```toml
[workspace]
resolver = "2"
members = [
  "crates/bucket-streamer",
  "crates/streaming-cli",
  "crates/repo-cli",
]

[workspace.package]
version = "0.1.0"
edition = "2021"
license = "MIT"

# This is populated by `cargo add --workspace`.
[workspace.dependencies]
```

#### 2) Create `rust-toolchain.toml`

Create `rust-toolchain.toml` at repo root:

```toml
[toolchain]
channel = "stable"
profile = "minimal"
components = ["rustfmt", "clippy"]
```

### Phase 2: Crate scaffolding

#### 3) Create 3 binary crates

Use Cargo to scaffold:

```bash
cargo new crates/bucket-streamer --bin
cargo new crates/streaming-cli --bin
cargo new crates/repo-cli --bin
```

Then update each crate’s `Cargo.toml` to use workspace metadata:

```toml
[package]
name = "..."
version.workspace = true
edition.workspace = true
```

### Phase 3: Dependencies (latest via `cargo add`)

#### 4) Install `cargo-edit` (for `cargo add`)

```bash
cargo install cargo-edit
```

Optional: sanity-check crate naming with `cargo search <crate>`.

#### 5) Add workspace dependencies

Run these at repo root:

```bash
# Async runtime
cargo add tokio --workspace --features full

# Web server
cargo add axum --workspace --features ws
cargo add tower --workspace
cargo add tower-http --workspace --features trace

# Serialization
cargo add serde --workspace --features derive
cargo add serde_json --workspace

# CLI
cargo add clap --workspace --features derive

# Storage
cargo add object_store --workspace --features aws
cargo add bytes --workspace

# FFmpeg + JPEG
cargo add ffmpeg-next --workspace
cargo add ffmpeg-sys-next --workspace
cargo add turbojpeg --workspace

# Error handling
cargo add anyhow --workspace
cargo add thiserror --workspace

# Logging
cargo add tracing --workspace
cargo add tracing-subscriber --workspace --features env-filter

# WebSocket client (for streaming-cli)
cargo add tokio-tungstenite --workspace
```

Important: `cargo add --workspace` only creates shared dependency declarations in the root `Cargo.toml`. Each crate must still opt into the dependencies it uses with `dep.workspace = true`.

#### 6) Update each crate `Cargo.toml` to reference workspace deps

**`crates/bucket-streamer/Cargo.toml`**

```toml
[package]
name = "bucket-streamer"
version.workspace = true
edition.workspace = true

[dependencies]
tokio.workspace = true
axum.workspace = true
tower.workspace = true
tower-http.workspace = true
serde.workspace = true
serde_json.workspace = true
clap.workspace = true
object_store.workspace = true
bytes.workspace = true
ffmpeg-next.workspace = true
ffmpeg-sys-next.workspace = true
turbojpeg.workspace = true
anyhow.workspace = true
thiserror.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
```

**`crates/streaming-cli/Cargo.toml`**

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
```

**`crates/repo-cli/Cargo.toml`**

Note: keep `repo-cli` lighter by depending on `ffmpeg-next` only; add `ffmpeg-sys-next` later if needed.

```toml
[package]
name = "repo-cli"
version.workspace = true
edition.workspace = true

[dependencies]
tokio.workspace = true
clap.workspace = true
serde.workspace = true
serde_json.workspace = true
ffmpeg-next.workspace = true
anyhow.workspace = true
thiserror.workspace = true
tracing.workspace = true
tracing-subscriber.workspace = true
```

### Phase 4: Rust source stubs (exact contents)

#### 7) Create crate entrypoints

**`crates/bucket-streamer/src/main.rs`**

```rust
mod config;
mod pipeline;
mod server;
mod storage;

fn main() {
    println!("bucket-streamer server");
}
```

**`crates/streaming-cli/src/main.rs`**

```rust
fn main() {
    println!("streaming-cli");
}
```

**`crates/repo-cli/src/main.rs`**

```rust
mod commands;

fn main() {
    println!("repo-cli");
}
```

#### 8) Create module trees

**`crates/bucket-streamer/src/server/mod.rs`**

```rust
pub mod protocol;
pub mod router;
pub mod websocket;
```

**`crates/bucket-streamer/src/pipeline/mod.rs`**

```rust
pub mod decoder;
pub mod encoder;
pub mod fetcher;
pub mod session;
```

**`crates/bucket-streamer/src/storage/mod.rs`**

```rust
pub mod backend;
```

**`crates/repo-cli/src/commands/mod.rs`**

```rust
pub mod convert;
```

#### 9) Create placeholder files (empty stubs)

Create these empty files (they will be implemented in later tasks):

- `crates/bucket-streamer/src/config.rs`
- `crates/bucket-streamer/src/server/protocol.rs`
- `crates/bucket-streamer/src/server/router.rs`
- `crates/bucket-streamer/src/server/websocket.rs`
- `crates/bucket-streamer/src/pipeline/session.rs`
- `crates/bucket-streamer/src/pipeline/fetcher.rs`
- `crates/bucket-streamer/src/pipeline/decoder.rs`
- `crates/bucket-streamer/src/pipeline/encoder.rs`
- `crates/bucket-streamer/src/storage/backend.rs`
- `crates/repo-cli/src/commands/convert.rs`

### Phase 5: Docker dev environment

#### 10) Create `Dockerfile` (multi-stage)

```dockerfile
# Stage 1: Development base with all dependencies
FROM rust:stable-slim-bookworm AS dev

RUN apt-get update && apt-get install -y \
    libavcodec-dev \
    libavformat-dev \
    libavutil-dev \
    libswscale-dev \
    libturbojpeg0-dev \
    pkg-config \
    clang \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /workspace

CMD ["cargo", "run", "-p", "bucket-streamer"]

# Stage 2: Builder
FROM dev AS builder

COPY Cargo.toml ./
COPY crates ./crates

RUN cargo build --release -p bucket-streamer

# Stage 3: Production runtime
FROM debian:bookworm-slim AS prod

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

#### 11) Create `docker-compose.yml`

```yaml
services:
  bucket-streamer:
    build:
      context: .
      target: dev
    ports:
      - "3000:3000"
    volumes:
      - .:/workspace
      - ./data:/workspace/data
      - cargo-cache:/usr/local/cargo/registry
      - target-cache:/workspace/target
    environment:
      - RUST_LOG=debug
      - RUST_BACKTRACE=1
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
    volumes:
      - minio-data:/data

volumes:
  cargo-cache:
  target-cache:
  minio-data:
```

#### 12) Create `.dockerignore`

```text
target/
.git/
data/**
!data/.gitkeep
```

#### 13) Update `.gitignore`

Append:

```text
data/**
!data/.gitkeep
```

#### 14) Ensure `data/.gitkeep` exists

Create:
- `data/.gitkeep`

### Phase 6: Validate

#### 15) Validate locally

```bash
cargo build --workspace
cargo run -p bucket-streamer
cargo run -p repo-cli
cargo run -p streaming-cli
```

#### 16) Validate Docker + MinIO

```bash
docker compose build

# Start MinIO only (quick verification)
docker compose up -d minio

# Or start everything (normal dev workflow)
docker compose up
```

Verify MinIO console:
- http://localhost:9001
- Login: `minioadmin` / `minioadmin`

Optional: build inside container:

```bash
docker compose run --rm bucket-streamer cargo build --workspace
```

## Command Script (single linear run)

Run in repo root:

```bash
cargo install cargo-edit
mkdir -p data && touch data/.gitkeep

cargo new crates/bucket-streamer --bin
cargo new crates/streaming-cli --bin
cargo new crates/repo-cli --bin

# Create: Cargo.toml (workspace) and rust-toolchain.toml

cargo add tokio --workspace --features full
cargo add axum --workspace --features ws
cargo add tower --workspace
cargo add tower-http --workspace --features trace
cargo add serde --workspace --features derive
cargo add serde_json --workspace
cargo add clap --workspace --features derive
cargo add object_store --workspace --features aws
cargo add bytes --workspace
cargo add ffmpeg-next --workspace
cargo add ffmpeg-sys-next --workspace
cargo add turbojpeg --workspace
cargo add anyhow --workspace
cargo add thiserror --workspace
cargo add tracing --workspace
cargo add tracing-subscriber --workspace --features env-filter
cargo add tokio-tungstenite --workspace

# Edit crate Cargo.toml files to use *.workspace = true
# Create stub Rust modules and empty placeholder files
# Create Dockerfile, docker-compose.yml, .dockerignore
# Append .gitignore data/** rules

cargo build --workspace

docker compose build

docker compose up -d minio
```

## Success Criteria

- [ ] `cargo build --workspace` succeeds
- [ ] `cargo run -p bucket-streamer` runs (prints stub message)
- [ ] `cargo run -p repo-cli` runs (prints stub message)
- [ ] `cargo run -p streaming-cli` runs (prints stub message)
- [ ] `docker compose build` completes without errors
- [ ] `docker compose run --rm bucket-streamer cargo build --workspace` succeeds
- [ ] MinIO is accessible at http://localhost:9001 after `docker compose up -d minio`

## Troubleshooting (quick)

- If `cargo add` is missing: install `cargo-edit` via `cargo install cargo-edit`.
- If Docker build fails due to missing FFmpeg libs: verify the `apt-get install` list in `Dockerfile`.
- If MinIO console not reachable: run `docker compose ps` and check port mappings for `9001`.
