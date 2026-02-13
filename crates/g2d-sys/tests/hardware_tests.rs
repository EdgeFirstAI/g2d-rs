// SPDX-FileCopyrightText: Copyright 2025 Au-Zone Technologies
// SPDX-License-Identifier: Apache-2.0

//! On-target integration tests for G2D hardware acceleration.
//!
//! These tests require:
//! - NXP i.MX hardware with G2D support
//! - libg2d.so.2 installed
//! - /dev/dma_heap available (uncached CMA preferred, cached CMA fallback)
//! - /dev/galcore accessible
//!
//! Tests are organized by heap type (uncached vs cached) to isolate
//! CPU cache coherency behavior and validate DMA_BUF_IOCTL_SYNC correctness.
//!
//! Run with: cargo test --test hardware_tests -- --test-threads=1 --nocapture

#![cfg(target_os = "linux")]

use dma_heap::{Heap, HeapKind};
use g2d_sys::{
    g2d_format, g2d_format_G2D_ABGR8888, g2d_format_G2D_ARGB8888, g2d_format_G2D_BGR565,
    g2d_format_G2D_BGR888, g2d_format_G2D_BGRA8888, g2d_format_G2D_BGRX8888, g2d_format_G2D_I420,
    g2d_format_G2D_NV12, g2d_format_G2D_NV16, g2d_format_G2D_NV21, g2d_format_G2D_NV61,
    g2d_format_G2D_RGB565, g2d_format_G2D_RGB888, g2d_format_G2D_RGBA8888, g2d_format_G2D_RGBX8888,
    g2d_format_G2D_UYVY, g2d_format_G2D_VYUY, g2d_format_G2D_XBGR8888, g2d_format_G2D_XRGB8888,
    g2d_format_G2D_YUYV, g2d_format_G2D_YV12, g2d_format_G2D_YVYU, g2d_rotation_G2D_ROTATION_0,
    G2DFormat, G2DPhysical, G2DSurface, G2D, NV12, RGB, RGBA, YUYV,
};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::ptr;
use std::time::Instant;

// =============================================================================
// DMA-buf synchronization constants (linux/dma-buf.h)
// =============================================================================

const DMA_BUF_BASE: u8 = b'b';
const DMA_BUF_IOCTL_SYNC_NR: u8 = 0;

const DMA_BUF_SYNC_READ: u64 = 1 << 0;
const DMA_BUF_SYNC_WRITE: u64 = 1 << 1;
const DMA_BUF_SYNC_START: u64 = 0 << 2;
const DMA_BUF_SYNC_END: u64 = 1 << 2;

#[repr(C)]
struct DmaBufSync {
    flags: u64,
}

// _IOW('b', 0, struct dma_buf_sync) = direction=1, size=8, type='b', nr=0
const DMA_BUF_IOCTL_SYNC_CMD: libc::c_ulong = (1 << 30)
    | ((std::mem::size_of::<DmaBufSync>() as libc::c_ulong) << 16)
    | ((DMA_BUF_BASE as libc::c_ulong) << 8)
    | DMA_BUF_IOCTL_SYNC_NR as libc::c_ulong;

// =============================================================================
// DRM PRIME import — creates persistent dma_buf_attach for cache maintenance
// =============================================================================
//
// The CMA heap's begin_cpu_access iterates over buffer->attachments to perform
// cache maintenance via dma_sync_sgtable_for_cpu(). Without any active
// attachments, DMA_BUF_IOCTL_SYNC is a no-op.
//
// By importing the DMA-buf fd through the DRM/GPU driver (DRM_IOCTL_PRIME_FD_TO_HANDLE),
// the GPU driver creates a persistent dma_buf_attach(). This makes
// DMA_BUF_IOCTL_SYNC actually perform cache invalidation/flush.

const DRM_IOCTL_BASE: u8 = b'd';

// DRM_IOCTL_PRIME_FD_TO_HANDLE = _IOWR('d', 0x2e, struct drm_prime_handle)
#[repr(C)]
struct DrmPrimeHandle {
    handle: u32,
    flags: u32,
    fd: i32,
}

const DRM_IOCTL_PRIME_FD_TO_HANDLE: libc::c_ulong = (3 << 30) // _IOWR
    | ((std::mem::size_of::<DrmPrimeHandle>() as libc::c_ulong) << 16)
    | ((DRM_IOCTL_BASE as libc::c_ulong) << 8)
    | 0x2e;

// DRM_IOCTL_GEM_CLOSE = _IOW('d', 0x09, struct drm_gem_close)
#[repr(C)]
struct DrmGemClose {
    handle: u32,
    pad: u32,
}

const DRM_IOCTL_GEM_CLOSE: libc::c_ulong = (1 << 30) // _IOW
    | ((std::mem::size_of::<DrmGemClose>() as libc::c_ulong) << 16)
    | ((DRM_IOCTL_BASE as libc::c_ulong) << 8)
    | 0x09;

/// Holds a DRM GEM handle that keeps a persistent dma_buf_attach alive.
/// When dropped, closes the GEM handle (which detaches the DMA-buf).
struct DrmAttachment {
    drm_fd: OwnedFd,
    gem_handle: u32,
}

impl DrmAttachment {
    /// Import a DMA-buf fd through the GPU DRM driver to create a persistent
    /// dma_buf_attach. Returns None if /dev/dri/renderD128 is not available.
    fn new(dma_buf_fd: &OwnedFd) -> Option<Self> {
        let path = b"/dev/dri/renderD128\0";
        let raw_fd = unsafe {
            libc::open(
                path.as_ptr() as *const libc::c_char,
                libc::O_RDWR | libc::O_CLOEXEC,
            )
        };
        if raw_fd < 0 {
            eprintln!(
                "  DrmAttachment: /dev/dri/renderD128 not available: {}",
                std::io::Error::last_os_error()
            );
            return None;
        }
        let drm_fd = unsafe { OwnedFd::from_raw_fd(raw_fd) };

        let mut prime = DrmPrimeHandle {
            handle: 0,
            flags: 0,
            fd: dma_buf_fd.as_raw_fd(),
        };

        let ret =
            unsafe { libc::ioctl(drm_fd.as_raw_fd(), DRM_IOCTL_PRIME_FD_TO_HANDLE, &mut prime) };
        if ret == -1 {
            eprintln!(
                "  DrmAttachment: PRIME_FD_TO_HANDLE failed: {}",
                std::io::Error::last_os_error()
            );
            return None;
        }

        eprintln!("  DrmAttachment: imported as GEM handle {}", prime.handle);

        Some(Self {
            drm_fd,
            gem_handle: prime.handle,
        })
    }
}

impl Drop for DrmAttachment {
    fn drop(&mut self) {
        let close = DrmGemClose {
            handle: self.gem_handle,
            pad: 0,
        };
        unsafe { libc::ioctl(self.drm_fd.as_raw_fd(), DRM_IOCTL_GEM_CLOSE, &close) };
    }
}

