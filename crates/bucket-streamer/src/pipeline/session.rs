use std::collections::VecDeque;

use anyhow::Result;
use bytes::Bytes;

use super::decoder::{Decoder, DecodedFrame};
use super::encoder::JpegEncoder;
use crate::server::protocol::FrameRequest;

/// Per-session state for frame processing
pub struct Session {
    pub video_path: Option<String>,
    pub video_data: Option<Bytes>,
    pub decoder: Option<Decoder>,
    pub encoder: JpegEncoder,
    pub frame_queue: VecDeque<FrameRequest>,
}

impl Session {
    /// Create a new session with specified JPEG quality
    pub fn new(jpeg_quality: u8) -> Result<Self> {
        Ok(Self {
            video_path: None,
            video_data: None,
            decoder: None,
            encoder: JpegEncoder::new(jpeg_quality)?,
            frame_queue: VecDeque::new(),
        })
    }

    /// Set video source, initializing decoder
    pub fn set_video(&mut self, path: String, data: Bytes) -> Result<()> {
        let decoder = Decoder::new(&data)?;

        self.video_path = Some(path);
        self.video_data = Some(data);
        self.decoder = Some(decoder);
        self.frame_queue.clear();

        Ok(())
    }

    /// Queue frames for processing
    pub fn queue_frames(&mut self, frames: Vec<FrameRequest>) {
        self.frame_queue.extend(frames);
    }

    /// Process next frame in queue
    ///
    /// Returns ProcessResult containing the request and result (JPEG bytes or error)
    pub fn process_next(&mut self) -> Option<ProcessResult> {
        let request = self.frame_queue.pop_front()?;

        let result = self.process_frame(&request);
        Some(ProcessResult { request, result })
    }

    fn process_frame(&mut self, request: &FrameRequest) -> Result<Vec<u8>> {
        let decoder = self
            .decoder
            .as_mut()
            .ok_or_else(|| anyhow::anyhow!("No decoder initialized"))?;

        let video_data = self
            .video_data
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("No video data loaded"))?;

        // Decode frame at offset
        let frame = decoder.decode_frame(video_data, request.offset)?;

        // Encode to JPEG
        let jpeg = self.encoder.encode(&frame)?;

        Ok(jpeg)
    }

    pub fn has_pending_frames(&self) -> bool {
        !self.frame_queue.is_empty()
    }

    pub fn clear_queue(&mut self) {
        self.frame_queue.clear();
    }
}

pub struct ProcessResult {
    pub request: FrameRequest,
    pub result: Result<Vec<u8>>,
}
