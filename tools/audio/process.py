#!/usr/bin/env python3
"""Measure, pick and process generated takes (docs/sound-system-prd.md 4.12).

For each sample named (or every sample in `review`), reads its takes from
`tools/audio/_takes/<sample>/*.mp3`, measures each one, pre-picks the best
by measurement - the session that runs this cannot listen; the owner reviews
the picks on the audition page and swaps or rejects - and processes the
picks into `static/audio/<bus>/<sample>_<n>.ogg`:

- one-shots: the silence before the onset and after the tail trimmed, a
  2 ms fade in and a short fade out;
- loops: no trim or fade; a seam that jumps is crossfaded round, so the
  loop plays seamlessly;
- every take: mono for a point source, levelled to its bus's target RMS,
  peaks held under -1 dBFS by a soft limiter, 48 kHz Ogg Vorbis q4.

Then writes the takes into `audio/cues.toml` with their provenance and
measurements, the sample `approved`, and every take's measurements to
`tools/audio/_takes/<sample>/metrics.json` for the audition page.

    python3 -I tools/audio/process.py --batch tools/audio/batches/<batch>.json [sample ...]

ffmpeg does the decoding and the encoding; the measuring is plain Python.
"""

import argparse
import array
import json
import math
import pathlib
import re
import subprocess
import sys
import tomllib

REPO = pathlib.Path(__file__).resolve().parents[2]
TAKES = REPO / "tools" / "audio" / "_takes"
STATIC = REPO / "static" / "audio"
MANIFEST = REPO / "audio" / "cues.toml"
RATE = 48_000
CEILING = 10 ** (-1.0 / 20)

# Each bus's level, as the RMS (dBFS) of a take's sounding part.
TARGET_RMS_DB = {
    "weapons": -15.0,
    "impacts": -17.0,
    "explosions": -14.0,
    "vehicles": -20.0,
    "world": -24.0,
    "warnings": -16.0,
    "ui": -20.0,
    "voice": -18.0,
    "music": -20.0,
}

# How many takes a sample ships: more for what is heard often.
TAKES_KEPT = {"ui": 1, "warnings": 1, "music": 1}
DEFAULT_KEPT = 2


def decode(path: pathlib.Path, channels: int) -> array.array:
    raw = subprocess.run(
        ["ffmpeg", "-v", "error", "-i", str(path), "-f", "f32le", "-ac", str(channels), "-ar", str(RATE), "-"],
        check=True,
        capture_output=True,
    ).stdout
    a = array.array("f")
    a.frombytes(raw)
    return a


def encode(samples: array.array, channels: int, out: pathlib.Path) -> None:
    out.parent.mkdir(parents=True, exist_ok=True)
    subprocess.run(
        ["ffmpeg", "-v", "error", "-y", "-f", "f32le", "-ar", str(RATE), "-ac", str(channels), "-i", "-",
         "-c:a", "libvorbis", "-q:a", "4", str(out)],
        input=samples.tobytes(),
        check=True,
    )


