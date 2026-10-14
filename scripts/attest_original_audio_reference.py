#!/usr/bin/env python3
"""Attest current remote audio/state/pool evidence without rewriting receipts.

This is collection provenance, not a claim that an old execution receipt
contained output hashes. The local mirror must match the successful remote
receipt, replay input, audio, replay states, pools and terminal for every case.
"""
import argparse
from datetime import datetime, timezone
import json
from pathlib import Path
import shlex
import shutil
import subprocess

from verify_audio_reference import digest, legal_case_ids


REMOTE = r'''
import hashlib,json,pathlib,sys
base=pathlib.Path(sys.argv[1]);names=json.loads(sys.argv[2]);result=[]
def digest(path):
 h=hashlib.sha256()
 with path.open('rb') as source:
  for block in iter(lambda:source.read(1024*1024),b''):h.update(block)
 return h.hexdigest()
for name in names:
 case=base/name
 try:
  receipt=json.loads((case/'execution.json').read_text())
  result.append({'id':name,'status':receipt.get('status'),
   'process_returncode':receipt.get('process_returncode'),
   'declared_input_sha256':receipt.get('input_sha256'),
   'sha256':{key:digest(case/path) for key,path in {
    'execution.json':'execution.json','input.rpy':'input.rpy',
    'audio.csv':'game/audio.csv','terminal.json':'game/terminal.json',
    'replay-state.csv':'game/replay-state.csv','pools.jsonl':'game/pools.jsonl'}.items()}})
 except (OSError,ValueError) as error:result.append({'id':name,'error':str(error)})
print(json.dumps(result))
'''


def attest(directory, host, remote):
    names = sorted(legal_case_ids())
    command = shlex.join(["python3", "-c", REMOTE, remote, json.dumps(names)])
    run = subprocess.run(["ssh", "-o", "BatchMode=yes", "-o", "ConnectTimeout=15",
                          host, command], text=True, capture_output=True, check=True)
    rows = json.loads(run.stdout)
    errors = []
    if len(rows) != 96 or {row["id"] for row in rows} != set(names):
        errors.append("remote attestation is not the exact legal96 case set")
    for row in rows:
        case = directory / row["id"]
        try:
            if row.get("error"):
                raise ValueError(row["error"])
            if row.get("status") != "completed-process" or row.get("process_returncode") != 0:
                raise ValueError("original execution was not successful")
            for key, path in {"execution.json": case / "original/execution.json",
                              "input.rpy": case / "input.rpy",
                              "audio.csv": case / "original/audio.csv",
                              "terminal.json": case / "original/terminal.json",
                              "replay-state.csv": case / "original/replay-state.csv",
                              "pools.jsonl": case / "original/pools.jsonl"}.items():
                if digest(path) != row["sha256"][key]:
                    raise ValueError(f"local {key} differs from remote snapshot")
            if row["sha256"]["input.rpy"] != row["declared_input_sha256"]:
                raise ValueError("remote input differs from its execution receipt")
        except (OSError, ValueError, KeyError) as error:
            errors.append(f"{row['id']}: {error}")
    return {"kind": "remote-original-audio-collection-attestation",
            "schema_version": 2,
            "attested_outputs": ["audio.csv", "replay-state.csv", "pools.jsonl", "terminal.json"],
            "observed_at_utc": datetime.now(timezone.utc).isoformat(),
            "host": host, "remote": remote, "required_cases": 96,
            "scope": "Current remote audio/state/pool/terminal snapshot matches the local mirror and successful original execution/input receipts. Original execution receipts are unchanged; this is later collection provenance, not output hashes recorded at original execution time.",
            "cases": rows, "errors": errors, "passed": not errors}


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("directory", type=Path)
    parser.add_argument("--host", default="ubuntu@blackoutburst.com")
    parser.add_argument("--remote", default="/home/ubuntu/touhou6-web/reference/scenarios")
    parser.add_argument("--output", type=Path)
    args = parser.parse_args()
    result = attest(args.directory, args.host, args.remote)
    output = args.output or args.directory / "original-audio-collection-attestation.json"
    if output.exists():
        previous = json.loads(output.read_text())
        stamp = previous.get("observed_at_utc", "undated").replace(":", "-")
        archived = output.with_name(output.stem + "-" + stamp + output.suffix)
        if archived.exists() and digest(archived) != digest(output):
            raise ValueError(f"Refusing to replace different preserved attestation: {archived}")
        if not archived.exists():
            shutil.copy2(output, archived)
    output.write_text(json.dumps(result, indent=2) + "\n")
    print(json.dumps({"passed": result["passed"], "cases": len(result["cases"]),
                      "errors": result["errors"], "output": str(output)}, indent=2))
    raise SystemExit(0 if result["passed"] else 1)
