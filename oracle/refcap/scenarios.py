#!/usr/bin/env python3
"""Run supplied T6RP scenarios in isolated copies of the real 1.02h executable.

Run this on the Wine host. Each case has its own game files, writable prefix,
X display and logs. No shared game copy is written and no web deployment is
involved. Output records execution/termination; parity is a separate check.
"""
import argparse
from concurrent.futures import ThreadPoolExecutor, as_completed
import hashlib
import json
import os
from pathlib import Path
import shutil
import struct
import subprocess
import time


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def replay_info(path):
    raw = path.read_bytes()
    if len(raw) < 80 or raw[:4] != b"T6RP":
        raise ValueError("input is not T6RP")
    data = bytearray(raw)
    key = data[14]
    for index in range(15, len(data)):
        data[index] = (data[index] - key) & 255
        key = (key + 7) & 255
    expected = struct.unpack_from("<I", data, 8)[0]
    if (sum(data[14:]) + 0x3F000318) & 0xFFFFFFFF != expected:
        raise ValueError("input checksum is invalid")
    offsets = struct.unpack_from("<7I", data, 52)
    return {"character": data[6], "difficulty": data[7],
            "stages": [i + 1 for i, at in enumerate(offsets) if at]}


def selected_frames(length):
    # Capture all three possible Present endpoints around each target because
    # skippable replay dialogue executes up to three calculation ticks/Present.
    targets = {1, 60, 120, 240, 480, 900, 960, 1000, 1800, 1920,
               2400, 3000, 3720, max(1, length - 60), max(1, length - 1)}
    targets.update(range(5400, length, 1800))
    return sorted({f + delta for f in targets if f <= length for delta in range(3)})


