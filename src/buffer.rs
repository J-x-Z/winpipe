//! Mirror Buffer Management
//!
//! Waypipe maintains "mirror" copies of shared memory buffers on both sides.
//! When a buffer is updated, only the changed regions (deltas) are transmitted.
//! This significantly reduces bandwidth for applications with relatively static UIs.

use std::collections::HashMap;

use crate::error::{Result, WinpipeError};

/// A mirrored shared memory buffer
#[derive(Debug)]
pub struct MirrorBuffer {
    /// Buffer ID (from Wayland object ID)
    pub id: u32,
    /// Buffer width in pixels
    pub width: u32,
    /// Buffer height in pixels
    pub height: u32,
    /// Bytes per pixel (typically 4 for ARGB8888)
    pub bpp: u32,
    /// Stride (bytes per row)
    pub stride: u32,
    /// Buffer data
    pub data: Vec<u8>,
    /// Previous frame data (for delta calculation)
    pub prev_data: Option<Vec<u8>>,
    /// Dirty regions that need to be synced
    dirty_regions: Vec<DirtyRegion>,
}

/// A dirty (changed) region of a buffer
#[derive(Debug, Clone)]
pub struct DirtyRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Delta encoding result
#[derive(Debug)]
pub struct BufferDelta {
    pub buffer_id: u32,
    /// Changed regions with their data
    pub regions: Vec<DeltaRegion>,
    /// Total bytes in delta
    pub total_bytes: usize,
}

/// A single delta region with data
#[derive(Debug)]
pub struct DeltaRegion {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

impl MirrorBuffer {
    /// Create a new mirror buffer
    pub fn new(id: u32, width: u32, height: u32, bpp: u32, stride: u32) -> Self {
        let size = (stride * height) as usize;
        Self {
            id,
            width,
            height,
            bpp,
            stride,
            data: vec![0u8; size],
            prev_data: None,
            dirty_regions: Vec::new(),
        }
    }

    /// Create from existing data
    pub fn from_data(id: u32, width: u32, height: u32, bpp: u32, stride: u32, data: Vec<u8>) -> Self {
        Self {
            id,
            width,
            height,
            bpp,
            stride,
            data,
            prev_data: None,
            dirty_regions: Vec::new(),
        }
    }

    /// Total buffer size in bytes
    pub fn size(&self) -> usize {
        self.data.len()
    }

    /// Update buffer data
    pub fn update(&mut self, data: &[u8]) {
        // Save previous for delta calculation
        self.prev_data = Some(self.data.clone());
        
        // Copy new data
        let copy_len = data.len().min(self.data.len());
        self.data[..copy_len].copy_from_slice(&data[..copy_len]);
    }

    /// Update a region of the buffer
    pub fn update_region(&mut self, x: u32, y: u32, width: u32, height: u32, data: &[u8]) {
        let src_stride = width * self.bpp;
        
        for row in 0..height {
            let dst_y = y + row;
            if dst_y >= self.height {
                break;
            }
            
            let src_offset = (row * src_stride) as usize;
            let dst_offset = (dst_y * self.stride + x * self.bpp) as usize;
            
            let copy_len = (width * self.bpp) as usize;
            let copy_len = copy_len.min(self.data.len() - dst_offset);
            let copy_len = copy_len.min(data.len() - src_offset);
            
            if copy_len > 0 {
                self.data[dst_offset..dst_offset + copy_len]
                    .copy_from_slice(&data[src_offset..src_offset + copy_len]);
            }
        }
        
        // Mark region as dirty
        self.dirty_regions.push(DirtyRegion { x, y, width, height });
    }