def frame_peaks(s: array.array, ch: int) -> list[float]:
    return [max(abs(s[i * ch + c]) for c in range(ch)) for i in range(len(s) // ch)]


def db(x: float) -> float:
    return -120.0 if x <= 1e-6 else 20.0 * math.log10(x)


def window_rms(s: array.array, ch: int, start: int, n: int) -> float:
    acc = 0.0
    count = 0
    for i in range(start, min(start + n, len(s) // ch)):
        for c in range(ch):
            v = s[i * ch + c]
            acc += v * v
            count += 1
    return math.sqrt(acc / count) if count else 0.0


def measure(s: array.array, ch: int, looped: bool) -> dict:
    peaks = frame_peaks(s, ch)
    n = len(peaks)
    peak = max(peaks) if peaks else 0.0
    win = RATE // 100  # 10 ms
    rms_windows = [window_rms(s, ch, i, win) for i in range(0, n, win)]
    active = [r for r in rms_windows if db(r) > -45.0]
    rms_active = math.sqrt(sum(r * r for r in active) / len(active)) if active else 0.0
    onset = next((i for i, p in enumerate(peaks) if p > 0.03), n)
    tail = next((n - i for i, p in enumerate(reversed(peaks)) if p > 0.003), 0)
    clipped = sum(1 for p in peaks if p >= 0.999)
    m = {
        "seconds": n / RATE,
        "peak_db": db(peak),
        "rms_db": db(rms_active),
        "crest_db": db(peak) - db(rms_active),
        "onset_s": onset / RATE,
        "tail_s": tail / RATE,
        "clipped": clipped,
    }
    if looped and n > 2 * win:
        # The seam: how far the last frame is from the first, against the
        # loop's own level; and how steady the loop's level is.
        jump = max(abs(s[(n - 1) * ch + c] - s[c]) for c in range(ch))
        m["seam_jump"] = jump / max(rms_active, 1e-4)
        dbs = [db(r) for r in rms_windows if r > 0]
        mean = sum(dbs) / len(dbs)
        m["steadiness_db"] = math.sqrt(sum((d - mean) ** 2 for d in dbs) / len(dbs))
    return m


def score(m: dict, looped: bool, bus: str) -> float:
    """Higher is better: what the measurements can say about a take."""
    s = -2.0 * m["clipped"] / max(m["seconds"] * RATE, 1) * 1000
    if looped:
        s -= 4.0 * m.get("seam_jump", 0.0)
        s -= 0.4 * m.get("steadiness_db", 0.0)
    else:
        # A hit, a shot, a blast starts at once.
        s -= 20.0 * max(m["onset_s"] - 0.01, 0.0)
        if bus in ("weapons", "impacts", "explosions"):
            s += min(m["crest_db"], 18.0) / 6.0
    # A take that is mostly silence is a poor take.
    s -= 3.0 * max(0.0, 0.2 - (m["seconds"] - m["onset_s"] - (m["seconds"] - m["tail_s"])))
    return s


def trim(s: array.array, ch: int, m: dict, fade_out: float) -> array.array:
    n = len(s) // ch
    start = max(int(m["onset_s"] * RATE) - int(0.004 * RATE), 0)
    end = min(int(m["tail_s"] * RATE) + int(0.02 * RATE), n)
    out = array.array("f", s[start * ch:end * ch])
    frames = len(out) // ch
    fade_in_n = max(int(0.002 * RATE), 1)
    fade_out_n = max(min(int(fade_out * RATE), frames // 2), 1)
    for i in range(min(fade_in_n, frames)):
        g = i / fade_in_n
        for c in range(ch):
            out[i * ch + c] *= g
    for i in range(fade_out_n):
        g = i / fade_out_n
        j = frames - 1 - i
        for c in range(ch):
            out[j * ch + c] *= g
    return out


def seam(s: array.array, ch: int, m: dict) -> array.array:
    """Crossfades a loop's end into its start when the seam jumps."""
    if m.get("seam_jump", 0.0) < 0.5:
        return s
    n = len(s) // ch
    xf = min(int(0.04 * RATE), n // 4)
    out = array.array("f", s[: (n - xf) * ch])
    for i in range(xf):
        t = i / xf
        a, b = math.cos(t * math.pi / 2), math.sin(t * math.pi / 2)
        for c in range(ch):
            out[i * ch + c] = s[(n - xf + i) * ch + c] * a + s[i * ch + c] * b
    return out


def level(s: array.array, ch: int, bus: str, rms_db: float) -> array.array:
    """Brings the sounding part to the bus's target and holds peaks under
    the ceiling with a gentle limiter (instant attack, 60 ms release)."""
    g = 10 ** ((TARGET_RMS_DB.get(bus, -18.0) - rms_db) / 20.0)
    g = min(g, 30.0)
    out = array.array("f", (v * g for v in s))
    env = 1.0
    release = 1.0 - math.exp(-1.0 / (0.06 * RATE))
    for i in range(len(out) // ch):
        p = max(abs(out[i * ch + c]) for c in range(ch))
        need = CEILING / p if p > CEILING else 1.0
        env = need if need < env else env + (1.0 - env) * release
        env = min(env, need)
        for c in range(ch):
            out[i * ch + c] *= env
    return out


def load_manifest() -> dict:
    return tomllib.loads(MANIFEST.read_text())


def write_takes(text: str, sample: str, takes: list[dict]) -> str:
    """Replaces `sample`'s take blocks and sets it approved, keeping every
    other line and comment of the manifest as it was."""
    lines = text.split("\n")
    header = f"[sample.{sample}]"
    start = lines.index(header)
    end = start + 1
    while end < len(lines) and not (lines[end].startswith("[") and not lines[end].startswith(f"[[sample.{sample}.take]]")):
        end += 1
    body = []
    for line in lines[start + 1:end]:
        if line.strip().startswith(f"[[sample.{sample}.take]]"):
            break
        body.append(line)
    while body and not body[-1].strip():
        body.pop()
    body = [l for l in body if not l.startswith("status = ")]
    block = [header, 'status = "approved"', *body]
    for t in takes:
        block.append(f"  [[sample.{sample}.take]]")
        for k in ("file", "flow", "generation", "made", "plan"):
            if t.get(k):
                block.append(f'  {k} = "{t[k]}"')
        for k in ("rms_db", "peak_dbtp", "seconds"):
            if k in t:
                block.append(f"  {k} = {t[k]:.2f}")
    block.append("")
    return "\n".join(lines[:start] + block + lines[end:])


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("--batch", required=True, help="tools/audio/batches/<batch>.json")
    ap.add_argument("--made", default=None, help="the date the takes were made (default: the batch's)")
    ap.add_argument("--plan", default="creator")
    ap.add_argument("samples", nargs="*")
    args = ap.parse_args()
    batch = json.loads(pathlib.Path(args.batch).read_text())
    made = args.made or batch.get("made") or batch["name"].split()[2]
    manifest = load_manifest()
    names = args.samples or sorted(set(batch["nodes"].values()))
    text = MANIFEST.read_text()
    for name in names:
        sample = manifest["sample"][name]
        bus = sample["bus"]
        looped = sample.get("loop", False)
        ch = sample.get("channels", 1)
        files = sorted((TAKES / name).glob("*.mp3"))
        if not files:
            print(f"{name}: no takes", file=sys.stderr)
            continue
        measured = []
        for f in files:
            s = decode(f, ch)
            m = measure(s, ch, looped)
            m["generation"] = f.stem
            m["score"] = score(m, looped, bus)
            measured.append((m, s))
        measured.sort(key=lambda ms: ms[0]["score"], reverse=True)
        (TAKES / name / "metrics.json").write_text(json.dumps([m for m, _ in measured], indent=1))
        kept = TAKES_KEPT.get(bus, DEFAULT_KEPT)
        takes = []
        for n, (m, s) in enumerate(measured[:kept], start=1):
            if looped:
                out = seam(s, ch, m)
            else:
                fade = 0.06 if bus in ("explosions", "world") else 0.02
                out = trim(s, ch, m, fade)
            after = measure(out, ch, looped)
            out = level(out, ch, bus, after["rms_db"])
            final = measure(out, ch, looped)
            rel = f"{bus}/{name}_{n}.ogg"
            encode(out, ch, STATIC / rel)
            takes.append({
                "file": rel,
                "flow": batch["flow"],
                "generation": m["generation"],
                "made": made,
                "plan": args.plan,
                "rms_db": final["rms_db"],
                "peak_dbtp": final["peak_db"],
                "seconds": final["seconds"],
            })
            print(f"{name}: take {n} {m['generation']} score {m['score']:.2f} -> {rel} ({final['seconds']:.2f} s, {final['rms_db']:.1f} dB RMS, peak {final['peak_db']:.1f} dB)")
        text = write_takes(text, name, takes)
    MANIFEST.write_text(text)
    tomllib.loads(MANIFEST.read_text())
    return 0


if __name__ == "__main__":
    sys.exit(main())
