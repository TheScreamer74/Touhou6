#!/usr/bin/env python3
"""Measure actual synchronized captures without rescaling or hidden masks.

This measures image differences, not state equivalence. Verify the associated
replay-state records before interpreting renderer differences. Text remains in
the full-frame/playfield metrics even when its GDI rasterizer is known to differ.
Requires Pillow and NumPy. Original BMPs and port PNGs are both accepted.
"""
import argparse
import hashlib
import json
from pathlib import Path

import numpy as np
from PIL import Image, ImageDraw


def region(value):
    try:
        name, *numbers = value.split(",")
        x, y, width, height = map(int, numbers)
        if not name or min(x, y) < 0 or min(width, height) <= 0:
            raise ValueError
        return name, (x, y, width, height)
    except ValueError as error:
        raise argparse.ArgumentTypeError("region must be name,x,y,width,height") from error


def compare(reference, actual, destination, regions, tolerance, bad_fraction, mae):
    ref = np.asarray(Image.open(reference).convert("RGB"))
    port = np.asarray(Image.open(actual).convert("RGB"))
    if ref.shape != (480, 640, 3) or port.shape != ref.shape:
        raise ValueError(f"Expected unscaled 640×480 captures, got {ref.shape} and {port.shape}")
    difference = np.abs(ref.astype(np.int16) - port.astype(np.int16))
    pixel_max = difference.max(axis=2)
    measured = {}
    for name, (x, y, width, height) in regions.items():
        if x + width > 640 or y + height > 480:
            raise ValueError(f"Region {name} extends beyond the original frame")
        channels = difference[y:y+height, x:x+width].astype(np.float64)
        pixels = pixel_max[y:y+height, x:x+width]
        result = {
            "rectangle_xywh": [x, y, width, height], "pixels": width * height,
            "mean_absolute_channel_error": float(channels.mean()),
            "root_mean_square_channel_error": float(np.sqrt(np.mean(channels ** 2))),
            "maximum_channel_error": int(channels.max()),
            "pixel_max_error_percentiles": dict(zip(["p50", "p95", "p99"], map(float, np.percentile(pixels, [50, 95, 99])))),
            "fraction_above_channel_error": {str(level): float(np.mean(pixels > level)) for level in sorted({0, 2, 8, 16, 32, tolerance})},
        }
        result["within_explicit_tolerance"] = (result["mean_absolute_channel_error"] <= mae and result["fraction_above_channel_error"][str(tolerance)] <= bad_fraction)
        measured[name] = result
    destination.mkdir(parents=True, exist_ok=True)
    Image.fromarray(difference.astype(np.uint8)).save(destination / "absolute-difference.png")
    # Monotone black→red→yellow→white heatmap; 64/255 error saturates at white.
    heat = np.stack([np.minimum(pixel_max*4, 255), np.clip(pixel_max*4-85, 0, 255), np.clip(pixel_max*4-170, 0, 255)], axis=2).astype(np.uint8)
    Image.fromarray(heat).save(destination / "heatmap.png")
    panel = Image.new("RGB", (1920, 504), "#202020")
    for index, image in enumerate([ref, port, heat]):
        panel.paste(Image.fromarray(image), (index*640, 24))
    draw = ImageDraw.Draw(panel)
    for index, label in enumerate(["Original capture (unmodified)", "Port capture (unmodified)", "Error heatmap (4x; white at >=64/255)"]):
        draw.text((index*640+8, 6), label, fill="white")
    panel.save(destination / "comparison.png")
    report = {
        "reference": str(reference.resolve()), "reference_sha256": hashlib.sha256(reference.read_bytes()).hexdigest(),
        "actual": str(actual.resolve()), "actual_sha256": hashlib.sha256(actual.read_bytes()).hexdigest(),
        "size": [640, 480], "space": "encoded RGB8 channel values; no gamma transform, resize, alignment or masks",
        "criteria": {"channel_error_tolerance": tolerance, "maximum_fraction_of_pixels_above_tolerance": bad_fraction, "maximum_mean_absolute_channel_error": mae},
        "regions": measured, "all_regions_within_explicit_tolerance": all(item["within_explicit_tolerance"] for item in measured.values()),
        "state_equivalence": "Not established by this image tool; compare checkpoint/replay-state records separately.",
    }
    (destination / "metrics.json").write_text(json.dumps(report, indent=2)+"\n")
    return report


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reference", type=Path)
    parser.add_argument("actual", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--region", type=region, action="append", default=[])
    parser.add_argument("--channel-tolerance", type=int, default=2)
    parser.add_argument("--max-bad-fraction", type=float, default=0.01)
    parser.add_argument("--max-mae", type=float, default=1.0)
    args = parser.parse_args()
    if not 0 <= args.channel_tolerance <= 255 or not 0 <= args.max_bad_fraction <= 1 or args.max_mae < 0:
        parser.error("Invalid explicit image tolerance")
    regions = {"full_frame": (0, 0, 640, 480), "playfield": (32, 16, 384, 448), "hud": (416, 0, 224, 480)}
    for name, rectangle in args.region:
        if name in regions:
            parser.error(f"Region name {name} already exists; defaults cannot be silently replaced")
        regions[name] = rectangle
    report = compare(args.reference, args.actual, args.output, regions, args.channel_tolerance, args.max_bad_fraction, args.max_mae)
    print(json.dumps(report, indent=2))
