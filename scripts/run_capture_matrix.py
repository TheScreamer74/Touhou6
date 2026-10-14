#!/usr/bin/env python3
"""Capture and compare original image checkpoints serially with a pinned build.

Preflight checks every case before opening the GPU. Existing captures survive
under their original names; a final run uses a new output directory. Complete
image reports retain all pixels and are written one scenario at a time.
"""
import argparse
import hashlib
import json
from pathlib import Path
import re

from capture_port_scenario import COMPLETE_OUTPUTS, capture, original_pool_frames
from compare_capture_scenario import run as compare
from verify_audio_reference import load_original_collection, verify_original_collection
from verify_scenario_reference import compare as compare_simulation


def digest(path):
    with path.open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def prepare_original_collection(cases, args):
    explicit = getattr(args, "original_attestation", None)
    args.original_collection = {}
    args.original_collection_provenance = None
    args.original_collection_path = None
    if not (args.require_matrix or getattr(args, "complete_reference", False)) and explicit is None:
        return
    if explicit is None:
        parents = {case.resolve().parent for case in cases}
        if len(parents) != 1:
            raise ValueError("Provide --original-attestation for cases with different parents")
        explicit = parents.pop() / "original-audio-collection-attestation.json"
    args.original_collection_path = explicit.resolve()
    args.original_collection, args.original_collection_provenance = load_original_collection(
        args.original_collection_path)


def verify_reference_collection(case, args):
    provenance = args.original_collection_provenance
    if provenance is None:
        if args.require_matrix or getattr(args, "complete_reference", False):
            raise ValueError("The final image matrix requires the original collection attestation")
        return
    if digest(args.original_collection_path) != provenance["sha256"]:
        raise ValueError("Original collection attestation changed after preflight")
    attested = args.original_collection.get(case.name)
    if attested is None:
        raise ValueError("Case is absent from original collection attestation")
    verify_original_collection(case, attested)


def current_reference_snapshot(case):
    original = case / "original"
    return {"execution_sha256": digest(original / "execution.json"),
            "scalar_sha256": digest(original / "replay-state.csv"),
            "images_sha256": {image.name: digest(image)
                              for image in sorted((original / "capture").iterdir())
                              if re.fullmatch(r"replay_stage\d+_frame\d+\.(?:bmp|png)", image.name)}}


def verify_reference_snapshot(case, args):
    verify_reference_collection(case, args)
    if current_reference_snapshot(case) != args.reference_snapshots[case.name]:
        raise ValueError(f"{case.name}: original reference changed after preflight")


