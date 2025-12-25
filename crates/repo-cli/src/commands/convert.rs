use anyhow::{Context, Result};
use clap::Args;
use ffmpeg_next as ffmpeg;
use indicatif::{ProgressBar, ProgressStyle};
use serde::Serialize;
use std::path::Path;
use std::sync::Arc;

use super::output::CommandOutput;
use crate::error::CliError;

//=============================================================================
// Args
//=============================================================================

#[derive(Args, Debug)]
pub struct ConvertArgs {
    /// Input: file path (single mode) or directory path (batch mode with -R)
    #[arg(short, long)]
    pub input: String,

    /// Enable recursive batch conversion of all .mp4 files in directory
    #[arg(short = 'R', long)]
    pub recursive: bool,

    /// Output file path (single mode) or output directory (batch mode)
    /// Default: single mode uses input.h265, batch mode uses input directory
    #[arg(short, long)]
    pub output: Option<String>,

    /// Extract frame byte offsets to JSON sidecar
    #[arg(long)]
    pub extract_offsets: bool,

    /// Storage URL for the output video (used in offset JSON)
    /// For S3: s3://bucket/path/video.h265
    /// For local: fs:///absolute/path/video.h265 (auto-generated if not specified)
    #[arg(long)]
    pub storage_url: Option<String>,

    /// Overwrite output file if it exists
    #[arg(long)]
    pub force: bool,

    /// Enable GPU acceleration (NVENC) for faster encoding
    #[arg(long)]
    pub gpu: bool,

    /// Downscale video by integer divisor (2 = half resolution, 4 = quarter)
    #[arg(long)]
    pub downscale: Option<u32>,

    /// Target output framerate (e.g., 30, 24, 15)
    #[arg(long)]
    pub fps: Option<f64>,
}

//=============================================================================
// Output Types
//=============================================================================

#[derive(Serialize)]
struct ConvertResult {
    input: String,
    output: String,
    storage_url: String,
    frame_count: usize,
    offsets_file: Option<String>,
}

#[derive(Serialize)]
struct FrameOffsets {
    /// S3 URL or fs:// URL for the video file
    video_url: String,
    /// All frames with their IRAP offsets
    frames: Vec<FrameEntry>,
}

#[derive(Serialize)]
struct FrameEntry {
    /// Byte offset of this frame in the file
    offset: u64,
    /// Byte offset of the IRAP (keyframe) needed to decode this frame
    irap_offset: u64,
}

//=============================================================================
// Batch Processing Types
//=============================================================================

/// Result of a single file conversion in batch mode
#[derive(Serialize, Clone)]
struct BatchFileResult {
    input: String,
    output: Option<String>,
    frame_count: Option<usize>,
    status: BatchStatus,
    error: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "lowercase")]
enum BatchStatus {
    Success,
    Failed,
    Skipped,
}

/// Summary of batch conversion
#[derive(Serialize)]
struct BatchSummary {
    total: usize,
    successful: usize,
    failed: usize,
    skipped: usize,
    results: Vec<BatchFileResult>,
}

//=============================================================================
// Validation
//=============================================================================

/// Validates input file exists and has valid extension
fn validate_input(path: &str) -> Result<()> {
    let p = Path::new(path);

    if !p.exists() {
        return Err(CliError::FileNotFound(path.to_string()).into());
    }

    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .ok_or_else(|| CliError::InvalidInput(format!("No extension found: {}", path)))?;

    if !matches!(ext.to_lowercase().as_str(), "mp4" | "mov" | "h265") {
        return Err(CliError::InvalidExtension(path.to_string()).into());
    }

    Ok(())
}

/// Check if output exists (error if not --force)
fn check_output_exists(path: &str, force: bool) -> Result<()> {
    if Path::new(path).exists() && !force {
        return Err(CliError::OutputExists(path.to_string()).into());
    }
    Ok(())
}

/// Determine output path: replace extension with .h265
fn determine_output(input: &str) -> String {
    Path::new(input)
        .with_extension("h265")
        .to_string_lossy()
        .to_string()
}