// =============================================================================
// Heap type abstraction
// =============================================================================

#[derive(Debug, Clone, Copy, PartialEq)]
enum HeapType {
    /// `/dev/dma_heap/linux,cma-uncached` — non-cacheable mapping, GPU writes
    /// are immediately visible to CPU reads without cache maintenance.
    Uncached,
    /// `/dev/dma_heap/linux,cma` — cached mapping, requires DMA_BUF_IOCTL_SYNC
    /// for CPU cache coherency after GPU DMA writes.
    Cached,
}

impl HeapType {
    fn name(&self) -> &str {
        match self {
            HeapType::Uncached => "linux,cma-uncached",
            HeapType::Cached => "linux,cma",
        }
    }

    fn heap_kind(&self) -> HeapKind {
        match self {
            HeapType::Uncached => {
                HeapKind::Custom(std::path::PathBuf::from("/dev/dma_heap/linux,cma-uncached"))
            }
            HeapType::Cached => HeapKind::Cma,
        }
    }

    fn is_available(&self) -> bool {
        Heap::new(self.heap_kind()).is_ok()
    }
}

impl std::fmt::Display for HeapType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

/// Run a test body with the given heap type, skipping if unavailable.
fn with_heap<F>(heap_type: HeapType, test_name: &str, f: F)
where
    F: FnOnce(HeapType),
{
    let _ = env_logger::try_init();
    if !heap_type.is_available() {
        eprintln!("SKIP {test_name}: {heap_type} heap not available");
        return;
    }
    eprintln!("RUN  {test_name} on {heap_type} heap");
    f(heap_type);
    eprintln!("PASS {test_name} on {heap_type} heap");
}

/// Macro to generate cached and uncached variants of a test.
macro_rules! heap_tests {
    ($base:ident, $body:ident) => {
        paste::paste! {
            #[test]
            fn [<$base _uncached>]() {
                with_heap(HeapType::Uncached, stringify!([<$base _uncached>]), |h| $body(h));
            }

            #[test]
            fn [<$base _cached>]() {
                with_heap(HeapType::Cached, stringify!([<$base _cached>]), |h| $body(h));
            }
        }
    };
}

// =============================================================================
// DMA Buffer with persistent mmap and proper DMA_BUF_IOCTL_SYNC
// =============================================================================

/// DMA buffer with persistent mmap and correct DMA_BUF_IOCTL_SYNC usage.
///
/// The buffer is mmapped once on creation and munmapped on drop. CPU access
/// is bracketed by SYNC_START/SYNC_END ioctls with full return value checking.
///
/// This follows the Linux DMA-buf CPU access protocol:
/// 1. `DMA_BUF_IOCTL_SYNC` with `SYNC_START` — begin CPU access
/// 2. CPU reads/writes via the persistent mmap
/// 3. `DMA_BUF_IOCTL_SYNC` with `SYNC_END` — end CPU access
struct DmaBuffer {
    fd: OwnedFd,
    phys: G2DPhysical,
    ptr: *mut u8,
    size: usize,
    heap_type: HeapType,
    /// DRM PRIME import handle — keeps a persistent dma_buf_attach alive so that
    /// DMA_BUF_IOCTL_SYNC actually performs cache maintenance on cached heaps.
    _drm_attachment: Option<DrmAttachment>,
}

impl DmaBuffer {
    fn new(heap_type: HeapType, size: usize) -> Result<Self, Box<dyn std::error::Error>> {
        let heap = Heap::new(heap_type.heap_kind())
            .map_err(|e| format!("Failed to open {heap_type} heap: {e}"))?;

        let fd = heap
            .allocate(size)
            .map_err(|e| format!("Failed to allocate {size} bytes from {heap_type} heap: {e}"))?;

        let phys = G2DPhysical::new(fd.as_raw_fd())?;

        // Persistent mmap — mapped once for the buffer's lifetime
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
            return Err(format!(
                "mmap failed for {heap_type} heap buffer ({size} bytes): {}",
                std::io::Error::last_os_error()
            )
            .into());
        }

        // For cached heaps, create a persistent DRM PRIME import so that
        // DMA_BUF_IOCTL_SYNC actually performs cache maintenance.
        // Without this, begin_cpu_access iterates an empty attachment list.
        let drm_attachment = if heap_type == HeapType::Cached {
            DrmAttachment::new(&fd)
        } else {
            None
        };

        eprintln!(
            "  DmaBuffer: {size} bytes from {heap_type} heap, phys=0x{:x}, drm_attach={}",
            phys.address(),
            drm_attachment.is_some()
        );

        Ok(Self {
            fd,
            phys,
            ptr: ptr as *mut u8,
            size,
            heap_type,
            _drm_attachment: drm_attachment,
        })
    }

    fn address(&self) -> u64 {
        self.phys.address()
    }

    /// Perform DMA_BUF_IOCTL_SYNC with full error checking.
    fn dma_buf_sync(&self, flags: u64) {
        let sync = DmaBufSync { flags };
        let ret = unsafe { libc::ioctl(self.fd.as_raw_fd(), DMA_BUF_IOCTL_SYNC_CMD, &sync) };
        assert_ne!(
            ret,
            -1,
            "DMA_BUF_IOCTL_SYNC (flags=0x{:x}) failed on {heap} heap: {err}",
            flags,
            heap = self.heap_type,
            err = std::io::Error::last_os_error()
        );
    }

    /// Begin CPU access with the given direction flags.
    fn sync_start(&self, flags: u64) {
        self.dma_buf_sync(flags | DMA_BUF_SYNC_START);
    }

    /// End CPU access with the given direction flags.
    fn sync_end(&self, flags: u64) {
        self.dma_buf_sync(flags | DMA_BUF_SYNC_END);
    }

    /// Write to the buffer with proper sync bracketing.
    ///
    /// Uses `DMA_BUF_SYNC_WRITE` — tells the kernel the CPU will write,
    /// so it can clean/flush caches on SYNC_END.
    fn write_with<F: FnOnce(&mut [u8])>(&self, f: F) {
        self.sync_start(DMA_BUF_SYNC_WRITE);
        f(unsafe { std::slice::from_raw_parts_mut(self.ptr, self.size) });
        self.sync_end(DMA_BUF_SYNC_WRITE);
    }

    /// Read from the buffer with proper sync bracketing.
    ///
    /// Uses `DMA_BUF_SYNC_READ` — tells the kernel the CPU will read,
    /// so it can invalidate caches on SYNC_START to see GPU/DMA writes.
    fn read_with<F: FnOnce(&[u8]) -> T, T>(&self, f: F) -> T {
        self.sync_start(DMA_BUF_SYNC_READ);
        let result = f(unsafe { std::slice::from_raw_parts(self.ptr, self.size) });
        self.sync_end(DMA_BUF_SYNC_READ);
        result
    }
}

