#!/usr/bin/env python3
"""Compare private executable audio traces with the port's diagnostics.

Only event/clock metadata is emitted. Never commit the supplied MIDI, WAV,
SoundFont or extracted game assets alongside a validation receipt.
"""
import argparse
import csv
import hashlib
import itertools
import json
from pathlib import Path


def digest(path):
    result = hashlib.sha256()
    with path.open("rb") as source:
        for block in iter(lambda: source.read(1024 * 1024), b""):
            result.update(block)
    return result.hexdigest()


def compare_midi(reference, port):
    differences, count = [], 0
    for index, (expected, actual) in enumerate(itertools.zip_longest(
            csv.DictReader((reference / "midi-events.csv").open()),
            csv.DictReader((port / "midi-events.csv").open()))):
        count += 1
        if expected != actual and len(differences) < 20:
            differences.append({"message": index, "expected": expected, "actual": actual})
    expected = list(csv.DictReader((reference / "midi-summary.csv").open()))
    actual = list(csv.DictReader((port / "midi-summary.csv").open()))
    return {
        "tracks": len(expected), "callbacks": sum(int(row["callbacks"]) for row in expected),
        "messages_compared": count, "clock_summaries_identical": expected == actual,
        "differences": differences,
        "passed": bool(count) and len(expected) == 17 and expected == actual and not differences,
        "reference_sha256": {name: digest(reference / name) for name in ("midi-events.csv", "midi-summary.csv")},
        "port_sha256": {name: digest(port / name) for name in ("midi-events.csv", "midi-summary.csv")},
    }


def event_rows(path, event_types, first, last, stages=None, clock="replay"):
    result = {}
    with path.open() as source:
        for row in csv.DictReader(source):
            frame = int(row["present" if clock == "present" else "frame"])
            if row["event"] == "HOOK_ERROR":
                raise ValueError(f"Original executable hook verification failed: {row}")
            stage = int(row["stage"])
            if first <= frame <= last and row["event"] in event_types and (stages is None or stage in stages):
                key = (stage, frame, "ordered-events")
                value = row["value"].replace(".mid", "").replace(".wav", "")
                result.setdefault(key, []).append([row["event"], value])
    return result


def compare_events(reference, port, event_types, first, last, stages=None, clock="replay"):
    expected = event_rows(reference, event_types, first, last, stages, clock)
    actual = event_rows(port, event_types, first, last, stages, clock)
    differences, difference_count = [], 0
    for key in sorted(expected.keys() | actual.keys()):
        if expected.get(key, []) != actual.get(key, []):
            difference_count += 1
            if len(differences) < 30:
                differences.append({"stage": key[0], "frame": key[1], "event": key[2],
                                    "expected": expected.get(key, []), "actual": actual.get(key, [])})
    return {
        "reference_sha256": digest(reference), "port_sha256": digest(port),
        "frame_range": [first, last], "event_types": event_types,
        "stages": stages, "clock": clock,
        "reference_events": sum(map(len, expected.values())),
        "port_events": sum(map(len, actual.values())),
        "different_frame_event_groups": difference_count, "first_differences": differences,
        "passed": bool(expected) and difference_count == 0,
    }


def legal_case_ids():
    return {f"replay-c{character}-d{difficulty}-s{stage}" for character in range(4)
            for difficulty in range(5)
            for stage in (range(1, 6) if difficulty == 0 else
                          range(7, 8) if difficulty == 4 else range(1, 7))}


