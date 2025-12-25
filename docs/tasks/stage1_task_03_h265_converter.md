# Task 03: H.265 Converter Subcommand

## Goal
Implement `repo-cli convert` to transcode videos to H.265 MP4 and extract frame byte offsets using ffmpeg-next bindings.

## Dependencies
- Task 02: Repo CLI Base

## Files to Modify

```
crates/repo-cli/src/commands/convert.rs   # Full implementation
```

## Steps

### 1. Add required dependencies to repo-cli Cargo.toml

Ensure these are present:
```toml
[dependencies]
ffmpeg-next.workspace = true
```

### 2. Implement conversion using ffmpeg-next

```rust
use anyhow::{Context, Result};
use clap::Args;
use ffmpeg_next as ffmpeg;
use serde::Serialize;
use std::path::Path;

#[derive(Args)]
pub struct ConvertArgs {
    /// Input video file path
    #[arg(short, long)]
    pub input: String,

    /// Output file path (default: input with .h265.mp4 extension)
    #[arg(short, long)]
    pub output: Option<String>,

    /// Extract frame byte offsets to JSON sidecar
    #[arg(long)]
    pub extract_offsets: bool,

    /// Output as JSON (for scripting)
    #[arg(long)]
    pub json: bool,
}

#[derive(Serialize)]
struct ConvertResult {
    input: String,
    output: String,
    frames_extracted: Option<usize>,
    offsets_file: Option<String>,
}

#[derive(Serialize)]
struct FrameOffsets {
    video_path: String,
    iraps: Vec<IrapGroup>,
}

#[derive(Serialize)]
struct IrapGroup {
    offset: u64,
    frames: Vec<FrameInfo>,
}

#[derive(Serialize)]
struct FrameInfo {
    offset: u64,
    size: u64,
    index: u32,
    is_keyframe: bool,
}

pub fn run(args: ConvertArgs) -> Result<()> {
    ffmpeg::init().context("Failed to initialize FFmpeg")?;

    let output_path = args.output.unwrap_or_else(|| {
        let p = Path::new(&args.input);
        let stem = p.file_stem().unwrap().to_str().unwrap();
        p.with_file_name(format!("{}.h265.mp4", stem))
            .to_string_lossy()
            .to_string()
    });

    // Convert video
    convert_to_h265(&args.input, &output_path)?;

    let mut result = ConvertResult {
        input: args.input.clone(),
        output: output_path.clone(),
        frames_extracted: None,
        offsets_file: None,
    };

    // Extract offsets if requested
    if args.extract_offsets {
        let offsets_path = format!("{}.offsets.json", output_path);
        let count = extract_frame_offsets(&output_path, &offsets_path)?;
        result.frames_extracted = Some(count);
        result.offsets_file = Some(offsets_path);
    }

    if args.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
    } else {
        println!("Converted: {} -> {}", result.input, result.output);
        if let Some(count) = result.frames_extracted {
            println!("Extracted {} frame offsets to {}", count, result.offsets_file.unwrap());
        }
    }

    Ok(())
}

fn convert_to_h265(input: &str, output: &str) -> Result<()> {
    // Open input
    let mut ictx = ffmpeg::format::input(input)
        .context("Failed to open input file")?;

    let input_stream = ictx
        .streams()
        .best(ffmpeg::media::Type::Video)
        .context("No video stream found")?;
    let video_stream_index = input_stream.index();

    // Get decoder
    let decoder_ctx = ffmpeg::codec::context::Context::from_parameters(input_stream.parameters())?;
    let mut decoder = decoder_ctx.decoder().video()?;

    // Setup output
    let mut octx = ffmpeg::format::output(output)?;

    let codec = ffmpeg::encoder::find(ffmpeg::codec::Id::HEVC)
        .context("H.265/HEVC encoder not found")?;

    let mut output_stream = octx.add_stream(codec)?;
    
    let mut encoder = ffmpeg::codec::context::Context::new_with_codec(codec)
        .encoder()
        .video()?;

    encoder.set_width(decoder.width());
    encoder.set_height(decoder.height());
    encoder.set_format(decoder.format());
    encoder.set_time_base(input_stream.time_base());
    encoder.set_frame_rate(input_stream.avg_frame_rate());
    
    let encoder = encoder.open_as(codec)?;
    output_stream.set_parameters(&encoder);

    octx.write_header()?;

    // Transcode frames
    let mut frame = ffmpeg::frame::Video::empty();
    let mut packet = ffmpeg::Packet::empty();

    for (stream, input_packet) in ictx.packets() {
        if stream.index() != video_stream_index {
            continue;
        }

        decoder.send_packet(&input_packet)?;
        
        while decoder.receive_frame(&mut frame).is_ok() {
            encoder.send_frame(&frame)?;
            
            while encoder.receive_packet(&mut packet).is_ok() {
                packet.set_stream(0);
                packet.write_interleaved(&mut octx)?;
            }
        }
    }

    // Flush
    decoder.send_eof()?;
    while decoder.receive_frame(&mut frame).is_ok() {
        encoder.send_frame(&frame)?;
        while encoder.receive_packet(&mut packet).is_ok() {
            packet.set_stream(0);
            packet.write_interleaved(&mut octx)?;
        }
    }

    encoder.send_eof()?;
    while encoder.receive_packet(&mut packet).is_ok() {
        packet.set_stream(0);
        packet.write_interleaved(&mut octx)?;
    }

    octx.write_trailer()?;

    Ok(())
}

fn extract_frame_offsets(video_path: &str, output_path: &str) -> Result<usize> {
    let mut ictx = ffmpeg::format::input(video_path)?;
    
    let video_stream = ictx
        .streams()
        .best(ffmpeg::media::Type::Video)
        .context("No video stream")?;
    let stream_index = video_stream.index();

    let mut iraps: Vec<IrapGroup> = Vec::new();
    let mut current_irap: Option<IrapGroup> = None;
    let mut frame_index: u32 = 0;

    for (stream, packet) in ictx.packets() {
        if stream.index() != stream_index {
            continue;
        }

        let is_keyframe = packet.is_key();
        let offset = packet.position() as u64;
        let size = packet.size() as u64;

        let frame_info = FrameInfo {
            offset,
            size,
            index: frame_index,
            is_keyframe,
        };

        if is_keyframe {
            // Save previous IRAP group
            if let Some(group) = current_irap.take() {
                iraps.push(group);
            }
            // Start new group
            current_irap = Some(IrapGroup {
                offset,
                frames: vec![frame_info],
            });
        } else if let Some(ref mut group) = current_irap {
            group.frames.push(frame_info);
        }

        frame_index += 1;
    }

    // Save last group
    if let Some(group) = current_irap {
        iraps.push(group);
    }

    let offsets = FrameOffsets {
        video_path: video_path.to_string(),
        iraps,
    };

    let json = serde_json::to_string_pretty(&offsets)?;
    std::fs::write(output_path, json)?;

    Ok(frame_index as usize)
}
```

