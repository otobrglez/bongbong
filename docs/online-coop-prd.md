# PRD: bongbong online co-op

Written 2026-09-15, re-checked against the tree on 2026-09-19 (section 3 names
the commit). Same shape as docs/android-port-prd.md: the decision, what
exists today, the design by area, the phases, the risks. The design study
behind it, with diagrams and the option comparison, is the "Bongbong Online"
page (rev 2); this document is the version that lives with the code and is the
one to update as things land. Nothing in the tree talks to a network yet.

## 1. The decision, in one paragraph

Multiplayer is **server-authoritative**. A Rust room server runs the existing
`simulation::Game` headless, one per room, at 60 Hz, fed each seat's `Intent`
(four directions, face, fire). Every third tick it sends a compact delta
snapshot plus the round's `Event`s. Clients keep a `Game` that is never
`update`d: they write the snapshot into it, tick only its cosmetics, and draw
it 100 ms in the past, interpolating between snapshots. Enemies, waves, props,
frogs and every hit stay on the server. That is **stage 1** and it ships a
complete co-op game. **Stage 2** adds client-side prediction of the player's
own tank on the same protocol: the client runs the shared `drive_tank` against
a small physics sandbox for every input the server has not yet acknowledged,
reconciles on each snapshot, and spawns its own shots provisionally. Transport
is WebSockets on every platform; rooms are created and joined through short
links on bongbong.io that play in the browser at once and open the native app
where it is installed; identity is a nickname and a device token, no accounts.
The first mode, and the only one this PRD designs, is co-op against the AI.

Deterministic lockstep, which the sim's seeded-replay design seems to invite,
was studied and rejected: it needs identical floats on emscripten, Apple,
Windows and Android (rapier's solver and about forty `sin`/`cos`/`atan2` call
sites do not give that), delay-based lockstep adds the round trip to steering,
and rollback needs a world clone with no serialisation in sight. Host-run peer
to peer needs signalling, STUN and TURN, which is a server, and the host's
phone locking ends the match for everyone.

## 2. Goals and non-goals

Goals
- Up to eight people, on any mix of web, macOS, Windows, Linux, iOS and
  Android, in one co-op round against the AI, on any map and mission the game
  has today, with the round's rules unchanged.
- A link in a chat message is a seat in the match: it plays in the browser
  with nothing installed, and opens the native app through a universal link
  where the app is there. No sign-up on the critical path.
- Own-tank steering that answers a key on the next frame (stage 2); everything
  else smooth at a fixed, known delay.
- Server upgrades that end nobody's round; crashes that lose at most a round;
  a link or a nickname that never stops working because a machine went away.
- Every claim about feel measurable in an offline rig with dialled-in delay,
  jitter and loss, before a real link is involved.

Non-goals, for now
- Free-for-all, team versus team, spectators, couch-plus-online: parked
  (section 9). The one rule kept for their sake is that `Owner::same_side`
  stays the single place deciding who may hurt whom.
- Accounts, friends lists, matchmaking, public room browsers, ranked anything.
- Datagram transports (WebTransport, WebRTC). The protocol separates droppable
  positions from reliable events from day one so the transport can change
  under it later.
- Cloudflare Durable Objects as the room host. Kept alive by building the sim
  crate for bare wasm in CI; not the first route.
- Lag compensation on the server (section 4.13); a decision for after stage 2
  measures.

## 3. What we have today (verified 2026-09-19, tree at 41fe857)

Working for us
- `simulation::Input` is two `Intent`s plus toggles. The part of an `Intent` a
  human can set is `move_dir: Option<Dir>`, `face: Option<Dir>`, `fire: bool`;
  its other two fields (`fire_aim_offset`, and `slow`, the throttle the enemy
  command layer eases) belong to the AI and never leave the server. Fire is
  "held"; `drive_player` detects the press edge per seat through
  `player_fire_held_last_frame`. The wire carries exactly today's input, two
  bytes per seat per tick.
- The dev server's `step` runs `Game::update(input, PHYSICS_FIXED_DT, w, h)` N
  times per rendered frame and is bit-for-bit replayable. The server tick loop
  is that loop.
- Headless play is proven and cheap, though not as cheap as first written. On
  one 2.1 GHz Xeon vCPU a tick of the default map costs about 95 µs with no
  enemies at all, plus about 11 µs per enemy and 13 µs per extra player:
  roughly 230 µs with twelve enemies and one player, 250 µs with two (`probe
  --scenario afk --spawn band --enemies 12 --rounds 20`, wall time over
  `frames_run`; a `Game::init` is 2.5 ms). Half of the fixed cost arrived
  with the flow-field router: the same sweep at f0cab22, before it, floors at
  40 µs, and on fixture maps identical in both trees the router adds 45 to 55
  µs a tick whatever the tank count. Section 5 has the projections and
  section 8 the mitigation.
- `drive_tank(&mut Physics, &mut Tank, Intent, dt, Footing)` is a free
  function: the whole locomotion model as one impulse per frame. `Footing` is
  what the ground under the hull says (docs/water.md): a ford's pace and grip
  and the current's velocity, read by `Footing::at` from `Game::water`, a
  `WaterLayout` built from the map's cells alone. The model is still a pure
  function of intent, the hull's own state and static ground, so it can run
  against a client-side sandbox unchanged, which is all stage 2's prediction
  needs from it.