/// Generate storage URL if not provided
/// Uses fs:// prefix with absolute path for local files
fn determine_storage_url(output_path: &str, provided: Option<&str>) -> Result<String> {
    if let Some(url) = provided {
        return Ok(url.to_string());
    }

    // Default to fs:// with absolute path
    let abs_path =
        std::fs::canonicalize(output_path).context("Failed to get absolute path for output")?;
    Ok(format!("fs://{}", abs_path.display()))
}

/// Validates downscale divisor and checks minimum output dimensions
fn validate_downscale(divisor: u32, width: u32, height: u32) -> Result<()> {
    // Must be power of 2
    if !divisor.is_power_of_two() {
        return Err(CliError::InvalidInput(format!(
            "Downscale divisor must be power of 2, got {}",
            divisor
        ))
        .into());
    }

    // Check minimum dimensions (64x64)
    if width / divisor < 64 || height / divisor < 64 {
        return Err(CliError::InvalidInput(format!(
            "Downscaled dimensions {}x{} too small (min 64x64)",
            width / divisor,
            height / divisor
        ))
        .into());
    }

    Ok(())
}

/// Validates target FPS is positive and not higher than input
fn validate_fps(fps: f64, input_fps: f64) -> Result<()> {
    if fps <= 0.0 {
        return Err(
            CliError::InvalidInput(format!("FPS must be positive, got {}", fps)).into(),
        );
    }

    if fps > input_fps {
        return Err(CliError::InvalidInput(format!(
            "Target FPS {} exceeds input FPS {:.2}",
            fps, input_fps
        ))
        .into());
    }

    Ok(())
}

//=============================================================================
// Batch Mode Validation
//=============================================================================

/// Validate batch mode: input must be a directory
fn validate_batch_input(path: &str) -> Result<()> {
    let p = Path::new(path);
    if !p.exists() {
        return Err(CliError::FileNotFound(path.to_string()).into());
    }
    if !p.is_dir() {
        return Err(CliError::InvalidInput(format!(
            "Batch mode (-R) requires directory, got file: {}",
            path
        ))
        .into());
    }
    Ok(())
}

/// Scan directory recursively for all .mp4 files
fn find_mp4_files(dir: &str) -> Result<Vec<String>> {
    let mut mp4_files = Vec::new();

    for entry in std::fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_file() {
            if let Some(ext) = path.extension() {
                if ext.to_str() == Some("mp4") {
                    mp4_files.push(path.to_string_lossy().to_string());
                }
            }
        } else if path.is_dir() {
            // Recursive scan
            if let Some(path_str) = path.to_str() {
                mp4_files.extend(find_mp4_files(path_str)?);
            }
        }
    }

    // Sort for consistent ordering
    mp4_files.sort();
    Ok(mp4_files)
}

//=============================================================================
// Progress Callback Type
//=============================================================================

/// Progress callback type for reporting transcoding progress
type ProgressCallback = Arc<dyn Fn(u64, u64) + Send + Sync>;

//=============================================================================
// FFmpeg Transcoding
//=============================================================================

/// Select encoder based on GPU acceleration preference
///
/// Returns the encoder codec and the appropriate pixel format to use.
/// If GPU is requested but NVENC is unavailable, falls back to CPU with a warning.
fn select_encoder(use_gpu: bool) -> Result<(ffmpeg::codec::codec::Codec, ffmpeg::format::Pixel)> {
    if use_gpu {
        if let Some(codec) = ffmpeg::encoder::find_by_name("hevc_nvenc") {
            println!("Using NVENC GPU encoder with p7 preset and CRF 9");
            return Ok((codec, ffmpeg::format::Pixel::NV12));
        } else {
            eprintln!("Warning: NVENC not available, falling back to CPU encoding");
        }
    }

    // Default: CPU encoding with libx265
    println!("Using libx265 CPU encoder with CRF 9 (near-lossless quality)");
    let codec = ffmpeg::encoder::find(ffmpeg::codec::Id::HEVC).ok_or(CliError::EncoderNotFound)?;
    Ok((codec, ffmpeg::format::Pixel::YUV420P))
}

