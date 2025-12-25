# Task 10: TurboJPEG Encoder Module

## Goal
Implement JPEG encoding using TurboJPEG with direct YUV420P input from decoder output.

## Dependencies
- Task 01: Project Skeleton

## Files to Modify

```
crates/bucket-streamer/src/pipeline/encoder.rs  # Full implementation
```

## Steps

### 1. Implement pipeline/encoder.rs

```rust
use anyhow::{Context, Result};
use turbojpeg::{Compressor, Image, PixelFormat, Subsamp};

use super::decoder::DecodedFrame;

/// JPEG encoder using TurboJPEG
pub struct JpegEncoder {
    compressor: Compressor,
    quality: i32,
}

impl JpegEncoder {
    /// Create a new JPEG encoder
    ///
    /// # Arguments
    /// * `quality` - JPEG quality (1-100, higher = better quality, larger size)
    pub fn new(quality: u8) -> Result<Self> {
        let quality = quality.clamp(1, 100) as i32;
        let compressor = Compressor::new()
            .context("Failed to create TurboJPEG compressor")?;

        Ok(Self { compressor, quality })
    }

    /// Encode a decoded frame to JPEG
    ///
    /// # Arguments
    /// * `frame` - Decoded frame in YUV420P format
    ///
    /// # Returns
    /// JPEG data as bytes
    pub fn encode(&mut self, frame: &DecodedFrame) -> Result<Vec<u8>> {
        let image = Image {
            pixels: &frame.data,
            width: frame.width as usize,
            height: frame.height as usize,
            pitch: frame.linesize[0] as usize,  // Y plane stride
            format: PixelFormat::I420,          // YUV420P planar
        };

        let jpeg = self.compressor
            .compress_to_vec(image)
            .context("JPEG compression failed")?;

        Ok(jpeg)
    }

    /// Encode with explicit YUV planes (alternative API)
    ///
    /// Use this if decoder provides separate plane buffers.
    pub fn encode_yuv_planes(
        &mut self,
        width: u32,
        height: u32,
        y_plane: &[u8],
        u_plane: &[u8],
        v_plane: &[u8],
        y_stride: usize,
        uv_stride: usize,
    ) -> Result<Vec<u8>> {
        // TurboJPEG's YUV encoding with separate planes
        let yuv_image = turbojpeg::YuvImage {
            pixels: &[y_plane, u_plane, v_plane],
            width: width as usize,
            height: height as usize,
            subsamp: Subsamp::Sub2x2,  // 4:2:0 subsampling
            strides: &[y_stride, uv_stride, uv_stride],
        };

        let jpeg = self.compressor
            .compress_yuv_to_vec(yuv_image)
            .context("YUV JPEG compression failed")?;

        Ok(jpeg)
    }

    /// Set encoding quality (1-100)
    pub fn set_quality(&mut self, quality: u8) {
        self.quality = quality.clamp(1, 100) as i32;
    }

    /// Get current quality setting
    pub fn quality(&self) -> u8 {
        self.quality as u8
    }
}

/// Convenience function for one-shot encoding
pub fn encode_frame_to_jpeg(frame: &DecodedFrame, quality: u8) -> Result<Vec<u8>> {
    let mut encoder = JpegEncoder::new(quality)?;
    encoder.encode(frame)
}
```

### 2. Add tests and benchmarks

