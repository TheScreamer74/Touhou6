#!/usr/bin/env python3
"""Run immutable original T6RP inputs against a pinned native port binary.

Requires genuine original terminal and pool evidence. Every case has private
storage and audio/scalar/pool outputs. This proves simulation only; image and
audio parity require their separate strict comparators.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
import hashlib
import json
import os
from pathlib import Path
import subprocess
import time

from verify_scenario_reference import compare
from verify_audio_reference import load_original_collection, verify_original_collection

NATIVE_OUTPUTS = ('audio.csv', 'replay-state.csv', 'pools.jsonl',
                  'replay-state.csv.terminal.jsonl')


def sha(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def requires_final_provenance(args):
    return args.require_matrix or args.name == 'port-final' or args.name.startswith('port-final-')


def load_build_provenance(args, binary_hash):
    receipt = args.binary.with_name('build-source.json')
    build = json.loads(receipt.read_text()) if receipt.exists() else None
    if requires_final_provenance(args) and build is None:
        raise ValueError('matrix/port-final requires build-source.json from scripts/pin_port_build.py')
    if build and build['binary_sha256'] != binary_hash:
        raise ValueError('build provenance does not match the pinned executable')
    if build:
        fingerprint = hashlib.sha256(json.dumps(build['runtime_files_sha256'], sort_keys=True,
                                               separators=(',', ':')).encode()).hexdigest()
        if build.get('runtime_source_sha256') != fingerprint:
            raise ValueError('invalid source manifest fingerprint')
    return build


def verify_native_outputs(out, execution):
    for name in NATIVE_OUTPUTS:
        if execution.get('outputs_sha256', {}).get(name) != sha(out / name):
            raise ValueError(f'native output differs from its execution receipt: {name}')


def prepare_original_collection(args):
    required = requires_final_provenance(args)
    explicit = getattr(args, 'original_attestation', None)
    args.original_collection = {}
    args.original_collection_provenance = None
    args.original_collection_path = None
    if not required and explicit is None:
        return
    if explicit is None:
        parents = {case.resolve().parent for case in args.cases}
        if len(parents) != 1:
            raise ValueError('provide --original-attestation when case directories have different parents')
        explicit = parents.pop() / 'original-audio-collection-attestation.json'
    args.original_collection_path = explicit.resolve()
    args.original_collection, args.original_collection_provenance = load_original_collection(
        args.original_collection_path)


def verify_reference_collection(case, args):
    provenance = getattr(args, 'original_collection_provenance', None)
    if provenance is None:
        if requires_final_provenance(args):
            raise ValueError('matrix/port-final requires the original audio/state/pool collection attestation')
        return None
    if sha(args.original_collection_path) != provenance['sha256']:
        raise ValueError('original collection attestation changed after preflight')
    attested = args.original_collection.get(case.name)
    if attested is None:
        raise ValueError(f'{case.name}: case is absent from original collection attestation')
    verify_original_collection(case, attested)
    return attested['sha256']


def run_case(case, args, binary_hash):
    original = case / 'original'
    reference_execution = json.loads((original / 'execution.json').read_text())
    if (reference_execution.get('status') != 'completed-process' or
            reference_execution.get('process_returncode') != 0 or
            reference_execution.get('id') != case.name or
            reference_execution.get('exe_sha256') != '7f38496b31b8625196900a69cd1bfed243cab1f9612073e7881dc354b876fd39' or
            reference_execution.get('input_sha256') != sha(case / 'input.rpy')):
        raise ValueError(f'{case.name}: incomplete or stale original execution receipt')
    reference_outputs = verify_reference_collection(case, args)
    terminal = json.loads((original / 'terminal.json').read_text())
    provenance = json.loads((case / 'input-provenance.json').read_text())
    out = case / args.name
    out.mkdir(exist_ok=True)
    receipt = out / 'execution.json'
    reusable = receipt.exists() and not args.rerun
    if reusable:
        old = json.loads(receipt.read_text())
        if old['binary_sha256'] != binary_hash or old['input_sha256'] != sha(case / 'input.rpy'):
            raise ValueError(f'{case.name}: existing receipt uses another binary/input; select --rerun')
        if reference_outputs is not None:
            if old.get('original_collection_attestation_sha256') != args.original_collection_provenance['sha256']:
                raise ValueError(f'{case.name}: existing receipt lacks or uses another original collection attestation; select --rerun')
        verify_native_outputs(out, old)
        reusable = old.get('returncode') == 0
    if not reusable:
        storage = case / (args.name + '-game')
        storage.mkdir(exist_ok=True)
        for source in args.game.iterdir():
            if ((source.suffix.upper() == '.DAT' and source.name.lower() != 'score.dat') or
                    source.name in ('bgm', 'gm.sf2', 'msgothic.ttc', 'msgothic.ttf')):
                target = storage / source.name
                if not target.exists():
                    target.symlink_to(source.resolve(), target_is_directory=source.is_dir())
        (storage / 'th06-options.bin').write_bytes(bytes([1, 2, 3, 0, 1, 1, 1, 1]))
        with (original / 'pools.jsonl').open() as pools:
            checkpoints = sorted({json.loads(line)['frame'] for line in pools})
        command = [str(args.binary), '--game-dir', str(storage), '--replay', str(case / 'input.rpy'),
                   '--frames', str(terminal['frame'] + 200), '--replay-dump', str(out / 'replay-state.csv'),
                   '--replay-pools', str(out / 'pools.jsonl'), '--capture-frames', ','.join(map(str, checkpoints))]
        if provenance.get('god', False):
            command.append('--god')
        env = dict(os.environ, TH06_AUDIO_TRACE=str(out / 'audio.csv'))
        for name in ('TH06_GOD', 'TH06_NO_DIALOGUE', 'TH06_BOMB', 'TH06_NOSHOOT',
                     'TH06_FONT', 'TH06_SOUNDFONT'):
            env.pop(name, None)
        begin = time.monotonic()
        with (out / 'run.log').open('w') as log:
            process = subprocess.run(command, env=env, stdout=log, stderr=subprocess.STDOUT,
                                     timeout=args.timeout)
        # A shared target/release path is deliberately forbidden: never certify
        # a hash that was silently rebuilt by a concurrent worker.
        if sha(args.binary) != binary_hash:
            raise ValueError('pinned executable changed during execution')
        data = {'id': case.name, 'binary_sha256': binary_hash, 'input_sha256': sha(case / 'input.rpy'),
                'god': provenance.get('god', False), 'returncode': process.returncode,
                'elapsed_seconds': time.monotonic() - begin, 'command': command,
                'build_provenance': args.build_provenance,
                'harness_sha256': args.harness_sha256,
                'reference_execution_sha256': sha(original / 'execution.json'),
                'reference_proxy_sha256': reference_execution['proxy_sha256'],
                'original_collection_attestation_sha256': (
                    args.original_collection_provenance['sha256'] if reference_outputs is not None else None),
                'original_collection_attestation': args.original_collection_provenance,
                'reference_current_collection_outputs_sha256': reference_outputs,
                'outputs_sha256': {name: sha(out / name) for name in NATIVE_OUTPUTS}}
        receipt.write_text(json.dumps(data, indent=2) + '\n')
        if process.returncode != 0:
            raise ValueError(f'{case.name}: port process failed with exit {process.returncode}')
    # Hash every current original member again immediately before comparison.
    # This remains collection provenance; historical execution receipts stay unchanged.
    verify_reference_collection(case, args)
    verify_native_outputs(out, json.loads(receipt.read_text()))
    result = compare(original / 'replay-state.csv', out / 'replay-state.csv',
                     original / 'pools.jsonl', out / 'pools.jsonl', terminal['stage'],
                     original / 'terminal.json', out / 'replay-state.csv.terminal.jsonl', None)
    result['id'] = case.name
    result['binary_sha256'] = binary_hash
    result['input_sha256'] = sha(case / 'input.rpy')
    result['original_collection_attestation'] = args.original_collection_provenance
    result['reference_current_collection_outputs_sha256'] = reference_outputs
    (out / 'comparison.json').write_text(json.dumps(result, indent=2) + '\n')
    return {key: result[key] for key in ('id', 'passed', 'compared_observations',
        'compared_pool_checkpoints', 'first_difference_frame', 'scalar_difference_count',
        'pool_difference_count', 'error_count')}


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('cases', nargs='+', type=Path)
    parser.add_argument('--binary', required=True, type=Path, help='Private pinned executable, not target/release')
    parser.add_argument('--game', required=True, type=Path)
    parser.add_argument('--name', default='port-working')
    parser.add_argument('--jobs', type=int, default=1, help='Concurrent local cases; default1 limits RAM/GPU pressure')
    parser.add_argument('--timeout', type=float, default=300)
    parser.add_argument('--rerun', action='store_true')
    parser.add_argument('--require-matrix', action='store_true', help='Require all96 legal shot/difficulty/stage IDs exactly once')
    parser.add_argument('--original-attestation', type=Path,
                        help='Current original audio/state/pool attestation; required automatically for matrix/port-final')
    parser.add_argument('--output', type=Path, help='Write aggregate execution and parity receipt')
    args = parser.parse_args()
    if Path(args.name).name != args.name or args.name in ('.', '..'):
        parser.error('--name must be a single directory component')
    args.binary, args.game = args.binary.resolve(), args.game.resolve()
    if args.require_matrix:
        expected = {f'replay-c{character}-d{difficulty}-s{stage}' for character in range(4)
                    for difficulty in range(5)
                    for stage in (range(1,6) if difficulty==0 else (range(7,8) if difficulty==4 else range(1,7)))}
        names = [case.name for case in args.cases]
        if len(names) != len(expected) or set(names) != expected:
            parser.error(f'matrix requires96 distinct legal IDs; missing={sorted(expected-set(names))}, extra={sorted(set(names)-expected)}')
    prepare_original_collection(args)
    if 'target/release' in str(args.binary):
        parser.error('copy the binary to an immutable private hash directory first')
    binary_hash = sha(args.binary)
    try:
        args.build_provenance = load_build_provenance(args, binary_hash)
    except (OSError, ValueError, KeyError) as error:
        parser.error(str(error))
    root = Path(__file__).resolve().parent.parent
    harness_files = [root / 'scripts' / name for name in
                     ('run_port_references.py', 'verify_scenario_reference.py',
                      'verify_audio_reference.py', 'attest_original_audio_reference.py')]
    harness_files += sorted(p for p in (root / 'oracle' / 'refcap').iterdir()
                            if p.suffix in ('.py', '.c', '.h', '.def'))
    args.harness_sha256 = {str(p.relative_to(root)): sha(p) for p in harness_files}
    results = []
    with ThreadPoolExecutor(max_workers=args.jobs) as executor:
        pending = [executor.submit(run_case, case.resolve(), args, binary_hash) for case in args.cases]
        for future in as_completed(pending):
            result = future.result()
            results.append(result)
            print(json.dumps(result), flush=True)
    if args.output:
        receipt = {'passed': bool(results) and all(result['passed'] for result in results),
                   'required_matrix': args.require_matrix, 'case_count': len(results),
                   'compared_observations': sum(result['compared_observations'] for result in results),
                   'compared_pool_checkpoints': sum(result['compared_pool_checkpoints'] for result in results),
                   'binary_sha256': binary_hash, 'build_provenance': args.build_provenance,
                   'harness_sha256': args.harness_sha256, 'cases': sorted(results,key=lambda result:result['id'])}
        receipt['original_collection_attestation'] = args.original_collection_provenance
        args.output.write_text(json.dumps(receipt, indent=2)+'\n')
    if any(not result['passed'] for result in results):
        raise SystemExit(1)