/// Transcode video to H.265 format (video only, no audio)
///
/// Runs in a blocking task since FFmpeg operations are CPU-intensive.
/// Reports progress via callback with (current_frame, total_frames).
fn convert_to_h265(
    input: &str,
    output: &str,
    use_gpu: bool,
    downscale: Option<u32>,
    target_fps: Option<f64>,
    progress: Option<ProgressCallback>,
) -> Result<usize> {
    ffmpeg::init().context("Failed to initialize FFmpeg")?;

    // Open input
    let ictx = ffmpeg::format::input(input).context("Failed to open input file")?;

    // Find video stream
    let input_stream = ictx
        .streams()
        .best(ffmpeg::media::Type::Video)
        .ok_or(CliError::NoVideoStream)?;

    let video_stream_index = input_stream.index();
    let time_base = input_stream.time_base();

    // Estimate total frames from duration and frame rate
    let duration_secs = ictx.duration() as f64 / f64::from(ffmpeg::ffi::AV_TIME_BASE);
    let frame_rate = input_stream.avg_frame_rate();
    let estimated_frames = if frame_rate.1 > 0 {
        (duration_secs * frame_rate.0 as f64 / frame_rate.1 as f64) as u64
    } else {
        0
    };

    // Setup decoder
    // Note: CUVID hardware decoders can have compatibility issues with some codecs/containers
    // For now, stick with software decoding for maximum compatibility
    let mut decoder_ctx =
        ffmpeg::codec::context::Context::from_parameters(input_stream.parameters())
            .context("Failed to create decoder context")?;

    // Enable multi-threading for decoder (frame-level threading)
    // Cap at 16 threads (recommended max for most codecs)
    let thread_count = num_cpus::get().min(16);
    decoder_ctx.set_threading(ffmpeg::codec::threading::Config {
        kind: ffmpeg::codec::threading::Type::Frame,
        count: thread_count,
    });

    let mut decoder = decoder_ctx.decoder().video()?;

    // Select encoder based on GPU preference
    let (codec, target_pixel_format) = select_encoder(use_gpu)?;

    // Setup output container (video only)
    let mut octx = ffmpeg::format::output(output).context("Failed to create output file")?;

    let mut output_stream = octx.add_stream(codec)?;
    let output_stream_index = output_stream.index();
    let output_time_base = output_stream.time_base();

    // Setup encoder context
    let mut encoder_ctx = ffmpeg::codec::context::Context::new_with_codec(codec);

    // Enable multi-threading for encoder (only for CPU encoding - GPU handles its own parallelism)
    // x265 has a hard limit on frame threads, so cap at reasonable number
    if !use_gpu {
        encoder_ctx.set_threading(ffmpeg::codec::threading::Config {
            kind: ffmpeg::codec::threading::Type::Frame,
            count: thread_count.min(8), // x265 works best with fewer frame threads
        });
    }

    let mut encoder = encoder_ctx.encoder().video()?;

    // Calculate output dimensions (apply downscaling if requested)
    let (output_width, output_height) = if let Some(divisor) = downscale {
        validate_downscale(divisor, decoder.width(), decoder.height())?;
        let new_width = decoder.width() / divisor;
        let new_height = decoder.height() / divisor;
        println!(
            "Downscaling video: {}x{} -> {}x{} (divisor: {})",
            decoder.width(),
            decoder.height(),
            new_width,
            new_height,
            divisor
        );
        (new_width, new_height)
    } else {
        (decoder.width(), decoder.height())
    };

    // Configure encoder
    encoder.set_width(output_width);
    encoder.set_height(output_height);
    encoder.set_time_base(time_base);

    let frame_rate = input_stream.avg_frame_rate();
    if frame_rate.numerator() > 0 {
        encoder.set_frame_rate(Some(frame_rate));
    }

    // Use pixel format appropriate for the encoder (NV12 for GPU, YUV420P for CPU)
    encoder.set_format(target_pixel_format);

    // Open encoder with quality settings
    let mut encoder = if use_gpu {
        // GPU: p7 preset with CRF 9
        let mut opts = ffmpeg::Dictionary::new();
        opts.set("preset", "p7");
        opts.set("rc", "vbr");
        opts.set("cq", "9");
        opts.set("b:v", "0");
        encoder.open_with(opts)?
    } else {
        // CPU: CRF 9 with medium preset
        let mut opts = ffmpeg::Dictionary::new();
        opts.set("crf", "9");
        opts.set("preset", "medium");
        encoder.open_with(opts)?
    };

    // Set stream parameters from encoder
    output_stream.set_parameters(&encoder);

    // Setup scaler for pixel format conversion and/or downscaling
    let needs_scaling = downscale.is_some() || decoder.format() != target_pixel_format;
    let mut scaler = if needs_scaling {
        Some(ffmpeg::software::scaling::Context::get(
            decoder.format(),
            decoder.width(),
            decoder.height(),
            target_pixel_format,
            output_width,
            output_height,
            ffmpeg::software::scaling::Flags::BILINEAR,
        )?)
    } else {
        None
    };

    octx.write_header()?;

    // Calculate frame skip ratio for FPS decimation
    let input_fps = if frame_rate.1 > 0 {
        frame_rate.0 as f64 / frame_rate.1 as f64
    } else {
        30.0 // Default assumption
    };

    let frame_skip_ratio = if let Some(target_fps) = target_fps {
        validate_fps(target_fps, input_fps)?;
        println!(
            "FPS decimation: {:.2} fps -> {:.2} fps (keeping ~1 in every {:.1} frames)",
            input_fps,
            target_fps,
            input_fps / target_fps
        );
        input_fps / target_fps
    } else {
        1.0 // No skipping
    };

    let mut frame_count: usize = 0;
    let mut output_frame_count: usize = 0;
    let mut decoded_frame = ffmpeg::frame::Video::empty();
    let mut scaled_frame = ffmpeg::frame::Video::empty();

    // Re-open input for packet iteration (ictx was moved)
    let mut ictx = ffmpeg::format::input(input)?;

    // Process packets
    for (stream, packet) in ictx.packets() {
        if stream.index() != video_stream_index {
            continue; // Skip non-video (e.g., audio)
        }

        decoder.send_packet(&packet)?;

        while decoder.receive_frame(&mut decoded_frame).is_ok() {
            // FPS decimation: skip frames based on ratio
            if frame_skip_ratio > 1.0 {
                let should_keep_frame = (frame_count as f64 / frame_skip_ratio)
                    - (output_frame_count as f64)
                    >= -0.0001; // Small epsilon for floating point comparison
                frame_count += 1;

                if !should_keep_frame {
                    continue; // Skip this frame
                }
            }

            // Scale if needed, otherwise use decoded frame directly
            let frame_to_encode = if let Some(ref mut scaler) = scaler {
                scaler.run(&decoded_frame, &mut scaled_frame)?;
                scaled_frame.set_pts(decoded_frame.pts());
                &scaled_frame
            } else {
                &decoded_frame
            };

            encoder.send_frame(frame_to_encode)?;

            // Receive and write encoded packets
            let mut encoded_packet = ffmpeg::Packet::empty();
            while encoder.receive_packet(&mut encoded_packet).is_ok() {
                encoded_packet.set_stream(output_stream_index);
                encoded_packet.rescale_ts(time_base, output_time_base);
                encoded_packet.write_interleaved(&mut octx)?;
            }

            output_frame_count += 1;

            // Report progress
            if let Some(ref cb) = progress {
                cb(output_frame_count as u64, estimated_frames);
            }
        }
    }

    // Flush decoder
    decoder.send_eof()?;
    while decoder.receive_frame(&mut decoded_frame).is_ok() {
        // FPS decimation: skip frames based on ratio
        if frame_skip_ratio > 1.0 {
            let should_keep_frame =
                (frame_count as f64 / frame_skip_ratio) - (output_frame_count as f64) >= -0.0001;
            frame_count += 1;

            if !should_keep_frame {
                continue; // Skip this frame
            }
        }

        let frame_to_encode = if let Some(ref mut scaler) = scaler {
            scaler.run(&decoded_frame, &mut scaled_frame)?;
            scaled_frame.set_pts(decoded_frame.pts());
            &scaled_frame
        } else {
            &decoded_frame
        };

        encoder.send_frame(frame_to_encode)?;

        let mut encoded_packet = ffmpeg::Packet::empty();
        while encoder.receive_packet(&mut encoded_packet).is_ok() {
            encoded_packet.set_stream(output_stream_index);
            encoded_packet.rescale_ts(time_base, output_time_base);
            encoded_packet.write_interleaved(&mut octx)?;
        }

        output_frame_count += 1;
        if let Some(ref cb) = progress {
            cb(output_frame_count as u64, estimated_frames);
        }
    }

    // Flush encoder
    encoder.send_eof()?;
    let mut encoded_packet = ffmpeg::Packet::empty();
    while encoder.receive_packet(&mut encoded_packet).is_ok() {
        encoded_packet.set_stream(output_stream_index);
        encoded_packet.rescale_ts(time_base, output_time_base);
        encoded_packet.write_interleaved(&mut octx)?;
    }

    octx.write_trailer()?;

    Ok(output_frame_count)
}

