#!/usr/bin/env python3
"""Reject incomplete, reordered, or stale audio parity evidence."""
import json
from pathlib import Path
import tempfile
import unittest

from verify_audio_reference import (compare_batch, compare_events, digest,
                                    legal_case_ids, verify_case_receipts,
                                    verify_original_collection, verify_current_asset_directory)


class AudioEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.case = self.root / 'replay-c0-d0-s1'
        (self.case / 'original').mkdir(parents=True)
        (self.case / 'port-final').mkdir()
        (self.case / 'input.rpy').write_bytes(b'immutable input fixture')
        self.binary = self.root / 'pin/th06'
        self.binary.parent.mkdir()
        self.binary.write_bytes(b'immutable executable fixture')
        self.build = {'binary_sha256': digest(self.binary), 'runtime_files_sha256': {},
                      'runtime_source_sha256': '44136fa355b3678a1146ad16f7e8649e94fb4fc21fe77e8310c060f61caaff8a'}
        self.write(self.binary.with_name('build-source.json'), self.build)
        self.reference = self.case / 'original/audio.csv'
        self.port = self.case / 'port-final/audio.csv'
        for path in (self.reference, self.port):
            path.write_text('present,frame,stage,event,value,detail\n'
                            '10,-1,1,BGM_REQUEST,th06_02.mid,\n'
                            '11,1,1,SFX_REQUEST,10,\n'
                            '12,2,1,SFX_REQUEST,12,\n')
        original = {'id': self.case.name, 'status': 'completed-process', 'process_returncode': 0,
                    'input_sha256': digest(self.case / 'input.rpy'), 'proxy_sha256': 'verified proxy',
                    'exe_sha256': '7f38496b31b8625196900a69cd1bfed243cab1f9612073e7881dc354b876fd39'}
        self.write(self.case / 'original/execution.json', original)
        self.write(self.case / 'original/terminal.json', {'stage': 1, 'frame': 2})
        (self.case / 'original/replay-state.csv').write_text('present,frame,stage\n11,1,1\n12,2,1\n')
        (self.case / 'original/pools.jsonl').write_text('{"stage":1,"frame":1}\n{"stage":1,"frame":2}\n')
        self.receipt = {'id': self.case.name, 'returncode': 0, 'input_sha256': original['input_sha256'],
                        'reference_execution_sha256': digest(self.case / 'original/execution.json'),
                        'reference_proxy_sha256': original['proxy_sha256'],
                        'command': [str(self.binary)], 'binary_sha256': digest(self.binary),
                        'build_provenance': self.build, 'outputs_sha256': {'audio.csv': digest(self.port)}}
        self.write(self.case / 'port-final/execution.json', self.receipt)

    @staticmethod
    def write(path, data):
        path.write_text(json.dumps(data) + '\n')

    def test_complete_single_case_fixture_passes(self):
        self.assertTrue(compare_batch(self.root, 1)['passed'])

    def test_diagnostic_directory_keeps_provenance_checks(self):
        (self.case / 'port-final').rename(self.case / 'port-diagnostic')
        self.assertTrue(compare_batch(self.root, 1, 'port-diagnostic')['passed'])
        with self.assertRaisesRegex(ValueError, 'single path component'):
            verify_case_receipts(self.case, {}, '../port-diagnostic')

    def test_modified_audio_is_rejected_even_if_event_values_match(self):
        self.port.write_text(self.port.read_text() + '\n')
        with self.assertRaisesRegex(ValueError, 'not bound'):
            verify_case_receipts(self.case, {})

    def test_changed_input_and_changed_binary_are_rejected(self):
        for path in (self.case / 'input.rpy', self.binary):
            before = path.read_bytes()
            path.write_bytes(before + b'changed')
            self.assertFalse(compare_batch(self.root, 1)['passed'])
            path.write_bytes(before)

    def test_failed_process_cannot_certify_matching_audio(self):
        self.receipt['returncode'] = 1
        self.write(self.case / 'port-final/execution.json', self.receipt)
        self.assertFalse(compare_batch(self.root, 1)['passed'])

    def test_incomplete_capture_cannot_certify_matching_audio(self):
        self.receipt['status'] = 'incomplete-output'
        self.write(self.case / 'port-final/execution.json', self.receipt)
        self.assertFalse(compare_batch(self.root, 1)['passed'])

    def test_original_audio_must_match_remote_collection_attestation(self):
        files = {'execution.json': self.case / 'original/execution.json',
                 'input.rpy': self.case / 'input.rpy',
                 'audio.csv': self.reference,
                 'terminal.json': self.case / 'original/terminal.json',
                 'replay-state.csv': self.case / 'original/replay-state.csv',
                 'pools.jsonl': self.case / 'original/pools.jsonl'}
        attested = {'sha256': {name: digest(path) for name, path in files.items()}}
        verify_original_collection(self.case, attested)
        self.reference.write_text(self.reference.read_text() + '\n')
        with self.assertRaisesRegex(ValueError, 'attested remote collection'):
            verify_original_collection(self.case, attested)

    def test_removed_original_state_or_pool_line_fails_attestation(self):
        paths = {'execution.json': self.case / 'original/execution.json',
                 'input.rpy': self.case / 'input.rpy',
                 **{name: self.case / 'original' / name for name in
                    ('audio.csv','terminal.json','replay-state.csv','pools.jsonl')}}
        attested = {'sha256': {name: digest(path) for name, path in paths.items()}}
        for name in ('replay-state.csv', 'pools.jsonl'):
            path = paths[name]
            before = path.read_bytes()
            path.write_bytes(before.rsplit(b'\n', 2)[0] + b'\n')
            with self.assertRaisesRegex(ValueError, 'attested remote collection'):
                verify_original_collection(self.case, attested)
            path.write_bytes(before)

    def test_current_asset_snapshot_detects_changed_native_assets(self):
        game = self.root / 'game'
        (game / 'bgm').mkdir(parents=True)
        manifest = {'archives': {}, 'original_wav_snapshot': {}}
        for category, names, folder in (
                ('archives', [stem + '.DAT' for stem in ('CM','ED','IN','MD','ST','TL')], game),
                ('original_wav_snapshot', [f'th06_{number:02}.wav' for number in range(1,18)], game/'bgm')):
            for name in names:
                path = folder / name
                path.write_bytes(name.encode())
                manifest[category][name] = {'sha256': digest(path), 'bytes': path.stat().st_size}
        verify_current_asset_directory(game, manifest, {})
        (game / 'ST.DAT').write_bytes(b'changed native archive')
        with self.assertRaisesRegex(ValueError, 'differs from attested snapshot'):
            verify_current_asset_directory(game, manifest, {})

    def test_96_directories_with_an_illegal_replacement_are_rejected(self):
        names = legal_case_ids() - {self.case.name, 'replay-c3-d4-s7'}
        for name in names | {'replay-c9-d4-s7'}:
            (self.root / name).mkdir()
        result = compare_batch(self.root, 96)
        self.assertFalse(result['passed'])
        self.assertTrue(any('Incomplete legal96 matrix' in error for error in result['errors']))

    def test_event_order_is_significant(self):
        self.reference.write_text('present,frame,stage,event,value,detail\n'
                                  '1,1,1,SFX_REQUEST,10,\n1,1,1,SFX_REQUEST,12,\n')
        self.port.write_text('present,frame,stage,event,value,detail\n'
                             '1,1,1,SFX_REQUEST,12,\n1,1,1,SFX_REQUEST,10,\n')
        self.assertFalse(compare_events(self.reference, self.port, ['SFX_REQUEST'], 1, 1)['passed'])

    def test_diagnostics_are_bounded_but_all_differences_are_counted(self):
        header = 'present,frame,stage,event,value,detail\n'
        self.reference.write_text(header + ''.join(f'{i},{i},1,SFX_REQUEST,10,\n' for i in range(100)))
        self.port.write_text(header + ''.join(f'{i},{i},1,SFX_REQUEST,12,\n' for i in range(100)))
        result = compare_events(self.reference, self.port, ['SFX_REQUEST'], 0, 99)
        self.assertEqual(result['different_frame_event_groups'], 100)
        self.assertEqual(len(result['first_differences']), 30)


if __name__ == '__main__':
    unittest.main()
