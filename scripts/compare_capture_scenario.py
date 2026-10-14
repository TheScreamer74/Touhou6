#!/usr/bin/env python3
"""Compare every real reference checkpoint, keeping transitions and text visible.

No spatial/temporal alignment, scale correction or masks are applied. A scalar
checkpoint must match before its image can be interpreted as a rendering test.
The report still retains image differences when that prerequisite fails.
"""
import argparse
import csv
import hashlib
import json
import math
from pathlib import Path
import re

from compare_capture_images import compare


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source,'sha256').hexdigest()


def run(reference, actual, output):
    receipt = None
    receipt_path = actual / "capture-execution.json"
    if receipt_path.exists():
        receipt = json.loads(receipt_path.read_text())
        if receipt.get("status") != "completed-process" or receipt.get("returncode") != 0:
            raise ValueError("Port capture did not finish successfully")
        if receipt.get("binary_sha256") != receipt.get("binary_after_sha256"):
            raise ValueError("Port capture executable changed during execution")
        for name, expected_hash in receipt.get("outputs_sha256", {}).items():
            if Path(name).name != name or name in (".", ".."):
                raise ValueError("Invalid captured output filename")
            path = actual / name
            if not path.is_file() or digest(path) != expected_hash:
                raise ValueError(f"Captured output changed or is missing: {name}")
    scalar_path = reference.parent / "replay-state.csv"
    fields = []
    states = {}
    if scalar_path.exists():
        reader = csv.DictReader(scalar_path.open())
        fields = [field for field in reader.fieldnames if field != "present"]
        for row in reader:
            key = int(row["stage"]), int(row["frame"])
            if key in states:
                raise ValueError(f"Duplicate original scalar checkpoint {key}")
            if not all(math.isfinite(float(row[field])) for field in fields):
                raise ValueError(f"Non-finite original scalar checkpoint {key}")
            states[key] = row
    port_states = {}
    for row in csv.DictReader((actual / "checkpoints.csv").open()):
        if row["clock"] == "replay":
            # The first replay checkpoint is calculated after drawing Replay
            # menu state, so draw_stage is empty. Transitions can likewise draw
            # another stage. Scalar identity belongs to the post-calc state.
            values = row["state"].split(",")
            if len(values) < 2 or int(values[0]) != int(row["checkpoint"]):
                raise ValueError("Port checkpoint ID differs from its replay state")
            key = int(values[1]), int(row["checkpoint"])
            if key in port_states:
                raise ValueError(f"Duplicate port scalar checkpoint {key}")
            if not all(math.isfinite(float(value)) for value in values):
                raise ValueError(f"Non-finite port scalar checkpoint {key}")
            port_states[key] = row
    reports, missing, differences = [], [], []
    regions = {"full_frame": (0, 0, 640, 480), "playfield": (32, 16, 384, 448),
               "hud": (416, 0, 224, 480), "fps": (512, 464, 128, 16)}
    seen = set()
    for original in sorted(reference.iterdir()):
        match = re.fullmatch(r"replay_stage(\d+)_frame(\d+)\.(?:bmp|png)", original.name)
        if not match:
            continue
        stage, frame = map(int, match.groups())
        if (stage, frame) in seen:
            raise ValueError(f"Duplicate original image checkpoint {(stage, frame)}")
        seen.add((stage, frame))
        if receipt is not None:
            declared = receipt.get("originals", {}).get(f"{stage}:{frame}")
            if declared is None or declared.get("sha256") != digest(original):
                raise ValueError(f"Original image changed since capture: {original.name}")
        image = actual / f"replay_stage{stage:02}_frame{frame:06}.png"
        if not image.exists():
            missing.append(str(image))
            continue
        expected, observed = states.get((stage, frame)), port_states.get((stage, frame))
        scalar_errors = []
        if expected is None or observed is None or not fields:
            scalar_errors.append("missing original or port scalar checkpoint")
        else:
            values = observed["state"].split(",")
            if len(values) != len(fields):
                scalar_errors.append("scalar field count differs")
            else:
                for field, value in zip(fields, values):
                    tolerance = 0.0002 if field in ("x", "y") else 0.0
                    if abs(float(expected[field]) - float(value)) > tolerance:
                        scalar_errors.append({"field": field, "original": expected[field], "port": value})
        if scalar_errors:
            differences.append({"stage": stage, "frame": frame, "differences": scalar_errors})
        destination = output / f"stage{stage:02}_frame{frame:06}"
        report = compare(original, image, destination, regions, 2, 0.01, 1.0)
        reports.append({"stage": stage, "frame": frame, "metrics": str(destination / "metrics.json"),
                        "scalar_checkpoint_equal": not scalar_errors, "regions": report["regions"],
                        "all_regions_within_explicit_tolerance": report["all_regions_within_explicit_tolerance"]})
    if not reports:
        raise ValueError("No real original and port image pairs")
    if receipt is not None and set(receipt.get("originals", {})) != {f"{stage}:{frame}" for stage,frame in seen}:
        raise ValueError("Original image checkpoint set changed since port capture")
    summary = {"reference_directory": str(reference.resolve()), "port_directory": str(actual.resolve()),
               "pairs": len(reports), "missing_images": missing, "scalar_differences": differences,
               "passed": not missing and not differences and all(
                   report["all_regions_within_explicit_tolerance"] for report in reports),
               "criteria": {"channel_error_tolerance": 2, "maximum_bad_fraction": 0.01, "maximum_mae": 1.0},
               "notes": ["All pixels, including fonts, FPS and transitions, remain in full_frame/playfield/hud metrics.",
                         ("Menu entry draws the original title/Replay input schedule and retains its framebuffer/FPS history before frame1."
                          if receipt and receipt.get("menu_entry") else
                          "Diagnostic direct --replay entry lacks the preceding Replay-menu framebuffer/FPS history; any frame1 mismatch is retained."),
                         "Scalar equality at selected checkpoints does not establish every-tick or object-pool equality.",
                         "Image criteria measure renderer differences and do not establish gameplay equivalence."],
               "checkpoints": reports}
    output.mkdir(parents=True, exist_ok=True)
    (output / "summary.json").write_text(json.dumps(summary, indent=2) + "\n")
    return summary


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reference", type=Path)
    parser.add_argument("port", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = run(args.reference, args.port, args.output)
    print(json.dumps({"pairs": result["pairs"], "missing_images": result["missing_images"],
                      "scalar_differences": result["scalar_differences"], "passed": result["passed"],
                      "summary": str(args.output / "summary.json")}, indent=2))
    raise SystemExit(0 if result["passed"] else 1)
