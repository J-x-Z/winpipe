//! Wayland Protocol Handler
//!
//! This module implements the core Wayland protocol responses that
//! allow clients to connect and discover available interfaces.
//!
//! This is the missing piece that makes winpipe act as a real compositor.

use std::collections::HashMap;
use log::{info, debug, warn};

use crate::wire::{Message, WireEncoder};
use crate::error::Result;

/// Object ID allocator
pub struct ObjectAllocator {
    next_id: u32,
}

impl ObjectAllocator {
    pub fn new() -> Self {
        Self { next_id: 2 } // 1 is reserved for wl_display
    }

    pub fn alloc(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }
}

impl Default for ObjectAllocator {
    fn default() -> Self {
        Self::new()
    }
}

/// Global interface definition
#[derive(Debug, Clone)]
pub struct Global {
    pub name: u32,
    pub interface: String,
    pub version: u32,
}

/// Wayland compositor state
pub struct Compositor {
    /// Registered globals
    globals: Vec<Global>,
    /// Object ID to interface mapping
    objects: HashMap<u32, String>,
    /// ID allocator
    allocator: ObjectAllocator,
    /// Encoder for responses
    encoder: WireEncoder,
    /// Next global name
    next_global_name: u32,
}

impl Compositor {
    pub fn new() -> Self {
        let mut comp = Self {
            globals: Vec::new(),
            objects: HashMap::new(),
            allocator: ObjectAllocator::new(),
            encoder: WireEncoder::new(),
            next_global_name: 1,
        };

        // Register wl_display (object 1)
        comp.objects.insert(1, "wl_display".to_string());

        // Register standard globals
        comp.register_global("wl_compositor", 5);
        comp.register_global("wl_subcompositor", 1);
        comp.register_global("wl_shm", 1);
        comp.register_global("wl_output", 4);
        comp.register_global("wl_seat", 8);
        comp.register_global("wl_data_device_manager", 3);
        comp.register_global("xdg_wm_base", 5);
        comp.register_global("wp_viewporter", 1);
        comp.register_global("zwp_linux_dmabuf_v1", 4);

        comp
    }

    /// Register a global interface
    fn register_global(&mut self, interface: &str, version: u32) {
        let name = self.next_global_name;
        self.next_global_name += 1;
        
        self.globals.push(Global {
            name,
            interface: interface.to_string(),
            version,
        });
        
        debug!("Registered global: {} v{} (name={})", interface, version, name);
    }

