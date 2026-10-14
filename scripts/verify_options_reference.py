#!/usr/bin/env python3
"""Compare read-only original menu options with native capture/options.csv.

Original Present p and port completed update p+1 consume the same scripted
keyboard interval. Windowed compares the original menu's pending choice, which
is what its UI displays; the original cfg.windowed is applied on leaving.
Startup/menu animation identity is outside this scalar comparison.
"""
import argparse
import csv
import hashlib
import json
from pathlib import Path


FIELDS = ("life_count", "bomb_count", "color_16bit", "music_mode",
          "play_sounds", "default_difficulty", "windowed")


def indexed(path, column, offset=0):
    with path.open() as source:
        rows = list(csv.DictReader(source))
    result = {}
    for row in rows:
        frame = int(row[column]) + offset
        if frame in result:
            raise ValueError(f"Duplicate {column} {frame}: {path}")
        result[frame] = row
    if not result:
        raise ValueError(f"Empty trace: {path}")
    return result


def compare(reference, port, start, end):
    if start < 0 or end < start:
        raise ValueError("Invalid required frame interval")
    expected = indexed(reference, "present")
    actual = indexed(port, "update", -1)
    differences, missing = {}, []
    compared = 0
    for frame in range(start, end + 1):
        if frame not in expected or frame not in actual:
            missing.append(frame)
            continue
        original, observed = expected[frame], actual[frame]
        if original["menu_state"] != "3" or observed["scene"] != "options":
            raise ValueError(f"Frame {frame} is outside the required Options scene")
        compared += 1
        for field in FIELDS:
            source_field = "pending_windowed" if field == "windowed" else field
            want, got = int(original[source_field]), int(observed[field])
            if want != got:
                detail = differences.setdefault(field, {
                    "first_present": frame, "expected": want, "actual": got,
                    "count": 0,
                })
                detail["count"] += 1
    return {
        "reference_sha256": hashlib.sha256(reference.read_bytes()).hexdigest(),
        "port_sha256": hashlib.sha256(port.read_bytes()).hexdigest(),
        "required_present_interval": [start, end],
        "required_observations": end - start + 1,
        "compared_observations": compared,
        "fields": list(FIELDS),
        "windowed_reference": "MainMenu.windowed (pending displayed choice)",
        "missing_frames": missing, "differences": differences,
        "passed": compared == end - start + 1 and not differences,
        "scope": "Options values only; excludes title/startup pixels and Windows device recreation.",
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reference", type=Path)
    parser.add_argument("port", type=Path)
    parser.add_argument("--start-present", type=int, required=True)
    parser.add_argument("--end-present", type=int, required=True)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = compare(args.reference, args.port, args.start_present, args.end_present)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))
    raise SystemExit(0 if result["passed"] else 1)