impl Drop for DmaBuffer {
    fn drop(&mut self) {
        let ret = unsafe { libc::munmap(self.ptr as *mut libc::c_void, self.size) };
        if ret != 0 {
            eprintln!(
                "WARNING: munmap failed for {heap} heap buffer: {err}",
                heap = self.heap_type,
                err = std::io::Error::last_os_error()
            );
        }
    }
}

// =============================================================================
// Surface creation helpers
// =============================================================================

/// Create a G2DSurface for a DMA buffer with given dimensions and format.
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

/// Create a G2DSurface for NV12 (two-plane format).
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
// Basic API Tests (no DMA heap dependency)
// =============================================================================

#[test]
fn test_g2d_open_close() {
    let _ = env_logger::try_init();

    let g2d = G2D::new("libg2d.so.2");
    assert!(g2d.is_ok(), "Failed to open G2D: {:?}", g2d.err());

    let g2d = g2d.unwrap();
    eprintln!("G2D version: {}", g2d.version());
}

#[test]
fn test_g2d_version_detection() {
    let _ = env_logger::try_init();

    let g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");
    let version = g2d.version();

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

    eprintln!("Detected G2D version: {version}");
}

#[test]
fn test_g2d_colorspace_configuration() {
    let _ = env_logger::try_init();

    let mut g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");

    let result = g2d.set_bt709_colorspace();
    assert!(result.is_ok(), "Failed to set BT.709: {:?}", result.err());

    let result = g2d.set_bt601_colorspace();
    assert!(result.is_ok(), "Failed to set BT.601: {:?}", result.err());
}

// =============================================================================
// Format Conversion Tests
// =============================================================================

#[test]
fn test_g2d_format_conversion() {
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
// Heap Availability Tests
// =============================================================================

#[test]
fn test_heap_availability() {
    let _ = env_logger::try_init();

    for heap_type in [HeapType::Uncached, HeapType::Cached] {
        if heap_type.is_available() {
            eprintln!("  {heap_type}: AVAILABLE");
        } else {
            eprintln!("  {heap_type}: NOT AVAILABLE");
        }
    }

    // At least one heap must be available for the test suite to be useful
    assert!(
        HeapType::Uncached.is_available() || HeapType::Cached.is_available(),
        "No DMA heap available — cannot run hardware tests"
    );
}

// =============================================================================
// Physical Address Tests
// =============================================================================

fn physical_address_test(heap_type: HeapType) {
    let size = 4096;
    let buf = DmaBuffer::new(heap_type, size).expect("Failed to allocate DMA buffer");

    let phys_addr = buf.address();
    assert!(phys_addr != 0, "Physical address should not be zero");
    eprintln!("  Physical address: 0x{phys_addr:x}");
}
heap_tests!(test_g2d_physical_address, physical_address_test);

// =============================================================================
// Clear Operation Tests (DMA-buf buffers, uncached + cached)
// =============================================================================

fn clear_rgba_test(heap_type: HeapType) {
    let width = 64;
    let height = 64;
    let size = width * height * 4;

    let buf = DmaBuffer::new(heap_type, size).expect("Failed to allocate DMA buffer");
    buf.write_with(|data| data.fill(0));

    let g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");
    let mut surface = create_surface(&buf, width, height, g2d_format_G2D_RGBA8888);

    let color = [255u8, 0, 0, 255];
    let result = g2d.clear(&mut surface, color);
    assert!(result.is_ok(), "G2D clear failed: {:?}", result.err());
    g2d.finish().unwrap();

    buf.read_with(|data| {
        for i in 0..10 {
            let offset = i * 4;
            assert_eq!(data[offset], 255, "Red channel mismatch at pixel {i}");
            assert_eq!(data[offset + 1], 0, "Green channel mismatch at pixel {i}");
            assert_eq!(data[offset + 2], 0, "Blue channel mismatch at pixel {i}");
            assert_eq!(data[offset + 3], 255, "Alpha channel mismatch at pixel {i}");
        }
    });
}
heap_tests!(test_g2d_clear_rgba, clear_rgba_test);

fn clear_multiple_colors_test(heap_type: HeapType) {
    let width = 32;
    let height = 32;
    let size = width * height * 4;

    let buf = DmaBuffer::new(heap_type, size).expect("Failed to allocate DMA buffer");
    let g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");
    let mut surface = create_surface(&buf, width, height, g2d_format_G2D_RGBA8888);

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
        g2d.finish().unwrap();

        buf.read_with(|data| {
            for pixel in [0, 10, 100, width * height - 1] {
                let offset = pixel * 4;
                assert_eq!(
                    &data[offset..offset + 4],
                    &color,
                    "Color mismatch at pixel {pixel} for {color:?}"
                );
            }
        });
    }
}
heap_tests!(test_g2d_clear_multiple_colors, clear_multiple_colors_test);

fn clear_large_surface_test(heap_type: HeapType) {
    let width = 1920;
    let height = 1080;
    let size = width * height * 4;

    let buf = DmaBuffer::new(heap_type, size).expect("Failed to allocate DMA buffer");
    let g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");
    let mut surface = create_surface(&buf, width, height, g2d_format_G2D_RGBA8888);

    let color = [0u8, 128, 255, 255]; // Blue-ish
    let result = g2d.clear(&mut surface, color);
    assert!(result.is_ok(), "G2D clear 1080p failed: {:?}", result.err());
    g2d.finish().unwrap();

    buf.read_with(|data| {
        let pixels_to_check = [
            0,                                // top-left
            width - 1,                        // top-right
            (height / 2) * width + width / 2, // center
            (height - 1) * width,             // bottom-left
            (height - 1) * width + width - 1, // bottom-right
        ];
        for pixel in pixels_to_check {
            let offset = pixel * 4;
            assert_eq!(
                &data[offset..offset + 4],
                &color,
                "Color mismatch at pixel {pixel}"
            );
        }
    });
}
heap_tests!(test_g2d_clear_large_surface, clear_large_surface_test);

// =============================================================================
// Clear Format Tests — g2d_clear with various destination formats
// =============================================================================
//
// The clrcolor field is always in RGBA8888 format. The GPU converts it to the
// destination surface format. These tests verify g2d_clear works correctly
// with each supported RGB destination format.

