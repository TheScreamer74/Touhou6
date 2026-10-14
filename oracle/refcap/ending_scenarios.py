#!/usr/bin/env python3
"""Run declared ending-entry scenarios in isolated copies of the supplied EXE.

This is an END interpreter fixture, not a claimed full story completion. The
proxy refuses entry unless an actual Replay is active, then changes character,
difficulty, retries and clear-state once. Every subsequent END tick is original.
Use a dedicated X display; cases sharing the prefix must run sequentially.
"""
from pathlib import Path
import os, shutil, subprocess, time, json, argparse, hashlib

parser = argparse.ArgumentParser()
parser.add_argument('--root', required=True, type=Path, help='Reference workspace containing game/, prefix/, tools/, ending-proxy/')
parser.add_argument('--display', default=':110')
parser.add_argument('--timeout', type=int, default=600)
parser.add_argument('--shot', type=int, choices=range(4), default=0)
parser.add_argument('--difficulty', type=int, choices=range(4), default=1)
parser.add_argument('--retries', type=int, choices=range(4), default=0)
parser.add_argument('--seen', type=int, choices=(0, 1), default=0)
parser.add_argument('--rerun', action='store_true')
parser.add_argument('--controls', choices=['plain', 'confirm', 'ctrl'], default='plain')
args = parser.parse_args()
root = args.root.resolve()
if args.timeout <= 0:
    parser.error('--timeout must be positive')
if hashlib.sha256((root/'game/102h.exe').read_bytes()).hexdigest() != '7f38496b31b8625196900a69cd1bfed243cab1f9612073e7881dc354b876fd39':
    parser.error('This memory-layout observer requires the supplied v1.02h executable')
name = f'c{args.shot}-d{args.difficulty}-r{args.retries}-seen{args.seen}-{args.controls}'
job = root / 'ending-scenarios' / name
game = job / 'game'
if (job/'run.json').exists() and not args.rerun:
    parser.error('Existing evidence: select --rerun explicitly to replace it')
game.mkdir(parents=True, exist_ok=True)
for item in ['102h.exe', 'CM.DAT', 'ED.DAT', 'IN.DAT', 'MD.DAT', 'ST.DAT', 'TL.DAT']:
    shutil.copy2(root/'game'/item, game/item)
shutil.copy2(root/'ending-proxy/d3d8.dll', game/'d3d8.dll')
(game/'replay').mkdir(exist_ok=True)
shutil.copy2(root/'game/replay/th6_01.rpy', game/'replay/th6_01.rpy')
if not (game/'bgm').exists():
    (game/'bgm').symlink_to(root/'game/bgm', target_is_directory=True)
for stale in ['score.dat', 'ending-state.csv', 'ending-terminal.txt', 'audio.csv', 'scenes.csv']:
    (game/stale).unlink(missing_ok=True)
if (game/'capture').exists():
    shutil.rmtree(game/'capture')
cfg = bytearray((root/'game/cfg.cfg').read_bytes())
cfg[24:32] = bytes([2,3,0,0,0,1,1,0])
(game/'cfg.cfg').write_bytes(cfg)
config = f'''scenefile scenes.csv
endingfile ending-state.csv
endingsetup {args.shot} {args.difficulty} {args.retries} {args.seen} 620
stopendingend 1
capdir capture
capstart 621
capend 50621
capstride 600
stopafter 50620
key 180 184 Q
key 240 244 DOWN
key 270 274 DOWN
tap 350 60 5 Z
'''
if args.controls != 'plain':
    config += 'key 621 50621 Z\n'
if args.controls == 'ctrl':
    config += 'key 621 50621 CTRL\n'
(game/'th06cap.txt').write_text(config)
prefix = root/'ending-scenarios/prefix'
if not prefix.exists():
    shutil.copytree(root/'prefix', prefix, symlinks=True)
with (job/'xvfb.log').open('w') as xlog:
    xvfb = subprocess.Popen(['Xvfb', args.display, '-screen', '0', '800x600x24', '-nolisten', 'tcp'], stdout=xlog, stderr=subprocess.STDOUT)
    time.sleep(1)
    if xvfb.poll() is not None:
        raise RuntimeError(f'Dedicated X display {args.display} could not start')
    env = os.environ.copy()
    env.update(DISPLAY=args.display, WINEPREFIX=str(prefix), WINEDLLOVERRIDES='d3d8=n,b', WINEDEBUG='-all', LIBGL_ALWAYS_SOFTWARE='1', TH06CAP_AUDIO='audio.csv')
    start = time.monotonic()
    try:
        with (job/'wine.log').open('w') as log:
            run = subprocess.run(['timeout',str(args.timeout),str(root/'tools/wine-11.0-amd64-wow64/bin/wine'),'./102h.exe'], cwd=game, env=env, stdout=log, stderr=subprocess.STDOUT)
        report = {'exe_sha256':hashlib.sha256((game/'102h.exe').read_bytes()).hexdigest(), 'proxy_sha256':hashlib.sha256((game/'d3d8.dll').read_bytes()).hexdigest(), 'config_sha256':hashlib.sha256((game/'th06cap.txt').read_bytes()).hexdigest(), 'fixture':vars(args)|{'root':str(root)}, 'exit': run.returncode, 'elapsed': time.monotonic()-start, 'case':name, 'terminal':(game/'ending-terminal.txt').read_text() if (game/'ending-terminal.txt').exists() else None}
        (job/'run.json').write_text(json.dumps(report, indent=2)+'\n')
        print(json.dumps(report), flush=True)
        if run.returncode or not report['terminal']:
            raise SystemExit(1)
    finally:
        subprocess.run([str(root/'tools/wine-11.0-amd64-wow64/bin/wineserver'),'-k'],env=env,stdout=subprocess.DEVNULL,stderr=subprocess.DEVNULL)
        xvfb.terminate()
        xvfb.wait()
