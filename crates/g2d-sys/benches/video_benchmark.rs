// SPDX-FileCopyrightText: Copyright 2025 Au-Zone Technologies
// SPDX-License-Identifier: Apache-2.0

//! Criterion benchmarks for G2D video pipeline operations.
//!
//! Measures real-world video processing performance including format conversion,
//! scaling, and letterbox operations across production resolutions and formats.
//!
//! ## Run on target (cross-compiled)
//! ```bash
//! ./video_benchmark --bench
//! ```
//!
//! ## Machine-readable output
//! ```bash
//! ./video_benchmark --bench --output-format bencher
//! ```

#![cfg(target_os = "linux")]

mod common;

use common::{
    calculate_letterbox, create_source_surface, create_surface, g2d_available, init_source_buffer,
    BenchConfig, DmaBuffer, HeapType, DST_FMT_RGBA, SRC_FMT_NV12, SRC_FMT_RGBA, SRC_FMT_YUYV,
};
use criterion::{black_box, criterion_group, criterion_main, BenchmarkId, Criterion};
use g2d_sys::G2D;

// =============================================================================
// Resolution and format matrices
// =============================================================================

const RESOLUTIONS: &[(usize, usize)] = &[
    (640, 480),
    (1024, 768),
    (1280, 720),
    (1920, 1080),
    (2592, 1944),
    (3840, 2160),
];

const YUV_FORMATS: &[u32] = &[SRC_FMT_NV12, SRC_FMT_YUYV];
const ALL_FORMATS: &[u32] = &[SRC_FMT_NV12, SRC_FMT_YUYV, SRC_FMT_RGBA];

// =============================================================================
// Convert Benchmarks — format conversion at same resolution
// =============================================================================

fn bench_convert(c: &mut Criterion) {
    if !g2d_available() {
        eprintln!("G2D not available, skipping convert benchmarks");
        return;
    }

    let mut group = c.benchmark_group("convert");
    group.sample_size(10);

    for &(width, height) in RESOLUTIONS {
        for &fmt in YUV_FORMATS {
            let config = BenchConfig::new(width, height, width, height, fmt, DST_FMT_RGBA);

            for heap_type in [HeapType::Uncached, HeapType::Cached] {
                if !heap_type.is_available() {
                    continue;
                }

                let src_size = config.src_buf_size();
                let dst_size = config.dst_buf_size();

                let src_buf = match DmaBuffer::new(heap_type, src_size) {
                    Ok(b) => b,
                    Err(e) => {
                        eprintln!(
                            "Skipping {}/{}: src alloc failed: {e}",
                            heap_type,
                            config.id()
                        );
                        continue;
                    }
                };
                let dst_buf = match DmaBuffer::new(heap_type, dst_size) {
                    Ok(b) => b,
                    Err(e) => {
                        eprintln!(
                            "Skipping {}/{}: dst alloc failed: {e}",
                            heap_type,
                            config.id()
                        );
                        continue;
                    }
                };

                init_source_buffer(&src_buf, width, height, fmt);

                let mut g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");
                g2d.set_bt709_colorspace()
                    .expect("Failed to set colorspace");

                let src_surface = create_source_surface(&src_buf, width, height, fmt);
                let dst_surface = create_surface(&dst_buf, width, height, DST_FMT_RGBA);

                group.throughput(config.throughput());
                group.bench_with_input(
                    BenchmarkId::new(heap_type.name(), config.id()),
                    &config,
                    |b, _| {
                        b.iter(|| {
                            g2d.blit(&src_surface, &dst_surface).expect("blit failed");
                            g2d.finish().expect("finish failed");
                            black_box(&dst_buf);
                        });
                    },
                );
            }
        }
    }

    group.finish();
}

// =============================================================================
// Resize Benchmarks — scale + convert to 640x480 RGBA
// =============================================================================

