# **High-Performance Random Access Decoding of H.265 Streams from S3: Architectural Analysis and Optimization Strategies**

## **1\. Introduction: The Latency Challenge in Cloud Video Retrieval**

The manipulation and retrieval of video data in cloud environments typically follow two paradigms: whole-file processing (transcoding) or sequential streaming (playback). The requirement to provide **random access to specific frames based on byte offsets from an S3 object** represents a third, distinct challenge: low-latency sparse access. This scenario, common in non-linear video editing, forensic analysis, and machine learning data loading, demands the retrieval of Frame N without the computational or temporal cost of processing Frame 0 through N-1.

The current performance metric of approximately 10 Frames Per Second (FPS) using a naive Rust-based CLI wrapper suggests a pipeline dominated by initialization overhead and Input/Output (I/O) latency. The traditional approach—downloading a segment to disk, spawning an ffmpeg process, decoding, and re-encoding—introduces multiple serialization boundaries that are fatal to low-latency performance. Achieving a sub-100ms response time requires a granular analysis of the High Efficiency Video Coding (H.265/HEVC) standard, the Amazon S3 network protocol, and the internal state management of the FFmpeg libraries.

### **1.1 The Anatomy of the Bottleneck**

The user’s current workflow relies on a Request \-\> S3 GET \-\> Disk Write \-\> Process Spawn \-\> Decode \-\> Encode \-\> Response cycle. Every step involving disk I/O or process creation introduces millisecond-level delays that accumulate. In modern operating systems, process spawning alone can consume 10-20ms, while the initialization of the FFmpeg library—parsing global headers, allocating memory for codec contexts, and setting up internal tables—can exceed 50ms per invocation. When multiplied by the latency of establishing a new TCP/TLS connection to S3 for every frame, the theoretical maximum throughput is severely capped regardless of available bandwidth.

To transcend these limitations, the architecture must shift to a persistent, memory-centric model. This involves maintaining warm connections to the storage layer, performing zero-copy transfers of compressed bitstreams directly into the decoder’s memory space, and keeping the decoder context active and initialized between requests. The goal is to transform the operation from a "batch processing" model to a "stateful service" model.

### **1.2 The Inter-Frame Dependency Constraint**

H.265 is an inter-frame codec, meaning that unlike MJPEG or Intra-only formats, a generic frame (P or B frame) cannot be decoded in isolation. It contains only the differences—residuals and motion vectors—relative to reference frames. To decode a target frame, the decoder requires a sequence starting from the nearest preceding Keyframe or Intra Random Access Point (IRAP).

This implies that for every request, the system must retrieve and process a byte range defined by $$. The efficiency of the solution depends heavily on how quickly the system can ingest this range, feed it to the decoder, and discard the decoded pixel data for the frames preceding the target. This "decode-to-discard" mechanism is unavoidable in software decoding of inter-coded video but can be optimized to run faster than real-time by disabling loop filters and post-processing steps for the discarded frames.

### **1.3 Scope of Analysis**

This report analyzes the optimization of this pipeline across four vertical layers. First, the **Network Layer** analysis focuses on optimizing S3 interaction for partial content fetching using persistent connection pools. Second, the **Decoding Layer** examination details leveraging FFmpeg internals (AVCodecContext) for state reuse and implementing custom in-memory I/O via AVIOContext. Third, the **Hardware Acceleration** section evaluates the integration of NVIDIA NVDEC to offload computational density. Finally, the **Image Encoding** section explores high-performance JPEG compression using SIMD-optimized libraries to minimize the final serialization latency.

## ---

**2\. Network Transport and S3 Interaction Dynamics**

The initial stage of the random-access pipeline involves retrieving the raw H.265 byte stream from Amazon S3. In a scenario where the client requests a specific frame based on a known byte offset and the position of the last IRAP, the efficiency of the network layer is paramount. Amazon S3 functions as an object store rather than a file system, which necessitates specific strategies to minimize Time-To-First-Byte (TTFB) and maximize throughput for small, fragmented reads.

### **2.1 The Mechanics of Byte-Range Requests**

Amazon S3 supports HTTP Range requests, allowing clients to fetch specific byte segments of an object. For video decoding, fetching the entire file is inefficient and introduces unacceptable latency. Instead, the system must fetch the byte range starting from the last\_irap offset up to the end of the target frame. This range typically encompasses a single Group of Pictures (GOP) or a fraction thereof.

The precision of these requests is critical. The logic must calculate the specific range header: Range: bytes={irap\_offset}-{frame\_end\_offset}. The frame\_end\_offset is derived from the user's log file. In cases where the exact end offset is unknown, a read-ahead buffer strategy is required, where the client requests a chunk size statistically likely to contain the target frame (e.g., 2-5 MB for high-bitrate HEVC). However, over-fetching wastes bandwidth and processing time, while under-fetching incurs the high latency penalty of a second HTTP request.

