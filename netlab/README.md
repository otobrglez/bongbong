# netlab

How far networked play is from local play, per metric, on a given link
(docs/online-coop-prd.md §4.16, "Measurement first").

A run puts the real room server (`bongbong-server`) in this process on an
ephemeral port, an impairment proxy in front of it, and two headless
clients behind the proxy. Each client is the window's own
`net::round::OnlineRound` over the real `net::native::NativeTransport`,
driven by a script on its own thread at a fixed frame rate. The same
scripts then go through a **local twin** - a two-seat `Game` stepped the way
`app.rs` steps a local round (`StepClock`: whole fixed steps, at most four a
frame) at the same frame rate - which is the "feels local" reference. Every
stamp is on one process clock, so end-to-end latencies are exact, and every
metric is one code path over the frames the online clients drew and the
frames the twin drew.

```
just netlab run --profile typical --scenario shoot
just netlab run --profile custom --delay-ms 80 --jitter-ms 20 --loss 0.02 --nagle
just netlab run --remote wss://rooms.bongbong.io/pr-48 --scenario duel
just netlab-suite --quick          # three profiles x three scenarios x hull on/off
just netlab-suite                  # every profile, 20 s a run
```

`run` prints a summary (`--json-out F` writes the whole report, `--frames-out
F` every frame each seat and the twin recorded - hulls, shots, events and the
client's readings - and `--quiet` prints nothing); `suite` prints one
markdown table, each run in its own process because the tuning table and the
room server's base table are process-wide. For the same reason a test that
calls `run::run` is the only test in its binary (`tests/lan_drive.rs`,
`tests/late_server.rs`).

## The link

The proxy models TCP, not packets (`src/link.rs`). Per direction it reads
chunks as they arrive (`TCP_NODELAY` on every socket), and delivers each at
`max(previous delivery, arrival + delay + U[0, jitter])` - monotone, never
reordered. Loss holds a chunk until `arrival + delay + rto` and, through the
`max`, everything behind it: head-of-line blocking as TCP does it. `--nagle`
coalesces a write made while the previous one is unacknowledged until that
round trip completes.

| profile | one-way delay | jitter | loss | rto |
|---|---|---|---|---|
| lan | 0 ms | 0 | 0 | 200 ms |
| good | 15 ms | 3 ms | 0 | 200 ms |
| typical | 40 ms | 10 ms | 0 | 200 ms |
| mobile | 60 ms | 25 ms | 1% | 200 ms |
| bad | 100 ms | 40 ms | 3% | 200 ms |

`rto` defaults to `max(200 ms, 2 x delay)`; `--rto-ms` overrides it.

`--remote URL` skips the server and the proxy and dials a real rooms host,
spelled as `--rooms` takes it (`wss://rooms.bongbong.io/pr-48`, with or
without `/ws`); an `http(s)://` address is read as its socket's scheme and a
bare `host:port` gets `ws://` on this machine and `wss://` anywhere else. A
dial the network refuses before the socket opens is made again every 250 ms
for up to 20 s, so netlab can be started beside a room server that is still
coming up (the summary counts the refused dials). The tap's metrics are
absent, and the presses are paired with their `Fired` by order.

## The scenarios

Played on `maps/arena.toml` (an open field, so a stall is the link's and not
a wall's), both seats in the same chassis (`--tank`, default the
single-barrelled scout, so one press is one `Fired`), band enemies,
`--mission destroy`, seed `0xB0B5`.

No scripted press may be refused for ammo. In-process the room shares this
process's tuning table, so the magazine (`max_shells`) is pinned at 100 for
the room, the clients and the twin. A remote room's magazine is its own and
nothing is pinned anywhere: each script stops tapping after the shells a
tank starts with (`max_shells` as this build ships it, 20), in the twin too.
The report records which (`max_shells`, `tap_limit`).

- **drive** - the host drives a rectangle (right 1.2 s, down 0.8, left 1.2,
  up 0.8), the guest stands still. No enemies.
- **shoot** - the host faces the enemies (the direction of their centre when
  the round begins), taps fire every 400 ms and strafes a 0.3 s step across
  that line every 2 s. Four enemies.
- **duel** - both seats drive rectangles half a lap out of phase and tap
  every 600 ms. Two enemies.

A round that ends before the script does (the host wrecked, every enemy
wrecked) stops the recording there; the summary says so.

## The metrics

Every frame records each seat's own scripted input (the twin both seats', a
client its own), the hulls and shots as drawn - each shot's firer read by id
(`Game::seat_shots`), a provisional marked as the local seat's - and, online,
the client's readings (`FrameSample::link`).

