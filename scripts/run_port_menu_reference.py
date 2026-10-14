#!/usr/bin/env python3
"""Run a natural MainMenu scenario with private storage and a pinned build.

Each output preserves input, replay, build and original-execution hashes before
execution. Only the declared menu-state/cursor/timer interval is compared; the
single final screenshot and port audio trace do not claim image/audio parity.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import time

from verify_menu_navigation import compare, indexed


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def run(args):
    case, binary, assets = args.case.resolve(), args.binary.resolve(), args.game.resolve()
    if "target/release" in str(binary):
        raise ValueError("Use an immutable pinned executable")
    build = json.loads(binary.with_name("build-source.json").read_text())
    if build["binary_sha256"] != digest(binary):
        raise ValueError("Pinned executable differs from its source receipt")
    original_path = case / "execution.json"
    original = json.loads(original_path.read_text())
    if (original.get("status") != "completed-process" or original.get("returncode") != 0
            or original.get("id") != case.name
            or original["input_sha256"] != digest(case / "input.txt")):
        raise ValueError("Original menu execution is failed or its input changed")
    end = original["stop_present"]
    if not 0 <= args.start <= end:
        raise ValueError("Invalid required interval")
    reference_trace = case / "game" / "menus.csv"
    original_rows = indexed(reference_trace, "present")
    for present in range(args.start, end + 1):
        if present not in original_rows or original_rows[present]["supervisor_state"] != "1":
            raise ValueError(f"Missing live original MainMenu observation at Present {present}")
    expected_replays = original.get("replay_files", [])
    if len(expected_replays) != len(args.replay_file):
        raise ValueError("Provide exactly the original numbered replay files")
    for expected, source in zip(expected_replays, args.replay_file, strict=True):
        if (Path(expected["name"]).name != expected["name"] or
                expected["sha256"] != digest(source)):
            raise ValueError("Replay fixture differs from the genuine original execution")
    output, storage = case / args.name, case / (args.name + "-game")
    if output.exists() or storage.exists():
        raise ValueError("Refusing to overwrite menu evidence; choose a new --name")
    output.mkdir()
    storage.mkdir()
    for source in assets.iterdir():
        if ((source.suffix.upper() == ".DAT" and source.name.lower() != "score.dat") or
                source.name in ("bgm", "gm.sf2", "msgothic.ttc", "msgothic.ttf")):
            (storage / source.name).symlink_to(source.resolve(), target_is_directory=source.is_dir())
    # Same cfg bytes as menu_scenarios.py: lives2, bombs3, 32-bit/WAV/SFX,
    # Normal default, windowed. No score file is inherited from another run.
    (storage / "th06-options.bin").write_bytes(bytes([1, 2, 3, 0, 1, 1, 1, 1]))
    if expected_replays:
        (storage / "replay").mkdir()
        for expected, source in zip(expected_replays, args.replay_file, strict=True):
            shutil.copyfile(source, storage / "replay" / expected["name"])
    command = [str(binary), "--game-dir", str(storage), "--input-script", str(case / "input.txt"),
               "--capture-dir", str(output), "--capture-clock", "update", "--capture-frames",
               str(end + 1), "--frames", str(end + 1)]
    env = dict(os.environ, TH06_AUDIO_TRACE=str(output / "audio.csv"))
    cleared = ("TH06_GOD", "TH06_NO_DIALOGUE", "TH06_BOMB", "TH06_NOSHOOT",
               "TH06_FONT", "TH06_SOUNDFONT")
    for name in cleared:
        env.pop(name, None)
    receipt = {"status": "running", "id": original["id"], "command": command,
               "binary_sha256": build["binary_sha256"], "build_provenance": build,
               "input_sha256": original["input_sha256"], "replay_files": expected_replays,
               "original_execution_sha256": digest(original_path),
               "reference_trace_sha256": digest(reference_trace),
               "required_present_interval": [args.start, end], "cleared_overrides": cleared,
               "harness_sha256": {path.name: digest(path) for path in (
                   Path(__file__), Path(__file__).with_name("verify_menu_navigation.py"))}}
    receipt_path = output / "execution.json"
    receipt_path.write_text(json.dumps(receipt, indent=2) + "\n")
    started = time.monotonic()
    try:
        with (output / "run.log").open("w") as log:
            process = subprocess.run(command, env=env, stdout=log, stderr=subprocess.STDOUT,
                                     timeout=args.timeout)
        receipt["returncode"] = process.returncode
        if process.returncode:
            raise ValueError(f"Native process exited {process.returncode}")
        if (digest(binary) != build["binary_sha256"] or
                digest(case / "input.txt") != original["input_sha256"] or
                digest(reference_trace) != receipt["reference_trace_sha256"]):
            raise ValueError("Executable, input or original observations changed during execution")
        result = compare(case / "game" / "menus.csv", output / "navigation.csv",
                         "navigation", args.start, end)
        (output / "comparison.json").write_text(json.dumps(result, indent=2) + "\n")
        receipt["status"] = "completed-process"
        receipt["passed"] = result["passed"]
        print(json.dumps({"id": original["id"], "passed": result["passed"],
                          "compared_observations": result["compared_observations"],
                          "differences": result["differences"]}), flush=True)
    except Exception as error:
        receipt["status"] = "failed"
        receipt["error"] = str(error)
        raise
    finally:
        receipt["elapsed_seconds"] = time.monotonic() - started
        receipt["outputs_sha256"] = {path.name: digest(path) for path in output.iterdir()
                                     if path.is_file() and path.name != "execution.json"}
        receipt_path.write_text(json.dumps(receipt, indent=2) + "\n")
    return receipt["passed"]


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("case", type=Path)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--game", type=Path, required=True)
    parser.add_argument("--name", required=True)
    parser.add_argument("--replay-file", type=Path, action="append", default=[])
    parser.add_argument("--start", type=int, default=180)
    parser.add_argument("--timeout", type=float, default=90)
    args = parser.parse_args()
    if Path(args.name).name != args.name or args.name in (".", ".."):
        parser.error("--name must be one directory component")
    raise SystemExit(0 if run(args) else 1)