- Effects are derived, not stored. Muzzle and impact flashes, blast fireballs,
  scorches, thrown rubble and shocks are pushed by the phases that cause them,
  shaped by a hash of their position; `fx.rs` diffs `game.events()`. A client
  rebuilds every visual from events plus positions.
- One seeded `SmallRng`, no `Instant` or `SystemTime` anywhere in the sim. A
  seed reproduces the ground, grass tufts and layout on a joining client.
- A presentation-only tick exists in miniature: while the mission banner
  shows, `update` runs only `tick_effects`.
- Two-player mode is half the seat model already: `Owner::Player(u8)`,
  `Game::players()`, one engage ring per player, per-player fire edges,
  `friendly_fire_damage_factor`, `Ai::target_player` over two players with
  hysteresis.
- The site is a Cloudflare Worker deployed by `wrangler` from CI on tags, with
  per-PR preview workers. The join page and the room directory are three
  routes on it.

In the way
- The sim links raylib. Seven simulation files import
  `sola_raylib::core::math::Vector2`, and every entity file (`tank.rs`,
  `frog.rs`, `shell.rs`, `obstacle.rs`, ...) keeps its `draw_*` next to its
  state. Nothing in `simulation/` needs a window, but the crate cannot build
  without the C library.
- No serializable world: `Game` is 63 fields around a `hecs::World`, neither
  derives serde, rapier is built without `serde-serialize`. Snapshots are a
  purpose-built wire struct.
- Ricochets are silent: `Shell`'s `Projectile::try_ricochet` reflects off iron
  and emits no event, and a barrel's chance bounce in `resolve_projectiles` is
  just as quiet; `Deflected` covers a shield's bounce only. Deriving
  projectiles from events needs a `Ricochet` event first (stage 1 sends
  projectile positions and does not care).
- Tuning is process-global (`tuning()` is one `RwLock`). A room's tuning diff
  travels in `Welcome` and the dev panel locks for the round.
- `Physics` has `set_position`, `velocity`, `apply_impulse`, `step`, but no
  `set_velocity` and no kinematic body constructor. Stage 2 needs both.
- The windowed loop passes a variable `get_frame_time()` to `update`; only the
  dev server steps at the fixed dt.
- `-sASYNCIFY=1` is on experimentally for the web build. The network bridge
  does not need it and it inflates the wasm on the invite's critical path.

## 4. Design, by area

### 4.1 Authority and the tick

- The server tick is the game's frame: a tokio interval at 60 Hz calls
  `Game::update(input, PHYSICS_FIXED_DT, w, h)` once per room per tick. The
  tick counter is `Game::frame`.
- Stage 1 samples inputs: each seat has a mailbox holding the newest `Intent`;
  the tick reads every mailbox into one `Input`. A seat whose intent has not
  arrived repeats its last one (a hiccup coasts rather than stops); a seat
  with nobody connected reads as no input, so its tank sits still and enemies
  still see it.
- Fire edges live in the sim. Because a press shorter than one packet interval
  could arrive as held-then-released inside one sample, the client sends its
  intent every tick and holds fire for at least two ticks.
- Every kind of authority stays where it is: hit tests, ram, blasts, fires,
  fuses, pickups, AI, waves, roll-ins, mission end, the players' own damage.
  None of this code changes for stage 1.
- The round's parameters are the room's: map TOML, seed, tuning diff, mission
  and spawn overrides, roster. Fixed at START, sent in `Welcome`, re-sent on
  rejoin. A rematch is a new seed under the same code.

### 4.2 What travels, what is derived, what is local

The replica does not need the server's memory; it needs enough to draw the
same picture. Every piece of `Game` and its entities sorts into three bins:
**travels** (snapshot or event), **derived** on the client from what
travelled, **local** cosmetic state the client ticks and the server never
sees. This table is the first pass; the phase 1 audit finalises it.

