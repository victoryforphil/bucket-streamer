use bytes::Bytes;
use ffmpeg_next as ffmpeg;
use ffmpeg_next::format::Pixel;
use ffmpeg_next::packet::Mut as _;
use ffmpeg_next::software::scaling::{Context as ScalerContext, Flags};
use ffmpeg_sys_next::{self as ffi, AVFormatContext};

use super::avio::{AvioContext, AvioError, open_format_context};

/// Decoded video frame ready for JPEG encoding
#[derive(Debug, Clone)]
pub struct DecodedFrame {
    /// Frame width in pixels
    pub width: u32,
    /// Frame height in pixels
    pub height: u32,
    /// Presentation timestamp (if available from container)
    pub pts: Option<i64>,
    /// YUV420P planar data: Y plane, then U plane, then V plane
    pub data: Vec<u8>,
    /// Row stride for each plane: [Y, U, V]
    pub linesize: [i32; 3],
}

impl DecodedFrame {
    /// Size of Y plane in bytes
    pub fn y_plane_size(&self) -> usize {
        (self.width * self.height) as usize
    }

    /// Size of each chroma plane (U or V) in bytes
    pub fn chroma_plane_size(&self) -> usize {
        self.y_plane_size() / 4
    }
}

/// Decoder error types
#[derive(Debug, thiserror::Error)]
pub enum DecoderError {
    #[error("FFmpeg initialization failed")]
    FfmpegInit,

    #[error("AVIO error: {0}")]
    Avio(#[from] AvioError),

    #[error("No video stream found in container")]
    NoVideoStream,

    #[error("HEVC/H.265 decoder not available")]
    DecoderNotFound,

    #[error("Failed to open decoder: {0}")]
    DecoderOpen(String),

    #[error("Failed to initialize scaler")]
    ScalerInit,

    #[error("Frame index {index} not found (GOP contains {total} frames)")]
    FrameNotFound { index: u32, total: u32 },

    #[error("Decode failed: {0}")]
    DecodeError(String),

    #[error("Send packet failed: {0}")]
    SendPacket(String),

    #[error("Receive frame failed: {0}")]
    ReceiveFrame(String),

    #[error("Format context error: {0}")]
    FormatContext(String),
}

/// H.265 decoder with persistent context
///
/// The decoder maintains FFmpeg codec context across multiple decode calls.
/// For each GOP, a new format context is created but the codec state is reused.
///
/// # Usage
/// ```ignore
/// let decoder = Decoder::new(&initial_video_sample)?;
/// 
/// // Decode specific frames from a GOP
/// let frames = decoder.decode_frames(&gop_data, &[0, 2, 4])?;
/// 
/// // Or decode all frames
/// let all_frames = decoder.decode_all_frames(&gop_data)?;
/// ```
///
/// # Thread Safety
/// `Decoder` is not `Send`/`Sync` due to FFmpeg internals. For async usage,
/// wrap decode calls in `tokio::task::spawn_blocking`.
pub struct Decoder {
    /// Video stream index in container
    video_stream_index: usize,
    /// FFmpeg video decoder (persistent)
    decoder: ffmpeg::decoder::Video,
    /// YUV420P scaler (initialized upfront)
    scaler: ScalerContext,
    /// Video dimensions
    width: u32,
    height: u32,
}

impl Decoder {
    /// Create decoder by probing video data to detect format
    ///
    /// The `initial_data` is used to detect video format, dimensions,
    /// and initialize the decoder and scaler. This can be a small sample
    /// or the complete video file.
    ///
    /// # Arguments
    /// * `initial_data` - Valid MP4 data to probe for codec parameters
    ///
    /// # Errors
    /// Returns error if FFmpeg init fails, no video stream found, or
    /// HEVC decoder is not available.
    pub fn new(initial_data: &Bytes) -> Result<Self, DecoderError> {
        ffmpeg::init().map_err(|_| DecoderError::FfmpegInit)?;

        let mut avio = AvioContext::new(initial_data.clone())?;

        unsafe {
            let fmt_ctx = open_format_context(&mut avio)?;

            // Find video stream
            let (stream_index, codecpar) = Self::find_video_stream(fmt_ctx)?;

            // Create decoder
            let codec = ffmpeg::decoder::find(ffmpeg::codec::Id::HEVC)
                .ok_or(DecoderError::DecoderNotFound)?;

            let mut decoder_ctx = ffmpeg::codec::Context::new_with_codec(codec);

            // Copy codec parameters to decoder context
            let ret = ffi::avcodec_parameters_to_context(
                decoder_ctx.as_mut_ptr(),
                codecpar,
            );
            if ret < 0 {
                ffi::avformat_close_input(&mut (fmt_ctx as *mut _));
                return Err(DecoderError::DecoderOpen(
                    format!("avcodec_parameters_to_context failed: {}", ret)
                ));
            }

            let decoder = decoder_ctx
                .decoder()
                .video()
                .map_err(|e| DecoderError::DecoderOpen(e.to_string()))?;

            let width = decoder.width();
            let height = decoder.height();
            let format = decoder.format();

            // Clean up format context (decoder is independent now)
            ffi::avformat_close_input(&mut (fmt_ctx as *mut _));

            // Initialize scaler upfront for YUV420P output
            let scaler = ScalerContext::get(
                format,
                width,
                height,
                Pixel::YUV420P,
                width,
                height,
                Flags::BILINEAR,
            )
            .map_err(|_| DecoderError::ScalerInit)?;

            Ok(Self {
                video_stream_index: stream_index,
                decoder,
                scaler,
                width,
                height,
            })
        }
    }

