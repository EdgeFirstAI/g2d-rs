// SPDX-FileCopyrightText: Copyright 2025 Au-Zone Technologies
// SPDX-License-Identifier: Apache-2.0

//! On-target integration tests for G2D hardware acceleration.
//!
//! These tests require:
//! - NXP i.MX hardware with G2D support
//! - libg2d.so.2 installed
//! - /dev/dma_heap available (CMA or system heap)
//! - /dev/galcore accessible
//!
//! Run with: cargo test --test hardware_tests

#![cfg(target_os = "linux")]

use dma_heap::{Heap, HeapKind};
use g2d_sys::{
    g2d_format_G2D_RGB888, g2d_format_G2D_RGBA8888, g2d_format_G2D_YUYV,
    g2d_rotation_G2D_ROTATION_0, G2DFormat, G2DPhysical, G2DSurface, G2D, NV12, RGB, RGBA, YUYV,
};
use std::os::fd::{AsRawFd, OwnedFd};
use std::ptr;

/// Helper to allocate a DMA buffer and get its physical address
struct DmaBuffer {
    #[allow(dead_code)] // Kept for RAII - fd must outlive the mmap
    fd: OwnedFd,
    ptr: *mut u8,
    phys: G2DPhysical,
    size: usize,
}

impl DmaBuffer {
    fn new(size: usize) -> Result<Self, Box<dyn std::error::Error>> {
        let heap = Heap::new(HeapKind::Cma)
            .or_else(|_| Heap::new(HeapKind::System))
            .map_err(|e| format!("Failed to open DMA heap: {e}"))?;

        let fd = heap
            .allocate(size)
            .map_err(|e| format!("Failed to allocate DMA buffer: {e}"))?;

        // mmap the buffer
        let ptr = unsafe {
            libc::mmap(
                ptr::null_mut(),
                size,
                libc::PROT_READ | libc::PROT_WRITE,
                libc::MAP_SHARED,
                fd.as_raw_fd(),
                0,
            )
        };

        if ptr == libc::MAP_FAILED {
            return Err(format!("mmap failed: {}", std::io::Error::last_os_error()).into());
        }

        let phys = G2DPhysical::new(fd.as_raw_fd())?;

        Ok(Self {
            fd,
            ptr: ptr as *mut u8,
            phys,
            size,
        })
    }

    fn address(&self) -> u64 {
        self.phys.address()
    }

    fn as_mut_slice(&mut self) -> &mut [u8] {
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.size) }
    }

    fn as_slice(&self) -> &[u8] {
        unsafe { std::slice::from_raw_parts(self.ptr, self.size) }
    }
}

impl Drop for DmaBuffer {
    fn drop(&mut self) {
        unsafe {
            libc::munmap(self.ptr as *mut libc::c_void, self.size);
        }
    }
}

/// Create a G2DSurface for a buffer with given dimensions and format
fn create_surface(buf: &DmaBuffer, width: usize, height: usize, format: u32) -> G2DSurface {
    G2DSurface {
        format,
        planes: [buf.address(), 0, 0],
        left: 0,
        top: 0,
        right: width as i32,
        bottom: height as i32,
        stride: width as i32,
        width: width as i32,
        height: height as i32,
        blendfunc: 0,
        global_alpha: 255,
        clrcolor: 0,
        rot: g2d_rotation_G2D_ROTATION_0,
    }
}

/// Create a G2DSurface for NV12 (two-plane format)
fn create_nv12_surface(buf: &DmaBuffer, width: usize, height: usize) -> G2DSurface {
    let uv_offset = (width * height) as u64;
    G2DSurface {
        format: g2d_sys::g2d_format_G2D_NV12,
        planes: [buf.address(), buf.address() + uv_offset, 0],
        left: 0,
        top: 0,
        right: width as i32,
        bottom: height as i32,
        stride: width as i32,
        width: width as i32,
        height: height as i32,
        blendfunc: 0,
        global_alpha: 255,
        clrcolor: 0,
        rot: g2d_rotation_G2D_ROTATION_0,
    }
}

// =============================================================================
// Basic API Tests
// =============================================================================

#[test]
fn test_g2d_open_close() {
    let _ = env_logger::try_init();

    let g2d = G2D::new("libg2d.so.2");
    assert!(g2d.is_ok(), "Failed to open G2D: {:?}", g2d.err());

    let g2d = g2d.unwrap();
    println!("G2D version: {}", g2d.version());

    // G2D is closed on drop
}

