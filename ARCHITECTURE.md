# g2d-rs Architecture

## Overview

g2d-rs provides Rust bindings for the NXP i.MX G2D 2D graphics accelerator library (`libg2d.so`). The library is loaded dynamically at runtime using `dlopen` via the `libloading` crate.

## Dynamic Loading

The G2D library is **not linked at compile time**. Instead, it is loaded at runtime:

```rust
let lib = libloading::Library::new("/usr/lib/libg2d.so.2")?;
```

This approach allows:
- Compilation on systems without G2D installed
- Runtime detection of G2D availability and version
- Graceful fallback when G2D is not present

## ABI Compatibility

The NXP G2D library has undergone **breaking ABI changes** across BSP versions. The key difference is the `g2d_surface` structure:

| BSP Version | `planes` field type | Size (bytes) |
|-------------|---------------------|--------------|
| < 6.4.11 (legacy) | `c_int[3]` | 60 |
| ≥ 6.4.11 (modern) | `c_ulong[3]` | 72 (on 64-bit) |

### Version Detection

The library version is detected by reading the `_G2D_VERSION` symbol:

```rust
let version_ptr = lib.get::<*const c_char>(b"_G2D_VERSION")?;
// Parses: "$VERSION$6.4.11:1049711:abc123$"
```

### Runtime Adaptation

Based on the detected version, the crate uses either:
- `G2DSurface` - Modern structure with `c_ulong` planes
- `G2DSurfaceLegacy` - Legacy structure with `c_int` planes

The `G2D::blit()` and `G2D::clear()` methods automatically select the correct structure.

## Pixel Format Convention

G2D format names describe **bit positions** within the 32-bit pixel word, ordered
from LSB to MSB:

| Format | Bits [0:7] | Bits [8:15] | Bits [16:23] | Bits [24:31] | Integer |
|--------|-----------|-------------|-------------|-------------|---------|
| `RGBA8888` | R | G | B | A | `0xAABBGGRR` |
| `BGRA8888` | B | G | R | A | `0xAARRGGBB` |
| `ARGB8888` | A | R | G | B | `0xBBGGRRAA` |
| `ABGR8888` | A | B | G | R | `0xRRGGBBAA` |

On little-endian ARM, memory byte order matches the format name (left to right).
For `RGBA8888`, memory bytes at ascending addresses are `[R, G, B, A]`.

### clrcolor Packing

The `clrcolor` field in `g2d_surface` is documented as "32-bit RGBA", meaning it
uses `RGBA8888` layout: `0xAABBGGRR` as an integer. The crate packs it using
`i32::from_le_bytes([R, G, B, A])`.

## Buffer Allocation

### g2d_alloc vs DMA-buf

The G2D library provides two buffer allocation paths:

- **`g2d_alloc()`** — GPU-managed contiguous memory.
- **DMA-buf heap** (`/dev/dma_heap/`) — Linux kernel DMA-buf allocator. Physical
  addresses are obtained via a vendor-specific `DMA_BUF_IOCTL_PHYS` ioctl.

G2D operations (`g2d_blit`, `g2d_clear`) work with both allocation methods.
When using DMA-buf buffers on cached CMA heaps, the complete cache coherency
protocol (DRM PRIME import + `DMA_BUF_IOCTL_SYNC`) must be followed for
correct operation.

### CPU Cache Coherency

DMA-buf memory allocated from the CMA heap (`linux,cma`) is mapped with CPU
caching enabled. When the GPU writes to these buffers via DMA and the CPU
subsequently reads via `mmap`, the CPU may see stale cached data unless the
correct cache coherency protocol is followed.

The kernel's CMA heap implements `begin_cpu_access` (called by
`DMA_BUF_IOCTL_SYNC`) by iterating over `buffer->attachments` and calling
`dma_sync_sgtable_for_cpu()` on each. **If no device has attached to the
DMA-buf, the sync loop has nothing to iterate and `DMA_BUF_IOCTL_SYNC` is a
complete no-op** — no cache invalidation or flush occurs.

