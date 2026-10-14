#!/usr/bin/env python3
"""Strict original/port player-state and timer comparison for unpaused replays."""
import argparse
import csv
import hashlib
import json
from pathlib import Path

FIELDS = ('player_state', 'respawn', 'grace', 'invuln', 'bomb', 'bomb_timer', 'retry', 'game_menu')


def read(path):
    rows, errors = {}, []
    with path.open() as source:
        for row in csv.DictReader(source):
            key = int(row['stage']), int(row['frame'])
            if key in rows:
                errors.append(f'duplicate lifecycle observation {key}')
            rows[key] = {name: int(row[name]) for name in FIELDS}
    if not rows:
        errors.append('empty lifecycle trace')
    return rows, errors


def compare(reference, actual, terminal):
    expected, errors = read(reference)
    observed, other_errors = read(actual)
    errors.extend(other_errors)
    endpoint = json.loads(terminal.read_text())
    key = endpoint['stage'], endpoint['frame']
    if key not in expected or key not in observed:
        errors.append('lifecycle trace is truncated before required original terminal')
    differences, compared = [], 0
    for key, row in sorted(expected.items()):
        if key not in observed:
            errors.append(f'missing port lifecycle observation {key}')
            continue
        compared += 1
        for name in FIELDS:
            if row[name] != observed[key][name]:
                differences.append({'stage': key[0], 'frame': key[1], 'field': name,
                                    'expected': row[name], 'actual': observed[key][name]})
    return {'passed': bool(compared) and not errors and not differences,
            'compared_observations': compared, 'difference_count': len(differences),
            'first_difference': differences[0] if differences else None,
            'differences': differences[:40], 'error_count': len(errors), 'errors': errors[:100],
            'reference_sha256': hashlib.sha256(reference.read_bytes()).hexdigest(),
            'port_sha256': hashlib.sha256(actual.read_bytes()).hexdigest(),
            'terminal_sha256': hashlib.sha256(terminal.read_bytes()).hexdigest()}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('reference', type=Path)
    parser.add_argument('actual', type=Path)
    parser.add_argument('--terminal', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = compare(args.reference, args.actual, args.terminal)
    args.output.write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps({key: result[key] for key in ('passed','compared_observations','difference_count','error_count','first_difference')}))
    raise SystemExit(0 if result['passed'] else 1)
