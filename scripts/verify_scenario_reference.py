#!/usr/bin/env python3
"""Fail-closed 102h/port comparison keyed by (stage, replay frame).

Scalar observations, explicit object-pool checkpoints and a semantic terminal
event are separate requirements. Missing/empty input cannot produce a PASS.
"""
import argparse
import csv
import hashlib
import json
import math
from pathlib import Path

POOLS = ("enemies", "shots", "bullets", "lasers", "items")
FLOAT_FIELDS = {"x", "y", "z", "vx", "vy", "angle", "speed", "width", "height",
                "start_offset", "end_offset"}


class Evidence(list):
    """Count every issue, but bound diagnostic storage on divergent long runs."""
    def __init__(self, limit):
        super().__init__()
        self.limit, self.count = limit, 0

    def append(self, value):
        self.count += 1
        if super().__len__() < self.limit:
            super().append(value)

    def extend(self, values):
        for value in values:
            self.append(value)
        if isinstance(values, Evidence):
            self.count += len(values) - list.__len__(values)

    def __len__(self):
        return self.count


def sha(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def read_scalars(path):
    rows = {}
    errors = Evidence(100)
    with path.open() as source:
        reader = csv.DictReader(source)
        has_present = 'present' in (reader.fieldnames or [])
        previous_present = None
        for row in reader:
            if has_present:
                present = int(row['present'])
                if previous_present is not None and present != previous_present + 1:
                    errors.append(f'non-contiguous original Present {previous_present} -> {present}')
                previous_present = present
            key = int(row["stage"]), int(row["frame"])
            row.pop("present", None)
            if key in rows:
                errors.append(f"duplicate scalar {key}")
            rows[key] = row
    if not rows:
        errors.append("empty scalar trace")
    return rows, errors


def iter_pools(path, errors, expected_stage):
    """Keep at most one decoded pool checkpoint in memory per trace."""
    seen, previous, count = set(), None, 0
    with path.open() as source:
        for line in source:
            item = json.loads(line)
            key = item["stage"], item["frame"]
            if not all(pool in item and isinstance(item[pool], list) for pool in POOLS):
                errors.append(f"missing pool array {key}")
                continue
            if key[0] != expected_stage:
                errors.append(f"pool contains a different stage {key}")
            if key in seen:
                errors.append(f"duplicate pool {key}")
            if previous is not None and key < previous:
                errors.append(f"out-of-order pool {key}")
            seen.add(key)
            previous = key
            count += 1
            yield key, item
    if not count:
        errors.append("empty pool trace")


def unequal(a, b, tolerance):
    if not isinstance(a, (float, int)) or not isinstance(b, (float, int)):
        return True
    return not math.isfinite(a) or not math.isfinite(b) or abs(a - b) > tolerance


def compare(reference, actual, pools_reference, pools_actual, expected_stage,
            expected_terminal, actual_terminal, required_frame, tolerance=0.0002):
    original, errors = read_scalars(reference)
    if expected_terminal is None and required_frame is None:
        errors.append("no required checkpoint or terminal was specified")
    port, port_errors = read_scalars(actual)
    errors.extend(port_errors)
    if any(stage != expected_stage for stage, _ in original):
        errors.append("reference contains a different stage")
    if any(stage != expected_stage for stage, _ in port):
        errors.append("port contains a different stage")
    if required_frame and (expected_stage, required_frame) not in original:
        errors.append(f"reference did not reach required frame {required_frame}")
    scalar_differences, pool_differences = Evidence(40), Evidence(40)
    compared = 0
    for key, expected in sorted(original.items()):
        if required_frame and key[1] > required_frame:
            continue
        observed = port.get(key)
        if observed is None:
            errors.append(f"missing port scalar {key}")
            continue
        compared += 1
        for field in expected:
            if field not in observed:
                errors.append(f"missing scalar field {field}")
                continue
            allowed = tolerance if field in ("x", "y") else 0
            if unequal(float(expected[field]), float(observed[field]), allowed):
                scalar_differences.append({"stage": key[0], "frame": key[1], "field": field,
                    "expected": expected[field], "actual": observed[field]})
    checkpoints = 0
    port_pools = iter(iter_pools(pools_actual, errors, expected_stage))
    observed_pool = next(port_pools, None)
    for key, expected in iter_pools(pools_reference, errors, expected_stage):
        if required_frame and key[1] > required_frame:
            continue
        while observed_pool is not None and observed_pool[0] < key:
            observed_pool = next(port_pools, None)
        observed = observed_pool[1] if observed_pool is not None and observed_pool[0] == key else None
        if observed is None:
            errors.append(f"missing port pool checkpoint {key}")
            continue
        checkpoints += 1
        for pool in POOLS:
            expected_objects = {row["slot"]: row for row in expected[pool]}
            observed_objects = {row["slot"]: row for row in observed[pool]}
            if (len(expected_objects) != len(expected[pool]) or
                    len(observed_objects) != len(observed[pool])):
                errors.append(f"duplicate object slot {key} {pool}")
            missing = sorted(expected_objects.keys() - observed_objects.keys())
            extra = sorted(observed_objects.keys() - expected_objects.keys())
            if missing or extra:
                pool_differences.append({"stage": key[0], "frame": key[1], "pool": pool,
                    "missing_slots": missing, "extra_slots": extra})
            for slot in expected_objects.keys() & observed_objects.keys():
                a, b = expected_objects[slot], observed_objects[slot]
                if a.keys() != b.keys():
                    errors.append(f"object field mismatch {key} {pool} {slot}")
                    continue
                for field in a:
                    if unequal(a[field], b[field], tolerance if field in FLOAT_FIELDS else 0):
                        pool_differences.append({"stage": key[0], "frame": key[1], "pool": pool,
                            "slot": slot, "field": field, "expected": a[field], "actual": b[field]})
    # Validate trailing observations too: duplicates, reordered records or a
    # truncated/empty source can never be hidden by a matched earlier subset.
    for _ in port_pools:
        pass
    terminal_match = None
    if expected_terminal is not None:
        expected = json.loads(expected_terminal.read_text())
        endpoint = (expected["stage"], expected["frame"])
        if endpoint not in original:
            errors.append("reference is truncated before its terminal observation")
        if endpoint not in port:
            errors.append("port is truncated before the reference terminal observation")
        game_over = expected.get("event") == "game-over"
        if not game_over and expected.get("finished_stage", 0) < 1:
            errors.append("original terminal did not finish the stage message")
        if not actual_terminal or not actual_terminal.exists():
            errors.append("missing port terminal event")
        else:
            events = [json.loads(line) for line in actual_terminal.read_text().splitlines()]
            if len(events) != 1:
                errors.append("port terminal evidence must contain exactly one event")
            terminal_match = any(event["event"] == ("game-over" if game_over else "msg-stage-end") and
                event["stage"] == expected["stage"] and event["frame"] == expected["frame"]
                for event in events)
            if not terminal_match:
                errors.append("terminal event stage/frame differs")
        if expected.get("is_replay") != 1 or expected.get("supervisor_state") not in ((2,) if game_over else (3, 7, 8, 10)):
            errors.append("original terminal is not a replay stage-end transition")
    first = min((row["frame"] for rows in (scalar_differences, pool_differences) for row in rows), default=None)
    return {
        "passed": bool(compared and checkpoints) and not errors and not scalar_differences and not pool_differences,
        "reference_sha256": sha(reference), "port_sha256": sha(actual),
        "reference_pools_sha256": sha(pools_reference), "port_pools_sha256": sha(pools_actual),
        "reference_terminal_sha256": sha(expected_terminal) if expected_terminal is not None else None,
        "port_terminal_sha256": sha(actual_terminal) if actual_terminal is not None and actual_terminal.exists() else None,
        "stage": expected_stage, "required_frame": required_frame,
        "reference_observations": len(original), "compared_observations": compared,
        "compared_pool_checkpoints": checkpoints, "terminal_matched": terminal_match,
        "first_difference_frame": first,
        "scalar_difference_count": len(scalar_differences), "pool_difference_count": len(pool_differences),
        "scalar_differences": scalar_differences[:40], "pool_differences": pool_differences[:40],
        "errors": errors[:100], "error_count": len(errors), "float_tolerance": tolerance,
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reference", type=Path)
    parser.add_argument("port", type=Path)
    parser.add_argument("--reference-pools", required=True, type=Path)
    parser.add_argument("--port-pools", required=True, type=Path)
    parser.add_argument("--stage", required=True, type=int)
    parser.add_argument("--reference-terminal", type=Path)
    parser.add_argument("--port-terminal", type=Path)
    parser.add_argument("--required-frame", type=int)
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    result = compare(args.reference, args.port, args.reference_pools, args.port_pools,
                     args.stage, args.reference_terminal, args.port_terminal, args.required_frame)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({key: result[key] for key in ("passed", "compared_observations",
        "compared_pool_checkpoints", "first_difference_frame", "scalar_difference_count",
        "pool_difference_count", "error_count")}, indent=2))
    raise SystemExit(0 if result["passed"] else 1)