The `DMA_BUF_IOCTL_PHYS` ioctl creates only a *temporary* attachment to
resolve the physical address, then immediately detaches. After it returns,
the attachment list is empty. This is by design — it is an address-resolution
ioctl, not a persistent import.

The complete protocol for correct DMA-buf cache coherency on cached CMA heaps:

1. **DRM PRIME import** — import the DMA-buf fd through the GPU DRM driver to
   create a persistent `dma_buf_attach`. Without this, all subsequent sync
   calls are no-ops on cached heaps.

   ```rust
   let drm_fd = open("/dev/dri/renderD128", O_RDWR);
   ioctl(drm_fd, DRM_IOCTL_PRIME_FD_TO_HANDLE, &prime);  // creates dma_buf_attach
   // Keep drm_fd and GEM handle alive for the buffer's lifetime
   // On cleanup: close GEM handle (DRM_IOCTL_GEM_CLOSE), then close drm_fd
   ```

2. **Persistent mmap** — map the buffer once and keep the mapping for its
   lifetime. Do NOT mmap/munmap per-access, as this can orphan CPU cache lines
   and interfere with the kernel's cache maintenance.

3. **SYNC_START before CPU access** — `DMA_BUF_IOCTL_SYNC` with `SYNC_START`
   invalidates CPU caches (for reads) or prepares for writes.

4. **CPU reads or writes** via the persistent mapping.

5. **SYNC_END after CPU access** — `DMA_BUF_IOCTL_SYNC` with `SYNC_END`
   flushes CPU caches (for writes) or completes the access.

6. **Always check ioctl return values** — silent sync failures lead to stale
   data with no error indication.

```rust
// Complete example: DRM import + persistent mmap + bracketed sync
let drm_fd = open("/dev/dri/renderD128", O_RDWR);
ioctl(drm_fd, DRM_IOCTL_PRIME_FD_TO_HANDLE, &prime);

let ptr = mmap(fd, size, PROT_READ | PROT_WRITE, MAP_SHARED);

// Before CPU read (after GPU write):
ioctl(fd, DMA_BUF_IOCTL_SYNC, SYNC_START | SYNC_READ);  // invalidate
// ... read from ptr ...
ioctl(fd, DMA_BUF_IOCTL_SYNC, SYNC_END | SYNC_READ);

// Before CPU write (before GPU read):
ioctl(fd, DMA_BUF_IOCTL_SYNC, SYNC_START | SYNC_WRITE);
// ... write to ptr ...
ioctl(fd, DMA_BUF_IOCTL_SYNC, SYNC_END | SYNC_WRITE);   // flush
```

Use the correct direction flags: `SYNC_READ` for CPU reads (triggers cache
invalidation), `SYNC_WRITE` for CPU writes (triggers cache flush). Do not use
`SYNC_READ | SYNC_WRITE` for pure writes — it adds unnecessary invalidation.

#### Heap Types

Two CMA heap types are available on i.MX 8M Plus and i.MX 95:

- **`linux,cma-uncached`** — non-cacheable mapping. GPU writes are immediately
  visible to CPU reads without cache maintenance. `DMA_BUF_IOCTL_SYNC` is
  still called (no-op) for correctness.
- **`linux,cma`** — cached mapping. Higher CPU bandwidth but requires the
  complete cache coherency protocol above (DRM PRIME import +
  `DMA_BUF_IOCTL_SYNC`) for correct operation.

Both heap types are tested comprehensively. The uncached heap avoids cache
coherency complexity at the cost of reduced CPU read/write bandwidth.

## Crate Structure

```
crates/
└── g2d-sys/          # Low-level FFI bindings
    ├── src/
    │   ├── lib.rs    # Public API, G2D wrapper, version detection
    │   └── ffi.rs    # Raw bindgen-generated FFI types
    └── g2d.h         # NXP G2D header (v2.5)
```

## Future: Safe API

A future `g2d` crate may provide:
- Safe Rust wrappers with lifetime management
- Builder pattern for surface configuration
- Integration with standard Rust image types
