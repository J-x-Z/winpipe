//! Wayland Wire Format Parser
//!
//! The Wayland wire protocol is a stream of 32-bit values (little-endian).
//! Each message consists of:
//! - Object ID (32-bit): The target object
//! - Size + Opcode (32-bit): High 16 bits = size in bytes, Low 16 bits = opcode
//! - Arguments (variable): Based on the message signature
//!
//! File descriptors are passed via ancillary data (which we handle specially
//! since Windows doesn't have Unix domain sockets).

use bytes::{Buf, BufMut, BytesMut};
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::io::Cursor;

use crate::error::{Result, WinpipeError};

/// Minimum message header size in bytes
pub const HEADER_SIZE: usize = 8;

/// Maximum message size (64KB - reasonable limit for Wayland)
pub const MAX_MESSAGE_SIZE: usize = 65536;

/// A parsed Wayland wire message
#[derive(Debug, Clone)]
pub struct Message {
    /// Target object ID
    pub object_id: u32,
    /// Message opcode
    pub opcode: u16,
    /// Raw payload data (without header)
    pub payload: Vec<u8>,
    /// Associated file descriptor count (for tracking FDs that need special handling)
    pub fd_count: u32,
}

impl Message {
    /// Create a new message
    pub fn new(object_id: u32, opcode: u16, payload: Vec<u8>) -> Self {
        Self {
            object_id,
            opcode,
            payload,
            fd_count: 0,
        }
    }

    /// Total message size in bytes (header + payload)
    pub fn wire_size(&self) -> usize {
        HEADER_SIZE + self.payload.len()
    }

    /// Serialize the message to wire format
    pub fn encode(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(self.wire_size());
        
        // Object ID
        buf.write_u32::<LittleEndian>(self.object_id).unwrap();
        
        // Size (high 16 bits) + Opcode (low 16 bits)
        let size_opcode = ((self.wire_size() as u32) << 16) | (self.opcode as u32);
        buf.write_u32::<LittleEndian>(size_opcode).unwrap();
        
        // Payload
        buf.extend_from_slice(&self.payload);
        
        buf
    }

    /// Parse a message from wire format
    pub fn decode(data: &[u8]) -> Result<Self> {
        if data.len() < HEADER_SIZE {
            return Err(WinpipeError::InvalidMessage(
                format!("Message too short: {} bytes", data.len())
            ));
        }

        let mut cursor = Cursor::new(data);
        
        // Read header
        let object_id = cursor.read_u32::<LittleEndian>()
            .map_err(|e| WinpipeError::InvalidMessage(e.to_string()))?;
        let size_opcode = cursor.read_u32::<LittleEndian>()
            .map_err(|e| WinpipeError::InvalidMessage(e.to_string()))?;
        
        let size = (size_opcode >> 16) as usize;
        let opcode = (size_opcode & 0xFFFF) as u16;
        
        // Validate size
        if size < HEADER_SIZE {
            return Err(WinpipeError::InvalidMessage(
                format!("Invalid message size: {}", size)
            ));
        }
        if size > MAX_MESSAGE_SIZE {
            return Err(WinpipeError::InvalidMessage(
                format!("Message too large: {} bytes", size)
            ));
        }
        if data.len() < size {
            return Err(WinpipeError::InvalidMessage(
                format!("Incomplete message: have {} bytes, need {}", data.len(), size)
            ));
        }
        
        // Extract payload
        let payload_size = size - HEADER_SIZE;
        let payload = data[HEADER_SIZE..size].to_vec();
        
        Ok(Self {
            object_id,
            opcode,
            payload,
            fd_count: 0,
        })
    }
}

/// Wire format decoder for streaming data
pub struct WireDecoder {
    buffer: BytesMut,
}

impl WireDecoder {
    pub fn new() -> Self {
        Self {
            buffer: BytesMut::with_capacity(MAX_MESSAGE_SIZE),
        }
    }

    /// Add data to the buffer
    pub fn push(&mut self, data: &[u8]) {
        self.buffer.extend_from_slice(data);
    }

    /// Try to decode the next complete message
    pub fn decode(&mut self) -> Option<Message> {
        if self.buffer.len() < HEADER_SIZE {
            return None;
        }

        // Peek at the size field (don't advance buffer yet)
        let size_opcode = u32::from_le_bytes([
            self.buffer[4],
            self.buffer[5],
            self.buffer[6],
            self.buffer[7],
        ]);
        let size = (size_opcode >> 16) as usize;

        // Validate and check if we have the complete message
        if size < HEADER_SIZE || size > MAX_MESSAGE_SIZE {
            // Protocol error - clear buffer to recover
            self.buffer.clear();
            return None;
        }
        if self.buffer.len() < size {
            // Need more data
            return None;
        }

        // Extract the complete message
        let msg_data = self.buffer.split_to(size);
        Message::decode(&msg_data).ok()
    }

    /// Number of bytes currently buffered
    pub fn buffered(&self) -> usize {
        self.buffer.len()
    }

    /// Clear the buffer
    pub fn clear(&mut self) {
        self.buffer.clear();
    }
}

impl Default for WireDecoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Wire format encoder
pub struct WireEncoder;

impl WireEncoder {
    pub fn new() -> Self {
        Self
    }

    /// Encode a single message
    pub fn encode(&self, msg: &Message) -> Vec<u8> {
        msg.encode()
    }

