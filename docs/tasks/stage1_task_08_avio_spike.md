# Task 08: AVIOContext Spike (Critical Path)

## Goal
Implement custom FFmpeg `AVIOContext` for in-memory video I/O. Validate that we can demux and decode H.265 frames from a `Bytes` buffer without disk I/O.

## Dependencies
- Task 01: Project Skeleton
- Task 03: H.265 Converter (for test video)

## Files to Create

```
crates/bucket-streamer/src/pipeline/avio.rs     # AVIOContext implementation
crates/bucket-streamer/src/pipeline/mod.rs      # Export avio module
```

## Steps

### 1. Update pipeline/mod.rs

```rust
pub mod avio;
pub mod decoder;
pub mod encoder;
pub mod fetcher;
pub mod session;
```

### 2. Implement pipeline/avio.rs

```rust
use bytes::Bytes;
use ffmpeg_sys_next::{
    self as ffi,
    AVIOContext, AVFormatContext,
    avio_alloc_context, avformat_alloc_context, avformat_open_input,
    avformat_close_input, avformat_find_stream_info, avio_context_free,
    av_malloc, av_free,
    AVSEEK_SIZE, AVIO_FLAG_READ,
};
use std::ffi::{c_void, CString};
use std::os::raw::{c_int, c_uchar};
use std::ptr;

/// Buffer size for AVIOContext (32KB is a good balance)
const AVIO_BUFFER_SIZE: usize = 32 * 1024;

/// Holds video data and read position for FFmpeg callbacks
pub struct InMemoryIO {
    data: Bytes,
    position: usize,
}

impl InMemoryIO {
    pub fn new(data: Bytes) -> Self {
        Self { data, position: 0 }
    }

    pub fn len(&self) -> usize {
        self.data.len()
    }

    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
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

/// FFmpeg seek callback - enables random access within buffer
unsafe extern "C" fn seek_packet(
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

    let base = match whence {
        libc::SEEK_SET => 0,
        libc::SEEK_CUR => io.position as i64,
        libc::SEEK_END => io.data.len() as i64,
        _ => return -1,
    };

    let new_pos = base + offset;
    if new_pos < 0 || new_pos > io.data.len() as i64 {
        return -1;
    }

    io.position = new_pos as usize;
    new_pos
}

/// RAII wrapper for AVIOContext
pub struct AvioContext {
    ctx: *mut AVIOContext,
    io: Box<InMemoryIO>,  // Must outlive ctx
    _buffer: *mut u8,      // Owned by ctx, freed when ctx is freed
}

impl AvioContext {
    /// Create AVIOContext from in-memory video data
    ///
    /// # Safety
    /// This function is safe to call, but the returned context
    /// contains raw pointers managed by FFmpeg.
    pub fn new(data: Bytes) -> Result<Self, AvioError> {
        if data.is_empty() {
            return Err(AvioError::EmptyBuffer);
        }

        unsafe {
            // Allocate IO buffer (owned by AVIOContext after creation)
            let buffer = av_malloc(AVIO_BUFFER_SIZE) as *mut u8;
            if buffer.is_null() {
                return Err(AvioError::AllocationFailed);
            }

            // Box the IO state so it has a stable address
            let mut io = Box::new(InMemoryIO::new(data));
            let io_ptr = &mut *io as *mut InMemoryIO as *mut c_void;

            let ctx = avio_alloc_context(
                buffer as *mut c_uchar,
                AVIO_BUFFER_SIZE as c_int,
                0,  // write_flag = 0 (read-only)
                io_ptr,
                Some(read_packet),
                None,  // write callback
                Some(seek_packet),
            );

            if ctx.is_null() {
                av_free(buffer as *mut c_void);
                return Err(AvioError::ContextCreationFailed);
            }

            Ok(Self {
                ctx,
                io,
                _buffer: buffer,
            })
        }
    }

    /// Get raw pointer for use with AVFormatContext
    pub fn as_ptr(&mut self) -> *mut AVIOContext {
        self.ctx
    }

    /// Reset read position to beginning
    pub fn reset(&mut self) {
        self.io.position = 0;
    }
}

impl Drop for AvioContext {
    fn drop(&mut self) {
        unsafe {
            if !self.ctx.is_null() {
                // Note: avio_context_free also frees the internal buffer
                avio_context_free(&mut self.ctx);
            }
        }
    }
}

// Safety: InMemoryIO contains only Bytes (which is Send+Sync) and usize
unsafe impl Send for AvioContext {}

#[derive(Debug, thiserror::Error)]
pub enum AvioError {
    #[error("Empty buffer provided")]
    EmptyBuffer,

    #[error("Failed to allocate AVIO buffer")]
    AllocationFailed,

    #[error("Failed to create AVIOContext")]
    ContextCreationFailed,
}

/// Open an AVFormatContext using in-memory data
///
/// # Safety
/// Caller must ensure the returned AVFormatContext is properly closed
/// with avformat_close_input, and that avio_ctx outlives it.
pub unsafe fn open_format_context(
    avio_ctx: &mut AvioContext,
) -> Result<*mut AVFormatContext, AvioError> {
    let mut fmt_ctx = avformat_alloc_context();
    if fmt_ctx.is_null() {
        return Err(AvioError::AllocationFailed);
    }

    // Attach our custom AVIO
    (*fmt_ctx).pb = avio_ctx.as_ptr();
    (*fmt_ctx).flags |= ffi::AVFMT_FLAG_CUSTOM_IO as i32;

    // Open input (NULL filename since we use custom IO)
    let ret = avformat_open_input(
        &mut fmt_ctx,
        ptr::null(),
        ptr::null_mut(),
        ptr::null_mut(),
    );

    if ret < 0 {
        avformat_close_input(&mut fmt_ctx);
        return Err(AvioError::ContextCreationFailed);
    }

    // Find stream info
    let ret = avformat_find_stream_info(fmt_ctx, ptr::null_mut());
    if ret < 0 {
        avformat_close_input(&mut fmt_ctx);
        return Err(AvioError::ContextCreationFailed);
    }

    Ok(fmt_ctx)
}
```

