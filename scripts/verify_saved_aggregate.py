#!/usr/bin/env python3
"""Compare native saves with an original semantic aggregate kept off-host.

The original parser runs beside the real original files and returns metadata
and a canonical semantic hash. This comparator never reconstructs those files.
"""
import argparse
from datetime import datetime
import hashlib
import json
from pathlib import Path
import re
import struct

from verify_saved_reference import read_highscores, read_replay, sha256

ORIGINAL_EXECUTABLE_SHA256 = '7f38496b31b8625196900a69cd1bfed243cab1f9612073e7881dc354b876fd39'


def validate_original(original, parser_path):
    if not isinstance(original, dict):
        raise ValueError('original aggregate must be a JSON object')
    if (original.get('kind') != 'original-score-save-semantic-aggregate' or
            original.get('schema_version') != 1):
        raise ValueError('unsupported original semantic aggregate')
    if original.get('returncode') != 0:
        raise ValueError('original execution was not successful')
    if not (original.get('replay_checksum_valid') is True and
            original.get('score_checksum_valid') is True):
        raise ValueError('original aggregate lacks valid replay/score checksums')
    if original.get('original_executable_sha256') != ORIGINAL_EXECUTABLE_SHA256:
        raise ValueError('original aggregate uses another executable')
    if original.get('parser_sha256') != sha256(parser_path):
        raise ValueError('original aggregate uses another saved-file parser')
    for field in ('execution_sha256', 'parser_sha256', 'replay_sha256', 'score_sha256',
                  'canonical_semantics_sha256'):
        if not isinstance(original.get(field), str) or not re.fullmatch('[0-9a-f]{64}', original[field]):
            raise ValueError(f'invalid original provenance hash: {field}')
    minimum = original.get('minimum_compared_highscore')
    if isinstance(minimum, bool) or not isinstance(minimum, int) or minimum < 0:
        raise ValueError('invalid original highscore threshold')
    if not isinstance(original.get('highscores'), list) or not original['highscores']:
        raise ValueError('original aggregate lacks declared highscore records')
    metadata = original.get('replay_metadata')
    if not isinstance(metadata, dict) or not isinstance(metadata.get('date'), str):
        raise ValueError('original aggregate lacks its exact replay date')
    try:
        datetime.strptime(metadata['date'], '%m/%d/%y')
    except ValueError as error:
        raise ValueError('invalid original replay date') from error
    if 'ignored_inputs_after_first_sentinel' not in original:
        raise ValueError('original aggregate lacks its declared ignored input tail')


def compare(aggregate_path, native, expected_original_sha256=None):
    aggregate_hash = sha256(aggregate_path)
    if expected_original_sha256 is not None and aggregate_hash != expected_original_sha256:
        raise ValueError('original aggregate differs from the expected provenance hash')
    original = json.loads(aggregate_path.read_text())
    repository = Path(__file__).resolve().parent.parent
    validate_original(original, repository / 'scripts/verify_saved_reference.py')
    replay, tails = read_replay(native / 'replay/th6_01.rpy')
    scores = read_highscores(native / 'score.dat', original['minimum_compared_highscore'])
    semantics = {'replay': replay, 'highscores': scores}
    canonical = json.dumps(semantics, sort_keys=True, separators=(',', ':')).encode()
    native_digest = hashlib.sha256(canonical).hexdigest()
    dates_equal = replay['date'] == original['replay_metadata']['date']
    return {
        'passed': bool(scores) and dates_equal and native_digest == original['canonical_semantics_sha256'],
        'original_aggregate_sha256': aggregate_hash,
        'original_execution_sha256': original['execution_sha256'],
        'original_executable_sha256': original['original_executable_sha256'],
        'parser_sha256': original['parser_sha256'],
        'comparator_sha256': sha256(Path(__file__).resolve()),
        'original_date': original['replay_metadata']['date'], 'native_date': replay['date'],
        'dates_identical': dates_equal,
        'canonical_semantics_identical': native_digest == original['canonical_semantics_sha256'],
        'original_canonical_semantics_sha256': original['canonical_semantics_sha256'],
        'native_canonical_semantics_sha256': native_digest,
        'original_sha256': {'replay/th6_01.rpy': original['replay_sha256'], 'score.dat': original['score_sha256']},
        'native_sha256': {name: sha256(native / name) for name in ('replay/th6_01.rpy', 'score.dat')},
        'native_replay_checksum_valid': True, 'native_score_checksum_valid': True,
        'minimum_compared_highscore': original['minimum_compared_highscore'],
        'original_highscores': original['highscores'], 'native_highscores': scores,
        'ignored_inputs_after_first_sentinel': {'original': original['ignored_inputs_after_first_sentinel'], 'native': tails},
        'scope': 'Exact canonical replay metadata including creation date, stage headers and every interpreted input through the first sentinel, plus ordered highscore records at the declared threshold. Both file checksums and boundaries are independently parsed. The original files stay on the server; no original file is reconstructed from the aggregate.'
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('aggregate', type=Path)
    parser.add_argument('native', type=Path)
    parser.add_argument('--expected-original-sha256', help='Bind the original aggregate to its recorded collection hash')
    parser.add_argument('--output', type=Path)
    args = parser.parse_args()
    try:
        result = compare(args.aggregate, args.native, args.expected_original_sha256)
    except (OSError, ValueError, KeyError, TypeError, struct.error) as error:
        result = {'passed': False, 'error': str(error)}
    body = json.dumps(result, indent=2) + '\n'
    if args.output:
        args.output.write_text(body)
    print(body, end='')
    return 0 if result['passed'] else 1


if __name__ == '__main__':
    raise SystemExit(main())