    /// Calculate delta from previous frame
    pub fn calculate_delta(&mut self) -> Option<BufferDelta> {
        let prev = self.prev_data.as_ref()?;
        
        if prev.len() != self.data.len() {
            return None;
        }

        // Simple approach: find changed rows
        // A more sophisticated approach would use block-based comparison
        let mut regions = Vec::new();
        let mut total_bytes = 0;
        
        let mut in_dirty_region = false;
        let mut region_start_y = 0u32;
        
        for y in 0..self.height {
            let row_start = (y * self.stride) as usize;
            let row_end = row_start + (self.width * self.bpp) as usize;
            
            // Ensure we don't go out of bounds
            let row_end = row_end.min(self.data.len()).min(prev.len());
            
            let row_changed = self.data[row_start..row_end] != prev[row_start..row_end];
            
            if row_changed && !in_dirty_region {
                // Start new dirty region
                in_dirty_region = true;
                region_start_y = y;
            } else if !row_changed && in_dirty_region {
                // End dirty region
                in_dirty_region = false;
                let region_height = y - region_start_y;
                
                // Extract region data
                let data = self.extract_region(0, region_start_y, self.width, region_height);
                total_bytes += data.len();
                
                regions.push(DeltaRegion {
                    x: 0,
                    y: region_start_y,
                    width: self.width,
                    height: region_height,
                    data,
                });
            }
        }
        
        // Handle region at end of buffer
        if in_dirty_region {
            let region_height = self.height - region_start_y;
            let data = self.extract_region(0, region_start_y, self.width, region_height);
            total_bytes += data.len();
            
            regions.push(DeltaRegion {
                x: 0,
                y: region_start_y,
                width: self.width,
                height: region_height,
                data,
            });
        }

        if regions.is_empty() {
            return None; // No changes
        }

        Some(BufferDelta {
            buffer_id: self.id,
            regions,
            total_bytes,
        })
    }

    /// Extract a region of the buffer
    fn extract_region(&self, x: u32, y: u32, width: u32, height: u32) -> Vec<u8> {
        let mut data = Vec::with_capacity((width * height * self.bpp) as usize);
        
        for row in 0..height {
            let src_y = y + row;
            if src_y >= self.height {
                break;
            }
            
            let src_offset = (src_y * self.stride + x * self.bpp) as usize;
            let copy_len = (width * self.bpp) as usize;
            
            if src_offset + copy_len <= self.data.len() {
                data.extend_from_slice(&self.data[src_offset..src_offset + copy_len]);
            }
        }
        
        data
    }

    /// Apply a delta update
    pub fn apply_delta(&mut self, delta: &BufferDelta) {
        for region in &delta.regions {
            self.update_region(region.x, region.y, region.width, region.height, &region.data);
        }
        self.dirty_regions.clear();
    }

    /// Clear dirty regions
    pub fn clear_dirty(&mut self) {
        self.dirty_regions.clear();
    }
}

/// Buffer manager for all mirrored buffers
pub struct BufferManager {
    buffers: HashMap<u32, MirrorBuffer>,
}

impl BufferManager {
    pub fn new() -> Self {
        Self {
            buffers: HashMap::new(),
        }
    }

    /// Register a new buffer
    pub fn create(&mut self, id: u32, width: u32, height: u32, bpp: u32, stride: u32) {
        let buffer = MirrorBuffer::new(id, width, height, bpp, stride);
        self.buffers.insert(id, buffer);
    }

    /// Get a buffer reference
    pub fn get(&self, id: u32) -> Option<&MirrorBuffer> {
        self.buffers.get(&id)
    }

    /// Get a mutable buffer reference
    pub fn get_mut(&mut self, id: u32) -> Option<&mut MirrorBuffer> {
        self.buffers.get_mut(&id)
    }

    /// Remove a buffer
    pub fn remove(&mut self, id: u32) -> Option<MirrorBuffer> {
        self.buffers.remove(&id)
    }

    /// Number of buffers
    pub fn count(&self) -> usize {
        self.buffers.len()
    }

    /// Total memory usage
    pub fn total_memory(&self) -> usize {
        self.buffers.values().map(|b| b.size()).sum()
    }
}

impl Default for BufferManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_mirror_buffer_update() {
        let mut buffer = MirrorBuffer::new(1, 100, 100, 4, 400);
        
        // Create test data
        let data = vec![0xFF; buffer.size()];
        buffer.update(&data);
        
        assert_eq!(buffer.data[0], 0xFF);
        assert!(buffer.prev_data.is_some());
    }

    #[test]
    fn test_delta_calculation() {
        let mut buffer = MirrorBuffer::new(1, 10, 10, 4, 40);
        
        // Initial state (all zeros from prev_data = None)
        let initial = vec![0u8; buffer.size()];
        buffer.update(&initial);
        
        // Modify some data
        let mut modified = initial.clone();
        modified[0..40].fill(0xFF); // First row
        buffer.update(&modified);
        
        let delta = buffer.calculate_delta();
        assert!(delta.is_some());
        
        let delta = delta.unwrap();
        assert!(!delta.regions.is_empty());
    }
}
