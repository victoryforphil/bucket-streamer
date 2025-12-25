# Test Plan: Tasks 12 & 12a - Streaming CLI and Flattened Offsets

## Implementation Status

### ✅ Completed
1. **Task 12a: Flattened Offsets Format**
   - Updated `FrameOffsets` struct in `crates/repo-cli/src/commands/convert.rs`
   - Changed from nested IRAP groups to flat array
   - Each frame now has explicit `irap_offset` field
   - Simplified structure: `{ video_url, frames: [{ offset, irap_offset }] }`

2. **Task 12: Streaming CLI Implementation**
   - Added dependencies: `futures-util`, `url`
   - Implemented full benchmark client in `crates/streaming-cli/src/main.rs`
   - Features: batch requests, latency tracking, frame saving, JSON output
   - Built successfully in release mode

### 🔄 Pending (Requires Video Conversion)
- Generate test H265 files with offsets

---

## Prerequisites

### 1. Convert Test Video and Generate Offsets

```bash
# Convert a test video and extract frame offsets with new format
cargo run -p repo-cli -- convert \
  --input data/test_dji_sfnight_1.mp4 \
  --output data/test.h265 \
  --extract-offsets \
  --force

# This creates:
# - data/test.h265 (converted video)
# - data/test.h265.offsets.json (flattened offset format)
```

### 2. Start the Server

Ensure `bucket-streamer` server is running with pipeline integration (Task 11):

```bash
# Terminal 1: Start server
cargo run -p bucket-streamer

# Server should be listening on ws://localhost:3000/ws
```

---

## Test Suite

### Test 1: Sequential Mode (Default, --batch 1)

**Purpose:** Measure true round-trip latency per frame

```bash
cargo run -p streaming-cli -- \
  --video data/test.h265 \
  --frames-file data/test.h265.offsets.json \
  --batch 1
```

**Expected Output:**
```
=== Benchmark Results ===
Frames requested: N
Frames received:  N
Frames errored:   0
Total time:       XXXms
Average FPS:      X.X
Total bytes:      XXX KB

Latency (ms):
  Avg: XX.XX
  Min: XX.XX
  Max: XX.XX
  P50: XX.XX
  P95: XX.XX
  P99: XX.XX
```

**Success Criteria:**
- ✅ All frames received (frames_received == frames_requested)
- ✅ No errors (frames_errored == 0)
- ✅ Latency stats calculated correctly
- ✅ Human-readable output format
- ✅ No crashes or connection drops

---

### Test 2: Batched Mode (--batch 10)

**Purpose:** Test throughput with parallel frame requests

```bash
cargo run -p streaming-cli -- \
  --video data/test.h265 \
  --frames-file data/test.h265.offsets.json \
  --batch 10
```

**Expected Behavior:**
- Higher FPS than sequential mode
- Latency measured from batch start to individual frame arrival
- All frames still received successfully

**Success Criteria:**
- ✅ Higher average FPS than batch=1
- ✅ All frames received
- ✅ P99 latency reasonable (< 1s for local server)

---

### Test 3: Frame Saving (-o flag)

**Purpose:** Verify JPEG frames are saved correctly

```bash
cargo run -p streaming-cli -- \
  --video data/test.h265 \
  --frames-file data/test.h265.offsets.json \
  --batch 5 \
  --output data/out/frames
```

**Verification Steps:**

```bash
# Check frames saved
ls -lh data/out/frames/

# Expected: frame_000000_<offset>.jpg, frame_000001_<offset>.jpg, etc.

# Verify JPEG format
file data/out/frames/frame_000000_*.jpg
# Expected: "JPEG image data"

# Open a frame visually (if display available)
# xdg-open data/out/frames/frame_000000_*.jpg
```

**Success Criteria:**
- ✅ Output directory created automatically
- ✅ One `.jpg` file per frame received
- ✅ Filenames follow pattern: `frame_{index:06d}_{offset}.jpg`
- ✅ Files are valid JPEG images
- ✅ File sizes reasonable (>1KB typically)

---

### Test 4: JSON Output (--json flag)

**Purpose:** Verify machine-readable output for scripting

```bash
cargo run -p streaming-cli -- \
  --video data/test.h265 \
  --frames-file data/test.h265.offsets.json \
  --batch 5 \
  --json > results.json
```

**Verification:**

```bash
# Validate JSON structure
cat results.json | jq .

# Check fields present
cat results.json | jq 'keys'
# Expected: [frames_requested, frames_received, frames_errored, 
#            total_time_ms, average_fps, latency_avg_ms, latency_min_ms,
#            latency_max_ms, latency_p50_ms, latency_p95_ms, 
#            latency_p99_ms, total_bytes]
```

**Success Criteria:**
- ✅ Valid JSON output (parseable by `jq`)
- ✅ No human-readable text mixed in
- ✅ All expected fields present
- ✅ Numeric values are numbers, not strings

---

### Test 5: Custom Server URL

**Purpose:** Verify connection to non-default server

```bash
cargo run -p streaming-cli -- \
  --url ws://localhost:3000/ws \
  --video data/test.h265 \
  --frames-file data/test.h265.offsets.json
```

**Success Criteria:**
- ✅ Connects successfully
- ✅ Works identically to default URL

---

### Test 6: Error Handling - Video Not Found

**Purpose:** Verify graceful error handling