#[test]
fn test_g2d_version_detection() {
    let _ = env_logger::try_init();

    let g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");
    let version = g2d.version();

    // Version should be reasonable (major >= 5, minor >= 0)
    assert!(
        version.major >= 5,
        "Unexpected major version: {}",
        version.major
    );
    assert!(
        version.minor >= 0,
        "Unexpected minor version: {}",
        version.minor
    );

    println!("Detected G2D version: {version}");
}

#[test]
fn test_g2d_colorspace_configuration() {
    let _ = env_logger::try_init();

    let mut g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");

    // Test BT.709 colorspace
    let result = g2d.set_bt709_colorspace();
    assert!(result.is_ok(), "Failed to set BT.709: {:?}", result.err());

    // Test BT.601 colorspace
    let result = g2d.set_bt601_colorspace();
    assert!(result.is_ok(), "Failed to set BT.601: {:?}", result.err());
}

// =============================================================================
// Format Conversion Tests
// =============================================================================

#[test]
fn test_g2d_format_conversion() {
    // Test FourCharCode to G2DFormat conversion
    let rgba = G2DFormat::try_from(RGBA);
    assert!(rgba.is_ok(), "RGBA format conversion failed");
    assert_eq!(rgba.unwrap().format(), g2d_format_G2D_RGBA8888);

    let rgb = G2DFormat::try_from(RGB);
    assert!(rgb.is_ok(), "RGB format conversion failed");
    assert_eq!(rgb.unwrap().format(), g2d_format_G2D_RGB888);

    let yuyv = G2DFormat::try_from(YUYV);
    assert!(yuyv.is_ok(), "YUYV format conversion failed");
    assert_eq!(yuyv.unwrap().format(), g2d_format_G2D_YUYV);

    let nv12 = G2DFormat::try_from(NV12);
    assert!(nv12.is_ok(), "NV12 format conversion failed");
}

// =============================================================================
// Clear Operation Tests
// =============================================================================

#[test]
fn test_g2d_clear_rgba() {
    let _ = env_logger::try_init();

    let width = 64;
    let height = 64;
    let size = width * height * 4; // RGBA = 4 bytes per pixel

    let mut buf = DmaBuffer::new(size).expect("Failed to allocate DMA buffer");

    // Initialize to zeros
    buf.as_mut_slice().fill(0);

    let g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");
    let mut surface = create_surface(&buf, width, height, g2d_format_G2D_RGBA8888);

    // Clear to red (RGBA: 255, 0, 0, 255)
    let color = [255u8, 0, 0, 255];
    let result = g2d.clear(&mut surface, color);
    assert!(result.is_ok(), "G2D clear failed: {:?}", result.err());

    // Verify the buffer was filled with red
    let data = buf.as_slice();
    for i in 0..10 {
        let offset = i * 4;
        assert_eq!(data[offset], 255, "Red channel mismatch at pixel {i}");
        assert_eq!(data[offset + 1], 0, "Green channel mismatch at pixel {i}");
        assert_eq!(data[offset + 2], 0, "Blue channel mismatch at pixel {i}");
        assert_eq!(data[offset + 3], 255, "Alpha channel mismatch at pixel {i}");
    }

    println!("G2D clear RGBA test passed");
}

#[test]
fn test_g2d_clear_multiple_colors() {
    let _ = env_logger::try_init();

    let width = 32;
    let height = 32;
    let size = width * height * 4;

    let buf = DmaBuffer::new(size).expect("Failed to allocate DMA buffer");
    let g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");
    let mut surface = create_surface(&buf, width, height, g2d_format_G2D_RGBA8888);

    // Test multiple colors
    let colors = [
        [255, 0, 0, 255],     // Red
        [0, 255, 0, 255],     // Green
        [0, 0, 255, 255],     // Blue
        [128, 128, 128, 255], // Gray
        [0, 0, 0, 255],       // Black
        [255, 255, 255, 255], // White
    ];

    for color in colors {
        let result = g2d.clear(&mut surface, color);
        assert!(
            result.is_ok(),
            "Clear with color {color:?} failed: {:?}",
            result.err()
        );

        // Spot check first pixel
        let data = buf.as_slice();
        assert_eq!(data[0], color[0], "Color mismatch for {color:?}");
        assert_eq!(data[1], color[1], "Color mismatch for {color:?}");
        assert_eq!(data[2], color[2], "Color mismatch for {color:?}");
        assert_eq!(data[3], color[3], "Color mismatch for {color:?}");
    }

    println!("G2D clear multiple colors test passed");
}

// =============================================================================
// Blit Operation Tests
// =============================================================================