/// Verify that g2d_clear rejects formats not supported as clear destinations.
///
/// As of G2D v6.4.11 (i.MX 8M Plus), g2d_clear only supports 2-byte (565)
/// and 4-byte (8888) RGB formats. 3-byte RGB and all YUV formats are rejected.
///
/// **NOTE FOR FUTURE DEVELOPERS:** If this test starts FAILING, it means
/// g2d_clear now SUCCEEDS on a format that previously returned an error.
/// This is GOOD NEWS — the GPU driver has gained new clear capabilities!
/// Update the test: move the newly-supported format from this list into
/// `clear_all_rgb_formats_test` (or add a new byte-level verification test),
/// and remove it from the unsupported list here.
fn clear_unsupported_formats_test(heap_type: HeapType) {
    let width = 64;
    let height = 64;

    // Allocate a buffer large enough for any format (4 bpp * 64 * 64 = 16 KiB)
    let size = width * height * 4;
    let buf = DmaBuffer::new(heap_type, size).expect("Failed to allocate DMA buffer");
    let g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");

    // Formats that g2d_clear does NOT support as of G2D v6.4.11.
    // Each entry: (format constant, human-readable name).
    //
    // Note: YUYV and UYVY ARE supported (tested in clear_all_formats_test).
    // Only YVYU/VYUY are rejected among packed YUV 4:2:2 formats.
    let unsupported: &[(g2d_format, &str)] = &[
        // 3-byte RGB — hardware only supports 2-byte and 4-byte clear targets
        (g2d_format_G2D_RGB888, "RGB888"),
        (g2d_format_G2D_BGR888, "BGR888"),
        // Packed YUV 4:2:2 (only YVYU/VYUY; YUYV/UYVY are supported)
        (g2d_format_G2D_YVYU, "YVYU"),
        (g2d_format_G2D_VYUY, "VYUY"),
        // Semi-planar YUV 4:2:0
        (g2d_format_G2D_NV12, "NV12"),
        (g2d_format_G2D_NV21, "NV21"),
        // Planar YUV 4:2:0
        (g2d_format_G2D_I420, "I420"),
        (g2d_format_G2D_YV12, "YV12"),
        // Semi-planar YUV 4:2:2
        (g2d_format_G2D_NV16, "NV16"),
        (g2d_format_G2D_NV61, "NV61"),
    ];

    let mut newly_supported = Vec::new();
    for &(format, name) in unsupported {
        let mut surface = create_surface(&buf, width, height, format);
        let result = g2d.clear(&mut surface, [255, 0, 0, 255]);
        if result.is_ok() {
            eprintln!("  {name}: UNEXPECTEDLY SUCCEEDED — driver now supports this format!");
            newly_supported.push(name);
        } else {
            eprintln!("  {name}: correctly rejected");
        }
    }
    assert!(
        newly_supported.is_empty(),
        "g2d_clear now SUCCEEDS for previously unsupported formats: {newly_supported:?}. \
         This is GOOD NEWS — the GPU driver has gained new clear capabilities! \
         Move these formats from clear_unsupported_formats_test into \
         clear_all_rgb_formats_test (or add dedicated byte-verification tests)."
    );
}
heap_tests!(
    test_g2d_clear_unsupported_formats,
    clear_unsupported_formats_test
);

fn clear_bgra8888_test(heap_type: HeapType) {
    let width = 64;
    let height = 64;
    let bpp = 4;
    let size = width * height * bpp;

    let buf = DmaBuffer::new(heap_type, size).expect("Failed to allocate DMA buffer");
    buf.write_with(|data| data.fill(0));

    let g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");
    let mut surface = create_surface(&buf, width, height, g2d_format_G2D_BGRA8888);

    // Clear with red (clrcolor is RGBA8888)
    let color = [255u8, 0, 0, 255];
    let result = g2d.clear(&mut surface, color);
    assert!(
        result.is_ok(),
        "G2D clear BGRA8888 failed: {:?}",
        result.err()
    );
    g2d.finish().unwrap();

    // BGRA8888 memory layout: [B, G, R, A] per pixel
    buf.read_with(|data| {
        for i in 0..10 {
            let off = i * bpp;
            assert_eq!(data[off], 0, "B mismatch at pixel {i}");
            assert_eq!(data[off + 1], 0, "G mismatch at pixel {i}");
            assert_eq!(data[off + 2], 255, "R mismatch at pixel {i}");
            assert_eq!(data[off + 3], 255, "A mismatch at pixel {i}");
        }
    });
}
heap_tests!(test_g2d_clear_bgra8888, clear_bgra8888_test);

fn clear_argb8888_test(heap_type: HeapType) {
    let width = 64;
    let height = 64;
    let bpp = 4;
    let size = width * height * bpp;

    let buf = DmaBuffer::new(heap_type, size).expect("Failed to allocate DMA buffer");
    buf.write_with(|data| data.fill(0));

    let g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");
    let mut surface = create_surface(&buf, width, height, g2d_format_G2D_ARGB8888);

    // Clear with red (clrcolor is RGBA8888)
    let color = [255u8, 0, 0, 255];
    let result = g2d.clear(&mut surface, color);
    assert!(
        result.is_ok(),
        "G2D clear ARGB8888 failed: {:?}",
        result.err()
    );
    g2d.finish().unwrap();

    // ARGB8888 memory layout: [A, R, G, B] per pixel
    buf.read_with(|data| {
        for i in 0..10 {
            let off = i * bpp;
            assert_eq!(data[off], 255, "A mismatch at pixel {i}");
            assert_eq!(data[off + 1], 255, "R mismatch at pixel {i}");
            assert_eq!(data[off + 2], 0, "G mismatch at pixel {i}");
            assert_eq!(data[off + 3], 0, "B mismatch at pixel {i}");
        }
    });
}
heap_tests!(test_g2d_clear_argb8888, clear_argb8888_test);

fn clear_rgb565_test(heap_type: HeapType) {
    let width = 64;
    let height = 64;
    let bpp = 2;
    let size = width * height * bpp;

    let buf = DmaBuffer::new(heap_type, size).expect("Failed to allocate DMA buffer");
    buf.write_with(|data| data.fill(0));

    let g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");
    let mut surface = create_surface(&buf, width, height, g2d_format_G2D_RGB565);

    // RGB565 LE layout: R(15:11) G(10:5) B(4:0)
    // Pure red   → R=31 G=0 B=0  → 0xF800
    // Pure green → R=0  G=63 B=0 → 0x07E0
    // Pure blue  → R=0  G=0 B=31 → 0x001F
    // White      → all-ones       → 0xFFFF
    let test_cases: [([u8; 4], u16, &str); 4] = [
        ([255, 0, 0, 255], 0xF800, "red"),
        ([0, 255, 0, 255], 0x07E0, "green"),
        ([0, 0, 255, 255], 0x001F, "blue"),
        ([255, 255, 255, 255], 0xFFFF, "white"),
    ];

    for (color, expected, name) in &test_cases {
        let result = g2d.clear(&mut surface, *color);
        assert!(
            result.is_ok(),
            "G2D clear RGB565 {name} failed: {:?}",
            result.err()
        );
        g2d.finish().unwrap();

        buf.read_with(|data| {
            for i in 0..10 {
                let off = i * bpp;
                let pixel = u16::from_le_bytes([data[off], data[off + 1]]);
                assert_eq!(
                    pixel, *expected,
                    "RGB565 {name} mismatch at pixel {i}: got 0x{pixel:04X}, expected 0x{expected:04X}"
                );
            }
        });
    }
}
heap_tests!(test_g2d_clear_rgb565, clear_rgb565_test);

