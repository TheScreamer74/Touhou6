#!/usr/bin/env python3
"""Join stage inputs into one original-format replay for transition probes.

The component replays must share character/difficulty and contain disjoint
stage data. This writes a new immutable input and declares synthetic provenance.
It does not patch the executable or transplant runtime state.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct


def decode(path):
    raw = bytearray(path.read_bytes())
    key = raw[14]
    for i in range(15, len(raw)):
        raw[i] = (raw[i] - key) & 255
        key = (key + 7) & 255
    if raw[:4] != b'T6RP' or (sum(raw[14:]) + 0x3f000318) & 0xffffffff != struct.unpack_from('<I', raw, 8)[0]:
        raise ValueError('invalid replay')
    offsets = struct.unpack_from('<7I', raw, 52)
    stages = {}
    for i, at in enumerate(offsets):
        if at:
            end = next((p for p in offsets[i + 1:] if p), len(raw))
            stages[i] = raw[at:end]
    return raw[:80], stages


def merge(inputs, output, reserve_sentinels=0):
    header, stages = decode(inputs[0])
    for path in inputs[1:]:
        current, extra = decode(path)
        if current[6:8] != header[6:8] or stages.keys() & extra.keys():
            raise ValueError('incompatible stages')
        stages.update(extra)
    raw = header
    struct.pack_into('<7I', raw, 52, *([0] * 7))
    for stage in sorted(stages):
        struct.pack_into('<I', raw, 52 + stage * 4, len(raw))
        raw.extend(stages[stage])
        for _ in range(reserve_sentinels):
            raw.extend(struct.pack('<iHH', 9999999, 0, 0))
    struct.pack_into('<I', raw, 8, (sum(raw[14:]) + 0x3f000318) & 0xffffffff)
    key = raw[14]
    for i in range(15, len(raw)):
        raw[i] = (raw[i] + key) & 255
        key = (key + 7) & 255
    output.mkdir(parents=True, exist_ok=True)
    if (output / 'input.rpy').exists():
        raise ValueError('refusing to overwrite immutable input')
    (output / 'input.rpy').write_bytes(raw)
    sources = [{'path': str(p), 'sha256': hashlib.sha256(p.read_bytes()).hexdigest()} for p in inputs]
    data = {'id': output.name, 'kind': 'synthetic-original-format-multistage-replay',
            'god': True, 'generator_frames': 15000, 'expected_checkpoint': 'original-msg-stage-end',
            'sources': sources, 'stages': [i + 1 for i in sorted(stages)],
            'reserved_sentinel_records_per_stage': reserve_sentinels,
            'reservation_reason': 'Original StopRecordingReplay writes two records after the current playback cursor on CutChain. Synthetic inputs may stop before the original terminal; reserve records prevent corrupting the following stage header.',
            'limit': 'Observe first transition into next recorded stage; later empty stages are not valid transition evidence.'}
    (output / 'input-provenance.json').write_text(json.dumps(data, indent=2) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('output', type=Path)
    parser.add_argument('inputs', nargs='+', type=Path)
    parser.add_argument('--reserve-sentinels', type=int, default=0)
    args = parser.parse_args()
    if not 0 <= args.reserve_sentinels <= 8:
        parser.error('reserve-sentinels must be0..8')
    merge(args.inputs, args.output, args.reserve_sentinels)