    /// Find video stream in format context
    unsafe fn find_video_stream(
        fmt_ctx: *mut AVFormatContext,
    ) -> Result<(usize, *const ffi::AVCodecParameters), DecoderError> {
        for i in 0..(*fmt_ctx).nb_streams {
            let stream = *(*fmt_ctx).streams.add(i as usize);
            let codecpar = (*stream).codecpar;
            if (*codecpar).codec_type == ffi::AVMediaType::AVMEDIA_TYPE_VIDEO {
                return Ok((i as usize, codecpar));
            }
        }
        Err(DecoderError::NoVideoStream)
    }

    /// Decode specific frames from a GOP byte range
    ///
    /// # Arguments
    /// * `gop_data` - Valid MP4 structure containing headers + GOP data
    /// * `frame_indices` - Relative indices within the GOP (0 = IRAP keyframe)
    ///
    /// # Returns
    /// Vector of decoded frames in the order requested.
    ///
    /// # Errors
    /// Returns error if any requested frame index is not found in the GOP.
    ///
    /// # Example
    /// ```ignore
    /// // Decode keyframe and frames 2, 4 from a GOP
    /// let frames = decoder.decode_frames(&gop_bytes, &[0, 2, 4])?;
    /// assert_eq!(frames.len(), 3);
    /// ```
    pub fn decode_frames(
        &mut self,
        gop_data: &Bytes,
        frame_indices: &[u32],
    ) -> Result<Vec<DecodedFrame>, DecoderError> {
        if frame_indices.is_empty() {
            return Ok(Vec::new());
        }

        // Find the maximum frame index we need to decode up to
        let max_index = *frame_indices.iter().max().unwrap();

        // Decode all frames up to max_index
        let all_frames = self.decode_up_to(gop_data, max_index)?;

        // Extract requested frames in order
        let mut result = Vec::with_capacity(frame_indices.len());
        for &idx in frame_indices {
            let frame = all_frames
                .get(idx as usize)
                .cloned()
                .ok_or(DecoderError::FrameNotFound {
                    index: idx,
                    total: all_frames.len() as u32,
                })?;
            result.push(frame);
        }

        Ok(result)
    }