def run_case(case, args):
    case = case.resolve()
    provenance_path = case / "input-provenance.json"
    provenance = json.loads(provenance_path.read_text()) if provenance_path.exists() else {
        "id": case.name, "kind": "original-supplied-replay", "god": False,
        "generator_frames": args.frames, "expected_checkpoint": "requested-replay-frame"}
    info = replay_info(case / "input.rpy")
    stage = info["stages"][0]
    game = case / "game"
    prefix = case / "prefix"
    # Refuse to overwrite previous evidence implicitly.
    if (case / "execution.json").exists() and not args.rerun:
        return json.loads((case / "execution.json").read_text())
    game.mkdir(exist_ok=True)
    # Explicit reruns replace only generated evidence, never replay inputs.
    for name in ("replay-state.csv", "scenes.csv", "menus.csv", "pools.jsonl",
                 "audio.csv", "lifecycle.csv", "terminal.json", "th06cap.log", "wine-run.log"):
        (game / name).unlink(missing_ok=True)
    if (game / "capture").exists():
        shutil.rmtree(game / "capture")
    for path in args.game.iterdir():
        if path.suffix.upper() == ".DAT" or path.name in ("102h.exe", "cfg.cfg"):
            shutil.copy2(path, game / path.name)
    # BGM and fonts are copied where supplied, never obtained from the network.
    for name in ("bgm", "gm.sf2", "msgothic.ttc", "msgothic.ttf"):
        source = args.game / name
        if source.is_dir() and not (game / name).exists():
            shutil.copytree(source, game / name)
        elif source.is_file():
            shutil.copy2(source, game / name)
    if not prefix.exists():
        subprocess.run(["cp", "-a", "--reflink=auto", str(args.prefix), str(prefix)], check=True)
    shutil.copy2(args.dll, game / "d3d8.dll")
    replay_dir = game / "replay"
    replay_dir.mkdir(exist_ok=True)
    shutil.copy2(case / "input.rpy", replay_dir / "th6_01.rpy")
    config = bytearray((game / "cfg.cfg").read_bytes())
    if len(config) != 56:
        raise ValueError("expected original 0x38-byte GameConfiguration")
    config[26] = args.color_bits == 16
    config[27] = 1  # WAV
    config[28] = 1  # SFX enabled; trace also reports missing DirectSound backend.
    config[30] = not args.fullscreen  # fullscreen required for a genuine 565 backbuffer
    config[31] = 0  # no frameskip
    if args.hardware:
        struct.pack_into("<I", config, 52, 0)
    (game / "cfg.cfg").write_bytes(config)
    frames = [] if args.only_checkpoints else selected_frames(provenance["generator_frames"])
    frames = sorted(set(frames) | set(args.checkpoint))
    complete = provenance["expected_checkpoint"] in ("original-msg-stage-end", "original-game-over")
    game_over = provenance["expected_checkpoint"] == "original-game-over"
    lines = ["statefile replay-state.csv", "scenefile scenes.csv", "menufile menus.csv",
             "poolfile pools.jsonl", f"poolstride {args.pool_stride}", "capdir capture",
             f"stopafter {max(args.frames, provenance['generator_frames'] * 2 + 3000) if complete else args.frames}",
             f"stopstageend {int(complete and not game_over)}", f"stopgameover {int(game_over)}",
             f"postterminal {args.post_terminal}",
             f"god {int(provenance.get('god', False))}"]
    if game_over:
        lines.append("lifecyclefile lifecycle.csv")
    if args.diagnostic_vms:
        lines.append("vmfile vms.jsonl")
    if getattr(args, 'diagnostic_gui', False):
        lines.append("guifile gui.csv")
    for frame in frames:
        lines.extend((f"capreplay {stage} {frame}", f"poolframe {stage} {frame}"))
        if args.diagnostic_vms:
            lines.append(f"vmframe {stage} {frame}")
    stage_checkpoints = []
    for value in args.stage_checkpoint:
        checkpoint_stage, checkpoint_frame = map(int, value.split(':'))
        if not 1 <= checkpoint_stage <= 7 or checkpoint_frame <= 0:
            raise ValueError('stage checkpoint must be a legal positive stage:frame')
        stage_checkpoints.append((checkpoint_stage, checkpoint_frame))
        lines.extend((f"capreplay {checkpoint_stage} {checkpoint_frame}",
                      f"poolframe {checkpoint_stage} {checkpoint_frame}"))
        if args.diagnostic_vms:
            lines.append(f"vmframe {checkpoint_stage} {checkpoint_frame}")
    # No score.dat is copied. Extra is therefore locked and the DOWN handler
    # skips it, making two DOWN taps select Replay (cursor 3).
    lines.extend(("key 180 184 Q", "key 240 244 DOWN", "key 270 274 DOWN", "tap 350 60 5 Z"))
    config_text = "\n".join(lines) + "\n"
    (game / "th06cap.txt").write_text(config_text)
    env = dict(os.environ, WINEPREFIX=str(prefix), WINEDLLOVERRIDES="d3d8=n,b;mscoree,mshtml=",
               WINEDEBUG="-all", LIBGL_ALWAYS_SOFTWARE="1", TH06CAP_AUDIO="audio.csv")
    if args.alsa_config:
        env["ALSA_CONFIG_PATH"] = str(args.alsa_config.resolve())
    # A separate display prevents WM_ACTIVATEAPP from pausing concurrent cases.
    command = ["xvfb-run", "-a", "-s", "-screen 0 640x480x24", str(args.wine), "102h.exe"]
    begin = time.monotonic()
    status = "completed-process"
    returncode = None
    try:
        with (game / "wine-run.log").open("wb") as log:
            run = subprocess.run(command, cwd=game, env=env, stdout=log, stderr=subprocess.STDOUT,
                                 timeout=args.timeout)
            returncode = run.returncode
    except subprocess.TimeoutExpired:
        status = "timeout"
    finally:
        # Kill only this case's Wine server, never another task's process.
        subprocess.run([str(args.wine.parent / "wineserver"), "-k"], env=env,
                       stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL, check=False)
    terminal = game / "terminal.json"
    result = {
        "id": case.name, "kind": provenance["kind"], "god": provenance.get("god", False),
        "replay": info, "input_sha256": digest(case / "input.rpy"),
        "exe_sha256": digest(game / "102h.exe"), "proxy_sha256": digest(game / "d3d8.dll"),
        "config_sha256": digest(game / "th06cap.txt"), "color_bits": args.color_bits,
        "windowed": not args.fullscreen, "hardware_profile": args.hardware,
        "capture_format_log": (game / "th06cap.log").read_text(errors="replace"),
        "elapsed_seconds": round(time.monotonic() - begin, 3), "status": status,
        "process_returncode": returncode,
        "expected_checkpoint": provenance["expected_checkpoint"],
        "terminal": json.loads(terminal.read_text()) if terminal.exists() else None,
        "requested_capture_frames": frames,
        "requested_stage_checkpoints": stage_checkpoints,
        "captures": sorted(path.name for path in (game / "capture").glob("replay_*.bmp")),
    }
    # No parity PASS is emitted here. An executable finishing is insufficient.
    (case / "execution.json").write_text(json.dumps(result, indent=2) + "\n")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cases", nargs="+", type=Path)
    parser.add_argument("--game", required=True, type=Path)
    parser.add_argument("--prefix", required=True, type=Path)
    parser.add_argument("--dll", required=True, type=Path)
    parser.add_argument("--wine", required=True, type=Path)
    parser.add_argument("--frames", type=int, default=4500)
    parser.add_argument("--timeout", type=int, default=600)
    parser.add_argument("--jobs", type=int, default=2)
    parser.add_argument("--color-bits", type=int, choices=(16, 32), default=32)
    parser.add_argument("--alsa-config", type=Path)
    parser.add_argument("--fullscreen", action="store_true")
    parser.add_argument("--post-terminal", type=int, default=0)
    parser.add_argument("--hardware", action="store_true", help="clear original compatibility flags; record runtime acceptance in proxy log")
    parser.add_argument("--pool-stride", type=int, default=60)
    parser.add_argument("--checkpoint", type=int, action='append', default=[])
    parser.add_argument("--only-checkpoints", action='store_true', help='Capture explicit checkpoints only; preserve complete scalar/pool tracing')
    parser.add_argument("--stage-checkpoint", action='append', default=[], metavar='STAGE:FRAME')
    parser.add_argument("--diagnostic-vms", action='store_true')
    parser.add_argument("--diagnostic-gui", action='store_true', help='Read-only post-calculation GUI fields each Present')
    parser.add_argument("--rerun", action="store_true")
    args = parser.parse_args()
    args.game, args.prefix, args.dll, args.wine = (p.resolve() for p in
        (args.game, args.prefix, args.dll, args.wine))
    if args.game in [case.resolve() / "game" for case in args.cases]:
        parser.error("the base game must differ from every scenario game")
    with ThreadPoolExecutor(max_workers=args.jobs) as executor:
        futures = {executor.submit(run_case, case, args): case for case in args.cases}
        for future in as_completed(futures):
            result = future.result()
            print(json.dumps({key: result[key] for key in
                ("id", "status", "elapsed_seconds", "terminal")}), flush=True)


if __name__ == "__main__":
    main()
