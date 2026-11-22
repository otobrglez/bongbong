#!/usr/bin/env python3
"""Build a batch's audition page (docs/sound-system-prd.md 4.12, step 7).

Reads the batch file, the manifest, each sample's measured takes
(`tools/audio/_takes/<sample>/metrics.json`, written by process.py) and a
map of the takes' uploaded URLs, and writes one self-contained page from
`tools/audio/audition/template.html`. The `sound-gen` skill uploads the
takes as the audition artifact's assets, writes the map, builds the page and
publishes it; the owner's picks come back through the artifact's database
(`verdicts/<sample>`).

    python3 -I tools/audio/audition.py --batch tools/audio/batches/<batch>.json \\
        --urls urls.json --out target/audition/index.html \\
        [--round-url URL --round-caption TEXT] [--credits TEXT]

`urls.json` maps each take's generation id to its URL.
"""

import argparse
import json
import pathlib
import tomllib

REPO = pathlib.Path(__file__).resolve().parents[1].parent
HERE = pathlib.Path(__file__).resolve().parent


def main() -> None:
    ap = argparse.ArgumentParser()
    ap.add_argument("--batch", required=True)
    ap.add_argument("--urls", required=True)
    ap.add_argument("--out", required=True)
    ap.add_argument("--round-url")
    ap.add_argument("--round-caption", default="")
    ap.add_argument("--credits", default="")
    args = ap.parse_args()

    batch = json.loads(pathlib.Path(args.batch).read_text())
    urls = json.loads(pathlib.Path(args.urls).read_text())
    manifest = tomllib.loads((REPO / "audio" / "cues.toml").read_text())
    samples = []
    for name in dict.fromkeys(batch["nodes"].values()):
        sample = manifest["sample"][name]
        metrics_path = REPO / "tools" / "audio" / "_takes" / name / "metrics.json"
        metrics = json.loads(metrics_path.read_text()) if metrics_path.exists() else []
        takes = [{"generation": m["generation"], "url": urls[m["generation"]], "metrics": m} for m in metrics if m["generation"] in urls]
        takes.sort(key=lambda t: t["generation"])
        cues = sorted(cid for cid, cue in manifest["cue"].items() if any(l["sample"] == name for l in cue.get("layers", [])))
        samples.append({
            "id": name,
            "bus": sample["bus"],
            "prompt": sample["prompt"],
            "looped": sample.get("loop", False),
            "keep": max(len(sample.get("take", [])), 1),
            "shipped": [t["generation"] for t in sample.get("take", [])],
            "cues": cues,
            "takes": takes,
        })
    data = {
        "batch": {"name": batch["name"], "made": batch.get("made") or batch["name"].split()[2], "model": batch["model"], "credits": args.credits or f"~{batch.get('estimate_credits', '?')} credits"},
        "round": {"url": args.round_url, "caption": args.round_caption} if args.round_url else None,
        "samples": samples,
    }
    page = (HERE / "audition" / "template.html").read_text().replace("__DATA__", json.dumps(data).replace("</", "<\\/"))
    out = pathlib.Path(args.out)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(page)
    print(f"{out}: {len(samples)} samples, {sum(len(s['takes']) for s in samples)} takes")


if __name__ == "__main__":
    main()