//=============================================================================
// Frame Offset Extraction
//=============================================================================

/// Extract frame byte offsets from an H.265 video file
///
/// Reads packet metadata to build flat array with each frame's IRAP offset.
fn extract_frame_offsets(video_path: &str, storage_url: &str, output_path: &str) -> Result<usize> {
    ffmpeg::init().context("Failed to initialize FFmpeg")?;

    let mut ictx =
        ffmpeg::format::input(video_path).context("Failed to open video for offset extraction")?;

    let video_stream = ictx
        .streams()
        .best(ffmpeg::media::Type::Video)
        .ok_or(CliError::NoVideoStream)?;
    let stream_index = video_stream.index();

    let mut frames: Vec<FrameEntry> = Vec::new();
    let mut current_irap_offset: u64 = 0;

    for (stream, packet) in ictx.packets() {
        if stream.index() != stream_index {
            continue;
        }

        let is_keyframe = packet.is_key();
        let offset = packet.position();

        // Skip if position is unknown (-1)
        if offset < 0 {
            continue;
        }

        let offset = offset as u64;

        // Update IRAP offset when we hit a keyframe
        if is_keyframe {
            current_irap_offset = offset;
        }

        frames.push(FrameEntry {
            offset,
            irap_offset: current_irap_offset,
        });
    }

    let offsets = FrameOffsets {
        video_url: storage_url.to_string(),
        frames,
    };

    let json = serde_json::to_string_pretty(&offsets)?;
    std::fs::write(output_path, json)?;

    Ok(offsets.frames.len())
}

