#!/usr/bin/env python3
"""Negative evidence checks for the natural menu comparison harness."""
import csv
from pathlib import Path
import tempfile
import unittest

from verify_menu_navigation import compare


class EvidenceTests(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory()
        self.addCleanup(self.temp.cleanup)
        self.root = Path(self.temp.name)
        self.reference = self.root / 'original.csv'
        self.actual = self.root / 'port.csv'
        self.reference.write_text('present,supervisor_state,menu_state,cursor,timer\n180,1,2,0,1\n181,1,2,0,2\n')
        self.actual.write_text('update,menu_state,cursor,timer\n181,2,0,1\n182,2,0,2\n')

    def check(self):
        return compare(self.reference, self.actual, 'navigation', 180, 181)

    def test_known_present_update_mapping(self):
        self.assertTrue(self.check()['passed'])

    def test_missing_tail_is_incomplete(self):
        self.actual.write_text('update,menu_state,cursor,timer\n181,2,0,1\n')
        self.assertFalse(self.check()['passed'])

    def test_empty_trace_rejected(self):
        self.actual.write_text('update,menu_state,cursor,timer\n')
        with self.assertRaises(ValueError):
            self.check()

    def test_duplicate_observation_rejected(self):
        with self.actual.open('a') as out:
            out.write('182,2,0,2\n')
        with self.assertRaises(ValueError):
            self.check()

    def test_stale_original_menu_global_rejected(self):
        self.reference.write_text(self.reference.read_text().replace('181,1,', '181,2,'))
        with self.assertRaises(ValueError):
            self.check()

    def test_unlock_must_match_every_progress_byte(self):
        reference = '63' * 40
        self.reference.write_text('present,state,timer,cursor,cheat_step,progress\n841,7,242,0,0,' + reference + '\n')
        self.actual.write_text('update,state,timer,cursor,cheat_step,progress\n842,7,242,0,0,' + '01' + reference[2:] + '\n')
        self.assertFalse(compare(self.reference, self.actual, 'results', 841, 841)['passed'])
        self.actual.write_text(self.actual.read_text().replace('01' + reference[2:], '63'))
        with self.assertRaises(ValueError):
            compare(self.reference, self.actual, 'results', 841, 841)


if __name__ == '__main__':
    unittest.main()
