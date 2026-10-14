#!/usr/bin/env python3
"""Compare complete END traces, including the required @z terminal.

These are explicit ending-entry fixtures. They verify the END interpreter and
credits after entry, not the preceding story completion or score persistence.
Original rows are observed after Present's draw-before-calc cycle. Both traces
must contain every tick before @z, without duplicates or gaps.
"""
import argparse
import csv
import hashlib
import json
import math
from pathlib import Path

FIELDS = ("file_prefix_fnv", "pc", "has_seen", "script_time", "pause", "clear",
          "minimum_pause", "minimum_clear", "long_delay", "short_delay",
          "text_line", "text_color", "background_y", "scroll_speed",
          "fade_kind", "fade_timer", "fade_duration")
FLOATS = {"background_y", "scroll_speed"}


def indexed(path):
    with path.open() as source:
        rows = list(csv.DictReader(source))
    result = {}
    for row in rows:
        tick = int(row["tick"])
        if tick in result:
            raise ValueError(f"Duplicate tick {tick}: {path}")
        result[tick] = row
    if not result:
        raise ValueError(f"Empty trace: {path}")
    return result


def compare(reference, port, reference_terminal, port_terminal):
    expected, actual = indexed(reference), indexed(port)
    ref_terminal_fields = reference_terminal.read_text().strip().split(",")
    if len(ref_terminal_fields) != 3 or int(ref_terminal_fields[2]) != 7:
        raise ValueError("Original ending did not reach ResultScreen via @z")
    want_terminal = int(ref_terminal_fields[1])
    got_terminal = int(port_terminal.read_text().strip())
    if want_terminal <= 1 or got_terminal <= 1:
        raise ValueError("Invalid terminal tick")
    differences, missing = {}, []
    compared = 0
    for tick in range(1, want_terminal):
        if tick not in expected or tick not in actual:
            missing.append(tick)
            continue
        compared += 1
        for field in FIELDS:
            if field in FLOATS:
                want, got = float(expected[tick][field]), float(actual[tick][field])
                if not math.isfinite(want) or not math.isfinite(got):
                    raise ValueError(f"Non-finite {field} at tick {tick}")
                different = abs(want-got) > 0.0002
            else:
                want, got = int(expected[tick][field]), int(actual[tick][field])
                different = want != got
            if different:
                detail = differences.setdefault(field, {
                    "first_tick": tick, "expected": want, "actual": got,
                    "count": 0, "maximum_absolute_error": 0,
                })
                detail["count"] += 1
                detail["maximum_absolute_error"] = max(detail["maximum_absolute_error"], abs(want-got))
    extra_reference = sorted(set(expected)-set(range(1, want_terminal)))
    extra_port = sorted(set(actual)-set(range(1, got_terminal)))
    return {
        "reference_sha256": hashlib.sha256(reference.read_bytes()).hexdigest(),
        "port_sha256": hashlib.sha256(port.read_bytes()).hexdigest(),
        "reference_terminal_tick": want_terminal, "port_terminal_tick": got_terminal,
        "required_observations": want_terminal-1, "compared_observations": compared,
        "fields": list(FIELDS), "float_absolute_tolerance": 0.0002,
        "missing_ticks": missing, "extra_reference_ticks": extra_reference,
        "extra_port_ticks": extra_port, "differences": differences,
        "passed": want_terminal == got_terminal and not differences and not missing
                  and not extra_reference and not extra_port,
        "scope": "END interpreter and credits after declared direct entry; preceding story clear excluded.",
    }


def compare_audio(reference, port, start_present, terminal_tick):
    expected, actual = [], []
    with reference.open() as source:
        for row in csv.DictReader(source):
            tick = int(row["present"]) - start_present + 1
            if not 1 <= tick <= terminal_tick:
                continue
            if row["event"] == "BGM_REQUEST":
                expected.append((tick, "music", row["value"].replace("\\", "/").split("/")[-1].lower().removesuffix(".mid")))
            elif row["event"] == "BGM_FADE":
                seconds = float(row["value"])
                if not math.isfinite(seconds):
                    raise ValueError("Non-finite reference fade duration")
                expected.append((tick, "fade", seconds))
    with port.open() as source:
        for row in csv.DictReader(source):
            tick, kind = int(row["tick"]), row["event"]
            if not 1 <= tick <= terminal_tick or kind not in ("music", "fade"):
                raise ValueError("Port ending audio event outside its required interval/type")
            value = float(row["value"]) if kind == "fade" else row["value"].lower()
            if kind == "fade" and not math.isfinite(value):
                raise ValueError("Non-finite port fade duration")
            actual.append((tick, kind, value))
    if not expected or not actual:
        raise ValueError("Empty ending music/fade trace")
    return {
        "reference_sha256": hashlib.sha256(reference.read_bytes()).hexdigest(),
        "port_sha256": hashlib.sha256(port.read_bytes()).hexdigest(),
        "reference_events": expected, "port_events": actual,
        "passed": expected == actual,
        "scope": "END music requests, fade durations and order; stage deletion StopAudio is outside this direct-entry fixture, PCM/timbre compared separately.",
    }


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("reference", type=Path)
    parser.add_argument("port", type=Path)
    parser.add_argument("--reference-terminal", type=Path, required=True)
    parser.add_argument("--port-terminal", type=Path, required=True)
    parser.add_argument("--output", type=Path, required=True)
    parser.add_argument("--reference-audio", type=Path)
    parser.add_argument("--port-events", type=Path)
    args = parser.parse_args()
    result = compare(args.reference, args.port, args.reference_terminal, args.port_terminal)
    if bool(args.reference_audio) != bool(args.port_events):
        parser.error("Supply both --reference-audio and --port-events")
    if args.reference_audio:
        start_present = int(indexed(args.reference)[1]["present"])
        result["audio"] = compare_audio(args.reference_audio, args.port_events, start_present, result["reference_terminal_tick"])
        result["passed"] = result["passed"] and result["audio"]["passed"]
    args.output.write_text(json.dumps(result, indent=2)+"\n")
    print(json.dumps(result, indent=2))
    raise SystemExit(0 if result["passed"] else 1)