### **2.2 Connection Pooling and Keep-Alive**

In a standard implementation using high-level SDKs or naive HTTP clients, the overhead of connection negotiation occurs for every request. This includes DNS resolution, the TCP handshake (SYN, SYN-ACK, ACK), and the TLS handshake (Client Hello, Server Hello, Key Exchange). For a 100ms latency budget, a TLS handshake alone can consume 30-50ms depending on geographical proximity and cipher suite complexity.

To mitigate this, the use of a **persistent connection pool** is mandatory. Rust’s aws-sdk-s3 (built on hyper and tokio) manages a pool of idle TCP connections. By sharing a single Client instance across the application, subsequent requests reuse existing TLS connections, eliminating the handshake overhead. This keeps the latency of fetching a frame purely dependent on the Round Trip Time (RTT) and the serialization speed of S3, significantly reducing TTFB.1

Benchmarks comparing Python’s boto3 and Rust implementations demonstrate that Rust-based clients consistently outperform Python equivalents in high-concurrency scenarios. Python implementations often suffer from the Global Interpreter Lock (GIL) and the overhead of the requests library, limiting the ability to saturate high-bandwidth links.2 For the user's requirement of "fastest options," avoiding the Python runtime for the network layer is a necessary architectural decision, validating the choice of Rust.

### **2.3 Zero-Copy Networking in Rust**

A significant bottleneck in the "naive" implementation is the writing of S3 data to disk or shared memory (/shm) before decoding. This introduces a read \-\> write \-\> read cycle that saturates the memory bus and adds filesystem syscall overhead.

The optimal approach is a **zero-copy architecture** where the bytes received from the socket are stored in a user-space memory buffer (e.g., a Bytes or Vec\<u8\> in Rust) and passed directly to the decoder. Rust’s ownership model and the bytes crate facilitate this by allowing immutable views into memory regions without duplication. When the S3 response body is received, it should be streamed directly into a pre-allocated buffer that acts as the input for the FFmpeg custom IO context. This bypasses the kernel page cache associated with file I/O entirely.3

### **2.4 Throughput vs. Latency Trade-offs in S3**

There is a non-linear relationship between chunk size and throughput. Very small chunks (e.g., \<1MB) suffer from protocol overhead dominance, where the time spent on headers and acknowledgments outweighs the data transfer time. Conversely, excessively large chunks increase the time before the decoder can receive sufficient data to begin processing.

Research indicates that for random access patterns, the "download buffer" strategy is critical. Rather than requesting only the exact bytes of the compressed packet, it is often more performant to request a slightly larger window or use a "read-ahead" buffer if the client is expected to scrub through the video.4 However, for pure random access where requests jump between disparate timestamps, the request must be precise. The latency of S3 is generally consistent, but "cold" objects (those not accessed recently) may incur a higher first-byte latency due to S3's internal tiering.

| Metric | Non-Pooled Connection | Persistent Connection Pool |
| :---- | :---- | :---- |
| **DNS Resolution** | 10 \- 50 ms | 0 ms (Cached) |
| **TCP Handshake** | 10 \- 30 ms | 0 ms (Keep-Alive) |
| **TLS Handshake** | 30 \- 100 ms | 0 ms (Session Reuse) |
| **Total Overhead** | **50 \- 180 ms** | **\< 1 ms** |

*Table 1: Comparative latency overhead of connection strategies.*

## ---

**3\. In-Memory FFmpeg Architecture**

The core of the solution lies in how FFmpeg is utilized to process the incoming byte stream. The standard CLI approach is process-heavy and ill-suited for low-latency random access because it initializes the entire library, parses global headers, and opens the codec context from scratch for every execution. To achieve high frame rates, the application must link against the FFmpeg libraries (libavcodec, libavformat) directly and manage the decoding state programmatically.

### **3.1 Custom AVIOContext for In-Memory Streaming**

One of the user's primary concerns was the perceived lack of support in Rust libraries for operating on in-memory streams. While libavformat typically expects a file path, it exposes an interface to define custom I/O operations via AVIOContext. This allows the application to present a block of memory (the video data fetched from S3) as a "file" to the demuxer.5

#### **3.1.1 Implementation Mechanism**

In C/C++ or Rust (via ffmpeg-sys-next), creating a custom AVIOContext involves allocating a buffer (using av\_malloc) and defining callback functions for read, write (optional), and seek.

* **Read Callback:** This function is called by FFmpeg when it needs more data. It must copy data from the application's S3 buffer into the buffer provided by FFmpeg.  
* **Seek Callback:** Crucial for MP4/MOV containers where metadata (moov atoms) might be located at the end of the file. However, since the user provides specific byte offsets from a log file, the application can often bypass the demuxing of the full container if strictly dealing with raw H.265 streams or if the offsets serve as packet boundaries.7

