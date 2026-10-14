#!/usr/bin/env python3
"""Collect completed private Wine evidence, never game assets or prefixes."""
import argparse
import json
from pathlib import Path
import shlex
import subprocess
import tarfile
import uuid

REMOTE = r'''
import json, pathlib, sys, tarfile
base=pathlib.Path(sys.argv[1]); already=set(json.loads(sys.argv[2])); requested=json.loads(sys.argv[3])
with tarfile.open(fileobj=sys.stdout.buffer, mode='w|gz') as tar:
 for case in ([base/name for name in requested] if requested else sorted(base.glob('replay-c*-d*-s*'))):
  manifest=case/'execution.json'
  if not manifest.exists() or case.name in already: continue
  data=json.loads(manifest.read_text())
  if data['status']!='completed-process' or (not requested and not data['terminal']): continue
  files=['replay-state.csv','pools.jsonl','audio.csv','scenes.csv','menus.csv','th06cap.log']
  if data.get('terminal'): files.append('terminal.json')
  if not all((case/'game'/name).is_file() for name in files): continue
  files += [name for name in ('vms.jsonl','lifecycle.csv') if (case/'game'/name).is_file()]
  tar.add(manifest,arcname=case.name+'/original/execution.json')
  tar.add(case/'game'/'th06cap.txt',arcname=case.name+'/original/th06cap.txt')
  for name in files: tar.add(case/'game'/name,arcname=case.name+'/original/'+name)
  for path in sorted((case/'game'/'capture').glob('*.bmp')):
   tar.add(path,arcname=case.name+'/original/capture/'+path.name)
'''


def collect(args):
    args.output.mkdir(parents=True, exist_ok=True)
    if any(Path(name).name != name or name in ('.', '..') for name in args.case):
        raise ValueError('case names must be single path components')
    already = [p.parent.parent.name for p in args.output.glob('*/original/execution.json')]
    archive = args.output / ('collected-reference-evidence-' + uuid.uuid4().hex + '.tar.gz')
    command = shlex.join(['python3', '-c', REMOTE, args.remote, json.dumps(already), json.dumps(args.case)])
    with archive.open('wb') as target:
        subprocess.run(['ssh', args.host, command], stdout=target, check=True)
    added = set()
    with tarfile.open(archive) as tar:
        for member in tar:
            path = Path(member.name)
            if path.is_absolute() or '..' in path.parts or member.issym() or member.islnk():
                raise ValueError('unsafe evidence archive member')
            tar.extract(member, args.output, filter='data')
            added.add(path.parts[0])
    result = {'collected': sorted(added), 'previous_completed': len(already),
              'archive': str(archive), 'bytes': archive.stat().st_size}
    print(json.dumps(result, indent=2))


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--host', default='ubuntu@blackoutburst.com')
    parser.add_argument('--remote', default='/home/ubuntu/touhou6-web/reference/scenarios')
    parser.add_argument('--output', type=Path, default=Path('/tmp/th06-scenarios'))
    parser.add_argument('--case', action='append', default=[], help='collect a named completed capture or transition case')
    collect(parser.parse_args())
