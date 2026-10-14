#!/usr/bin/env python3
"""Capture missing late-stage images in new private genuine-EXE runs.

The original matrix inputs and old execution/capture evidence are read-only.
Each supplement copies exactly the input bytes, keeps its own Wine prefix and
executes the same declared god/input/clock profile with additional checkpoints.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
import hashlib
import json
from pathlib import Path
import shutil
import subprocess


def sha(path):
    digest=hashlib.sha256()
    with path.open('rb') as source:
        for block in iter(lambda:source.read(1024*1024),b''):
            digest.update(block)
    return digest.hexdigest()


def run(item,args):
    source=args.matrix/item['id']
    case=args.output/item['id']
    execution=json.loads((source/'execution.json').read_text())
    if execution['status']!='completed-process' or execution['process_returncode']!=0 or not execution['terminal']:
        raise ValueError('incomplete original matrix case')
    input_sha=sha(source/'input.rpy')
    if input_sha!=execution['input_sha256']:
        raise ValueError('source input changed')
    if (case/'execution.json').exists():
        raise ValueError(f'complement already exists: {case}')
    case.mkdir(parents=True,exist_ok=True)
    shutil.copyfile(source/'input.rpy',case/'input.rpy')
    provenance=json.loads((source/'input-provenance.json').read_text())
    provenance.update(image_supplement=True,source_execution_sha256=sha(source/'execution.json'),
        source_input_sha256=input_sha,additional_image_checkpoints=item['additional_requested_frames'])
    (case/'input-provenance.json').write_text(json.dumps(provenance,indent=2)+'\n')
    command=['python3',str(args.runner),str(case),'--game',str(args.game),'--prefix',str(args.prefix),
        '--dll',str(args.dll),'--wine',str(args.wine),'--frames',str(item['terminal_frame']+1000),
        '--jobs','1','--timeout','900','--only-checkpoints']
    for frame in item['additional_requested_frames']:
        command.extend(('--checkpoint',str(frame)))
    with (case/'runner.log').open('w') as log:
        process=subprocess.run(command,stdout=log,stderr=subprocess.STDOUT,timeout=1000)
    if process.returncode:
        raise ValueError(f'{case.name}: supplement process failed {process.returncode}')
    result=json.loads((case/'execution.json').read_text())
    if result.get('terminal')!=execution['terminal']:
        raise ValueError(f'{case.name}: supplement terminal diverged from original matrix')
    return {'id':case.name,'terminal':result['terminal'],'captures':len(result['captures']),
        'input_sha256':input_sha,'source_execution_sha256':sha(source/'execution.json'),
        'supplement_execution_sha256':sha(case/'execution.json')}


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    for name in ('plan','matrix','output','runner','game','prefix','dll','wine'):
        parser.add_argument('--'+name,type=Path,required=True)
    parser.add_argument('--jobs',type=int,default=2)
    args=parser.parse_args()
    for name in ('plan','matrix','output','runner','game','prefix','dll','wine'):
        setattr(args,name,getattr(args,name).resolve())
    plan=json.loads(args.plan.read_text())['tail_cases']
    results=[]
    with ThreadPoolExecutor(max_workers=args.jobs) as executor:
        futures=[executor.submit(run,item,args) for item in plan]
        for future in as_completed(futures):
            result=future.result(); results.append(result)
            print(json.dumps(result),flush=True)
    (args.output/'supplement-collection.json').write_text(json.dumps({'cases':results,'case_count':len(results)},indent=2)+'\n')
