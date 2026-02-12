#!/usr/bin/env python3
# SPDX-FileCopyrightText: Copyright 2025 Au-Zone Technologies
# SPDX-License-Identifier: Apache-2.0

"""Generate markdown summary with charts from G2D criterion benchmarks.

Parses Criterion JSON data (preferred) or bencher text output and generates
a grouped markdown summary with QuickChart.io bar charts for GitHub Actions
step summary.

Usage:
    # From Criterion JSON (richest output)
    python benchmark_summary.py --criterion-dir criterion --output summary.md

    # From bencher text output (fallback)
    python benchmark_summary.py --bencher-file benchmark-output.txt

    # Both (JSON preferred, bencher fallback)
    python benchmark_summary.py --criterion-dir criterion \
        --bencher-file benchmark-output.txt --output summary.md
"""

import argparse
import glob
import json
import os
import re
import sys
import urllib.parse
from collections import defaultdict


# =============================================================================
# Criterion JSON Parsing
# =============================================================================


def parse_criterion_json(criterion_dir):
    """Parse Criterion JSON files into structured benchmark results.

    Returns list of dicts with keys: group, heap, config, ns, throughput_str.
    """
    results = []

    for bench_json_path in glob.glob(
        f"{criterion_dir}/**/new/benchmark.json", recursive=True
    ):
        estimates_path = os.path.join(os.path.dirname(bench_json_path), "estimates.json")
        if not os.path.exists(estimates_path):
            continue

        try:
            with open(bench_json_path) as f:
                bench_data = json.load(f)
            with open(estimates_path) as f:
                estimates = json.load(f)

            full_id = bench_data.get("full_id", "")
            if not full_id:
                continue

            # Parse full_id: "group/heap/config"
            parts = full_id.split("/", 2)
            if len(parts) < 3:
                continue

            group, heap, config = parts[0], parts[1], parts[2]

            # Get point estimate (slope preferred, then median)
            time_data = estimates.get("slope") or estimates.get("median", {})
            point_estimate = time_data.get("point_estimate")
            if point_estimate is None:
                continue

            ns = float(point_estimate)

            # Calculate throughput from benchmark metadata
            throughput_str = "N/A"
            throughput = bench_data.get("throughput")
            if throughput and ns > 0:
                bytes_val = throughput.get("Bytes")
                if bytes_val:
                    bytes_per_sec = bytes_val * 1e9 / ns
                    if bytes_per_sec >= 1024**3:
                        throughput_str = f"{bytes_per_sec / 1024**3:.2f} GiB/s"
                    elif bytes_per_sec >= 1024**2:
                        throughput_str = f"{bytes_per_sec / 1024**2:.0f} MiB/s"
                    else:
                        throughput_str = f"{bytes_per_sec / 1024:.0f} KiB/s"

            results.append(
                {
                    "group": group,
                    "heap": heap,
                    "config": config,
                    "ns": ns,
                    "throughput_str": throughput_str,
                }
            )
        except Exception as e:
            print(f"Warning: Failed to parse {bench_json_path}: {e}", file=sys.stderr)

    return results


# =============================================================================
# Bencher Text Parsing (fallback)
# =============================================================================


def parse_bencher_output(filepath):
    """Parse criterion --output-format bencher text into structured results."""
    results = []
    pattern = re.compile(
        r"test\s+(\w+)/(\w+)/(.+?)\s+\.\.\.\s+bench:\s+([\d,]+)\s+ns/iter\s+\(\+/-\s+([\d,]+)\)"
    )

    with open(filepath) as f:
        for line in f:
            m = pattern.match(line.strip())
            if m:
                ns = int(m.group(4).replace(",", ""))
                results.append(
                    {
                        "group": m.group(1),
                        "heap": m.group(2),
                        "config": m.group(3),
                        "ns": ns,
                        "throughput_str": "N/A",
                    }
                )

    return results


# =============================================================================
# Formatting Helpers
# =============================================================================


def format_time(ns):
    """Format nanoseconds into human-readable time."""
    if ns >= 1_000_000:
        return f"{ns / 1_000_000:.2f} ms"
    elif ns >= 1_000:
        return f"{ns / 1_000:.1f} us"
    else:
        return f"{ns:.0f} ns"


def format_time_ms(ns):
    """Convert nanoseconds to milliseconds (float)."""
    return round(ns / 1_000_000, 3)


# Group display metadata
GROUP_INFO = {
    "convert": {
        "title": "Format Conversion",
        "desc": "YUV to RGBA conversion at native resolution (no scaling)",
    },
    "resize": {
        "title": "Resize",
        "desc": "Scale + convert to 640x480 RGBA destination",
    },
    "letterbox": {
        "title": "Letterbox",
        "desc": "Aspect-preserving resize with gray border (clear + blit)",
    },
}

# Resolution sort order
RESOLUTION_ORDER = {
    "640x480": 0,
    "1024x768": 1,
    "1280x720": 2,
    "1920x1080": 3,
    "2592x1944": 4,
    "3840x2160": 5,
}


def resolution_sort_key(config):
    """Extract resolution from config string for sorting."""
    m = re.match(r"(\d+x\d+)", config)
    if m:
        return RESOLUTION_ORDER.get(m.group(1), 99)
    return 99


# =============================================================================
# Chart Generation
# =============================================================================


