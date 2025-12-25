use bytes::Bytes;
use ffmpeg_next as ffmpeg;
use ffmpeg_next::format::Pixel;
use ffmpeg_next::packet::Mut as _;
use ffmpeg_next::software::scaling::{Context as ScalerContext, Flags};
use ffmpeg_sys_next::{self as ffi, AVFormatContext};

use super::avio::{open_format_context, AvioContext, AvioError};

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

    #[error("Frame at offset {0} not found")]
    FrameNotFound(u64),

    #[error("Decode failed: {0}")]
    DecodeError(String),

    #[error("Send packet failed: {0}")]
    SendPacket(String),
}

/// H.265 decoder with persistent codec context
///
/// Decodes frames by byte offset, matching the protocol's addressing scheme.
/// The decoder maintains FFmpeg codec context across multiple decode calls
/// for efficiency.
///
/// # Usage
/// ```ignore
/// let decoder = Decoder::new(&video_data)?;
///
/// // Decode frame at specific byte offset
/// let frame = decoder.decode_frame(&video_data, 12591)?;
/// ```
///
/// # Thread Safety
/// `Decoder` is not `Send`/`Sync` due to FFmpeg internals. For async usage,
/// wrap decode calls in `tokio::task::spawn_blocking`.
pub struct Decoder {
    video_stream_index: usize,
    decoder: ffmpeg::decoder::Video,
    scaler: ScalerContext,
    width: u32,
    height: u32,
}