    /// Encode multiple messages into a single buffer
    pub fn encode_batch(&self, messages: &[Message]) -> Vec<u8> {
        let total_size: usize = messages.iter().map(|m| m.wire_size()).sum();
        let mut buf = Vec::with_capacity(total_size);
        for msg in messages {
            buf.extend(msg.encode());
        }
        buf
    }
}

impl Default for WireEncoder {
    fn default() -> Self {
        Self::new()
    }
}

/// Well-known Wayland protocol opcodes for core objects
pub mod opcodes {
    // wl_display (object 1)
    pub mod display {
        pub const ERROR: u16 = 0;
        pub const DELETE_ID: u16 = 1;
        pub const SYNC: u16 = 0; // Request
        pub const GET_REGISTRY: u16 = 1; // Request
    }

    // wl_registry
    pub mod registry {
        pub const GLOBAL: u16 = 0;        // Event
        pub const GLOBAL_REMOVE: u16 = 1; // Event
        pub const BIND: u16 = 0;          // Request
    }

    // wl_callback
    pub mod callback {
        pub const DONE: u16 = 0;
    }

    // wl_shm
    pub mod shm {
        pub const FORMAT: u16 = 0;      // Event
        pub const CREATE_POOL: u16 = 0; // Request
    }

    // wl_shm_pool
    pub mod shm_pool {
        pub const CREATE_BUFFER: u16 = 0;
        pub const DESTROY: u16 = 1;
        pub const RESIZE: u16 = 2;
    }

    // wl_buffer
    pub mod buffer {
        pub const RELEASE: u16 = 0;
        pub const DESTROY: u16 = 0; // Request
    }

    // wl_surface
    pub mod surface {
        pub const ENTER: u16 = 0;
        pub const LEAVE: u16 = 1;
        pub const DESTROY: u16 = 0;
        pub const ATTACH: u16 = 1;
        pub const DAMAGE: u16 = 2;
        pub const FRAME: u16 = 3;
        pub const SET_OPAQUE_REGION: u16 = 4;
        pub const SET_INPUT_REGION: u16 = 5;
        pub const COMMIT: u16 = 6;
        pub const SET_BUFFER_TRANSFORM: u16 = 7;
        pub const SET_BUFFER_SCALE: u16 = 8;
        pub const DAMAGE_BUFFER: u16 = 9;
    }

    // xdg_wm_base
    pub mod xdg_wm_base {
        pub const PING: u16 = 0;            // Event
        pub const DESTROY: u16 = 0;         // Request
        pub const CREATE_POSITIONER: u16 = 1;
        pub const GET_XDG_SURFACE: u16 = 2;
        pub const PONG: u16 = 3;
    }

    // xdg_surface
    pub mod xdg_surface {
        pub const CONFIGURE: u16 = 0; // Event
        pub const DESTROY: u16 = 0;
        pub const GET_TOPLEVEL: u16 = 1;
        pub const GET_POPUP: u16 = 2;
        pub const SET_WINDOW_GEOMETRY: u16 = 3;
        pub const ACK_CONFIGURE: u16 = 4;
    }

    // xdg_toplevel
    pub mod xdg_toplevel {
        pub const CONFIGURE: u16 = 0;       // Event
        pub const CLOSE: u16 = 1;           // Event
        pub const DESTROY: u16 = 0;
        pub const SET_PARENT: u16 = 1;
        pub const SET_TITLE: u16 = 2;
        pub const SET_APP_ID: u16 = 3;
        pub const SHOW_WINDOW_MENU: u16 = 4;
        pub const MOVE: u16 = 5;
        pub const RESIZE: u16 = 6;
        pub const SET_MAX_SIZE: u16 = 7;
        pub const SET_MIN_SIZE: u16 = 8;
        pub const SET_MAXIMIZED: u16 = 9;
        pub const UNSET_MAXIMIZED: u16 = 10;
        pub const SET_FULLSCREEN: u16 = 11;
        pub const UNSET_FULLSCREEN: u16 = 12;
        pub const SET_MINIMIZED: u16 = 13;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_message_encode_decode() {
        let msg = Message::new(1, 5, vec![0x12, 0x34, 0x56, 0x78]);
        let encoded = msg.encode();
        
        assert_eq!(encoded.len(), 12); // 8 header + 4 payload
        
        let decoded = Message::decode(&encoded).unwrap();
        assert_eq!(decoded.object_id, 1);
        assert_eq!(decoded.opcode, 5);
        assert_eq!(decoded.payload, vec![0x12, 0x34, 0x56, 0x78]);
    }

    #[test]
    fn test_wire_decoder_streaming() {
        let mut decoder = WireDecoder::new();
        
        // Create two messages
        let msg1 = Message::new(1, 1, vec![0xAA, 0xBB]);
        let msg2 = Message::new(2, 2, vec![0xCC, 0xDD, 0xEE, 0xFF]);
        
        let data = [msg1.encode(), msg2.encode()].concat();
        
        // Push data in chunks
        decoder.push(&data[..5]);
        assert!(decoder.decode().is_none()); // Not enough data
        
        decoder.push(&data[5..]);
        
        // Should decode both messages
        let d1 = decoder.decode().unwrap();
        assert_eq!(d1.object_id, 1);
        
        let d2 = decoder.decode().unwrap();
        assert_eq!(d2.object_id, 2);
        
        assert!(decoder.decode().is_none());
    }
}
