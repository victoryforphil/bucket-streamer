# Task 06: WebSocket Protocol Types

## Goal
Define `ClientMessage` and `ServerMessage` enums with Serde serialization for WebSocket communication protocol.

## Dependencies
- Task 01: Project Skeleton

## Files to Modify

```
crates/bucket-streamer/src/server/protocol.rs   # Full implementation
crates/bucket-streamer/src/server/mod.rs        # Export types
```

## Steps

### 1. Update server/mod.rs

```rust
pub mod protocol;
pub mod router;
pub mod websocket;

pub use protocol::{ClientMessage, ServerMessage, FrameRequest};
```

### 2. Implement server/protocol.rs

```rust
use serde::{Deserialize, Serialize};

/// Messages sent from client to server
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum ClientMessage {
    /// Set the video source for this session
    SetVideo { path: String },

    /// Request frames by byte offset
    RequestFrames {
        /// List of frames to extract (each with its own irap_offset)
        frames: Vec<FrameRequest>,
    },
}

/// Individual frame request within a RequestFrames message
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct FrameRequest {
    /// Byte offset of the frame in the video file
    pub offset: u64,
    /// Byte offset of the IRAP (keyframe) needed to decode this frame
    pub irap_offset: u64,
    /// Frame index (client-assigned, echoed back in response)
    pub index: u32,
}

/// Messages sent from server to client
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(tag = "type")]
pub enum ServerMessage {
    /// Acknowledgment of SetVideo
    VideoSet { path: String, ok: bool },

    /// Frame metadata (binary JPEG follows immediately)
    Frame {
        /// Frame index (from request)
        index: u32,
        /// Byte offset in source video
        offset: u64,
        /// Size of JPEG data in bytes
        size: u32,
    },

    /// Frame decode/encode failed
    FrameError {
        /// Frame index (from request)
        index: u32,
        /// Byte offset that failed
        offset: u64,
        /// Error description
        error: String,
    },

    /// General error (malformed request, video not found, etc.)
    Error { message: String },
}

impl ClientMessage {
    /// Parse from JSON string
    pub fn from_json(json: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(json)
    }
}

impl ServerMessage {
    /// Serialize to JSON string
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("ServerMessage serialization should not fail")
    }
}
```

### 3. Add unit tests

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_set_video_serialization() {
        let msg = ClientMessage::SetVideo {
            path: "videos/test.mp4".to_string(),
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"SetVideo""#));
        assert!(json.contains(r#""path":"videos/test.mp4""#));

        let parsed: ClientMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, msg);
    }

    #[test]
    fn test_request_frames_serialization() {
        let msg = ClientMessage::RequestFrames {
            frames: vec![
                FrameRequest { offset: 1500, irap_offset: 1000, index: 0 },
                FrameRequest { offset: 2100, irap_offset: 1000, index: 1 },
            ],
        };
        let json = serde_json::to_string(&msg).unwrap();
        assert!(json.contains(r#""type":"RequestFrames""#));
        assert!(json.contains(r#""irap_offset":1000"#));

        let parsed: ClientMessage = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, msg);
    }

    #[test]
    fn test_video_set_response() {
        let msg = ServerMessage::VideoSet {
            path: "videos/test.mp4".to_string(),
            ok: true,
        };
        let json = msg.to_json();
        assert!(json.contains(r#""type":"VideoSet""#));
        assert!(json.contains(r#""ok":true"#));
    }

    #[test]
    fn test_frame_response() {
        let msg = ServerMessage::Frame {
            index: 0,
            offset: 1500,
            size: 45230,
        };
        let json = msg.to_json();
        assert!(json.contains(r#""type":"Frame""#));
        assert!(json.contains(r#""size":45230"#));
    }

    #[test]
    fn test_frame_error_response() {
        let msg = ServerMessage::FrameError {
            index: 5,
            offset: 2800,
            error: "decode_failed".to_string(),
        };
        let json = msg.to_json();
        assert!(json.contains(r#""type":"FrameError""#));
        assert!(json.contains(r#""error":"decode_failed""#));
    }

    #[test]
    fn test_parse_invalid_json() {
        let result = ClientMessage::from_json("not json");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_unknown_type() {
        let result = ClientMessage::from_json(r#"{"type":"Unknown"}"#);
        assert!(result.is_err());
    }
}
```

## Success Criteria

- [ ] `ClientMessage::SetVideo` serializes with `"type": "SetVideo"`
- [ ] `ClientMessage::RequestFrames` serializes with nested `frames` array
- [ ] All `ServerMessage` variants serialize correctly
- [ ] Round-trip JSON parsing works for all types
- [ ] Invalid JSON returns parse error (not panic)
- [ ] Unknown message types return parse error
- [ ] All tests pass: `cargo test -p bucket-streamer -- protocol`

## Context

### Serde Tagged Enum Pattern
Using `#[serde(tag = "type")]` creates "internally tagged" enums:
```json
{"type": "SetVideo", "path": "..."}
```
Instead of externally tagged:
```json
{"SetVideo": {"path": "..."}}
```
This is cleaner for WebSocket protocols.

### Binary Frame Delivery Pattern
Protocol flow for frame delivery:
1. Server sends JSON: `{"type": "Frame", "index": 0, "size": 45230, ...}`
2. Server immediately sends binary message with JPEG bytes
3. Client matches binary to preceding Frame message

This is more efficient than base64 encoding JPEG in JSON.

### Error Granularity
- `FrameError`: Specific frame failed, session continues
- `Error`: Session-level problem (video not found, malformed request)

This allows clients to gracefully handle partial failures.

### Per-Frame IRAP Offset
Each `FrameRequest` carries its own `irap_offset` rather than having a batch-level IRAP. This design:
- Allows requesting frames from different keyframe groups in one batch
- Makes each frame request self-contained
- Aligns with the flat offsets format (Task 12a) where each frame knows its IRAP

Keyframe detection: `offset == irap_offset` indicates the frame is itself a keyframe.