def validate_supplement(original, execution):
    path = original / "image-supplement.json"
    if not path.exists():
        return set()
    supplement = json.loads(path.read_text())
    execution_name = supplement["supplement_execution_file"]
    if Path(execution_name).name != execution_name:
        raise ValueError("Supplement execution path must be one filename")
    execution_path = original / execution_name
    supplement_execution = json.loads(execution_path.read_text())
    execution_hash = digest(execution_path)
    if (supplement["version"] != 1 or
            supplement["original_execution_sha256"] != digest(original / "execution.json") or
            supplement["supplement_execution_sha256"] != execution_hash or
            supplement_execution.get("status") != "completed-process" or
            supplement_execution.get("process_returncode") != 0):
        raise ValueError("Incomplete or changed original image supplement")
    for key in ("input_sha256", "exe_sha256", "proxy_sha256", "terminal"):
        if supplement[key] != supplement_execution[key]:
            raise ValueError(f"Supplement attribution differs for {key}")
    for key in ("input_sha256", "exe_sha256", "terminal"):
        if supplement[key] != execution[key]:
            raise ValueError(f"Supplement changes original {key}")
    if supplement["full_scalar_trace_sha256"] != digest(original / "replay-state.csv"):
        raise ValueError("Supplement scalar trace differs from preserved original")
    images = supplement["images"]
    if set(images) != set(supplement_execution["captures"]):
        raise ValueError("Supplement images differ from its own execution receipt")
    observed = set()
    stage, terminal = execution["terminal"]["stage"], execution["terminal"]["frame"]
    for name, image in images.items():
        match = re.fullmatch(r"replay_stage(\d+)_frame(\d+)\.bmp", name)
        if (not match or tuple(map(int, match.groups())) != (image["stage"], image["frame"]) or
                image["stage"] != stage or image["supplement_execution_sha256"] != execution_hash or
                image["sha256"] != digest(original / "capture" / name)):
            raise ValueError(f"Changed or misattributed supplemental image {name}")
        observed.add(image["frame"])
    windows = [[frame, min(frame + 2, terminal)] for frame in range(
        ((supplement["last_previous_image_frame"] // 1800) + 1) * 1800, terminal + 1, 1800)]
    if (supplement["required_terminal_frames"] != [terminal - 1, terminal] or
            not {terminal - 1, terminal} <= observed or
            supplement["required_cadence_windows"] != windows or
            any(not any(lo <= frame <= hi for frame in observed) for lo, hi in windows)):
        raise ValueError("Supplement misses terminal or declared cadence images")
    return set(images)


def preflight(cases, args):
    args.complete_reference = args.require_matrix or getattr(args, "complete_reference", False)
    if "target/release" in str(args.binary):
        raise ValueError("Use a private pinned build, not the shared target/release executable")
    binary_hash = digest(args.binary)
    build_path = args.binary.with_name("build-source.json")
    build = json.loads(build_path.read_text())
    if build["binary_sha256"] != binary_hash:
        raise ValueError("Pinned executable differs from build-source.json")
    fingerprint = hashlib.sha256(json.dumps(build["runtime_files_sha256"], sort_keys=True,
                                           separators=(",", ":")).encode()).hexdigest()
    if build.get("runtime_source_sha256") != fingerprint:
        raise ValueError("Invalid source manifest fingerprint")
    names = [case.name for case in cases]
    if len(set(names)) != len(names):
        raise ValueError("Duplicate scenario IDs")
    if args.require_matrix:
        if not getattr(args, "menu_entry", True):
            raise ValueError("The final matrix requires natural Replay-menu entry")
        expected = {f"replay-c{character}-d{difficulty}-s{stage}"
                    for character in range(4) for difficulty in range(5)
                    for stage in (range(1, 6) if difficulty == 0 else
                                  range(7, 8) if difficulty == 4 else range(1, 7))}
        if len(names) != len(expected) or set(names) != expected:
            raise ValueError(f"Require all 96 legal IDs; missing={sorted(expected-set(names))}, "
                             f"extra={sorted(set(names)-expected)}")
    prepare_original_collection(cases, args)
    args.reference_snapshots = {}
    failures = []
    for case in cases:
        try:
            original = case / "original"
            execution = json.loads((original / "execution.json").read_text())
            if (execution.get("status") != "completed-process" or
                    execution.get("process_returncode") != 0 or
                    execution.get("id") != case.name or
                    execution.get("exe_sha256") != "7f38496b31b8625196900a69cd1bfed243cab1f9612073e7881dc354b876fd39" or
                    execution.get("input_sha256") != digest(case / "input.rpy")):
                raise ValueError("Incomplete or stale original execution receipt")
            verify_reference_collection(case, args)
            provenance = json.loads((case / "input-provenance.json").read_text())
            hardware = (getattr(args, "hardware_texture_blending", False) or
                        provenance.get("hardware_texture_blending", False))
            if (execution["color_bits"] != getattr(args, "color_bits", 32) or
                    execution.get("hardware_profile", False) != hardware or
                    execution.get("god", False) != provenance.get("god", False)):
                raise ValueError("Port display/blending/god profile differs from original execution")
            if not (original / "replay-state.csv").is_file():
                raise ValueError("Missing original scalar checkpoints")
            declared = set(execution["captures"]) | validate_supplement(original, execution)
            checkpoints = set()
            images = set()
            for image in (original / "capture").iterdir():
                match = re.fullmatch(r"replay_stage(\d+)_frame(\d+)\.(?:bmp|png)", image.name)
                if match:
                    key = tuple(map(int, match.groups()))
                    if key in checkpoints:
                        raise ValueError(f"Duplicate original image checkpoint {key}")
                    checkpoints.add(key)
                    images.add(image.name)
            if not checkpoints:
                raise ValueError("Missing original images")
            if images != declared:
                raise ValueError("Original images differ from their attributed execution lists")
            terminal = execution["terminal"]
            if args.complete_reference:
                original_pool_frames(original, terminal)
            if args.require_matrix and (terminal["stage"], terminal["frame"]) not in checkpoints:
                raise ValueError("Missing original semantic terminal image; supplement required")
            if (case / args.name).exists():
                raise ValueError(f"Output {args.name} already exists; choose a new run name")
            if args.complete_reference and (case / (args.name + "-game")).exists():
                raise ValueError("Complete reference capture requires fresh private storage")
            args.reference_snapshots[case.name] = current_reference_snapshot(case)
        except (OSError, ValueError, KeyError) as error:
            failures.append(f"{case.name}: {error}")
    if failures:
        raise ValueError("Preflight failed before GPU launch:\n" + "\n".join(failures))
    return binary_hash, build


def compare_complete_simulation(case, actual, captured, args):
    if not captured.get("complete_reference"):
        raise ValueError("Complete simulation comparison requires a complete-reference capture")
    for name in COMPLETE_OUTPUTS:
        if captured.get("outputs_sha256", {}).get(name) != digest(actual / name):
            raise ValueError(f"Native complete-reference output changed before comparison: {name}")
    original = case / "original"
    terminal = json.loads((original / "terminal.json").read_text())
    result = compare_simulation(original / "replay-state.csv", actual / "replay-state.csv",
                                original / "pools.jsonl", actual / "pools.jsonl", terminal["stage"],
                                original / "terminal.json", actual / "replay-state.csv.terminal.jsonl", None)
    result.update({"id": case.name, "binary_sha256": captured["binary_sha256"],
                   "input_sha256": captured["input_sha256"],
                   "original_collection_attestation": args.original_collection_provenance,
                   "reference_current_collection_outputs_sha256":
                       captured["reference_current_collection_outputs_sha256"]})
    (actual / "comparison.json").write_text(json.dumps(result, indent=2) + "\n")
    return result


def update_aggregate(receipt):
    cases = receipt["cases"]
    complete = receipt["status"] == "completed" and len(cases) == receipt["case_count"] and bool(cases)
    receipt["completed_cases"] = len(cases)
    receipt["image_passed_cases"] = sum(case["image_passed"] for case in cases)
    receipt["image_pairs"] = sum(case["pairs"] for case in cases)
    receipt["image_passed"] = complete and all(case["image_passed"] for case in cases)
    if receipt["complete_reference"]:
        receipt["simulation_passed_cases"] = sum(case["simulation_passed"] for case in cases)
        receipt["compared_observations"] = sum(case["compared_observations"] for case in cases)
        receipt["compared_pool_checkpoints"] = sum(case["compared_pool_checkpoints"] for case in cases)
        receipt["simulation_passed"] = complete and all(case["simulation_passed"] for case in cases)
    else:
        receipt["simulation_passed"] = None
    receipt["passed"] = receipt["image_passed"] and (
        receipt["simulation_passed"] if receipt["complete_reference"] else True)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("cases", type=Path, nargs="+")
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--game", required=True, type=Path)
    parser.add_argument("--name", required=True, help="New capture directory name for this run")
    parser.add_argument("--color-bits", type=int, choices=(16, 32), default=32)
    parser.add_argument("--hardware-texture-blending", action="store_true")
    parser.add_argument("--menu-entry", action=argparse.BooleanOptionalAction, default=True)
    parser.add_argument("--timeout", type=float, default=300)
    parser.add_argument("--require-matrix", action="store_true")
    parser.add_argument("--complete-reference", action="store_true",
                        help="Compare every scalar observation, original pool checkpoint and terminal in the same pass; always enabled for --require-matrix")
    parser.add_argument("--original-attestation", type=Path,
                        help="Original audio/state/pool collection attestation; required automatically for the matrix")
    parser.add_argument("--check-only", action="store_true", help="Preflight without opening the GPU")
    parser.add_argument("--output", required=True, type=Path)
    args = parser.parse_args()
    args.binary, args.game = args.binary.resolve(), args.game.resolve()
    args.rerun = False
    cases = sorted(case.resolve() for case in args.cases)
    if Path(args.name).name != args.name or args.name in (".", ".."):
        parser.error("--name must be one directory component")
    try:
        binary_hash, build = preflight(cases, args)
    except (OSError, ValueError, KeyError) as error:
        parser.error(str(error))
    if args.check_only:
        print(json.dumps({"preflight": "passed", "cases": len(cases),
                          "binary_sha256": binary_hash, "gpu_started": False,
                          "complete_reference": args.complete_reference,
                          "original_collection_attestation": args.original_collection_provenance,
                          "reference_images": sum(len(item["images_sha256"])
                                                  for item in args.reference_snapshots.values())}))
        return
    results = []
    receipt = {"status": "running", "serial": True, "required_matrix": args.require_matrix,
               "case_count": len(cases), "color_bits": args.color_bits,
               "complete_reference": args.complete_reference,
               "menu_entry": args.menu_entry,
               "binary_sha256": binary_hash, "build_provenance": build,
               "original_collection_attestation": args.original_collection_provenance,
               "original_reference_snapshot_scope": "Current local reference hashes observed during preflight and rechecked before capture/comparison. Primary image hashes are not retroactive execution-time provenance; supplemental images retain their separate execution hash binding.",
               "original_reference_snapshots": args.reference_snapshots,
               "cases": results}
    scripts = Path(__file__).resolve().parent
    receipt["harness_sha256"] = {name: digest(scripts / name) for name in (
        "run_capture_matrix.py", "capture_port_scenario.py", "compare_capture_scenario.py",
        "compare_capture_images.py", "verify_audio_reference.py", "verify_scenario_reference.py")}
    args.output.parent.mkdir(parents=True, exist_ok=True)
    try:
        for case in cases:
            verify_reference_snapshot(case, args)
            captured = capture(case, case / "original" / "capture", args)
            if (captured["status"] != "completed-process" or captured["returncode"] != 0 or
                    captured["binary_sha256"] != binary_hash):
                raise ValueError(f"{case.name}: capture failed or executable hash changed")
            verify_reference_snapshot(case, args)
            simulation = (compare_complete_simulation(case, case / args.name, captured, args)
                          if args.complete_reference else None)
            report_path = case / (args.name + "-comparison")
            report = compare(case / "original" / "capture", case / args.name, report_path)
            result = {"id": case.name, "pairs": report["pairs"],
                      "missing_images": len(report["missing_images"]),
                      "scalar_differing_checkpoints": len(report["scalar_differences"]),
                      "image_passed": report["passed"],
                      "simulation_passed": simulation["passed"] if simulation is not None else None,
                      "passed": report["passed"] and (simulation["passed"] if simulation is not None else True),
                      "compared_observations": simulation["compared_observations"] if simulation else 0,
                      "compared_pool_checkpoints": simulation["compared_pool_checkpoints"] if simulation else 0,
                      "simulation_comparison": str(case / args.name / "comparison.json") if simulation else None,
                      "elapsed_seconds": captured["elapsed_seconds"],
                      "summary": str(report_path / "summary.json")}
            results.append(result)
            update_aggregate(receipt)
            args.output.write_text(json.dumps(receipt, indent=2) + "\n")
            print(json.dumps(result), flush=True)
        receipt["status"] = "completed"
    except Exception as error:
        receipt["status"] = "failed"
        receipt["error"] = str(error)
        raise
    finally:
        update_aggregate(receipt)
        args.output.write_text(json.dumps(receipt, indent=2) + "\n")
    if not receipt["passed"]:
        raise SystemExit(1)


if __name__ == "__main__":
    main()