//=============================================================================
// Progress Bar
//=============================================================================

/// Creates a progress bar for transcoding
fn create_progress_bar(global: &crate::GlobalOpts, estimated_frames: u64) -> Option<ProgressBar> {
    if global.no_progress || estimated_frames == 0 {
        return None;
    }

    let pb = ProgressBar::new(estimated_frames);
    pb.set_style(
        ProgressStyle::with_template(
            "[{elapsed_precise}] {bar:40.cyan/blue} {pos:>7}/{len:7} frames ({percent}%) {msg}",
        )
        .unwrap()
        .progress_chars("##-"),
    );
    Some(pb)
}

//=============================================================================
// Single File Conversion
//=============================================================================

/// Convert a single file (used by both single and batch modes)
async fn convert_single_file(
    global: &crate::GlobalOpts,
    input: &str,
    output_path: &str,
    extract_offsets: bool,
    storage_url: Option<&str>,
    force: bool,
    gpu: bool,
    downscale: Option<u32>,
    fps: Option<f64>,
) -> Result<ConvertResult> {
    // Validate input file
    validate_input(input).context("Input validation failed")?;

    // Check if output exists (before doing any work)
    check_output_exists(output_path, force)?;

    // Get estimated frame count for progress bar
    let estimated_frames = {
        ffmpeg::init().ok();
        ffmpeg::format::input(input)
            .ok()
            .and_then(|ctx| {
                let stream = ctx.streams().best(ffmpeg::media::Type::Video)?;
                let duration = ctx.duration() as f64 / f64::from(ffmpeg::ffi::AV_TIME_BASE);
                let rate = stream.avg_frame_rate();
                if rate.1 > 0 {
                    Some((duration * rate.0 as f64 / rate.1 as f64) as u64)
                } else {
                    None
                }
            })
            .unwrap_or(0)
    };

    let pb = create_progress_bar(global, estimated_frames);

    // Clone paths for the blocking task
    let input_clone = input.to_string();
    let output_clone = output_path.to_string();

    // Create progress callback
    let pb_clone = pb.clone();
    let progress_cb: Option<ProgressCallback> = pb_clone.map(|pb| {
        Arc::new(move |current: u64, _total: u64| {
            pb.set_position(current);
        }) as ProgressCallback
    });

    // Run transcoding in blocking task
    let frame_count = tokio::task::spawn_blocking(move || {
        convert_to_h265(
            &input_clone,
            &output_clone,
            gpu,
            downscale,
            fps,
            progress_cb,
        )
    })
    .await
    .context("Transcoding task panicked")??;

    // Finish progress bar
    if let Some(pb) = pb {
        pb.finish_with_message("done");
    }

    // Extract offsets if requested
    let offsets_file = if extract_offsets {
        let offsets_path = format!("{}.offsets.json", output_path);

        // Get the actual storage URL (now that file exists, we can canonicalize)
        let final_storage_url = determine_storage_url(output_path, storage_url)?;

        extract_frame_offsets(output_path, &final_storage_url, &offsets_path)?;
        Some(offsets_path)
    } else {
        None
    };

    // Build result
    let final_storage_url = determine_storage_url(output_path, storage_url)?;

    Ok(ConvertResult {
        input: input.to_string(),
        output: output_path.to_string(),
        storage_url: final_storage_url,
        frame_count,
        offsets_file,
    })
}

