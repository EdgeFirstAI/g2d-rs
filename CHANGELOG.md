# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.0.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [1.2.0] - 2026-02-14

### Changed

- **Breaking:** `clear()` and `blit()` no longer call `g2d_finish()` internally.
  Callers must now call `finish()` explicitly after queuing operations. This
  enables batching multiple operations (e.g., clear + blit) with a single
  GPU synchronization barrier, which is the correct usage pattern for the
  G2D hardware.

### Added

- `finish()` method to explicitly wait for all queued G2D operations to complete.
- `flush()` method to submit queued operations for asynchronous GPU execution
  without waiting. Useful for overlapping GPU work with CPU work in pipelines
  where the result is not needed immediately.
- Partial clear tests (`test_g2d_clear_partial_region`, `test_g2d_clear_partial_left_right`)
  verifying sub-region clearing via the surface left/top/right/bottom fields.
- Partial clear benchmarks (`bench_partial_clear`) comparing G2D sub-region
  clear vs CPU memset for letterbox bar configurations (top+bottom, left+right)
  at realistic sizes (640×640 with 140px and 32px bars).
- Comprehensive `g2d_clear` format tests across all 12 supported destination
  formats (RGB565, BGR565, RGBA8888, RGBX8888, BGRA8888, BGRX8888, ARGB8888,
  ABGR8888, XRGB8888, XBGR8888, YUYV, UYVY) with byte-level verification for
  BGRA8888, ARGB8888, and RGB565, plus stale-data detection for all formats.
- Expect-failure tests for unsupported `g2d_clear` destinations (RGB888, BGR888,
  YVYU, VYUY, NV12, NV21, I420, YV12, NV16, NV61) that produce actionable
  guidance when a driver update adds new format support.

## [1.1.0] - 2026-02-12

### Added

- Criterion benchmarks for video pipeline operations (convert, resize,
  letterbox) covering 6 resolutions (640x480 through 3840x2160), 3 source
  formats (NV12, YUYV, RGBA), and 2 heap types (uncached, cached)
- On-demand CI benchmark workflow (`bench.yml`) on NXP i.MX 8M Plus with
  QuickChart.io summary charts and `github-action-benchmark` trend tracking
  on GitHub Pages
- Benchmark summary script (`.github/scripts/benchmark_summary.py`) parsing
  Criterion JSON data with fallback to bencher text output
- `make bench` target for running benchmarks
- Library scope sections in README.md and crates/g2d-sys/README.md clarifying
  user responsibility for cache management and buffer lifecycle
- TESTING.md documenting test infrastructure, DMA buffer implementation,
  on-target test execution, manual benchmark execution, and CI integration
  for both tests and benchmarks
- ARCHITECTURE.md documenting ABI compatibility handling
- GitHub Actions workflows for CI/CD
- SBOM generation and license compliance checking

### Changed

- Moved to standalone repository (previously part of EdgeFirst HAL)
- Changed license from MIT to Apache-2.0 for consistency
- Updated to use workspace version inheritance
- Added comprehensive documentation
- Benchmarks separated from tests into proper `[[bench]]` criterion targets
  with `criterion = { version = "0.5", default-features = false }`
- Reframed DMA-buf cache coherency documentation as standard Linux protocol
  rather than platform-specific workaround
- DRM PRIME import is now step 1 of the cache coherency protocol in
  ARCHITECTURE.md, presented as a required part of correct DMA-buf usage
- All tests now use DMA-buf exclusively (no more `g2d_alloc` test buffers)
- Clear tests are now parameterized by heap type (`_uncached` / `_cached`)
- Release workflow uses OIDC trusted publishing instead of stored token

### Removed

- Hand-rolled `measure()` timing helper and `bench_*` test functions
  (superseded by criterion benchmarks)
- `fill()` method — `g2d_clear()` works directly on DMA-buf buffers with
  proper DRM PRIME attachment; the blit-based workaround is no longer needed
- `G2DAllocBuffer` test infrastructure and `create_g2d_alloc_surface()` helper
- "Known Limitations" section from TESTING.md (g2d_clear/DMA-buf limitation
  was a cache coherency symptom, not a fundamental limitation)

## [1.0.1] - 2025-11-15

### Fixed

- ABI compatibility with older G2D library versions (< 6.4.11)
- Version detection from `_G2D_VERSION` symbol

## [1.0.0] - 2025-10-01

### Added

- Initial release of g2d-sys FFI bindings
- Dynamic loading via libloading
- Support for G2D blit, clear, and format conversion operations
- Version detection and ABI adaptation
- Support for RGB, RGBA, NV12, YUYV formats
