# Idea: Pure Rust MP4 Demuxing vs FFmpeg

**Status:** Decision documented  
**Decision:** Use FFmpeg for Stage 1  
**Created:** 2024-12-24

---

## Context

Our video files are H.265/HEVC streams in MP4 containers. We need to:
1. **Demux**: Extract compressed video data (NAL units) from MP4 container
2. **Decode**: Decompress H.265 NAL units into raw YUV frames

This document compares two approaches for the demuxing step.

---

## Option A: FFmpeg for Everything

Use `ffmpeg-next` / `ffmpeg-sys-next` for both demuxing and decoding.

### Architecture
```
MP4 bytes → AVFormatContext (demux) → AVPacket → AVCodecContext (decode) → AVFrame
```

### Pros
- **Single dependency** for container + codec handling
- **Battle-tested** - handles weird MP4 variants, fragmented MP4, etc.
- **Same AVIOContext** works for both demux and decode (in-memory streaming)
- **Less integration work** - packets flow directly to decoder
- **Already needed** for H.265 decoding anyway

### Cons
- Heavier binary size (~20-50MB depending on FFmpeg build)
- FFmpeg's MP4 parser does more than we strictly need
- If FFmpeg demuxing has issues, we're blocked on both demux AND decode
- Harder to debug (C library, fewer Rust-native tools)

### Risk Level: **Low-Medium**
- Widely used, well-documented
- Risk is in the monolithic nature - one dependency for everything

---

## Option B: `mp4` Crate for Demux, FFmpeg for Decode

Use pure Rust `mp4` crate for container parsing, then feed extracted NAL units to FFmpeg decoder.

### Architecture
```
MP4 bytes → mp4::Mp4Reader (demux) → Vec<u8> NAL units → AVCodecContext (decode) → AVFrame
```

### Pros
- **Pure Rust demuxing** - safer, easier to debug
- **Separation of concerns** - can diagnose demux vs decode issues independently
- **Lighter weight** for the parsing phase
- **`HevcConfig` struct** exists - explicit HEVC support in the crate
- **Could eventually eliminate FFmpeg** if we move to WebCodecs

### Cons
- **Only 1.87% documented** (docs.rs metric) - may hit undocumented edge cases
- **Manual NAL unit handling** - need to extract and feed to FFmpeg correctly
- **Two codebases to understand** - `mp4` crate + FFmpeg internals
- **May not handle all MP4 variants** - robot cameras may produce quirky files
- **More glue code** - bridging mp4 output to FFmpeg input

### Risk Level: **Medium**
- Less battle-tested than FFmpeg for edge cases
- Documentation gaps are concerning for Stage 1

---

## Decision: Option A (FFmpeg for Everything)

### Rationale

1. **Reduce variables** when debugging the critical AVIOContext spike
2. **Robot camera MP4s might have quirks** that FFmpeg handles but `mp4` crate doesn't
3. **1.87% documentation** on `mp4` crate is a red flag for a prototype
4. **Unified in-memory path** - same AVIOContext serves demux and decode
5. **We can revisit later** without architectural changes

---

## When to Revisit

Consider switching to pure Rust demuxing if:

- [ ] **FFmpeg demuxing becomes a bottleneck** (unlikely - parsing is fast)
- [ ] **We want to eliminate FFmpeg entirely** (WebCodecs future - see `idea_webcodecs_raw_frames.md`)
- [ ] **We hit FFmpeg bugs** that a simpler parser would avoid
- [ ] **Binary size becomes critical** for edge deployment
- [ ] **`mp4` crate matures** with better documentation and broader testing

---

## Migration Path (If Needed)

The switch would be isolated to the `pipeline/decoder.rs` module:

```rust
// Current: FFmpeg demux + decode
pub struct Decoder {
    format_ctx: AVFormatContext,  // Demuxing
    codec_ctx: AVCodecContext,    // Decoding
    avio_ctx: AVIOContext,        // In-memory IO
}

// Future: mp4 crate demux + FFmpeg decode
pub struct Decoder {
    mp4_reader: mp4::Mp4Reader<Cursor<Bytes>>,  // Demuxing
    codec_ctx: AVCodecContext,                   // Decoding (still FFmpeg)
    // No AVIOContext needed for demux
}
```

The rest of the pipeline (fetcher, encoder, WebSocket) wouldn't change.

---

## Alternative Crates to Watch

| Crate | Description | Docs Coverage | Notes |
|-------|-------------|---------------|-------|
| `mp4` | MP4 reader/writer | 1.87% | Has HevcConfig, low docs |
| `mp4parse` | Mozilla's MP4 parser | ~50% | Used in Firefox, more mature |
| `matroska` | MKV parser | ~30% | If we ever support MKV |
| `symphonia` | Pure Rust media framework | ~60% | Growing ecosystem, worth watching |

---

## Research Prompt (For Future)

If we decide to explore pure Rust demuxing:

> "Rust MP4 demuxing for H.265/HEVC: Compare mp4 crate vs mp4parse vs 
> symphonia for extracting NAL units from MP4 container. Which handles:
> 1. Fragmented MP4 (fMP4)
> 2. Multiple video tracks
> 3. Seeking to specific byte offsets
> 4. Extracting SPS/PPS/VPS parameter sets
> Show example of reading MP4 and extracting raw H.265 NAL units 
> suitable for feeding to a decoder."
