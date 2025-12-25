# High Level Goals
- Given a s3 hosted H265 video file - respond to requests for invidual frames when given a specifc irap and frame byte offsets.
- Realtime performance (20FPS minimum)
- Scalable across kubernetes clusters
- Web client renderer

# Design Ideas
- Use a WebSocket connection to maitain session support and routing 
    - Offers live back-and-forth channel 
    - Client can send requests for given frames
    - Server can store reqeusts in LIFO queues to respond the clients latest request
    - As frames process - can send them out on WebSockets
    - If seperate from processing, one processing loop can respond to multiple clients.
- Main processing loop (conceptual):
    - LOAD_HEADER: Load File Header from S3 for `src_video`
    - LOAD_FRAME: Load frame data from S3 for `src_video`
    - DECODE_FRAME: Decode the frame requested 
    - ENCODE_FRAME: Encode the frame as the response time (E.g. JPEG)
- FFMpeg Session Note:
    - FFMpeg decoder creation takes time (~33ms) and was a bottle neck on the last design 
    - In theory if the clips are the same "specs" (resolution, format, colorspace, etc) then a single one can be re-used.
    - In reality we can more easily create a decoder per video clip.
- Loading when we need it:
    - `src_video` 
        - We know this early on - before even we start requesting frames. 
        - Can easily start to read from this "ahead of time" or do common tasks
        - can be used as a session creation (or higher level sessions discussed later)
    - Headers:
        - We can load these once we know `src_video` 
        - Saves a good 10-20ms
    - Frames: 
        - We at minimum have to respond to requested frames as we get them
        - However pre-loading or at least caching at a common layer could help.
- Pipeline approach
    - Ideally we can use a "streaming" / "pipeline" approach to this application
    - A core set of tasks (async) respond to a bus of events. 
    - We process data in a streaming / chunked fashion to get data back to the client ASAP
        - We can for example load sections of the s3 video file between IRAP frames 
            - _Should the clients send all the iraps ahead of time so we know the chunks?_
        - On each valid `video_chunk` (IRAP + its "delta" frames), we can decode into JPEG
        - We can emit a `FRAME_DONE` event that can be stored / cached and emitted to any "interested" parties
            - Interested Parties: Clients that have requested said frame. Pub/Sub pattern
        - If we are fancy we can respond in HTTP streamign
    - Use websockets for clients to connect to this bus system (or an sub-set of for the client) to operate live
- Client WebSocket flow
    - Clients send the list of clips they are interested in
    - Clients can send a "Register Chunk(s)" message to preload / register irap sections
    - Clients can then start to "Request Chunk" or "Request Frames" 
        - Chunk by irap and will send all the frames in said chunk
        - Frams uses Irap + byte offset and will said just the requested frames
    - Stream server will start the pipeline and convert frames - noting that WS session's "Interested frames"
    - Stream server sends frames to the session as they are converted.
        - In the future we should check a cache first (spread this as wide as possible)



        