impl Decoder {
    /// Create decoder by probing video data to detect format
    ///
    /// # Arguments
    /// * `initial_data` - Valid H.265 MP4 data to probe for codec parameters
    ///
    /// # Errors
    /// Returns error if FFmpeg init fails, no video stream found, or
    /// HEVC decoder is not available.
    pub fn new(initial_data: &Bytes) -> Result<Self, DecoderError> {
        ffmpeg::init().map_err(|_| DecoderError::FfmpegInit)?;

        let mut avio = AvioContext::new(initial_data.clone())?;

        unsafe {
            let fmt_ctx = open_format_context(&mut avio)?;

            let (stream_index, codecpar) = Self::find_video_stream(fmt_ctx)?;

            let codec = ffmpeg::decoder::find(ffmpeg::codec::Id::HEVC)
                .ok_or(DecoderError::DecoderNotFound)?;

            let mut decoder_ctx = ffmpeg::codec::Context::new_with_codec(codec);

            let ret = ffi::avcodec_parameters_to_context(decoder_ctx.as_mut_ptr(), codecpar);
            if ret < 0 {
                ffi::avformat_close_input(&mut (fmt_ctx as *mut _));
                return Err(DecoderError::DecoderOpen(format!(
                    "avcodec_parameters_to_context failed: {}",
                    ret
                )));
            }

            let decoder = decoder_ctx
                .decoder()
                .video()
                .map_err(|e| DecoderError::DecoderOpen(e.to_string()))?;

            let width = decoder.width();
            let height = decoder.height();
            let format = decoder.format();

            ffi::avformat_close_input(&mut (fmt_ctx as *mut _));

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

    /// Decode a single frame at the given byte offset
    ///
    /// Decodes sequentially from the start of the video data until
    /// reaching the packet at `target_offset`. H.265 requires sequential
    /// decoding from the nearest IRAP (keyframe).
    ///
    /// # Arguments
    /// * `video_data` - Video file data (should start from IRAP for correct decoding)
    /// * `target_offset` - Byte offset of the target frame in the original file
    ///
    /// # Returns
    /// The decoded frame at `target_offset` in YUV420P format.
    ///
    /// # Errors
    /// Returns `FrameNotFound` if no packet matches the target offset.
    ///
    /// # Note
    /// For optimal performance, `video_data` should contain only the GOP
    /// starting from the relevant IRAP. The `target_offset` should match
    /// a packet position within that data.
    pub fn decode_frame(
        &mut self,
        video_data: &Bytes,
        target_offset: u64,
    ) -> Result<DecodedFrame, DecoderError> {
        self.decoder.flush();

        let mut avio = AvioContext::new(video_data.clone())?;

        unsafe {
            let fmt_ctx = open_format_context(&mut avio)?;

            let mut packet = ffmpeg::Packet::empty();
            let mut frame = ffmpeg::frame::Video::empty();
            let mut target_frame: Option<DecodedFrame> = None;

            // Track which packet positions have been decoded
            let mut last_decoded_offset: Option<u64> = None;

            while ffi::av_read_frame(fmt_ctx, packet.as_mut_ptr()) >= 0 {
                if packet.stream() != self.video_stream_index {
                    packet.rescale_ts(ffmpeg::Rational::new(1, 1), ffmpeg::Rational::new(1, 1));
                    continue;
                }

                let packet_offset = packet.position();
                let is_target = packet_offset >= 0 && packet_offset as u64 == target_offset;

                self.decoder
                    .send_packet(&packet)
                    .map_err(|e| DecoderError::SendPacket(e.to_string()))?;

                // Receive decoded frames
                while self.decoder.receive_frame(&mut frame).is_ok() {
                    // If this frame corresponds to our target packet
                    if is_target || last_decoded_offset == Some(target_offset) {
                        let decoded = self.convert_frame(&frame)?;
                        target_frame = Some(decoded);
                    }
                    last_decoded_offset = if packet_offset >= 0 {
                        Some(packet_offset as u64)
                    } else {
                        None
                    };
                }

                // Early exit if we found our target
                if target_frame.is_some() {
                    ffi::avformat_close_input(&mut (fmt_ctx as *mut _));
                    return Ok(target_frame.unwrap());
                }

                if is_target {
                    last_decoded_offset = Some(target_offset);
                }
            }

            // Flush decoder for remaining frames
            self.decoder
                .send_eof()
                .map_err(|e| DecoderError::SendPacket(e.to_string()))?;

            while self.decoder.receive_frame(&mut frame).is_ok() {
                if last_decoded_offset == Some(target_offset) {
                    let decoded = self.convert_frame(&frame)?;
                    target_frame = Some(decoded);
                    break;
                }
            }

            ffi::avformat_close_input(&mut (fmt_ctx as *mut _));

            target_frame.ok_or(DecoderError::FrameNotFound(target_offset))
        }
    }

    fn convert_frame(
        &mut self,
        frame: &ffmpeg::frame::Video,
    ) -> Result<DecodedFrame, DecoderError> {
        let mut output = ffmpeg::frame::Video::empty();
        self.scaler
            .run(frame, &mut output)
            .map_err(|e| DecoderError::DecodeError(e.to_string()))?;

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

    /// Flush decoder state (called automatically by decode_frame)
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
            std::fs::read(&path).expect(
                "Test video not found. Run: repo-cli convert -i <video> -o data/test.h265.mp4",
            ),
        )
    }

    /// Helper to get first frame offset from test video
    fn get_first_frame_offset() -> u64 {
        use ffmpeg_next as ffmpeg;
        ffmpeg::init().unwrap();

        let path = std::env::var("TEST_VIDEO_PATH").unwrap_or_else(|_| {
            let possible = vec![
                "data/test.h265.mp4",
                "../../../data/test.h265.mp4",
                "../../data/test.h265.mp4",
            ];
            for p in possible {
                if std::path::Path::new(p).exists() {
                    return p.to_string();
                }
            }
            "data/test.h265.mp4".to_string()
        });

        let ictx = ffmpeg::format::input(&path).unwrap();
        let video_stream = ictx.streams().best(ffmpeg::media::Type::Video).unwrap();
        let stream_idx = video_stream.index();

        let mut ictx = ffmpeg::format::input(&path).unwrap();
        for (stream, packet) in ictx.packets() {
            if stream.index() == stream_idx {
                let pos = packet.position();
                if pos >= 0 {
                    return pos as u64;
                }
            }
        }
        panic!("No video packets found");
    }

    #[test]
    fn test_decoder_creation() {
        let data = load_test_video();
        let decoder = Decoder::new(&data);
        assert!(
            decoder.is_ok(),
            "Decoder creation failed: {:?}",
            decoder.err()
        );

        let decoder = decoder.unwrap();
        assert!(decoder.width() > 0, "Width should be > 0");
        assert!(decoder.height() > 0, "Height should be > 0");
    }

    #[test]
    fn test_decode_frame_by_offset() {
        let data = load_test_video();
        let first_offset = get_first_frame_offset();

        let mut decoder = Decoder::new(&data).expect("Decoder creation failed");
        let frame = decoder.decode_frame(&data, first_offset);

        assert!(frame.is_ok(), "Decode failed: {:?}", frame.err());
        let frame = frame.unwrap();
        assert_eq!(frame.width, decoder.width());
        assert_eq!(frame.height, decoder.height());
        assert!(!frame.data.is_empty());

        // Verify YUV420P data size
        let expected_size = frame.y_plane_size() + 2 * frame.chroma_plane_size();
        assert_eq!(frame.data.len(), expected_size);
    }

    #[test]
    fn test_frame_not_found() {
        let data = load_test_video();
        let mut decoder = Decoder::new(&data).expect("Decoder creation failed");

        // Use an offset that doesn't exist
        let result = decoder.decode_frame(&data, 99999999);

        assert!(result.is_err());
        match result.unwrap_err() {
            DecoderError::FrameNotFound(offset) => {
                assert_eq!(offset, 99999999);
            }
            e => panic!("Expected FrameNotFound error, got: {:?}", e),
        }
    }

    #[test]
    fn test_decoder_reuse() {
        let data = load_test_video();
        let first_offset = get_first_frame_offset();

        let mut decoder = Decoder::new(&data).expect("Decoder creation failed");

        // Decode same frame twice
        let frame1 = decoder
            .decode_frame(&data, first_offset)
            .expect("First decode failed");
        let frame2 = decoder
            .decode_frame(&data, first_offset)
            .expect("Second decode failed");

        assert_eq!(frame1.width, frame2.width);
        assert_eq!(frame1.height, frame2.height);
        assert_eq!(frame1.data.len(), frame2.data.len());
    }

    #[test]
    fn test_yuv420p_format() {
        let data = load_test_video();
        let first_offset = get_first_frame_offset();

        let mut decoder = Decoder::new(&data).expect("Decoder creation failed");
        let frame = decoder
            .decode_frame(&data, first_offset)
            .expect("Decode failed");

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
        let path =
            std::env::var("TEST_VIDEO_PATH").unwrap_or_else(|_| "data/test.h265.mp4".to_string());
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

        // Get first frame offset for benchmark
        let first_offset = {
            use ffmpeg_next as ffmpeg;
            let path = std::env::var("TEST_VIDEO_PATH")
                .unwrap_or_else(|_| "data/test.h265.mp4".to_string());
            let ictx = ffmpeg::format::input(&path).unwrap();
            let stream_idx = ictx
                .streams()
                .best(ffmpeg::media::Type::Video)
                .unwrap()
                .index();
            let mut ictx = ffmpeg::format::input(&path).unwrap();
            ictx.packets()
                .find(|(s, p)| s.index() == stream_idx && p.position() >= 0)
                .map(|(_, p)| p.position() as u64)
                .unwrap()
        };

        // Warm up
        let _ = decoder.decode_frame(&data, first_offset).unwrap();

        let iterations = 50;
        let start = Instant::now();
        for _ in 0..iterations {
            let _ = decoder.decode_frame(&data, first_offset).unwrap();
        }
        let elapsed = start.elapsed();

        let avg_ms = elapsed.as_secs_f64() * 1000.0 / iterations as f64;
        let fps = 1000.0 / avg_ms;
        println!("Single frame decode: {:.2}ms ({:.1} FPS)", avg_ms, fps);
    }
}
