// SPDX-FileCopyrightText: Copyright 2025 Au-Zone Technologies
// SPDX-License-Identifier: Apache-2.0

//! Shared benchmark infrastructure for G2D criterion benchmarks.
//!
//! This module duplicates the DMA-buf and surface infrastructure from
//! `hardware_tests.rs` because benchmark and test compilation units cannot
//! share code directly.

#![allow(dead_code)]

use criterion::Throughput;
use dma_heap::{Heap, HeapKind};
use g2d_sys::{
    g2d_format_G2D_NV12, g2d_format_G2D_RGBA8888, g2d_format_G2D_YUYV, g2d_rotation_G2D_ROTATION_0,
    G2DPhysical, G2DSurface, G2D,
};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::ptr;
use std::sync::OnceLock;

// =============================================================================
// Hardware Availability Cache
// =============================================================================

static G2D_AVAILABLE: OnceLock<bool> = OnceLock::new();

/// Check if G2D hardware is available (cached).
pub fn g2d_available() -> bool {
    *G2D_AVAILABLE.get_or_init(|| G2D::new("libg2d.so.2").is_ok())
}

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

const DRM_IOCTL_BASE: u8 = b'd';

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
struct DrmAttachment {
    drm_fd: OwnedFd,
    gem_handle: u32,
}

impl DrmAttachment {
    fn new(dma_buf_fd: &OwnedFd) -> Option<Self> {
        let path = b"/dev/dri/renderD128\0";
        let raw_fd = unsafe {
            libc::open(
                path.as_ptr() as *const libc::c_char,
                libc::O_RDWR | libc::O_CLOEXEC,
            )
        };
        if raw_fd < 0 {
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
            return None;
        }

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
pub enum HeapType {
    Uncached,
    Cached,
}

impl HeapType {
    pub fn name(&self) -> &str {
        match self {
            HeapType::Uncached => "uncached",
            HeapType::Cached => "cached",
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

    pub fn is_available(&self) -> bool {
        Heap::new(self.heap_kind()).is_ok()
    }
}

impl std::fmt::Display for HeapType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.name())
    }
}

// =============================================================================
// DMA Buffer with persistent mmap and proper DMA_BUF_IOCTL_SYNC
// =============================================================================

pub struct DmaBuffer {
    fd: OwnedFd,
    phys: G2DPhysical,
    ptr: *mut u8,
    size: usize,
    heap_type: HeapType,
    _drm_attachment: Option<DrmAttachment>,
}

impl DmaBuffer {
    pub fn new(heap_type: HeapType, size: usize) -> Result<Self, Box<dyn std::error::Error>> {
        let heap = Heap::new(heap_type.heap_kind())
            .map_err(|e| format!("Failed to open {heap_type} heap: {e}"))?;

        let fd = heap
            .allocate(size)
            .map_err(|e| format!("Failed to allocate {size} bytes from {heap_type} heap: {e}"))?;

        let phys = G2DPhysical::new(fd.as_raw_fd())?;

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

        let drm_attachment = if heap_type == HeapType::Cached {
            DrmAttachment::new(&fd)
        } else {
            None
        };

        Ok(Self {
            fd,
            phys,
            ptr: ptr as *mut u8,
            size,
            heap_type,
            _drm_attachment: drm_attachment,
        })
    }

    pub fn address(&self) -> u64 {
        self.phys.address()
    }

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

    fn sync_start(&self, flags: u64) {
        self.dma_buf_sync(flags | DMA_BUF_SYNC_START);
    }

    fn sync_end(&self, flags: u64) {
        self.dma_buf_sync(flags | DMA_BUF_SYNC_END);
    }

