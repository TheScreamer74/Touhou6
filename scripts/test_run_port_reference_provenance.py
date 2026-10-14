#!/usr/bin/env python3
"""Final comparisons must bind unchanged original and native evidence."""
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from run_port_references import (NATIVE_OUTPUTS, load_build_provenance,
                                 prepare_original_collection, run_case, sha,
                                 verify_reference_collection)
from verify_audio_reference import legal_case_ids


class ReferenceProvenanceTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.case = self.root / 'replay-c0-d0-s1'
        original = self.case / 'original'
        original.mkdir(parents=True)
        (self.case / 'input.rpy').write_bytes(b'input fixture')
        self.write(self.case / 'input-provenance.json', {'god': True})
        self.write(original / 'execution.json', {
            'id': self.case.name, 'status': 'completed-process', 'process_returncode': 0,
            'input_sha256': sha(self.case / 'input.rpy'), 'proxy_sha256': 'verified proxy',
            'exe_sha256': '7f38496b31b8625196900a69cd1bfed243cab1f9612073e7881dc354b876fd39'})
        self.write(original / 'terminal.json', {'stage': 1, 'frame': 2})
        (original / 'replay-state.csv').write_text('present,frame,stage\n10,1,1\n11,2,1\n')
        (original / 'pools.jsonl').write_text('{"stage":1,"frame":1}\n{"stage":1,"frame":2}\n')
        (original / 'audio.csv').write_text('present,frame,stage,event,value\n10,1,1,SFX_REQUEST,10\n')
        members = {'execution.json': original / 'execution.json', 'input.rpy': self.case / 'input.rpy',
                   **{name: original / name for name in
                      ('audio.csv', 'terminal.json', 'replay-state.csv', 'pools.jsonl')}}
        hashes = {name: sha(path) for name, path in members.items()}
        rows = []
        for name in sorted(legal_case_ids()):
            values = hashes if name == self.case.name else dict.fromkeys(hashes, '0' * 64)
            rows.append({'id': name, 'status': 'completed-process', 'process_returncode': 0,
                         'declared_input_sha256': values['input.rpy'], 'sha256': values})
        self.attestation = self.root / 'original-audio-collection-attestation.json'
        self.snapshot = {'kind': 'remote-original-audio-collection-attestation', 'schema_version': 2,
                         'attested_outputs': ['audio.csv','terminal.json','replay-state.csv','pools.jsonl'],
                         'observed_at_utc': '2026-10-13T00:00:00+00:00',
                         'scope': 'Later collection snapshot; original execution receipts unchanged.',
                         'cases': rows, 'errors': [], 'passed': True}
        self.write(self.attestation, self.snapshot)
        binary = self.root / 'pin/th06'
        binary.parent.mkdir()
        binary.write_bytes(b'immutable binary fixture')
        game = self.root / 'game'
        game.mkdir()
        self.args = SimpleNamespace(cases=[self.case], name='port-final', require_matrix=False,
                                    original_attestation=None, binary=binary, game=game,
                                    rerun=False, timeout=1, build_provenance={'binary_sha256': sha(binary)},
                                    harness_sha256={})
        prepare_original_collection(self.args)

    @staticmethod
    def write(path, value):
        path.write_text(json.dumps(value) + '\n')

    def native_outputs(self):
        out = self.case / self.args.name
        out.mkdir(exist_ok=True)
        for name in NATIVE_OUTPUTS:
            (out / name).write_text('native evidence fixture\n')
        return out

    def reusable_receipt(self, bound):
        out = self.native_outputs()
        data = {'binary_sha256': sha(self.args.binary), 'input_sha256': sha(self.case/'input.rpy'),
                'returncode': 0, 'outputs_sha256': {name: sha(out/name) for name in NATIVE_OUTPUTS}}
        if bound:
            data['original_collection_attestation_sha256'] = sha(self.attestation)
        self.write(out / 'execution.json', data)
        return out

    def test_valid_snapshot_binds_all_six_original_members(self):
        self.assertEqual(set(verify_reference_collection(self.case, self.args)), {
            'execution.json','input.rpy','audio.csv','terminal.json','replay-state.csv','pools.jsonl'})

    def test_final_requires_an_attestation(self):
        self.attestation.unlink()
        with self.assertRaises(FileNotFoundError):
            prepare_original_collection(self.args)

    def test_require_matrix_requires_attestation_for_diagnostic_name(self):
        self.args.name = 'port-candidate'
        self.args.require_matrix = True
        self.attestation.unlink()
        with self.assertRaises(FileNotFoundError):
            prepare_original_collection(self.args)

    def test_final_name_with_hash_still_requires_attestation(self):
        self.args.name = 'port-final-01234567'
        self.attestation.unlink()
        with self.assertRaises(FileNotFoundError):
            prepare_original_collection(self.args)

    def test_matrix_and_final_hash_name_require_build_source(self):
        for name, matrix in (('port-candidate', True), ('port-final-01234567', False)):
            with self.subTest(name=name, matrix=matrix):
                self.args.name, self.args.require_matrix = name, matrix
                with self.assertRaisesRegex(ValueError, 'requires build-source.json'):
                    load_build_provenance(self.args, sha(self.args.binary))

    def test_changed_build_source_fingerprint_is_rejected(self):
        self.write(self.args.binary.with_name('build-source.json'), {
            'binary_sha256': sha(self.args.binary), 'runtime_files_sha256': {},
            'runtime_source_sha256': '0' * 64})
        with self.assertRaisesRegex(ValueError, 'source manifest fingerprint'):
            load_build_provenance(self.args, sha(self.args.binary))

    def test_audio_only_attestation_cannot_certify_state_or_pool_evidence(self):
        self.snapshot['schema_version'] = 1
        self.snapshot['attested_outputs'] = ['audio.csv','terminal.json']
        self.write(self.attestation, self.snapshot)
        with self.assertRaisesRegex(ValueError, 'audio/state/pool snapshot'):
            prepare_original_collection(self.args)

    def test_removed_original_line_is_rejected_before_native_execution(self):
        for name in ('replay-state.csv','pools.jsonl'):
            path = self.case / 'original' / name
            before = path.read_bytes()
            path.write_bytes(before.rsplit(b'\n', 2)[0] + b'\n')
            with patch('run_port_references.subprocess.run') as execute, patch('run_port_references.compare') as compare:
                with self.assertRaisesRegex(ValueError, 'attested remote collection'):
                    run_case(self.case, self.args, sha(self.args.binary))
                execute.assert_not_called()
                compare.assert_not_called()
            path.write_bytes(before)

    def test_attestation_cannot_change_after_preflight(self):
        self.attestation.write_text(self.attestation.read_text() + '\n')
        with self.assertRaisesRegex(ValueError, 'changed after preflight'):
            verify_reference_collection(self.case, self.args)

    def test_unbound_old_native_receipt_requires_rerun(self):
        self.reusable_receipt(bound=False)
        with patch('run_port_references.subprocess.run') as execute:
            with self.assertRaisesRegex(ValueError, 'another original collection attestation'):
                run_case(self.case, self.args, sha(self.args.binary))
            execute.assert_not_called()

    def test_changed_reused_native_output_is_rejected(self):
        out = self.reusable_receipt(bound=True)
        (out/'replay-state.csv').write_text('changed native evidence\n')
        with patch('run_port_references.compare') as compare:
            with self.assertRaisesRegex(ValueError, 'native output differs'):
                run_case(self.case, self.args, sha(self.args.binary))
            compare.assert_not_called()

    def test_changed_reused_terminal_is_rejected_before_comparison(self):
        out = self.reusable_receipt(bound=True)
        (out/'replay-state.csv.terminal.jsonl').write_text('changed terminal event\n')
        with patch('run_port_references.compare') as compare:
            with self.assertRaisesRegex(ValueError, 'native output differs.*terminal.jsonl'):
                run_case(self.case, self.args, sha(self.args.binary))
            compare.assert_not_called()

    def test_old_receipt_without_terminal_hash_requires_rerun(self):
        out = self.reusable_receipt(bound=True)
        execution = json.loads((out/'execution.json').read_text())
        del execution['outputs_sha256']['replay-state.csv.terminal.jsonl']
        self.write(out/'execution.json', execution)
        with patch('run_port_references.compare') as compare:
            with self.assertRaisesRegex(ValueError, 'native output differs.*terminal.jsonl'):
                run_case(self.case, self.args, sha(self.args.binary))
            compare.assert_not_called()

    def test_original_changed_during_native_execution_never_reaches_comparison(self):
        def execute(*_args, **_kwargs):
            self.native_outputs()
            path = self.case/'original/replay-state.csv'
            path.write_text(path.read_text().splitlines()[0] + '\n')
            return SimpleNamespace(returncode=0)
        with patch('run_port_references.subprocess.run', side_effect=execute), patch('run_port_references.compare') as compare:
            with self.assertRaisesRegex(ValueError, 'attested remote collection'):
                run_case(self.case, self.args, sha(self.args.binary))
            compare.assert_not_called()

    def test_successful_native_execution_and_comparison_keep_attestation_hash(self):
        def execute(*_args, **_kwargs):
            self.native_outputs()
            return SimpleNamespace(returncode=0)
        result = {'passed': True, 'compared_observations': 2, 'compared_pool_checkpoints': 2,
                  'first_difference_frame': None, 'scalar_difference_count': 0,
                  'pool_difference_count': 0, 'error_count': 0}
        with patch('run_port_references.subprocess.run', side_effect=execute), patch('run_port_references.compare', return_value=result):
            self.assertTrue(run_case(self.case, self.args, sha(self.args.binary))['passed'])
        out = self.case / self.args.name
        execution = json.loads((out/'execution.json').read_text())
        comparison = json.loads((out/'comparison.json').read_text())
        self.assertEqual(execution['original_collection_attestation_sha256'], sha(self.attestation))
        self.assertEqual(comparison['original_collection_attestation']['sha256'], sha(self.attestation))
        self.assertEqual(execution['reference_current_collection_outputs_sha256']['pools.jsonl'],
                         sha(self.case/'original/pools.jsonl'))
        self.assertEqual(execution['outputs_sha256']['replay-state.csv.terminal.jsonl'],
                         sha(out/'replay-state.csv.terminal.jsonl'))


if __name__ == '__main__':
    unittest.main()