### 3. Handle edge cases

- Missing H.265 encoder: Error with helpful message
- Invalid input file: Context-wrapped error
- Overwrite protection: Optional `--force` flag

## Success Criteria

- [ ] `repo-cli convert -i video.mp4` produces `video.h265.mp4`
- [ ] Output video is playable and uses H.265 codec
- [ ] `--extract-offsets` creates `.offsets.json` sidecar file
- [ ] JSON contains IRAP groups with frame byte offsets
- [ ] `--json` outputs structured result
- [ ] Errors are informative (missing encoder, bad input, etc.)

## Test Video

<!-- TODO: Download instructions to be provided -->

For testing, you can use any MP4 file. The Big Buck Bunny trailer works well:
```bash
# Example placeholder - actual source TBD
wget -O data/test_input.mp4 "https://example.com/video.mp4"
repo-cli convert -i data/test_input.mp4 --extract-offsets
```

## Context

### Why ffmpeg-next instead of subprocess?
- Structured access to packet metadata (offsets, sizes, keyframe flags)
- No parsing of ffprobe text output
- Consistent with bucket-streamer decoder approach
- Type safety for all parameters

### Offset JSON Format

```json
{
  "video_path": "data/test.h265.mp4",
  "iraps": [
    {
      "offset": 48,
      "frames": [
        { "offset": 48, "size": 12543, "index": 0, "is_keyframe": true },
        { "offset": 12591, "size": 892, "index": 1, "is_keyframe": false },
        { "offset": 13483, "size": 1204, "index": 2, "is_keyframe": false }
      ]
    },
    {
      "offset": 156789,
      "frames": [
        { "offset": 156789, "size": 11234, "index": 30, "is_keyframe": true }
      ]
    }
  ]
}
```

### H.265 Encoder Availability
- libx265 is the common software encoder
- NVENC available if NVIDIA GPU present
- Encoder selection can be extended later with `--encoder` flag
