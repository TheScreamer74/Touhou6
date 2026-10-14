#!/usr/bin/env python3
"""Summarize a GUI trace without transferring its individual observations.

Canonical SHA-256 hashes start with b'TH06-GUI-v1\0'. Each sorted (stage,frame)
row packs stage, frame and the nine fields as little-endian 32-bit words using
<iiiiIIiiiii. The two bar values use their IEEE754 binary32 bits; other values
are signed integers. Rust Display<f32> and the original %.9g both round-trip
binary32. Hash equality is exact, with no numeric tolerance or alignment.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct

from compare_boss_ui import FIELDS, load


def aggregate(path, stage, first, last):
    rows = load(path, stage, first, last)
    expected = {(stage, frame) for frame in range(first, last + 1)}
    if set(rows) != expected:
        raise ValueError('GUI trace does not contain every requested calculation ID')
    hashes = {field: hashlib.sha256(b'TH06-GUI-v1\0') for field in FIELDS}
    combined = hashlib.sha256(b'TH06-GUI-v1\0')
    transitions = []
    previous_state = None
    for (row_stage, frame), values in sorted(rows.items()):
        words = []
        for field in FIELDS:
            value = values[field]
            if field in ('bar_target', 'bar_display'):
                packed = struct.pack('<f', value)
                word = struct.unpack('<I', packed)[0]
            else:
                word = int(value)
                if word != value:
                    raise ValueError(f'Non-integer GUI field {field}')
                packed = struct.pack('<i', word)
            words.append(word)
            hashes[field].update(struct.pack('<ii', row_stage, frame) + packed)
        combined.update(struct.pack('<iiiiIIiiiii', row_stage, frame, *words))
        state = values['boss_present'], values['health_bar_state']
        if state != previous_state:
            transitions.append({'stage': row_stage, 'frame': frame, **values})
            previous_state = state
    stats = {}
    for field in FIELDS:
        values = [row[field] for _, row in sorted(rows.items())]
        stats[field] = {'min': min(values), 'max': max(values),
                        'unique': len(set(values)),
                        'changes': sum(a != b for a, b in zip(values, values[1:])),
                        'canonical_sha256': hashes[field].hexdigest()}
    return {'schema': 'TH06-GUI-v1: <iiiiIIiiiii; exact binary32 bar bits',
            'stage': stage, 'first': first, 'last': last,
            'rows': len(rows), 'complete_requested_range': True,
            'canonical_sha256': combined.hexdigest(),
            'fields': stats, 'transitions': transitions}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('trace', type=Path)
    parser.add_argument('--stage', type=int, required=True)
    parser.add_argument('--first', type=int, default=1)
    parser.add_argument('--last', type=int, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = aggregate(args.trace, args.stage, args.first, args.last)
    args.output.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps(result, indent=2))