```rust
#[cfg(test)]
mod tests {
    use super::*;

    fn create_test_frame(width: u32, height: u32) -> DecodedFrame {
        let y_size = (width * height) as usize;
        let uv_size = y_size / 4;

        // Create gradient test pattern
        let mut data = Vec::with_capacity(y_size + 2 * uv_size);

        // Y plane: horizontal gradient
        for y in 0..height {
            for x in 0..width {
                let luma = ((x as f32 / width as f32) * 255.0) as u8;
                data.push(luma);
            }
        }

        // U plane: constant 128 (neutral)
        data.extend(std::iter::repeat(128).take(uv_size));

        // V plane: constant 128 (neutral)
        data.extend(std::iter::repeat(128).take(uv_size));

        DecodedFrame {
            width,
            height,
            data,
            linesize: [width as i32, (width / 2) as i32, (width / 2) as i32],
        }
    }

    #[test]
    fn test_encoder_creation() {
        let encoder = JpegEncoder::new(80);
        assert!(encoder.is_ok());
    }

    #[test]
    fn test_encode_frame() {
        let frame = create_test_frame(640, 480);
        let mut encoder = JpegEncoder::new(80).unwrap();

        let jpeg = encoder.encode(&frame).unwrap();

        // Verify JPEG magic bytes
        assert!(jpeg.len() > 2);
        assert_eq!(jpeg[0], 0xFF);
        assert_eq!(jpeg[1], 0xD8);  // JPEG SOI marker
    }

    #[test]
    fn test_quality_affects_size() {
        let frame = create_test_frame(640, 480);

        let jpeg_low = encode_frame_to_jpeg(&frame, 30).unwrap();
        let jpeg_high = encode_frame_to_jpeg(&frame, 95).unwrap();

        // Higher quality should produce larger file
        assert!(jpeg_high.len() > jpeg_low.len());
    }

    #[test]
    fn test_large_frame() {
        let frame = create_test_frame(1920, 1080);
        let jpeg = encode_frame_to_jpeg(&frame, 80).unwrap();

        // 1080p JPEG should be reasonable size
        assert!(jpeg.len() > 10_000);
        assert!(jpeg.len() < 1_000_000);
    }
}
```

### 3. Add encoding benchmark

```rust
#[cfg(test)]
mod benchmarks {
    use super::*;
    use std::time::Instant;

    #[test]
    #[ignore]  // Run with: cargo test -p bucket-streamer --release -- --ignored
    fn benchmark_encoding_speeds() {
        let resolutions = [
            (640, 480, "480p"),
            (1280, 720, "720p"),
            (1920, 1080, "1080p"),
        ];

        let qualities = [60, 80, 95];
        let iterations = 100;

        println!("\n=== JPEG Encoding Benchmark ===");

        for (width, height, label) in resolutions {
            let frame = create_test_frame(width, height);
            let mut encoder = JpegEncoder::new(80).unwrap();

            for quality in qualities {
                encoder.set_quality(quality);

                let start = Instant::now();
                for _ in 0..iterations {
                    let _ = encoder.encode(&frame).unwrap();
                }
                let elapsed = start.elapsed();

                let avg_ms = elapsed.as_secs_f64() * 1000.0 / iterations as f64;
                let fps = 1000.0 / avg_ms;

                println!(
                    "{} @ q{}: {:.2}ms/frame ({:.1} FPS)",
                    label, quality, avg_ms, fps
                );
            }
        }
    }
}
```

## Success Criteria

- [ ] `JpegEncoder::new(quality)` succeeds
- [ ] `encode()` produces valid JPEG (0xFF 0xD8 magic bytes)
- [ ] Higher quality produces larger files
- [ ] 1080p frames encode without error
- [ ] Encoder is reusable across multiple frames
- [ ] Benchmark shows >30 FPS for 1080p at quality 80
- [ ] Tests pass: `cargo test -p bucket-streamer encoder`

## Benchmark Commands

```bash
# Run encoding benchmark
cargo test -p bucket-streamer --release -- --ignored --nocapture benchmark

# Expected output (varies by hardware):
# 480p @ q80: 0.8ms/frame (1250 FPS)
# 720p @ q80: 2.1ms/frame (476 FPS)
# 1080p @ q80: 4.5ms/frame (222 FPS)
```

## Context

### Why TurboJPEG?
- 2-3x faster than image-rs jpeg encoder
- Direct YUV input (no RGB conversion needed)
- Hardware-optimized (SIMD)
- Used by major video applications

### PixelFormat::I420
I420 is the same as YUV420P:
- Y plane: full resolution
- U plane: 1/4 resolution (2x2 subsampled)
- V plane: 1/4 resolution

This matches FFmpeg's output format.

### Quality vs Size Tradeoffs

| Quality | Typical Size (1080p) | Use Case |
|---------|---------------------|----------|
| 60 | 50-100 KB | Fast scrubbing |
| 80 | 100-200 KB | Normal viewing |
| 95 | 300-500 KB | High quality |

### Thread Safety
`Compressor` is not thread-safe. Each session should have its own encoder instance, or use a pool with synchronization.

### Memory Allocation
`compress_to_vec` allocates output buffer. For high-performance scenarios, consider preallocating with `compress_to_owned`.
