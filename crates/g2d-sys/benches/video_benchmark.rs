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

criterion_group!(benches, bench_convert, bench_resize, bench_letterbox);
criterion_main!(benches);
