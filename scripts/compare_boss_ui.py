#!/usr/bin/env python3
"""Compare read-only original and port GUI states at post-calculation IDs."""
import argparse
import csv
import json
import math
from pathlib import Path


FIELDS = ('boss_present', 'health_bar_state', 'bar_target', 'bar_display',
          'opacity', 'lives', 'seconds', 'last_draw_seconds', 'dialogue')


def load(path, stage, first, last):
    rows = {}
    with path.open() as source:
        reader = csv.DictReader(source)
        if not {'stage', 'frame', *FIELDS}.issubset(reader.fieldnames or ()):
            raise ValueError(f'Missing GUI columns in {path}')
        for row in reader:
            key = int(row['stage']), int(row['frame'])
            if key[0] != stage or not first <= key[1] <= last:
                continue
            if key in rows:
                raise ValueError(f'Duplicate GUI checkpoint {key} in {path}')
            values = {field: float(row[field]) for field in FIELDS}
            if not all(math.isfinite(value) for value in values.values()):
                raise ValueError(f'Non-finite GUI checkpoint {key} in {path}')
            rows[key] = values
    if not rows:
        raise ValueError(f'No GUI checkpoints in {path}')
    return rows


def compare(original, port, stage, first, last):
    reference = load(original, stage, first, last)
    observed = load(port, stage, first, last)
    missing = sorted(set(reference) - set(observed))
    extra = sorted(set(observed) - set(reference))
    differences = []
    transitions = []
    previous = None
    for key, expected in sorted(reference.items()):
        state = expected['boss_present'], expected['health_bar_state']
        if state != previous:
            transitions.append({'stage': key[0], 'frame': key[1], **expected})
            previous = state
        actual = observed.get(key)
        if actual is None:
            continue
        for field in FIELDS:
            tolerance = 0.000002 if field in ('bar_target', 'bar_display') else 0
            if abs(expected[field] - actual[field]) > tolerance:
                differences.append({'stage': key[0], 'frame': key[1], 'field': field,
                                    'original': expected[field], 'port': actual[field]})
    return {'original': str(original.resolve()), 'port': str(port.resolve()),
            'clock': 'post-calculation replay frame, without temporal alignment',
            'stage': stage, 'first': first, 'last': last, 'fields': list(FIELDS),
            'float_tolerance': 0.000002, 'original_checkpoints': len(reference),
            'port_checkpoints': len(observed), 'missing': missing, 'extra': extra,
            'differences': differences, 'original_transitions': transitions,
            'passed': not (missing or extra or differences)}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('original', type=Path)
    parser.add_argument('port', type=Path)
    parser.add_argument('--stage', type=int, required=True)
    parser.add_argument('--first', type=int, default=1)
    parser.add_argument('--last', type=int, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = compare(args.original, args.port, args.stage, args.first, args.last)
    args.output.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({key: result[key] for key in
                      ('original_checkpoints', 'port_checkpoints', 'passed')}, indent=2))
    raise SystemExit(0 if result['passed'] else 1)
