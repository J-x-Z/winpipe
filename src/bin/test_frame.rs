//! Quick test to send a render frame to win-way

use std::io::Write;
use std::net::TcpStream;

fn main() {
    println!("🧪 Testing win-way connection...");
    
    // Connect to win-way
    let mut stream = match TcpStream::connect("127.0.0.1:9999") {
        Ok(s) => {
            println!("✅ Connected to win-way at 127.0.0.1:9999");
            s
        }
        Err(e) => {
            println!("❌ Failed to connect: {}", e);
            return;
        }
    };
    
    // Create a test frame (100x100 gradient)
    let width: u32 = 200;
    let height: u32 = 200;
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    
    for y in 0..height {
        for x in 0..width {
            // Create a colorful gradient pattern
            let r = ((x * 255) / width) as u8;
            let g = ((y * 255) / height) as u8;
            let b = 128u8;
            let a = 255u8;
            pixels.extend_from_slice(&[b, g, r, a]); // BGRA format
        }
    }
    
    // Build frame header
    let mut frame = Vec::new();
    frame.extend_from_slice(b"WPRD"); // Magic
    frame.extend_from_slice(&width.to_le_bytes());
    frame.extend_from_slice(&height.to_le_bytes());
    frame.extend_from_slice(&0u32.to_le_bytes()); // Format: ARGB8888
    frame.extend_from_slice(&(pixels.len() as u32).to_le_bytes());
    frame.extend_from_slice(&pixels);
    
    println!("📤 Sending test frame: {}x{} ({} bytes)", width, height, frame.len());
    
    match stream.write_all(&frame) {
        Ok(_) => println!("✅ Frame sent successfully!"),
        Err(e) => println!("❌ Failed to send: {}", e),
    }
    
    println!("🎉 Test complete! Check the win-way window for the gradient.");
}