#[test]
fn test_g2d_blit_rgba_to_rgba() {
    let _ = env_logger::try_init();

    let width = 64;
    let height = 64;
    let size = width * height * 4;

    let mut src_buf = DmaBuffer::new(size).expect("Failed to allocate src buffer");
    let mut dst_buf = DmaBuffer::new(size).expect("Failed to allocate dst buffer");

    // Fill source with a pattern
    for (i, byte) in src_buf.as_mut_slice().iter_mut().enumerate() {
        *byte = (i % 256) as u8;
    }

    // Clear destination
    dst_buf.as_mut_slice().fill(0);

    let mut g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");
    g2d.set_bt709_colorspace()
        .expect("Failed to set colorspace");

    let src_surface = create_surface(&src_buf, width, height, g2d_format_G2D_RGBA8888);
    let dst_surface = create_surface(&dst_buf, width, height, g2d_format_G2D_RGBA8888);

    let result = g2d.blit(&src_surface, &dst_surface);
    assert!(result.is_ok(), "G2D blit failed: {:?}", result.err());

    // Verify destination matches source
    let src_data = src_buf.as_slice();
    let dst_data = dst_buf.as_slice();

    for i in 0..100 {
        assert_eq!(
            src_data[i], dst_data[i],
            "Data mismatch at byte {i}: src={} dst={}",
            src_data[i], dst_data[i]
        );
    }

    println!("G2D blit RGBA to RGBA test passed");
}

#[test]
fn test_g2d_blit_with_scaling() {
    let _ = env_logger::try_init();

    let src_width = 128;
    let src_height = 128;
    let dst_width = 64;
    let dst_height = 64;

    let src_size = src_width * src_height * 4;
    let dst_size = dst_width * dst_height * 4;

    let mut src_buf = DmaBuffer::new(src_size).expect("Failed to allocate src buffer");
    let mut dst_buf = DmaBuffer::new(dst_size).expect("Failed to allocate dst buffer");

    // Fill source with a gradient pattern
    for y in 0..src_height {
        for x in 0..src_width {
            let offset = (y * src_width + x) * 4;
            let slice = src_buf.as_mut_slice();
            slice[offset] = (x * 2) as u8; // R
            slice[offset + 1] = (y * 2) as u8; // G
            slice[offset + 2] = 128; // B
            slice[offset + 3] = 255; // A
        }
    }

    dst_buf.as_mut_slice().fill(0);

    let mut g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");
    g2d.set_bt709_colorspace()
        .expect("Failed to set colorspace");

    let src_surface = create_surface(&src_buf, src_width, src_height, g2d_format_G2D_RGBA8888);
    let dst_surface = create_surface(&dst_buf, dst_width, dst_height, g2d_format_G2D_RGBA8888);

    let result = g2d.blit(&src_surface, &dst_surface);
    assert!(
        result.is_ok(),
        "G2D blit with scaling failed: {:?}",
        result.err()
    );

    // Verify destination is not all zeros (scaling happened)
    let dst_data = dst_buf.as_slice();
    let non_zero_count = dst_data.iter().filter(|&&b| b != 0).count();
    assert!(
        non_zero_count > dst_size / 2,
        "Destination buffer appears empty after scaling"
    );

    println!("G2D blit with scaling test passed");
}

#[test]
fn test_g2d_blit_rgba_to_rgb() {
    let _ = env_logger::try_init();

    let width = 64;
    let height = 64;
    let src_size = width * height * 4; // RGBA
    let dst_size = width * height * 3; // RGB

    let mut src_buf = DmaBuffer::new(src_size).expect("Failed to allocate src buffer");
    let mut dst_buf = DmaBuffer::new(dst_size).expect("Failed to allocate dst buffer");

    // Fill source with known values (red)
    for i in 0..(width * height) {
        let offset = i * 4;
        let slice = src_buf.as_mut_slice();
        slice[offset] = 255; // R
        slice[offset + 1] = 0; // G
        slice[offset + 2] = 0; // B
        slice[offset + 3] = 255; // A
    }

    dst_buf.as_mut_slice().fill(0);

    let mut g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");
    g2d.set_bt709_colorspace()
        .expect("Failed to set colorspace");

    let src_surface = create_surface(&src_buf, width, height, g2d_format_G2D_RGBA8888);
    let dst_surface = create_surface(&dst_buf, width, height, g2d_format_G2D_RGB888);

    let result = g2d.blit(&src_surface, &dst_surface);
    assert!(
        result.is_ok(),
        "G2D RGBA to RGB blit failed: {:?}",
        result.err()
    );

    // Verify destination has red values
    let dst_data = dst_buf.as_slice();
    for i in 0..10 {
        let offset = i * 3;
        assert_eq!(dst_data[offset], 255, "Red channel mismatch at pixel {i}");
        assert_eq!(
            dst_data[offset + 1],
            0,
            "Green channel mismatch at pixel {i}"
        );
        assert_eq!(
            dst_data[offset + 2],
            0,
            "Blue channel mismatch at pixel {i}"
        );
    }

    println!("G2D blit RGBA to RGB test passed");
}