    /// Decode all frames in a GOP
    ///
    /// # Arguments
    /// * `gop_data` - Valid MP4 structure containing headers + GOP data
    ///
    /// # Returns
    /// All decoded frames in decode order (frame 0 = IRAP).
    pub fn decode_all_frames(
        &mut self,
        gop_data: &Bytes,
    ) -> Result<Vec<DecodedFrame>, DecoderError> {
        self.decode_up_to(gop_data, u32::MAX)
    }

    /// Decode frames from GOP up to (and including) max_index
    fn decode_up_to(
        &mut self,
        gop_data: &Bytes,
        max_index: u32,
    ) -> Result<Vec<DecodedFrame>, DecoderError> {
        // Always flush before decoding new GOP
        self.decoder.flush();

        let mut avio = AvioContext::new(gop_data.clone())?;

        unsafe {
            let fmt_ctx = open_format_context(&mut avio)?;

            let mut decoded_frames = Vec::new();
            let mut packet = ffmpeg::Packet::empty();
            let mut frame = ffmpeg::frame::Video::empty();
            let mut current_index: u32 = 0;

            // Read and decode packets
            while ffi::av_read_frame(fmt_ctx, packet.as_mut_ptr()) >= 0 {
                // Skip non-video streams
                if packet.stream() != self.video_stream_index {
                    packet.rescale_ts(
                        ffmpeg::Rational::new(1, 1),
                        ffmpeg::Rational::new(1, 1),
                    );
                    continue;
                }

                // Send packet to decoder
                self.decoder
                    .send_packet(&packet)
                    .map_err(|e| DecoderError::SendPacket(e.to_string()))?;

                // Receive all available frames
                while self.decoder.receive_frame(&mut frame).is_ok() {
                    let decoded = self.convert_frame(&frame)?;
                    decoded_frames.push(decoded);

                    current_index += 1;
                    if current_index > max_index {
                        ffi::avformat_close_input(&mut (fmt_ctx as *mut _));
                        return Ok(decoded_frames);
                    }
                }
            }

            // Flush decoder to get any remaining frames
            self.decoder
                .send_eof()
                .map_err(|e| DecoderError::SendPacket(e.to_string()))?;

            while self.decoder.receive_frame(&mut frame).is_ok() {
                let decoded = self.convert_frame(&frame)?;
                decoded_frames.push(decoded);

                current_index += 1;
                if current_index > max_index {
                    break;
                }
            }

            ffi::avformat_close_input(&mut (fmt_ctx as *mut _));
            Ok(decoded_frames)
        }
    }

    /// Convert FFmpeg frame to DecodedFrame (YUV420P)
    fn convert_frame(
        &mut self,
        frame: &ffmpeg::frame::Video,
    ) -> Result<DecodedFrame, DecoderError> {
        let mut output = ffmpeg::frame::Video::empty();
        self.scaler
            .run(frame, &mut output)
            .map_err(|e| DecoderError::DecodeError(e.to_string()))?;

        // Copy YUV planes to contiguous buffer
        let y_size = (self.width * self.height) as usize;
        let uv_size = y_size / 4;
        let mut data = Vec::with_capacity(y_size + 2 * uv_size);

        // Y plane
        for row in 0..self.height as usize {
            let start = row * output.stride(0);
            let end = start + self.width as usize;
            data.extend_from_slice(&output.data(0)[start..end]);
        }

        // U plane
        let uv_height = self.height as usize / 2;
        let uv_width = self.width as usize / 2;
        for row in 0..uv_height {
            let start = row * output.stride(1);
            let end = start + uv_width;
            data.extend_from_slice(&output.data(1)[start..end]);
        }

        // V plane
        for row in 0..uv_height {
            let start = row * output.stride(2);
            let end = start + uv_width;
            data.extend_from_slice(&output.data(2)[start..end]);
        }

        Ok(DecodedFrame {
            width: self.width,
            height: self.height,
            pts: frame.pts(),
            data,
            linesize: [
                self.width as i32,
                (self.width / 2) as i32,
                (self.width / 2) as i32,
            ],
        })
    }

