# Testing

## Overview

g2d-rs tests require NXP i.MX hardware with the G2D library installed. Tests are
cross-compiled on the host and executed on target hardware via SSH.

Tests are parameterized by DMA heap type (uncached and cached) to validate
CPU cache coherency with `DMA_BUF_IOCTL_SYNC` and isolate heap-specific behavior.

**Tested platforms:**
- i.MX 8M Plus (MCIMX8M-PLUS)
- i.MX 95 (MCIMX95-EVK)

Other i.MX platforms with G2D support should work but are not currently tested.

## Prerequisites

- `cargo-zigbuild` for cross-compilation: `cargo install cargo-zigbuild`
- `zig` compiler: available from [ziglang.org](https://ziglang.org/download/)
- SSH access to target hardware
- Target must have `libg2d.so.2` installed
- Target must have `/dev/dma_heap/` available with `linux,cma-uncached` heap
  (preferred) and/or `linux,cma` heap
- Target must have `/dev/dri/renderD128` accessible for DRM PRIME import
  (required for cached heap cache coherency)

## Manual On-Target Testing

### Cross-Compile

```bash
cargo zigbuild --target aarch64-unknown-linux-gnu --tests
```

The test binary is located at:
```
target/aarch64-unknown-linux-gnu/debug/deps/hardware_tests-<hash>
```

### Deploy and Run

```bash
BINARY=$(find target/aarch64-unknown-linux-gnu/debug/deps -name 'hardware_tests-*' -executable | head -1)
scp "$BINARY" <target>:/tmp/hardware_tests
ssh <target> "/tmp/hardware_tests --test-threads=1 --nocapture"
```

**Important:** Use `--test-threads=1` to avoid concurrent G2D handle contention.

### Running Specific Tests

```bash
# Run only uncached heap tests
ssh <target> "/tmp/hardware_tests --test-threads=1 --nocapture uncached"

# Run only cached heap tests
ssh <target> "/tmp/hardware_tests --test-threads=1 --nocapture cached"

# Run only stress tests
ssh <target> "/tmp/hardware_tests --test-threads=1 --nocapture stress"

# Run only correctness tests
ssh <target> "/tmp/hardware_tests --test-threads=1 --nocapture double_write\|multi_read\|roundtrip\|color_cycle"

# Run a single named test
ssh <target> "/tmp/hardware_tests --test-threads=1 --nocapture test_g2d_clear_rgba_cached"
```

## Test Categories

Tests that use DMA-buf buffers are run in both `_uncached` and `_cached` variants
using the `heap_tests!` macro. Tests skip automatically when the required heap is
not available.

### Initialization Tests
- `test_g2d_open_close` — Verify G2D library can be loaded and handle opened
- `test_g2d_version_detection` — Verify version string is detected and parsed
- `test_g2d_invalid_library_path` — Verify graceful failure with invalid path

### Heap Availability
- `test_heap_availability` — Report which DMA heaps are available on the target

### DMA Buffer Tests
- `test_g2d_physical_address_{uncached,cached}` — Verify physical address
  resolution via ioctl on each heap type

### Clear Tests (DMA-buf buffers, uncached + cached)
- `test_g2d_clear_rgba_{uncached,cached}` — Clear a DMA-buf surface with a
  single RGBA color
- `test_g2d_clear_multiple_colors_{uncached,cached}` — Clear same buffer with 6
  colors sequentially
- `test_g2d_clear_large_surface_{uncached,cached}` — Clear a 1920x1080 surface

### Blit Tests (DMA-buf buffers, uncached + cached)
- `test_g2d_blit_rgba_to_rgba_{uncached,cached}` — Blit between same-format
  DMA-buf surfaces
- `test_g2d_blit_rgba_to_rgb_{uncached,cached}` — RGBA to RGB565 format
  conversion
- `test_g2d_blit_with_scaling_{uncached,cached}` — Blit with resolution scaling

### YUV Format Tests (uncached + cached)
- `test_g2d_blit_yuyv_to_rgba_{uncached,cached}` — YUYV to RGBA conversion
- `test_g2d_blit_nv12_to_rgba_{uncached,cached}` — NV12 to RGBA conversion

### Cache Coherency Correctness Tests
- `test_double_write_overwrite_{uncached,cached}` — GPU fills with color A, CPU
  reads, GPU fills with color B, CPU reads. Verifies no stale data from first
  fill. **Most critical test for cache coherency.**
- `test_multi_read_consistency_{uncached,cached}` — After a single GPU write,
  multiple CPU reads all return the same data.
- `test_cpu_gpu_roundtrip_{uncached,cached}` — CPU writes pattern to source,
  GPU blits to destination, CPU reads destination and verifies every pixel.
- `test_sequential_color_cycle_{uncached,cached}` — Fills same buffer with 6
  colors sequentially, verifying every pixel after each fill.

### Stress Tests
- `test_stress_clear_100_{uncached,cached}` — 100 sequential clear+readback
  cycles with different colors.
- `test_stress_blit_100_{uncached,cached}` — 100 sequential blit+readback cycles
  with unique patterns.

### Pixel Format Tests
- `test_g2d_format_conversion` — Verify RGBA, BGRA, ARGB, ABGR byte layouts
- `test_g2d_format_invalid` — Verify graceful handling of invalid formats
- `test_g2d_colorspace_configuration` — Verify colorspace setting on surfaces

## Benchmarks

Benchmarks use [Criterion](https://docs.rs/criterion) for statistically rigorous
measurement of G2D video pipeline operations. They are separate from tests and
run as a dedicated `[[bench]]` target.

### Benchmark Groups

- **convert** — Format conversion at same resolution (NV12/YUYV → RGBA)
- **resize** — Scale + convert to 640x480 RGBA destination
- **letterbox** — Aspect-preserving resize with gray border to 640x480 and 640x640

Each benchmark is run on both uncached and cached DMA heaps across 6 source
resolutions (640x480 through 3840x2160) and up to 3 source formats (NV12, YUYV, RGBA).

### Manual On-Target Benchmarks

#### Cross-Compile

```bash
cargo zigbuild --target aarch64-unknown-linux-gnu --benches
```

The benchmark binary is at:
```
target/aarch64-unknown-linux-gnu/release/deps/video_benchmark-<hash>
```

#### Deploy and Run

```bash
BINARY=$(find target/aarch64-unknown-linux-gnu/release/deps -name 'video_benchmark-*' -executable ! -name '*.d' | head -1)
scp "$BINARY" <target>:/tmp/video_benchmark
ssh <target> "/tmp/video_benchmark --bench"
```

#### Common Options

```bash
# Full benchmark run with statistical analysis
./video_benchmark --bench

# Machine-readable output (for CI)
./video_benchmark --bench --output-format bencher

# Run specific group only
./video_benchmark --bench convert
./video_benchmark --bench resize
./video_benchmark --bench letterbox

# Save baseline for comparison
./video_benchmark --bench --save-baseline my-baseline
```

#### On Host (if G2D hardware available)

```bash
make bench
# or
cargo bench -p g2d-sys --bench video_benchmark
```

## CI Integration

### Automated Tests (`test.yml`)

Tests run automatically on every push to `main`/`develop` and on every pull
request. The workflow has 4 jobs:

1. **Build & Lint** (`ubuntu-22.04-arm`) — Formatting, clippy, docs, and
   builds test binaries with coverage instrumentation
2. **Hardware Test** (`nxp-imx8mp-latest`) — Downloads pre-built test binaries
   and runs all tests with `--test-threads=1` on NXP i.MX 8M Plus EVK hardware
3. **Process Coverage** (`ubuntu-22.04-arm`) — Merges profraw files from
   hardware test run and generates LCOV coverage report
4. **SonarCloud Analysis** (`ubuntu-22.04-arm`) — Uploads coverage and runs
   static analysis

Tests produce coverage artifacts (profraw, LCOV) and test result artifacts
retained for 30 days.

### On-Demand Benchmarks (`bench.yml`)

Benchmarks are triggered manually via the **Benchmark** workflow
(Actions → Benchmark → Run workflow). The workflow has 3 phases:

1. **Build Benchmarks** (`ubuntu-22.04-arm`) — Builds criterion benchmark
   binary in release mode
2. **Run on i.MX 8M Plus** (`nxp-imx8mp-latest`) — Executes benchmarks with
   `--save-baseline github-ci --output-format bencher`, producing both Criterion
   JSON data (in `target/criterion/`) and bencher text output
3. **Process Results** (`ubuntu-22.04-arm`) — Extracts Criterion JSON, generates
   a markdown summary with tables and
   [QuickChart.io](https://quickchart.io/) bar charts for the GitHub Actions
   step summary, and stores trend data via
   [github-action-benchmark](https://github.com/benchmark-action/github-action-benchmark)

Benchmark results are tracked over time with interactive charts published to
GitHub Pages at `dev/bench/`. A 150% alert threshold triggers comments on
regression.

Benchmark result artifacts (including raw Criterion JSON) are retained for 90
days.

## DMA Buffer Implementation

Tests and benchmarks use a `DmaBuffer` struct with persistent mmap and proper
`DMA_BUF_IOCTL_SYNC` protocol:

1. Buffer is `mmap`'d once on creation (persistent mapping)
2. For cached heaps, the DMA-buf fd is imported through the GPU DRM driver
   (`DRM_IOCTL_PRIME_FD_TO_HANDLE` on `/dev/dri/renderD128`) to create a
   persistent `dma_buf_attach` — required for `DMA_BUF_IOCTL_SYNC` to
   actually perform cache maintenance (see below)
3. CPU reads are bracketed by `SYNC_START(READ)` / `SYNC_END(READ)`
4. CPU writes are bracketed by `SYNC_START(WRITE)` / `SYNC_END(WRITE)`
5. All ioctl and mmap/munmap return values are checked — silent failures are
   never tolerated
6. GEM handle is closed on drop (detaches DMA-buf), then buffer is `munmap`'d

This follows the correct Linux DMA-buf CPU access protocol and works reliably
on both cached and uncached heaps.

### Why DRM PRIME Import is Required for Cached Heaps

The kernel's CMA heap `begin_cpu_access` callback (called by
`DMA_BUF_IOCTL_SYNC`) iterates over `buffer->attachments` to perform cache
maintenance via `dma_sync_sgtable_for_cpu()`. **Without any active device
attachments, sync is a complete no-op** — no cache invalidation or flush occurs.

The `DMA_BUF_IOCTL_PHYS` ioctl only creates a temporary attachment to resolve
the physical address, then immediately detaches. After it returns, no
attachments remain. This is standard DMA-buf behavior — the ioctl resolves an
address, it does not establish a persistent import.

By importing the DMA-buf fd through the GPU DRM driver
(`DRM_IOCTL_PRIME_FD_TO_HANDLE`), a persistent `dma_buf_attach` is created.
This attachment stays alive as long as the GEM handle is open, giving
`DMA_BUF_IOCTL_SYNC` an attachment to iterate for cache operations.

This was verified empirically: without the DRM attachment, double-fill tests
show 29% stale pixels on cached CMA; with the attachment, 0% stale pixels
across all test categories and stress tests. See
[ARCHITECTURE.md](ARCHITECTURE.md#cpu-cache-coherency) for the complete
protocol.
