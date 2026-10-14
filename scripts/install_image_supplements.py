#!/usr/bin/env python3
"""Install separately attributed genuine image supplements without rewriting history.

The old original execution.json and captures are immutable. A complement must
match the original input, EXE, complete scalar trace and semantic terminal.
Its own execution JSON and per-image hashes remain explicitly attributed to
the new run in original/image-supplement.json.
"""
import argparse
import json
from pathlib import Path
import re
import shutil

from verify_scenario_reference import sha


def install(case, supplement, plan):
    original, fresh = case/'original', supplement/'original'
    manifest=original/'image-supplement.json'
    if manifest.exists():
        raise ValueError('refusing to replace an installed supplement')
    old=json.loads((original/'execution.json').read_text())
    new=json.loads((fresh/'execution.json').read_text())
    if (new['status']!='completed-process' or new['process_returncode']!=0 or
            new['input_sha256']!=old['input_sha256'] or
            new['input_sha256']!=sha(case/'input.rpy') or
            new['exe_sha256']!=old['exe_sha256'] or new['terminal']!=old['terminal']):
        raise ValueError('supplement execution/input/EXE/terminal differs')
    scalar_hash=sha(fresh/'replay-state.csv')
    if scalar_hash!=sha(original/'replay-state.csv'):
        raise ValueError('supplement full scalar trace differs from the original run')
    requested=set(plan['additional_requested_frames'])
    images={}
    for path in sorted((fresh/'capture').iterdir()):
        match=re.fullmatch(r'replay_stage(\d+)_frame(\d+)\.bmp',path.name)
        if not match:
            continue
        stage,frame=map(int,match.groups())
        if stage!=old['terminal']['stage'] or frame not in requested:
            raise ValueError(f'undeclared supplemental image {path.name}')
        target=original/'capture'/path.name
        file_hash=sha(path)
        if target.exists() and sha(target)!=file_hash:
            raise ValueError(f'original image would be overwritten: {path.name}')
        images[path.name]={'stage':stage,'frame':frame,'sha256':file_hash,
                          'added_to_original':not target.exists()}
    terminal=old['terminal']['frame']
    observed={image['frame'] for image in images.values()}
    if not {terminal-1,terminal} <= observed:
        raise ValueError('supplement did not observe terminal-1 and terminal images')
    windows=[[frame,min(frame+2,terminal)] for frame in
        range(((plan['last_image_frame']//1800)+1)*1800,terminal+1,1800)]
    if any(not any(lo<=frame<=hi for frame in observed) for lo,hi in windows):
        raise ValueError('missing supplemental cadence window')
    if set(images)!=set(new['captures']):
        raise ValueError('supplement capture list differs from its execution receipt')
    execution_hash=sha(fresh/'execution.json')
    for image in images.values():
        image['supplement_execution_sha256']=execution_hash
    result={'version':1,'kind':'genuine-original-additional-image-run',
        'original_execution_sha256':sha(original/'execution.json'),
        'supplement_execution_file':'image-supplement-execution.json',
        'supplement_execution_sha256':execution_hash,'input_sha256':new['input_sha256'],
        'exe_sha256':new['exe_sha256'],'proxy_sha256':new['proxy_sha256'],
        'full_scalar_trace_sha256':scalar_hash,'terminal':new['terminal'],
        'last_previous_image_frame':plan['last_image_frame'],'cadence_replay_ticks':1800,
        'required_cadence_windows':windows,'required_terminal_frames':[terminal-1,terminal],
        'requested_frames':sorted(requested),'observed_frames':sorted(observed),
        'images':images,'installer_sha256':sha(Path(__file__)),
        'attribution':'Added images belong to this supplemental execution; the earlier execution.json and its original capture list are preserved.'}
    # All checks complete before any mutation of the original evidence folder.
    shutil.copyfile(fresh/'execution.json',original/result['supplement_execution_file'])
    for filename,image in images.items():
        if image['added_to_original']:
            shutil.copyfile(fresh/'capture'/filename,original/'capture'/filename)
    manifest.write_text(json.dumps(result,indent=2)+'\n')
    return {'id':case.name,'added_images':sum(image['added_to_original'] for image in images.values()),
        'observed_supplement_images':len(images),'supplement_execution_sha256':execution_hash}


if __name__=='__main__':
    parser=argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--matrix',type=Path,required=True)
    parser.add_argument('--supplements',type=Path,required=True)
    parser.add_argument('--plan',type=Path,required=True)
    parser.add_argument('--output',type=Path,required=True)
    args=parser.parse_args()
    results=[]
    for item in json.loads(args.plan.read_text())['tail_cases']:
        case=args.matrix/item['id']
        if not (args.supplements/item['id']/'original/execution.json').exists():
            continue
        if (case/'original/image-supplement.json').exists():
            continue
        result=install(case,args.supplements/item['id'],item)
        results.append(result)
        print(json.dumps(result),flush=True)
    args.output.write_text(json.dumps({'installed_this_run':results,'count':len(results)},indent=2)+'\n')
