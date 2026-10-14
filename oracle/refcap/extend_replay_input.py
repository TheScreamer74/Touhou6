#!/usr/bin/env python3
"""Write a new immutable replay whose final held input lasts longer.

Only a last zero-input record moves; earlier keyboard records and stage-start
state remain byte-identical. This prevents an estimated native completion time
from releasing Shoot/Skip before the genuine executable reaches its terminal.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct

from merge_replays import decode


def extend(source, output, release_frame, expected_frames):
    header, stages = decode(source / 'input.rpy')
    if len(stages) != 1:
        raise ValueError('only a single-stage replay can be extended')
    stage, data = next(iter(stages.items()))
    records = list(struct.iter_unpack('<iHH', data[16:]))
    if len(records) < 3 or records[-1][0] != 9999999 or records[-2][1] != 0:
        raise ValueError('expected a final input release followed by the sentinel')
    if not records[-3][1] or not records[-2][0] < expected_frames < release_frame < 9999999:
        raise ValueError('new endpoint must follow the old release and precede the sentinel')
    raw = header + data[:16]
    struct.pack_into('<7I', raw, 52, *(80 if index == stage else 0 for index in range(7)))
    previous_release = records[-2][0]
    records[-2] = (release_frame, 0, records[-2][2])
    for record in records:
        raw.extend(struct.pack('<iHH', *record))
    struct.pack_into('<I', raw, 8, (sum(raw[14:]) + 0x3f000318) & 0xffffffff)
    key = raw[14]
    for index in range(15, len(raw)):
        raw[index] = (raw[index] + key) & 255
        key = (key + 7) & 255
    output.mkdir(parents=True, exist_ok=True)
    if (output / 'input.rpy').exists():
        raise ValueError('refusing to replace immutable replay input')
    (output / 'input.rpy').write_bytes(raw)
    provenance = json.loads((source / 'input-provenance.json').read_text())
    provenance.update(id=output.name, generator_frames=expected_frames,
        source_input_sha256=hashlib.sha256((source / 'input.rpy').read_bytes()).hexdigest(),
        input_extension={'old_release_frame': previous_release, 'new_release_frame': release_frame,
            'held_buttons': records[-3][1],
            'reason': 'Old native completion estimate released input before original MSG terminal; earlier records and stage header are preserved.'})
    (output / 'input-provenance.json').write_text(json.dumps(provenance, indent=2)+'\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('source', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--release-frame', required=True, type=int)
    parser.add_argument('--expected-frames', required=True, type=int)
    args = parser.parse_args()
    extend(args.source, args.output, args.release_frame, args.expected_frames)
