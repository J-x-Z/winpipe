//! LZ4 Compression for Waypipe Protocol
//!
//! Waypipe uses compression to reduce bandwidth when forwarding
//! Wayland messages over the network.

use lz4_flex::{compress_prepend_size, decompress_size_prepended};

use crate::error::{Result, WinpipeError};

/// Compression level (0 = none, higher = more compression)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CompressionLevel {
    None,
    Fast,    // LZ4 default
    High,    // LZ4 HC (not supported by lz4_flex, fallback to fast)
}

impl Default for CompressionLevel {
    fn default() -> Self {
        Self::Fast
    }
}

/// Compressor/Decompressor for winpipe messages
pub struct Compressor {
    level: CompressionLevel,
    stats: CompressionStats,
}

/// Compression statistics
#[derive(Debug, Default, Clone)]
pub struct CompressionStats {
    pub bytes_in: u64,
    pub bytes_out: u64,
    pub messages: u64,
}

impl CompressionStats {
    pub fn ratio(&self) -> f64 {
        if self.bytes_in == 0 {
            1.0
        } else {
            self.bytes_out as f64 / self.bytes_in as f64
        }
    }
}

impl Compressor {
    pub fn new(level: CompressionLevel) -> Self {
        Self {
            level,
            stats: CompressionStats::default(),
        }
    }

    /// Compress data
    pub fn compress(&mut self, data: &[u8]) -> Vec<u8> {
        self.stats.bytes_in += data.len() as u64;
        self.stats.messages += 1;

        let result = match self.level {
            CompressionLevel::None => data.to_vec(),
            CompressionLevel::Fast | CompressionLevel::High => {
                compress_prepend_size(data)
            }
        };

        self.stats.bytes_out += result.len() as u64;
        result
    }

    /// Decompress data
    pub fn decompress(&mut self, data: &[u8]) -> Result<Vec<u8>> {
        self.stats.bytes_in += data.len() as u64;
        self.stats.messages += 1;

        let result = match self.level {
            CompressionLevel::None => data.to_vec(),
            CompressionLevel::Fast | CompressionLevel::High => {
                decompress_size_prepended(data)
                    .map_err(|e| WinpipeError::Compression(e.to_string()))?
            }
        };

        self.stats.bytes_out += result.len() as u64;
        Ok(result)
    }

    /// Get compression statistics
    pub fn stats(&self) -> &CompressionStats {
        &self.stats
    }

    /// Reset statistics
    pub fn reset_stats(&mut self) {
        self.stats = CompressionStats::default();
    }
}

impl Default for Compressor {
    fn default() -> Self {
        Self::new(CompressionLevel::Fast)
    }
}

/// Frame wrapper for compressed messages
/// 
/// Format:
/// - 4 bytes: Compressed size (little-endian)
/// - 4 bytes: Uncompressed size (little-endian)  
/// - N bytes: Compressed data
#[derive(Debug)]
pub struct CompressedFrame {
    pub compressed_size: u32,
    pub uncompressed_size: u32,
    pub data: Vec<u8>,
}

impl CompressedFrame {
    /// Create a new compressed frame
    pub fn new(data: Vec<u8>, uncompressed_size: u32) -> Self {
        Self {
            compressed_size: data.len() as u32,
            uncompressed_size,
            data,
        }
    }

    /// Encode to wire format
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(8 + self.data.len());
        buf.extend_from_slice(&self.compressed_size.to_le_bytes());
        buf.extend_from_slice(&self.uncompressed_size.to_le_bytes());
        buf.extend_from_slice(&self.data);
        buf
    }

    /// Decode from wire format
    pub fn decode(data: &[u8]) -> Result<Self> {
        if data.len() < 8 {
            return Err(WinpipeError::InvalidMessage("Frame too short".to_string()));
        }

        let compressed_size = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
        let uncompressed_size = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);

        let expected_len = 8 + compressed_size as usize;
        if data.len() < expected_len {
            return Err(WinpipeError::InvalidMessage(
                format!("Incomplete frame: have {}, need {}", data.len(), expected_len)
            ));
        }

        Ok(Self {
            compressed_size,
            uncompressed_size,
            data: data[8..expected_len].to_vec(),
        })
    }

    /// Total wire size
    pub fn wire_size(&self) -> usize {
        8 + self.data.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compress_decompress() {
        let mut compressor = Compressor::new(CompressionLevel::Fast);
        
        let original = b"Hello, World! This is a test of LZ4 compression. \
                         Let's add some repetitive content: aaaaaaaaaaaaaaaa";
        
        let compressed = compressor.compress(original);
        
        let mut decompressor = Compressor::new(CompressionLevel::Fast);
        let decompressed = decompressor.decompress(&compressed).unwrap();
        
        assert_eq!(decompressed, original);
    }

    #[test]
    fn test_compressed_frame() {
        let data = vec![1, 2, 3, 4, 5];
        let frame = CompressedFrame::new(data.clone(), 100);
        
        let encoded = frame.encode();
        let decoded = CompressedFrame::decode(&encoded).unwrap();
        
        assert_eq!(decoded.compressed_size, 5);
        assert_eq!(decoded.uncompressed_size, 100);
        assert_eq!(decoded.data, data);
    }
}