    /// Handle an incoming message and return response messages
    pub fn handle_message(&mut self, msg: &Message) -> Vec<Message> {
        let interface = self.objects.get(&msg.object_id)
            .map(|s| s.as_str())
            .unwrap_or("unknown");

        debug!("Handle: {}@{}.opcode={}", interface, msg.object_id, msg.opcode);

        match (interface, msg.opcode) {
            // wl_display.sync (opcode 0) -> send wl_callback.done
            ("wl_display", 0) => {
                // Payload contains new callback ID
                if msg.payload.len() >= 4 {
                    let callback_id = u32::from_le_bytes([
                        msg.payload[0], msg.payload[1], 
                        msg.payload[2], msg.payload[3]
                    ]);
                    self.objects.insert(callback_id, "wl_callback".to_string());
                    
                    // Send wl_callback.done (opcode 0)
                    let serial = 1u32;
                    let response = Message::new(
                        callback_id, 
                        0, // done
                        serial.to_le_bytes().to_vec()
                    );
                    info!("wl_display.sync -> callback.done (id={})", callback_id);
                    return vec![response];
                }
            }

            // wl_display.get_registry (opcode 1) -> send globals
            ("wl_display", 1) => {
                if msg.payload.len() >= 4 {
                    let registry_id = u32::from_le_bytes([
                        msg.payload[0], msg.payload[1],
                        msg.payload[2], msg.payload[3]
                    ]);
                    self.objects.insert(registry_id, "wl_registry".to_string());
                    
                    info!("wl_display.get_registry (id={})", registry_id);
                    
                    // Send wl_registry.global for each registered global
                    let mut responses = Vec::new();
                    for global in &self.globals {
                        let mut payload = Vec::new();
                        
                        // name (u32)
                        payload.extend_from_slice(&global.name.to_le_bytes());
                        
                        // interface (string: length + data + padding)
                        let interface_bytes = global.interface.as_bytes();
                        let len = interface_bytes.len() as u32 + 1; // include null terminator
                        payload.extend_from_slice(&len.to_le_bytes());
                        payload.extend_from_slice(interface_bytes);
                        payload.push(0); // null terminator
                        // Pad to 4-byte boundary
                        while payload.len() % 4 != 0 {
                            payload.push(0);
                        }
                        
                        // version (u32)
                        payload.extend_from_slice(&global.version.to_le_bytes());
                        
                        responses.push(Message::new(registry_id, 0, payload)); // opcode 0 = global
                    }
                    
                    return responses;
                }
            }

            // wl_registry.bind (opcode 0) -> create the bound object
            ("wl_registry", 0) => {
                // Payload: name (u32), interface (string), version (u32), new_id (u32)
                if msg.payload.len() >= 4 {
                    let name = u32::from_le_bytes([
                        msg.payload[0], msg.payload[1],
                        msg.payload[2], msg.payload[3]
                    ]);
                    
                    // Find the global
                    if let Some(global) = self.globals.iter().find(|g| g.name == name) {
                        // The new_id is at the end of payload (need to parse string first)
                        // For simplicity, we'll extract from the end
                        let payload_len = msg.payload.len();
                        if payload_len >= 8 {
                            let new_id = u32::from_le_bytes([
                                msg.payload[payload_len - 4],
                                msg.payload[payload_len - 3],
                                msg.payload[payload_len - 2],
                                msg.payload[payload_len - 1],
                            ]);
                            
                            self.objects.insert(new_id, global.interface.clone());
                            info!("wl_registry.bind: {}@{}", global.interface, new_id);
                            
                            // Send wl_output events when output is bound
                            if global.interface == "wl_output" {
                                return self.send_output_info(new_id);
                            }
                        }
                    }
                }
            }

            // wl_compositor.create_surface (opcode 0)
            ("wl_compositor", 0) => {
                if msg.payload.len() >= 4 {
                    let surface_id = u32::from_le_bytes([
                        msg.payload[0], msg.payload[1],
                        msg.payload[2], msg.payload[3]
                    ]);
                    self.objects.insert(surface_id, "wl_surface".to_string());
                    info!("wl_compositor.create_surface (id={})", surface_id);
                }
            }

            // wl_shm.create_pool (opcode 0)
            ("wl_shm", 0) => {
                if msg.payload.len() >= 8 {
                    let pool_id = u32::from_le_bytes([
                        msg.payload[0], msg.payload[1],
                        msg.payload[2], msg.payload[3]
                    ]);
                    self.objects.insert(pool_id, "wl_shm_pool".to_string());
                    info!("wl_shm.create_pool (id={})", pool_id);
                    
                    // Send wl_shm.format events for supported formats
                    let formats = [0u32, 1]; // ARGB8888, XRGB8888
                    let mut responses = Vec::new();
                    for format in formats {
                        responses.push(Message::new(
                            msg.object_id,
                            0, // format event
                            format.to_le_bytes().to_vec()
                        ));
                    }
                    return responses;
                }
            }

            // xdg_wm_base.get_xdg_surface (opcode 2)
            ("xdg_wm_base", 2) => {
                if msg.payload.len() >= 8 {
                    let xdg_surface_id = u32::from_le_bytes([
                        msg.payload[0], msg.payload[1],
                        msg.payload[2], msg.payload[3]
                    ]);
                    self.objects.insert(xdg_surface_id, "xdg_surface".to_string());
                    info!("xdg_wm_base.get_xdg_surface (id={})", xdg_surface_id);
                }
            }

            // xdg_surface.get_toplevel (opcode 1)
            ("xdg_surface", 1) => {
                if msg.payload.len() >= 4 {
                    let toplevel_id = u32::from_le_bytes([
                        msg.payload[0], msg.payload[1],
                        msg.payload[2], msg.payload[3]
                    ]);
                    self.objects.insert(toplevel_id, "xdg_toplevel".to_string());
                    info!("xdg_surface.get_toplevel (id={})", toplevel_id);
                    
                    let mut responses = Vec::new();
                    
                    // 1. Send xdg_toplevel.configure (width=1920, height=1080, states=[])
                    let mut toplevel_conf = Vec::new();
                    toplevel_conf.extend_from_slice(&1920i32.to_le_bytes()); // width
                    toplevel_conf.extend_from_slice(&1080i32.to_le_bytes()); // height
                    toplevel_conf.extend_from_slice(&0u32.to_le_bytes());    // states array length
                    responses.push(Message::new(toplevel_id, 0, toplevel_conf));
                    
                    // 2. Send xdg_surface.configure (serial) - THIS IS CRITICAL
                    let serial = 1u32;
                    responses.push(Message::new(msg.object_id, 0, serial.to_le_bytes().to_vec()));
                    
                    info!("Sent xdg configure: 1920x1080, serial={}", serial);
                    return responses;
                }
            }

            // xdg_surface.ack_configure (opcode 4)
            ("xdg_surface", 4) => {
                debug!("xdg_surface.ack_configure");
            }

            // wl_surface.commit (opcode 6)
            ("wl_surface", 6) => {
                debug!("wl_surface.commit");
                // This is where we'd capture the surface content
            }

            _ => {
                debug!("Unhandled: {}@{}.{}", interface, msg.object_id, msg.opcode);
            }
        }

        Vec::new()
    }

