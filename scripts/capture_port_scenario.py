#!/usr/bin/env python3
"""Capture native port at checkpoint IDs that a real original run reached.

The input replay drives simulation. This does not transplant original state,
rescale images, synthesize reference screenshots, or change the public site.
Every case receives separate writable storage. Images remain unmodified PNGs.
"""
import argparse
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import subprocess
import time

from verify_audio_reference import load_original_collection, verify_original_collection

COMPLETE_OUTPUTS = ("audio.csv", "replay-state.csv", "pools.jsonl",
                    "replay-state.csv.terminal.jsonl")


def digest(path):
    with path.open('rb') as source:
        return hashlib.file_digest(source, 'sha256').hexdigest()


def pinned_executable(case, binary, complete_reference):
    binary = binary.resolve()
    binary_hash = digest(binary)
    build_path = binary.with_name("build-source.json")
    if build_path.exists():
        build = json.loads(build_path.read_text())
        fingerprint = hashlib.sha256(json.dumps(build["runtime_files_sha256"], sort_keys=True,
                                               separators=(",", ":")).encode()).hexdigest()
        if (build.get("binary_sha256") != binary_hash or
                build.get("runtime_source_sha256") != fingerprint):
            raise ValueError("Pinned executable/build provenance mismatch")
        if "target/release" in str(binary):
            raise ValueError("Build provenance requires a private immutable executable")
        return binary, binary_hash, build
    if complete_reference:
        raise ValueError("Complete reference capture requires a pin with build-source.json")
    # Diagnostic-only builds without source provenance still need an immutable
    # executable copy. Never detach a real pin from its adjacent source receipt.
    binaries = case.parent / "bin"
    binaries.mkdir(exist_ok=True)
    snapshot = binaries / f"th06-{binary_hash}"
    if not snapshot.exists():
        shutil.copyfile(binary, snapshot)
        snapshot.chmod(0o755)
    if digest(snapshot) != binary_hash:
        raise ValueError("Immutable binary snapshot hash mismatch")
    return snapshot.resolve(), binary_hash, None


def original_pool_frames(original, terminal):
    frames, previous = [], None
    with (original / "pools.jsonl").open() as source:
        for line in source:
            checkpoint = json.loads(line)
            key = checkpoint["stage"], checkpoint["frame"]
            if key[0] != terminal["stage"] or key[1] < 1 or (previous is not None and key <= previous):
                raise ValueError("Original pool checkpoint IDs must be unique, ordered and in the terminal stage")
            frames.append(key[1])
            previous = key
    if not frames:
        raise ValueError("Complete capture requires nonempty original pool checkpoints")
    return frames


def complete_original_reference(case, args):
    path = getattr(args, "original_collection_path", None)
    if path is None:
        path = getattr(args, "original_attestation", None) or (
            case.parent / "original-audio-collection-attestation.json")
    collection, provenance = load_original_collection(Path(path))
    attested = collection.get(case.name)
    if attested is None:
        raise ValueError("Complete capture case is absent from original collection attestation")
    verify_original_collection(case, attested)
    execution = json.loads((case / "original/execution.json").read_text())
    if (execution.get("id") != case.name or execution.get("status") != "completed-process" or
            execution.get("process_returncode") != 0 or
            execution.get("exe_sha256") != "7f38496b31b8625196900a69cd1bfed243cab1f9612073e7881dc354b876fd39" or
            execution.get("input_sha256") != digest(case / "input.rpy")):
        raise ValueError("Complete capture requires the verified successful original execution")
    terminal = json.loads((case / "original/terminal.json").read_text())
    frames = original_pool_frames(case / "original", terminal)
    return execution, terminal, frames, provenance, attested["sha256"]