| Family | Travels | Derived | Local |
|---|---|---|---|
| Tank (50 fields) | `position`, `rotation`, body velocity, `damage`, `wreck_col`, `row` (in `Welcome`), `owner`, ammo counts, `flame_fuel`, active weapon and variants, `shield_hp` and `shield_broke`, `speed_boost_timer`, `burn_timer`, `despawn_timer`, a "hit this interval" bit, `flame_held` | `damage_variant`/stage from `damage`; `shell_variant` from row and the alternating shot; `hull_frame` from velocity and time; `wet_timer` from the hull's own wading (the water layout is the map's) | `visual_rotation`, `turret_visual_rotation`, `ring_position`/`ring_velocity`, `hull_anim_accum`, `minigun_cycle_timer`, track wobble and jitter, `track_accum`, `pending_shot` timing (the second barrel's shell arrives as its own `Fired`); `throttle` and `shield_recharge_delay` are server bookkeeping and never travel |
| Frog (two) | `position`, `health`, a state byte with phase, `hop_end` while hopping | animation frame | — |
| Obstacle (one per solid cell: 578 on the default 34 x 17 field, more on a map with its own `size`) | layout in `Welcome`; then deltas: `health`, `burning`, `fuse` armed with total, `scorched` mask, `destroyed`, a ram-lean byte | `variant`, `edge_mask` (recomputed as `refresh_edge_masks` does), `burn_frame` from time since ignition | `burn_frame_timer`, `heat` (ignition is an event) |
| Pickups | bitmask of slot-backed pickups present; the Health slot's bonus shields and frog packs as explicit cells | kind and position from the map's slots | bob and spin |
| Shells, bullets, plasma | stage 1: id, kind, position, heading, state per live projectile. Later, with `Ricochet`: spawn from `Fired`, removal from `Hit`, nothing between | choreography frame from state and interval | — |
| Laser, flamethrower | laser: `Fired` plus the hit point; flame: origin, direction, capped reach while held | beam fade, cone flicker | — |
| Fires, oil, drums | `FireStarted`/`Ignited`, burning cells with remaining time as deltas, `oil_cells` in `Welcome` and removals, `DrumLaunched` with landing cell | the drum's arc and shadow; fire loop frames | — |
| Round state | `mission`, spawn plan (`Welcome`), wave index/size/alive/pending, `intro_timer`, `time`, `outcome`, `restart_timer`, alert position and timer | banners, HUD | `paused` (meaningless online), `shadows_enabled`, `debug_overlays` |
| Fireballs, scorches, rubble, shocks, flashes, screen flash, cook-offs | only their causing events | everything: `wreck_fx`, `apply_blast`'s cosmetic half, `obstacle_died` hash it all from position | their ages (`tick_effects`) |
| Grass, tracks, ground, water | layout via map plus seed (the map carries the theme and the water cells); burnt cells as deltas | ground, water (`ground::Layout` for the picture, `WaterLayout` for the rules, both from the map's cells) and tufts from `Game::init`; tread marks from interpolated motion, wet ones after a ford | `crush`, `push`, sway, the water's shimmer and current marks (drawn from the round clock), spray, particles, camera shake |
| AI, the commander, engage rings, nav and route grids with their flow fields, terrain snapshot | nothing | nothing | nothing; the client has no AI |

Two rules fall out. Cosmetic state is owned by the replica and driven from
interpolated authoritative values, never sent. Every cause must be an event,
because the client sees positions, not reasons; `Ricochet` is the known gap
and the audit will find a few more (a shell expiring at the field edge, a fuse
lit by a burning cell, a wreck fading).

### 4.3 Wire protocol

```
// client -> server, every tick. Stage 1 samples the newest; stage 2 consumes in order.
Intent   { tick: u32, move_dir: u8 /* 0 none, 1-4 */, face: u8, fire: bool }

// server -> client, every 3rd tick (20 Hz), delta against the previous snapshot
Snapshot { tick: u32, server_ms: u32,
           acked:  [u32; MAX_SEATS],   // last input tick applied per seat (stage 2)
           tanks:  [{ id: u8, x: i16, y: i16, vx: i8, vy: i8, dir: u8, hp: u8,
                      flags: u8 /* wreck|shield|boost|burning|hit|flame */, weapon: u8, ammo: u8 }],
           shots:  [{ id: u16, kind: u8, x: i16, y: i16, heading: u8, state: u8 }],
           frogs:  [{ side: u8, x: i16, y: i16, hp: u8, state: u8, phase: u8 }],
           pickups: u64, bonus_shields: [u16],
           tiles:  [{ cell: u16, hp: u8, flags: u8, faces: u8 }],
           fires:  [{ cell: u16, left: u8 }],
           round:  { wave: u8, alive: u8, pending: u8, intro: u8, outcome: u8 },
           events: [Event] }              // AI trace variants filtered out

// once, on join and on every rejoin
Welcome  { protocol: u16, sim_version: &str, seat: u8, roster: [{ seat, nick, chassis }],
           map_toml, seed: u64, tuning_json, mission, spawn, oil_cells,
           snapshot: Snapshot /* full */ }

// lobby, JSON over the same socket
Lobby    { Join { nick, device_token, code } | Ready | Start | Leave | Kick { seat } | Chat { text } }
```

Positions in quarter pixels fit an `i16` on fields up to 8192 px wide. Encode
with `postcard`; JSON for the lobby. Events and tile deltas are reliable by
construction on a WebSocket and sit in their own part of the packet so a
datagram transport can later drop positions and keep the rest.

Compression, in the order it pays, all but the last built in phase 1: quantise
and varint (about 300 B for an eight-tank snapshot); delta against the
previous snapshot (the baseline is simply the last snapshot sent, since a
WebSocket loses nothing; about 80 to 120 B); projectiles as events once
`Ricochet` exists (60 to 90 B); deflate the one-off `Welcome` (about 20 KB to
4 KB). Stream compression (`permessage-deflate`) only if a real link shows it
is needed. Never compress the intent stream.

### 4.4 Events and the client's reactions

`Event` as it exists, and what the replica does with each. `AiAction`,
`EngageSlot`, `StuckEscape`, `Breach`, `Retreat`, `Alert`, `Retarget` are
never sent (the AI's trace, recorded only while `Game::trace_ai` is set);
`PhysicsQuarantine` is logged server-side.

| Event | Client reaction | Existing code it calls |
|---|---|---|
| `RoundStarted` | arrives in `Welcome`; re-init the replica from map and seed; banner | `Game::init` (presentation parts), `Mission::banner` |
| `Fired {slot, weapon}` | muzzle flash; laser beam; stage 2: confirms a predicted shot | `weapons.rs`' flash push, `LaserBeam::new` |
| `Hit {target, damage, killed, x, y}` | impact flash; hit flash on the tank | `impact_flashes.push` |
| `Wreck {slot, x, y}` | fireball, flash, scorch, thrown parts, cook-off queue, screen flash, ripple, shake | `Game::wreck_fx`, `flash_screen`, `SHOCK_KILL` |
| `Ram` | contact sparks and dust | `fx.rs` |
| `PickupCollected` / `PickupRespawned` | pop / appear | `fx.rs` |
| `Deflected` (a shield turned a shot, which changes owner), `ShellsCollided` | spark, small flash | `impact_flashes.push` |
| `FrogBite` | bite cue, frog ripple | `SHOCK_FROG` |
| `FrogHealed {side, slot, amount, x, y}` | green sparks off the frog, its gauge refills | `fx.rs` |
| `ShieldBroken {slot, x, y}` | a ring of sparks and smoke off the hull, ripple and shake, the shield ring goes | `fx.rs`, `drain_shield_breaks`' `SHOCK_SHIELD_BREAK` |
| `WaveStarted`, `TankEntered`, `WreckRemoved` | WAVE N banner; roll-in appears at the gate; a wreck fades | HUD, `despawn_timer` |
| `ObstacleDestroyed {material, x, y}` | rubble decal, thrown | `props::obstacle_died`'s decal part |
| `Blast {x, y, chained, drum}`, `CookOff` | shaped fireball, scorch, parts, re-thrown rubble, flattened grass, burnt-in tracks, ripple, flash | `apply_blast`'s cosmetic half |
| `DrumLaunched` | the flying drum from launch to landing | `draw_flying_drum` from a launch time |
| `FireStarted`, `Ignited` | light the cell or tile | `light_cell`'s visual part |
| `RoundEnded {outcome}` | end screen, results, rematch | the end screen |
| new `Ricochet {slot, x, y, heading}` | the shell's heading changes | `Shell::reflect_off` |

The refactor this implies: the cosmetic half of `wreck_fx`, `apply_blast` and
`obstacle_died` becomes a function callable from an event, and the server
calls the same function, so the two never drift.

### 4.5 The client replica

A `Game` used as a presentation model, since `Game::render` reads a `&Game`.
Four pieces:

- `apply_snapshot(&mut Game, &Snapshot)`: writes the authoritative bins into
  the entities, matched by id (seat or enemy slot for tanks, projectile id for
  shots). A tank in the snapshot but not the world is spawned; a world tank
  missing from the snapshot is removed. Enemies get a `Tank` and no `Ai`, like
  a rolling-in wave tank, so every `.with::<&Ai>()` query excludes them for
  free.
- Interpolation: the replica keeps the last few snapshots. Render time is
  server time minus the interpolation delay (100 ms fixed at first, adaptive
  later: two intervals plus a jitter margin, clamped 60 to 200 ms). Positions
  blend linearly; hulls snap by the four-way rule. A gap extrapolates on the
  last velocity for at most two intervals, then holds. Server time comes from
  `server_ms` through a smoothed offset.
- `Game::tick_presentation(dt)`: `tick_effects` plus the cosmetic parts of the
  entity ticks (`ease_visual_rotation`, `ease_turret_visual_rotation`,
  `ease_ring_position`, hull animation, tread marks from displacement - none
  in water, wet for `wet_timer` after a ford -, grass crush and push from hull
  boxes, fire and fuse frames, decal flight, the drum's arc, the wave banner
  timer), and the round clock the water's shimmer and current marks and the
  fire loops are drawn from: `time` travels, the replica advances it between
  snapshots and re-pins it on each. `Game::wading` (every hull in a ford,
  what `fx.rs` throws spray from) is a position query against the map's water
  and needs nothing sent. All of it exists inside phases that also do
  authoritative work; the refactor separates the halves.
- Effects from events, applied at their tick's render time, not on arrival, so
  a fireball appears when the hull it belongs to is drawn there. `Fx::observe`
  then sees the same `game.events()` it sees today.

Join in progress and rejoin are one path: `Welcome` carries a full snapshot. A
late joiner misses earlier cosmetics (scorches, rubble); if it matters,
`Welcome` can carry the decal and scorch lists, a few hundred bytes of
positions plus hashed variants.

### 4.6 Transport

WebSockets everywhere in v1: the only transport all five platforms speak
without a native stack, passes every proxy, TLS is free with the domain. Its
weakness, TCP head-of-line blocking on a lossy link, costs a stutter at 20
packets a second, not a disconnect.

- Web (emscripten): a small `net.js` added with `--js-library` owns the
  browser `WebSocket`. `onmessage` copies the bytes into wasm memory through
  `Module.ccall("bb_net_push", ...)`; Rust drains the queue at the frame
  boundary, the discipline `tuning::apply_pending` and the dev server follow.
  Outgoing frames call an `extern "C" bb_js_send(ptr, len)` implemented in
  that library; the handler sends a copy of the heap slice (the heap detaches
  on growth under `ALLOW_MEMORY_GROWTH`). Single-threaded JS is not a problem:
  the frame is a requestAnimationFrame task and socket events dispatch between
  frames; nothing ever blocks. Handle `visibilitychange` (a hidden tab stops
  rAF while `onmessage` keeps firing) by draining and keeping the newest
  snapshot. No wasm threads, no Worker, no Asyncify.
- Native (desktop, iOS, Android): `tokio-tungstenite` with `rustls` and
  `webpki-roots`, on a thread behind an mpsc channel, the dev server's
  pattern.
- One `Transport` trait, `send(&[u8])` and `drain(&mut Vec<Msg>)`, with three
  implementations: the JS bridge, the native thread, and an in-process channel
  with artificial delay, jitter and loss for the rig.

The datagram upgrade, later: WebTransport (native via `quinn`/`wtransport`;
browser support only recently reached Safari, keep the WebSocket fallback) or
WebRTC data channels (every browser, but drags an ICE/DTLS/SCTP stack into the
server). Confined to the transport layer by the protocol's split.

### 4.7 Room server

`bongbong-server`, one binary: axum for HTTP (health, metrics, the WebSocket
upgrade), tokio for the rest.

- One task per room owns the `Game`. Each tick it drains the seats' mailboxes
  into an `Input`, calls `update`, appends the intents to the replay log, and
  every third tick encodes the delta snapshot once and hands the same bytes to
  every seat's writer.
- One task per connection reads, decodes, and drops intents into the seat's
  mailbox; lobby messages go to the room task over a command channel. The
  writer has a bounded queue; a slow client gets snapshots skipped, never
  queued.
- Seats outlive connections: nickname, device token, chassis, connection
  state, last acked tick. A disconnect starts a 30 s grace; a reconnect with
  the same token reclaims the seat and gets a fresh `Welcome`; past the grace
  the seat is away but still owned for the whole round.
- The directory client registers the instance at start, heartbeats, reports
  room creation and closure.
- The dev server rides along: every room can expose the existing tools on a
  loopback port behind `dev-tools`, so a misbehaving live round is inspectable
  with the MCP tools used today.
- Bounds everywhere: rooms per instance, waiting rooms (30 min), empty rooms
  (5 min), rounds (30 min).

### 4.8 Hosting

| Option | Fits because | Hurts because | Cost |
|---|---|---|---|
| **Native server on a small VM** (Fly.io machines, or one Hetzner box) — recommended | same crate as the probe and tests; `cargo run` debugging; real 60 Hz timers; native TLS and QUIC later; dozens of rooms per vCPU (section 5) | a machine to keep alive; regions are yours (Frankfurt first) | ~$3–5/month per region |
| Cloudflare Durable Object per room (`workers-rs`) | zero machines; next to the site; placed near the room's creator | only after the sim compiles to bare wasm; a 60 Hz timer in a DO is workable, not a documented target; `wrangler dev` debugging; no datagrams ever | Workers Paid plus usage |
| Split: Worker for the lobby, VM for rooms | each piece where it is easiest; both deploy lanes exist | two lanes | as the VM |

The split is the VM option written out and is what the design draws: the
Worker gains `POST /rooms`, `GET /j/:code`, the directory (KV or one DO) and
the well-known files; the room server is the only new machine.

### 4.9 Persistence and upgrades

Three kinds of data:

| Kind | Examples | Where | Lifetime |
|---|---|---|---|
| the live world | the `Game`, hecs, rapier, timers, the AI's memory | server memory only | one round; never on disk |
| the replay log | map, seed, tuning diff, mode, then every tick's intents (~3 KB/s per room) | appended in memory, flushed to local disk every few seconds | the round plus a day |
| records | room: code, host, map, mission, roster, results, owning instance; player: nickname, device token, later stats; shared custom maps | the directory beside the Worker, or SQLite beside the server | rooms a day, players indefinitely |

The room's life: **waiting** (not ticking; reaped 30 min after the last seat
leaves) → **playing** (60 Hz) → **paused, nobody connected** (not ticking: an
empty room must not burn CPU and the AI must not kill the frog with nobody
watching; the world stays in memory 5 min, resumes where it stopped on
reconnect) → **ended** (world dropped at once; the record and results stay a
day for rematch and a results link).

Recovery: a crash or same-binary restart reads the replay logs and
fast-forwards each room to its last tick (ten minutes of play is 36 000 ticks,
under ten seconds at section 5's tick cost); clients reconnect and resume. A
CI test replays a logged round and compares the final snapshot. A new build
cannot replay the old build's log, which is why upgrades are a routing
problem:

- Instances are addressable. Each server process registers `{address, version,
  protocol, region}` in the directory on start with a heartbeat; room records
  carry the owning instance. One field, built in from phase 2.
- Deploy is start new, drain old. The new instance passes a health check,
  registers, and becomes the only member of the new-rooms pool. The old one
  gets `SIGTERM`, leaves the pool, refuses new rooms and rematches, keeps
  ticking its rooms, and exits when the last ends or after 15 min. On Fly.io:
  a machine started beside the old one plus a delayed stop. On one box: a
  supervisor and two ports.
- Joins follow the room: `GET /j/CODE` resolves to the owner, whichever
  version. Rollback is the same move backwards.
- Versions: a **protocol** number gates joining (web: "reload the page"; app:
  "update, or play this round in the browser", which the invite page already
  offers); a **sim** version is stamped on logs and records. Ship the web
  build and the server from the same tag in the existing release workflow.
- Tuning is a build, not a hot reload: the native `--tuning` file re-read
  would alter every live round on a global table. Captured per room at
  creation, sent in `Welcome`, changed by deploying.
- Migrations are additive; records carry a schema version. Watch per instance:
  rooms, seats, tick p50/p99, snapshot bytes/s, reconnects/min, version; tick
  time nearing 16 ms is the one capacity alarm.

### 4.10 Invites and identity

Whoever receives a link is in the match within one tap with nothing installed:
the browser build is the universal fallback, the apps the upgrade.

- Host presses ONLINE → HOST; `POST /rooms` returns a four-character code from
  a 20-letter alphabet without vowels or look-alikes (160 000 codes; idle
  rooms are reaped, collisions are a non-issue) and the owning server. The
  link `bongbong.io/j/K7QX` goes to the share sheet; a QR and the code stay on
  the host's screen while the room is open.
- The join page checks the room is open and the version, then: browser, the
  wasm build with the code in the URL, read at start through
  `emscripten_run_script` like `bbShift`; iOS, universal links
  (`apple-app-site-association` under `/.well-known/` on the Worker,
  associated-domains entitlement), `bongbong://j/K7QX` as the button fallback;
  Android, app links (`assetlinks.json`, an `autoVerify` intent filter in the
  manifest template, the code from the activity's intent data); desktop, type
  the code (the release archives register no scheme; an installer can, later).
- Identity: a nickname and a random device token minted on the join page and
  reused across visits (localStorage, the app's keychain). The token is the
  reconnect key. Accounts can attach to it later without touching the
  protocol.
- Abuse: Turnstile on the web creation path, a per-IP rate limit at the
  Worker, a nickname length cap and filter before anything public exists.
- The lobby is a third mode next to Play and Build: seats with nicknames and
  chassis, the host picks map and mission (a builder map travels in
  `Welcome`), READY, the code and QR in a corner.

### 4.11 Co-op specifics

The two-player round with the seats spread across machines.
`Owner::Player(u8)` versus `Owner::Enemy(slot)` is the co-op split;
`same_side` already keeps players from hurting each other at full damage; the
end rule from docs/two-players.md (frog dead or every player wrecked) is the
co-op end rule.

| Piece | Today | For online co-op |
|---|---|---|
| Seats | `PlayerCount::One\|Two`, two intents in `Input` | a roster of up to `MAX_SEATS` (8), one intent per seat; a seat with no human sends no input |
| Spawns | `start`/`start2`; player 2 beside player 1 when unset | "beside the start" for N with `Game::init`'s clearance and `dry_cell_near`'s shore rule; numbered starts in the builder later |
| Enemy targeting | `Ai::target_player` over two, with hysteresis | the same rule over N; the engage ring is already per player |
| Colours, labels | `TEAM_COLORS` two entries; sheet blocks enemy, P1, P2; `P1`/`P2` labels | seats past two draw the P1 block, told apart by ring colour and `P3`..`P8`; proper blocks from `gen_tanks.py` when a fourth friend shows up |
| HUD | `SLOTS_ONE`/`SLOTS_TWO` | online: the local seat in full, others as a strip of ring-coloured hearts; couch tables stay |
| Death | a wrecked player waits for the round to end | wave rounds: re-enter with the next wave through a gate (the roll-in that exists), no penalty in v1; band rounds keep today's rule (decision 7) |
| Frog, pickups | unchanged | unchanged; the server decides who reached a pickup first, and a frog health pack heals the collector's side's frog wherever it stands (`FrogHealed`) |

Difficulty scales with seats: a wave plan authored for one tank is a walk for
four. The room can scale `tanks`, wave size and tier ramp by seat count with
one tuning diff; the probe's `--players` sweeps are the tool for tuning the
curve.

### 4.12 Stage 2: prediction

Stage 1 shows your own tank 100 ms plus half a round trip after the key; stage
2 shows it on the next frame. Rewind and replay, the Quake 3 technique, with
an easy time here because locomotion is a pure function of intent and a mostly
static world.

Predicted: own hull position, rotation, velocity; own shots (flash and shell
on the frame of the press, confirmed by `Fired`; cooldown and ammo from the
last snapshot plus local decrements); collisions with static terrain and
water (both exact: deep cells are static colliders, and a ford's `Footing` is
a function of the hull's position on the map's `WaterLayout`).
Approximate: collisions with other tanks and wrecks (kinematic stand-ins at
interpolated positions; the server's answer wins). Not predicted: damage,
knockback, ram, pickups, buffs; everyone else stays interpolated.

Protocol changes:
- `Intent.tick` becomes meaningful: the client stamps each intent with its own
  tick, running at the server's rate and ahead by the client's lead, and sends
  the last three intents per packet.
- The server consumes each seat's intents in order, one per tick, from a small
  jitter buffer. Empty at a tick: repeat the last and count a starvation; the
  client widens its lead on starvation and narrows it when the buffer runs
  deep.
- `Snapshot.acked[seat]` carries the last input tick applied per seat.
- `Event::Fired` gains `seat` and `client_tick` for player shots.

The sandbox and the loop. The client owns a second `Physics`: the same statics
`Game::init` spawns for walls, obstacles, deep water and the frog,
`spawn_tank` for its own tank, a kinematic body per other live tank re-placed
every frame. It keeps the last 120 intents by tick. Per frame: sample input,
stamp, append, send; `drive_tank(sandbox, own, intent, dt, Footing::at(water,
own.position))` and `sandbox.step()`; render the predicted tank for the local
seat. On each snapshot: reset the sandbox tank to
the server's position, rotation and velocity at `acked` (the new
`set_velocity`); replay every intent after `acked` (typically 5 to 15,
microseconds each); compare with the pre-snapshot prediction. Within a quarter
pixel: nothing. Otherwise the difference is a visual offset on the drawn hull
decayed to zero over about 100 ms; over a hull width, snap. The replay lands
because it runs the same tick count, `dt`, tuning, `drive_tank` and rapier on
the same static shapes and the same `WaterLayout`; cross-platform drift is
sub-pixel per step and corrected every 50 ms, a nudge rather than a desync.

Provisional shots: id `(seat, client_tick)`; adopts the server's projectile id
when `Fired` arrives; removed quietly after one round trip plus one interval
without confirmation. Full-auto weapons predict the visual stream and let the
server's events drive hits.

Under prediction your hull is drawn at the present while others are 100 ms in
the past. Ramming a friend looks like arriving a beat early and being settled
back; an enemy's hit is judged against where you were on the server. With
hulls 64 px wide and shells slow, rare. Lag compensation (rewind targets to
the shooter's view, capped at 200 ms, using a per-room position ring like the
dev server's history) is optional and mostly for the laser (decision 9).

Sim additions for stage 2: `Physics::set_velocity` and `spawn_kinematic`;
statics buildable from a map on a bare world
(`battlefield::spawn_walls`/`spawn_from_map` and the deep-water boxes callable
outside `Game::init`; `WaterLayout::build` already takes only cells);
`drive_tank` and `Footing::at` reachable from the client module; a `Tank`
constructor for the local seat from the roster.

Measured in the rig before any real link: prediction error per snapshot (mass
under a quarter pixel, a tail at contacts), corrections per minute above nudge
and snap thresholds, lead and starvation per seat.

## 5. Numbers (order of magnitude)

| Item | Value |
|---|---|
| Intent up, 60 Hz (three per packet in stage 2) | 0.4–0.8 KB/s payload, ~3 KB/s framed |
| Snapshot down, 20 Hz, 8 tanks + 24 shots, delta | ~2–3 KB/s payload, ~4 KB/s framed |
| Worst case, ~40 tanks in a wave round (`wave_max_alive`'s cap of 31 plus eight seats) | ~10 KB/s payload |
| `Welcome`, deflated | ~4 KB once |
| Server tick, 12 enemies + 1 player (measured, section 3) | ≈ 230 µs on a 2.1 GHz Xeon vCPU, ≈ 95 µs of it per-frame fixed cost |
| Server tick, 12 enemies + 8 seats (projected) | ≈ 320 µs; ≈ 550 µs at `wave_max_alive` |
| CPU per room at 60 Hz | ≈ 2 % of a core at 8 seats, ≈ 3 % worst case; 30–50 rooms per vCPU of that class, more on a desktop core |
| Memory per room | < 2 MB |
| Client replay per snapshot, stage 2 | < 0.2 ms |
| Own-tank latency, stage 1 | ≈ 150–250 ms (≤16 sample + 20–60 up + ≤16 tick + ≤50 cadence + 20–60 down + 100 interpolation) |
| Own-tank latency, stage 2 | the next frame |

## 6. Phased plan

Sizes: S days, M a week or two, L several weeks. Phases 0 to 3 are stage 1 and
ship a complete co-op game; 4 and 5 are stage 2.

0. **Headless simulation crate (M).** Replace the seven `sola_raylib::Vector2`
   imports with a local `math::Vec2`; move each entity file's `draw_*` and
   `*Textures` behind a `render` feature or into `*_draw.rs` siblings;
   generalise `Input` to N seats; a fixed 60 Hz accumulator in `app.rs`. Done
   when `probe` and `cargo test --lib` build with `--no-default-features` and
   no C compiler, and `just probe-fixtures` is byte-identical. Bonus: CI
   builds the crate for `wasm32-unknown-unknown`.
1. **Replica and protocol, offline (L).** The state audit made real:
   `Snapshot`, `Welcome`, the intent packet, delta encoding;
   `encode_snapshot`/`apply_snapshot`; `tick_presentation`; the cosmetic
   halves of `wreck_fx`, `apply_blast`, `obstacle_died` callable from events;
   `Ricochet` (iron and the barrel bounce) and whatever else the audit finds
   silent. The rig: an authoritative `Game` on a thread, a replica in the
   window, an in-process transport with dialled delay, jitter and loss. Done
   when the round looks and feels like the local game at 80 ms and 2 % loss
   with every effect present; a snapshot round-trip test next to
   `determinism_tests`.
2. **Room server, WebSockets, join links (L).** `bongbong-server`; the JS
   bridge and tungstenite behind the `Transport` trait; the Worker's routes
   and directory with the owning instance on each record; the well-known
   files; the ONLINE dialog, the lobby, the code and QR; one region with
   start-new-drain-old. Done when a link from a phone puts a laptop in the
   same wave round, a closed tab rejoins the same seat, and a deploy mid-round
   ends nobody's game.
3. **Co-op polish (M).** Seats beyond two, the compact HUD, N-seat spawns,
   re-entry with the next wave, end screen and rematch, host hand-over,
   builder maps in rooms, clock sync and adaptive interpolation delay. Done
   when four friends on four platforms finish a wave round, one drops and
   rejoins, and they rematch without a new link. **Stage 1 ships.**
4. **Prediction: sandbox and reconciliation (L).** `set_velocity` and
   kinematic stand-ins; statics from a map on a bare world; input history and
   the client tick with adaptive lead; the server's ordered buffer and
   `acked`; rewind and replay; the visual correction with nudge and snap
   thresholds; the metrics. In the rig first. Done when at 120 ms RTT with
   jitter and loss the tank answers on the next frame, the error histogram
   sits under a quarter pixel away from contacts, and a wall stop matches the
   server's to the pixel.
5. **Prediction: shots, contacts, maybe lag compensation (M).** Provisional
   shells; predicted cooldown and ammo; full-auto visual streams; stand-ins
   and ram behaviour; the rewind ring and lag-compensated hit test if decision
   9 says yes. Feel pass on real links. **Stage 2 ships.**
6. **Horizon.** Projectiles as events; datagram transport; more regions;
   accounts and friends on the device token; public rooms; replays from the
   log; a desktop installer with the URL scheme; the parked modes.

## 7. Decisions and recommendations

1. Room host: native Rust server on a VM, or Cloudflare Durable Objects?
   **VM**, Worker for links and directory; keep the DO route alive with a
   bare-wasm CI build.
2. Transport v1: WebSockets, or WebRTC data channels from day one?
   **WebSockets**; revisit only if lossy mobile links stutter, then
   WebTransport.
3. Identity: nickname plus device token, or accounts? **No accounts in v1.**
4. Room size: up to 8 seats on the shared 34×17 field? **8**, 60 Hz tick, 20
   Hz snapshots; expect four to be common.
5. Desktop deep links: type the code, or an installer registering
   `bongbong://`? **Type the code.**
6. Crate split: a `bongbong-sim` workspace crate, or a `render` feature?
   **Feature first**; promote when the server wants a slimmer graph.
7. A wrecked player in a wave round: watch, or re-enter with the next wave?
   **Re-enter through a gate, no penalty**; band rounds keep today's rule.
8. Interpolation delay: fixed 100 ms, or adaptive? **Fixed in phases 1–2,
   adaptive in 3.**
9. Lag compensation: rewind to the shooter's view, or judge at server time?
   **Server time through stage 2, then measure.** Only the laser will
   complain, and in co-op the "shot behind cover" complaint has no victim with
   a grudge.
10. Other tanks in the sandbox: kinematic stand-ins, or ignore? **Stand-ins**;
    without them a predicted tank drives through a friend for a round trip and
    snaps back.

## 8. Risks, mitigations, stop conditions

- **The state audit is the real work of stage 1.** Fields that are
  authoritative-but-invisible (a timer gating a visual) surface by playing the
  rig with an offline diff on. Phase 1 is L, not M.
- **The split touches twenty files.** Mechanical, not hard; the fixture sweep
  is the oracle. Stop condition: if `just probe-fixtures` moves during phase
  0, the split leaked a behaviour change and must be found before anything is
  built on it.
- **Reconciliation against dynamic bodies.** Exact against statics,
  approximate against tanks and wrecks; rams produce nudges. Smoothing and the
  snap threshold are knobs tuned in the rig; enemy separation already keeps
  pile-ups rare.
- **Float drift under prediction.** Sub-pixel per step, corrected every 50 ms.
  If a platform shows systematic drift (a different `exp` in the decel curve),
  route the few transcendentals in `drive_tank` through `libm`, locally,
  without lockstep-grade effort.
- **Tuning agreement.** Online rounds apply the `Welcome` diff and lock the
  panel; the sandbox reads the same table; a mismatch shows as systematic
  prediction error in the metrics.
- **Upgrades depend on the directory.** Skipping the owning-instance field in
  phase 2 turns every deploy into a drain with a gap for new rooms. Build it
  in.
- **Web build.** Retire `-sASYNCIFY=1` before phase 2; the bridge does not
  need it and it inflates the wasm on the invite's critical path.
- **Mobile lifecycle.** Neither iOS nor Android handles backgrounding yet; the
  grace covers the seat, the app must resume the socket and clock sync
  cleanly.
- **Co-op difficulty scales with seats.** Scale the wave plan by seat count
  with a tuning diff; play-test the curve with the probe's sweeps.
- **The tick's fixed cost bounds rooms per core.** About 95 µs of every tick
  is work that does not scale with tanks (section 3), and half of it is
  `route_grid`: the nav grid rebuilt from every obstacle, labelled, priced,
  and one Dijkstra per shared target - a field per seat plus one for the
  frog, so eight seats are nine fields a tick. Watch tick p50 per room; if
  it matters, build the route grid every other tick or only when a target
  changed cell, both server-side changes no client sees, with the probe
  fixtures as the oracle that the AI did not change.
- **Abuse of open rooms.** Rate limits, Turnstile, nickname filter.

## 9. Parked: future modes

Recorded so co-op does not close the door on them; none are in the plan.

- Free-for-all: a `Team` on `Owner` so `same_side` can say "different humans,
  full damage"; `Mission::Deathmatch` with kills or time; respawn; a
  scoreboard.
- Team versus team: two teams, optionally a frog each; bots by widening
  `Ai::target_player` to "nearest tank not on my team"; team colours three and
  four.
- Spectators: a seat kind that receives and never sends; the replica is
  already a viewer.
- Couch plus online: two players on one keyboard in two seats of an online
  room. The protocol lets a client own more than one seat from day one.

## 10. References

- Bongbong Online, the design page (rev 2, 2026-09-15): option comparison,
  diagrams, the stage 2 timeline.
- docs/two-players.md: the seat model this generalises.
- docs/dev-server-design.md: the lockstep `step`, the frame-boundary
  discipline, the history ring.
- docs/gameplay-verification-design.md: seeded replay, the probe, the fixtures
  that guard phase 0.
- docs/physics-engine-design.md: why rapier is not cross-platform
  deterministic and why that only matters for lockstep.
- docs/fullscreen-resolution-research.md: the shared field, the window, and
  touch, which the invite page inherits.
- docs/runtime-tuning-design.md: the tuning table the `Welcome` diff patches.
- docs/water.md: the `Footing` and the deep-water colliders the stage 2
  sandbox carries; the shimmer and spray the replica draws for itself.
- docs/enemy-command-and-control-prd.md: the commander, the one layer that
  reads several enemies at once; all of it stays on the server.

