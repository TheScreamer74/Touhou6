#!/usr/bin/env python3
"""Fail-closed comparison of natural menu/result navigation observations.

Original Present p and port completed update p+1 consume the same keys.
Navigation requires the original MainMenu scene, preventing stale global menu
memory from being accepted after a transition. Result rows require a live
ResultScreen callback in the original capture proxy.
"""
import argparse
import csv
import hashlib
import json
from pathlib import Path


def digest(path):
    result = hashlib.sha256()
    with path.open('rb') as source:
        for block in iter(lambda: source.read(1024 * 1024), b''):
            result.update(block)
    return result.hexdigest()


def indexed(path, key, offset=0):
    result = {}
    with path.open() as source:
        for row in csv.DictReader(source):
            frame = int(row[key]) + offset
            if frame in result:
                raise ValueError(f'Duplicate {key} {frame}: {path}')
            result[frame] = row
    if not result:
        raise ValueError(f'Empty trace: {path}')
    return result


def compare(reference, actual, kind, start, end):
    if start < 0 or end < start:
        raise ValueError('Invalid required frame interval')
    expected = indexed(reference, 'present')
    observed = indexed(actual, 'update', -1)
    fields = ('menu_state', 'cursor', 'timer') if kind == 'navigation' else ('state', 'timer', 'cursor', 'cheat_step', 'progress')
    differences, missing, compared = {}, [], 0
    for frame in range(start, end + 1):
        if frame not in expected or frame not in observed:
            missing.append(frame)
            continue
        want, got = expected[frame], observed[frame]
        if kind == 'navigation' and want['supervisor_state'] != '1':
            raise ValueError(f'Original no longer in MainMenu at Present {frame}')
        compared += 1
        for field in fields:
            left, right = want[field], got[field]
            if field == 'progress':
                if len(bytes.fromhex(left)) != 40 or len(bytes.fromhex(right)) != 40:
                    raise ValueError('Expected all40 original CLRD bytes')
            else:
                left, right = int(left), int(right)
            if left != right:
                detail = differences.setdefault(field, {'first_present': frame, 'expected': left, 'actual': right, 'count': 0})
                detail['count'] += 1
    return {'kind': kind, 'required_present_interval': [start, end],
            'required_observations': end - start + 1, 'compared_observations': compared,
            'fields': fields, 'missing_frames': missing, 'differences': differences,
            'reference_sha256': digest(reference), 'port_sha256': digest(actual),
            'passed': compared == end - start + 1 and not differences,
            'scope': 'Declared natural menu navigation/state; pixels, raw controller hardware and physical audio require separate evidence.'}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('reference', type=Path)
    parser.add_argument('actual', type=Path)
    parser.add_argument('--kind', choices=('navigation', 'results'), required=True)
    parser.add_argument('--start', type=int, required=True)
    parser.add_argument('--end', type=int, required=True)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = compare(args.reference, args.actual, args.kind, args.start, args.end)
    args.output.write_text(json.dumps(result, indent=2) + '\n')
    print(json.dumps({'passed': result['passed'], 'compared_observations': result['compared_observations'], 'differences': result['differences']}))
    raise SystemExit(0 if result['passed'] else 1)
