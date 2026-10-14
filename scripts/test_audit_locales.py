import struct
import tempfile
from pathlib import Path
import unittest

from audit_locales import ending_strings, msg_strings, music_strings, spell_strings, ui_keys


class LocaleInventoryTests(unittest.TestCase):
    def test_new_namespaces_are_audited_instead_of_silently_falling_back(self):
        with tempfile.TemporaryDirectory() as directory:
            root = Path(directory)
            (root / "hud.rs").write_text('"hud.score" "future.caption" "score.dat" "input.rpy"')
            self.assertEqual(ui_keys(root), {"hud.score", "future.caption"})

    def test_every_msg_entry_and_intro_is_inventoried(self):
        def text(opcode, value):
            args = struct.pack("<hh", 1, 0) + value.encode("cp932") + b"\0"
            return struct.pack("<HBB", 60, opcode, len(args)) + args
        stop = struct.pack("<HBB", 61, 10, 0)
        first = text(3, "こんにちは") + stop
        second = text(8, "紅い月　 ") + stop
        data = struct.pack("<III", 2, 12, 12 + len(first)) + first + second
        strings, count = msg_strings(data)
        self.assertEqual(count, 2)
        self.assertEqual(strings, {"こんにちは", "紅い月　 "})

    def test_truncated_msg_arguments_do_not_look_like_coverage(self):
        data = struct.pack("<IIHBB", 1, 8, 0, 3, 255)
        with self.assertRaisesRegex(ValueError, "truncated MSG arguments"):
            msg_strings(data)

    def test_spell_names_retain_exact_source_spaces(self):
        name = "月符「光」　".encode("cp932") + b"\0"
        args = struct.pack("<hh", 0, 1) + name
        instruction = struct.pack("<IhhBBH", 0, 93, 12 + len(args), 0, 0, 0) + args
        stop = struct.pack("<IhhBBH", 0, 0, 0, 0, 0, 0)
        data = struct.pack("<HHIIII", 1, 0, 0, 0, 0, 20) + instruction + stop
        self.assertEqual(spell_strings(data), {"月符「光」　"})

    def test_ending_keys_use_byte_end_before_nul_and_keep_context(self):
        prefix = b"@c0\0\n"
        first = "あ" * 16 + "」"
        second = "い" * 16 + "」"
        data = prefix + first.encode("cp932") + b"\0\n" + second.encode("cp932") + b"\0\n@z\0\n"
        lines, chunks, contexts = ending_strings(data, "END00.END")
        self.assertEqual(lines, {first, second})
        self.assertEqual(chunks, {"あ" * 16, "い" * 16, "」"})
        first_start = len(prefix)
        second_start = len(prefix) + len(first.encode("cp932")) + 2
        self.assertEqual(contexts, {
            f"end:end00.end:{first_start + 32}",
            f"end:end00.end:{first_start + 34}",
            f"end:end00.end:{second_start + 32}",
            f"end:end00.end:{second_start + 34}",
        })

    def test_shiftjis_space_is_not_a_music_track_header(self):
        data = "@bgm/th06_01.mid\r\n赤い夢\r\nNo.1 赤い夢\r\n　説明　 \r\n".encode("cp932")
        titles, comments, count = music_strings(data)
        self.assertEqual(count, 1)
        self.assertEqual(titles, {"赤い夢"})
        self.assertEqual(comments, {"No.1 赤い夢", "　説明　 "})


if __name__ == "__main__":
    unittest.main()