    /// Flush decoder state
    ///
    /// Called automatically between GOP decodes, but can be called
    /// manually if needed.
    pub fn flush(&mut self) {
        self.decoder.flush();
    }

    /// Video width in pixels
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Video height in pixels
    pub fn height(&self) -> u32 {
        self.height
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn load_test_video() -> Bytes {
        let possible_paths = vec![
            "data/test.h265.mp4",
            "../../../data/test.h265.mp4",
            "../../data/test.h265.mp4",
        ];

        let path = std::env::var("TEST_VIDEO_PATH")
            .ok()
            .or_else(|| {
                for p in possible_paths.iter() {
                    if std::path::Path::new(p).exists() {
                        return Some(p.to_string());
                    }
                }
                None
            })
            .unwrap_or_else(|| "data/test.h265.mp4".to_string());

        Bytes::from(
            std::fs::read(&path)
                .expect("Test video not found. Run: repo-cli convert -i <video> -o data/test.h265.mp4"),
        )
    }

    #[test]
    fn test_decoder_creation() {
        let data = load_test_video();
        let decoder = Decoder::new(&data);
        assert!(decoder.is_ok(), "Decoder creation failed: {:?}", decoder.err());

        let decoder = decoder.unwrap();
        assert!(decoder.width() > 0, "Width should be > 0");
        assert!(decoder.height() > 0, "Height should be > 0");
    }

    #[test]
    fn test_decode_first_frame() {
        let data = load_test_video();
        let mut decoder = Decoder::new(&data).expect("Decoder creation failed");

        let frames = decoder.decode_frames(&data, &[0]).expect("Decode failed");

        assert_eq!(frames.len(), 1);
        let frame = &frames[0];
        assert_eq!(frame.width, decoder.width());
        assert_eq!(frame.height, decoder.height());
        assert!(!frame.data.is_empty());

        // Verify YUV420P data size
        let expected_size = frame.y_plane_size() + 2 * frame.chroma_plane_size();
        assert_eq!(frame.data.len(), expected_size);
    }

    #[test]
    fn test_decode_multiple_frames() {
        let data = load_test_video();
        let mut decoder = Decoder::new(&data).expect("Decoder creation failed");

        // Decode frames 0, 2, 4
        let indices = [0, 2, 4];
        let frames = decoder.decode_frames(&data, &indices);

        assert!(frames.is_ok(), "Failed to decode frames: {:?}", frames.err());
        let frames = frames.unwrap();
        assert_eq!(frames.len(), indices.len());
    }

    #[test]
    fn test_decode_all_frames() {
        let data = load_test_video();
        let mut decoder = Decoder::new(&data).expect("Decoder creation failed");

        let frames = decoder.decode_all_frames(&data);
        assert!(frames.is_ok(), "Failed to decode all frames: {:?}", frames.err());

        let frames = frames.unwrap();
        assert!(!frames.is_empty(), "Should decode at least one frame");
        println!("Decoded {} frames", frames.len());
    }

    #[test]
    fn test_decoder_reuse() {
        let data = load_test_video();
        let mut decoder = Decoder::new(&data).expect("Decoder creation failed");

        // Decode first GOP
        let frames1 = decoder.decode_frames(&data, &[0]).expect("First decode failed");

        // Decode again (simulating second GOP with same data for test)
        let frames2 = decoder.decode_frames(&data, &[0]).expect("Second decode failed");

        // Both should succeed with same dimensions
        assert_eq!(frames1[0].width, frames2[0].width);
        assert_eq!(frames1[0].height, frames2[0].height);
        assert_eq!(frames1[0].data.len(), frames2[0].data.len());
    }

    #[test]
    fn test_frame_not_found() {
        let data = load_test_video();
        let mut decoder = Decoder::new(&data).expect("Decoder creation failed");

        // Request a frame index that's likely beyond the video length
        let result = decoder.decode_frames(&data, &[9999]);

        assert!(result.is_err());
        match result.unwrap_err() {
            DecoderError::FrameNotFound { index, total } => {
                assert_eq!(index, 9999);
                assert!(total < 9999);
            }
            e => panic!("Expected FrameNotFound error, got: {:?}", e),
        }
    }

    #[test]
    fn test_empty_frame_indices() {
        let data = load_test_video();
        let mut decoder = Decoder::new(&data).expect("Decoder creation failed");

        let frames = decoder.decode_frames(&data, &[]).expect("Empty decode failed");
        assert!(frames.is_empty());
    }

    #[test]
    fn test_yuv420p_format() {
        let data = load_test_video();
        let mut decoder = Decoder::new(&data).expect("Decoder creation failed");

        let frames = decoder.decode_frames(&data, &[0]).expect("Decode failed");
        let frame = &frames[0];

        // Verify linesize for packed YUV420P
        assert_eq!(frame.linesize[0], frame.width as i32);
        assert_eq!(frame.linesize[1], (frame.width / 2) as i32);
        assert_eq!(frame.linesize[2], (frame.width / 2) as i32);
    }
}

#[cfg(test)]
mod benchmarks {
    use super::*;
    use std::time::Instant;

