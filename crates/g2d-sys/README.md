# g2d-sys

[![Crates.io](https://img.shields.io/crates/v/g2d-sys.svg)](https://crates.io/crates/g2d-sys)
[![Documentation](https://docs.rs/g2d-sys/badge.svg)](https://docs.rs/g2d-sys)
[![License](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](../LICENSE)

**Low-level FFI bindings for NXP i.MX G2D 2D graphics accelerator.**

This crate provides unsafe bindings to `libg2d.so` for hardware-accelerated 2D graphics operations on NXP i.MX8/i.MX9 platforms.

## Features

- **Dynamic loading** - Library loaded at runtime via `libloading`
- **ABI compatibility** - Handles G2D library version differences
- **Zero dependencies on NXP SDK** - Compiles anywhere, runs on i.MX

## Usage

```rust
use g2d_sys::{G2D, G2DSurface, G2DFormat, G2DPhysical};

fn main() -> g2d_sys::Result<()> {
    let g2d = G2D::new("/usr/lib/libg2d.so.2")?;
    println!("G2D version: {}", g2d.version());
    Ok(())
}
```

## Supported Operations

| Operation | Description |
|-----------|-------------|
| `blit` | Copy with format conversion and scaling |
| `clear` | Fill rectangle with solid color |
| `enable/disable` | Configure colorspace (BT.601/BT.709) |

## Requirements

- NXP i.MX8/i.MX9 platform
- `libg2d.so.2` installed

## License

Apache-2.0