def verify_case_receipts(case, binary_hashes, port_name="port-final"):
    if not port_name or Path(port_name).name != port_name or port_name in (".", ".."):
        raise ValueError("port evidence directory must be a single path component")
    port_directory = case / port_name
    original = json.loads((case / "original/execution.json").read_text())
    port = json.loads((port_directory / "execution.json").read_text())
    input_hash = digest(case / "input.rpy")
    if (original.get("id") != case.name or port.get("id") != case.name or
            original.get("status") != "completed-process" or
            original.get("process_returncode") != 0 or port.get("returncode") != 0 or
            port.get("status", "completed-process") != "completed-process"):
        raise ValueError("missing successful original/port execution receipts")
    if original.get("input_sha256") != input_hash or port.get("input_sha256") != input_hash:
        raise ValueError("execution input hash does not match input.rpy")
    if port.get("reference_execution_sha256") != digest(case / "original/execution.json"):
        raise ValueError("port execution used a different original receipt")
    if port.get("reference_proxy_sha256") != original.get("proxy_sha256"):
        raise ValueError("port execution used a different original proxy")
    if original.get("exe_sha256") != "7f38496b31b8625196900a69cd1bfed243cab1f9612073e7881dc354b876fd39":
        raise ValueError("original execution did not use the verified 1.02h executable")
    binary = Path(port["command"][0]).resolve()
    if binary not in binary_hashes:
        binary_hashes[binary] = digest(binary)
    binary_hash = binary_hashes[binary]
    build = port.get("build_provenance")
    if not build or port.get("binary_sha256") != binary_hash or build.get("binary_sha256") != binary_hash:
        raise ValueError("missing or mismatched pinned binary/build provenance")
    if json.loads(binary.with_name("build-source.json").read_text()) != build:
        raise ValueError("execution build provenance differs from immutable pin")
    manifest = build["runtime_files_sha256"]
    fingerprint = hashlib.sha256(json.dumps(manifest, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    if build.get("runtime_source_sha256") != fingerprint:
        raise ValueError("invalid source manifest fingerprint")
    if port.get("outputs_sha256", {}).get("audio.csv") != digest(port_directory / "audio.csv"):
        raise ValueError("port audio trace is not bound to the successful execution receipt")
    return {"input_sha256": input_hash, "binary_sha256": binary_hash,
            "original_execution_sha256": digest(case / "original/execution.json"),
            "port_execution_sha256": digest(port_directory / "execution.json"),
            "runtime_source_sha256": fingerprint,
            "original_collection_attestation_sha256": port.get("original_collection_attestation_sha256"),
            "game_directory": (port["command"][port["command"].index("--game-dir") + 1]
                               if "--game-dir" in port["command"] else None)}


def load_original_collection(path):
    attestation = json.loads(path.read_text())
    rows = attestation["cases"]
    outputs = {"audio.csv", "replay-state.csv", "pools.jsonl", "terminal.json"}
    members = outputs | {"execution.json", "input.rpy"}
    if (attestation.get("kind") != "remote-original-audio-collection-attestation"
            or attestation.get("schema_version") != 2
            or not attestation.get("passed") or attestation.get("errors")
            or set(attestation.get("attested_outputs", [])) != outputs
            or len(rows) != 96 or {row["id"] for row in rows} != legal_case_ids()):
        raise ValueError("collection attestation is not a successful exact legal96 audio/state/pool snapshot")
    for row in rows:
        hashes = row.get("sha256", {})
        if (row.get("status") != "completed-process" or row.get("process_returncode") != 0
                or not members <= hashes.keys()
                or any(not isinstance(hashes[name], str) or len(hashes[name]) != 64
                       or any(c not in "0123456789abcdef" for c in hashes[name]) for name in members)
                or hashes["input.rpy"] != row.get("declared_input_sha256")):
            raise ValueError(f"invalid or incomplete collection attestation member: {row['id']}")
    return {row["id"]: row for row in rows}, {
        "sha256": digest(path), "schema_version": 2,
        "observed_at_utc": attestation["observed_at_utc"], "scope": attestation["scope"],
        "attested_outputs": sorted(outputs)}


def verify_original_collection(case, attested):
    for name, path in {"execution.json": case / "original/execution.json",
                       "input.rpy": case / "input.rpy",
                       "audio.csv": case / "original/audio.csv",
                       "terminal.json": case / "original/terminal.json",
                       "replay-state.csv": case / "original/replay-state.csv",
                       "pools.jsonl": case / "original/pools.jsonl"}.items():
        if attested.get("sha256", {}).get(name) != digest(path):
            raise ValueError(f"original {name} differs from attested remote collection")
    return {"audio_sha256": attested["sha256"]["audio.csv"],
            "terminal_sha256": attested["sha256"]["terminal.json"],
            "replay_state_sha256": attested["sha256"]["replay-state.csv"],
            "pools_sha256": attested["sha256"]["pools.jsonl"]}


def verify_current_asset_directory(game, manifest, hashes):
    for category, folder in (("archives", game), ("original_wav_snapshot", game / "bgm")):
        names = ({f"{stem}.DAT" for stem in ("CM", "ED", "IN", "MD", "ST", "TL")}
                 if category == "archives" else {f"th06_{number:02}.wav" for number in range(1, 18)})
        if set(manifest[category]) != names:
            raise ValueError(f"incomplete current asset attestation: {category}")
        for name, record in manifest[category].items():
            path = (folder / name).resolve()
            stat = path.stat()
            key = (path, stat.st_size, stat.st_mtime_ns)
            if key not in hashes:
                hashes[key] = digest(path)
            if stat.st_size != record["bytes"] or hashes[key] != record["sha256"]:
                raise ValueError(f"native current asset differs from attested snapshot: {name}")
    return {"compared_archives": 6, "compared_original_wav_tracks": 17,
            "scope": "Current native storage inspected after execution; not retroactive execution-time asset provenance."}


def compare_batch(directory, required_cases, port_name="port-final"):
    cases = sorted(path for path in directory.glob("replay-c*-d*-s*") if path.is_dir())
    errors, results, binary_hashes = [], [], {}
    legal, observed = legal_case_ids(), {case.name for case in cases}
    if observed - legal:
        errors.append(f"Illegal case IDs: {sorted(observed - legal)}")
    if required_cases == 96 and observed != legal:
        errors.append(f"Incomplete legal96 matrix: missing={sorted(legal - observed)}, extra={sorted(observed - legal)}")
    elif len(cases) != required_cases:
        errors.append(f"Expected {required_cases} case directories, found {len(cases)}")
    attestation_path = directory / "original-audio-collection-attestation.json"
    collection, collection_provenance = {}, None
    if required_cases == 96 or attestation_path.exists():
        try:
            collection, collection_provenance = load_original_collection(attestation_path)
        except (OSError, ValueError, KeyError, TypeError) as error:
            errors.append(f"Original audio collection: {error}")
    asset_path = directory / "reference-assets-attestation.json"
    assets, asset_provenance, asset_hashes = None, None, {}
    if required_cases == 96 or asset_path.exists():
        try:
            snapshot = json.loads(asset_path.read_text())
            if (snapshot.get("kind") != "current-reference-native-asset-attestation"
                    or not snapshot.get("passed") or snapshot.get("errors")):
                raise ValueError("current reference/native asset snapshot did not pass")
            assets = snapshot["native"]
            asset_provenance = {"sha256": digest(asset_path),
                                "observed_at_utc": snapshot["observed_at_utc"],
                                "scope": snapshot["scope"],
                                "current_original_scenarios_with_wav_directory":
                                    snapshot["scenarios_with_current_wav_directory"]}
        except (OSError, ValueError, KeyError, TypeError) as error:
            errors.append(f"Current asset snapshot: {error}")
    for case in cases:
        try:
            provenance = verify_case_receipts(case, binary_hashes, port_name)
            if collection:
                provenance["original_collection"] = verify_original_collection(case, collection[case.name])
                if (required_cases == 96 and provenance["original_collection_attestation_sha256"]
                        != collection_provenance["sha256"]):
                    raise ValueError("port execution is not bound to this original collection attestation")
            if assets:
                if not provenance["game_directory"]:
                    raise ValueError("execution receipt lacks its native asset storage directory")
                provenance["current_asset_snapshot"] = verify_current_asset_directory(
                    Path(provenance["game_directory"]), assets, asset_hashes)
            terminal = json.loads((case / "original/terminal.json").read_text())
            stage, last = int(terminal["stage"]), int(terminal["frame"])
            if last < 1 or stage != int(case.name.rsplit("-s", 1)[1]):
                raise ValueError("terminal stage/frame does not match this legal scenario")
            original = case / "original/audio.csv"
            port = case / port_name / "audio.csv"
            gameplay = compare_events(original, port, ["SFX_REQUEST", "BGM_REQUEST", "BGM_FADE"], 1, last, [stage])
            starts = []
            for path in (original, port):
                with path.open() as source:
                    starts.append(next((row["value"].replace(".mid", "").replace(".wav", "")
                        for row in csv.DictReader(source) if int(row["stage"]) == stage
                        and row["frame"] == "-1" and row["event"] == "BGM_REQUEST"), None))
            result = {"case": case.name, "provenance": provenance, "gameplay": gameplay,
                      "stage_entry_request": {"expected": starts[0], "actual": starts[1],
                                              "passed": starts[0] is not None and starts[0] == starts[1]},
                      "passed": gameplay["passed"] and starts[0] is not None and starts[0] == starts[1]}
            results.append(result)
        except (OSError, ValueError, KeyError, TypeError, IndexError) as error:
            errors.append(f"{case.name}: {error}")
    if len(set(binary_hashes.values())) > 1:
        errors.append("The matrix mixes port binaries; one frozen binary is required")
    return {"kind": "real-executable-predefined-replay-audio", "required_cases": required_cases,
            "port_evidence_directory": port_name,
            "compared_cases": len(results), "passed_cases": sum(case["passed"] for case in results),
            "original_collection_provenance": collection_provenance,
            "current_asset_snapshot_provenance": asset_provenance,
            "scope": "Ordered sound/music requests and fades during replay frames1..original terminal, plus the first stage-entry music request. Scene transitions after the terminal and physical output PCM are separate probes.",
            "errors": errors, "cases": results,
            "passed": len(results) == required_cases and not errors and all(case["passed"] for case in results)}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    sub = parser.add_subparsers(dest="kind", required=True)
    midi = sub.add_parser("midi")
    midi.add_argument("reference", type=Path)
    midi.add_argument("port", type=Path)
    batch = sub.add_parser("batch")
    batch.add_argument("directory", type=Path)
    batch.add_argument("--require-cases", type=int, default=96)
    batch.add_argument("--port-name", default="port-final",
                       help="Evidence directory within each case; retain one pinned binary per batch")
    events = sub.add_parser("events")
    events.add_argument("reference", type=Path)
    events.add_argument("port", type=Path)
    events.add_argument("--types", default="SFX_REQUEST,SFX_PLAY,BGM_REQUEST,BGM_FADE")
    events.add_argument("--first-frame", type=int, default=0)
    events.add_argument("--last-frame", type=int, default=3720)
    events.add_argument("--stages", help="Explicit stage subset, e.g. 4; omit to compare all stages")
    events.add_argument("--clock", choices=("replay", "present"), default="replay")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    if args.kind == "midi":
        result = compare_midi(args.reference, args.port)
    elif args.kind == "batch":
        result = compare_batch(args.directory, args.require_cases, args.port_name)
    else:
        stages = list(map(int, args.stages.split(","))) if args.stages else None
        result = compare_events(args.reference, args.port, args.types.split(","), args.first_frame, args.last_frame, stages, args.clock)
    body = json.dumps(result, indent=2) + "\n"
    if args.output:
        args.output.write_text(body)
    print(body)
    raise SystemExit(0 if result["passed"] else 1)
