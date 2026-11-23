#!/usr/bin/env python3
"""Download generated takes from ElevenLabs (docs/sound-system-prd.md 4.12).

The `sound-gen` skill reads each run's signed URLs with the connector
(`creative_get_flow_run_status`) and hands them here as a TSV, one take a
line: `<sample id>\t<generation id>\t<url>`. Each lands at
`tools/audio/_takes/<sample>/<generation>.mp3` (gitignored: the flow keeps
every take, so they can be fetched again). A take already on disk is kept.

    python3 -I tools/audio/fetch.py takes.tsv [more.tsv ...]
"""

import pathlib
import sys
import urllib.request

ROOT = pathlib.Path(__file__).resolve().parent / "_takes"


def fetch(sample: str, generation: str, url: str) -> str:
    out = ROOT / sample / f"{generation}.mp3"
    if out.exists() and out.stat().st_size > 0:
        return "kept"
    out.parent.mkdir(parents=True, exist_ok=True)
    req = urllib.request.Request(url, headers={"User-Agent": "bongbong-sound-gen"})
    with urllib.request.urlopen(req, timeout=60) as resp:
        data = resp.read()
    tmp = out.with_suffix(".part")
    tmp.write_bytes(data)
    tmp.rename(out)
    return f"{len(data)} bytes"


def main(paths: list[str]) -> int:
    failed = 0
    for path in paths:
        for line in pathlib.Path(path).read_text().splitlines():
            if not line.strip() or line.startswith("#"):
                continue
            sample, generation, url = line.split("\t")
            try:
                print(f"{sample}/{generation}: {fetch(sample, generation, url)}")
            except Exception as e:  # noqa: BLE001 - report and go on
                failed += 1
                print(f"{sample}/{generation}: FAILED {e}", file=sys.stderr)
    return 1 if failed else 0


if __name__ == "__main__":
    sys.exit(main(sys.argv[1:]))
