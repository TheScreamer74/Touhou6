#!/usr/bin/env python3
"""Prepare a local Web Audio loop check using a separately owned TH06 install.

Extract MD.DAT with the formats crate's `extract` example first. Then run:
  python3 scripts/verify_music_loops.py --game-dir /path/to/kouma \
      --pos-dir /path/to/extracted-MD --ogg-dir /path/to/hosted/assets \
      --output-dir /tmp/th06-audio-check --serve

Open the printed localhost URL and press the check button. The browser renders
both sides of each original loop point at 44.1 and 48 kHz and reports the actual
PCM source alignment. This is a data/render check, not a listening test.
All game files and the generated metadata stay outside the repository.
"""

import argparse
import http.server
import json
from pathlib import Path
import shutil
import struct
import wave


def vorbis_metadata(path):
    """Read source sample rate and final PCM frame count from Ogg page headers."""
    data = path.read_bytes()
    offset = 0
    sample_rate = None
    final_frame = None
    serial = None
    while offset < len(data):
        header = data[offset : offset + 27]
        if len(header) != 27 or header[:5] != b"OggS\0":
            raise ValueError(f"Invalid Ogg page: {path}")
        page_serial = struct.unpack_from("<I", header, 14)[0]
        if serial is not None and serial != page_serial:
            raise ValueError(f"Multiplexed Ogg is unsupported: {path}")
        serial = page_serial
        count = header[26]
        lacing = data[offset + 27 : offset + 27 + count]
        if len(lacing) != count:
            raise ValueError(f"Truncated Ogg segment table: {path}")
        body = offset + 27 + count
        end = body + sum(lacing)
        if end > len(data):
            raise ValueError(f"Truncated Ogg body: {path}")
        if sample_rate is None:
            packet = data[body:end]
            if len(packet) < 30 or packet[:7] != b"\x01vorbis":
                raise ValueError(f"Not a Vorbis identification packet: {path}")
            sample_rate = struct.unpack_from("<I", packet, 12)[0]
        granule = struct.unpack_from("<Q", header, 6)[0]
        if header[5] & 4:
            final_frame = granule
        offset = end
    if not sample_rate or final_frame is None:
        raise ValueError(f"Missing Vorbis rate/end position: {path}")
    return sample_rate, final_frame


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--game-dir", type=Path, required=True)
    parser.add_argument("--pos-dir", type=Path, required=True)
    parser.add_argument("--ogg-dir", type=Path, required=True)
    parser.add_argument("--output-dir", type=Path, required=True)
    parser.add_argument("--serve", action="store_true")
    parser.add_argument("--port", type=int, default=8766)
    args = parser.parse_args()
    output = args.output_dir.resolve()
    repo = Path(__file__).resolve().parents[1]
    if output == repo or repo in output.parents:
        parser.error("Choose an output directory outside the code repository.")
    tracks = []
    for number in range(1, 18):
        name = f"th06_{number:02}"
        wav = args.game_dir / "bgm" / f"{name}.wav"
        ogg = args.ogg_dir / f"{name}.ogg"
        with wave.open(str(wav)) as audio:
            rate, frames = audio.getframerate(), audio.getnframes()
        source_rate, source_frames = vorbis_metadata(ogg)
        if (source_rate, source_frames) != (rate, frames):
            raise ValueError(f"Vorbis rate/frame count differs from original WAV: {name}")
        pos = (args.pos_dir / f"{name}.pos").read_bytes()
        start, end = struct.unpack_from("<II", pos)
        if not 0 <= start < end <= frames:
            raise ValueError(f"Loop points outside original WAV: {name}")
        tracks.append(dict(name=name, rate=rate, length=frames, start=start, end=end))
    output.mkdir(parents=True, exist_ok=True)
    (output / "ogg").mkdir(exist_ok=True)
    for track in tracks:
        name = f"{track['name']}.ogg"
        destination = output / "ogg" / name
        if not destination.exists() or not destination.samefile(args.ogg_dir / name):
            shutil.copy2(args.ogg_dir / name, destination)
    shutil.copyfile(Path(__file__).with_name("music_loop_check.html"), output / "index.html")
    (output / "tracks.json").write_text(json.dumps(tracks, indent=2), encoding="utf-8")
    print(f"PASS: all 17 Vorbis rates/frame counts and original loop bounds match.\nPrepared: {output}")
    if args.serve:
        handler = lambda *a, **kw: http.server.SimpleHTTPRequestHandler(*a, directory=str(output), **kw)
        with http.server.ThreadingHTTPServer(("127.0.0.1", args.port), handler) as server:
            print(f"Open http://127.0.0.1:{args.port}/ and press the check button.", flush=True)
            server.serve_forever()


if __name__ == "__main__":
    main()
