# Task 10: TurboJPEG Encoder Module

## Goal
Implement JPEG encoding using TurboJPEG with direct YUV420P input from decoder output.

## Dependencies
- Task 01: Project Skeleton
- Task 09: H.265 Decoder Module (for `DecodedFrame` type)

## Build Requirements

TurboJPEG requires NASM assembler for SIMD optimizations:

### Local Development
```bash
# Ubuntu/Debian
sudo apt-get install nasm

# macOS
brew install nasm

# Fedora/RHEL
sudo dnf install nasm
```

### Docker
Add to Dockerfile:
```dockerfile
RUN apt-get update && apt-get install -y nasm
```

## Files to Modify

```
crates/bucket-streamer/Cargo.toml           # Add turbojpeg dependency
crates/bucket-streamer/src/pipeline/encoder.rs  # Full implementation
```

## Steps

### 1. Add dependency to Cargo.toml

```toml
# Add to [dependencies]
turbojpeg = "1.3"
```

### 2. Implement pipeline/encoder.rs

```rust
use anyhow::{Context, Result};
use turbojpeg::{Compressor, Subsamp, YuvImage};

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
        let mut compressor = Compressor::new()
            .context("Failed to create TurboJPEG compressor")?;

        compressor
            .set_quality(quality)
            .context("Failed to set JPEG quality")?;
        compressor
            .set_subsamp(Subsamp::Sub2x2)
            .context("Failed to set subsampling")?;

        Ok(Self { compressor, quality })
    }

    /// Encode a decoded frame to JPEG
    ///
    /// # Arguments
    /// * `frame` - Decoded frame in YUV420P format (contiguous Y, U, V planes)
    ///
    /// # Returns
    /// JPEG data as bytes
    pub fn encode(&mut self, frame: &DecodedFrame) -> Result<Vec<u8>> {
        let yuv_image = YuvImage {
            pixels: frame.data.as_slice(),
            width: frame.width as usize,
            height: frame.height as usize,
            align: 1, // Data is tightly packed (no row padding)
            subsamp: Subsamp::Sub2x2, // 4:2:0 subsampling
        };

        self.compressor
            .compress_yuv_to_vec(yuv_image)
            .context("JPEG compression failed")
    }

    /// Set encoding quality (1-100)
    pub fn set_quality(&mut self, quality: u8) -> Result<()> {
        self.quality = quality.clamp(1, 100) as i32;
        self.compressor
            .set_quality(self.quality)
            .context("Failed to set quality")
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

### 3. Add tests

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
            for _ in 0..width {
                let luma = ((y as f32 / height as f32) * 255.0) as u8;
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
            pts: None,
            data,
            linesize: [width as i32, (width / 2) as i32, (width / 2) as i32],
        }
    }

    #[test]
    fn test_encoder_creation() {
        let encoder = JpegEncoder::new(80);
        assert!(encoder.is_ok());
        assert_eq!(encoder.unwrap().quality(), 80);
    }

    #[test]
    fn test_encode_frame() {
        let frame = create_test_frame(640, 480);
        let mut encoder = JpegEncoder::new(80).unwrap();

        let jpeg = encoder.encode(&frame).unwrap();

        // Verify JPEG magic bytes
        assert!(jpeg.len() > 2);
        assert_eq!(jpeg[0], 0xFF);
        assert_eq!(jpeg[1], 0xD8); // JPEG SOI marker
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

    #[test]
    fn test_set_quality() {
        let mut encoder = JpegEncoder::new(50).unwrap();
        assert_eq!(encoder.quality(), 50);

        encoder.set_quality(90).unwrap();
        assert_eq!(encoder.quality(), 90);
    }

    #[test]
    fn test_quality_clamping() {
        // Quality 0 should clamp to 1
        let encoder = JpegEncoder::new(0).unwrap();
        assert_eq!(encoder.quality(), 1);

        // Quality 255 should clamp to 100
        let encoder = JpegEncoder::new(255).unwrap();
        assert_eq!(encoder.quality(), 100);
    }

    #[test]
    fn test_encoder_reuse() {
        let mut encoder = JpegEncoder::new(80).unwrap();

        let frame1 = create_test_frame(640, 480);
        let frame2 = create_test_frame(1280, 720);

        let jpeg1 = encoder.encode(&frame1).unwrap();
        let jpeg2 = encoder.encode(&frame2).unwrap();

        // Both should be valid JPEGs
        assert_eq!(jpeg1[0..2], [0xFF, 0xD8]);
        assert_eq!(jpeg2[0..2], [0xFF, 0xD8]);

        // Different sizes due to resolution
        assert!(jpeg2.len() > jpeg1.len());
    }
}
```