/// Bytes per pixel for a g2d_format, or None for multi-plane/unsupported formats.
#[allow(non_upper_case_globals)]
fn format_bpp(format: g2d_format) -> Option<usize> {
    match format {
        g2d_format_G2D_RGB565 | g2d_format_G2D_BGR565 => Some(2),
        g2d_format_G2D_YUYV | g2d_format_G2D_UYVY => Some(2),
        g2d_format_G2D_RGB888 | g2d_format_G2D_BGR888 => Some(3),
        g2d_format_G2D_RGBA8888
        | g2d_format_G2D_RGBX8888
        | g2d_format_G2D_BGRA8888
        | g2d_format_G2D_BGRX8888
        | g2d_format_G2D_ARGB8888
        | g2d_format_G2D_ABGR8888
        | g2d_format_G2D_XRGB8888
        | g2d_format_G2D_XBGR8888 => Some(4),
        _ => None,
    }
}

/// Format name for diagnostic output.
#[allow(non_upper_case_globals)]
fn format_name(format: g2d_format) -> &'static str {
    match format {
        g2d_format_G2D_RGB565 => "RGB565",
        g2d_format_G2D_RGBA8888 => "RGBA8888",
        g2d_format_G2D_RGBX8888 => "RGBX8888",
        g2d_format_G2D_BGRA8888 => "BGRA8888",
        g2d_format_G2D_BGRX8888 => "BGRX8888",
        g2d_format_G2D_BGR565 => "BGR565",
        g2d_format_G2D_ARGB8888 => "ARGB8888",
        g2d_format_G2D_ABGR8888 => "ABGR8888",
        g2d_format_G2D_XRGB8888 => "XRGB8888",
        g2d_format_G2D_XBGR8888 => "XBGR8888",
        g2d_format_G2D_RGB888 => "RGB888",
        g2d_format_G2D_BGR888 => "BGR888",
        g2d_format_G2D_YUYV => "YUYV",
        g2d_format_G2D_UYVY => "UYVY",
        _ => "unknown",
    }
}

/// Comprehensive clear test across all supported destination formats.
///
/// For each format: clear with two different colors and verify the buffer
/// contents change between clears (not stale). This validates that g2d_clear
/// correctly handles the format without requiring exact byte-order knowledge
/// for every variant.
///
/// Unsupported formats are tested separately in `clear_unsupported_formats_test`.
fn clear_all_formats_test(heap_type: HeapType) {
    let width = 64;
    let height = 64;

    let formats = [
        // 16-bit RGB
        g2d_format_G2D_RGB565,
        g2d_format_G2D_BGR565,
        // 32-bit RGB variants
        g2d_format_G2D_RGBA8888,
        g2d_format_G2D_RGBX8888,
        g2d_format_G2D_BGRA8888,
        g2d_format_G2D_BGRX8888,
        g2d_format_G2D_ARGB8888,
        g2d_format_G2D_ABGR8888,
        g2d_format_G2D_XRGB8888,
        g2d_format_G2D_XBGR8888,
        // Packed YUV 4:2:2 (YUYV/UYVY supported; YVYU/VYUY are not)
        g2d_format_G2D_YUYV,
        g2d_format_G2D_UYVY,
    ];

    let g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");

    for format in formats {
        let name = format_name(format);
        let bpp = format_bpp(format).expect("unknown bpp");
        let size = width * height * bpp;

        let buf = DmaBuffer::new(heap_type, size).expect("Failed to allocate DMA buffer");
        buf.write_with(|data| data.fill(0));

        let mut surface = create_surface(&buf, width, height, format);

        // Clear with red
        let red = [255u8, 0, 0, 255];
        let result = g2d.clear(&mut surface, red);
        assert!(
            result.is_ok(),
            "{name}: clear with red failed: {:?}",
            result.err()
        );
        g2d.finish().unwrap();

        let red_snapshot = buf.read_with(|data| data[..bpp * 10].to_vec());

        // Buffer must not be all zeros after clear
        assert!(
            red_snapshot.iter().any(|&b| b != 0),
            "{name}: buffer still all zeros after red clear"
        );

        // Clear with blue
        let blue = [0u8, 0, 255, 255];
        let result = g2d.clear(&mut surface, blue);
        assert!(
            result.is_ok(),
            "{name}: clear with blue failed: {:?}",
            result.err()
        );
        g2d.finish().unwrap();

        let blue_snapshot = buf.read_with(|data| data[..bpp * 10].to_vec());

        // Blue clear must produce different bytes than red clear
        assert_ne!(
            red_snapshot, blue_snapshot,
            "{name}: buffer unchanged between red and blue clears (stale data?)"
        );

        eprintln!("  {name} ({bpp} bpp): OK");
    }
}
heap_tests!(test_g2d_clear_all_formats, clear_all_formats_test);

// =============================================================================
// Partial Clear Tests — sub-region clearing for letterbox
// =============================================================================

/// Test that g2d_clear respects the left/top/right/bottom region of interest.
///
/// Clears only the top and bottom bars of an RGBA surface (simulating
/// letterbox borders) and verifies that the content area is untouched.
fn clear_partial_region_test(heap_type: HeapType) {
    let width = 128;
    let height = 128;
    let bpp = 4;
    let size = width * height * bpp;

    let buf = DmaBuffer::new(heap_type, size).expect("Failed to allocate DMA buffer");

    // Fill entire buffer with a known pattern (green)
    let green = [0u8, 255, 0, 255];
    buf.write_with(|data| {
        for chunk in data.chunks_exact_mut(4) {
            chunk.copy_from_slice(&green);
        }
    });

    let g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");

    // Clear only the top 32 rows with red
    let red = [255u8, 0, 0, 255];
    let mut top_surface = create_surface(&buf, width, height, g2d_format_G2D_RGBA8888);
    top_surface.left = 0;
    top_surface.top = 0;
    top_surface.right = width as i32;
    top_surface.bottom = 32;
    g2d.clear(&mut top_surface, red).unwrap();

    // Clear only the bottom 32 rows with blue
    let blue = [0u8, 0, 255, 255];
    let mut bottom_surface = create_surface(&buf, width, height, g2d_format_G2D_RGBA8888);
    bottom_surface.left = 0;
    bottom_surface.top = 96;
    bottom_surface.right = width as i32;
    bottom_surface.bottom = 128;
    g2d.clear(&mut bottom_surface, blue).unwrap();

    // Single finish for both clears
    g2d.finish().unwrap();

    buf.read_with(|data| {
        // Top bar (rows 0-31): should be red
        for row in 0..32 {
            let offset = row * width * bpp;
            assert_eq!(
                &data[offset..offset + 4],
                &red,
                "Top bar pixel at row {row} should be red"
            );
        }

        // Content area (rows 32-95): should still be green (untouched)
        for row in 32..96 {
            let offset = row * width * bpp;
            assert_eq!(
                &data[offset..offset + 4],
                &green,
                "Content area pixel at row {row} should be green (untouched)"
            );
        }

        // Bottom bar (rows 96-127): should be blue
        for row in 96..128 {
            let offset = row * width * bpp;
            assert_eq!(
                &data[offset..offset + 4],
                &blue,
                "Bottom bar pixel at row {row} should be blue"
            );
        }
    });
}
heap_tests!(test_g2d_clear_partial_region, clear_partial_region_test);

