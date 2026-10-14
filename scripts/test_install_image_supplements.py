#!/usr/bin/env python3
import json
from pathlib import Path
import tempfile
import unittest

from install_image_supplements import install
from verify_scenario_reference import sha


class ImageSupplementEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.tmp=tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        root=Path(self.tmp.name)
        self.case,self.supplement=root/'matrix',root/'supplement'
        for folder in (self.case,self.supplement):
            (folder/'original/capture').mkdir(parents=True)
            (folder/'original/replay-state.csv').write_text('frame,stage\n1799,1\n1800,1\n')
        (self.case/'input.rpy').write_bytes(b'fixture-input')
        self.terminal=dict(stage=1,frame=1800,present=1900,supervisor_state=3,finished_stage=2,is_replay=1)
        self.execution=dict(status='completed-process',process_returncode=0,input_sha256=sha(self.case/'input.rpy'),
            exe_sha256='locked-test-exe',proxy_sha256='test-proxy',terminal=self.terminal,captures=[])
        (self.case/'original/execution.json').write_text(json.dumps(self.execution))
        for frame in (1799,1800):
            name=f'replay_stage01_frame{frame:06}.bmp'
            (self.supplement/'original/capture'/name).write_bytes(f'image{frame}'.encode())
            self.execution['captures'].append(name)
        (self.supplement/'original/execution.json').write_text(json.dumps(self.execution))
        self.plan=dict(last_image_frame=1700,additional_requested_frames=[1799,1800,1801])

    def test_added_images_retain_separate_provenance_and_old_receipt(self):
        old=sha(self.case/'original/execution.json')
        result=install(self.case,self.supplement,self.plan)
        self.assertEqual(result['added_images'],2)
        self.assertEqual(sha(self.case/'original/execution.json'),old)
        manifest=json.loads((self.case/'original/image-supplement.json').read_text())
        self.assertEqual(manifest['original_execution_sha256'],old)
        self.assertEqual(manifest['supplement_execution_sha256'],sha(self.case/'original/image-supplement-execution.json'))

    def test_changed_scalar_trace_rejects_before_any_original_write(self):
        (self.supplement/'original/replay-state.csv').write_text('frame,stage\n1799,1\n1801,1\n')
        with self.assertRaises(ValueError):
            install(self.case,self.supplement,self.plan)
        self.assertFalse((self.case/'original/image-supplement-execution.json').exists())

    def test_unobserved_terminal_cannot_be_claimed(self):
        (self.supplement/'original/capture/replay_stage01_frame001800.bmp').unlink()
        with self.assertRaises(ValueError):
            install(self.case,self.supplement,self.plan)

    def test_changed_existing_image_is_never_overwritten(self):
        target=self.case/'original/capture/replay_stage01_frame001799.bmp'
        target.write_bytes(b'earlier-evidence')
        with self.assertRaises(ValueError):
            install(self.case,self.supplement,self.plan)
        self.assertEqual(target.read_bytes(),b'earlier-evidence')


if __name__=='__main__':
    unittest.main()
