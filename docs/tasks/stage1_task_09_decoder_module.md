# Task 09: H.265 Decoder Module

## Goal
Implement full H.265 decoder with persistent context, frame-by-frame decoding, and context reuse across seeks.

## Dependencies
- Task 08: AVIOContext Spike

## Files to Modify

```
crates/bucket-streamer/src/pipeline/decoder.rs  # Full implementation
```

## Steps

### 1. Implement pipeline/decoder.rs

```rust
use anyhow::{Context, Result};
use bytes::Bytes;
use ffmpeg_next as ffmpeg;
use ffmpeg_next::format::Pixel;
use ffmpeg_next::software::scaling::{Context as ScalerContext, Flags};

use super::avio::{AvioContext, open_format_context};

/// Decoded video frame ready for encoding
pub struct DecodedFrame {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,      // YUV420P planar data
    pub linesize: [i32; 3], // Y, U, V linesizes
}

/// H.265 decoder with persistent context
pub struct Decoder {
    video_stream_index: usize,
    decoder: ffmpeg::decoder::Video,
    scaler: Option<ScalerContext>,
    width: u32,
    height: u32,
}

impl Decoder {
    /// Create decoder from video data
    ///
    /// Initializes FFmpeg decoder context. Can be reused for multiple
    /// decode calls on the same video format.
    pub fn new(video_data: &Bytes) -> Result<Self> {
        ffmpeg::init().context("FFmpeg init failed")?;

        let mut avio = AvioContext::new(video_data.clone())
            .context("Failed to create AVIO context")?;

        unsafe {
            let fmt_ctx = open_format_context(&mut avio)
                .context("Failed to open format context")?;

            // Find video stream
            let (stream_index, codecpar) = Self::find_video_stream(fmt_ctx)?;

            // Create decoder
            let codec = ffmpeg::decoder::find(ffmpeg::codec::Id::HEVC)
                .context("H.265/HEVC decoder not found")?;

            let mut decoder_ctx = ffmpeg::codec::Context::new();
            
            // Copy codec parameters
            ffmpeg_sys_next::avcodec_parameters_to_context(
                decoder_ctx.as_mut_ptr(),
                codecpar,
            );

            let decoder = decoder_ctx
                .decoder()
                .video()
                .context("Failed to open video decoder")?;

            let width = decoder.width();
            let height = decoder.height();

            // Clean up format context (decoder is independent now)
            ffmpeg_sys_next::avformat_close_input(&mut (fmt_ctx as *mut _));

            Ok(Self {
                video_stream_index: stream_index,
                decoder,
                scaler: None,
                width,
                height,
            })
        }
    }

    /// Find video stream in format context
    unsafe fn find_video_stream(
        fmt_ctx: *mut ffmpeg_sys_next::AVFormatContext,
    ) -> Result<(usize, *const ffmpeg_sys_next::AVCodecParameters)> {
        for i in 0..(*fmt_ctx).nb_streams {
            let stream = *(*fmt_ctx).streams.add(i as usize);
            let codecpar = (*stream).codecpar;
            if (*codecpar).codec_type == ffmpeg_sys_next::AVMediaType::AVMEDIA_TYPE_VIDEO {
                return Ok((i as usize, codecpar));
            }
        }
        anyhow::bail!("No video stream found")
    }

    /// Decode a specific frame from video data
    ///
    /// # Arguments
    /// * `video_data` - Complete video file bytes
    /// * `target_frame_index` - Frame index to decode (0-based)
    ///
    /// # Returns
    /// Decoded frame in YUV420P format
    pub fn decode_frame(
        &mut self,
        video_data: &Bytes,
        target_frame_index: u32,
    ) -> Result<DecodedFrame> {
        let mut avio = AvioContext::new(video_data.clone())?;

        unsafe {
            let fmt_ctx = open_format_context(&mut avio)?;

            let mut packet = ffmpeg::Packet::empty();
            let mut frame = ffmpeg::frame::Video::empty();
            let mut current_frame: u32 = 0;

            // Read packets until we reach target frame
            while ffmpeg_sys_next::av_read_frame(fmt_ctx, packet.as_mut_ptr()) >= 0 {
                if packet.stream() != self.video_stream_index {
                    continue;
                }

                self.decoder.send_packet(&packet)?;

                while self.decoder.receive_frame(&mut frame).is_ok() {
                    if current_frame == target_frame_index {
                        let decoded = self.convert_frame(&frame)?;
                        ffmpeg_sys_next::avformat_close_input(&mut (fmt_ctx as *mut _));
                        return Ok(decoded);
                    }
                    current_frame += 1;
                }
            }

            ffmpeg_sys_next::avformat_close_input(&mut (fmt_ctx as *mut _));
            anyhow::bail!("Frame {} not found (video has {} frames)", target_frame_index, current_frame)
        }
    }

    /// Decode frame at specific byte offset
    ///
    /// More efficient than frame index for random access - seeks to
    /// IRAP then decodes forward to target offset.
    pub fn decode_at_offset(
        &mut self,
        video_data: &Bytes,
        irap_offset: u64,
        target_offset: u64,
    ) -> Result<DecodedFrame> {
        let mut avio = AvioContext::new(video_data.clone())?;

        unsafe {
            let fmt_ctx = open_format_context(&mut avio)?;

            // Seek to IRAP offset
            let stream = *(*fmt_ctx).streams.add(self.video_stream_index);
            let time_base = (*stream).time_base;
            
            // Convert byte offset to timestamp (approximate)
            // For precise seeking, we'd need packet position mapping
            let ret = ffmpeg_sys_next::av_seek_frame(
                fmt_ctx,
                self.video_stream_index as i32,
                irap_offset as i64,
                ffmpeg_sys_next::AVSEEK_FLAG_BYTE as i32,
            );

            if ret < 0 {
                // Fallback: seek to beginning
                ffmpeg_sys_next::av_seek_frame(
                    fmt_ctx,
                    -1,
                    0,
                    ffmpeg_sys_next::AVSEEK_FLAG_BYTE as i32,
                );
            }

            // Flush decoder after seek
            self.decoder.flush();

            let mut packet = ffmpeg::Packet::empty();
            let mut frame = ffmpeg::frame::Video::empty();
            let mut decoded_frame: Option<DecodedFrame> = None;

            // Decode until we hit target offset
            while ffmpeg_sys_next::av_read_frame(fmt_ctx, packet.as_mut_ptr()) >= 0 {
                if packet.stream() != self.video_stream_index {
                    continue;
                }

                let pkt_pos = (*packet.as_ptr()).pos as u64;

                self.decoder.send_packet(&packet)?;

                while self.decoder.receive_frame(&mut frame).is_ok() {
                    if pkt_pos >= target_offset {
                        decoded_frame = Some(self.convert_frame(&frame)?);
                        break;
                    }
                }

                if decoded_frame.is_some() {
                    break;
                }
            }

            ffmpeg_sys_next::avformat_close_input(&mut (fmt_ctx as *mut _));

            decoded_frame.context("Target offset not found in video")
        }
    }

    /// Convert FFmpeg frame to our DecodedFrame format
    fn convert_frame(&mut self, frame: &ffmpeg::frame::Video) -> Result<DecodedFrame> {
        // Ensure scaler is initialized for YUV420P output
        let scaler = self.scaler.get_or_insert_with(|| {
            ScalerContext::get(
                frame.format(),
                frame.width(),
                frame.height(),
                Pixel::YUV420P,
                self.width,
                self.height,
                Flags::BILINEAR,
            )
            .expect("Failed to create scaler")
        });

        let mut output = ffmpeg::frame::Video::empty();
        scaler.run(frame, &mut output)?;

        // Copy YUV planes to contiguous buffer
        let y_size = (self.width * self.height) as usize;
        let uv_size = y_size / 4;
        let mut data = Vec::with_capacity(y_size + 2 * uv_size);

        data.extend_from_slice(&output.data(0)[..y_size]);
        data.extend_from_slice(&output.data(1)[..uv_size]);
        data.extend_from_slice(&output.data(2)[..uv_size]);

        Ok(DecodedFrame {
            width: self.width,
            height: self.height,
            data,
            linesize: [
                output.stride(0) as i32,
                output.stride(1) as i32,
                output.stride(2) as i32,
            ],
        })
    }

    /// Flush decoder (call between seeks)
    pub fn flush(&mut self) {
        self.decoder.flush();
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }
}
```