### 3. Create spike test

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use ffmpeg_next as ffmpeg;

    fn load_test_video() -> Bytes {
        // Load test video from data/ directory
        let path = std::env::var("TEST_VIDEO_PATH")
            .unwrap_or_else(|_| "data/test.h265.mp4".to_string());
        let data = std::fs::read(&path)
            .expect("Test video not found. Run: repo-cli convert -i <video> -o data/test.h265.mp4");
        Bytes::from(data)
    }

    #[test]
    fn test_avio_context_creation() {
        let data = Bytes::from(vec![0u8; 1024]);
        let ctx = AvioContext::new(data);
        assert!(ctx.is_ok());
    }

    #[test]
    fn test_avio_empty_buffer() {
        let data = Bytes::new();
        let ctx = AvioContext::new(data);
        assert!(matches!(ctx.unwrap_err(), AvioError::EmptyBuffer));
    }

    #[test]
    fn test_decode_first_frame() {
        ffmpeg::init().unwrap();

        let data = load_test_video();
        let mut avio = AvioContext::new(data).unwrap();

        unsafe {
            let fmt_ctx = open_format_context(&mut avio).unwrap();

            // Find video stream
            let mut video_stream_idx = None;
            for i in 0..(*fmt_ctx).nb_streams {
                let stream = *(*fmt_ctx).streams.add(i as usize);
                let codec_type = (*(*stream).codecpar).codec_type;
                if codec_type == ffi::AVMediaType::AVMEDIA_TYPE_VIDEO {
                    video_stream_idx = Some(i as usize);
                    break;
                }
            }

            let stream_idx = video_stream_idx.expect("No video stream found");
            let stream = *(*fmt_ctx).streams.add(stream_idx);
            let codecpar = (*stream).codecpar;

            // Verify it's H.265
            assert_eq!((*codecpar).codec_id, ffi::AVCodecID::AV_CODEC_ID_HEVC);

            // Get dimensions
            let width = (*codecpar).width;
            let height = (*codecpar).height;
            println!("Video dimensions: {}x{}", width, height);
            assert!(width > 0 && height > 0);

            // Clean up
            ffi::avformat_close_input(&mut (fmt_ctx as *mut _));
        }
    }
}
```

### 4. Document memory management

Key lifetime requirements:
1. `InMemoryIO` (via Box) must outlive `AVIOContext`
2. `AvioContext` must outlive `AVFormatContext`
3. `AVFormatContext` must be closed before dropping `AvioContext`

```
┌─────────────────────────────────────────────────────────┐
│  AvioContext (owns all)                                 │
│  ├─ Box<InMemoryIO>  ─── stable address for callbacks   │
│  ├─ *mut AVIOContext ─── points to InMemoryIO           │
│  └─ *mut u8 (buffer) ─── freed by avio_context_free     │
│       │                                                 │
│       ▼                                                 │
│  AVFormatContext.pb ─── borrows AVIOContext             │
│  (must close before AvioContext drops)                  │
└─────────────────────────────────────────────────────────┘
```

## Success Criteria

- [ ] `AvioContext::new(data)` succeeds with valid MP4 data
- [ ] `open_format_context` finds video stream
- [ ] H.265 codec ID detected correctly
- [ ] Video dimensions extracted (non-zero)
- [ ] No memory leaks (test with valgrind if available)
- [ ] Empty buffer returns `AvioError::EmptyBuffer`
- [ ] Tests pass: `cargo test -p bucket-streamer avio`

## Validation Commands

```bash
# Run spike test
TEST_VIDEO_PATH=data/test.h265.mp4 cargo test -p bucket-streamer avio -- --nocapture

# Check for memory leaks (if valgrind available)
cargo build -p bucket-streamer --tests
valgrind --leak-check=full ./target/debug/deps/bucket_streamer-* avio
```

## Context

### Why Custom AVIOContext?
FFmpeg normally reads from files or network URLs. For our use case:
- Data comes from S3 via `object_store` (already in memory)
- We want zero-copy streaming from `Bytes`
- No temp files or disk I/O

### Callback Requirements
FFmpeg callbacks are C functions:
- `read_packet`: Return bytes read, or AVERROR_EOF
- `seek`: Return new position, or -1 on error
- Both receive `opaque` pointer to our `InMemoryIO`

### AVSEEK_SIZE
Special whence value (0x10000) asking "what's the total size?"
FFmpeg uses this for format detection and seeking.

### Thread Safety
`AvioContext` implements `Send` because:
- `Bytes` is `Send + Sync`
- FFmpeg calls are not made concurrently from Rust
- Each decoder gets its own `AvioContext`

### Key Risk Mitigated
This task validates the most uncertain part of the design. If AVIOContext doesn't work correctly, we'd need an alternative approach (temp files, or pure Rust demux).
