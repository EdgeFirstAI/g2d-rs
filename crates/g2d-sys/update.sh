#!/bin/sh

# Generate Rust FFI bindings from g2d.h
# --dynamic-loading: Generate libloading wrapper instead of linking
# --allowlist-function: Only generate bindings for g2d_* functions
# --no-layout-tests: Skip layout assertions using offset_of! (MSRV 1.75 compat)
bindgen --dynamic-loading g2d \
    --allowlist-function 'g2d_.*' \
    --no-layout-tests \
    g2d.h > src/ffi.rs