#!/usr/bin/env python3
"""Check locale coverage against a user's extracted Touhou 6 archives.

Example:
  python3 scripts/audit_locales.py --st /private/ST --ed /private/ED \
      --music /private/MD/musiccmt.txt --output /tmp/locale-audit.json

Only the audit receipt is public. The input data remains outside the repository.
Keys preserve the exact Windows Shift-JIS strings used by the game, including
spaces. END coverage includes both whole lines and the original 32-byte VM
chunks. Proper names and intentional whitespace can remain unchanged.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import re
import struct
from collections import defaultdict
from pathlib import Path

ROOT = Path(__file__).resolve().parents[1]
LANGUAGES = ("fr", "en", "de", "nl")


def digest(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def decode(data: bytes) -> str:
    return data.decode("cp932", errors="strict")


def cstring(data: bytes) -> str:
    return decode(data.split(b"\0", 1)[0])


def msg_strings(data: bytes) -> tuple[set[str], int]:
    """Walk every entry, including post-boss and Extra dialogue branches."""
    count = struct.unpack_from("<I", data)[0]
    if count > 1024 or len(data) < 4 + 4 * count:
        raise ValueError("invalid MSG offset table")
    offsets = struct.unpack_from("<" + "I" * count, data, 4)
    strings: set[str] = set()
    for offset in offsets:
        for _ in range(10_000):
            if offset + 4 > len(data):
                raise ValueError("truncated MSG instruction")
            _, opcode, size = struct.unpack_from("<HBB", data, offset)
            end = offset + 4 + size
            if end > len(data):
                raise ValueError("truncated MSG arguments")
            if opcode in (3, 8) and size > 4:
                text = cstring(data[offset + 8:end])
                if text.strip():
                    strings.add(text)
            if opcode in (0, 10):
                break
            offset = end
        else:
            raise ValueError("MSG entry exceeds instruction budget")
    return strings, count


def spell_strings(data: bytes) -> set[str]:
    count = struct.unpack_from("<H", data)[0]
    if count > 4096 or len(data) < 16 + 4 * count:
        raise ValueError("invalid ECL subroutine table")
    offsets = struct.unpack_from("<" + "I" * count, data, 16)
    strings: set[str] = set()
    for offset in offsets:
        for _ in range(100_000):
            if offset + 12 > len(data):
                break
            opcode, size = struct.unpack_from("<hh", data, offset + 4)
            if size <= 0:
                break
            end = offset + size
            if end > len(data):
                raise ValueError("truncated ECL instruction")
            if opcode == 93:
                text = cstring(data[offset + 16:end])
                if text.strip():
                    strings.add(text)
            offset = end
        else:
            raise ValueError("ECL subroutine exceeds instruction budget")
    return strings


def ending_strings(data: bytes, filename: str) -> tuple[set[str], set[str], set[str]]:
    lines, chunks, contexts = set(), set(), set()
    line_offset = 0
    for raw_line in data.splitlines(keepends=True):
        line = raw_line.rstrip(b"\r\n")
        if not line or line.startswith(b"@"):
            line_offset += len(raw_line)
            continue
        source_offset = line_offset
        for raw in line.split(b"\0"):
            if not raw:
                source_offset += 1
                continue
            text = decode(raw)
            if text.strip():
                lines.add(text)
            for offset in range(0, len(raw), 32):
                part = decode(raw[offset:offset + 32])
                if part.strip():
                    chunks.add(part)
                contexts.add(f"end:{filename.lower()}:{source_offset + min(offset + 32, len(raw))}")
            source_offset += len(raw) + 1
        line_offset += len(raw_line)
    return lines, chunks, contexts


def music_strings(data: bytes) -> tuple[set[str], set[str], int]:
    titles, comments = set(), set()
    blocks = re.split(r"^@bgm/", decode(data), flags=re.MULTILINE)[1:]
    for block in blocks:
        lines = block.splitlines()
        if len(lines) < 2:
            raise ValueError("truncated Music Room track")
        titles.add(lines[1])
        comments.update(line for line in lines[2:] if line.strip())
    return titles, comments, len(blocks)


def ui_keys(directory: Path) -> set[str]:
    # Inspect new namespaces too: a misspelled HUD namespace must not silently
    # escape the catalog audit and fall back to English during gameplay.
    pattern = re.compile(r'"([a-z][a-z0-9_]*\.[a-z_]+)"')
    extensions = {"csv", "jpg", "png", "anm", "json", "jsonl", "txt", "rs",
                  "dat", "end", "wav", "ogg", "pos", "bak", "tmp", "sf2",
                  "ttc", "ttf", "bin", "exe", "dll", "log", "bmp", "rpy"}
    return {
        key
        for path in directory.rglob("*.rs")
        for key in pattern.findall(path.read_text())
        if key.rsplit(".", 1)[-1] not in extensions
    }


def audit(st: Path, ed: Path, music: Path, catalog_dir: Path) -> dict:
    inventory: dict[str, set[str]] = defaultdict(set)
    source_hashes = {}
    stages = []
    entry_count = 0

    def read(path: Path) -> bytes:
        data = path.read_bytes()
        source_hashes[path.name] = digest(data)
        return data

    for stage in range(1, 8):
        strings, entries = msg_strings(read(st / f"msg{stage}.dat"))
        inventory["dialogue"].update(strings)
        entry_count += entries
        inventory["spell"].update(spell_strings(read(st / f"ecldata{stage}.ecl")))
        std = read(st / f"stage{stage}.std")
        if len(std) < 656:
            raise ValueError("truncated STD text header")
        for index, offset in enumerate((16, 144, 272, 400, 528)):
            text = cstring(std[offset:offset + 128])
            if text.strip():
                inventory["stage" if index == 0 else "song"].add(text)
        stages.append({"stage": stage, "msg_entries": entries, "dialogue_keys": len(strings)})

    ending_files = sorted(ed.glob("*.end"))
    if len(ending_files) != 7:
        raise ValueError("expected six endings and staff00.end")
    for path in ending_files:
        lines, chunks, contexts = ending_strings(read(path), path.name)
        inventory["ending_line"].update(lines)
        inventory["ending_chunk"].update(chunks)
        inventory["ending_context"].update(contexts)
    titles, comments, track_count = music_strings(read(music))
    if track_count != 17:
        raise ValueError("expected 17 Music Room tracks")
    inventory["music_title"].update(titles)
    inventory["music_comment"].update(comments)
    inventory["song_prefixed"].update("♪" + source for source in inventory["song"] | titles)
    inventory["song_prefixed"].update("♪ " + source for source in inventory["song"] | titles)
    inventory["ui"].update(ui_keys(ROOT / "crates/game/src"))
    required = set().union(*inventory.values())
    languages = {}
    for language in LANGUAGES:
        path = catalog_dir / f"{language}.json"
        data = path.read_bytes()
        pack = json.loads(data)
        if not isinstance(pack, dict) or not all(isinstance(key, str) and isinstance(value, str) for key, value in pack.items()):
            raise ValueError(f"invalid catalog: {language}")
        missing = sorted(required - pack.keys())
        invalid = sorted(key for key, value in pack.items() if not value or "\ufffd" in key or "\ufffd" in value)
        languages[language] = {
            "pass": not missing and not invalid,
            "keys": len(pack), "bytes": len(data), "sha256": digest(data),
            "missing": missing, "invalid": invalid,
            "covered_by_category": {key: len(values & pack.keys()) for key, values in inventory.items()},
        }
    key_sets = [set(json.loads((catalog_dir / f"{language}.json").read_text())) for language in LANGUAGES]
    identical_keys = all(keys == key_sets[0] for keys in key_sets[1:])
    return {
        "schema": 1,
        "pass": identical_keys and all(language["pass"] for language in languages.values()),
        "scope": "Exact source-key presence and UTF-8 validity; this is coverage, not human linguistic review or rendered layout approval.",
        "source": "User-provided original v1.02h ST.DAT, ED.DAT and MD.DAT, extracted outside the repository.",
        "translation_base": "User-provided English patch where available, original Japanese for spell names; FR/DE/NL authored for this port.",
        "msg_entries": entry_count, "stages": stages,
        "ending_files": [path.name for path in ending_files], "music_tracks": track_count,
        "required_unique_keys": len(required),
        "required_key_sha256": digest("\n".join(sorted(required)).encode()),
        "required_by_category": {key: len(value) for key, value in inventory.items()},
        "identical_language_keys": identical_keys,
        "languages": languages, "input_member_sha256": source_hashes,
    }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--st", type=Path, required=True)
    parser.add_argument("--ed", type=Path, required=True)
    parser.add_argument("--music", type=Path, required=True)
    parser.add_argument("--catalogs", type=Path, default=ROOT / "crates/game/assets/locales")
    parser.add_argument("--output", type=Path, required=True)
    args = parser.parse_args()
    receipt = audit(args.st, args.ed, args.music, args.catalogs)
    args.output.parent.mkdir(parents=True, exist_ok=True)
    args.output.write_text(json.dumps(receipt, ensure_ascii=False, indent=2) + "\n")
    print(json.dumps({"pass": receipt["pass"], "required_unique_keys": receipt["required_unique_keys"], "languages": {key: {"pass": value["pass"], "keys": value["keys"], "missing": len(value["missing"])} for key, value in receipt["languages"].items()}}, indent=2))
    raise SystemExit(0 if receipt["pass"] else 1)


if __name__ == "__main__":
    main()
