#!/usr/bin/env python3
"""Check REA's complete WASM output against independent WABT output.

Uses a generated, asset-free module beyond the old 8/32 MiB limits and,
optionally, a selected production module. Full intermediate evidence is kept
only in a temporary directory; the receipt contains sizes, hashes and IDs.
"""

import argparse
import hashlib
import json
import os
from pathlib import Path
import subprocess
import tempfile


def sha256(data):
    return hashlib.sha256(data).hexdigest()


def leb128(value):
    result = bytearray()
    while value >= 128:
        result.append((value & 127) | 128)
        value >>= 7
    result.append(value)
    return result


def verify(rea, wabt, module, glue, work):
    original = module.read_bytes()
    wat_path = work / "independent.wat"
    subprocess.run(
        [str(wabt / "wasm2wat"), "--enable-all", str(module), "-o", str(wat_path)],
        check=True, timeout=60,
    )
    evidence_path = work / "evidence.json"
    command = [str(rea), "inspect-wasm-artifact", str(module), "--format", "json"]
    for path in glue:
        command.extend(["--glue_paths", str(path)])
    with evidence_path.open("wb") as destination:
        subprocess.run(command, stdout=destination, check=True, timeout=180)
    evidence = json.loads(evidence_path.read_text())
    report = evidence["normalized_result"]
    assert report["validation"] == "valid", report.get("validation")
    assert report["artifact"]["sha256"] == sha256(original)
    assert report["artifact"]["bytes"] == len(original)
    wat = wat_path.read_bytes()
    assert report["wat"]["text"].encode() == wat, "WAT was changed or truncated"
    assert report["wat"]["sha256"] == sha256(wat)
    assert report["sections"][-1]["end"] == len(original), "Incomplete sections"
    assert module.read_bytes() == original, "Selected artifact was modified"
    assert report["runtime_execution"] == "not-performed"
    assert report["network_fetch"] == "not-performed"
    return {
        "artifact_bytes": len(original),
        "artifact_sha256": sha256(original),
        "wat_bytes": len(wat),
        "wat_sha256": sha256(wat),
        "evidence_id": evidence["evidence_id"],
        "sections": len(report["sections"]),
        "imports": len(report["imports"]),
        "exports": len(report["exports"]),
        "output_budget_bytes": report["tool_profile"]["output_budget_bytes"],
        "wat_matches_independent_wabt": True,
        "artifact_unchanged": True,
    }


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--rea", type=Path, required=True, help="REA CLI executable")
    parser.add_argument("--wabt-bin", type=Path, required=True)
    parser.add_argument("--wasm", type=Path)
    parser.add_argument("--glue", type=Path, action="append", default=[])
    parser.add_argument("--report", type=Path, required=True)
    args = parser.parse_args()
    os.environ["REA_WABT_BIN_DIRECTORY"] = str(args.wabt_bin.resolve())
    results = {}
    with tempfile.TemporaryDirectory(prefix="th06-rea-capacity-") as directory:
        work = Path(directory)
        # Uninterpreted custom payload is valid WASM and expands 3:1 as WAT.
        payload = b"\x08capacity" + b"\xff" * (12 * 1024 * 1024)
        module = work / "capacity.wasm"
        module.write_bytes(b"\0asm\x01\0\0\0\0" + leb128(len(payload)) + payload)
        results["generated_capacity"] = verify(
            args.rea.resolve(), args.wabt_bin.resolve(), module, [], work,
        )
        assert results["generated_capacity"]["artifact_bytes"] > 8 * 1024 * 1024
        assert results["generated_capacity"]["wat_bytes"] > 32 * 1024 * 1024
        if args.wasm:
            results["production"] = verify(
                args.rea.resolve(), args.wabt_bin.resolve(), args.wasm.resolve(),
                [path.resolve() for path in args.glue], work,
            )
    args.report.write_text(json.dumps({"status": "passed", **results}, indent=2) + "\n")
    print(args.report.read_text(), end="")


if __name__ == "__main__":
    main()