/// Test partial clear with left/right vertical bars (portrait letterbox).
fn clear_partial_left_right_test(heap_type: HeapType) {
    let width = 128;
    let height = 64;
    let bpp = 4;
    let size = width * height * bpp;

    let buf = DmaBuffer::new(heap_type, size).expect("Failed to allocate DMA buffer");

    // Fill with green
    let green = [0u8, 255, 0, 255];
    buf.write_with(|data| {
        for chunk in data.chunks_exact_mut(4) {
            chunk.copy_from_slice(&green);
        }
    });

    let g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");
    let gray = [114u8, 114, 114, 255];

    // Clear left 16 columns
    let mut left_surface = create_surface(&buf, width, height, g2d_format_G2D_RGBA8888);
    left_surface.left = 0;
    left_surface.top = 0;
    left_surface.right = 16;
    left_surface.bottom = height as i32;
    g2d.clear(&mut left_surface, gray).unwrap();

    // Clear right 16 columns
    let mut right_surface = create_surface(&buf, width, height, g2d_format_G2D_RGBA8888);
    right_surface.left = 112;
    right_surface.top = 0;
    right_surface.right = 128;
    right_surface.bottom = height as i32;
    g2d.clear(&mut right_surface, gray).unwrap();

    g2d.finish().unwrap();

    buf.read_with(|data| {
        for row in 0..height {
            let row_offset = row * width * bpp;

            // Left bar (cols 0-15): gray
            assert_eq!(
                &data[row_offset..row_offset + 4],
                &gray,
                "Left bar at row {row} col 0 should be gray"
            );

            // Content (col 16): green
            let mid_offset = row_offset + 16 * bpp;
            assert_eq!(
                &data[mid_offset..mid_offset + 4],
                &green,
                "Content at row {row} col 16 should be green"
            );

            // Content (col 111): green
            let mid_offset = row_offset + 111 * bpp;
            assert_eq!(
                &data[mid_offset..mid_offset + 4],
                &green,
                "Content at row {row} col 111 should be green"
            );

            // Right bar (col 112): gray
            let right_offset = row_offset + 112 * bpp;
            assert_eq!(
                &data[right_offset..right_offset + 4],
                &gray,
                "Right bar at row {row} col 112 should be gray"
            );
        }
    });
}
heap_tests!(
    test_g2d_clear_partial_left_right,
    clear_partial_left_right_test
);

// =============================================================================
// Blit Operation Tests
// =============================================================================

fn blit_rgba_to_rgba_test(heap_type: HeapType) {
    let width = 64;
    let height = 64;
    let size = width * height * 4;

    let src_buf = DmaBuffer::new(heap_type, size).expect("Failed to allocate src buffer");
    let dst_buf = DmaBuffer::new(heap_type, size).expect("Failed to allocate dst buffer");

    src_buf.write_with(|data| {
        for (i, byte) in data.iter_mut().enumerate() {
            *byte = (i % 256) as u8;
        }
    });
    dst_buf.write_with(|data| data.fill(0));

    let mut g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");
    g2d.set_bt709_colorspace()
        .expect("Failed to set colorspace");

    let src_surface = create_surface(&src_buf, width, height, g2d_format_G2D_RGBA8888);
    let dst_surface = create_surface(&dst_buf, width, height, g2d_format_G2D_RGBA8888);

    let result = g2d.blit(&src_surface, &dst_surface);
    assert!(result.is_ok(), "G2D blit failed: {:?}", result.err());
    g2d.finish().unwrap();

    let src_snapshot = src_buf.read_with(|data| data[..100].to_vec());
    dst_buf.read_with(|data| {
        for i in 0..100 {
            assert_eq!(
                src_snapshot[i], data[i],
                "Data mismatch at byte {i}: src={} dst={}",
                src_snapshot[i], data[i]
            );
        }
    });
}
heap_tests!(test_g2d_blit_rgba_to_rgba, blit_rgba_to_rgba_test);

fn blit_with_scaling_test(heap_type: HeapType) {
    let src_width = 128;
    let src_height = 128;
    let dst_width = 64;
    let dst_height = 64;

    let src_size = src_width * src_height * 4;
    let dst_size = dst_width * dst_height * 4;

    let src_buf = DmaBuffer::new(heap_type, src_size).expect("Failed to allocate src buffer");
    let dst_buf = DmaBuffer::new(heap_type, dst_size).expect("Failed to allocate dst buffer");

    src_buf.write_with(|slice| {
        for y in 0..src_height {
            for x in 0..src_width {
                let offset = (y * src_width + x) * 4;
                slice[offset] = (x * 2) as u8;
                slice[offset + 1] = (y * 2) as u8;
                slice[offset + 2] = 128;
                slice[offset + 3] = 255;
            }
        }
    });
    dst_buf.write_with(|data| data.fill(0));

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
    g2d.finish().unwrap();

    dst_buf.read_with(|dst_data| {
        let non_zero_count = dst_data.iter().filter(|&&b| b != 0).count();
        assert!(
            non_zero_count > dst_size / 2,
            "Destination buffer appears empty after scaling"
        );
    });
}
heap_tests!(test_g2d_blit_with_scaling, blit_with_scaling_test);

fn blit_rgba_to_rgb_test(heap_type: HeapType) {
    let width = 64;
    let height = 64;
    let src_size = width * height * 4; // RGBA
    let dst_size = width * height * 3; // RGB

    let src_buf = DmaBuffer::new(heap_type, src_size).expect("Failed to allocate src buffer");
    let dst_buf = DmaBuffer::new(heap_type, dst_size).expect("Failed to allocate dst buffer");

    src_buf.write_with(|slice| {
        for i in 0..(width * height) {
            let offset = i * 4;
            slice[offset] = 255;
            slice[offset + 1] = 0;
            slice[offset + 2] = 0;
            slice[offset + 3] = 255;
        }
    });
    dst_buf.write_with(|data| data.fill(0));

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
    g2d.finish().unwrap();

    dst_buf.read_with(|dst_data| {
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
    });
}
heap_tests!(test_g2d_blit_rgba_to_rgb, blit_rgba_to_rgb_test);