For ffmpeg-next in Rust, this requires handling unsafe pointers (void\* opaque) to pass the Rust reader structure into the C callback. Mismanagement here leads to segmentation faults. The opaque pointer usually points to the struct holding the S3 data. The read function casts this pointer back to the Rust struct and performs the memory copy.6 This mechanism eliminates the need for intermediate file storage, directly addressing the "write to disk" bottleneck identified in the query.

#### **3.1.2 Rust Implementation Details**

The integration requires a hybrid approach. While ffmpeg-next provides safe wrappers for high-level concepts like Packet and Frame, the custom I/O context often requires dropping down to ffmpeg-sys (the direct C bindings). The Rust application must allocate a memory buffer that outlives the AVIOContext. The read\_packet callback must be an extern "C" function that interfaces with the Rust Read trait.

Rust

unsafe extern "C" fn read\_packet(opaque: \*mut c\_void, buf: \*mut u8, buf\_size: c\_int) \-\> c\_int {  
    let reader \= &mut \*(opaque as \*mut Cursor\<&\[u8\]\>);  
    let buf\_slice \= slice::from\_raw\_parts\_mut(buf, buf\_size as usize);  
    match reader.read(buf\_slice) {  
        Ok(bytes) \=\> bytes as c\_int,  
        Err(\_) \=\> \-1 // AVERROR\_EOF  
    }  
}

This read\_packet function serves as the bridge, allowing FFmpeg to consume the S3 byte stream directly from RAM.

### **3.2 Decoder Context Reuse and Persistence**

A critical insight from the research is the cost of avcodec\_open2. Initializing an HEVC decoder involves allocating memory for reference frames (DPB \- Decoded Picture Buffer), parsing the Sequence Parameter Set (SPS) and Picture Parameter Set (PPS), and initializing internal tables. In a CLI workflow, this happens for *every* frame request, creating a massive performance penalty.

To maximize performance, the system should maintain a pool of open AVCodecContext instances. When a request for a frame arrives, a worker thread claims an idle decoder from the pool.

* **Flushing:** Before decoding unrelated data (i.e., a jump to a new random offset), the decoder must be flushed to clear internal buffers and reset the state. The function avcodec\_flush\_buffers(AVCodecContext \*avctx) is designed specifically for this purpose. It signals the decoder that the stream position has changed (seek) and that previous reference frames (unless they are part of the new sequence context) are invalid or should be cleared from the output queue.8  
* **Parameter Sets:** If the random access point (IRAP) relies on global headers (extradata) that were parsed once at startup, these remain valid in the persistent context. If the video stream changes resolution or parameters mid-stream (rare in stored VOD but possible), the decoder can usually handle reconfiguration, though explicit checks against AVCodecContext.width/height might be needed.11

#### **3.2.3 The "Draining" vs. "Flushing" Distinction**

It is vital to distinguish between draining and flushing.

* **Draining** involves sending NULL or an empty packet to avcodec\_send\_packet to retrieve buffered frames at the end of a stream.  
* **Flushing** (avcodec\_flush\_buffers) instantly resets the decoder, discarding internal data. For random access, we utilize flushing to prepare the "warm" decoder for a new byte range. This avoids the CPU overhead of avcodec\_close and avcodec\_open2.12

### **3.3 Rust Bindings: ffmpeg-next vs. ffmpeg-sys**

The user mentions ffmpeg-next. While ffmpeg-next provides a safe, idiomatic wrapper, it often lacks exposure for advanced features like custom AVIOContext constructors or granular control over buffer management. The research highlights that users often have to drop down to ffmpeg-sys (the direct FFI bindings) to implement custom IO or specialized flushing logic.

* **Recommendation:** Use ffmpeg-next for high-level structures (Packets, Frames) but use unsafe blocks with ffmpeg-sys APIs to set up the pb (packet buffer) field of the AVFormatContext with the custom AVIOContext. This hybrid approach provides safety where possible and flexibility where necessary.5

## ---

**4\. The HEVC Decoding Pipeline**

H.265/HEVC decoding is computationally intensive due to its complex coding tools (CTUs, SAO, etc.). Efficient random access requires handling the dependency structure of the video.

### **4.1 Handling Inter-Frame Dependencies**

The user has the offset of the "last IRAP" (Intra Random Access Point). This is crucial because an arbitrary frame N (P or B frame) depends on previous frames for reconstruction.

