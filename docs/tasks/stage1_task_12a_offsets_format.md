# Task 12a: Flatten Offsets JSON Format

## Goal
Simplify the frame offsets JSON format from nested IRAP groups to a flat array. Each frame carries its own `irap_offset` for direct use by clients.

## Dependencies
- Task 03: H265 Converter (generates current nested format)

## Files to Modify

```
crates/repo-cli/src/commands/convert.rs   # Update offset extraction
data/test.h265.h265.offsets.json          # Regenerate with new format
```

## Format Change

### Old Format (Nested by IRAP)

```json
{
  "video_url": "fs:///path/to/video.h265",
  "frame_count": 50,
  "iraps": [
    {
      "offset": 0,
      "frames": [
        { "offset": 0, "size": 2448, "index": 0, "is_keyframe": true },
        { "offset": 2448, "size": 28, "index": 1, "is_keyframe": false }
      ]
    },
    {
      "offset": 6000,
      "frames": [
        { "offset": 6000, "size": 2500, "index": 25, "is_keyframe": true }
      ]
    }
  ]
}
```

### New Format (Flat Array)

```json
{
  "video_url": "fs:///path/to/video.h265",
  "frames": [
    { "offset": 0, "irap_offset": 0 },
    { "offset": 2448, "irap_offset": 0 },
    { "offset": 2476, "irap_offset": 0 },
    { "offset": 6000, "irap_offset": 6000 },
    { "offset": 7200, "irap_offset": 6000 }
  ]
}
```

### Key Differences

| Aspect | Old | New |
|--------|-----|-----|
| Structure | Nested by IRAP group | Flat array |
| `irap_offset` | Implicit (parent group) | Explicit per frame |
| `size` | Included | Removed (server-side only) |
| `index` | Included | Removed (assigned at runtime) |
| `is_keyframe` | Included | Derived: `offset == irap_offset` |
| `frame_count` | Included | Removed (use `frames.len()`) |

## Steps

### 1. Update Struct Definitions in convert.rs

Replace the nested structs:

```rust
// OLD structs to remove:
// - FrameOffsets (with iraps: Vec<IrapGroup>)
// - IrapGroup
// - FrameInfo

// NEW structs:
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
```

### 2. Update extract_frame_offsets Function

```rust
fn extract_frame_offsets(video_path: &str, storage_url: &str, output_path: &str) -> Result<usize> {
    ffmpeg::init()?;

    let mut ictx = ffmpeg::format::input(&video_path)
        .context("Failed to open video for offset extraction")?;

    let video_stream_index = ictx
        .streams()
        .best(ffmpeg::media::Type::Video)
        .map(|s| s.index())
        .context("No video stream found")?;

    let mut frames: Vec<FrameEntry> = Vec::new();
    let mut current_irap_offset: u64 = 0;

    for (stream, packet) in ictx.packets() {
        if stream.index() != video_stream_index {
            continue;
        }

        let offset = packet.pos().unwrap_or(0) as u64;
        let is_keyframe = packet.is_key();

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
```

### 3. Regenerate Test Data

After updating the code, regenerate the test offsets file:

```bash
cargo run -p repo-cli -- convert \
  --input data/test.mp4 \
  --output data/test.h265.h265 \
  --extract-offsets \
  --force
```

## Success Criteria

- [ ] `FrameOffsets` struct uses flat `frames: Vec<FrameEntry>` array
- [ ] Each `FrameEntry` has `offset` and `irap_offset` fields only
- [ ] Keyframes have `offset == irap_offset`
- [ ] `extract_frame_offsets` correctly tracks current IRAP offset
- [ ] Generated JSON matches new format
- [ ] `cargo test -p repo-cli` passes
- [ ] Test offsets file regenerated

## Context

### Why Flatten?

1. **Simpler client consumption** - No nested iteration needed
2. **Direct protocol mapping** - Frame entry maps 1:1 to `FrameRequest`
3. **Self-contained** - Each frame has all info needed to request it
4. **Smaller file** - Removed redundant `size`, `index`, `is_keyframe`

### Detecting Keyframes

Client can detect keyframes with:
```rust
let is_keyframe = frame.offset == frame.irap_offset;
```

### Backward Compatibility

This is a breaking change to the offsets format. Any existing offsets files need regeneration. Since we're pre-v1, this is acceptable.

### Protocol Alignment

This format change aligns with Task 06 protocol update where `FrameRequest` gains per-frame `irap_offset`:

```rust
// Protocol FrameRequest (Task 06 amendment)
struct FrameRequest {
    offset: u64,
    irap_offset: u64,
    index: u32,  // Assigned by client at runtime
}
```

The CLI reads the offsets file and adds `index` when building requests.