### 2. Add tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn load_test_video() -> Bytes {
        let path = std::env::var("TEST_VIDEO_PATH")
            .unwrap_or_else(|_| "data/test.h265.mp4".to_string());
        Bytes::from(std::fs::read(&path).expect("Test video not found"))
    }

    #[test]
    fn test_decoder_creation() {
        let data = load_test_video();
        let decoder = Decoder::new(&data);
        assert!(decoder.is_ok());

        let decoder = decoder.unwrap();
        assert!(decoder.width() > 0);
        assert!(decoder.height() > 0);
    }

    #[test]
    fn test_decode_first_frame() {
        let data = load_test_video();
        let mut decoder = Decoder::new(&data).unwrap();

        let frame = decoder.decode_frame(&data, 0).unwrap();

        assert_eq!(frame.width, decoder.width());
        assert_eq!(frame.height, decoder.height());
        assert!(!frame.data.is_empty());
    }

    #[test]
    fn test_decode_multiple_frames() {
        let data = load_test_video();
        let mut decoder = Decoder::new(&data).unwrap();

        // Decode frames 0, 5, 10
        for i in [0, 5, 10] {
            let frame = decoder.decode_frame(&data, i);
            assert!(frame.is_ok(), "Failed to decode frame {}", i);
        }
    }

    #[test]
    fn test_decoder_reuse() {
        let data = load_test_video();
        let mut decoder = Decoder::new(&data).unwrap();

        // Decode same frame twice - decoder context reused
        let frame1 = decoder.decode_frame(&data, 0).unwrap();
        decoder.flush();
        let frame2 = decoder.decode_frame(&data, 0).unwrap();

        assert_eq!(frame1.data.len(), frame2.data.len());
    }
}
```

## Success Criteria

- [ ] `Decoder::new()` initializes from video bytes
- [ ] `decode_frame()` returns valid YUV420P data
- [ ] Frame dimensions match video dimensions
- [ ] Multiple frames can be decoded sequentially
- [ ] Decoder context is reused (not recreated per frame)
- [ ] `flush()` resets decoder state for seeking
- [ ] Tests pass: `cargo test -p bucket-streamer decoder`

## Context

### Decoder Reuse
Creating an FFmpeg decoder takes ~30ms. By keeping the decoder context alive and only flushing between seeks, we avoid this overhead per frame.

### YUV420P Format
Standard video format with:
- Y plane: full resolution (width × height)
- U plane: quarter resolution (width/2 × height/2)
- V plane: quarter resolution

TurboJPEG can encode YUV420P directly without RGB conversion.

### Seeking Strategy
For random access:
1. Seek to IRAP (keyframe) byte offset
2. Flush decoder
3. Decode forward to target frame

This is faster than decoding from start for distant frames.

### Thread Safety
`Decoder` is not `Send`/`Sync` due to FFmpeg internals. In the server, each session owns its decoder. For multi-threaded decoding, use `spawn_blocking`.
