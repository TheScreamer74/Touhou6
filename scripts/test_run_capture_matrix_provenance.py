#!/usr/bin/env python3
"""An image PASS must retain the same original and native provenance."""
import hashlib
import json
from pathlib import Path
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from capture_port_scenario import COMPLETE_OUTPUTS, capture
from run_capture_matrix import (compare_complete_simulation, digest, preflight,
                                prepare_original_collection, update_aggregate,
                                verify_reference_snapshot)
from verify_audio_reference import legal_case_ids, verify_case_receipts


class CaptureMatrixProvenanceTests(unittest.TestCase):
    def setUp(self):
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        self.root = Path(temporary.name)
        self.case = self.root / "replay-c0-d0-s1"
        original = self.case / "original"
        (original / "capture").mkdir(parents=True)
        (self.case / "input.rpy").write_bytes(b"replay fixture")
        self.write(self.case / "input-provenance.json", {"god": True})
        self.image = original / "capture/replay_stage01_frame000002.bmp"
        self.image.write_bytes(b"image content fixture; preflight does not decode images")
        self.execution = {"id": self.case.name, "status": "completed-process", "process_returncode": 0,
                          "input_sha256": digest(self.case / "input.rpy"),
                          "exe_sha256": "7f38496b31b8625196900a69cd1bfed243cab1f9612073e7881dc354b876fd39",
                          "proxy_sha256": "verified proxy fixture",
                          "color_bits": 32, "god": True,
                          "captures": [self.image.name], "terminal": {"stage": 1, "frame": 2,
                              "is_replay": 1, "supervisor_state": 3, "finished_stage": 2}}
        self.write(original / "execution.json", self.execution)
        self.write(original / "terminal.json", self.execution["terminal"])
        (original / "replay-state.csv").write_text("present,frame,stage\n11,1,1\n12,2,1\n")
        pool = {"stage": 1, "enemies": [], "shots": [], "bullets": [], "lasers": [], "items": []}
        (original / "pools.jsonl").write_text("".join(json.dumps(dict(pool, frame=frame)) + "\n"
                                                   for frame in (1, 2)))
        (original / "audio.csv").write_text("present,frame,stage,event,value\n11,1,1,SFX_REQUEST,10\n")
        binary = self.root / "pin/th06"
        binary.parent.mkdir()
        binary.write_bytes(b"pinned executable fixture")
        manifest = {}
        fingerprint = hashlib.sha256(json.dumps(manifest, sort_keys=True,
                                               separators=(",", ":")).encode()).hexdigest()
        self.build = {"binary_sha256": digest(binary), "runtime_files_sha256": manifest,
                      "runtime_source_sha256": fingerprint}
        self.write(binary.with_name("build-source.json"), self.build)
        self.attestation = self.root / "original-audio-collection-attestation.json"
        self.write_attestation()
        self.args = SimpleNamespace(binary=binary, require_matrix=False, name="image-new",
                                    original_attestation=self.attestation, color_bits=32,
                                    hardware_texture_blending=False, menu_entry=True,
                                    complete_reference=False, timeout=1, rerun=False,
                                    game=self.root / "game")
        self.args.game.mkdir()

    @staticmethod
    def write(path, value):
        path.write_text(json.dumps(value) + "\n")

    def write_attestation(self):
        original = self.case / "original"
        paths = {"execution.json": original / "execution.json", "input.rpy": self.case / "input.rpy",
                 **{name: original / name for name in
                    ("audio.csv", "terminal.json", "replay-state.csv", "pools.jsonl")}}
        hashes = {name: digest(path) for name, path in paths.items()}
        rows = [{"id": name, "status": "completed-process", "process_returncode": 0,
                 "declared_input_sha256": hashes["input.rpy"], "sha256": hashes}
                for name in sorted(legal_case_ids())]
        self.write(self.attestation, {"kind": "remote-original-audio-collection-attestation",
                    "schema_version": 2, "passed": True, "errors": [], "cases": rows,
                    "attested_outputs": ["audio.csv", "terminal.json", "replay-state.csv", "pools.jsonl"],
                    "observed_at_utc": "2026-10-13T00:00:00+00:00", "scope": "Test collection snapshot"})

    def test_explicit_collection_binds_a_diagnostic_image_case(self):
        binary_hash, build = preflight([self.case], self.args)
        self.assertEqual(binary_hash, digest(self.args.binary))
        self.assertEqual(build, self.build)
        self.assertEqual(self.args.original_collection_provenance["sha256"], digest(self.attestation))
        verify_reference_snapshot(self.case, self.args)

    def test_matrix_requires_original_collection_even_without_explicit_path(self):
        self.args.require_matrix = True
        self.args.original_attestation = None
        self.attestation.unlink()
        with self.assertRaises(FileNotFoundError):
            prepare_original_collection([self.case], self.args)

    def test_different_executable_or_case_id_is_rejected(self):
        for key, value in (("exe_sha256", "0" * 64), ("id", "another-case")):
            before = self.execution[key]
            self.execution[key] = value
            self.write(self.case / "original/execution.json", self.execution)
            self.write_attestation()
            with self.subTest(key=key), self.assertRaisesRegex(ValueError, "stale original"):
                preflight([self.case], self.args)
            self.execution[key] = before

    def test_changed_original_scalar_is_rejected_before_capture(self):
        path = self.case / "original/replay-state.csv"
        path.write_text(path.read_text() + "13,3,1\n")
        with self.assertRaisesRegex(ValueError, "attested remote collection"):
            preflight([self.case], self.args)

    def test_changed_image_after_preflight_is_rejected(self):
        preflight([self.case], self.args)
        self.image.write_bytes(b"changed image")
        with self.assertRaisesRegex(ValueError, "reference changed after preflight"):
            verify_reference_snapshot(self.case, self.args)

    def test_changed_attestation_after_preflight_is_rejected(self):
        preflight([self.case], self.args)
        self.attestation.write_text(self.attestation.read_text() + "\n")
        with self.assertRaisesRegex(ValueError, "attestation changed after preflight"):
            verify_reference_snapshot(self.case, self.args)

    def test_invalid_build_fingerprint_is_rejected(self):
        self.build["runtime_source_sha256"] = "0" * 64
        self.write(self.args.binary.with_name("build-source.json"), self.build)
        with self.assertRaisesRegex(ValueError, "source manifest fingerprint"):
            preflight([self.case], self.args)

    def native_process(self, command, **_kwargs):
        actual = Path(command[command.index("--capture-dir") + 1])
        original = self.case / "original"
        (actual / "replay-state.csv").write_text("frame,stage\n1,1\n2,1\n")
        (actual / "pools.jsonl").write_bytes((original / "pools.jsonl").read_bytes())
        (actual / "audio.csv").write_bytes((original / "audio.csv").read_bytes())
        (actual / "replay-state.csv.terminal.jsonl").write_text(json.dumps({
            "stage": 1, "frame": 2, "event": "msg-stage-end"}) + "\n")
        (actual / "replay_stage01_frame000002.png").write_bytes(b"native image fixture")
        return SimpleNamespace(returncode=0)

    def complete_capture(self):
        self.args.complete_reference = True
        preflight([self.case], self.args)
        with patch("capture_port_scenario.subprocess.run", side_effect=self.native_process):
            return capture(self.case, self.case / "original/capture", self.args)

    def test_complete_capture_preserves_pin_and_separate_pool_schedule(self):
        result = self.complete_capture()
        command = result["command"]
        self.assertEqual(Path(command[0]), self.args.binary.resolve())
        self.assertEqual(command[command.index("--capture-frames") + 1], "2")
        self.assertEqual(command[command.index("--capture-pool-frames") + 1], "1,2")
        self.assertIn("--complete-reference", command)
        self.assertTrue(set(COMPLETE_OUTPUTS) <= result["outputs_sha256"].keys())
        provenance = verify_case_receipts(self.case, {}, self.args.name)
        self.assertEqual(provenance["original_collection_attestation_sha256"], digest(self.attestation))
        self.assertEqual(provenance["runtime_source_sha256"], self.build["runtime_source_sha256"])

    def test_same_capture_compares_full_state_pools_and_terminal(self):
        captured = self.complete_capture()
        result = compare_complete_simulation(self.case, self.case / self.args.name, captured, self.args)
        self.assertTrue(result["passed"])
        self.assertEqual(result["compared_observations"], 2)
        self.assertEqual(result["compared_pool_checkpoints"], 2)
        self.assertTrue(result["terminal_matched"])

    def test_changed_complete_terminal_is_rejected_before_simulation_comparison(self):
        captured = self.complete_capture()
        path = self.case / self.args.name / "replay-state.csv.terminal.jsonl"
        path.write_text(path.read_text() + "\n")
        with self.assertRaisesRegex(ValueError, "output changed.*terminal.jsonl"):
            compare_complete_simulation(self.case, self.case / self.args.name, captured, self.args)

    def test_missing_complete_outputs_cannot_be_capture_success(self):
        self.args.complete_reference = True
        with patch("capture_port_scenario.subprocess.run", return_value=SimpleNamespace(returncode=0)):
            result = capture(self.case, self.case / "original/capture", self.args)
        self.assertEqual(result["status"], "incomplete-output")
        self.assertEqual(set(result["required_outputs_missing"]), set(COMPLETE_OUTPUTS))

    def test_complete_capture_requires_source_receipt_before_output_creation(self):
        self.args.complete_reference = True
        self.args.binary.with_name("build-source.json").unlink()
        with self.assertRaisesRegex(ValueError, "requires a pin with build-source.json"):
            capture(self.case, self.case / "original/capture", self.args)
        self.assertFalse((self.case / self.args.name).exists())

    def test_complete_capture_without_terminal_image_fails_before_execution(self):
        self.args.complete_reference = True
        self.image.unlink()
        (self.case / "original/capture/replay_stage01_frame000001.bmp").write_bytes(b"earlier image")
        with patch("capture_port_scenario.subprocess.run") as execute:
            with self.assertRaisesRegex(ValueError, "actual original terminal image"):
                capture(self.case, self.case / "original/capture", self.args)
            execute.assert_not_called()
        self.assertFalse((self.case / self.args.name).exists())

    def test_existing_complete_storage_is_rejected_without_removing_evidence(self):
        self.args.complete_reference = True
        self.args.rerun = True
        (self.case / (self.args.name + "-game")).mkdir()
        actual = self.case / self.args.name
        actual.mkdir()
        receipt = actual / "capture-execution.json"
        receipt.write_bytes(b"preserved existing evidence")
        with self.assertRaisesRegex(ValueError, "fresh private storage"):
            capture(self.case, self.case / "original/capture", self.args)
        self.assertEqual(receipt.read_bytes(), b"preserved existing evidence")

    def test_duplicate_original_pool_schedule_is_rejected_before_execution(self):
        self.args.complete_reference = True
        path = self.case / "original/pools.jsonl"
        path.write_text(path.read_text() * 2)
        self.write_attestation()
        with patch("capture_port_scenario.subprocess.run") as execute:
            with self.assertRaisesRegex(ValueError, "unique, ordered"):
                capture(self.case, self.case / "original/capture", self.args)
            execute.assert_not_called()

    def test_image_failure_never_becomes_an_overall_pass(self):
        case = {"pairs": 10, "image_passed": False, "simulation_passed": True,
                "compared_observations": 100, "compared_pool_checkpoints": 20}
        receipt = {"status": "completed", "case_count": 1, "complete_reference": True, "cases": [case]}
        update_aggregate(receipt)
        self.assertTrue(receipt["simulation_passed"])
        self.assertFalse(receipt["image_passed"])
        self.assertFalse(receipt["passed"])
        receipt["case_count"] = 96
        case["image_passed"] = True
        update_aggregate(receipt)
        self.assertFalse(receipt["simulation_passed"])
        self.assertFalse(receipt["passed"])


if __name__ == "__main__":
    unittest.main()