- **own input** - from each of a seat's scripted direction changes to the
  first frame that seat's drawn hull has moved 0.5 px the new way, in ms and
  in frames (the verdict reads frames, so a frame the scheduler delivered
  late is not counted as latency). The host's, and in duel the guest's too.
- **remote pacing** - the host's hull as the guest draws it (twin: as drawn
  locally), over the rectangle's straight legs less 150 ms after each turn:
  per-frame displacement `d` against `e = |v| x frame dt` - stall
  (`d < 0.25e`), jump (`d > 1.75e`), backward (against travel), and the
  coefficient of variation of `d / e`.
- **remote lag** - for each guest frame, when the host itself drew its hull
  where the guest draws it (a search of the host's own drawn path on the
  shared clock). Rectangle scenarios only: the strafe retraces itself.
- **shot ledger** (the host's presses; `src/shots.rs` follows the drawn
  shots - a room copy by the room's id, a provisional by continuity, since a
  provisional's id is an index into the client's live shots that shifts as
  older ones retire):
  - *drawn* - press to the first frame its shot is drawn leaving the muzzle
    (the provisional where the client draws one, the room's copy where not);
  - *fired* - press to the frame its `Fired` is handed over. In-process the
    tap pairs each press with the `Fired` that names its own intent's tick
    and reads which snapshot carried it (`by input_tick`); remote and in the
    twin, oldest first (`by order`);
  - *hand-offs* - an own shot appearing anywhere but the muzzle, taken as the
    own shot that left the picture last in the 250 ms before it: the gap is
    how far the shot jumped (a provisional swapped for the room's copy, a
    room copy shown after its provisional was gone, a provisional snapped to
    a room impact). On the present timeline nothing is swapped, so the count
    is the finding;
  - *drawn twice* - a room copy that appears while the provisional standing
    for the same shot is still drawn ahead of it on its path;
  - *room copies shown* - distinct room copies of the host's shots drawn at
    all, on a client that draws its own from the press;
  - *hit* - press to the first enemy hit (shoot only, where the host is the
    only one shooting), and *guest fired* - press to the guest's `Fired`.
- **incoming fire** - for each hit on the host, the shot that made it: a
  shot not the host's that the picture stopped in the second before the
  hit was handed over, either taken off in flight on a path through the
  room's impact point (a client drawing incoming fire in the present takes
  a shot off the picture where its path meets the drawn hull, and draws
  its impact there) or bursting at that point in its impact frames. *From
  afar* is the gap between where the picture stopped it and the drawn
  hull's hit boxes (hull and turret, as `Tank::hull_bbox_world` and
  `turret_bbox_world` build them, grown by the shot's half extent), on that
  frame: nought locally, and online for a strike drawn at the hull; a shot
  drawn passing the hull that the room then bursts where the hull was a
  moment ago is hit from afar. *Strike to hit* is how long the strike stood
  before the damage was handed over; hits no drawn shot accounts for (a
  beam, a ram, a blast, a shot never drawn) are counted, and so are
  *strikes drawn with no hit* - a shot stopped at the hull the room judged
  a miss.
- per seat: prediction nudges, snaps and max error, the lead's ups and downs,
  the interpolation delay, jitter, lateness p50/p95, extrapolated frames,
  head-of-line stalls, the playout rate, the interpolator's corrections, the
  round trip, and refused dials. The report's `series` has them frame by
  frame for each seat (cumulative counters, this frame's delay, target,
  lateness, rate, buffer, round trip and whether it extrapolated).
- **tap** (in-process only; `src/wstap.rs` reads the WebSocket frames going
  through the proxy): one-way delay each way as applied, the server hold
  (an intent delivered to the first snapshot leaving the server whose
  `acked` covers it), snapshot inter-arrival and stalls over 50/100/250 ms,
  bytes per second each way, and every `Fired` with the input tick it names.
- frame CPU of `OnlineRound::frame` (twin: the frame's `Game::update`s) and
  the frame loop's own intervals.

## The verdict

`local` when every one holds, `far` when any misses by more than three times
its allowance (more than five corrections or doubles), `close` otherwise:

- own input p95 no more than one frame over the twin's;
- remote stall% no more than two points over the twin's, jump% at most 2,
  backward% at most 0.5;
- hand-off gap p95 at most 2 px;
- no own shot drawn twice;
- hit from afar p95 at most 12 px;
- no nudges or snaps with the client owning its hull.
