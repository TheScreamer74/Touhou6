#!/usr/bin/env python3
"""Pin a freshly built port executable together with its frozen source state.

Call only after the shared source has been frozen and the release build has
completed. Earlier diagnostic pins without a source receipt cannot be upgraded
retroactively: a new build is required to establish their provenance.
"""
import argparse
import hashlib
import json
from pathlib import Path
import shutil


def sha(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


def source_manifest(root):
    paths = [root / 'Cargo.lock', root / 'Cargo.toml']
    for crate in ('game', 'engine', 'formats'):
        paths.extend(p for p in (root / 'crates' / crate).rglob('*')
                     if p.is_file() and 'target' not in p.parts and p.name != '.DS_Store')
    files = {str(path.relative_to(root)): sha(path) for path in sorted(paths)}
    fingerprint = hashlib.sha256(json.dumps(files, sort_keys=True, separators=(',', ':')).encode()).hexdigest()
    return {'runtime_source_sha256': fingerprint, 'runtime_files_sha256': files}


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument('--binary', type=Path, default=Path('target/release/th06'))
    parser.add_argument('--output', type=Path, default=Path('/tmp/th06-scenarios/bin'))
    args = parser.parse_args()
    root = Path(__file__).resolve().parent.parent
    binary_hash = sha(args.binary)
    directory = args.output / binary_hash
    directory.mkdir(parents=True, exist_ok=True)
    destination = directory / 'th06'
    receipt = directory / 'build-source.json'
    if destination.exists() or receipt.exists():
        parser.error('immutable pin already exists; reuse its receipt or rebuild after changes')
    data = source_manifest(root)
    data['binary_sha256'] = binary_hash
    shutil.copy2(args.binary, destination)
    if sha(destination) != binary_hash:
        raise ValueError('executable changed while pinning')
    receipt.write_text(json.dumps(data, indent=2) + '\n')
    print(destination)


if __name__ == '__main__':
    main()
