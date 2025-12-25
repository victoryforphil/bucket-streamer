use thiserror::Error;

#[derive(Error, Debug)]
pub enum CliError {
    #[error("File not found: {0}")]
    FileNotFound(String),

    #[error("Invalid file extension: {0} (expected .mp4 or .mov)")]
    InvalidExtension(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[allow(dead_code)]
    #[error("Internal error: {0}")]
    Internal(String),
}
