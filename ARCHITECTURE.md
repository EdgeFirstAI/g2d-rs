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
- Runtime detection of G2D availability
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

## Crate Structure

```
crates/
└── g2d-sys/          # Low-level FFI bindings
    ├── src/
    │   ├── lib.rs    # Public API, G2D wrapper, version detection
    │   └── ffi.rs    # Raw bindgen-generated FFI types
    └── g2d.h         # NXP G2D header (for bindgen)
```

## Future: Safe API

A future `g2d` crate may provide:
- Safe Rust wrappers with lifetime management
- Builder pattern for surface configuration
- Integration with standard Rust image types
