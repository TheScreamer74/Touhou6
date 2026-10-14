#!/usr/bin/env python3
"""Regression tests for rejecting misleading or incomplete reference evidence."""
import csv
import json
from pathlib import Path
import tempfile
import unittest

from verify_scenario_reference import compare


class ReferenceEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.root = Path(self.tmp.name)
        self.reference = self.root / 'original.csv'
        self.port = self.root / 'port.csv'
        self.original_pools = self.root / 'original.jsonl'
        self.port_pools = self.root / 'port.jsonl'
        for path in (self.reference, self.port):
            path.write_text('frame,stage,x,y,score\n1,1,192,384,0\n2,1,192,384,100\n')
        item = {'stage': 1, 'frame': 2, 'enemies': [], 'shots': [], 'bullets': [], 'lasers': [], 'items': []}
        for path in (self.original_pools, self.port_pools):
            path.write_text(json.dumps(item) + '\n')

    def result(self, required_frame=2, original_terminal=None, port_terminal=None):
        return compare(self.reference, self.port, self.original_pools, self.port_pools,
                       1, original_terminal, port_terminal, required_frame)

    def test_equal_nonempty_evidence_reaches_required_checkpoint(self):
        self.assertTrue(self.result()['passed'])

    def test_nan_and_infinity_are_never_equal_positions(self):
        for value in ('nan', 'inf', '-inf'):
            self.port.write_text(f'frame,stage,x,y,score\n1,1,192,384,0\n2,1,{value},384,100\n')
            self.assertFalse(self.result()['passed'])

    def test_empty_trace_cannot_pass(self):
        self.reference.write_text('frame,stage,x,y,score\n')
        self.assertFalse(self.result()['passed'])

    def test_missing_port_observation_cannot_pass(self):
        self.port.write_text('frame,stage,x,y,score\n1,1,192,384,0\n')
        self.assertFalse(self.result()['passed'])

    def test_missing_interior_original_present_cannot_pass(self):
        self.reference.write_text('present,frame,stage,x,y,score\n100,1,1,192,384,0\n102,3,1,192,384,100\n')
        self.port.write_text('frame,stage,x,y,score\n1,1,192,384,0\n2,1,192,384,50\n3,1,192,384,100\n')
        self.assertFalse(self.result(required_frame=3)['passed'])

    def test_dialogue_replay_frame_jumps_keep_contiguous_present_clock(self):
        self.reference.write_text('present,frame,stage,x,y,score\n100,1,1,192,384,0\n101,4,1,192,384,100\n')
        self.port.write_text('frame,stage,x,y,score\n1,1,192,384,0\n4,1,192,384,100\n')
        item = {'stage': 1, 'frame': 4, 'enemies': [], 'shots': [], 'bullets': [], 'lasers': [], 'items': []}
        for path in (self.original_pools, self.port_pools):
            path.write_text(json.dumps(item) + '\n')
        self.assertTrue(self.result(required_frame=4)['passed'])

    def test_duplicate_observations_and_pool_checkpoints_cannot_pass(self):
        self.port.write_text(self.port.read_text() + '2,1,192,384,100\n')
        self.assertFalse(self.result()['passed'])
        self.port.write_text(self.reference.read_text())
        self.port_pools.write_text(self.port_pools.read_text() * 2)
        self.assertFalse(self.result()['passed'])

    def test_no_endpoint_is_not_full_scenario_evidence(self):
        self.assertFalse(self.result(required_frame=None)['passed'])

    def test_terminal_sidecar_does_not_hide_truncated_trace(self):
        original_terminal = self.root / 'terminal.json'
        port_terminal = self.root / 'port-terminal.jsonl'
        original_terminal.write_text(json.dumps({'stage': 1, 'frame': 3, 'is_replay': 1,
            'supervisor_state': 3, 'finished_stage': 2}))
        port_terminal.write_text(json.dumps({'stage': 1, 'frame': 3, 'event': 'msg-stage-end'}) + '\n')
        self.assertFalse(self.result(None, original_terminal, port_terminal)['passed'])

    def test_trailing_duplicate_pool_is_not_hidden_after_last_reference(self):
        line = json.loads(self.port_pools.read_text())
        line['frame'] = 3
        self.port_pools.write_text(self.port_pools.read_text() + (json.dumps(line) + '\n') * 2)
        self.assertFalse(self.result()['passed'])

    def test_matching_terminal_does_not_hide_duplicate_terminal_events(self):
        original_terminal = self.root / 'terminal.json'
        port_terminal = self.root / 'port-terminal.jsonl'
        original_terminal.write_text(json.dumps({'stage': 1, 'frame': 2, 'is_replay': 1,
            'supervisor_state': 3, 'finished_stage': 2}))
        event = json.dumps({'stage': 1, 'frame': 2, 'event': 'msg-stage-end'})+'\n'
        port_terminal.write_text(event*2)
        self.assertFalse(self.result(None, original_terminal, port_terminal)['passed'])

    def test_large_difference_count_has_bounded_diagnostics(self):
        row = json.loads(self.original_pools.read_text())
        row['shots'] = [{'slot': i, 'x': 0.0} for i in range(100)]
        self.original_pools.write_text(json.dumps(row) + '\n')
        row['shots'] = [{'slot': i, 'x': 1.0} for i in range(100)]
        self.port_pools.write_text(json.dumps(row) + '\n')
        result = self.result()
        self.assertFalse(result['passed'])
        self.assertEqual(result['pool_difference_count'], 100)
        self.assertEqual(len(result['pool_differences']), 40)


if __name__ == '__main__':
    unittest.main()
