# Agent Guidelines for bucket-streamer

## Project Overview
This is a Rust-based video streaming server that responds to requests for individual frames from S3-hosted H265 video files. Key components include a WebSocket server, async event pipeline, FFmpeg integration, and Kubernetes deployment support.

## Development Commands

### Building
```bash
cargo build              # Debug build
cargo build --release    # Release build
cargo check              # Quick compile check
```

### Testing
```bash
cargo test               # Run all tests
cargo test <test_name>   # Run specific test
cargo test -- --nocapture  # Run tests with stdout output
cargo test -- --ignored  # Run ignored tests
```

### Linting and Formatting
```bash
cargo fmt                # Format code
cargo fmt --check        # Check formatting
cargo clippy             # Run linter
cargo clippy --fix       # Auto-fix clippy warnings
cargo clippy -- -D warnings  # Treat warnings as errors
```

### Running
```bash
cargo run                # Run debug binary
cargo run --release      # Run release binary
```

## Code Style Guidelines

### Imports and Module Organization
- Group imports by standard library, external crates, and local modules
- Use `use` statements consistently at the top of files
- Prefer absolute paths (`crate::module::item`) over relative paths
- Example:
  ```rust
  use std::sync::Arc;
  
  use tokio::sync::mpsc;
  use axum::extract::WebSocket;
  
  use crate::channel::Channel;
  use crate::pipeline::events::Event;
  ```

### Naming Conventions
- **Types/Structs/Enums**: `PascalCase` - `WebSocketSession`, `FrameDecoder`
- **Functions/Methods**: `snake_case` - `decode_frame`, `send_to_client`
- **Constants**: `SCREAMING_SNAKE_CASE` - `MAX_FRAME_SIZE`
- **Modules**: `snake_case` - `websocket`, `pipeline`
- **Private fields**: `snake_case` with optional trailing underscore if name shadows type - `decoder: Decoder`, `client: Client_`

### Error Handling
- Use `Result<T, E>` for fallible operations
- Define domain-specific error types using `thiserror` or `anyhow`
- Use `?` operator for error propagation
- Use `context()` from `anyhow` to add error context
- Example:
  ```rust
  use anyhow::{Context, Result};
  
  pub fn load_frame(&self, offset: u64) -> Result<Vec<u8>> {
      let data = s3_client
          .get_object(offset)
          .await
          .context("Failed to load frame from S3")?;
      Ok(data)
  }
  ```

### Async Code
- Use `tokio` runtime for async operations
- Always mark async functions with `.await` on awaitable expressions
- Use `tokio::sync` for concurrency primitives (channels, mutexes, rwlocks)
- Prefer `tokio::sync::mpsc` channels for async message passing
- Example:
  ```rust
  use tokio::sync::mpsc;
  
  pub async fn process_frames(mut receiver: mpsc::Receiver<FrameRequest>) {
      while let Some(request) = receiver.recv().await {
          let frame = decode_frame(request).await?;
          sender.send(frame).await?;
      }
  }
  ```

### Structs and Traits
- Derive common traits: `Debug`, `Clone` where appropriate
- Use `#[derive(Serialize, Deserialize)]` with `serde` for types sent over WebSocket
- Use builder pattern for complex struct initialization
- Implement `Default` for structs with sensible defaults

### Documentation
- Use `///` for public API documentation
- Include examples for non-trivial functions
- Document error conditions
- Example:
  ```rust
  /// Decodes a frame at the specified byte offset.
  ///
  /// # Errors
  /// Returns an error if the frame data is corrupt or the decoder fails.
  ///
  /// # Example
  /// ```
  /// let frame = decoder.decode_frame(offset).await?;
  /// ```
  pub async fn decode_frame(&self, offset: u64) -> Result<Frame>;
  ```

### Memory and Performance
- Use `Arc<T>` for shared ownership across async tasks
- Prefer `Bytes` or `Vec<u8>` for binary data (from `bytes` crate)
- Avoid blocking operations in async contexts
- Use `tokio::task::spawn_blocking` for CPU-intensive operations
- Consider zero-copy strategies where possible for video data

## Architecture Guidelines

### WebSocket Communication
- Use `tokio-tungstenite` or Axum WebSocket extractors
- Implement message types as structs with `Serialize/Deserialize`
- Use JSON for WebSocket payloads initially, consider binary formats later

### Pipeline Design
- Use async channels (`mpsc`, `broadcast`) for event passing
- Separate concerns: WebSocket handling, frame decoding, encoding, sending
- Implement LIFO queues for frame requests as per design notes

### FFmpeg Integration
- Use `ffmpeg-sys` or higher-level wrappers
- Create one decoder per video clip to avoid initialization overhead
- Cache decoded frames where performance is critical

## Testing Strategy
- Write unit tests for individual functions
- Use `tokio::test` for async test functions
- Mock external dependencies (S3, FFmpeg) in tests
- Integration tests for WebSocket communication flows

## Commit Convention
- Conventional commits: `feat:`, `fix:`, `refactor:`, `docs:`, `test:`, `chore:`
- Examples:
  - `feat: implement frame decoding pipeline`
  - `fix: handle WebSocket connection errors`
  - `refactor: extract channel management logic`