fn bench_resize(c: &mut Criterion) {
    if !g2d_available() {
        eprintln!("G2D not available, skipping resize benchmarks");
        return;
    }

    let mut group = c.benchmark_group("resize");
    group.sample_size(10);

    let dst_w = 640;
    let dst_h = 480;

    for &(src_w, src_h) in RESOLUTIONS {
        for &fmt in ALL_FORMATS {
            let config = BenchConfig::new(src_w, src_h, dst_w, dst_h, fmt, DST_FMT_RGBA);

            for heap_type in [HeapType::Uncached, HeapType::Cached] {
                if !heap_type.is_available() {
                    continue;
                }

                let src_size = config.src_buf_size();
                let dst_size = config.dst_buf_size();

                let src_buf = match DmaBuffer::new(heap_type, src_size) {
                    Ok(b) => b,
                    Err(e) => {
                        eprintln!(
                            "Skipping {}/{}: src alloc failed: {e}",
                            heap_type,
                            config.id()
                        );
                        continue;
                    }
                };
                let dst_buf = match DmaBuffer::new(heap_type, dst_size) {
                    Ok(b) => b,
                    Err(e) => {
                        eprintln!(
                            "Skipping {}/{}: dst alloc failed: {e}",
                            heap_type,
                            config.id()
                        );
                        continue;
                    }
                };

                init_source_buffer(&src_buf, src_w, src_h, fmt);

                let mut g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");
                if fmt != SRC_FMT_RGBA {
                    g2d.set_bt709_colorspace()
                        .expect("Failed to set colorspace");
                }

                let src_surface = create_source_surface(&src_buf, src_w, src_h, fmt);
                let dst_surface = create_surface(&dst_buf, dst_w, dst_h, DST_FMT_RGBA);

                group.throughput(config.throughput());
                group.bench_with_input(
                    BenchmarkId::new(heap_type.name(), config.id()),
                    &config,
                    |b, _| {
                        b.iter(|| {
                            g2d.blit(&src_surface, &dst_surface).expect("blit failed");
                            g2d.finish().expect("finish failed");
                            black_box(&dst_buf);
                        });
                    },
                );
            }
        }
    }

    group.finish();
}

// =============================================================================
// Letterbox Benchmarks — aspect-preserving resize with gray border
// =============================================================================

fn bench_letterbox(c: &mut Criterion) {
    if !g2d_available() {
        eprintln!("G2D not available, skipping letterbox benchmarks");
        return;
    }

    let mut group = c.benchmark_group("letterbox");
    group.sample_size(10);

    // Gray color for letterbox border (YOLO convention)
    let gray = [114u8, 114, 114, 255];

    let dst_sizes: &[(usize, usize)] = &[(640, 480), (640, 640)];

    for &(src_w, src_h) in RESOLUTIONS {
        for &fmt in ALL_FORMATS {
            for &(dst_w, dst_h) in dst_sizes {
                let config = BenchConfig::new(src_w, src_h, dst_w, dst_h, fmt, DST_FMT_RGBA);

                let (left, top, new_w, new_h) = calculate_letterbox(src_w, src_h, dst_w, dst_h);

                for heap_type in [HeapType::Uncached, HeapType::Cached] {
                    if !heap_type.is_available() {
                        continue;
                    }

                    let src_size = config.src_buf_size();
                    let dst_size = config.dst_buf_size();

                    let src_buf = match DmaBuffer::new(heap_type, src_size) {
                        Ok(b) => b,
                        Err(e) => {
                            eprintln!(
                                "Skipping {}/{}: src alloc failed: {e}",
                                heap_type,
                                config.id()
                            );
                            continue;
                        }
                    };
                    let dst_buf = match DmaBuffer::new(heap_type, dst_size) {
                        Ok(b) => b,
                        Err(e) => {
                            eprintln!(
                                "Skipping {}/{}: dst alloc failed: {e}",
                                heap_type,
                                config.id()
                            );
                            continue;
                        }
                    };

                    init_source_buffer(&src_buf, src_w, src_h, fmt);

                    let g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");

                    let src_surface = create_source_surface(&src_buf, src_w, src_h, fmt);

                    // Full destination surface for clear
                    let mut dst_clear = create_surface(&dst_buf, dst_w, dst_h, DST_FMT_RGBA);

                    // Sub-region destination surface for blit (letterbox content area)
                    let dst_blit = {
                        let mut s = create_surface(&dst_buf, dst_w, dst_h, DST_FMT_RGBA);
                        s.left = left as i32;
                        s.top = top as i32;
                        s.right = (left + new_w) as i32;
                        s.bottom = (top + new_h) as i32;
                        s
                    };

                    // Set colorspace for YUV formats (must be done per-G2D instance)
                    let g2d = if fmt != SRC_FMT_RGBA {
                        let mut g = g2d;
                        g.set_bt709_colorspace().expect("Failed to set colorspace");
                        g
                    } else {
                        g2d
                    };

                    group.throughput(config.throughput());
                    group.bench_with_input(
                        BenchmarkId::new(heap_type.name(), config.id()),
                        &config,
                        |b, _| {
                            b.iter(|| {
                                g2d.clear(&mut dst_clear, gray).expect("clear failed");
                                g2d.blit(&src_surface, &dst_blit).expect("blit failed");
                                g2d.finish().expect("finish failed");
                                black_box(&dst_buf);
                            });
                        },
                    );
                }
            }
        }
    }

    group.finish();
}