// =============================================================================
// YUV Format Tests
// =============================================================================

fn blit_yuyv_to_rgba_test(heap_type: HeapType) {
    let width = 64;
    let height = 64;
    let src_size = width * height * 2; // YUYV = 2 bytes per pixel
    let dst_size = width * height * 4; // RGBA

    let src_buf = DmaBuffer::new(heap_type, src_size).expect("Failed to allocate src buffer");
    let dst_buf = DmaBuffer::new(heap_type, dst_size).expect("Failed to allocate dst buffer");

    src_buf.write_with(|slice| {
        for i in 0..(src_size / 4) {
            let offset = i * 4;
            slice[offset] = 128; // Y0
            slice[offset + 1] = 128; // U
            slice[offset + 2] = 128; // Y1
            slice[offset + 3] = 128; // V
        }
    });
    dst_buf.write_with(|data| data.fill(0));

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
    g2d.finish().unwrap();

    dst_buf.read_with(|dst_data| {
        let non_zero = dst_data.iter().filter(|&&b| b != 0).count();
        assert!(
            non_zero > dst_size / 4,
            "Destination appears empty after YUV conversion"
        );
    });
}
heap_tests!(test_g2d_blit_yuyv_to_rgba, blit_yuyv_to_rgba_test);

fn blit_nv12_to_rgba_test(heap_type: HeapType) {
    let width = 64;
    let height = 64;
    let src_size = width * height + width * height / 2; // Y + UV
    let dst_size = width * height * 4;

    let src_buf = DmaBuffer::new(heap_type, src_size).expect("Failed to allocate src buffer");
    let dst_buf = DmaBuffer::new(heap_type, dst_size).expect("Failed to allocate dst buffer");

    let y_size = width * height;
    src_buf.write_with(|data| {
        data[..y_size].fill(128);
        data[y_size..].fill(128);
    });
    dst_buf.write_with(|data| data.fill(0));

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
    g2d.finish().unwrap();

    dst_buf.read_with(|dst_data| {
        let non_zero = dst_data.iter().filter(|&&b| b != 0).count();
        assert!(
            non_zero > dst_size / 4,
            "Destination appears empty after NV12 conversion"
        );
    });
}
heap_tests!(test_g2d_blit_nv12_to_rgba, blit_nv12_to_rgba_test);

// =============================================================================
// Cache Coherency Correctness Tests (Phase 2)
// =============================================================================

/// Double-write overwrite test — the most likely to expose stale cache issues.
///
/// Sequence:
/// 1. GPU clears buffer with color A
/// 2. CPU reads and verifies color A
/// 3. GPU clears same buffer with color B
/// 4. CPU reads and verifies color B (must NOT see stale color A)
fn double_write_overwrite_test(heap_type: HeapType) {
    let width = 64;
    let height = 64;
    let size = width * height * 4;

    let buf = DmaBuffer::new(heap_type, size).expect("Failed to allocate DMA buffer");
    let g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");
    let mut surface = create_surface(&buf, width, height, g2d_format_G2D_RGBA8888);

    let color_a = [255u8, 0, 0, 255]; // Red
    let color_b = [0u8, 0, 255, 255]; // Blue

    // Step 1: GPU clears with color A
    let result = g2d.clear(&mut surface, color_a);
    assert!(
        result.is_ok(),
        "Clear with color A failed: {:?}",
        result.err()
    );
    g2d.finish().unwrap();

    // Step 2: CPU reads — should see color A
    buf.read_with(|data| {
        for pixel in [0, 100, width * height / 2, width * height - 1] {
            let offset = pixel * 4;
            assert_eq!(
                &data[offset..offset + 4],
                &color_a,
                "Step 2: expected color A at pixel {pixel}, got {:?}",
                &data[offset..offset + 4]
            );
        }
    });

    // Step 3: GPU clears with color B (overwrite)
    let result = g2d.clear(&mut surface, color_b);
    assert!(
        result.is_ok(),
        "Clear with color B failed: {:?}",
        result.err()
    );
    g2d.finish().unwrap();

    // Step 4: CPU reads — MUST see color B, not stale color A
    buf.read_with(|data| {
        for pixel in [0, 100, width * height / 2, width * height - 1] {
            let offset = pixel * 4;
            assert_eq!(
                &data[offset..offset + 4],
                &color_b,
                "Step 4: STALE DATA — expected color B at pixel {pixel}, got {:?} (color A was {:?})",
                &data[offset..offset + 4],
                color_a
            );
        }
    });
}
heap_tests!(test_double_write_overwrite, double_write_overwrite_test);

/// Multiple reads without intervening GPU operations.
///
/// After a single GPU write, multiple CPU reads should all return the same data.
fn multi_read_consistency_test(heap_type: HeapType) {
    let width = 64;
    let height = 64;
    let size = width * height * 4;

    let buf = DmaBuffer::new(heap_type, size).expect("Failed to allocate DMA buffer");
    let g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");
    let mut surface = create_surface(&buf, width, height, g2d_format_G2D_RGBA8888);

    let color = [0u8, 255, 0, 255]; // Green
    let result = g2d.clear(&mut surface, color);
    assert!(result.is_ok(), "Clear failed: {:?}", result.err());
    g2d.finish().unwrap();

    // Read 5 times — all must return the same data
    for read_num in 0..5 {
        buf.read_with(|data| {
            for pixel in [0, width * height / 2, width * height - 1] {
                let offset = pixel * 4;
                assert_eq!(
                    &data[offset..offset + 4],
                    &color,
                    "Read #{read_num}: color mismatch at pixel {pixel}"
                );
            }
        });
    }
}
heap_tests!(test_multi_read_consistency, multi_read_consistency_test);