def capture(case, reference, args):
    complete_reference = getattr(args, "complete_reference", False)
    reference_execution, terminal, pool_frames, original_provenance, original_hashes = None, None, [], None, None
    if complete_reference:
        (reference_execution, terminal, pool_frames, original_provenance,
         original_hashes) = complete_original_reference(case, args)
    snapshot, binary_hash, build = pinned_executable(case, args.binary, complete_reference)
    harness_hashes = {name: digest(Path(__file__).resolve().parent / name) for name in (
        "capture_port_scenario.py", "verify_audio_reference.py")}
    checkpoints = set()
    originals = {}
    for path in reference.iterdir():
        match = re.fullmatch(r"replay_stage(\d+)_frame(\d+)\.(?:bmp|png)", path.name)
        if match:
            stage, frame = map(int, match.groups())
            checkpoints.add(frame)
            originals[f"{stage}:{frame}"] = {"path": str(path.resolve()), "sha256": digest(path)}
    if not checkpoints:
        raise ValueError(f"No original stage/frame screenshots in {reference}")
    if complete_reference and f"{terminal['stage']}:{terminal['frame']}" not in originals:
        raise ValueError("Complete reference capture requires its actual original terminal image")
    replay = case / "input.rpy"
    provenance_path = case / "input-provenance.json"
    provenance = json.loads(provenance_path.read_text()) if provenance_path.exists() else {}
    output_name = getattr(args, "name", None) or f"port{args.color_bits}"
    if Path(output_name).name != output_name or output_name in (".", ".."):
        raise ValueError("Capture output name must be one directory component")
    storage = case / f"{output_name}-game"
    if complete_reference and storage.exists():
        raise ValueError("Complete reference capture requires fresh private storage; choose a new --name")
    out = case / output_name
    out.mkdir(exist_ok=True)
    receipt = out / "capture-execution.json"
    if receipt.exists() and not args.rerun:
        raise ValueError(f"Existing evidence {receipt}; select --rerun explicitly to replace it")
    # Interrupted reruns must not inherit images from a longer successful run.
    # Only this fixture's generated files are removed; unrelated files survive.
    for path in out.iterdir():
        if re.fullmatch(r"replay_stage\d+_frame\d+\.png", path.name) or path.name in (
                "capture-execution.json", "execution.json", "comparison.json", "checkpoints.csv",
                "replay-state.csv", "replay-state.csv.terminal.jsonl", "pools.jsonl", "options.csv",
                "lifecycle.csv", "stages.csv", "transitions.csv", "navigation.csv", "results.csv", "boss-ui.csv", "terminal.jsonl", "audio.csv"):
            path.unlink()
    storage.mkdir(exist_ok=True)
    for path in args.game.iterdir():
        if (path.suffix.upper() == ".DAT" and path.name.lower() != "score.dat") or path.name in ("bgm", "gm.sf2", "msgothic.ttc", "msgothic.ttf"):
            target = storage / path.name
            if not target.exists():
                target.symlink_to(path.resolve(), target_is_directory=path.is_dir())
    # Starting storage is deterministic and private; no source score/config is
    # copied. ReplayManager restores its recorded lives, bombs, rank and power.
    (storage / "th06-options.bin").write_bytes(bytes([1, 2, 3, args.color_bits == 16, 1, 1, args.color_bits != 16, 1]))
    menu_entry = getattr(args, "menu_entry", True)
    entry_arguments = ["--replay", str(replay.resolve())]
    if menu_entry:
        replay_directory = storage / "replay"
        replay_directory.mkdir(exist_ok=True)
        (replay_directory / "th6_01.rpy").write_bytes(replay.read_bytes())
        entry_script = out / "menu-entry-input.txt"
        entry_script.write_text("key 180 184 Q\nkey 240 244 DOWN\nkey 270 274 DOWN\ntap 350 60 5 Z\n")
        entry_arguments = ["--input-script", str(entry_script.resolve())]
    last_frame = max(max(checkpoints), terminal["frame"] if terminal is not None else 0)
    command = [str(snapshot.resolve()), "--game-dir", str(storage.resolve()),
               *entry_arguments, "--capture-dir", str(out.resolve()),
               "--capture-frames", ",".join(map(str, sorted(checkpoints))),
               "--capture-clock", "replay", "--replay-pools", str((out / "pools.jsonl").resolve()),
               "--frames", str(last_frame + (700 if menu_entry else 200)), f"--color-{args.color_bits}bit",
               # The original draws no hitbox; the port's extension defaults to Focus.
               "--hitbox", "off"]
    if complete_reference:
        command.extend(["--complete-reference", "--capture-pool-frames", ",".join(map(str, pool_frames))])
    if provenance.get("god", False):
        command.append("--god")
    hardware_blending = args.hardware_texture_blending or provenance.get("hardware_texture_blending", False)
    command.append("--hardware-texture-blending" if hardware_blending else "--software-texture-blending")
    env = dict(os.environ, TH06_AUDIO_TRACE=str((out / "audio.csv").resolve()))
    # Never inherit an undeclared simulation override from the invoking shell.
    for variable in ("TH06_GOD", "TH06_NO_DIALOGUE", "TH06_NOSHOOT", "TH06_BOMB",
                     "TH06_FONT", "TH06_SOUNDFONT"):
        env.pop(variable, None)
    begin = time.monotonic()
    status, returncode = "completed-process", None
    try:
        with (out / "capture-run.log").open("wb") as log:
            run = subprocess.run(command, stdout=log, stderr=subprocess.STDOUT, env=env,
                                 timeout=args.timeout)
            returncode = run.returncode
    except subprocess.TimeoutExpired:
        status = "timeout"
    if digest(snapshot) != binary_hash:
        raise ValueError("Pinned executable changed during capture")
    if build is not None and json.loads(snapshot.with_name("build-source.json").read_text()) != build:
        raise ValueError("Pinned build provenance changed during capture")
    if complete_reference:
        _, _, _, current_provenance, current_hashes = complete_original_reference(case, args)
        if current_provenance != original_provenance or current_hashes != original_hashes:
            raise ValueError("Original collection changed during complete capture")
    missing = [name for name in COMPLETE_OUTPUTS if not (out / name).is_file()] if complete_reference else []
    if missing and status == "completed-process" and returncode == 0:
        status = "incomplete-output"
    result = {"id": case.name, "case": case.name, "reference_kind": "real-original-capture-checkpoint-IDs",
              "drawing_order": "Original GameWindow::Render draws the pre-calc state, then runs 1–3 calculation ticks, then Presents. Checkpoint ID/state is post-calc; draw_frame is recorded separately in checkpoints.csv.",
              "fps_clock": "Declared synthetic 60-Hz original draw-chain clock, starting at fixture entry, like refcap's timeGetTime hook; no label is copied or forced to 60.",
              "binary_sha256": binary_hash, "binary_after_sha256": digest(snapshot),
              "replay_sha256": digest(replay), "input_sha256": digest(replay), "build_provenance": build,
              "complete_reference": complete_reference, "requested_pool_frames": pool_frames,
              "menu_entry": menu_entry,
              "entry_history": ("Original menu input schedule: Q180, DOWN240/270, five Z taps from350 every60 frames; private replay/th6_01.rpy. Menu backbuffer and FPS history are drawn before replay frame1."
                                if menu_entry else "Diagnostic direct replay entry: original frame1 instead retains the Replay menu. This difference is retained in image metrics."),
              "color_bits": args.color_bits, "god": provenance.get("god", False),
              "hardware_texture_blending": hardware_blending,
              "windowed": args.color_bits != 16, "framebuffer_override": args.color_bits,
              "force_16bit_textures": False,
              "command": command, "requested_frames": sorted(checkpoints),
              "originals": originals, "status": status, "returncode": returncode,
              "required_outputs_missing": missing,
              "reference_execution_sha256": digest(case / "original/execution.json") if reference_execution else None,
              "reference_proxy_sha256": reference_execution["proxy_sha256"] if reference_execution else None,
              "original_collection_attestation_sha256": original_provenance["sha256"] if original_provenance else None,
              "original_collection_attestation": original_provenance,
              "reference_current_collection_outputs_sha256": original_hashes,
              "harness_sha256": harness_hashes,
              "outputs_sha256": {path.name: digest(path) for path in out.iterdir()
                                 if path.is_file() and (path.suffix in (".png", ".csv", ".jsonl")
                                                      or path.name == "menu-entry-input.txt")},
              "elapsed_seconds": time.monotonic() - begin,
              "parity": "Not established by capture completion; compare simulation snapshots and images separately."}
    receipt.write_text(json.dumps(result, indent=2) + "\n")
    if complete_reference:
        (out / "execution.json").write_text(json.dumps(result, indent=2) + "\n")
    return result


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("case", type=Path)
    parser.add_argument("reference", type=Path, help="Original BMP/PNG checkpoint directory")
    parser.add_argument("--game", type=Path, required=True, help="Read-only source assets")
    parser.add_argument("--binary", type=Path, default=Path("target/release/th06"))
    parser.add_argument("--color-bits", type=int, choices=(16, 32), required=True)
    parser.add_argument("--name", help="New capture directory name; default port16/port32")
    parser.add_argument("--menu-entry", action=argparse.BooleanOptionalAction, default=True,
                        help="Use original title/Replay menu input schedule (default); --no-menu-entry is diagnostic direct replay")
    parser.add_argument("--hardware-texture-blending", action="store_true", help="Original opts.useSwTextureBlending=false profile")
    parser.add_argument("--timeout", type=float, default=180)
    parser.add_argument("--rerun", action="store_true")
    parser.add_argument("--complete-reference", action="store_true",
                        help="Record every simulation observation, original pool checkpoints and terminal alongside PNGs")
    parser.add_argument("--original-attestation", type=Path,
                        help="Required original collection attestation for complete-reference; defaults beside cases")
    args = parser.parse_args()
    result = capture(args.case.resolve(), args.reference.resolve(), args)
    print(json.dumps(result, indent=2))
    raise SystemExit(0 if result["returncode"] == 0 and result["status"] == "completed-process" else 1)