```bash
cargo run -p streaming-cli -- \
  --video data/nonexistent.h265 \
  --frames-file data/test.h265.offsets.json
```

**Expected Output:**
```
Error: Video not found: data/nonexistent.h265
```

**Success Criteria:**
- ✅ Clear error message
- ✅ Non-zero exit code
- ✅ No panic or crash

---

### Test 7: Error Handling - Invalid Offsets File

**Purpose:** Verify input validation

```bash
# Create invalid JSON
echo "invalid json" > /tmp/bad.json

cargo run -p streaming-cli -- \
  --video data/test.h265 \
  --frames-file /tmp/bad.json
```

**Expected Output:**
```
Error: Failed to parse frames file
```

**Success Criteria:**
- ✅ Clear error message
- ✅ Non-zero exit code
- ✅ No panic

---

### Test 8: Error Handling - Server Offline

**Purpose:** Verify connection failure handling

```bash
# Stop the server first

cargo run -p streaming-cli -- \
  --video data/test.h265 \
  --frames-file data/test.h265.offsets.json
```

**Expected Output:**
```
Error: Failed to connect
```

**Success Criteria:**
- ✅ Clear error message
- ✅ Non-zero exit code
- ✅ No hang or timeout issues

---

### Test 9: Large Batch Size

**Purpose:** Stress test with high concurrency

```bash
cargo run -p streaming-cli -- \
  --video data/test.h265 \
  --frames-file data/test.h265.offsets.json \
  --batch 100
```

**Success Criteria:**
- ✅ All frames received
- ✅ No timeout errors
- ✅ Performance improvement over small batches

---

### Test 10: Hyperfine Integration

**Purpose:** Verify compatibility with benchmark tools

```bash
# Install hyperfine if not available
# sudo apt install hyperfine

hyperfine --warmup 2 \
  'cargo run -p streaming-cli --release -- \
     --video data/test.h265 \
     --frames-file data/test.h265.offsets.json'

# Compare batch sizes
hyperfine --parameter-list batch 1,5,10,20 \
  'cargo run -p streaming-cli --release -- \
     --video data/test.h265 \
     --frames-file data/test.h265.offsets.json \
     --batch {batch}'
```

**Success Criteria:**
- ✅ Runs without errors
- ✅ Shows timing statistics
- ✅ Results consistent across runs

---

## Validation Checklist

### Offset Format (Task 12a)

- [x] `FrameOffsets` has `video_url` and `frames` fields
- [x] `FrameEntry` has `offset` and `irap_offset` fields only
- [x] No nested `iraps` structure
- [x] Code compiles without errors
- [ ] Generated JSON matches new format (pending video conversion)
- [ ] Keyframes have `offset == irap_offset` (verify in generated file)

### CLI Features (Task 12)

- [x] Binary builds successfully
- [x] All dependencies added to Cargo.toml
- [x] CLI arguments match spec (url, video, frames-file, batch, json, output)
- [x] Default values correct (url, batch=1)
- [ ] Sequential mode works (--batch 1)
- [ ] Batched mode works (--batch N)
- [ ] Frame saving works (-o dir)
- [ ] JSON output works (--json)
- [ ] Error handling graceful

### Protocol Compliance

- [x] `ClientMessage::SetVideo` implemented
- [x] `ClientMessage::RequestFrames` implemented
- [x] `ServerMessage::VideoSet` handled
- [x] `ServerMessage::Frame` handled
- [x] `ServerMessage::FrameError` handled
- [x] `ServerMessage::Error` handled
- [x] Binary message pairing via queue

### Statistics

- [x] Latency tracking implemented
- [x] Percentiles calculated (P50, P95, P99)
- [x] FPS calculation correct
- [x] Total bytes tracked
- [x] Frame counts tracked (requested, received, errored)

---

## Performance Expectations

### Local Server (same machine)
- **Sequential (batch=1):** 10-30 FPS, latency 30-100ms
- **Batched (batch=10):** 50-100+ FPS, latency varies
- **P99 latency:** < 200ms typically

### Network Server (LAN)
- **Sequential:** 5-20 FPS, latency 50-200ms
- **Batched:** 20-50 FPS
- **P99 latency:** < 500ms typically

*Note: Actual performance depends on video complexity, hardware, and network conditions.*

---

## Debugging Tips

### CLI won't connect
```bash
# Check server is running
curl -I http://localhost:3000/

# Check WebSocket endpoint
wscat -c ws://localhost:3000/ws
```

### Frames not received
```bash
# Check server logs for errors
# Verify offsets file format is correct
cat data/test.h265.offsets.json | jq .

# Try with smaller batch size
--batch 1
```

### Invalid latency stats
- Ensure server responds promptly (no blocking operations)
- Check for clock skew if using remote server
- Verify batch size matches actual responses

---

## Next Steps

1. **Convert test video** and generate offsets with new format
2. **Run tests 1-10** in order
3. **Document results** and actual performance numbers
4. **Fix any issues** discovered during testing
5. **Consider extracting protocol types** to shared crate (noted TODO)

---

## Summary

**Implementation Complete:**
- ✅ Task 12a: Flattened offsets format
- ✅ Task 12: Streaming CLI benchmark client

**Ready for Testing:**
- All code compiles and builds
- Comprehensive test plan provided
- Error handling implemented
- All success criteria defined

**Blockers:**
- Need H265 video conversion to generate test offsets file
- Need running server with pipeline integration (Task 11)