1. **Ingest:** The system reads bytes from IRAP\_Offset to Target\_Frame\_End.  
2. **Demux/Parse:** The AVPackets are extracted.  
3. **Decode Loop:**  
   * Packets from the IRAP onwards are sent to the decoder (avcodec\_send\_packet).  
   * Frames are received (avcodec\_receive\_frame).  
   * **Discard Logic:** Frames with timestamps (PTS) *prior* to the target frame's timestamp are discarded. This "decode-and-discard" process is unavoidable in software decoding of inter-coded video but is faster than network latency.  
   * **Target Frame:** Once the frame with the target PTS is emitted, it is captured for processing.

### **4.2 Software vs. Hardware Decoding**

The user's initial attempt yielded \~10 FPS, likely CPU-bound or I/O-bound by the disk. Software decoding (CPU) offers the highest compatibility. However, for H.265, hardware acceleration is transformative.

#### **4.2.1 NVIDIA NVDEC Integration**

NVIDIA's NVDEC engine can offload the entire decoding process. FFmpeg supports this via the \-hwaccel cuda or \-hwaccel cuvid options in CLI, and via AVHWDeviceContext in the C API.

* **Latency Trade-off:** NVDEC decodes into GPU memory (VRAM). To save the frame as a JPEG (usually done on CPU) or serve it to a web client, the frame must be copied back to system RAM (hwdownload). This PCIe transfer introduces latency.  
* **Optimization:** If the system is under heavy load (many concurrent requests), NVDEC is superior as it frees up the CPU for network handling and JPEG encoding. For single-request latency, a fast CPU (AVX2/AVX-512 optimized software decoder like libhevc) might rival NVDEC due to the absence of PCIe transfer overhead, but generally, NVDEC provides better throughput.15  
* **Implementation:** In Rust, this requires initializing an AVBufferRef for the hardware device and attaching it to the AVCodecContext. The decoded frame format will be AV\_PIX\_FMT\_CUDA or AV\_PIX\_FMT\_NV12. Converting NV12 to YUV420P or RGB is required before JPEG encoding.18

#### **4.2.2 Reusing NVDEC Contexts**

Similar to software decoders, NVDEC contexts should be reused. Destroying and recreating a CUDA context and decoder session is an expensive operation (tens to hundreds of milliseconds). The avcodec\_flush\_buffers call works with hardware decoders, allowing the same GPU session to handle a new sequence of NAL units.19

### **4.3 Low-Latency Decoding Flags**

Standard decoding prioritizes throughput and display order. For random access, latency is key.

* **AV\_CODEC\_FLAG\_LOW\_DELAY**: This flag (in AVCodecContext.flags) tells the decoder not to buffer frames for reordering if the codec syntax allows it.  
* **Threading:** FFmpeg supports thread\_type (Frame vs. Slice). FF\_THREAD\_FRAME adds latency (one frame delay per thread). FF\_THREAD\_SLICE is preferred for low latency as it parallelizes the decoding of a single frame, though it scales less linearly.21  
* **Probe Size:** Reducing probesize and analyzeduration on the AVFormatContext speeds up the initial stream info detection, although with known byte offsets and codec parameters, one can often skip avformat\_find\_stream\_info entirely by manually populating the AVStream parameters.22

## ---

**5\. Image Encoding and Color Space Conversion**

Once the raw video frame (YUV/NV12) is obtained, it must be converted to JPEG for browser display. This step is a frequent hidden bottleneck, often consuming more time than the decoding itself if unoptimized.

### **5.1 image-rs vs. libjpeg-turbo: The Performance Trap**

The user mentions image-rs. While idiomatic, the pure Rust image crate's JPEG encoder is significantly slower than libjpeg-turbo, which uses hand-written SIMD assembly (AVX2, NEON) for DCT and Huffman coding. Benchmarks indicate that libjpeg-turbo is consistently 2x to 10x faster than pure software implementations like mozjpeg (which optimizes for size, not speed) or older image-rs versions.23

For high-throughput applications, the use of **turbojpeg** (a Rust crate providing bindings to libjpeg-turbo) or **zune-jpeg** (a high-performance pure Rust alternative) is strongly recommended. zune-jpeg has recently emerged as a viable competitor, offering performance parity with libjpeg-turbo while maintaining memory safety, making it an attractive option for a pure Rust stack.25

### **5.2 Color Space Handling and Zero-Copy Encoding**

Video is typically stored in YUV420p (planar). JPEGs natively support YCbCr (effectively YUV).

* **Avoid RGB Conversion:** A common mistake is converting YUV \-\> RGB \-\> JPEG. This is wasteful. libjpeg-turbo can accept YUV planar data directly (tjCompressFromYUVPlanes). This skips the expensive YUV-to-RGB color matrix multiplication, saving several milliseconds per frame.27  
* **Zero-Copy to Encoder:** If using software decoding, the AVFrame data pointers (frame-\>data, frame-\>data, etc.) can be passed directly to the JPEG encoder's input function, avoiding an intermediate memory copy.28

## ---

**6\. Hardware Acceleration: Integration of NVIDIA NVDEC**

