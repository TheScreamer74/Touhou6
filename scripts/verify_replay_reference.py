#!/usr/bin/env python3
"""Compare refcap's read-only 1.02h scalar trace with --replay-dump.

Use the first original replay session (the title demo repeats). Compare by
ReplayManager.frameId, rather than Present count: eligible replay dialogue
executes up to three simulation ticks before the original renders again.
"""
import argparse
import csv
import hashlib
import json
import math
from pathlib import Path


def compare(reference, port, tolerance):
    original = []
    previous = -1
    for row in csv.DictReader(reference.open()):
        frame = int(row["frame"])
        if frame < previous:
            break
        original.append(row)
        previous = frame
    actual = {int(row["frame"]): row for row in csv.DictReader(port.open())}
    fields = [key for key in original[0] if key != "present"]
    differences = {}
    compared = 0
    missing = []
    for expected in original:
        frame = int(expected["frame"])
        observed = actual.get(frame)
        if observed is None:
            missing.append(frame)
            continue
        compared += 1
        for field in fields:
            allowed = tolerance if field in ("x", "y") else 0
            expected_value, observed_value = float(expected[field]), float(observed[field])
            if (not math.isfinite(expected_value) or not math.isfinite(observed_value)
                    or abs(expected_value - observed_value) > allowed):
                record = differences.setdefault(field, {
                    "count": 0, "first_frame": frame,
                    "expected": expected[field], "actual": observed[field],
                })
                record["count"] += 1
    first = min((value["first_frame"] for value in differences.values()), default=None)
    return {
        "reference_sha256": hashlib.sha256(reference.read_bytes()).hexdigest(),
        "port_sha256": hashlib.sha256(port.read_bytes()).hexdigest(),
        "reference_observations": len(original), "compared_observations": compared,
        "last_reference_frame": previous, "missing_frames": missing,
        "float_position_tolerance_pixels": tolerance,
        "first_difference_frame": first, "differences": differences,
        "passed": compared == len(original) and not differences,
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reference", type=Path)
    parser.add_argument("port", type=Path)
    parser.add_argument("--output", type=Path)
    parser.add_argument("--position-tolerance", type=float, default=0.0002)
    args = parser.parse_args()
    result = compare(args.reference, args.port, args.position_tolerance)
    body = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.write_text(body)
    print(body)
    raise SystemExit(0 if result["passed"] else 1)