//=============================================================================
// Batch Conversion
//=============================================================================

/// Run batch conversion on directory
async fn run_batch(
    global: &crate::GlobalOpts,
    input_dir: &str,
    output_dir: Option<&str>,
    extract_offsets: bool,
    storage_url: Option<&str>,
    force: bool,
    gpu: bool,
    downscale: Option<u32>,
    fps: Option<f64>,
) -> Result<BatchSummary> {
    // Find all .mp4 files
    let mp4_files = find_mp4_files(input_dir)?;

    if mp4_files.is_empty() {
        println!("No .mp4 files found in directory: {}", input_dir);
        return Ok(BatchSummary {
            total: 0,
            successful: 0,
            failed: 0,
            skipped: 0,
            results: vec![],
        });
    }

    let total_files = mp4_files.len();
    println!("Found {} .mp4 file(s) to convert\n", total_files);

    let mut results = Vec::new();
    let mut successful = 0;
    let mut failed = 0;
    let mut skipped = 0;

    // Determine output directory
    let out_dir = output_dir.unwrap_or(input_dir);

    // Process files sequentially to avoid progress bar conflicts
    for (idx, input_file) in mp4_files.iter().enumerate() {
        println!("[{}/{}] Converting: {}", idx + 1, total_files, input_file);

        // Determine output path for this file
        let output_path = Path::new(input_file)
            .file_name()
            .and_then(|n| n.to_str())
            .map(|name| {
                Path::new(out_dir)
                    .join(name)
                    .with_extension("h265")
                    .to_string_lossy()
                    .to_string()
            })
            .unwrap();

        // Check if output exists (skip unless --force)
        if Path::new(&output_path).exists() && !force {
            println!("  ⏭ Skipped (output exists, use --force to overwrite)\n");
            results.push(BatchFileResult {
                input: input_file.clone(),
                output: Some(output_path),
                frame_count: None,
                status: BatchStatus::Skipped,
                error: Some("Output file exists".to_string()),
            });
            skipped += 1;
            continue;
        }

        // Convert the file
        match convert_single_file(
            global,
            input_file,
            &output_path,
            extract_offsets,
            storage_url,
            force,
            gpu,
            downscale,
            fps,
        )
        .await
        {
            Ok(result) => {
                println!("  ✓ Success: {} frames\n", result.frame_count);
                results.push(BatchFileResult {
                    input: input_file.clone(),
                    output: Some(result.output),
                    frame_count: Some(result.frame_count),
                    status: BatchStatus::Success,
                    error: None,
                });
                successful += 1;
            }
            Err(e) => {
                println!("  ✗ Failed: {}\n", e);
                results.push(BatchFileResult {
                    input: input_file.clone(),
                    output: None,
                    frame_count: None,
                    status: BatchStatus::Failed,
                    error: Some(e.to_string()),
                });
                failed += 1;
            }
        }
    }

    Ok(BatchSummary {
        total: total_files,
        successful,
        failed,
        skipped,
        results,
    })
}

