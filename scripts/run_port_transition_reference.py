#!/usr/bin/env python3
"""Run one pinned native replay through a genuine stage transition window."""
import argparse
import csv
import json
import os
from pathlib import Path
import subprocess
import time

from verify_scenario_reference import sha
from verify_stage_transition_reference import verify


def run(args):
    case, binary, game = args.case.resolve(), args.binary.resolve(), args.game.resolve()
    original, out = case/'original', case/args.name
    if 'target/release' in str(binary):
        raise ValueError('use an immutable pinned executable')
    build_path = binary.with_name('build-source.json')
    build = json.loads(build_path.read_text())
    if build['binary_sha256'] != sha(binary):
        raise ValueError('build-source receipt does not match executable')
    reference = json.loads((original/'execution.json').read_text())
    if (reference.get('status') != 'completed-process' or reference['process_returncode'] != 0
            or reference['input_sha256'] != sha(case/'input.rpy')):
        raise ValueError('original reference is failed/stale')
    if out.exists():
        raise ValueError('refusing to overwrite transition evidence; choose a new --name')
    out.mkdir()
    storage=case/(args.name+'-game')
    storage.mkdir()
    for source in game.iterdir():
        if ((source.suffix.upper()=='.DAT' and source.name.lower()!='score.dat') or
                source.name in ('bgm','gm.sf2','msgothic.ttc','msgothic.ttf')):
            (storage/source.name).symlink_to(source.resolve(), target_is_directory=source.is_dir())
    (storage/'th06-options.bin').write_bytes(bytes([1,2,3,0,1,1,1,1]))
    with (original/'replay-state.csv').open() as source:
        frames=sum(1 for _ in csv.DictReader(source))
    with (original/'pools.jsonl').open() as source:
        checkpoints=sorted({json.loads(line)['frame'] for line in source})
    command=[str(binary),'--game-dir',str(storage),'--replay',str(case/'input.rpy'),
        '--frames',str(frames),'--replay-dump',str(out/'replay-state.csv'),
        '--replay-pools',str(out/'pools.jsonl'),'--capture-frames',','.join(map(str,checkpoints))]
    provenance=json.loads((case/'input-provenance.json').read_text())
    if provenance.get('god'):
        command.append('--god')
    env=dict(os.environ,TH06_AUDIO_TRACE=str(out/'audio.csv'))
    for key in ('TH06_GOD','TH06_NO_DIALOGUE','TH06_BOMB','TH06_NOSHOOT',
                'TH06_FONT','TH06_SOUNDFONT'):
        env.pop(key,None)
    start=time.monotonic()
    with (out/'run.log').open('w') as log:
        process=subprocess.run(command,env=env,stdout=log,stderr=subprocess.STDOUT,timeout=args.timeout)
    if sha(binary)!=build['binary_sha256']:
        raise ValueError('pinned executable changed during execution')
    receipt={'id':case.name,'command':command,'returncode':process.returncode,
        'binary_sha256':build['binary_sha256'],'build_provenance':build,
        'input_sha256':sha(case/'input.rpy'),'reference_execution_sha256':sha(original/'execution.json'),
        'elapsed_seconds':time.monotonic()-start,
        'harness_sha256':{path.name:sha(path) for path in (Path(__file__),Path(__file__).with_name('verify_stage_transition_reference.py'),Path(__file__).with_name('verify_scenario_reference.py'))}}
    receipt['outputs_sha256']={path.name:sha(path) for path in out.iterdir() if path.is_file() and path.name!='run.log'}
    (out/'execution.json').write_text(json.dumps(receipt,indent=2)+'\n')
    if process.returncode:
        raise ValueError(f'port process exited {process.returncode}')
    result=verify(original,out)
    result['execution_sha256']=sha(out/'execution.json')
    (out/'comparison.json').write_text(json.dumps(result,indent=2)+'\n')
    print(json.dumps({key:result[key] for key in ('passed','audio_requests','audio_difference_count','error_count')}))
    return result['passed']


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('case',type=Path)
    parser.add_argument('--binary',type=Path,required=True)
    parser.add_argument('--game',type=Path,required=True)
    parser.add_argument('--name',default='port-transition-final')
    parser.add_argument('--timeout',type=float,default=120)
    args=parser.parse_args()
    if Path(args.name).name != args.name or args.name in ('.','..'):
        parser.error('--name must be a single directory component')
    raise SystemExit(0 if run(args) else 1)
