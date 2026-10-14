#!/usr/bin/env python3
"""Compare the replay and declared highscore produced by a natural save flow.

Checksums and file boundaries are verified before comparing interpreted records.
The original can append inputs after its first replay sentinel; playback never
reads that tail, so it is reported separately from the compared input stream.
"""

import argparse
import hashlib
import json
import math
from pathlib import Path
import struct


def sha256(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def u16(data, at):
    return struct.unpack_from("<H", data, at)[0]


def u32(data, at):
    return struct.unpack_from("<I", data, at)[0]


def text(data):
    return bytes(data).split(b"\0", 1)[0].decode("shift_jis")


def read_replay(path):
    raw = bytearray(path.read_bytes())
    if not 80 <= len(raw) <= 7 * 0x69780 + 80 or raw[:4] != b"T6RP":
        raise ValueError("invalid replay file")
    if u16(raw, 4) != 0x102 or raw[6] >= 4 or raw[7] >= 5:
        raise ValueError("unsupported replay header")
    key = raw[14]
    for i in range(15, len(raw)):
        raw[i] = (raw[i] - key) & 255
        key = (key + 7) & 255
    if (sum(raw[14:]) + 0x3F000318) & 0xFFFFFFFF != u32(raw, 8):
        raise ValueError("replay checksum mismatch")
    if not math.isfinite(struct.unpack_from("<f", raw, 44)[0]):
        raise ValueError("invalid replay slowdown")
    offsets = struct.unpack_from("<7I", raw, 52)
    stages = {}
    tails = {}
    previous_end = 80
    for stage, at in enumerate(offsets):
        if not at:
            continue
        end = next((value for value in offsets[stage + 1:] if value), len(raw))
        if at != previous_end or not at + 32 <= end <= len(raw) or (end - at - 16) % 8:
            raise ValueError("invalid replay stage offsets")
        if raw[at + 8] > 128 or raw[at + 9] > 8 or raw[at + 10] > 8 or raw[at + 11] > 32:
            raise ValueError("invalid replay stage state")
        inputs = []
        for position in range(at + 16, end, 8):
            frame, buttons, _ = struct.unpack_from("<iHH", raw, position)
            if not 0 <= frame <= 9_999_999 or inputs and frame < inputs[-1][0]:
                raise ValueError("unordered replay input")
            inputs.append([frame, buttons & 0x1F7])
            if frame == 9_999_999:
                tails[stage + 1] = (end - position - 8) // 8
                break
        if not inputs or inputs[0][0] != 0 or inputs[-1][0] != 9_999_999:
            raise ValueError("missing replay initial input or sentinel")
        stages[stage + 1] = {
            "score": u32(raw, at), "seed": u16(raw, at + 4),
            "point_items": u16(raw, at + 6), "power": raw[at + 8],
            "lives": raw[at + 9], "bombs": raw[at + 10],
            "rank": raw[at + 11], "power_item_count": raw[at + 12],
            "inputs": inputs,
        }
        previous_end = end
    if not stages or previous_end != len(raw):
        raise ValueError("missing replay stages or trailing data")
    return {
        "version": u16(raw, 4), "character": raw[6], "difficulty": raw[7],
        "date": text(raw[16:25]), "name": text(raw[25:33]),
        "score": u32(raw, 36), "slowdown_bits": raw[44:48].hex(),
        "stages": stages,
    }, tails


def read_highscores(path, minimum_score):
    raw = bytearray(path.read_bytes())
    if not 20 <= len(raw) <= 0xA0000:
        raise ValueError("invalid score.dat size")
    xor = 0
    for i in range(2, len(raw)):
        xor = (xor + raw[i - 1]) & 255
        xor = ((xor << 3) | (xor >> 5)) & 255
        raw[i] ^= xor
    if sum(raw[4:]) & 65535 != u16(raw, 2):
        raise ValueError("score.dat checksum mismatch")
    at, end = u32(raw, 8), u32(raw, 16)
    if end != len(raw) or not 20 <= at <= end:
        raise ValueError("invalid score.dat offsets")
    records = []
    header = False
    while at < end:
        if end - at < 12:
            raise ValueError("truncated score.dat record")
        size = u16(raw, at + 4)
        if not 12 <= size <= end - at:
            raise ValueError("invalid score.dat record size")
        record = raw[at:at + size]
        if record[8] == 16:
            if record[:4] == b"TH6K":
                header = True
            elif record[:4] == b"HSCR" and size >= 28:
                if record[16] >= 4 or record[17] >= 5:
                    raise ValueError("invalid highscore character/difficulty")
                if u32(record, 12) >= minimum_score:
                    records.append({
                        "score": u32(record, 12), "character": record[16],
                        "difficulty": record[17], "stage": record[18],
                        "name": text(record[19:28]),
                    })
        at += size
    if not header:
        raise ValueError("missing score.dat TH6K header")
    return records


def compare(original, port, slot=1, minimum_score=5_000_000):
    replay_name = f"replay/th6_{slot:02}.rpy"
    original_replay, original_tail = read_replay(original / replay_name)
    port_replay, port_tail = read_replay(port / replay_name)
    original_scores = read_highscores(original / "score.dat", minimum_score)
    port_scores = read_highscores(port / "score.dat", minimum_score)
    replay_equal = original_replay == port_replay
    scores_equal = bool(original_scores) and original_scores == port_scores
    return {
        "original_sha256": {name: sha256(original / name) for name in (replay_name, "score.dat")},
        "port_sha256": {name: sha256(port / name) for name in (replay_name, "score.dat")},
        "replay_playback_semantics_identical": replay_equal,
        "original_replay": original_replay, "port_replay": port_replay,
        "ignored_inputs_after_first_sentinel": {"original": original_tail, "port": port_tail},
        "minimum_compared_highscore": minimum_score,
        "highscore_records_identical": scores_equal,
        "original_highscores": original_scores, "port_highscores": port_scores,
        "protocol": "Validate both file checksums and offsets. Compare replay metadata, stage headers and every input through the first sentinel. Report ignored input tails separately. Compare all highscore records at or above the declared fixture threshold, preserving their order.",
        "passed": replay_equal and scores_equal,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("original", type=Path)
    parser.add_argument("port", type=Path)
    parser.add_argument("--slot", type=int, default=1)
    parser.add_argument("--minimum-score", type=int, default=5_000_000)
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    try:
        result = compare(args.original, args.port, args.slot, args.minimum_score)
    except (OSError, ValueError, struct.error) as error:
        result = {"passed": False, "error": str(error)}
    rendered = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.write_text(rendered)
    print(rendered, end="")
    return 0 if result["passed"] else 1


if __name__ == "__main__":
    raise SystemExit(main())