    /// Encode responses to wire format
    pub fn encode_responses(&self, messages: &[Message]) -> Vec<u8> {
        self.encoder.encode_batch(messages)
    }

    /// Send wl_output information events
    fn send_output_info(&self, output_id: u32) -> Vec<Message> {
        let mut responses = Vec::new();

        // wl_output.geometry (opcode 0)
        // x, y, physical_width, physical_height, subpixel, make, model, transform
        let mut geometry = Vec::new();
        geometry.extend_from_slice(&0i32.to_le_bytes());    // x
        geometry.extend_from_slice(&0i32.to_le_bytes());    // y
        geometry.extend_from_slice(&1920i32.to_le_bytes()); // physical_width mm
        geometry.extend_from_slice(&1080i32.to_le_bytes()); // physical_height mm
        geometry.extend_from_slice(&0i32.to_le_bytes());    // subpixel: unknown
        // make string
        let make = b"Winpipe";
        geometry.extend_from_slice(&(make.len() as u32 + 1).to_le_bytes());
        geometry.extend_from_slice(make);
        geometry.push(0);
        while geometry.len() % 4 != 0 { geometry.push(0); }
        // model string
        let model = b"Virtual Display";
        geometry.extend_from_slice(&(model.len() as u32 + 1).to_le_bytes());
        geometry.extend_from_slice(model);
        geometry.push(0);
        while geometry.len() % 4 != 0 { geometry.push(0); }
        geometry.extend_from_slice(&0i32.to_le_bytes());    // transform: normal
        responses.push(Message::new(output_id, 0, geometry));

        // wl_output.mode (opcode 1)
        // flags, width, height, refresh
        let mut mode = Vec::new();
        mode.extend_from_slice(&3u32.to_le_bytes());       // flags: current | preferred
        mode.extend_from_slice(&1920i32.to_le_bytes());    // width
        mode.extend_from_slice(&1080i32.to_le_bytes());    // height
        mode.extend_from_slice(&60000i32.to_le_bytes());   // refresh (mHz)
        responses.push(Message::new(output_id, 1, mode));

        // wl_output.scale (opcode 3) - for version >= 2
        let scale = 1i32.to_le_bytes().to_vec();
        responses.push(Message::new(output_id, 3, scale));

        // wl_output.done (opcode 2) - for version >= 2
        responses.push(Message::new(output_id, 2, vec![]));

        info!("Sent wl_output info: 1920x1080@60Hz");
        responses
    }
}

impl Default for Compositor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compositor_init() {
        let comp = Compositor::new();
        assert!(!comp.globals.is_empty());
    }

    #[test]
    fn test_handle_get_registry() {
        let mut comp = Compositor::new();
        
        // wl_display.get_registry with new_id = 2
        let msg = Message::new(1, 1, 2u32.to_le_bytes().to_vec());
        let responses = comp.handle_message(&msg);
        
        // Should get global events for each registered interface
        assert!(!responses.is_empty());
    }
}
