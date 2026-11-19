"""The repeatable perf scene: what the window draws and how its frames are
paced on fixed rounds, read off a running dev-tools build's dev server.

  cargo run --release --features dev-tools -- --resolution 1280x720   (one terminal)
  python3 tools/perf/scene.py                                         (`just perf-scene`)

For each scene (a shipped map at a pinned seed) it restarts the round,
steps it in lockstep with seat 0 driving and firing, and after every step
reads `status.frame_stages.draws` - what the frozen frame drew through
`GpuCanvas` and `Rl`, and the blend, shader and render-target scopes it
opened (src/frame_stages.rs). The particle cap and the wall-piece cap are
pinned to 0 for the run and put back after it: particles and pieces draw
from `rand::rng()`, everything else on a stepped frame is the round's.
The counters are read twice over, and the run fails unless both passes
agree number for number.

Then it lets each scene run in real time for `--seconds`, the two caps
back where they were, and reads the stage averages, the last frame's
draws and `status.frame_stages.pace`: the worst frame and the 99th
percentile over the last 600 frames and how many steps each ran. Those
are this machine's, this window's, and differ run to run.

`counters:` lines are the repeatable half, `timing:` lines the measured
half. BONGBONG_DEV_PORT picks the game (default 4747), as `just mcp-call`
does.
"""

import argparse
import json
import os
import socket
import sys
import time

SEED = "0xB0B5"
# (name, restart parameters). The default arena shown whole, and the
# largest shipped field under the follow camera.
SCENES = [
    ("default", {"map": "maps/default.toml", "enemies": 6}),
    ("longwater", {"map": "maps/longwater.toml"}),
]
# Each pass steps this many times, this many frames a time, seat 0 driving
# round the compass a leg a step with the trigger held one frame in ten.
STEPS = 8
FRAMES = 60
LEGS = ["up", "right", "down", "left"]
# The presentation layers that roll their own dice (`fx.rs`, `pieces.rs`),
# switched off while the counters are read.
PINNED = {"fx_max_particles": 0, "fx_max_pieces": 0}
# A stepped frame is drawn before its counters are read: two frames at the
# slowest refresh this is run under.
SETTLE = 0.2


def call(port, method, params=None):
    with socket.create_connection(("127.0.0.1", port), timeout=5) as s:
        s.settimeout(120)
        s.sendall((json.dumps({"id": 1, "method": method, "params": params or {}}) + "\n").encode())
        line = s.makefile().readline()
    reply = json.loads(line)
    if "error" in reply:
        raise SystemExit(f"perf-scene: {method}: {reply['error']}")
    return reply.get("result")


def counters(port, name, params):
    """One pass over a scene: the draws after each step."""
    call(port, "restart", {"seed": SEED, "players": 1, **params})
    rows = []
    for i in range(STEPS):
        call(port, "step", {"frames": FRAMES, "move_dir": LEGS[i % len(LEGS)], "fire": True, "fire_every": 10})
        time.sleep(SETTLE)
        status = call(port, "status", {"sections": ["frame_stages"]})
        draws = status["frame_stages"]["draws"]
        rows.append((status["frame"], draws))
    return rows


def main():
    parser = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    parser.add_argument("--seconds", type=float, default=12.0, help="real time each scene runs for its pace (0 skips the timing half)")
    args = parser.parse_args()
    port = int(os.environ.get("BONGBONG_DEV_PORT", "4747"))

    status = call(port, "status", {"sections": ["frame_stages"]})
    if status["mode"] != "play":
        raise SystemExit(f"perf-scene: the window is in {status['mode']} mode; `just mcp-call play` first")
    if "draws" not in status.get("frame_stages", {}):
        raise SystemExit("perf-scene: this game reports no draw counters; build it from this tree with --features dev-tools")
    print(f"perf-scene: version {status['version']} port {port} seed {SEED}")
    knobs = call(port, "tuning_get")
    kept = {knob: knobs[knob] for knob in PINNED}
    call(port, "tuning_set", {"patch": PINNED})
    failed = False
    try:
        for name, params in SCENES:
            first = counters(port, name, params)
            second = counters(port, name, params)
            for frame, draws in first:
                print(f"counters: {name} frame={frame} " + " ".join(f"{k}={v}" for k, v in draws.items()))
            if first != second:
                failed = True
                print(f"perf-scene: {name}: the two passes DISAGREE", file=sys.stderr)
                for a, b in zip(first, second):
                    if a != b:
                        print(f"  first  {a}\n  second {b}", file=sys.stderr)
            if args.seconds > 0:
                # The second pass's round, let go: the same state every run
                # at the moment it starts.
                ui = call(port, "status", {"sections": ["ui"]})["ui"]
                window = f"{ui['screen']['w'] * ui['scale']:.0f}x{ui['screen']['h'] * ui['scale']:.0f}"
                # Timed as it is played: the particles and pieces back on.
                call(port, "tuning_set", {"patch": kept})
                call(port, "resume")
                time.sleep(args.seconds)
                stages = call(port, "status", {"sections": ["frame_stages"]})["frame_stages"]
                call(port, "pause")
                call(port, "tuning_set", {"patch": PINNED})
                pace = stages.pop("pace")
                draws = stages.pop("draws")
                print(
                    f"timing: {name} window={window} frames={pace['frames']} p99_ms={pace['p99_ms']:.2f} max_ms={pace['max_ms']:.2f} "
                    f"steps={'/'.join(str(n) for n in pace['steps'])} | " + " ".join(f"{k}={v:.3f}" for k, v in stages.items())
                    + " | last frame: " + " ".join(f"{k}={v}" for k, v in draws.items())
                )
    finally:
        call(port, "tuning_set", {"patch": kept})
    if failed:
        raise SystemExit(1)
    print("perf-scene: both passes of every scene drew the same counters")


if __name__ == "__main__":
    main()
