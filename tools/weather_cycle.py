#!/usr/bin/env python3
"""Cycle the running game through every sky, a few seconds each.

Talks to the dev server (`just run-dev`, 127.0.0.1:4747) directly: one
newline-delimited JSON request per line, `{"id", "method", "params"}`.
Each sky changes mid-round - the round plays on (`Game::change_weather`);
snow ices the water over, and ice stays ice until the next round.

    python3 tools/weather_cycle.py              # 6 s each, round and round
    python3 tools/weather_cycle.py --seconds 3 --once
"""

import argparse
import json
import os
import socket
import time

SKIES = ["clear", "night", "dusk", "rain", "storm", "fog", "sandstorm", "snow" ]

#, "heat_haze"]


def call(sock_file, method, params=None, _id=[0]):
    _id[0] += 1
    sock_file.write(json.dumps({"id": _id[0], "method": method, "params": params or {}}) + "\n")
    sock_file.flush()
    reply = json.loads(sock_file.readline())
    if "error" in reply:
        raise RuntimeError(f"{method}: {reply['error']}")
    return reply.get("result")


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("--seconds", type=float, default=6.0, help="seconds per sky (default 6)")
    ap.add_argument("--port", type=int, default=int(os.environ.get("BONGBONG_DEV_PORT", 4747)))
    ap.add_argument("--once", action="store_true", help="go through the skies once, then stop")
    args = ap.parse_args()

    with socket.create_connection(("127.0.0.1", args.port)) as sock:
        f = sock.makefile("rw", encoding="utf-8")
        try:
            while True:
                for sky in SKIES:
                    call(f, "weather", {"name": sky})
                    print(f"{time.strftime('%H:%M:%S')}  {sky}", flush=True)
                    time.sleep(args.seconds)
                if args.once:
                    break
        except KeyboardInterrupt:
            print()


if __name__ == "__main__":
    main()
