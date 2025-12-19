//! Error types for Winpipe

use thiserror::Error;

#[derive(Error, Debug)]
pub enum WinpipeError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("Protocol error: {0}")]
    Protocol(String),

    #[error("Connection closed")]
    ConnectionClosed,

    #[error("Invalid message: {0}")]
    InvalidMessage(String),

    #[error("Compression error: {0}")]
    Compression(String),

    #[error("Buffer error: {0}")]
    Buffer(String),
}

pub type Result<T> = std::result::Result<T, WinpipeError>;