// =============================================================================
// YUV Format Tests
// =============================================================================

#[test]
fn test_g2d_blit_yuyv_to_rgba() {
    let _ = env_logger::try_init();

    let width = 64;
    let height = 64;
    let src_size = width * height * 2; // YUYV = 2 bytes per pixel
    let dst_size = width * height * 4; // RGBA

    let mut src_buf = DmaBuffer::new(src_size).expect("Failed to allocate src buffer");
    let mut dst_buf = DmaBuffer::new(dst_size).expect("Failed to allocate dst buffer");

    // Fill with YUYV pattern (gray: Y=128, U=128, V=128)
    for i in 0..(src_size / 4) {
        let offset = i * 4;
        let slice = src_buf.as_mut_slice();
        slice[offset] = 128; // Y0
        slice[offset + 1] = 128; // U
        slice[offset + 2] = 128; // Y1
        slice[offset + 3] = 128; // V
    }

    dst_buf.as_mut_slice().fill(0);

    let mut g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");
    g2d.set_bt709_colorspace()
        .expect("Failed to set colorspace");

    let src_surface = create_surface(&src_buf, width, height, g2d_format_G2D_YUYV);
    let dst_surface = create_surface(&dst_buf, width, height, g2d_format_G2D_RGBA8888);

    let result = g2d.blit(&src_surface, &dst_surface);
    assert!(
        result.is_ok(),
        "G2D YUYV to RGBA blit failed: {:?}",
        result.err()
    );

    // Verify output is not empty (gray should convert to approximately gray in RGB)
    let dst_data = dst_buf.as_slice();
    let non_zero = dst_data.iter().filter(|&&b| b != 0).count();
    assert!(
        non_zero > dst_size / 4,
        "Destination appears empty after YUV conversion"
    );

    println!("G2D blit YUYV to RGBA test passed");
}

#[test]
fn test_g2d_blit_nv12_to_rgba() {
    let _ = env_logger::try_init();

    let width = 64;
    let height = 64;
    // NV12: Y plane (width*height) + UV plane (width*height/2)
    let src_size = width * height + width * height / 2;
    let dst_size = width * height * 4; // RGBA

    let mut src_buf = DmaBuffer::new(src_size).expect("Failed to allocate src buffer");
    let mut dst_buf = DmaBuffer::new(dst_size).expect("Failed to allocate dst buffer");

    // Fill Y plane with 128 (gray)
    let y_size = width * height;
    src_buf.as_mut_slice()[..y_size].fill(128);

    // Fill UV plane with 128 (neutral chroma)
    src_buf.as_mut_slice()[y_size..].fill(128);

    dst_buf.as_mut_slice().fill(0);

    let mut g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");
    g2d.set_bt709_colorspace()
        .expect("Failed to set colorspace");

    let src_surface = create_nv12_surface(&src_buf, width, height);
    let dst_surface = create_surface(&dst_buf, width, height, g2d_format_G2D_RGBA8888);

    let result = g2d.blit(&src_surface, &dst_surface);
    assert!(
        result.is_ok(),
        "G2D NV12 to RGBA blit failed: {:?}",
        result.err()
    );

    // Verify output is not empty
    let dst_data = dst_buf.as_slice();
    let non_zero = dst_data.iter().filter(|&&b| b != 0).count();
    assert!(
        non_zero > dst_size / 4,
        "Destination appears empty after NV12 conversion"
    );

    println!("G2D blit NV12 to RGBA test passed");
}

// =============================================================================
// Physical Address Tests
// =============================================================================

#[test]
fn test_g2d_physical_address() {
    let _ = env_logger::try_init();

    let size = 4096;
    let buf = DmaBuffer::new(size).expect("Failed to allocate DMA buffer");

    let phys_addr = buf.address();
    assert!(phys_addr != 0, "Physical address should not be zero");

    println!("Physical address: 0x{:x}", phys_addr);
}

// =============================================================================
// Error Handling Tests
// =============================================================================

#[test]
fn test_g2d_invalid_library_path() {
    let result = G2D::new("nonexistent_library.so");
    assert!(result.is_err(), "Opening non-existent library should fail");
}

#[test]
fn test_g2d_format_invalid() {
    use four_char_code::four_char_code;

    let invalid = four_char_code!("XXXX");
    let result = G2DFormat::try_from(invalid);
    assert!(result.is_err(), "Invalid format should return error");
}