/// Print batch conversion summary
fn print_batch_summary(summary: &BatchSummary, json_output: bool) -> Result<()> {
    if json_output {
        let output_data = CommandOutput::success(serde_json::to_value(summary)?);
        println!("{}", serde_json::to_string_pretty(&output_data)?);
    } else {
        println!("╔════════════════════════════════════════════════════════════╗");
        println!("║              BATCH CONVERSION COMPLETE                     ║");
        println!("╠════════════════════════════════════════════════════════════╣");
        println!(
            "║ Total Files:     {:>5}                                     ║",
            summary.total
        );
        println!(
            "║ Successful:      {:>5} ✓                                  ║",
            summary.successful
        );
        println!(
            "║ Failed:          {:>5} ✗                                  ║",
            summary.failed
        );
        println!(
            "║ Skipped:         {:>5} ⏭                                   ║",
            summary.skipped
        );
        println!("╚════════════════════════════════════════════════════════════╝");

        // Show failed files if any
        if summary.failed > 0 {
            println!("\nFailed conversions:");
            for result in &summary.results {
                if matches!(result.status, BatchStatus::Failed) {
                    println!("  ✗ {}", result.input);
                    if let Some(ref err) = result.error {
                        println!("    Error: {}", err);
                    }
                }
            }
        }
    }

    Ok(())
}

//=============================================================================
// Main Run Function
//=============================================================================

pub async fn run(global: &crate::GlobalOpts, args: ConvertArgs) -> Result<()> {
    if args.recursive {
        // Batch mode
        validate_batch_input(&args.input)?;

        let summary = run_batch(
            global,
            &args.input,
            args.output.as_deref(),
            args.extract_offsets,
            args.storage_url.as_deref(),
            args.force,
            args.gpu,
            args.downscale,
            args.fps,
        )
        .await?;

        // Print batch summary
        print_batch_summary(&summary, global.json)?;
    } else {
        // Single file mode (existing behavior)
        let output = args.output.unwrap_or_else(|| determine_output(&args.input));

        let result = convert_single_file(
            global,
            &args.input,
            &output,
            args.extract_offsets,
            args.storage_url.as_deref(),
            args.force,
            args.gpu,
            args.downscale,
            args.fps,
        )
        .await?;

        // Output result
        if global.json {
            let output_data = CommandOutput::success(serde_json::to_value(&result)?);
            println!("{}", serde_json::to_string_pretty(&output_data)?);
        } else {
            println!("Converted: {} -> {}", result.input, result.output);
            println!("  Frames: {}", result.frame_count);
            println!("  Storage URL: {}", result.storage_url);
            if let Some(ref offsets) = result.offsets_file {
                println!("  Offsets: {}", offsets);
            }
        }
    }

    Ok(())
}
