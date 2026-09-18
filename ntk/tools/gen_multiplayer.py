"""Multiplayer follow-up to the build-pipeline sketch:
ntk/assets/multiplayer.svg (+ .png at 4x). Same style. Top half: the
source split into shared / client / server crates, the client row (the four
pipelines of the previous sketch, collapsed) and the new server row through
a container image. Bottom half: runtime - clients on WebSockets to a room
on the server, both sides running the same deterministic simulation.

    PYTHONPATH=ntk/tools nix-shell -p librsvg --run "python3 ntk/tools/gen_multiplayer.py"
"""
from sketch import Sketch

s = Sketch()

# --- build: source crates -------------------------------------------------
s.box(60, 100, 330, 130, ["simulation/ (Game::update)", "map · tuning · protocol", "no raylib, no sockets"], title="shared crate", size=19)
s.box(60, 260, 330, 110, ["raylib render + input", "builder, HUD, touch"], title="client crate", size=19)
s.box(60, 400, 330, 110, ["headless, tokio", "WebSocket rooms, lobby"], title="server crate", size=19)
# shared feeds both
s.arrow(225, 230, 225, 260)
s.line(40, 165, 60, 165)
s.line(40, 165, 40, 455)
s.arrow(40, 455, 60, 455)

s.arrow(390, 315, 470, 315)
s.arrow(390, 455, 470, 455)
s.box(470, 300, 200, 200, ["rustc →", "LLVM →", "machine code"], title="LLVM", size=20)

cols = [786, 1070, 1354, 1638]
BW, BH = 262, 130
rows = [
    ("CLIENT", 120, [
        ["4 target triples:", "desktop · wasm · iOS · Android"],
        ["the four pipelines", "of the previous sketch"],
        ["binary · .wasm · .app · .apk", "+ static/ assets"],
        ["players", "(releases, bongbong.io, stores)"]]),
    ("SERVER", 380, [
        ["x86_64-unknown-linux-musl", "no raylib, no GPU"],
        ["static binary →", "FROM scratch image", "(docker build)"],
        ["container registry →", "orchestrator (k8s / Fly.io)", "N replicas"],
        ["wss:// endpoint", "TLS at the edge"]]),
]
BUS = 740
s.line(670, 400, BUS, 400)
s.line(BUS, rows[0][1] + BH / 2, BUS, rows[-1][1] + BH / 2)
for name, y, boxes in rows:
    cy = y + BH / 2
    s.text(cols[0], y - 16, name, 20, 700)
    s.arrow(BUS, cy, cols[0], cy)
    for i, lines in enumerate(boxes):
        s.box(cols[i], y, BW, BH, lines, size=18)
        if i < 3:
            s.arrow(cols[i] + BW, cy, cols[i + 1], cy)

# --- runtime -------------------------------------------------------------
s.region(60, 620, 1840, 400, "RUNTIME")
clients = ["Desktop", "Browser (wasm)", "iPhone", "Android"]
for i, name in enumerate(clients):
    y = 660 + i * 84
    s.box(100, y, 340, 74, [name, "same Game::update, renders"], size=17, bold_first=True)
    s.line(440, y + 37, 470, y + 37)
s.line(470, 697, 470, 949)          # client bus
s.arrow(470, 819, 700, 819, both=True)
s.text(585, 790, "WebSocket", 18, 700, "middle")
s.text(585, 848, "wss://", 18, 400, "middle")

# messages
s.box(700, 660, 420, 100, ["↑ per-frame Input intents,", "join / ready / map choice"], size=18)
s.box(700, 880, 420, 100, ["↓ seed + everyone's inputs per tick,", "state hash, snapshot for late join"], size=18)
s.line(910, 760, 910, 880)

# server
s.box(1160, 660, 700, 320, [
    "one room = one seeded Game, stepped in lockstep",
    "at PHYSICS_FIXED_DT; inputs collected per tick,",
    "relayed to every client; the authoritative state",
    "hash detects a desync; snapshots resync a client",
    "",
    "lobby / matchmaking · rooms per container replica",
], title="server container", size=18)
s.arrow(1120, 819, 1160, 819, both=True)

s.save("ntk/assets/multiplayer.svg", "ntk/assets/multiplayer.png")
