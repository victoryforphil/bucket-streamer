# Idea: Raw Frame Delivery with WebCodecs Client-Side Decoding

**Status:** Research parking lot  
**Priority:** Future optimization  
**Created:** 2024-12-24

---

## Concept

Instead of the server performing JPEG encoding, send raw or minimally-compressed frame data and let the browser decode using the WebCodecs API.

### Current Flow (Stage 1)
```
Server: H.265 bytes → FFmpeg decode → YUV → TurboJPEG encode → JPEG
Client: JPEG → Browser decode → Canvas render
```

### Proposed Alternative Flow
```
Server: H.265 bytes → Pass through (or minimal processing)
Client: H.265 NAL units → WebCodecs VideoDecoder → VideoFrame → Canvas
```

---

## Potential Benefits

1. **Eliminates server-side JPEG encoding** (~3-5ms per frame saved)
2. **Client GPU handles decode** (offloads server CPU significantly)
3. **Better quality** - no lossy JPEG recompression artifacts
4. **Lower server CPU usage** - just byte shuffling, no transcoding
5. **Could enable higher frame rates** if server decode is the bottleneck

---

## Key Questions to Research

### 1. Browser HEVC/H.265 Support via WebCodecs

Which browsers support H.265 decoding via WebCodecs in 2025?

| Browser | HEVC WebCodecs | Hardware Decode | Notes |
|---------|----------------|-----------------|-------|
| Chrome  | ?              | ?               | Check `VideoDecoder.isConfigSupported()` |
| Safari  | ?              | ?               | Apple historically better HEVC support |
| Firefox | ?              | ?               | Historically avoided HEVC due to patents |
| Edge    | ?              | ?               | Chromium-based, likely follows Chrome |

**Research prompt:**
> "WebCodecs API HEVC H.265 browser support 2025. Which browsers support 
> VideoDecoder with 'hev1' or 'hvc1' codec string? Is hardware decode 
> available? What are the patent/licensing implications?"

### 2. Bandwidth Trade-offs

| Format | 1080p Frame Size | Notes |
|--------|------------------|-------|
| Raw YUV420 | ~3 MB | Uncompressed, impractical |
| JPEG q80 | ~50-150 KB | Current approach |
| H.265 GOP | ~50-500 KB | Depends on motion, GOP size |
| Single H.265 frame | Variable | P/B frames are tiny, I-frames larger |

**Key insight:** We're already fetching H.265 bytes from storage. If we could send the IRAP + delta frames directly to the client, we'd avoid both server-side decode AND encode.

**Questions:**
- Can we send individual NAL units to WebCodecs?
- Does the client need SPS/PPS/VPS headers for each chunk?
- What's the overhead of WebSocket framing for small NAL units?

### 3. WebCodecs API Specifics

```javascript
// Pseudocode - needs validation
const decoder = new VideoDecoder({
    output: (frame) => {
        // Render to canvas
        ctx.drawImage(frame, 0, 0);
        frame.close(); // Must release!
    },
    error: (e) => console.error(e),
});

await decoder.configure({
    codec: 'hev1.1.6.L93.B0', // HEVC Main profile
    codedWidth: 1920,
    codedHeight: 1080,
});

// Feed NAL units from server
decoder.decode(new EncodedVideoChunk({
    type: 'key',  // or 'delta'
    timestamp: 0,
    data: nalUnitBytes,
}));
```

**Questions:**
- What's the latency of `VideoDecoder.decode()`?
- How does `timestamp` work for random access (non-sequential frames)?
- Memory management: how quickly must we call `frame.close()`?
- Can we decode frames out of order?

### 4. Hybrid Approach

Could we support both paths?

```
Client capability detection:
  if (WebCodecs && HEVC supported) → raw H.265 path
  else → JPEG fallback path
```

Server could negotiate format on WebSocket connect:
```json
{ "type": "Capabilities", "formats": ["h265", "jpeg"] }
{ "type": "FormatSelected", "format": "h265" }
```

---

## Implementation Considerations

### Server Changes Required
- Skip FFmpeg decode entirely (or decode only for JPEG fallback)
- Parse MP4 to extract NAL units with proper boundaries
- Send codec configuration (SPS/PPS/VPS) to client on video set
- Frame messages include NAL unit type (keyframe vs delta)

### Client Changes Required
- WebCodecs VideoDecoder setup
- Capability detection and fallback
- Frame timestamp/ordering management
- Memory management (VideoFrame lifecycle)

### New Failure Modes
- Client decode failures (corrupted data, unsupported profile)
- Browser compatibility issues
- Mobile device thermal throttling with client-side decode

---

## Why Not Now (Stage 1)

1. **Adds significant client-side complexity** - Stage 1 is about proving server pipeline
2. **Browser support still inconsistent** for HEVC specifically
3. **JPEG path is proven and simple** to benchmark
4. **Server-side decode gives us control** over error handling
5. **Can add as optimization layer later** without changing server architecture

---

## When to Revisit

- [ ] After Stage 1 benchmarks show JPEG encoding is a significant bottleneck
- [ ] If server CPU usage is high but bandwidth is plentiful
- [ ] When targeting specific known-good browsers (e.g., Safari-only deployment)
- [ ] If we need to reduce server costs at scale

---

## Research Tasks for Future

1. **Browser compatibility matrix**: Test `VideoDecoder.isConfigSupported()` across browsers with HEVC codec strings

2. **Latency comparison**: Build prototype measuring:
   - Server JPEG encode + network + browser decode
   - vs. Network + client WebCodecs decode
   
3. **NAL unit extraction**: Research how to cleanly extract NAL units from MP4 container for streaming

4. **Error resilience**: What happens when a NAL unit is corrupted? Does WebCodecs recover?

---

## Related Resources

- [WebCodecs API Spec](https://www.w3.org/TR/webcodecs/)
- [WebCodecs Explainer](https://github.com/w3c/webcodecs/blob/main/explainer.md)
- [Can I Use: WebCodecs](https://caniuse.com/webcodecs)
- [HEVC Patent Licensing](https://en.wikipedia.org/wiki/High_Efficiency_Video_Coding#Patent_licensing)

---

## Research Prompt Template

For future deep-dive:

> "WebCodecs API 2025: How to decode H.265/HEVC video frames in browser 
> using VideoDecoder? Can individual NAL units be fed to VideoDecoder 
> for random-access frame extraction? What's the decode latency compared 
> to receiving pre-encoded JPEG frames over WebSocket? Show JavaScript 
> example of:
> 1. Checking HEVC support with isConfigSupported()  
> 2. Configuring VideoDecoder for HEVC Main profile
> 3. Feeding EncodedVideoChunk with NAL unit data
> 4. Rendering decoded VideoFrame to canvas
> 5. Proper memory management with frame.close()"
