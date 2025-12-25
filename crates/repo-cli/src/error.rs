use thiserror::Error;

#[derive(Error, Debug)]
pub enum CliError {
    #[error("File not found: {0}")]
    FileNotFound(String),

    #[error("Invalid file extension: {0} (expected .mp4, .mov, or .h265)")]
    InvalidExtension(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Output file already exists: {0} (use --force to overwrite)")]
    OutputExists(String),

    #[error("FFmpeg error: {0}")]
    FfmpegError(String),

    #[error("No video stream found in input file")]
    NoVideoStream,

    #[error("H.265/HEVC encoder not available (is libx265 installed?)")]
    EncoderNotFound,

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[allow(dead_code)]
    #[error("Internal error: {0}")]
    Internal(String),
}