    pub fn write_with<F: FnOnce(&mut [u8])>(&self, f: F) {
        self.sync_start(DMA_BUF_SYNC_WRITE);
        f(unsafe { std::slice::from_raw_parts_mut(self.ptr, self.size) });
        self.sync_end(DMA_BUF_SYNC_WRITE);
    }
}

impl Drop for DmaBuffer {
    fn drop(&mut self) {
        unsafe { libc::munmap(self.ptr as *mut libc::c_void, self.size) };
    }
}

// =============================================================================
// Surface creation helpers
// =============================================================================

pub fn create_surface(buf: &DmaBuffer, width: usize, height: usize, format: u32) -> G2DSurface {
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

pub fn create_nv12_surface(buf: &DmaBuffer, width: usize, height: usize) -> G2DSurface {
    let uv_offset = (width * height) as u64;
    G2DSurface {
        format: g2d_format_G2D_NV12,
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
// Source format constants
// =============================================================================

pub const SRC_FMT_NV12: u32 = g2d_format_G2D_NV12;
pub const SRC_FMT_YUYV: u32 = g2d_format_G2D_YUYV;
pub const SRC_FMT_RGBA: u32 = g2d_format_G2D_RGBA8888;
pub const DST_FMT_RGBA: u32 = g2d_format_G2D_RGBA8888;

// =============================================================================
// Benchmark Configuration
// =============================================================================

#[derive(Clone)]
pub struct BenchConfig {
    pub in_w: usize,
    pub in_h: usize,
    pub out_w: usize,
    pub out_h: usize,
    pub in_fmt: u32,
    pub out_fmt: u32,
}

impl BenchConfig {
    pub fn new(
        in_w: usize,
        in_h: usize,
        out_w: usize,
        out_h: usize,
        in_fmt: u32,
        out_fmt: u32,
    ) -> Self {
        Self {
            in_w,
            in_h,
            out_w,
            out_h,
            in_fmt,
            out_fmt,
        }
    }

    pub fn id(&self) -> String {
        if self.in_w == self.out_w && self.in_h == self.out_h {
            format!(
                "{}x{}/{}->{}",
                self.in_w,
                self.in_h,
                format_name(self.in_fmt),
                format_name(self.out_fmt)
            )
        } else {
            format!(
                "{}x{}/{}->{}x{}/{}",
                self.in_w,
                self.in_h,
                format_name(self.in_fmt),
                self.out_w,
                self.out_h,
                format_name(self.out_fmt)
            )
        }
    }

    pub fn throughput(&self) -> Throughput {
        Throughput::Bytes(self.src_buf_size() as u64)
    }

    /// Buffer size in bytes for the source format.
    pub fn src_buf_size(&self) -> usize {
        buf_size(self.in_w, self.in_h, self.in_fmt)
    }

    /// Buffer size in bytes for the destination format (always RGBA).
    pub fn dst_buf_size(&self) -> usize {
        buf_size(self.out_w, self.out_h, self.out_fmt)
    }
}

/// Calculate buffer size in bytes for a given resolution and G2D format.
pub fn buf_size(width: usize, height: usize, fmt: u32) -> usize {
    match fmt {
        f if f == SRC_FMT_NV12 => width * height * 3 / 2,
        f if f == SRC_FMT_YUYV => width * height * 2,
        f if f == SRC_FMT_RGBA => width * height * 4,
        _ => width * height * 4,
    }
}

/// Human-readable name for a G2D format constant.
pub fn format_name(fmt: u32) -> &'static str {
    match fmt {
        f if f == SRC_FMT_NV12 => "NV12",
        f if f == SRC_FMT_YUYV => "YUYV",
        f if f == SRC_FMT_RGBA => "RGBA",
        _ => "???",
    }
}

// =============================================================================
// Letterbox Calculation
// =============================================================================

/// Calculate letterbox dimensions to fit source into destination while
/// preserving aspect ratio.
///
/// Returns (left_offset, top_offset, scaled_width, scaled_height).
pub fn calculate_letterbox(
    src_w: usize,
    src_h: usize,
    dst_w: usize,
    dst_h: usize,
) -> (usize, usize, usize, usize) {
    let src_aspect = src_w as f64 / src_h as f64;
    let dst_aspect = dst_w as f64 / dst_h as f64;

    let (new_w, new_h) = if src_aspect > dst_aspect {
        let new_h = (dst_w as f64 / src_aspect).round() as usize;
        (dst_w, new_h)
    } else {
        let new_w = (dst_h as f64 * src_aspect).round() as usize;
        (new_w, dst_h)
    };

    let left = (dst_w - new_w) / 2;
    let top = (dst_h - new_h) / 2;

    (left, top, new_w, new_h)
}

// =============================================================================
// Source Buffer Initialization
// =============================================================================

/// Initialize a source DMA buffer with uniform data appropriate for the format.
pub fn init_source_buffer(buf: &DmaBuffer, width: usize, height: usize, fmt: u32) {
    buf.write_with(|data| match fmt {
        f if f == SRC_FMT_NV12 => {
            let y_size = width * height;
            data[..y_size].fill(128); // Y plane: neutral gray
            data[y_size..].fill(128); // UV plane: neutral chroma
        }
        f if f == SRC_FMT_YUYV => {
            // YUYV: [Y0, U, Y1, V] macropixels
            for chunk in data.chunks_exact_mut(4) {
                chunk[0] = 128; // Y0
                chunk[1] = 128; // U
                chunk[2] = 128; // Y1
                chunk[3] = 128; // V
            }
        }
        _ => {
            // RGBA: neutral gray with full alpha
            for chunk in data.chunks_exact_mut(4) {
                chunk[0] = 128; // R
                chunk[1] = 128; // G
                chunk[2] = 128; // B
                chunk[3] = 255; // A
            }
        }
    });
}

/// Create a source surface for the given format, handling NV12 specially.
pub fn create_source_surface(buf: &DmaBuffer, width: usize, height: usize, fmt: u32) -> G2DSurface {
    if fmt == SRC_FMT_NV12 {
        create_nv12_surface(buf, width, height)
    } else {
        create_surface(buf, width, height, fmt)
    }
}
