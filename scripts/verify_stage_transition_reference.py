#!/usr/bin/env python3
"""Compare a genuine replay stage terminal plus the next four calculations.

The matrix comparator stays single-stage and fail-closed. This wrapper requires
the complete first-stage evidence and exactly frames1..4 of the following stage,
including all four object-pool checkpoints. Audio requests use Present time,
so replay clock resets cannot hide an early scene stop/play request.
"""
import argparse
import csv
import json
from pathlib import Path
import tempfile

from verify_scenario_reference import Evidence, compare, sha

EVENTS = {'SFX_REQUEST', 'BGM_REQUEST', 'BGM_STOP_REQUEST', 'BGM_FADE'}


def split(source, destination, stages, pools=False):
    handles = {stage: (destination / f'{stage}.{ "jsonl" if pools else "csv"}').open('w') for stage in stages}
    keys = {stage: [] for stage in stages}
    errors = Evidence(100)
    try:
        with source.open() as data:
            if pools:
                rows = (json.loads(line) for line in data)
                writers = None
            else:
                rows = csv.DictReader(data)
                writers = {stage: csv.DictWriter(handle, fieldnames=rows.fieldnames) for stage, handle in handles.items()}
                for writer in writers.values():
                    writer.writeheader()
            previous = None
            for row in rows:
                stage, frame = int(row['stage']), int(row['frame'])
                key = stage, frame
                if previous is not None and key <= previous:
                    errors.append(f'{source}: duplicate/out-of-order observation {key}')
                previous = key
                if stage not in stages:
                    errors.append(f'{source}: unexpected stage {stage}')
                    continue
                keys[stage].append(frame)
                if pools:
                    handles[stage].write(json.dumps(row)+'\n')
                else:
                    writers[stage].writerow(row)
    finally:
        for handle in handles.values():
            handle.close()
    return keys, errors


def audio_requests(path, start, end, offset):
    events = []
    with path.open() as data:
        for row in csv.DictReader(data):
            present = int(row['present'])
            if row['event'] not in EVENTS or not start <= present <= end:
                continue
            value = row['value']
            if row['event'] == 'BGM_REQUEST':
                value = Path(value.replace('\\', '/')).stem.lower()
            elif row['event'] == 'BGM_FADE':
                value = float(value)
            events.append((present-offset, int(row['stage']), row['event'], value))
    return events


def verify(original, port):
    terminal = json.loads((original/'terminal.json').read_text())
    first, following = terminal['stage'], terminal['stage']+1
    if first not in range(1, 6):
        raise ValueError('expected a stage1..5 transition')
    errors, results = Evidence(100), []
    files = {'reference_states': original/'replay-state.csv', 'port_states': port/'replay-state.csv',
             'reference_pools': original/'pools.jsonl', 'port_pools': port/'pools.jsonl',
             'reference_audio': original/'audio.csv', 'port_audio': port/'audio.csv',
             'reference_terminal': original/'terminal.json',
             'port_terminal': port/'replay-state.csv.terminal.jsonl'}
    with tempfile.TemporaryDirectory(prefix='th06-transition-') as temporary:
        root = Path(temporary)
        paths = {}
        for name in ('reference_states', 'port_states', 'reference_pools', 'port_pools'):
            directory = root/name
            directory.mkdir()
            keys, problems = split(files[name], directory, (first, following), name.endswith('pools'))
            errors.extend(problems)
            if keys[following] != [1, 2, 3, 4]:
                errors.append(f'{name}: following stage requires exactly frames1,2,3,4')
            paths[name] = directory
        for stage in (first, following):
            results.append(compare(paths['reference_states']/f'{stage}.csv',
                paths['port_states']/f'{stage}.csv', paths['reference_pools']/f'{stage}.jsonl',
                paths['port_pools']/f'{stage}.jsonl', stage,
                files['reference_terminal'] if stage == first else None,
                files['port_terminal'] if stage == first else None,
                4 if stage == following else None))
    starts, reference_end = [], None
    with files['reference_states'].open() as data:
        for row in csv.DictReader(data):
            reference_end = int(row['present'])
            if int(row['stage']) == first and int(row['frame']) == 1:
                starts.append(reference_end)
    if len(starts) != 1:
        raise ValueError('reference first replay-frame Present anchor is missing/duplicated')
    reference_start = starts[0]
    # Native diagnostics count Present from0 at the direct replay entry. The
    # original reaches that same input after its genuine Replay menu.
    with files['port_audio'].open() as data:
        starts = {int(row['present']) for row in csv.DictReader(data)
                  if int(row['stage']) == first and int(row['frame']) == 1 and row['event']=='SFX_REQUEST'}
    if len(starts) != 1:
        raise ValueError('port first replay-frame audio Present anchor is missing/duplicated')
    port_start = starts.pop()
    span = reference_end-reference_start
    expected = audio_requests(files['reference_audio'], reference_start, reference_end, reference_start)
    actual = audio_requests(files['port_audio'], port_start, port_start+span, port_start)
    audio_differences = Evidence(40)
    for index, (a, b) in enumerate(zip(expected, actual)):
        if a != b:
            audio_differences.append({'index': index, 'reference': a, 'port': b})
    if len(expected) != len(actual):
        errors.append(f'audio request count differs: {len(expected)} != {len(actual)}')
    if not expected or not any(row[1] == following and row[2] == 'BGM_REQUEST' for row in expected):
        errors.append('original did not request next-stage BGM within transition window')
    return {'passed': not errors and not audio_differences and all(row['passed'] for row in results),
        'scope': 'complete replay stage plus next-stage frames1..4; ordered SFX/BGM/fade/stop requests on Present clock; rendering and backend playback measured separately',
        'stages': results, 'audio_requests': len(expected), 'port_audio_requests': len(actual),
        'audio_difference_count': len(audio_differences), 'audio_differences': audio_differences[:40],
        'reference_start_present': reference_start, 'port_start_present': port_start,
        'present_span': span, 'errors': errors[:100], 'error_count': len(errors),
        'evidence_sha256': {name: sha(path) for name,path in files.items()}}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('original', type=Path)
    parser.add_argument('port', type=Path)
    parser.add_argument('--output', type=Path, required=True)
    args = parser.parse_args()
    result = verify(args.original, args.port)
    args.output.write_text(json.dumps(result, indent=2)+'\n')
    print(json.dumps({key:result[key] for key in ('passed','audio_requests','audio_difference_count','error_count')}))
    raise SystemExit(0 if result['passed'] else 1)
