//! TCP Connection Manager
//!
//! Handles TCP connections between winpipe instances.
//! Supports both server mode (Windows side) and client mode (WSL side placeholder).

use std::net::SocketAddr;
use std::sync::Arc;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use log::{info, warn, error, debug};

use crate::error::{Result, WinpipeError};
use crate::wire::{Message, WireDecoder, WireEncoder};
use crate::compress::{Compressor, CompressionLevel};

/// Connection configuration
#[derive(Debug, Clone)]
pub struct ConnectionConfig {
    /// Listen address for server mode
    pub bind_addr: SocketAddr,
    /// Compression level
    pub compression: CompressionLevel,
    /// Buffer size for reads
    pub buffer_size: usize,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            bind_addr: "0.0.0.0:9999".parse().unwrap(),
            compression: CompressionLevel::Fast,
            buffer_size: 65536,
        }
    }
}

/// Messages from the connection to the application
#[derive(Debug)]
pub enum ConnectionEvent {
    /// New client connected
    Connected { id: u32 },
    /// Client disconnected
    Disconnected { id: u32 },
    /// Received Wayland message
    Message { id: u32, msg: Message },
    /// Raw data received (for passthrough mode)
    RawData { id: u32, data: Vec<u8> },
}

/// Handle to communicate with a connection task
pub struct ConnectionHandle {
    /// Receive events from connection
    pub events: mpsc::Receiver<ConnectionEvent>,
    /// Send data to connection
    pub sender: mpsc::Sender<Vec<u8>>,
}

/// TCP Server for accepting waypipe client connections
pub struct Server {
    listener: TcpListener,
    config: ConnectionConfig,
    next_client_id: u32,
}

impl Server {
    /// Create a new server
    pub async fn bind(config: ConnectionConfig) -> Result<Self> {
        let listener = TcpListener::bind(config.bind_addr).await?;
        info!("📡 Winpipe server listening on {}", config.bind_addr);
        
        Ok(Self {
            listener,
            config,
            next_client_id: 1,
        })
    }

    /// Accept a single client connection
    pub async fn accept(&mut self) -> Result<(Connection, u32)> {
        let (stream, addr) = self.listener.accept().await?;
        let client_id = self.next_client_id;
        self.next_client_id = self.next_client_id.wrapping_add(1);
        
        info!("🔗 Client {} connected from {}", client_id, addr);
        
        let conn = Connection::new(stream, self.config.clone(), client_id);
        Ok((conn, client_id))
    }

    /// Run the accept loop, forwarding events through channels
    pub async fn run(mut self, event_tx: mpsc::Sender<ConnectionEvent>) -> Result<()> {
        loop {
            match self.accept().await {
                Ok((conn, id)) => {
                    let tx = event_tx.clone();
                    
                    // Notify of connection
                    let _ = tx.send(ConnectionEvent::Connected { id }).await;
                    
                    // Spawn handler task
                    tokio::spawn(async move {
                        if let Err(e) = conn.run(tx.clone()).await {
                            warn!("Client {} error: {}", id, e);
                        }
                        let _ = tx.send(ConnectionEvent::Disconnected { id }).await;
                    });
                }
                Err(e) => {
                    error!("Accept error: {}", e);
                }
            }
        }
    }
}

/// A single client connection
pub struct Connection {
    stream: TcpStream,
    config: ConnectionConfig,
    client_id: u32,
    decoder: WireDecoder,
    encoder: WireEncoder,
    compressor: Compressor,
}

impl Connection {
    /// Create new connection from stream
    pub fn new(stream: TcpStream, config: ConnectionConfig, client_id: u32) -> Self {
        Self {
            stream,
            compressor: Compressor::new(config.compression),
            config,
            client_id,
            decoder: WireDecoder::new(),
            encoder: WireEncoder::new(),
        }
    }

    /// Run the connection, forwarding messages to channel
    pub async fn run(mut self, tx: mpsc::Sender<ConnectionEvent>) -> Result<()> {
        let mut buffer = vec![0u8; self.config.buffer_size];
        
        loop {
            let n = self.stream.read(&mut buffer).await?;
            if n == 0 {
                // Connection closed
                return Ok(());
            }
            
            debug!("📥 Received {} bytes from client {}", n, self.client_id);
            
            // Try to decompress if using compression
            let data = if self.config.compression != CompressionLevel::None {
                match self.compressor.decompress(&buffer[..n]) {
                    Ok(d) => d,
                    Err(_) => {
                        // Fallback: treat as raw data
                        buffer[..n].to_vec()
                    }
                }
            } else {
                buffer[..n].to_vec()
            };
            
            // Feed to wire decoder
            self.decoder.push(&data);
            
            // Extract all complete messages
            while let Some(msg) = self.decoder.decode() {
                debug!("📨 Decoded message: obj={}, opcode={}, payload={} bytes",
                       msg.object_id, msg.opcode, msg.payload.len());
                
                if tx.send(ConnectionEvent::Message { 
                    id: self.client_id, 
                    msg 
                }).await.is_err() {
                    return Ok(()); // Receiver dropped
                }
            }
            
            // Also send raw data event for passthrough handling
            if !data.is_empty() {
                let _ = tx.send(ConnectionEvent::RawData {
                    id: self.client_id,
                    data: data.clone(),
                }).await;
            }
        }
    }

    /// Send a message to the client
    pub async fn send_message(&mut self, msg: &Message) -> Result<()> {
        let data = self.encoder.encode(msg);
        self.send_raw(&data).await
    }

    /// Send raw data to the client
    pub async fn send_raw(&mut self, data: &[u8]) -> Result<()> {
        let to_send = if self.config.compression != CompressionLevel::None {
            self.compressor.compress(data)
        } else {
            data.to_vec()
        };
        
        self.stream.write_all(&to_send).await?;
        Ok(())
    }
}

/// Utility function to forward between two connections (bidirectional proxy)
pub async fn forward(
    mut client: TcpStream,
    mut server: TcpStream,
) -> Result<()> {
    let (mut cr, mut cw) = client.split();
    let (mut sr, mut sw) = server.split();
    
    let client_to_server = async {
        tokio::io::copy(&mut cr, &mut sw).await
    };
    
    let server_to_client = async {
        tokio::io::copy(&mut sr, &mut cw).await
    };
    
    tokio::select! {
        result = client_to_server => {
            result.map_err(WinpipeError::from)?;
        }
        result = server_to_client => {
            result.map_err(WinpipeError::from)?;
        }
    }
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_server_creation() {
        let config = ConnectionConfig {
            bind_addr: "127.0.0.1:0".parse().unwrap(), // Random port
            ..Default::default()
        };
        let server = Server::bind(config).await;
        assert!(server.is_ok());
    }
}
