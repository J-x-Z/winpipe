//! Winpipe: Windows-native Waypipe Implementation
//!
//! A transparent proxy for Wayland protocol that enables
//! running Wayland applications from WSL on Windows.
//!
//! Usage:
//!   winpipe server [--port PORT]     # Run as Wayland compositor server

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;

use clap::{Parser, Subcommand};
use log::{info, error, debug, warn};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Mutex;

use winpipe::wire::{Message, WireDecoder, WireEncoder, HEADER_SIZE};
use winpipe::compositor::Compositor;

/// Winpipe: Windows-native Waypipe Implementation
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Enable debug logging
    #[arg(short, long)]
    debug: bool,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand, Debug)]
enum Commands {
    /// Run as Wayland compositor server (Windows side)
    Server {
        /// Port to listen on
        #[arg(short, long, default_value_t = 9999)]
        port: u16,
    },
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();

    // Initialize logging
    if args.debug {
        env_logger::Builder::from_env(
            env_logger::Env::default().default_filter_or("debug")
        ).init();
    } else {
        env_logger::Builder::from_env(
            env_logger::Env::default().default_filter_or("info")
        ).init();
    }

    println!();
    println!("  ╔═══════════════════════════════════════════════════╗");
    println!("  ║       🔌 Winpipe: Wayland Compositor Proxy        ║");
    println!("  ║       Windows-native Waypipe Implementation       ║");
    println!("  ╚═══════════════════════════════════════════════════╝");
    println!();

    match args.command {
        Commands::Server { port } => {
            run_server(port).await?;
        }
    }

    Ok(())
}

/// Run winpipe as a Wayland compositor server
async fn run_server(port: u16) -> anyhow::Result<()> {
    let addr: SocketAddr = format!("0.0.0.0:{}", port).parse()?;
    let listener = TcpListener::bind(addr).await?;

    info!("🚀 Winpipe Wayland compositor listening on port {}", port);
    info!("💡 Connect from WSL:");
    info!("   WIN_IP=$(ip route | grep default | cut -d' ' -f3)");
    info!("   rm -f /tmp/wayland-winpipe && socat UNIX-LISTEN:/tmp/wayland-winpipe,fork TCP:$WIN_IP:{} &", port);
    info!("   export WAYLAND_DISPLAY=/tmp/wayland-winpipe");
    info!("   your-wayland-app");

    info!("✅ Server ready, waiting for connections...");

    let mut client_id = 0u32;

    loop {
        match listener.accept().await {
            Ok((stream, addr)) => {
                client_id = client_id.wrapping_add(1);
                info!("🔗 Client {} connected from {}", client_id, addr);
                
                let id = client_id;
                tokio::spawn(async move {
                    if let Err(e) = handle_client(stream, id).await {
                        warn!("Client {} error: {}", id, e);
                    }
                    info!("🔌 Client {} disconnected", id);
                });
            }
            Err(e) => {
                error!("Accept error: {}", e);
            }
        }
    }
}

/// Handle a single Wayland client connection
async fn handle_client(mut stream: TcpStream, client_id: u32) -> anyhow::Result<()> {
    let mut compositor = Compositor::new();
    let mut decoder = WireDecoder::new();
    let encoder = WireEncoder::new();
    let mut buffer = vec![0u8; 65536];

    let mut msg_count = 0u64;

    loop {
        let n = stream.read(&mut buffer).await?;
        if n == 0 {
            return Ok(()); // Connection closed
        }

        debug!("[{}] Received {} bytes", client_id, n);

        // Decode messages
        decoder.push(&buffer[..n]);

        while let Some(msg) = decoder.decode() {
            msg_count += 1;
            debug!("[{}] Message #{}: obj={} op={} payload={} bytes",
                   client_id, msg_count, msg.object_id, msg.opcode, msg.payload.len());

            // Handle message and get responses
            let responses = compositor.handle_message(&msg);

            // Send responses back to client
            if !responses.is_empty() {
                let response_data = encoder.encode_batch(&responses);
                debug!("[{}] Sending {} responses ({} bytes)",
                       client_id, responses.len(), response_data.len());
                stream.write_all(&response_data).await?;
            }
        }
    }
}