Given the computational intensity of H.265 decoding, especially at higher resolutions like 4K, software decoding on the CPU may become a bottleneck, particularly under high concurrency. NVIDIA’s NVDEC (NVIDIA Video Decoder) hardware offloading provides a mechanism to shift this load to the GPU.

### **6.1 The Mechanics of NVDEC in Rust**

FFmpeg interfaces with NVDEC via the hwaccel infrastructure. In a CLI context, this is invoked with \-hwaccel cuda. Programmatically in Rust, this requires a more explicit setup:

1. **Device Context Creation:** An AVHWDeviceContext must be created for the cuda device type using av\_hwdevice\_ctx\_create. This context represents the connection to the GPU driver.  
2. **Codec Context Attachment:** This device context is then attached to the AVCodecContext.hw\_device\_ctx.  
3. **Format Negotiation:** A get\_format callback must be implemented. When the decoder initializes, it proposes a list of pixel formats. The callback must select AV\_PIX\_FMT\_CUDA (or AV\_PIX\_FMT\_NV12 if supported directly) to enable the hardware path.17

### **6.2 Latency Trade-offs: The PCIe Bottleneck**

A critical architectural consideration with NVDEC is memory locality. The decoder outputs frames into GPU Video RAM (VRAM). To encode this frame as a JPEG for the web client, the data must typically be accessible to the CPU (unless using a GPU-accelerated JPEG encoder like NVJPEG).

* **HWDownload:** Moving the frame from VRAM to system RAM (hwdownload) involves a transfer over the PCIe bus. This introduces latency and synchronization overhead.  
* **Optimization:** For single-frame requests where latency is the absolute priority, a high-frequency CPU with AVX-512 instructions (using software decoding) might actually outperform NVDEC due to the absence of this PCIe round-trip. However, for aggregate throughput (serving many clients simultaneously), NVDEC is vastly superior as it leaves the CPU free to handle network I/O and JPEG encoding.16

### **6.3 Persistent Hardware Contexts**

Just like software decoders, the initialization of a CUDA context and the loading of decoder firmware on the GPU are slow operations (often taking 100ms+). Therefore, the "Persistent Worker Pool" strategy is even more critical when using NVDEC. The avcodec\_flush\_buffers function works seamlessly with hardware decoders, allowing the GPU session to be reset and reused for a new byte stream without the heavy initialization penalty.20

## ---

**7\. Alternative Architectures: GStreamer and Python**

While the user indicated a preference for Rust, they remained open to options. It is necessary to evaluate alternative stacks to ensure the Rust/FFmpeg approach is indeed optimal.

### **7.1 GStreamer: The Pipeline Approach**

GStreamer offers a pipeline-based architecture that can theoretically handle this workflow.

* **The Pipeline:** appsrc\! h265parse\! avdec\_h265\! jpegenc\! appsink.  
* **AppSrc:** This element allows the application to push data buffers (from S3) into the pipeline.  
* **Latency Challenges:** GStreamer is designed for continuous streaming. Managing state resets for random access is complex. To reuse a pipeline for a new frame, one must either perform a flushing seek or tag buffers with the DISCONT (discontinuity) flag. Failure to handle these flags correctly leads to the decoder waiting for timestamps to align or dropping "late" frames.30  
* **Conclusion:** While powerful, GStreamer's message-passing overhead and the complexity of managing pipeline state for single-frame extraction often result in higher latency than a tightly controlled, raw FFmpeg loop.

### **7.2 Python and Decord: Rapid Prototyping vs. Performance**

The decord library in Python is designed specifically for random access video decoding, often used in machine learning. It supports GPU acceleration and mimics a list-like access pattern (video).

* **Pros:** Extremely simple API; built-in S3 support (though often via local caching).  
* **Cons:** Python's Global Interpreter Lock (GIL) and the overhead of the Python runtime make it less suitable for a high-throughput, low-latency web service compared to Rust. Furthermore, decord's handling of direct S3 byte streams without intermediate files is less mature than aws-sdk-rust combined with FFmpeg.32 The Rust architecture offers finer-grained control over memory allocation and thread scheduling.

## ---

**8\. Proposed System Architecture: The "Worker Pool" Model**

To achieve the "fastest option," a request-response architecture using a persistent worker pool is recommended. This design amortizes the cost of initialization across thousands of requests.

### **8.1 System Components**

1. **Web Front-End (Actix-web/Axum):** A high-performance async web server handles incoming HTTP requests. It parses the last\_irap and offset parameters.  
2. **Decoder Pool:** A bounded set of worker threads (or async actors), each holding:  
   * An initialized AVCodecContext (HEVC).  
   * An initialized AVFormatContext with a recyclable custom AVIOContext.  
   * Pre-allocated buffers for I/O and decoding.  
