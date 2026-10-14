#!/usr/bin/env python3
"""Strictly compare original Present observations with port update p+1.

Both runs must consume the same natural menu/input sequence. This comparator
does not transplant state, shift an inferred clock, or permit missing frames.
"""
import argparse
import csv
import hashlib
import json
from pathlib import Path


FIELDS = ("scene", "frame", "stage", "pause_state", "pause_timer",
          "retry_state", "retry_timer", "game_menu", "retry")
STAGE_FIELDS = ("frame", "stage", "x", "y", "lives", "bombs", "power", "score",
                "graze_stage", "graze_total", "rank", "subrank", "rng", "bullets",
                "lasers", "items", "particle_timer")


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def indexed(path, clock, offset):
    result = {}
    with path.open() as source:
        for row in csv.DictReader(source):
            frame = int(row[clock]) + offset
            if frame in result:
                raise ValueError(f"Duplicate observation {frame}: {path}")
            result[frame] = {field: int(row[field]) for field in FIELDS}
    if not result:
        raise ValueError(f"Empty transition trace: {path}")
    return result


def compare(reference, port, first, last):
    if first < 0 or last < first:
        raise ValueError("Invalid required presentation interval")
    expected = indexed(reference, "present", 0)
    actual = indexed(port, "update", -1)
    differences, missing = [], []
    count = compared = 0
    for frame in range(first, last + 1):
        if frame not in expected or frame not in actual:
            if len(missing) < 30:
                missing.append(frame)
            count += 1
            continue
        compared += 1
        for field in FIELDS:
            want, got = expected[frame][field], actual[frame][field]
            if want != got:
                count += 1
                if len(differences) < 30:
                    differences.append({"present": frame, "field": field,
                                        "expected": want, "actual": got})
    return {
        "reference_sha256": digest(reference), "port_sha256": digest(port),
        "required_present_interval": [first, last], "fields": list(FIELDS),
        "required_observations": last - first + 1, "compared_observations": compared,
        "different_or_missing_fields": count, "first_missing_frames": missing,
        "first_differences": differences,
        "passed": compared == last - first + 1 and count == 0,
        "scope": "Supervisor, stage/input clock, pause/retry states and clocks; pixels, gameplay pools and physical PCM are independent comparisons.",
    }


def compare_stage_observations(reference, port, first=None, last=None):
    """Compare every passive gameplay observation, including the final callback.

    Coordinates are compared as the original diagnostic's six-decimal strings;
    no epsilon or inferred timing offset is applied. Menus may repeat a stage
    frame, so the canonical presentation clock identifies each observation.
    """
    with reference.open() as source:
        expected = [row for row in csv.DictReader(source)
                    if (first is None or int(row["present"]) >= first)
                    and (last is None or int(row["present"]) <= last)]
    actual = {}
    with port.open() as source:
        for row in csv.DictReader(source):
            present = int(row["update"]) - 1
            if present in actual:
                raise ValueError(f"Duplicate stage observation {present}: {port}")
            actual[present] = row
    seen, differences = set(), []
    count = 0
    for row in expected:
        present = int(row["present"])
        if present in seen:
            raise ValueError(f"Duplicate reference stage observation {present}: {reference}")
        seen.add(present)
        for field in STAGE_FIELDS:
            want = row[field]
            got = actual.get(present, {}).get("active_calc" if field == "frame" else field)
            if want != got:
                count += 1
                if len(differences) < 30:
                    differences.append({"present": present, "field": field,
                                        "expected": want, "actual": got})
    return {"reference_sha256": digest(reference), "port_sha256": digest(port),
            "required_present_interval": [first, last],
            "compared_original_observations": len(expected), "fields": list(STAGE_FIELDS),
            "different_fields": count, "first_differences": differences,
            "coordinate_comparison": "Exact six-decimal diagnostic strings, without epsilon.",
            "passed": bool(expected) and count == 0}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reference", type=Path)
    parser.add_argument("port", type=Path)
    parser.add_argument("--first-present", type=int, required=True)
    parser.add_argument("--last-present", type=int, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--stage-reference", type=Path)
    parser.add_argument("--stage-port", type=Path)
    args = parser.parse_args()
    result = compare(args.reference, args.port, args.first_present, args.last_present)
    if args.stage_reference or args.stage_port:
        if not args.stage_reference or not args.stage_port:
            parser.error("--stage-reference and --stage-port must be supplied together")
        result["stage_observations"] = compare_stage_observations(
            args.stage_reference, args.stage_port, args.first_present, args.last_present)
        result["passed"] = result["passed"] and result["stage_observations"]["passed"]
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps(result, indent=2))
    raise SystemExit(0 if result["passed"] else 1)
