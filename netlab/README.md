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

`run` prints a summary (`--json-out F` writes the whole report,
`--quiet` prints nothing); `suite` prints one markdown table, each run in its
own process because the tuning table and the room server's base table are
process-wide.

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
`--remote URL` skips the server and the proxy and dials a real rooms host
(`--rooms` spelling, with or without `/ws`); the tap's metrics are then
absent.

## The scenarios

Played on `maps/arena.toml` (an open field, so a stall is the link's and not
a wall's), both seats in the same chassis (`--tank`, default the
single-barrelled scout, so one press is one `Fired`), band enemies,
`--mission destroy`, seed `0xB0B5`. Every scripted shooter gets a full
magazine (`max_shells` 100) so a press is never refused for ammo.

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

- **own input** - from each scripted direction change to the first frame the
  drawn own hull has moved 0.5 px the new way, in ms and in frames (the
  verdict reads frames, so a frame the scheduler delivered late is not
  counted as latency).
- **remote pacing** - the host's hull as the guest draws it (twin: as drawn
  locally), over the rectangle's straight legs less 150 ms after each turn:
  per-frame displacement `d` against `e = |v| x frame dt` - stall
  (`d < 0.25e`), jump (`d > 1.75e`), backward (against travel), and the
  coefficient of variation of `d / e`.
- **remote lag** - for each guest frame, when the host itself drew its hull
  where the guest draws it (a search of the host's own drawn path on the
  shared clock). Rectangle scenarios only: the strafe retraces itself.
- **shot ledger** (host's presses) - press to the first own shot drawn
  (provisional, ids from `net::predict::PROVISIONAL_ID_BASE`), press to its
  `Fired` handed over, the hand-off gap (on the frame a provisional
  disappears, how far the room's copy is), press to the first enemy hit
  (shoot only, where the host is the only one shooting), press to the
  guest's `Fired`.
- **incoming fire** - for each hit on the host, the gap between the drawn
  hull's hit boxes (hull and turret, as `Tank::hull_bbox_world` and
  `turret_bbox_world` build them, grown by the shell's half extent) and the
  enemy shot that hit, carried from the frame before to the hit frame along
  its drawn motion: nought locally, "hit from afar" online.
- per seat: prediction nudges, snaps and max error, the lead's ups and downs,
  the interpolation delay, jitter and extrapolated frames, the round trip.
- **tap** (in-process only; `src/wstap.rs` reads the WebSocket frames going
  through the proxy): one-way delay each way as applied, the server hold
  (an intent delivered to the first snapshot leaving the server whose
  `acked` covers it), snapshot inter-arrival and stalls over 50/100/250 ms,
  bytes per second each way.
- frame CPU of `OnlineRound::frame` (twin: the frame's `Game::update`s) and
  the frame loop's own intervals.

## The verdict

`local` when every one holds, `far` when any misses by more than three times
its allowance (more than five corrections), `close` otherwise:

- own input p95 no more than one frame over the twin's;
- remote stall% no more than two points over the twin's, jump% at most 2,
  backward% at most 0.5;
- hand-off gap p95 at most 2 px;
- hit from afar p95 at most 12 px;
- no nudges or snaps with the client owning its hull.