3. **S3 Fetcher:** An async module using aws-sdk-s3 to fetch byte ranges efficiently.

### **8.2 Request Flow**

1. **Client Request:** GET /frame?irap=1000\&end=5000  
2. **Acquire Worker:** The web handler checks out a DecoderWorker from the pool. If the pool is empty, it waits (backpressure).  
3. **S3 Fetch:** Concurrently, the application fetches bytes 1000-5000 from S3 using the persistent connection pool.  
4. **IO Feed:** The bytes are wrapped in a Cursor or Bytes object and linked to the worker's AVIOContext via the opaque pointer mechanism.  
5. **Flush & Decode:**  
   * Call avcodec\_flush\_buffers on the worker's decoder.  
   * Pump packets from the AVIOContext into the decoder.  
   * Discard output frames until the frame with the target PTS is emitted.  
6. **Encode:** Pass the target AVFrame (YUV data) directly to turbojpeg.  
7. **Return:** Send the resulting JPEG bytes to the client.  
8. **Recycle:** The worker is returned to the pool (decoder remains open, context is flushed).

### **8.3 Implementation Stack**

* **Web Framework:** actix-web or axum for high-concurrency HTTP handling.34  
* **FFmpeg Binding:** ffmpeg-next for general API, mixed with ffmpeg-sys for the AVIOContext implementation.  
* **Image Encoding:** turbojpeg (preferred for speed) or zune-jpeg (for pure Rust safety).  
* **S3 Client:** aws-sdk-s3 with tokio.

## ---

**9\. Conclusion**

The "10 FPS" limitation experienced by the user is a symptom of a "Process-per-Frame" architecture that is fundamentally misaligned with the requirements of low-latency random access. Transitioning to a **Persistent Worker Pool** architecture in Rust will yield the fastest results.

The critical optimizations identified are:

1. **Network:** Use persistent S3 connections and fetch strict byte ranges (IRAP to Target) to minimize TTFB and data transfer.  
2. **Memory:** Implement Zero-Copy transfer from the Network Buffer to FFmpeg via custom AVIOContext.  
3. **Decoder:** Never close the decoder. Use avcodec\_flush\_buffers to reset state between requests, saving 50ms+ per frame.  
4. **Encoding:** Use turbojpeg to encode directly from the YUV output of the decoder, avoiding expensive RGB conversions.  
5. **Hardware:** Deploy NVDEC if aggregate throughput is the primary bottleneck, utilizing the same persistent context strategy.

By implementing this architecture, the system shifts from being bound by I/O latency to being bound only by network bandwidth and raw decode speed, potentially reducing per-frame latency to the 35-65ms range and increasing throughput by an order of magnitude.

### **Appendix: Performance Comparison**

| Component | "Naive" Implementation | Optimized (Worker Pool) |
| :---- | :---- | :---- |
| **S3 Connection** | New TLS Handshake (30-50ms) | Persistent Keep-Alive (\<1ms) |
| **Data Storage** | Disk Write \+ Read (5-10ms) | Zero-Copy RAM (0ms) |
| **Decoder Init** | avcodec\_open2 (10-30ms) | avcodec\_flush\_buffers (\<0.1ms) |
| **Header Parsing** | avformat\_find\_stream\_info (50ms+) | Cached Extradata (0ms) |
| **Image Encoding** | image-rs / RGB conv (10-20ms) | turbojpeg YUV (3-5ms) |
| **Total Latency** | **\~100-150ms** (10 FPS) | **\~35-50ms** (20-30 FPS) |

*Table 2: Estimated latency breakdown comparison.*

#### **Works cited**

