#!/usr/bin/env python3
import json
from pathlib import Path
import tempfile
import unittest

from verify_stage_transition_reference import verify


class StageTransitionEvidenceTests(unittest.TestCase):
    def setUp(self):
        self.tmp = tempfile.TemporaryDirectory()
        self.addCleanup(self.tmp.cleanup)
        self.original, self.port = (Path(self.tmp.name)/name for name in ('original','port'))
        for directory in (self.original, self.port):
            directory.mkdir()
            rows, pools = [], []
            for index, (stage, frame) in enumerate(((1,1),(1,2),(1,3),(2,1),(2,2),(2,3),(2,4))):
                rows.append(f'{100+index},{frame},{stage},192,384,100\n')
                pools.append(json.dumps(dict(stage=stage,frame=frame,enemies=[],shots=[],bullets=[],lasers=[],items=[]))+'\n')
            (directory/'replay-state.csv').write_text('present,frame,stage,x,y,score\n'+''.join(rows))
            (directory/'pools.jsonl').write_text(''.join(pools))
        (self.original/'terminal.json').write_text(json.dumps(dict(stage=1,frame=3,is_replay=1,supervisor_state=3,finished_stage=2)))
        (self.port/'replay-state.csv.terminal.jsonl').write_text(json.dumps(dict(stage=1,frame=3,event='msg-stage-end'))+'\n')
        (self.original/'audio.csv').write_text('present,frame,stage,event,value,detail\n100,-1,1,BGM_REQUEST,th06_02.mid,\n100,1,1,SFX_REQUEST,0,\n103,3,1,BGM_STOP_REQUEST,,\n103,-1,2,BGM_REQUEST,th06_04.mid,\n103,1,2,SFX_REQUEST,0,\n')
        (self.port/'audio.csv').write_text('present,frame,stage,event,value,detail\n0,-1,1,BGM_REQUEST,th06_02.wav,\n0,1,1,SFX_REQUEST,0,\n3,-1,1,BGM_STOP_REQUEST,,\n3,-1,2,BGM_REQUEST,th06_04.wav,\n3,1,2,SFX_REQUEST,0,\n')

    def test_complete_transition_uses_present_clock_across_replay_reset(self):
        self.assertTrue(verify(self.original,self.port)['passed'])

    def test_missing_next_stage_pool_fails(self):
        path=self.port/'pools.jsonl'
        path.write_text(''.join(path.read_text().splitlines(True)[:-1]))
        self.assertFalse(verify(self.original,self.port)['passed'])

    def test_early_audio_stop_does_not_hide_under_context_frame_none(self):
        path=self.port/'audio.csv'
        path.write_text(path.read_text().replace('3,-1,1,BGM_STOP_REQUEST','2,-1,1,BGM_STOP_REQUEST'))
        self.assertFalse(verify(self.original,self.port)['passed'])

    def test_duplicate_next_stage_scalar_fails(self):
        path=self.port/'replay-state.csv'
        path.write_text(path.read_text()+'106,4,2,192,384,100\n')
        self.assertFalse(verify(self.original,self.port)['passed'])


if __name__=='__main__':
    unittest.main()
