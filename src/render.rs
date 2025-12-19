//! Render Protocol for forwarding surface data to win-way
//!
//! Simple protocol for sending buffer data from winpipe to win-way:
//!
//! Frame format:
//! - Magic (4 bytes): "WPRD" (WinPipe RenDer)
//! - Width (4 bytes, LE)
//! - Height (4 bytes, LE)
//! - Format (4 bytes, LE): 0=ARGB8888, 1=XRGB8888
//! - Data size (4 bytes, LE)
//! - Data (N bytes): Raw pixel data

use std::net::SocketAddr;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use log::{info, debug, error};

use crate::error::{Result, WinpipeError};

/// Magic bytes for render frame
pub const FRAME_MAGIC: &[u8; 4] = b"WPRD";

/// Frame header size
pub const HEADER_SIZE: usize = 20;

/// Pixel format
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
pub enum PixelFormat {
    ARGB8888 = 0,
    XRGB8888 = 1,
}

/// A render frame to send to win-way
#[derive(Debug)]
pub struct RenderFrame {
    pub width: u32,
    pub height: u32,
    pub format: PixelFormat,
    pub data: Vec<u8>,
}

impl RenderFrame {
    /// Create a new render frame
    pub fn new(width: u32, height: u32, format: PixelFormat, data: Vec<u8>) -> Self {
        Self { width, height, format, data }
    }

    /// Encode to wire format
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(HEADER_SIZE + self.data.len());
        
        buf.extend_from_slice(FRAME_MAGIC);
        buf.extend_from_slice(&self.width.to_le_bytes());
        buf.extend_from_slice(&self.height.to_le_bytes());
        buf.extend_from_slice(&(self.format as u32).to_le_bytes());
        buf.extend_from_slice(&(self.data.len() as u32).to_le_bytes());
        buf.extend_from_slice(&self.data);
        
        buf
    }

    /// Decode from wire format
    pub fn decode(data: &[u8]) -> Result<Self> {
        if data.len() < HEADER_SIZE {
            return Err(WinpipeError::InvalidMessage("Frame too short".to_string()));
        }

        // Check magic
        if &data[0..4] != FRAME_MAGIC {
            return Err(WinpipeError::InvalidMessage("Invalid frame magic".to_string()));
        }

        let width = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
        let height = u32::from_le_bytes([data[8], data[9], data[10], data[11]]);
        let format_val = u32::from_le_bytes([data[12], data[13], data[14], data[15]]);
        let data_size = u32::from_le_bytes([data[16], data[17], data[18], data[19]]) as usize;

        let format = match format_val {
            0 => PixelFormat::ARGB8888,
            1 => PixelFormat::XRGB8888,
            _ => PixelFormat::ARGB8888,
        };

        if data.len() < HEADER_SIZE + data_size {
            return Err(WinpipeError::InvalidMessage("Incomplete frame data".to_string()));
        }

        Ok(Self {
            width,
            height,
            format,
            data: data[HEADER_SIZE..HEADER_SIZE + data_size].to_vec(),
        })
    }
}

/// Client for sending frames to win-way
pub struct RenderClient {
    stream: Option<TcpStream>,
    addr: SocketAddr,
}

impl RenderClient {
    /// Create a new render client
    pub fn new(addr: SocketAddr) -> Self {
        Self {
            stream: None,
            addr,
        }
    }

    /// Connect to win-way
    pub async fn connect(&mut self) -> Result<()> {
        info!("🎨 Connecting to win-way at {}", self.addr);
        let stream = TcpStream::connect(self.addr).await?;
        self.stream = Some(stream);
        info!("✅ Connected to win-way renderer");
        Ok(())
    }

    /// Send a frame to win-way
    pub async fn send_frame(&mut self, frame: &RenderFrame) -> Result<()> {
        let stream = self.stream.as_mut()
            .ok_or_else(|| WinpipeError::Protocol("Not connected".to_string()))?;
        
        let data = frame.encode();
        debug!("📤 Sending frame {}x{} ({} bytes)", frame.width, frame.height, data.len());
        
        stream.write_all(&data).await?;
        Ok(())
    }

    /// Check if connected
    pub fn is_connected(&self) -> bool {
        self.stream.is_some()
    }

    /// Disconnect
    pub fn disconnect(&mut self) {
        self.stream = None;
    }
}

/// Frame decoder for receiving frames (used by win-way)
pub struct FrameDecoder {
    buffer: Vec<u8>,
}

impl FrameDecoder {
    pub fn new() -> Self {
        Self {
            buffer: Vec::with_capacity(1024 * 1024), // 1MB initial
        }
    }

    /// Add data to buffer
    pub fn push(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
    }

    /// Try to decode next frame
    pub fn decode(&mut self) -> Option<RenderFrame> {
        if self.buffer.len() < HEADER_SIZE {
            return None;
        }

        // Check magic
        if &self.buffer[0..4] != FRAME_MAGIC {
            // Skip to find next magic
            if let Some(pos) = self.find_magic() {
                self.buffer.drain(..pos);
            } else {
                self.buffer.clear();
            }
            return None;
        }

        // Get data size
        let data_size = u32::from_le_bytes([
            self.buffer[16], self.buffer[17], self.buffer[18], self.buffer[19]
        ]) as usize;

        let total_size = HEADER_SIZE + data_size;
        if self.buffer.len() < total_size {
            return None; // Need more data
        }

        // Decode frame
        match RenderFrame::decode(&self.buffer[..total_size]) {
            Ok(frame) => {
                self.buffer.drain(..total_size);
                Some(frame)
            }
            Err(_) => {
                self.buffer.drain(..4); // Skip bad magic
                None
            }
        }
    }

    fn find_magic(&self) -> Option<usize> {
        self.buffer.windows(4)
            .position(|w| w == FRAME_MAGIC)
    }
}

impl Default for FrameDecoder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_frame_encode_decode() {
        let frame = RenderFrame::new(
            100, 100,
            PixelFormat::ARGB8888,
            vec![0xFF; 100 * 100 * 4],
        );

        let encoded = frame.encode();
        let decoded = RenderFrame::decode(&encoded).unwrap();

        assert_eq!(decoded.width, 100);
        assert_eq!(decoded.height, 100);
        assert_eq!(decoded.format, PixelFormat::ARGB8888);
        assert_eq!(decoded.data.len(), 100 * 100 * 4);
    }

    #[test]
    fn test_frame_decoder_streaming() {
        let mut decoder = FrameDecoder::new();
        
        let frame = RenderFrame::new(10, 10, PixelFormat::XRGB8888, vec![0u8; 400]);
        let data = frame.encode();

        // Push partial data
        decoder.push(&data[..10]);
        assert!(decoder.decode().is_none());

        // Push rest
        decoder.push(&data[10..]);
        let decoded = decoder.decode().unwrap();
        assert_eq!(decoded.width, 10);
    }
}