// =============================================================================
// Partial Clear Benchmarks — G2D sub-region clear vs CPU memset for letterbox
// =============================================================================

/// Compare G2D partial clear vs CPU fill for letterbox-style bar clearing.
///
/// Simulates the clear phase of a letterbox resize pipeline where only the
/// border bars (top/bottom or left/right) need to be filled with a solid color.
/// Tests at the realistic 1920×1080 → 640×640 ratio (~44% border area with
/// 140px top and bottom bars).
fn bench_partial_clear(c: &mut Criterion) {
    if !g2d_available() {
        eprintln!("G2D not available, skipping partial clear benchmarks");
        return;
    }

    let mut group = c.benchmark_group("partial_clear");
    group.sample_size(200);

    let gray = [114u8, 114, 114, 255];

    // Letterbox configurations: (dst_w, dst_h, bar_size, orientation)
    // 1920×1080 → 640×640: content 640×360, bars 140px top + 140px bottom
    struct LetterboxConfig {
        name: &'static str,
        dst_w: usize,
        dst_h: usize,
        bars: Vec<(i32, i32, i32, i32)>, // (left, top, right, bottom) for each bar
    }

    let configs = [
        LetterboxConfig {
            name: "640x640/top+bottom/140px",
            dst_w: 640,
            dst_h: 640,
            bars: vec![
                (0, 0, 640, 140),   // top bar
                (0, 500, 640, 640), // bottom bar
            ],
        },
        LetterboxConfig {
            name: "640x640/left+right/140px",
            dst_w: 640,
            dst_h: 640,
            bars: vec![
                (0, 0, 140, 640),   // left bar
                (500, 0, 640, 640), // right bar
            ],
        },
        LetterboxConfig {
            name: "640x640/top+bottom/32px",
            dst_w: 640,
            dst_h: 640,
            bars: vec![
                (0, 0, 640, 32),    // top bar (small)
                (0, 608, 640, 640), // bottom bar (small)
            ],
        },
    ];

    for config in &configs {
        let bpp = 4usize;
        let dst_size = config.dst_w * config.dst_h * bpp;

        // CPU partial fill: write only the bar regions via mmap
        {
            let buf = match DmaBuffer::new(HeapType::Cached, dst_size) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("Skipping CPU {}: alloc failed: {e}", config.name);
                    continue;
                }
            };

            let bars = config.bars.clone();
            let w = config.dst_w;

            group.bench_function(BenchmarkId::new("cpu", config.name), |b| {
                b.iter(|| {
                    buf.write_with(|data| {
                        for &(left, top, right, bottom) in &bars {
                            for row in top..bottom {
                                let row_start = (row as usize * w + left as usize) * bpp;
                                let row_end = (row as usize * w + right as usize) * bpp;
                                for chunk in data[row_start..row_end].chunks_exact_mut(4) {
                                    chunk.copy_from_slice(&gray);
                                }
                            }
                        }
                    });
                    black_box(&buf);
                });
            });
        }

        // G2D partial clear: clear each bar sub-region separately
        for heap_type in [HeapType::Uncached, HeapType::Cached] {
            if !heap_type.is_available() {
                continue;
            }

            let buf = match DmaBuffer::new(heap_type, dst_size) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!(
                        "Skipping g2d/{} {}: alloc failed: {e}",
                        heap_type, config.name
                    );
                    continue;
                }
            };

            let g2d = G2D::new("libg2d.so.2").expect("Failed to open G2D");

            let bars: Vec<_> = config
                .bars
                .iter()
                .map(|&(left, top, right, bottom)| {
                    let mut s = create_surface(&buf, config.dst_w, config.dst_h, DST_FMT_RGBA);
                    s.left = left;
                    s.top = top;
                    s.right = right;
                    s.bottom = bottom;
                    s
                })
                .collect();

            let g2d_id = format!("g2d/{}", heap_type.name());
            group.bench_function(BenchmarkId::new(&g2d_id, config.name), |b| {
                let mut bar_surfaces = bars.clone();
                b.iter(|| {
                    for surface in &mut bar_surfaces {
                        g2d.clear(surface, gray).expect("clear failed");
                    }
                    g2d.finish().expect("finish failed");
                    black_box(&buf);
                });
            });
        }
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_convert,
    bench_resize,
    bench_letterbox,
    bench_partial_clear
);
criterion_main!(benches);
