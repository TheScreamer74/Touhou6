"""Analyze audio-tap dumps: per-phase continuity, silence gaps and click candidates.

Block row: [frame, rmsL, peakL, maxDeltaL, rmsR, peakR, maxDeltaR, inputChannels].
"""
import json
import statistics
import sys

SILENT = 1e-6        # peak below this = digital silence
CLICK_ABS = 0.25     # max |x[n]-x[n-1]| that may be a click
CLICK_RATIO = 6.0    # ... and this many times the local median delta


def load(path):
    d = json.load(open(path))
    return d["rate"], d["stats"], d["events"]


def analyze(path):
    rate, rows, events = load(path)
    if not rows:
        return {"file": path, "blocks": 0}
    frames = [r[0] for r in rows]
    gaps = [(frames[i - 1], frames[i] - frames[i - 1]) for i in range(1, len(frames)) if frames[i] - frames[i - 1] != 128]
    peak = [max(r[2], r[5]) for r in rows]
    delta = [max(r[3], r[6]) for r in rows]
    # Silence runs inside otherwise sounding audio (short gaps = dropouts; long = intentional).
    runs, start = [], None
    for i, p in enumerate(peak):
        if p < SILENT and start is None:
            start = i
        elif p >= SILENT and start is not None:
            runs.append((start, i - start)); start = None
    if start is not None:
        runs.append((start, len(peak) - start))
    inner = [(frames[s] / rate, n * 128 / rate) for s, n in runs if s > 0 and s + n < len(peak)]
    short_dropouts = [(round(t, 3), round(dur * 1000, 1)) for t, dur in inner if dur < 0.25]
    clicks = []
    for i in range(len(delta)):
        lo, hi = max(0, i - 40), min(len(delta), i + 41)
        neigh = sorted(delta[lo:hi]); med = neigh[len(neigh) // 2]
        if delta[i] > CLICK_ABS and delta[i] > CLICK_RATIO * max(med, 1e-4):
            clicks.append((round(frames[i] / rate, 3), round(delta[i], 3), round(med, 4)))
    ev = [(e["label"], round(e["frame"] / rate, 3) if e["frame"] is not None else None, round(e["t"] / 1000, 3), e["state"]) for e in events]
    # Audio clock vs wall clock between consecutive marks (stall detection).
    clock = []
    for a, b in zip(events, events[1:]):
        if a["frame"] is None or b["frame"] is None:
            continue
        wall = (b["t"] - a["t"]) / 1000; audio = (b["frame"] - a["frame"]) / rate
        clock.append((a["label"], round(wall, 3), round(audio, 3), round(audio / wall, 4) if wall > 0 else None))
    return {"file": path, "rate": rate, "blocks": len(rows), "seconds": round(len(rows) * 128 / rate, 2),
            "first_s": round(frames[0] / rate, 3), "frame_gaps": gaps[:10], "frame_gap_count": len(gaps),
            "silence_runs_inside": len(inner), "short_dropouts_lt250ms": short_dropouts[:20],
            "click_candidates": clicks[:20], "click_count": len(clicks),
            "median_peak": round(statistics.median(peak), 4), "events": ev, "audio_vs_wall": clock}


if __name__ == "__main__":
    for p in sys.argv[1:]:
        print(json.dumps(analyze(p), indent=1))
