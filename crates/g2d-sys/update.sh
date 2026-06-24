#!/bin/sh

# Generate Rust FFI bindings from g2d.h
# --dynamic-loading: Generate libloading wrapper instead of linking
# --allowlist-function: Only generate bindings for g2d_* functions
# --no-layout-tests: Skip layout assertions to keep src/ffi.rs stable and small.
#   (No longer required by MSRV — offset_of! has been stable since 1.77 and the
#   MSRV is now 1.88 — so layout tests may be re-enabled by dropping this flag.)
#
# Post-generation patch required for libloading >= 0.9:
#   bindgen (as of 0.72.1) emits `Library::new(path)`, but libloading 0.9
#   replaced the `AsRef<OsStr>` bound with the sealed `AsFilename` trait.
#   After regenerating, change that call to `Library::new(path.as_ref())` so
#   the public `g2d::new<P: AsRef<OsStr>>` signature keeps compiling unchanged.
bindgen --dynamic-loading g2d \
    --allowlist-function 'g2d_.*' \
    --no-layout-tests \
    g2d.h > src/ffi.rs