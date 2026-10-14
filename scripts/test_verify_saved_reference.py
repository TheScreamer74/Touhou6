#!/usr/bin/env python3
"""Saved-file evidence must reject corruption and meaningful record changes."""
from pathlib import Path
import struct
import tempfile
import unittest

from verify_saved_reference import compare, read_highscores, read_replay


def replay(inputs=((0, 0), (9_999_999, 0)), stage_offset=80):
    raw = bytearray(80)
    raw[:4] = b"T6RP"
    struct.pack_into("<H", raw, 4, 0x102)
    raw[7] = 1
    raw[14] = 0x40
    raw[16:24] = b"10/12/26"
    raw[25:33] = b"AAAAAAAA"
    struct.pack_into("<I", raw, 36, 5_000_500)
    struct.pack_into("<I", raw, 52, stage_offset)
    raw.extend(struct.pack("<IHHBBBBB3x", 5_000_500, 60777, 0, 0, 0, 3, 16, 0))
    for frame, buttons in inputs:
        raw.extend(struct.pack("<iHH", frame, buttons, 0))
    struct.pack_into("<I", raw, 8, (sum(raw[14:]) + 0x3F000318) & 0xFFFFFFFF)
    key = raw[14]
    for i in range(15, len(raw)):
        raw[i] = (raw[i] + key) & 255
        key = (key + 7) & 255
    return raw


def score(name=b"AB      ", value=5_000_500, record_size=28):
    raw = bytearray(60)
    raw[1] = 0x5A
    struct.pack_into("<I", raw, 8, 20)
    struct.pack_into("<I", raw, 16, len(raw))
    raw[20:24] = b"TH6K"
    struct.pack_into("<HHB", raw, 24, 12, 12, 16)
    raw[32:36] = b"HSCR"
    struct.pack_into("<HHB", raw, 36, record_size, record_size, 16)
    struct.pack_into("<I", raw, 44, value)
    raw[49] = 1
    raw[50] = 1
    raw[51:51 + len(name)] = name
    struct.pack_into("<H", raw, 2, sum(raw[4:]) & 65535)
    xor = raw[1]
    for i in range(2, len(raw)):
        before = raw[i]
        xor = ((xor << 3) | (xor >> 5)) & 255
        raw[i] ^= xor
        xor = (xor + before) & 255
    return raw


class SavedEvidenceTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.original = Path(temporary.name) / "original"
        self.port = Path(temporary.name) / "port"
        for directory in (self.original, self.port):
            (directory / "replay").mkdir(parents=True)
            (directory / "replay/th6_01.rpy").write_bytes(replay())
            (directory / "score.dat").write_bytes(score())

    def test_equal_valid_saves_pass(self):
        self.assertTrue(compare(self.original, self.port)["passed"])

    def test_original_ignored_tail_is_reported(self):
        (self.original / "replay/th6_01.rpy").write_bytes(
            replay(((0, 0), (9_999_999, 0), (123, 1), (9_999_999, 0))))
        result = compare(self.original, self.port)
        self.assertTrue(result["passed"])
        self.assertEqual(result["ignored_inputs_after_first_sentinel"]["original"], {1: 2})

    def test_changed_input_with_valid_checksum_fails(self):
        (self.port / "replay/th6_01.rpy").write_bytes(replay(((0, 1), (9_999_999, 0))))
        self.assertFalse(compare(self.original, self.port)["passed"])

    def test_corrupted_replay_checksum_fails(self):
        raw = replay()
        raw[-1] ^= 1
        path = self.port / "replay/th6_01.rpy"
        path.write_bytes(raw)
        with self.assertRaisesRegex(ValueError, "checksum"):
            read_replay(path)

    def test_valid_checksum_cannot_hide_invalid_offsets(self):
        path = self.port / "replay/th6_01.rpy"
        path.write_bytes(replay(stage_offset=81))
        with self.assertRaisesRegex(ValueError, "offsets"):
            read_replay(path)

    def test_changed_highscore_name_and_empty_fixture_fail(self):
        path = self.port / "score.dat"
        for raw in (score(name=b"AC      "), score(value=4_999_999)):
            path.write_bytes(raw)
            self.assertFalse(compare(self.original, self.port)["passed"])

    def test_corrupt_or_zero_sized_score_record_fails(self):
        path = self.port / "score.dat"
        raw = score()
        raw[-1] ^= 1
        for value in (raw, score(record_size=0)):
            path.write_bytes(value)
            with self.assertRaises(ValueError):
                read_highscores(path, 5_000_000)


if __name__ == "__main__":
    unittest.main()