    fn load_test_video() -> Bytes {
        let possible_paths = vec![
            "data/test.h265.mp4",
            "../../../data/test.h265.mp4",
            "../../data/test.h265.mp4",
        ];

        let path = std::env::var("TEST_VIDEO_PATH")
            .ok()
            .or_else(|| {
                for p in possible_paths.iter() {
                    if std::path::Path::new(p).exists() {
                        return Some(p.to_string());
                    }
                }
                None
            })
            .unwrap_or_else(|| "data/test.h265.mp4".to_string());

        Bytes::from(std::fs::read(&path).expect("Test video not found"))
    }

    #[test]
    #[ignore] // Run with: cargo test -p bucket-streamer --release -- --ignored --nocapture
    fn benchmark_decoder_creation() {
        let data = load_test_video();
        let iterations = 10;

        let start = Instant::now();
        for _ in 0..iterations {
            let _ = Decoder::new(&data).unwrap();
        }
        let elapsed = start.elapsed();

        let avg_ms = elapsed.as_secs_f64() * 1000.0 / iterations as f64;
        println!("Decoder creation: {:.2}ms average", avg_ms);
    }

    #[test]
    #[ignore]
    fn benchmark_frame_decode() {
        let data = load_test_video();
        let mut decoder = Decoder::new(&data).unwrap();

        // Warm up
        let _ = decoder.decode_frames(&data, &[0]).unwrap();

        let iterations = 50;
        let start = Instant::now();
        for _ in 0..iterations {
            let _ = decoder.decode_frames(&data, &[0]).unwrap();
        }
        let elapsed = start.elapsed();

        let avg_ms = elapsed.as_secs_f64() * 1000.0 / iterations as f64;
        let fps = 1000.0 / avg_ms;
        println!("Single frame decode: {:.2}ms ({:.1} FPS)", avg_ms, fps);
    }

    #[test]
    #[ignore]
    fn benchmark_sequential_frames() {
        let data = load_test_video();
        let mut decoder = Decoder::new(&data).unwrap();

        let start = Instant::now();
        let frames = decoder.decode_all_frames(&data).unwrap();
        let elapsed = start.elapsed();

        let avg_ms = elapsed.as_secs_f64() * 1000.0 / frames.len() as f64;
        let fps = frames.len() as f64 / elapsed.as_secs_f64();
        println!(
            "Sequential decode: {} frames in {:.2}ms ({:.2}ms/frame, {:.1} FPS)",
            frames.len(),
            elapsed.as_secs_f64() * 1000.0,
            avg_ms,
            fps
        );
    }
}
