#!/usr/bin/env python3
"""Attest current reference/native asset snapshots with streaming hashes.

These are later collection observations, not asset hashes from historical
execution receipts. Original WAV, hosted Ogg, archive POS/MIDI and the optional
free SoundFont have separate roles and must not be reported as interchangeable.
"""
import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import shlex
import struct
import subprocess
import tempfile

from verify_audio_reference import digest, legal_case_ids

ARCHIVES = ("CM.DAT", "ED.DAT", "IN.DAT", "MD.DAT", "ST.DAT", "TL.DAT")
WAVS = tuple(f"th06_{number:02}.wav" for number in range(1, 18))
REMOTE = r'''
from pathlib import Path
import hashlib,json,sys
dat,wav,scenarios=map(Path,sys.argv[1:4]);names=json.loads(sys.argv[4]);archives=json.loads(sys.argv[5]);wavs=json.loads(sys.argv[6]);cache={}
def record(path):
 stat=path.stat();key=(stat.st_dev,stat.st_ino,stat.st_size,stat.st_mtime_ns)
 if key not in cache:
  h=hashlib.sha256()
  with path.open('rb') as source:
   for block in iter(lambda:source.read(1024*1024),b''):h.update(block)
  cache[key]=h.hexdigest()
 return {'sha256':cache[key],'bytes':stat.st_size,'resolved_path':str(path.resolve())}
result={'archives':{name:record(dat/name) for name in archives},'original_wav_snapshot':{name:record(wav/name) for name in wavs},'wav_names':sorted(p.name for p in wav.glob('*.wav')),'cases':[]}
for name in names:
 game=scenarios/name/'game';bgm=game/'bgm'
 result['cases'].append({'id':name,'archives':{asset:record(game/asset) for asset in archives},'bgm_directory_present':bgm.is_dir(),'bgm_resolved_path':str(bgm.resolve()) if bgm.is_dir() else None,'present_wav_names':sorted(p.name for p in bgm.glob('*.wav')) if bgm.is_dir() else []})
print(json.dumps(result))
'''


def local_record(path):
    return {"sha256": digest(path), "bytes": path.stat().st_size,
            "resolved_path": str(path.resolve())}


def attest(args):
    command = shlex.join(["python3", "-c", REMOTE, args.remote_dat, args.remote_wav,
                          args.remote_scenarios, json.dumps(sorted(legal_case_ids())),
                          json.dumps(ARCHIVES), json.dumps(WAVS)])
    run = subprocess.run(["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=15",
                          args.host, command], text=True, capture_output=True, check=True)
    remote = json.loads(run.stdout)
    native = {"archives": {name: local_record(args.game / name) for name in ARCHIVES},
              "original_wav_snapshot": {name: local_record(args.game / "bgm" / name) for name in WAVS}}
    errors = []
    for category in ("archives", "original_wav_snapshot"):
        for name, record in native[category].items():
            if any(record[field] != remote[category][name][field] for field in ("sha256", "bytes")):
                errors.append(f"reference/native {category} differs: {name}")
    if remote["wav_names"] != list(WAVS):
        errors.append("remote WAV snapshot is not the exact17 original tracks")
    if len(remote["cases"]) != 96 or {row["id"] for row in remote["cases"]} != legal_case_ids():
        errors.append("remote archive attestation is not the exact legal96 case set")
    for row in remote["cases"]:
        for name, record in row["archives"].items():
            if record["sha256"] != native["archives"][name]["sha256"]:
                errors.append(f"scenario archive differs: {row['id']}/{name}")
    derived = None
    if args.extractor:
        with tempfile.TemporaryDirectory(prefix="th06-md-attestation-") as temporary:
            output = Path(temporary)
            subprocess.run([str(args.extractor.resolve()), str((args.game / "MD.DAT").resolve()),
                            str(output)], check=True, stdout=subprocess.DEVNULL)
            members = {path.name: local_record(path) for path in sorted(output.iterdir())}
            for record in members.values():
                record.pop("resolved_path")
            points = {}
            for number in range(1, 18):
                path = output / f"th06_{number:02}.pos"
                if path.stat().st_size != 8:
                    errors.append(f"invalid archive POS size: {path.name}")
                else:
                    points[path.name] = list(struct.unpack("<II", path.read_bytes()))
            derived = {"archive_sha256": native["archives"]["MD.DAT"]["sha256"],
                       "extractor_sha256": digest(args.extractor), "members": members,
                       "pos_points_source_sample_frames": points,
                       "scope": "Fresh extraction from the attested native MD.DAT, byte-identical to the reference archive. POS and MIDI are archive members, not external Ogg files or a synthesizer bank."}
    bank = local_record(args.bank) if args.bank else None
    return {"kind": "current-reference-native-asset-attestation",
            "observed_at_utc": datetime.now(timezone.utc).isoformat(), "host": args.host,
            "scope": "Current remote/native snapshots only. Historical execution receipts were not rewritten and did not contain these asset hashes. All96 current original scenario DAT copies are compared. WAV equality concerns the separate original WAV snapshot; per-case WAV presence is explicitly recorded. Hosted Ogg and synthesized MIDI timbre are outside this byte-equality result.",
            "remote": remote, "native": native, "archive_pos_midi": derived,
            "optional_free_soundfont": {"asset": bank, "scope": "Optional port synthesizer bank. It is separate from original MIDI events, original WAV recordings and POS points; no Windows MIDI timbre identity is claimed."},
            "scenarios_with_current_wav_directory": sum(row["bgm_directory_present"] for row in remote["cases"]),
            "errors": errors, "passed": not errors}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--game", type=Path, required=True)
    parser.add_argument("--host", default="ubuntu@blackoutburst.com")
    parser.add_argument("--remote-dat", default="/home/ubuntu/touhou6-web/reference/game")
    parser.add_argument("--remote-wav", default="/home/ubuntu/th06-res/res/bgm")
    parser.add_argument("--remote-scenarios", default="/home/ubuntu/touhou6-web/reference/scenarios")
    parser.add_argument("--extractor", type=Path)
    parser.add_argument("--bank", type=Path)
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    result = attest(args)
    args.output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"passed": result["passed"], "errors": result["errors"],
                      "scenario_count": len(result["remote"]["cases"]),
                      "wav_directory_count": result["scenarios_with_current_wav_directory"],
                      "output": str(args.output)}, indent=2))
    raise SystemExit(0 if result["passed"] else 1)