### 4. Add benchmarks

```rust
#[cfg(test)]
mod benchmarks {
    use super::*;
    use std::time::Instant;

    #[test]
    #[ignore] // Run with: cargo test -p bucket-streamer --release -- --ignored
    fn benchmark_encoding_speeds() {
        let resolutions = [
            (640, 480, "480p"),
            (1280, 720, "720p"),
            (1920, 1080, "1080p"),
        ];

        let qualities = [60, 80, 95];
        let iterations = 100;

        println!("\n=== JPEG Encoding Benchmark (Baseline) ===");

        for (width, height, label) in resolutions {
            let frame = create_test_frame(width, height);
            let mut encoder = JpegEncoder::new(80).unwrap();

            for quality in qualities {
                encoder.set_quality(quality).unwrap();

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

    #[test]
    #[ignore]
    fn benchmark_encoder_creation() {
        let iterations = 100;

        let start = Instant::now();
        for _ in 0..iterations {
            let _ = JpegEncoder::new(80).unwrap();
        }
        let elapsed = start.elapsed();

        let avg_ms = elapsed.as_secs_f64() * 1000.0 / iterations as f64;
        println!("\nEncoder creation: {:.3}ms average", avg_ms);
    }
}
```

## Success Criteria

- [ ] `turbojpeg` dependency added to Cargo.toml
- [ ] `JpegEncoder::new(quality)` succeeds and applies quality
- [ ] `encode()` produces valid JPEG (0xFF 0xD8 magic bytes)
- [ ] Higher quality produces larger files
- [ ] 1080p frames encode without error
- [ ] Encoder is reusable across multiple frames of different sizes
- [ ] `set_quality()` returns `Result<()>` and updates compressor
- [ ] Quality values are clamped to 1-100 range
- [ ] Tests pass: `cargo test -p bucket-streamer encoder`

## Benchmark Commands

```bash
# Run encoding benchmark (baseline, no specific targets)
cargo test -p bucket-streamer --release -- --ignored --nocapture benchmark
```

## Context

### Why TurboJPEG?
- 2-3x faster than image-rs jpeg encoder
- Direct YUV input (no RGB conversion needed)
- Hardware-optimized (SIMD via NASM)
- Used by major video applications

### YuvImage and YUV420P
TurboJPEG's `YuvImage` expects:
- Contiguous buffer with Y, U, V planes stored sequentially
- `align: 1` for tightly packed data (no row padding)
- `subsamp: Subsamp::Sub2x2` for 4:2:0 subsampling

This matches the `DecodedFrame` output from Task 09's decoder.

### Quality vs Size Tradeoffs

| Quality | Typical Size (1080p) | Use Case |
|---------|---------------------|----------|
| 60 | 50-100 KB | Fast scrubbing |
| 80 | 100-200 KB | Normal viewing |
| 95 | 300-500 KB | High quality |

### Thread Safety
`Compressor` is `Send` but not `Sync`. Each session should have its own encoder instance.

### Encoder Reuse
Creating a new `JpegEncoder` takes ~0.1ms. For optimal performance:
- Reuse one encoder per session/pipeline
- Avoid creating new encoder per frame
- Safe to encode frames of different sizes with same encoder

### Memory Allocation
`compress_yuv_to_vec()` allocates output buffer. For high-performance scenarios where allocation overhead matters, consider `compress_yuv_to_owned()` which returns `OwnedBuf` managed by TurboJPEG.
