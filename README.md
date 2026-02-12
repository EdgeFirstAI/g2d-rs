# g2d-rs

[![Crates.io](https://img.shields.io/crates/v/g2d-sys.svg)](https://crates.io/crates/g2d-sys)
[![Documentation](https://docs.rs/g2d-sys/badge.svg)](https://docs.rs/g2d-sys)
[![License](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE)
[![CI](https://github.com/EdgeFirstAI/g2d-rs/actions/workflows/test.yml/badge.svg)](https://github.com/EdgeFirstAI/g2d-rs/actions/workflows/test.yml)
[![MSRV](https://img.shields.io/badge/MSRV-1.75-blue.svg)](https://blog.rust-lang.org/2023/12/28/Rust-1.75.0.html)

**Rust bindings for NXP i.MX G2D 2D graphics accelerator.**

This repository provides Rust bindings to `libg2d.so` for hardware-accelerated 2D graphics operations on NXP i.MX platforms.

## Crates

| Crate | Description |
|-------|-------------|
| [`g2d-sys`](crates/g2d-sys/) | Low-level unsafe FFI bindings with dynamic loading |

## Requirements

- **Rust 1.75+** (MSRV - Minimum Supported Rust Version)
- NXP i.MX8/i.MX9 platform with G2D support
- `libg2d.so.2` installed (typically at `/usr/lib/libg2d.so.2`)
- Linux only (G2D is not available on other platforms)

## Features

The G2D library provides hardware-accelerated:

- **Blitting** - Fast memory-to-memory copies with format conversion
- **Scaling** - High-quality image resize
- **Rotation** - 0/90/180/270 degree rotation and horizontal/vertical flip
- **Color space conversion** - YUV ↔ RGB (BT.601/BT.709)
- **Alpha blending** - Porter-Duff compositing operations
- **Clear** - Fast rectangle fills with solid color

## Supported Formats

| Format | Description |
|--------|-------------|
| `G2D_RGBA8888` | 32-bit RGBA |
| `G2D_RGBX8888` | 32-bit RGBx (alpha ignored) |
| `G2D_RGB888` | 24-bit RGB |
| `G2D_RGB565` | 16-bit RGB |
| `G2D_NV12` | YUV 4:2:0 semi-planar |
| `G2D_NV16` | YUV 4:2:2 semi-planar |
| `G2D_YUYV` | YUV 4:2:2 packed |
| `G2D_I420` | YUV 4:2:0 planar |

## Usage

Add `g2d-sys` to your `Cargo.toml`:

```toml
[dependencies]
g2d-sys = "1.1"
```

### Basic Example

```rust
use g2d_sys::{G2D, G2DSurface, G2DFormat, G2DPhysical};

fn main() -> g2d_sys::Result<()> {
    // Open G2D device with dynamic library loading
    let g2d = G2D::new("/usr/lib/libg2d.so.2")?;
    
    println!("G2D version: {}", g2d.version());
    
    // Configure source and destination surfaces...
    // (requires DMA buffer file descriptors)
    
    Ok(())
}
```

### Library Scope

`g2d-sys` is a low-level FFI crate that maps `libg2d.so` to Rust through
`dlopen`-based dynamic loading and ABI version detection. It does not provide
safe Rust abstractions, cache management, or buffer lifecycle management.
A high-level safe Rust API (`g2d` crate) is a future consideration.

When using DMA-buf buffers, you are responsible for:

- **Error handling** — all ioctl return values must be checked
- **DMA-buf cache coherency** — including DRM PRIME attachment for cached heaps
  (see [ARCHITECTURE.md](ARCHITECTURE.md#cpu-cache-coherency))
- **Buffer allocation and lifetime management**

See [`hardware_tests.rs`](crates/g2d-sys/tests/hardware_tests.rs) for a
working example of correct DMA-buf usage with both cached and uncached heaps.

### Library Loading

The bindings use dynamic loading via `libloading`. The library path must be specified when creating a `G2D` instance:

```rust
// Standard path on i.MX8/i.MX9 platforms
let g2d = G2D::new("/usr/lib/libg2d.so.2")?;

// Or use an environment variable
let path = std::env::var("LIBG2D_PATH").unwrap_or("/usr/lib/libg2d.so.2".into());
let g2d = G2D::new(path)?;
```

## Platform Support

| Platform | Status |
|----------|--------|
| NXP i.MX 8M Plus | ✅ Tested |
| NXP i.MX 95 | ✅ Tested |
| Other i.MX 8/9 variants | ⚠️ Should work (untested) |
| Other Linux | ❌ No G2D hardware |
| macOS/Windows | ❌ Linux only |

**Note:** G2D is a portable library across i.MX platforms. Other i.MX variants should work but are not currently tested. Bug reports for untested platforms are welcome.

## ABI Compatibility

The G2D library has undergone ABI changes across different i.MX BSP versions. This crate handles compatibility by:

1. **Version detection** - Parsing `_G2D_VERSION` symbol from the library
2. **Structure adaptation** - Using `G2DSurface` (modern) or `G2DSurfaceLegacy` (older) based on version
3. **Runtime switching** - Automatically selecting the correct structure layout

See [ARCHITECTURE.md](ARCHITECTURE.md) for details on ABI handling.

## Related Projects

- [edgefirst-hal](https://github.com/EdgeFirstAI/hal) - Hardware abstraction layer using g2d-sys for image processing

## Contributing

See [CONTRIBUTING.md](CONTRIBUTING.md) for development setup and guidelines.

## License

Licensed under the Apache License 2.0. See [LICENSE](LICENSE) for details.

The G2D API header (`g2d.h`) is provided by NXP under their license terms.
