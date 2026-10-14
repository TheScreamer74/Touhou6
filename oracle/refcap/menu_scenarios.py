#!/usr/bin/env python3
"""Run natural title, selection, controller-menu and Extra unlock scenarios.

No gameplay/menu state is patched. The proxy only supplies the declared keys,
clock and passive observations. Each case has private cfg/score/Wine storage.
Run on the reference host with an already compiled proxy.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time


SCENARIOS = {
    'replay-cancel-held': (560, '''key 180 184 Q
key 240 244 DOWN
key 270 274 DOWN
key 350 354 Z
key 415 510 X
'''),
    'replay-entry-held': (500, '''key 180 184 Q
key 240 244 DOWN
key 270 274 DOWN
key 350 354 Z
key 370 470 DOWN
'''),
    'replay-fast-stages': (570, '''key 180 184 Q
key 240 244 DOWN
key 270 274 DOWN
key 350 354 Z
key 415 419 Z
key 460 464 X
key 505 509 X
'''),
    'selection-cancel-held': (510, '''key 180 184 Q
key 240 244 Z
key 340 450 X
'''),
    'selection-entry-held': (430, '''key 180 184 Q
key 240 340 DOWN
key 240 340 Z
'''),
    'options-entry-held': (550, '''key 180 184 Q
key 240 244 UP
key 310 500 UP
key 340 344 Z
key 410 414 X
key 420 424 Z
'''),
    'selection-cancel': (500, '''key 180 184 Q
key 240 244 Z
key 305 309 DOWN
key 340 344 X
key 380 384 Q
key 410 414 S
key 450 454 UP
'''),
    'selection-fast': (650, '''key 180 184 Q
key 240 244 Z
key 305 309 Z
key 340 344 RIGHT
key 350 354 Z
key 380 384 DOWN
key 410 414 Z
'''),
    'keyconfig-return': (800, '''key 180 184 Q
key 240 244 UP
key 270 274 UP
key 330 334 Z
key 370 374 UP
key 400 404 UP
key 440 444 Z
key 480 484 UP
key 510 514 UP
key 550 554 Z
key 580 584 DOWN
key 620 624 Z
key 660 664 ESC
key 700 704 Z
key 740 744 UP
key 740 744 DOWN
'''),
    'extra-unlock': (1000, '''key 180 184 Q
key 240 244 DOWN
key 270 274 DOWN
key 300 304 DOWN
key 350 354 Z
key 480 484 DOWN
key 510 514 DOWN
key 540 544 DOWN
key 600 604 Z
key 650 1001 SHIFT
tap 660 20 5 HOME
tap 760 20 2 Q
tap 800 20 3 S
'''),
}


def digest(path):
    result = hashlib.sha256()
    with path.open('rb') as source:
        for block in iter(lambda: source.read(1024 * 1024), b''):
            result.update(block)
    return result.hexdigest()


def run_case(name, args, display):
    stop, keys = SCENARIOS[name]
    case = args.output / name
    case.mkdir(parents=True, exist_ok=True)
    receipt = case / 'execution.json'
    if receipt.exists():
        raise ValueError(f'Refusing to overwrite evidence: {receipt}')
    game = case / 'game'
    game.mkdir(exist_ok=True)
    for filename in ('102h.exe', 'CM.DAT', 'ED.DAT', 'IN.DAT', 'MD.DAT', 'ST.DAT', 'TL.DAT'):
        shutil.copy2(args.game / filename, game / filename)
    shutil.copy2(args.dll, game / 'd3d8.dll')
    (game / 'bgm').symlink_to(args.bgm, target_is_directory=True)
    cfg = bytearray((args.game / 'cfg.cfg').read_bytes())
    if len(cfg) != 56:
        raise ValueError('Expected original 56-byte cfg')
    cfg[24:32] = bytes([2, 3, 0, 1, 1, 1, 1, 0])
    (game / 'cfg.cfg').write_bytes(cfg)
    (case / 'input.txt').write_text(keys)
    replay_files = []
    if args.replay_file:
        replay_directory = game / 'replay'
        replay_directory.mkdir()
        for index, source in enumerate(args.replay_file, 1):
            replay_name = f'th6_{index:02d}.rpy'
            shutil.copy2(source, replay_directory / replay_name)
            replay_files.append({'name': replay_name, 'sha256': digest(source)})
    config = ('menufile menus.csv\nresultfile results.csv\nscenefile scenes.csv\n'
              'capdir capture\ncapstart 300\ncapend ' + str(stop + 1) +
              '\ncapstride 20\nstopafter ' + str(stop) + '\n' + keys)
    (game / 'th06cap.txt').write_text(config)
    prefix = case / 'prefix'
    subprocess.run(['cp', '-a', '--reflink=auto', str(args.prefix), str(prefix)], check=True)
    with (case / 'xvfb.log').open('w') as log:
        xvfb = subprocess.Popen(['Xvfb', f':{display}', '-screen', '0', '800x600x24', '-nolisten', 'tcp'], stdout=log, stderr=subprocess.STDOUT)
    time.sleep(1)
    env = dict(os.environ, DISPLAY=f':{display}', WINEPREFIX=str(prefix),
               WINEDLLOVERRIDES='d3d8=n,b', WINEDEBUG='-all', LIBGL_ALWAYS_SOFTWARE='1')
    start = time.monotonic()
    command = ['timeout', '90', str(args.wine), './102h.exe']
    try:
        with (case / 'wine.log').open('w') as log:
            process = subprocess.run(command, cwd=game, env=env, stdout=log, stderr=subprocess.STDOUT)
    finally:
        subprocess.run([str(args.wine.with_name('wineserver')), '-k'], env=env, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
        xvfb.terminate()
        xvfb.wait()
    data = {'id': name, 'status': 'completed-process', 'returncode': process.returncode,
            'elapsed_seconds': time.monotonic() - start, 'command': command,
            'exe_sha256': digest(game / '102h.exe'), 'proxy_sha256': digest(game / 'd3d8.dll'),
            'config_sha256': digest(game / 'th06cap.txt'), 'input_sha256': digest(case / 'input.txt'),
            'stop_present': stop, 'scene_entry': 'natural keyboard navigation',
            'replay_files': replay_files,
            'patches': 'declared input and synthetic clock only; no state/score/menu writes'}
    receipt.write_text(json.dumps(data, indent=2) + '\n')
    if process.returncode != 0:
        raise ValueError(f'{name}: original exited {process.returncode}')
    return data


if __name__ == '__main__':
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--game', type=Path, required=True)
    parser.add_argument('--prefix', type=Path, required=True)
    parser.add_argument('--wine', type=Path, required=True)
    parser.add_argument('--dll', type=Path, required=True)
    parser.add_argument('--bgm', type=Path, required=True)
    parser.add_argument('--output', type=Path, required=True)
    parser.add_argument('--display', type=int, default=151)
    parser.add_argument('--case', choices=tuple(SCENARIOS), action='append')
    parser.add_argument('--replay-file', type=Path, action='append', default=[],
                        help='Immutable valid original replay installed into private numbered slots')
    args = parser.parse_args()
    for name in ('game', 'prefix', 'wine', 'dll', 'bgm', 'output'):
        setattr(args, name, getattr(args, name).resolve())
    for index, name in enumerate(args.case or SCENARIOS):
        print(json.dumps(run_case(name, args, args.display + index)), flush=True)
