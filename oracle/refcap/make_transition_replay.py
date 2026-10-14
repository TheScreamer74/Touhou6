#!/usr/bin/env python3
"""Make an immutable, original-format replay for deaths and bombs, without god.

The template supplies an ordinary stage-start header only. Gameplay state is
never copied from a running process or patched. Both engines execute the same
fixed keyboard bits, with the original replay checksum and sentinel.
"""
import argparse
import hashlib
import json
from pathlib import Path
import struct


def make(template, output, bombs=(900, 1800)):
    encoded = template.read_bytes()
    raw = bytearray(encoded)
    key = raw[14]
    for i in range(15, len(raw)):
        raw[i] = (raw[i] - key) & 255
        key = (key + 7) & 255
    if raw[:4] != b'T6RP' or (sum(raw[14:]) + 0x3f000318) & 0xffffffff != struct.unpack_from('<I', raw, 8)[0]:
        raise ValueError('invalid replay template')
    offsets = struct.unpack_from('<7I', raw, 52)
    stage = next(i for i, at in enumerate(offsets) if at)
    at = offsets[stage]
    data = raw[:80] + raw[at:at + 16]
    # A legal stage-start stock, copied from the template. No invulnerability.
    struct.pack_into('<7I', data, 52, *(80 if i == stage else 0 for i in range(7)))
    if any(frame <= 0 or frame >= 29999 for frame in bombs) or len(set(bombs)) != len(bombs):
        raise ValueError('bomb frames must be distinct and inside1..29998')
    if any(abs(a-b)<=1 for a in bombs for b in bombs if a!=b):
        raise ValueError('bomb pulses must have at least one released input between them')
    inputs = [(0, 1 | 4 | 256)]
    for frame in sorted(bombs):
        inputs.extend(((frame, 1 | 2 | 4 | 256), (frame+1, 1 | 4 | 256)))
    inputs.extend(((30000, 0), (9999999, 0)))
    for frame, buttons in inputs:
        data.extend(struct.pack('<iHH', frame, buttons, 0))
    struct.pack_into('<I', data, 8, (sum(data[14:]) + 0x3f000318) & 0xffffffff)
    key = data[14]
    for i in range(15, len(data)):
        data[i] = (data[i] + key) & 255
        key = (key + 7) & 255
    output.mkdir(parents=True, exist_ok=True)
    if (output / 'input.rpy').exists():
        raise ValueError('refusing to replace immutable input')
    (output / 'input.rpy').write_bytes(data)
    provenance = {'id': output.name, 'kind': 'synthetic-original-format-replay',
                  'god': False, 'stage': stage + 1, 'character': raw[6],
                  'difficulty': raw[7], 'generator_frames': 30000,
                  'expected_checkpoint': 'original-game-over',
                  'template_sha256': hashlib.sha256(encoded).hexdigest(),
                  'inputs': inputs, 'goal': 'bomb activation, deaths, respawn and exhausted stock; replay does not permit Continue'}
    (output / 'input-provenance.json').write_text(json.dumps(provenance, indent=2) + '\n')


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('template', type=Path)
    parser.add_argument('output', type=Path)
    parser.add_argument('--bomb', action='append', type=int, help='Replay input frame for a one-frame bomb pulse; repeat for multiple pulses')
    args = parser.parse_args()
    make(args.template, args.output, args.bomb if args.bomb is not None else (900, 1800))