/// Full CPU-write, GPU-read, GPU-write, CPU-read round-trip.
///
/// 1. CPU writes known pattern to source buffer
/// 2. GPU blits source → destination
/// 3. CPU reads destination and verifies pattern
fn cpu_gpu_roundtrip_test(heap_type: HeapType) {
    let width = 64;
    let height = 64;
    let size = width * height * 4;

    let src_buf = DmaBuffer::new(heap_type, size).expect("Failed to allocate src buffer");
    let dst_buf = DmaBuffer::new(heap_type, size).expect("Failed to allocate dst buffer");

    // CPU writes a known pattern to source
    src_buf.write_with(|data| {
        for i in 0..(width * height) {
            let offset = i * 4;
            data[offset] = (i % 251) as u8; // R (prime to avoid period alignment)
            data[offset + 1] = ((i * 3) % 251) as u8; // G
            data[offset + 2] = ((i * 7) % 251) as u8; // B
            data[offset + 3] = 255; // A
        }
    });
    dst_buf.write_with(|data| data.fill(0));

    // GPU blits source → destination
    let mut g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");
    g2d.set_bt709_colorspace()
        .expect("Failed to set colorspace");

    let src_surface = create_surface(&src_buf, width, height, g2d_format_G2D_RGBA8888);
    let dst_surface = create_surface(&dst_buf, width, height, g2d_format_G2D_RGBA8888);

    let result = g2d.blit(&src_surface, &dst_surface);
    assert!(result.is_ok(), "Blit failed: {:?}", result.err());
    g2d.finish().unwrap();

    // CPU reads destination — should match the pattern written to source
    let src_snapshot = src_buf.read_with(|data| data.to_vec());
    dst_buf.read_with(|dst_data| {
        let total_pixels = width * height;
        let mut mismatches = 0;
        for i in 0..total_pixels {
            let offset = i * 4;
            if dst_data[offset..offset + 4] != src_snapshot[offset..offset + 4] {
                if mismatches < 5 {
                    eprintln!(
                        "  Mismatch at pixel {i}: src={:?} dst={:?}",
                        &src_snapshot[offset..offset + 4],
                        &dst_data[offset..offset + 4]
                    );
                }
                mismatches += 1;
            }
        }
        assert_eq!(
            mismatches, 0,
            "Round-trip had {mismatches}/{total_pixels} pixel mismatches"
        );
    });
}
heap_tests!(test_cpu_gpu_roundtrip, cpu_gpu_roundtrip_test);

/// Sequential color cycling test — clears the same buffer with 6 different colors
/// in sequence, verifying every pixel after each clear. This tests that the
/// persistent mmap + sync correctly reflects each new GPU write.
fn sequential_color_cycle_test(heap_type: HeapType) {
    let width = 128;
    let height = 128;
    let size = width * height * 4;

    let buf = DmaBuffer::new(heap_type, size).expect("Failed to allocate DMA buffer");
    let g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");
    let mut surface = create_surface(&buf, width, height, g2d_format_G2D_RGBA8888);

    let colors: [[u8; 4]; 6] = [
        [255, 0, 0, 255],     // Red
        [0, 255, 0, 255],     // Green
        [0, 0, 255, 255],     // Blue
        [128, 128, 128, 255], // Gray
        [0, 0, 0, 255],       // Black
        [255, 255, 255, 255], // White
    ];

    for (round, color) in colors.iter().enumerate() {
        let result = g2d.clear(&mut surface, *color);
        assert!(
            result.is_ok(),
            "Round {round}: clear with {color:?} failed: {:?}",
            result.err()
        );
        g2d.finish().unwrap();

        // Verify ALL pixels
        buf.read_with(|data| {
            let total_pixels = width * height;
            let mut mismatches = 0;
            for pixel in 0..total_pixels {
                let offset = pixel * 4;
                if data[offset..offset + 4] != *color {
                    mismatches += 1;
                }
            }
            assert_eq!(
                mismatches, 0,
                "Round {round} ({color:?}): {mismatches}/{total_pixels} pixels wrong"
            );
        });
    }
}
heap_tests!(test_sequential_color_cycle, sequential_color_cycle_test);

// =============================================================================
// Stress Tests (Phase 5)
// =============================================================================

/// Stress test: 100 sequential clear+readback cycles with different colors.
fn stress_clear_100_test(heap_type: HeapType) {
    let width = 64;
    let height = 64;
    let size = width * height * 4;

    let buf = DmaBuffer::new(heap_type, size).expect("Failed to allocate DMA buffer");
    let g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");
    let mut surface = create_surface(&buf, width, height, g2d_format_G2D_RGBA8888);

    let start = Instant::now();

    for i in 0..100u32 {
        let color = [
            (i * 37 % 256) as u8,
            (i * 73 % 256) as u8,
            (i * 131 % 256) as u8,
            255u8,
        ];

        let result = g2d.clear(&mut surface, color);
        assert!(
            result.is_ok(),
            "Iteration {i}: clear failed: {:?}",
            result.err()
        );
        g2d.finish().unwrap();

        buf.read_with(|data| {
            // Spot check several pixels
            for pixel in [0, width * height / 2, width * height - 1] {
                let offset = pixel * 4;
                assert_eq!(
                    &data[offset..offset + 4],
                    &color,
                    "Iteration {i}: mismatch at pixel {pixel}, expected {color:?}, got {:?}",
                    &data[offset..offset + 4]
                );
            }
        });
    }

    let elapsed = start.elapsed();
    eprintln!(
        "  100 clear+readback cycles in {elapsed:.2?} ({:.2?}/cycle)",
        elapsed / 100
    );
}
heap_tests!(test_stress_clear_100, stress_clear_100_test);

/// Stress test: 100 blit+readback cycles.
fn stress_blit_100_test(heap_type: HeapType) {
    let width = 64;
    let height = 64;
    let size = width * height * 4;

    let src_buf = DmaBuffer::new(heap_type, size).expect("Failed to allocate src buffer");
    let dst_buf = DmaBuffer::new(heap_type, size).expect("Failed to allocate dst buffer");

    let mut g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");
    g2d.set_bt709_colorspace()
        .expect("Failed to set colorspace");

    let src_surface = create_surface(&src_buf, width, height, g2d_format_G2D_RGBA8888);
    let dst_surface = create_surface(&dst_buf, width, height, g2d_format_G2D_RGBA8888);

    let start = Instant::now();

    for i in 0..100u32 {
        // Write a unique pattern each iteration
        src_buf.write_with(|data| {
            for pixel in 0..(width * height) {
                let offset = pixel * 4;
                data[offset] = ((pixel + i as usize) % 256) as u8;
                data[offset + 1] = ((pixel * 3 + i as usize) % 256) as u8;
                data[offset + 2] = ((pixel * 7 + i as usize) % 256) as u8;
                data[offset + 3] = 255;
            }
        });

        let result = g2d.blit(&src_surface, &dst_surface);
        assert!(
            result.is_ok(),
            "Iteration {i}: blit failed: {:?}",
            result.err()
        );
        g2d.finish().unwrap();

        // Verify first few pixels match
        let src_snapshot = src_buf.read_with(|data| data[..16].to_vec());
        dst_buf.read_with(|data| {
            assert_eq!(
                &data[..16],
                &src_snapshot[..],
                "Iteration {i}: first 4 pixels mismatch"
            );
        });
    }

    let elapsed = start.elapsed();
    eprintln!(
        "  100 blit+readback cycles in {elapsed:.2?} ({:.2?}/cycle)",
        elapsed / 100
    );
}
heap_tests!(test_stress_blit_100, stress_blit_100_test);

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