1. Streaming large objects from S3 with ranged GET requests \- alexwlchan, accessed December 24, 2025, [https://alexwlchan.net/2019/streaming-large-s3-objects/](https://alexwlchan.net/2019/streaming-large-s3-objects/)  
2. Improving Python S3 Client Performance with Rust | by Joshua Robinson \- Medium, accessed December 24, 2025, [https://joshua-robinson.medium.com/improving-python-s3-client-performance-with-rust-e9639359072f](https://joshua-robinson.medium.com/improving-python-s3-client-performance-with-rust-e9639359072f)  
3. ffmpeg-next-io \- crates.io: Rust Package Registry, accessed December 24, 2025, [https://crates.io/crates/ffmpeg-next-io](https://crates.io/crates/ffmpeg-next-io)  
4. A Quick Look at S3 Read Speeds and Python Lambda Functions \- Bryson Tyrrell, accessed December 24, 2025, [https://bryson3gps.wordpress.com/2021/04/01/a-quick-look-at-s3-read-speeds-and-python-lambda-functions/](https://bryson3gps.wordpress.com/2021/04/01/a-quick-look-at-s3-read-speeds-and-python-lambda-functions/)  
5. ffmpeg\_sys\_next \- Rust \- Docs.rs, accessed December 24, 2025, [https://docs.rs/ffmpeg-sys-next](https://docs.rs/ffmpeg-sys-next)  
6. Ffi+ffmpeg recast opaque pointer to struct \- help \- The Rust Programming Language Forum, accessed December 24, 2025, [https://users.rust-lang.org/t/ffi-ffmpeg-recast-opaque-pointer-to-struct/20352](https://users.rust-lang.org/t/ffi-ffmpeg-recast-opaque-pointer-to-struct/20352)  
7. Using custom I/O callbacks with ffmpeg \- Coder's Diary, accessed December 24, 2025, [https://cdry.wordpress.com/2009/09/09/using-custom-io-callbacks-with-ffmpeg/](https://cdry.wordpress.com/2009/09/09/using-custom-io-callbacks-with-ffmpeg/)  
8. Utility functions \- FFmpeg, accessed December 24, 2025, [https://www.ffmpeg.org/doxygen/1.2/group\_\_lavc\_\_misc.html](https://www.ffmpeg.org/doxygen/1.2/group__lavc__misc.html)  
9. Utility functions \- FFmpeg, accessed December 24, 2025, [https://www.ffmpeg.org/doxygen/2.2/group\_\_lavc\_\_misc.html](https://www.ffmpeg.org/doxygen/2.2/group__lavc__misc.html)  
10. Utility functions \- FFmpeg, accessed December 24, 2025, [https://www.ffmpeg.org/doxygen/1.0/group\_\_lavc\_\_misc.html](https://www.ffmpeg.org/doxygen/1.0/group__lavc__misc.html)  
11. AVCodecContext Struct Reference \- FFmpeg, accessed December 24, 2025, [https://www.ffmpeg.org/doxygen/3.0/structAVCodecContext.html](https://www.ffmpeg.org/doxygen/3.0/structAVCodecContext.html)  
12. How to force AVCodecContext to release all references to any buffers \- Stack Overflow, accessed December 24, 2025, [https://stackoverflow.com/questions/78940432/how-to-force-avcodeccontext-to-release-all-references-to-any-buffers](https://stackoverflow.com/questions/78940432/how-to-force-avcodeccontext-to-release-all-references-to-any-buffers)  
13. FFmpeg What does flushing a codec do? \- Stack Overflow, accessed December 24, 2025, [https://stackoverflow.com/questions/65007067/ffmpeg-what-does-flushing-a-codec-do](https://stackoverflow.com/questions/65007067/ffmpeg-what-does-flushing-a-codec-do)  
14. ffmpeg\_sys\_next\_crossfix \- Rust \- Docs.rs, accessed December 24, 2025, [https://docs.rs/ffmpeg-sys-next-crossfix](https://docs.rs/ffmpeg-sys-next-crossfix)  
15. Using FFmpeg with NVIDIA GPU Hardware Acceleration, accessed December 24, 2025, [https://docs.nvidia.com/video-technologies/video-codec-sdk/12.0/ffmpeg-with-nvidia-gpu/index.html](https://docs.nvidia.com/video-technologies/video-codec-sdk/12.0/ffmpeg-with-nvidia-gpu/index.html)  
16. Implementing NVIDIA Video Codec SDK in Streaming Workflows \- Cincopa.com, accessed December 24, 2025, [https://www.cincopa.com/learn/implementing-nvidia-video-codec-sdk-in-streaming-workflows](https://www.cincopa.com/learn/implementing-nvidia-video-codec-sdk-in-streaming-workflows)  
17. Using FFmpeg with NVIDIA GPU Hardware Acceleration, accessed December 24, 2025, [https://docs.nvidia.com/video-technologies/video-codec-sdk/12.2/ffmpeg-with-nvidia-gpu/index.html](https://docs.nvidia.com/video-technologies/video-codec-sdk/12.2/ffmpeg-with-nvidia-gpu/index.html)  
18. How to hardware decode a h265 video, lower resolution and fps and pass on as raw video in yuv420 format \- Stack Overflow, accessed December 24, 2025, [https://stackoverflow.com/questions/71233586/how-to-hardware-decode-a-h265-video-lower-resolution-and-fps-and-pass-on-as-raw](https://stackoverflow.com/questions/71233586/how-to-hardware-decode-a-h265-video-lower-resolution-and-fps-and-pass-on-as-raw)  
19. Decoding \- FFmpeg, accessed December 24, 2025, [https://ffmpeg.org/doxygen/6.1/group\_\_lavc\_\_decoding.html](https://ffmpeg.org/doxygen/6.1/group__lavc__decoding.html)  
20. NVDEC Application Note \- NVIDIA Docs, accessed December 24, 2025, [https://docs.nvidia.com/video-technologies/video-codec-sdk/12.1/nvdec-application-note/index.html](https://docs.nvidia.com/video-technologies/video-codec-sdk/12.1/nvdec-application-note/index.html)  
21. ffmpeg-codecs(1) \- Debian Manpages, accessed December 24, 2025, [https://manpages.debian.org/unstable/ffmpeg/ffmpeg-codecs.1.en.html](https://manpages.debian.org/unstable/ffmpeg/ffmpeg-codecs.1.en.html)  
22. \[Libav-user\] Reducing latency of file opening (when playing from http), accessed December 24, 2025, [https://libav-user.ffmpeg.narkive.com/drDcW0Y6/reducing-latency-of-file-opening-when-playing-from-http](https://libav-user.ffmpeg.narkive.com/drDcW0Y6/reducing-latency-of-file-opening-when-playing-from-http)  
23. mozjpeg is an order of magnitude slower than libjpeg-turbo · Issue \#13 \- GitHub, accessed December 24, 2025, [https://github.com/mozilla/mozjpeg/issues/13](https://github.com/mozilla/mozjpeg/issues/13)  
24. Need for Speed: A Comprehensive Benchmark of JPEG Decoders in Pythonhttps://github.com/ternaus/imread\_benchmark \- arXiv, accessed December 24, 2025, [https://arxiv.org/html/2501.13131v1](https://arxiv.org/html/2501.13131v1)  
25. Announcing zune-jpeg: Rust's fastest JPEG decoder : r/rust \- Reddit, accessed December 24, 2025, [https://www.reddit.com/r/rust/comments/11f4jre/announcing\_zunejpeg\_rusts\_fastest\_jpeg\_decoder/](https://www.reddit.com/r/rust/comments/11f4jre/announcing_zunejpeg_rusts_fastest_jpeg_decoder/)  
26. zune-jpeg v0.4.13 is out, fixes rare decoding panics in \`image\` : r/rust \- Reddit, accessed December 24, 2025, [https://www.reddit.com/r/rust/comments/1e8npt1/zunejpeg\_v0413\_is\_out\_fixes\_rare\_decoding\_panics/](https://www.reddit.com/r/rust/comments/1e8npt1/zunejpeg_v0413_is_out_fixes_rare_decoding_panics/)  
27. turbojpeg \- Rust \- Docs.rs, accessed December 24, 2025, [https://docs.rs/turbojpeg](https://docs.rs/turbojpeg)  
28. Leveraging ffmpeg-next and image-rs for Multimedia Processing in Rust | by Alexis Kinsella, accessed December 24, 2025, [https://akinsella.medium.com/leveraging-ffmpeg-next-and-image-rs-for-multimedia-processing-in-rust-2097d1137d53?source=rss------programming-5](https://akinsella.medium.com/leveraging-ffmpeg-next-and-image-rs-for-multimedia-processing-in-rust-2097d1137d53?source=rss------programming-5)  
29. Optimizing Video Memory Usage with the NVDECODE API and NVIDIA Video Codec SDK, accessed December 24, 2025, [https://developer.nvidia.com/blog/optimizing-video-memory-usage-with-the-nvdecode-api-and-nvidia-video-codec-sdk/](https://developer.nvidia.com/blog/optimizing-video-memory-usage-with-the-nvdecode-api-and-nvidia-video-codec-sdk/)  
30. AppSrc in gstreamer\_app \- Rust, accessed December 24, 2025, [https://gstreamer.freedesktop.org/documentation//rust/git/docs/gstreamer\_app/struct.AppSrc.html](https://gstreamer.freedesktop.org/documentation//rust/git/docs/gstreamer_app/struct.AppSrc.html)  
31. GstBufferFlags (gstreamer.c.types.GstBufferFlags), accessed December 24, 2025, [https://api.gtkd.org/gstreamer.c.types.GstBufferFlags.html](https://api.gtkd.org/gstreamer.c.types.GstBufferFlags.html)  
32. dmlc/decord: An efficient video loader for deep learning with smart shuffling that's super easy to digest \- GitHub, accessed December 24, 2025, [https://github.com/dmlc/decord](https://github.com/dmlc/decord)  
33. Extremely slow accurate seek · Issue \#111 · dmlc/decord \- GitHub, accessed December 24, 2025, [https://github.com/dmlc/decord/issues/111](https://github.com/dmlc/decord/issues/111)  
34. actix\_web \- Rust \- Docs.rs, accessed December 24, 2025, [https://docs.rs/actix-web](https://docs.rs/actix-web)  
35. Robust State Management in Actix Web and Axum Applications | Leapcell, accessed December 24, 2025, [https://leapcell.io/blog/robust-state-management-in-actix-web-and-axum-applications](https://leapcell.io/blog/robust-state-management-in-actix-web-and-axum-applications)