def generate_chart_url(title, labels, datasets, unit="ms"):
    """Generate a QuickChart.io URL for a grouped horizontal bar chart."""
    chart_config = {
        "type": "horizontalBar",
        "data": {"labels": labels, "datasets": datasets},
        "options": {
            "title": {"display": True, "text": f"{title} ({unit})"},
            "scales": {
                "xAxes": [
                    {
                        "ticks": {"beginAtZero": True},
                        "scaleLabel": {"display": True, "labelString": f"Time ({unit})"},
                    }
                ]
            },
            "plugins": {
                "datalabels": {
                    "display": True,
                    "anchor": "end",
                    "align": "end",
                    "formatter": "(value) => value > 0 ? value.toFixed(2) : ''",
                }
            },
        },
    }

    chart_json = json.dumps(chart_config, separators=(",", ":"))
    height = max(200, len(labels) * 40)
    return f"https://quickchart.io/chart?c={urllib.parse.quote(chart_json)}&w=700&h={height}"


# =============================================================================
# Summary Generation
# =============================================================================


def generate_summary(results):
    """Generate markdown summary with tables and charts."""
    lines = [
        "## Benchmark Results",
        "",
        "**Target:** NXP i.MX 8M Plus (Cortex-A53 @ 1.8GHz)",
        f"**Total benchmarks:** {len(results)}",
        "",
    ]

    if not results:
        lines.append("No benchmark data available.")
        return "\n".join(lines)

    # Group by benchmark group
    groups = defaultdict(list)
    for r in results:
        groups[r["group"]].append(r)

    for group_name in ["convert", "resize", "letterbox"]:
        entries = groups.get(group_name)
        if not entries:
            continue

        info = GROUP_INFO.get(group_name, {"title": group_name, "desc": ""})
        lines.append(f"### {info['title']}")
        lines.append("")
        lines.append(f"*{info['desc']}*")
        lines.append("")

        # Group by config, with heap as columns
        configs = defaultdict(dict)
        for e in entries:
            configs[e["config"]][e["heap"]] = e

        sorted_configs = sorted(configs.items(), key=lambda x: resolution_sort_key(x[0]))

        # Table with uncached/cached columns
        lines.append("| Configuration | Uncached | Cached | Throughput |")
        lines.append("|--------------|----------|--------|------------|")

        chart_labels = []
        uncached_times = []
        cached_times = []

        for config, heap_data in sorted_configs:
            uncached = heap_data.get("uncached")
            cached = heap_data.get("cached")

            uncached_str = format_time(uncached["ns"]) if uncached else "N/A"
            cached_str = format_time(cached["ns"]) if cached else "N/A"

            # Use uncached throughput if available, else cached
            thrpt = "N/A"
            if uncached and uncached["throughput_str"] != "N/A":
                thrpt = uncached["throughput_str"]
            elif cached and cached["throughput_str"] != "N/A":
                thrpt = cached["throughput_str"]

            lines.append(f"| {config} | {uncached_str} | {cached_str} | {thrpt} |")

            # Collect for chart
            chart_labels.append(config)
            uncached_times.append(format_time_ms(uncached["ns"]) if uncached else 0)
            cached_times.append(format_time_ms(cached["ns"]) if cached else 0)

        lines.append("")

        # Generate chart
        datasets = [
            {
                "label": "Uncached",
                "data": uncached_times,
                "backgroundColor": "rgba(54, 162, 235, 0.8)",
                "borderColor": "rgba(54, 162, 235, 1)",
                "borderWidth": 1,
            },
            {
                "label": "Cached",
                "data": cached_times,
                "backgroundColor": "rgba(255, 159, 64, 0.8)",
                "borderColor": "rgba(255, 159, 64, 1)",
                "borderWidth": 1,
            },
        ]

        chart_url = generate_chart_url(info["title"], chart_labels, datasets)
        lines.append(f"![{info['title']} Chart]({chart_url})")
        lines.append("")

    return "\n".join(lines)


# =============================================================================
# Main
# =============================================================================


def main():
    parser = argparse.ArgumentParser(
        description="Generate markdown summary from G2D criterion benchmarks."
    )
    parser.add_argument(
        "--criterion-dir",
        help="Path to Criterion JSON data directory (preferred)",
    )
    parser.add_argument(
        "--bencher-file",
        help="Path to bencher text output file (fallback)",
    )
    parser.add_argument("--output", "-o", help="Output file (default: stdout)")

    # Support legacy positional argument
    parser.add_argument(
        "benchmark_file",
        nargs="?",
        help="Legacy: path to bencher output file",
    )

    args = parser.parse_args()

    results = []

    # Try Criterion JSON first (richer data)
    if args.criterion_dir and os.path.isdir(args.criterion_dir):
        results = parse_criterion_json(args.criterion_dir)
        if results:
            print(f"Parsed {len(results)} benchmarks from Criterion JSON", file=sys.stderr)

    # Fall back to bencher text
    if not results:
        bencher_file = args.bencher_file or args.benchmark_file
        if bencher_file and os.path.exists(bencher_file):
            results = parse_bencher_output(bencher_file)
            if results:
                print(
                    f"Parsed {len(results)} benchmarks from bencher output",
                    file=sys.stderr,
                )

    summary = generate_summary(results)

    if args.output:
        with open(args.output, "w") as f:
            f.write(summary)
        print(f"Wrote summary to {args.output}", file=sys.stderr)
    else:
        print(summary)

    return 0


if __name__ == "__main__":
    sys.exit(main())
