# PRD: bongbong online co-op

Written 2026-09-15, re-checked against the tree on 2026-09-19 (section 3 names
the commit), on 2026-09-26 at v0.2.0, when stage 1 had shipped and stage 2
was half built (section 3, "Where it stands"), and on 2026-09-28 after the
first real-link play-tests came back bad (section 3, "What the field said",
and section 4.14, stage 3). Same shape as
docs/android-port-prd.md: the decision, what exists today, the design by area,
the phases, the risks. The design study behind it, with diagrams, the
measurements and the open checklist, is the "Bongbong online co-op" page
(2026-09-19, superseding "Bongbong Online" rev 2); this document is the
version that lives with the code and is the one to update as things land -
where a section says how something is *built*, that wording is the tree's,
and where it still says how something *will* work, it has not landed. Four
requirements added on 2026-09-19 are folded in: the server ships as a
container on the Hetzner Kubernetes cluster (4.8), version 1 keeps nothing on
disk (4.9), the client runtime was weighed against tokio (4.6), and local play
stays untouched (4.5).

## 1. The decision, in one paragraph

Multiplayer is **server-authoritative**. A Rust room server runs the existing
`simulation::Game` headless, one per room, at 60 Hz, fed each seat's `Intent`
(four directions, face, fire). Every tick it sends a compact delta snapshot
plus the round's `Event`s. Clients keep a `Game` that is never `update`d: they
write the snapshot into it, tick only its cosmetics, and draw it a little in
the past - `online_interpolation_delay_ms`, 33 ms on a clean link, widened by
the link's own jitter - interpolating between snapshots. Enemies, waves, props,
frogs and every hit stay on the server. That is **stage 1** and it ships a
complete co-op game. **Stage 2** adds client-side prediction of the player's
own tank on the same protocol: the client runs the shared `drive_tank` against
a small physics sandbox for every input the server has not yet acknowledged,
reconciles on each snapshot, and spawns its own shots provisionally. Transport
is WebSockets on every platform; rooms are created and joined through short
links on bongbong.io that play in the browser at once and open the native app
where it is installed; identity is a nickname and a device token, no accounts.
The room server is one container image on the Hetzner Kubernetes cluster, run
as **a single instance**, and version 1 keeps nothing on disk: every room
lives in that one process's memory and a code names a room and nothing else.
Spreading rooms over several instances is deferred work (4.8). Local play is untouched: the online round is a third driver
beside Play and Build. The first mode, and the only one this PRD designs, is
co-op against the AI.

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
- A planned deploy that ends nobody's round: the server drains its rounds
  before it exits. A crash ends every live round in v1, the price of keeping
  nothing on disk, until the replay log arrives.
- Local play untouched: the single-player and couch two-player rounds, the
  builder, the probe and the dev server keep working exactly as today, in the
  same binary.
- Every claim about feel measurable in an offline rig with dialled-in delay,
  jitter and loss, before a real link is involved.

Non-goals, for now
- Persistence of any kind in v1: no replay log, no records, no database. A
  room is memory in one process (4.9).
- More than one server instance: one is what the traffic needs and one is what
  ships. The routing a second instance would want - which is the only thing
  that would put a server's identity back into a room code or a URL - is
  designed in 4.8 and built when the capacity asks for it.
- Free-for-all, team versus team, spectators, couch-plus-online: parked
  (section 9). The one rule kept for their sake is that `Owner::same_side`
  stays the single place deciding who may hurt whom.
- Accounts, friends lists, matchmaking, public room browsers, ranked anything.
- Datagram transports (WebTransport, WebRTC). The protocol separates droppable
  positions from reliable events from day one so the transport can change
  under it later.
- Cloudflare Durable Objects as the room host. Kept alive by building the sim
  crate for bare wasm in CI; not the first route.
- Lag compensation on the server (section 4.12); a decision for after stage 2
  measures.

## 3. What we have today

### Where it stands (2026-09-26, v0.2.0, `feature/coop-improvements`)

Stage 1 shipped: the headless crate, the replica and the wire, the room server
in its container with a server per PR, the lobby with codes, QR and invite
links, seats to eight with re-entry and seat-scaled waves. Phase 4 of stage 2
is in - the sandbox, reconciliation with nudge and snap, the ordered mailbox
and `acked` - and phase 5 has its first item, the provisional shell. Section
4.12 says which of its pieces are built and which are not; section 6 marks
the phases. The five things the plan still owed on that date, and what this
revision does about each:

| Owed | Status |
|---|---|
| adaptive interpolation delay (decision 8) | built in this revision: the floor is the knob, the link's jitter widens it (4.5) |
| the client's lead over the room (4.12) | built in this revision as a *depth*, not a stamp: the snapshot reports each seat's mailbox and the client adds or skips a packet (4.12) |
| the prediction metrics (4.12, phase 4's exit test) | built in this revision: `status.round.prediction` on the dev server, off the same counters the rig tests read |
| predicted cooldown and ammo | built in this revision: the sandbox is a projection of the server's seat, so its weapon and ammo gate the provisional shot, and the cooldown is seeded from the room's own `Fired` |
| full-auto streams | built in this revision for the minigun's burst and the twin barrel's second shot; plasma is a shell here. The laser, the missile pod and the flamethrower's cone are drawn from the room's word (4.12 says why) |
| lag compensation (decision 9) | deferred in this revision; built since, in 4.16 (`IntentMsg::view_tick`, `HitBoxHistory`, `sweep_rewound`), with 4.12's instrument (`crossings` against `crossings_hit`/`crossings_missed` on `status.round.prediction`) reading whether its rewind is right |

### What the field said (2026-09-28, v0.2.3, `feature/coop-ng-2`)

The first rounds played over real links, from phones and browsers against
production, came back with three complaints: lasers are not visible in
co-op, shooting is delayed, and the whole thing is slow. The branch
`feature/coop-ng-2` is the answer. What was found, in the order it matters:

1. **Neither end of the socket set `TCP_NODELAY`.** The room sends a small
   frame sixty times a second and hears a ten-byte intent back as often;
   with Nagle's algorithm on, the kernel holds every small segment until
   the previous one is acknowledged, so on any link with a real round trip
   both streams left in bursts *once per round trip*. Intents arrived in
   clumps and starved the ticks between (repeated last intent, then a
   correction on every snapshot as the sandbox fell behind the hull the
   room had moved on); snapshots arrived in clumps, read as jitter, and the
   adaptive delay widened toward its cap. That is the "delayed" and the
   "slow" together, and nothing in the tree could see it: the rig is a
   loopback link and the server tests run on `127.0.0.1`, where a round
   trip is fifty microseconds. Fixed on both ends (decision 16) -
   `axum::serve` does not set it, `tungstenite::connect` does not set it,
   a browser's WebSocket already does.
2. **The laser beam never travelled.** An instant hit leaves nothing in the
   world for a snapshot to list and `Fired` carries no geometry, so a
   replica had nothing to draw. `Event::LaserBeam` now carries the muzzle
   and the stop point, and the replica draws and fades it (protocol 7).
3. **A client drawing at 30 fps sent 30 intents a second.** `send` paid
   out at most one packet per rendered frame, so a phone below 60 fps
   starved the room every other tick; the room repeated the last intent,
   the hull moved on ticks the sandbox never stepped, and every snapshot
   arrived as a correction. Fixed: a frame sends every tick it covered, up
   to `SEND_CATCH_UP_TICKS`.
4. **The web build links `-sASYNCIFY=1`**, which instruments the whole
   binary and roughly halves its speed (section 8 has asked for it to go
   since phase 2). Retiring it is phase 7's last item.
5. **The simulation is cheap, natively.** Measured in release on the
   default map (`net::round::tests::measure_client_costs`), per operation:

   | Operation | 0 enemies | 12 | 30 |
   |---|---|---|---|
   | server `Game::update`, one tick | 54 µs | 79 µs | 50 µs |
   | client `delta::apply_delta` | 0.2 µs | 0.6 µs | 0.8 µs |
   | client `Interpolator::sample` (a blend) | 0.1 µs | 0.2 µs | 0.3 µs |
   | client `apply::snapshot` onto the replica, per frame | 3.7 µs | 5.0 µs | 6.0 µs |
   | client `tick_presentation`, per frame | 0.8 µs | 3.4 µs | 7.1 µs |
   | client one predicted tick | 1.6 µs | 5.9 µs | 12 µs |
   | client `reconcile`, 11 inputs in flight, every snapshot | 17 µs | 99 µs | 164 µs |

   At sixty snapshots a second the reconcile is the one cost that grows
   with the round - one per cent of a native core at thirty enemies, three
   to five times that in WebAssembly - because the sandbox is a whole world
   and every replayed tick steps every enemy's body. Everything else is
   noise. So "slow" was the transport and the web build's flag, not the
   design; but the design still pays for its own hull with a machine
   (sandbox, replay, nudge, lead, provisional swap) whose whole purpose is
   to hide a server that owns a hull only the client ever steers. Stage 3
   (4.14) takes that machine out.

### As found (verified 2026-09-19, tree at ac5ebe4, v0.1.0)

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
- `trig.rs` is portable `sin`/`cos`/`atan2`: IEEE add and multiply in f64,
  rounded once, bit-identical on every platform. It exists for the pinned
  CPU render, and it is exactly the tool stage 2 reaches for if
  `drive_tank`'s transcendentals ever drift between a phone and the server.
- The site is a Cloudflare Worker deployed by `wrangler` from CI on tags, with
  per-PR preview workers. The join page and the well-known files are the two
  things it adds; it holds no room state.

In the way
- The sim links raylib. Seven simulation files import
  `sola_raylib::core::math::Vector2`, and every entity file (`tank.rs`,
  `frog.rs`, `shell.rs`, `obstacle.rs`, ...) keeps its `draw_*` next to its
  state. Nothing in `simulation/` needs a window, but the crate cannot build
  without the C library.
- No serializable world: `Game` is 66 fields around a `hecs::World`, neither
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
- Each seat has a mailbox (`net::mailbox`, shared by the room server and the
  rig): an **ordered jitter buffer** of intents keyed by the client's tick,
  applied one per tick, oldest first, capped at `BUFFER_MAX`. The tick reads
  every mailbox into one `Input`. A tick that finds a seat's buffer empty
  repeats its last intent and counts a starvation (a hiccup coasts rather
  than stops, for at most `INTENT_COAST`); a seat with nobody connected reads
  as no input, so its tank sits still and enemies still see it. Stage 1
  sampled the newest intent instead; the buffer replaced that because a
  replay on the client has to run the inputs the server ran, in order.
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
| Tank (51 fields) | `position`, `rotation`, body velocity, `damage`, `wreck_col`, `row` (in `Welcome`), `owner`, ammo counts, `flame_fuel`, active weapon and variants, `shield_hp` and `shield_broke`, `speed_boost_timer`, `burn_timer`, `portal_cooldown`, a "hit this interval" bit, `flame_held` | `damage_variant`/stage from `damage`; `shell_variant` from row and the alternating shot; `hull_frame` from velocity and time; `wet_timer` from the hull's own wading (the water layout is the map's); `despawn_timer` armed the first frame a wreck is seen, so the fade costs no bytes | `visual_rotation`, `turret_visual_rotation`, `ring_position`/`ring_velocity`, `hull_anim_accum`, `minigun_cycle_timer`, track wobble and jitter, `track_accum`, `pending_shot` timing (the second barrel's shell arrives as its own `Fired`); `throttle` and `shield_recharge_delay` are server bookkeeping and never travel |
| Frog (two) | `position`, `health`, a state byte with phase, `hop_end` while hopping | animation frame | — |
| Obstacle (one per solid cell: 578 on the default 34 x 17 field, more on a map with its own `size`) | layout in `Welcome`; then deltas: `health`, `burning`, `fuse` armed with total, `scorched` mask, `destroyed`, a ram-lean byte | `variant`, `edge_mask` (recomputed as `refresh_edge_masks` does), `burn_frame` from time since ignition | `burn_frame_timer`, `heat` (ignition is an event) |
| Portals (docs/teleporting.md) | anchors in `Welcome`; a hop is `Teleported` plus the tank's new position in the next snapshot | whether the network is active (two or more anchors) | the spiral's turn, the arrival flash |
| Pickups | bitmask of slot-backed pickups present; the Health slot's bonus shields and frog packs as explicit cells | kind and position from the map's slots | bob and spin |
| Shells, bullets, plasma | stage 1: id, kind, position, heading, state per live projectile. Later, with `Ricochet`: spawn from `Fired`, removal from `Hit`, nothing between | choreography frame from state and interval | — |
| Laser, flamethrower | laser: `Fired` plus the hit point; flame: origin, direction, capped reach while held | beam fade, cone flicker | — |
| Fires, oil, drums | `FireStarted`/`Ignited`, burning cells with remaining time as deltas, `oil_cells` in `Welcome` and removals, `DrumLaunched` with landing cell | the drum's arc and shadow; fire loop frames | — |
| Round state | `mission`, spawn plan (`Welcome`), wave index/size/alive/pending, the breather before the next wave, `intro_timer`, `outcome`, `restart_timer`, alert position and timer | `time` from the tick; banners, HUD | `paused` (meaningless online), `shadows_enabled`, `debug_overlays` |
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
// client -> server, one per tick, consumed in `tick` order (the mailbox)
Intent   { tick: u32, move_dir: u8 /* 0 none, 1-4 */, face: u8, fire: bool }

// server -> client, every tick (60 Hz), delta against the previous snapshot
Snapshot { tick: u32, server_ms: u32,
           acked:   [u32; MAX_SEATS],  // last input tick applied per seat
           mailbox: [u8; MAX_SEATS],   // per seat: intents still waiting after this tick's read, bit 7 = this tick starved
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
Lobby    { Create { nick, device_token, map, mission } | Join { nick, device_token, code } | Ready | Start | Leave | Kick { seat } | Chat { text } }
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
| `WaveStarted`, `TankEntered`, `WreckRemoved` | WAVE N banner; roll-in appears at the gate; the wreck goes (it has been fading on its own) | HUD, `fade_wrecks` |
| `ObstacleDestroyed {material, x, y}` | rubble decal, thrown | `props::obstacle_died`'s decal part |
| `Blast {x, y, chained, drum}`, `CookOff` | shaped fireball, scorch, parts, re-thrown rubble, flattened grass, burnt-in tracks, ripple, flash | `apply_blast`'s cosmetic half |
| `DrumLaunched` | the flying drum from launch to landing | `draw_flying_drum` from a launch time |
| `FireStarted`, `Ignited` | light the cell or tile | `light_cell`'s visual part |
| `Teleported {slot, from, to}` | the departure and arrival flashes; the hull is drawn at `to` from that tick on, no interpolation across the hop | the portal phase's cosmetic part |
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
- Interpolation (`net::interp`): the replica keeps the last few snapshots.
  Render time is server time minus the interpolation delay. **The delay is
  adaptive** (decision 8): one measured snapshot interval plus one 60 Hz
  frame plus the 95th percentile of how late snapshots arrive behind the
  fastest (isolated head-of-line stalls left out, and ridden out on
  extrapolation instead), floored at `online_interpolation_delay_ms` (33 ms
  at the room's 60 Hz) and capped at `online_interpolation_delay_max_ms`.
  Render time runs on its own playout clock and never jumps back: it is
  steered toward its target a few per cent faster or slower than real
  time, so a jittery spell stretches time rather than rewinding the
  picture, and a link that settles gives the milliseconds back the same
  way. `online_interpolation_adaptive` off pins the delay at the floor, for
  comparing the two on one link. Positions blend linearly; hulls snap by
  the four-way rule. A gap extrapolates on the last velocity and then
  dead-blends to a stop. Server time is the lower envelope of the snapshots'
  arrivals on the tick schedule (§4.16, "A controlled playout clock").
- `Game::tick_presentation(dt)`: `tick_effects` plus the cosmetic parts of the
  entity ticks (`ease_visual_rotation`, `ease_turret_visual_rotation`,
  `ease_ring_position`, hull animation, tread marks from displacement - none
  in water, wet for `wet_timer` after a ford -, grass crush and push from hull
  boxes, fire and fuse frames, decal flight, the drum's arc, the wave banner
  timer), and the round clock the water's shimmer and current marks and the
  fire loops are drawn from: `time` is derived, not sent - a room server's
  round runs without the mission banner, so its clock is exactly
  `tick * PHYSICS_FIXED_DT`, which every snapshot pins and the replica
  advances between them. `Game::wading` (every hull in a ford,
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

**One client, three modes.** Local play stays exactly as it is: the online
round is a third driver beside Play and Build, never a replacement for the
in-process `Game::update`.

- `mode::Session` gains an `Online` driver next to Play and Build. Play still
  runs `Game::update` in-process with the keyboard's `Input`, as today; Online
  runs `apply_snapshot` and `tick_presentation` on the replica and sends the
  local seat's intent. `Game::render`, `hud`, `fx`, the builder and touch are
  shared by construction, which is why the replica is a `Game`.
- The offline round keeps its whole toolchain: `--seed` replays, the probe,
  `just probe-fixtures`, the dev server's lockstep `step` and the determinism
  tests all run the local `Game` and see no network code. Couch two-player
  stays a local mode; couch seats inside an online room stay parked.
- One binary, one web build, one app per platform. The mode is chosen in the
  bar (PLAY, BUILD, ONLINE), by `--join CODE` or `--host` on the command line,
  or by the code in the join link's URL on the web. Embedded builds keep local
  play with touch and add ONLINE to the same bar.
- Phase 0's `render` feature is on by default for every client build; only the
  server image and CI's bare-wasm check turn it off. Local play never depends
  on a feature being off.

### 4.6 Transport and the client runtime

WebSockets everywhere in v1: the only transport all five platforms speak
without a native stack, passes every proxy, TLS is free with the domain. Its
weakness, TCP head-of-line blocking on a lossy link, costs a stutter at 20
packets a second, not a disconnect.

- Web (emscripten): emscripten's own WebSocket API (`emscripten/websocket.h`,
  linked with `-lwebsocket.js`) called through FFI from Rust:
  `emscripten_websocket_new`, `emscripten_websocket_send_binary`, and the
  open, message, error and close callbacks. The message callback copies the
  bytes into a Rust queue; the frame drains it at the frame boundary, the
  discipline `tuning::apply_pending` and the dev server follow. No site
  JavaScript and no `--js-library` file. Single-threaded JS is not a problem:
  the frame is a requestAnimationFrame task and socket callbacks run between
  frames; nothing ever blocks. Handle `visibilitychange` (a hidden tab stops
  rAF while messages keep arriving) by draining and keeping the newest
  snapshot. No wasm threads, no Worker, no Asyncify.
- Native (desktop, iOS, Android): blocking `tungstenite` with `rustls` and
  `webpki-roots` on one std thread that owns the socket: it reads with a short
  timeout and flushes the outgoing queue between reads, and talks to the frame
  through two mpsc channels, the dev server's pattern.
- One `Transport` trait, `send(&[u8])` and `drain(&mut Vec<Msg>)`, with three
  implementations: the emscripten socket, the native thread, and an in-process
  channel with artificial delay, jitter and loss for the rig. Everything the
  client says to a room, including creating one, goes over that one socket as
  a lobby message, so the game needs no HTTP client.

The datagram upgrade, later: WebTransport (native via `quinn`/`wtransport`;
browser support only recently reached Safari, keep the WebSocket fallback) or
WebRTC data channels (every browser, but drags an ICE/DTLS/SCTP stack into the
server). Confined to the transport layer by the protocol's split.

**Client runtime: tokio or threads?** Evaluated as asked, reading "use the
Rust version" as: prefer a plain Rust path that adds no runtime dependency to
the client. The frame loop is synchronous and raylib-driven, there is exactly
one socket, and everything crosses into the frame at its boundary, so an async
runtime has nothing to schedule on the client.

| Option | Native | Web (emscripten) | Adds to the client | Verdict |
|---|---|---|---|---|
| tokio and `tokio-tungstenite` on a background thread | works | does not: tokio's reactor needs mio, which has no emscripten support, so the web needs its own path anyway | tokio, tokio-tungstenite, rustls, webpki-roots, about 60 crates | two code paths, and the heavy one buys nothing |
| blocking `tungstenite` on a std thread with mpsc channels | works, on iOS and Android too | does not (no sockets) | tungstenite, rustls, webpki-roots, about 25 crates | the native path |
| emscripten's WebSocket API through FFI | not applicable | works; callbacks land between frames; emscripten ships the JS | nothing | the web path |
| a hand-written `net.js` `--js-library` | not applicable | works | a JS file to maintain in the repo | dropped; emscripten's API does the same |
| tokio through `wasm-bindgen-futures` | not applicable | wrong target: that is `wasm32-unknown-unknown`, not emscripten | nothing usable | not applicable |

What is shared with the server is the protocol, not the runtime: one `net`
module with the codec (`Intent`, `Snapshot`, `Welcome`, the lobby messages)
and the seat's state machine as plain synchronous code, used by the server's
async tasks and the client's thread alike. tokio stays a server dependency
only.

### 4.7 Room server

`bongbong-server`, one binary: axum for HTTP (health, metrics, the WebSocket
upgrade), tokio for the rest.

- One task per room owns the `Game`. Each tick it reads one intent from each
  seat's mailbox into an `Input`, calls `update`, and encodes the delta
  snapshot once and hands the same bytes to every seat's writer.
- One task per connection reads, decodes, and drops intents into the seat's
  mailbox; lobby messages go to the room task over a command channel. The
  writer has a bounded queue; a slow client gets snapshots skipped, never
  queued.
- Seats outlive connections, in memory: nickname, device token, chassis,
  connection state, last acked tick. A disconnect starts a 30 s grace; a
  reconnect with the same token reclaims the seat and gets a fresh `Welcome`;
  past the grace the seat is away but still owned for the whole round.
- Room codes are five letters from the 20-letter alphabet (4.10), and name a
  room and nothing else: one server holds them all, so a join needs neither a
  directory nor a letter reserved for routing.
- The dev server rides along: every room can expose the existing tools on a
  loopback port behind `dev-tools`, so a misbehaving live round is inspectable
  with the MCP tools used today.
- Bounds everywhere: rooms per instance (`--max-rooms`), waiting rooms
  (30 min), empty rooms (5 min), rounds (30 min).

### 4.8 Hosting: one container on Kubernetes at Hetzner

The room server ships as one container image and runs as **a single instance**
on the existing Kubernetes cluster at Hetzner (a requirement of 2026-09-19,
narrowed to one instance on 2026-09-24). The pipeline is boo-run's, which is
the pattern this cluster already runs: a private registry reached over
Tailscale, `docker buildx` for the image, and `kustomize edit set image` plus
`kubectl apply -k` for the rollout.

- The image (`Dockerfile`): a two-stage build - `cargo build --release -p
  bongbong-server` on `rust:1.97.1-bookworm`, then the binary alone on
  `gcr.io/distroless/cc-debian12`; about 59 MB, of which the binary is 12 MB.
  Headlessness is the manifest's, not a flag's: `server/Cargo.toml` takes the
  game crate with `default-features = false`, so raylib is not in the graph
  and the builder needs no cmake, no X11 and no GL - which is why phase 0's
  headless crate came first. The build asserts it, failing if `cargo tree`
  ever finds `sola` in the server's graph. The server reads no `static/`
  assets: `SHIPPED_MAPS` are embedded and a builder map travels in `Welcome`.
- The workload (`k8s/base/`): a Deployment `rooms` in namespace
  `bongbong-prod` with **one replica and `strategy: Recreate`**, one Service,
  one Ingress for `rooms.bongbong.io`, and a `ServiceMonitor` scraping
  `/metrics`. Nothing in the URL names an instance, because there is only
  one. `Recreate` rather than `RollingUpdate` because a room is memory: two
  replicas during a rollout would take rooms the other cannot see, and a code
  minted by one would name a room the other has never heard of. The ingress
  raises nginx's WebSocket read and send timeouts to an hour, or a round is
  hung up on after sixty seconds. There are **no secrets and no volumes**: the
  server holds no credentials, talks to no database and keeps nothing on disk.
- Probes, and the one subtlety: readiness is `/health`, which answers 503 the
  moment a drain starts - exactly right for taking the pod out of the
  Service's endpoints. **Liveness is a TCP check, not `/health`.** An HTTP
  liveness probe would see that same 503 and restart the pod seconds into
  every deploy, killing every round the drain exists to protect.
- Resources, from section 5: `requests` of 500m and 256 MiB against
  `--max-rooms 25`, at about 2 % of a core and under 2 MB per room. Modest on
  purpose - the cluster is a single node shared with every other production
  namespace, and a whole core reserved for a server nobody has found yet would
  be a poor neighbour. **No CPU limit**, deliberately: the tick is 60 Hz with
  a 16.6 ms budget, and a CFS quota would throttle at the period boundary and
  stutter every room on the pod together. The request guarantees the share;
  with no limit a busy pod still bursts into the node's headroom.
- Pulling the image: the namespace needs a `regcred` dockerconfigjson secret
  attached to its **default ServiceAccount**, which is how `boo-prod` does it
  and why neither project's Deployment mentions `imagePullSecrets`. It is
  per-namespace, so a new namespace has to be given one; k8s/README.md has the
  two commands.
- The drain: `terminationGracePeriodSeconds` 1830, just past `DRAIN_MAX`. On
  `SIGTERM` the server stops accepting new rooms and rematches, reports not
  ready, closes every room with no round in play (a lobby, a paused round, an
  end screen after `DRAIN_ENDED_TTL`), keeps ticking the rounds in play, and
  exits when the last ends or the grace runs out. Existing sockets stay up through the drain; a rejoin while it
  drains fails in v1, and a new room waits for the replacement. No `preStop`
  hook: there is nothing for one to buy when the process does not die for half
  an hour anyway.
- Per-PR previews (`.github/workflows/pr-server.yml`,
  `k8s/preview/rooms-preview.yaml`): every open PR gets a server in its own
  namespace `bongbong-pr-<N>`, at `wss://rooms.bongbong.io/pr-<N>/ws`. The
  preview takes a **path on production's host** rather than a host of its
  own, so it costs no DNS record, no certificate and no new API token - the
  ingress rewrites `/pr-<N>/ws` to `/ws`, nginx matches the longest path
  first, and production keeps everything else. No client change either:
  `--rooms wss://rooms.bongbong.io/pr-<N>` already produces that URL, since
  `socket_url` appends `/ws` to its base. A sticky comment links the PR's web
  preview at the PR's own server, so a protocol change can be played by a
  matching client rather than refused by production's `PROTOCOL_VERSION`.
  Closing the PR deletes the namespace, which is the whole preview.
- The pipeline (`.github/workflows/deploy-rooms.yml`): connect to the tailnet
  with a `tag:ci` OAuth client, `docker login` the private registry,
  `docker buildx build --platform linux/amd64 --push` two tags (`latest` and
  `git describe`), then in a second job `kustomize edit set image` and
  `kubectl apply -k k8s/base/`, annotate the Deployment with the revision and
  who deployed it, and wait out `kubectl rollout status --timeout=35m` - the
  timeout is the drain, not a guess. A third job prunes the registry to the
  newest three tags. **The trigger is the version tag
  `cloudflare-deploy.yml` already uses**, so the wasm client and the image
  ship together: a client and a server that disagree about
  `net::PROTOCOL_VERSION` refuse each other by name.
- TLS and DNS: `rooms.bongbong.io` **proxied through Cloudflare**, the way
  `boo.run` is - it resolves to Cloudflare addresses, and that is why its
  Ingress carries no `tls` block: the edge terminates TLS and the origin
  serves none. The cluster runs no cert-manager at all (`clusterissuer` is not
  a resource type there), so this is not a preference but the only path that
  works today. WebSockets pass the proxy, and its 100 s idle limit is never
  reached by a 60 Hz snapshot stream. Taking the socket DNS-only and off the
  proxy, which this section originally preferred, costs a cert-manager install
  and an issuer - worth doing if and only if the extra hop measures badly.
- The Worker keeps the site, the join page and the well-known files; it holds
  no room state.
- **Deferred: more than one instance.** When one process is not enough, rooms
  have to be findable across instances, and the cheapest way is the one this
  design was originally drawn with: a StatefulSet so instances have stable
  names, a per-instance Service selecting
  `statefulset.kubernetes.io/pod-name`, one Ingress path each (`/r/rooms-0/`),
  `/rooms` across all of them to create, and a routing letter carried in the
  room code so a client derives the path from the code with no directory. That
  costs a letter of the code, a URL rule with two shapes, and an operator
  setting per instance - none of which is worth paying before the traffic asks
  for it, which is why v1 pays none of it. The alternative, a directory the
  replay log of phase 6 already makes possible, is the better answer once
  there is persistence at all.

| Option | Fits because | Hurts because | Cost |
|---|---|---|---|
| **One container on the Hetzner Kubernetes cluster** — chosen | the cluster exists; one image, one manifest, one replica; a deploy is a drain and a restart; metrics and logs where the rest of the infrastructure has them | one instance is one failure domain and one capacity ceiling (~50 rooms per vCPU, section 5); a rejoin during a drain fails until persistence arrives | the cluster's existing cost |
| Cloudflare Durable Object per room (`workers-rs`) | zero machines; next to the site; placed near the room's creator | only after the sim compiles to bare wasm; a 60 Hz timer in a DO is workable, not a documented target; `wrangler dev` debugging; no datagrams ever | Workers Paid plus usage |
| A bare VM (Fly.io machines, or one box) | `cargo run` debugging; no cluster | a second way to run things next to the cluster | ~$3–5/month per region |

### 4.9 Rooms in memory, deploys and recovery

Version 1 keeps nothing on disk (a requirement of 2026-09-19): a room is
memory in the one server process, a planned deploy drains that memory
gracefully, and a crash loses it. Persistence is the horizon, not the plan.

| What | Where in v1 | Lifetime |
|---|---|---|
| the live world: the `Game`, hecs, rapier, timers, the AI's memory | the room task's memory | one round |
| seats: nickname, device token, chassis, connection state, last acked tick | the room task's memory | the room's life, so a rejoin within the grace works |
| the room record: code, host, map, mission, roster, results | the room task's memory | until the room ends and its 5 min for a rematch pass |
| nickname and device token on the player's side | localStorage, the app's keychain | across visits; the token is the reconnect key |

The room's life: **waiting** (not ticking; reaped 30 min after the last seat
leaves) → **playing** (60 Hz) → **paused, nobody connected** (not ticking: an
empty room must not burn CPU and the AI must not kill the frog with nobody
watching; the world stays in memory 5 min, resumes where it stopped on
reconnect) → **ended** (results and roster kept 5 min for a rematch, then
dropped).

- Deploys: a `Recreate` rollout with the drain of 4.8. The running server
  refuses new rooms, keeps ticking the rounds it has, and exits when the last
  ends or after 30 min; the replacement takes rooms from the moment it is
  ready. Rounds in progress finish on the old build, and for as long as they
  run no new room can be made - the price of one instance holding everything,
  and a deploy is therefore something to do when nobody is playing. A rollback
  is the same rollout backwards.
- Recovery: there is no replay log, so a crash or an unplanned restart ends
  every live round. Clients see the socket close and land on the end screen
  with "the room is gone"; the host makes a new room in one tap. Accepted for
  v1.
- Versions: a **protocol** number gates joining (web: "reload the page"; app:
  "update, or play this round in the browser", which the invite page already
  offers); a **sim** version is stamped in `Welcome` and in the logs. Ship the
  web build and the image from the same tag in the existing release workflow.
- Tuning is a build, not a hot reload: the native `--tuning` file re-read
  would alter every live round on a global table. Captured per room at
  creation, sent in `Welcome`, changed by deploying.
- Watch: rooms, seats, tick p50/p99, snapshot bytes/s, reconnects/min,
  version, as Prometheus metrics on `/metrics`; tick time nearing 16 ms is the
  one capacity alarm, and with one instance it is also the signal to build the
  distribution 4.8 defers.
- Persistence, when it comes (phase 6): the replay log (map, seed, tuning
  diff, then every tick's intents, ~3 KB/s per room) makes a crash cost
  nothing and lets a fresh process fast-forward a room, which turns the drain
  into a real blue/green and gives the several-instance design of 4.8 the
  directory it would rather have than a routing letter; records give results
  links, stats and shared custom maps. Both are additive: the room task
  already holds the intents and the record in memory.

### 4.10 Invites and identity

Whoever receives a link is in the match within one tap with nothing installed:
the browser build is the universal fallback, the apps the upgrade.

- Host presses ONLINE → HOST; creating a room is a lobby message on the
  WebSocket to `wss://rooms.bongbong.io/ws`, and the server mints a code of
  five letters from the 20-letter alphabet without vowels or look-alikes
  (3.2 million codes against a few hundred live rooms; idle rooms are reaped,
  collisions are a non-issue). The link `bongbong.io/j/CK7QX` goes to the
  share sheet; a QR and the code stay on the host's screen while the room is
  open. **The link points back at the page that minted it**
  (`net::rooms::SiteBase`): the deployed site from a desktop or phone build,
  and the page's own origin in a browser, so a PR preview's QR reaches that
  preview rather than handing a phone production's build of the game. The
  site is static and has no page at that path, so
  `site/public/_redirects` sends `/j/*` to `/?join=:splat`, the other spelling
  `Invite::parse` already reads. That redirect replaces the query rather than
  merging it, so `join_url` writes the query form directly when the host is
  overridden (`?join=CODE&rooms=...`) and keeps the pretty path for the
  deployed one - a QR minted against a laptop still reaches the laptop. **The code picks no URL**: every client - hosting or joining - dials
  that same `/ws`, and the code travels inside the join message, so no
  directory and no derived path stands between a link and a seat.
- The join page checks the version, then: browser, the wasm build with the
  code in the URL, read at start through `emscripten_run_script` like
  `bbShift`; iOS, universal links (`apple-app-site-association` under
  `/.well-known/` on the Worker, associated-domains entitlement),
  `bongbong://j/CK7QX` as the button fallback; Android, app links
  (`assetlinks.json`, an `autoVerify` intent filter in the manifest template,
  the code from the activity's intent data); desktop, type the code or
  `--join CK7QX` (the release archives register no scheme; an installer can,
  later).
- Identity: a nickname and a random device token minted on the join page and
  reused across visits (localStorage, the app's keychain). The token is the
  reconnect key within a room's life. Accounts can attach to it later without
  touching the protocol.
- Abuse: Turnstile on the web join page, a per-IP rate limit at the ingress
  for room creation, a nickname length cap and filter before anything public
  exists.
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
| Death | a wrecked player waits for the round to end | **built**: in a wave round of two or more a wrecked seat re-enters with the next wave through a gate (`simulation/waves.rs`), as a fresh tank of its own chassis - no penalty; band rounds and solo rounds keep today's rule (decision 7) |
| Frog, pickups | unchanged | unchanged; the server decides who reached a pickup first, and a frog health pack heals the collector's side's frog wherever it stands (`FrogHealed`) |

**Re-entry, as built.** `call_wave` queues every seat that is a wreck, in
seat order; the queue is drained at the head of that wave's own roll-ins,
one lane per `wave_stagger_seconds`, through `pick_gate` - the same lane
chooser the wave's tanks use, so a returning player and an arriving enemy
never share a gate. The seat keeps its entity, owner slot, chassis and worn
tread look and everything else is what `init` would have spawned: full
health, shells, no special weapon, no shield. It drives in kinematically
like a wave tank and takes no `Ai` on arrival - it answers its own stick the
frame it is through the gate. Four rules hold it together:

- **The round is still losable.** `check_round_end` walks `players()`, not
  the on-field seats, so a seat waiting for the next wave is a wreck like
  any other: the frame *every* seat is a wreck the round is Lost, exactly as
  before.
- **A solo round is untouched.** Its one wreck is every seat wrecked, so the
  round has already ended before anything could be queued; nothing is
  queued below two seats and a band round calls no wave at all. Both replay
  byte for byte (`probe-waves` and the band sweeps are identical across the
  change).
- **Off the field means off the field.** While a seat is in the gate lane
  `Game::seats_on_field()` reads it as empty - it is not shot at, blasted,
  burnt, rammed, retargeted to, routed to or the centre of an engagement
  ring, and `waves_finished` does not wait for it - the same shelter an
  entering wave tank gets for free by having no `Ai` yet.
- **A round that ends first keeps its result.** `wave_phase` only runs while
  the round is Playing, so a seat waiting when the round is won or lost
  stays where it fell, and a seat wrecked after the final wave is called has
  no wave left to come back with.

**Difficulty scales with seats** - a wave plan authored for one tank is a
walk for four - and the room is the only thing that scales it, through the
tuning patch its `Welcome` already carries (`room::tuning_patch`):
`wave_size_scale` multiplies every wave of the map's plan and `wave_tier_step`
lifts its tier ramp, both worked out from the seats the round starts with
and the two dials `online_wave_size_per_seat` (0.75) and
`online_wave_tier_seats_per_step` (3). One seat sends `{}`, so a room of one
is the offline round to the byte, and local play - couch, probe, builder -
never reads either row. The curve and the `--players` sweeps behind it are
in docs/maps-to-levels.md, "Difficulty by seat count".

### 4.12 Stage 2: prediction

Stage 1 shows your own tank the interpolation delay plus half a round trip
after the key; stage 2 shows it on the next frame. Rewind and replay, the
Quake 3 technique, with an easy time here because locomotion is a pure
function of intent and a mostly static world. This section is written as
built (`net::predict`, `net::round`, `net::mailbox`); the last paragraph is
what is deliberately not.

**Predicted.** The own hull's position, rotation and velocity; the own shot
leaving the muzzle - the shell or plasma bolt on the frame of the press, a
twin-barrel chassis's second shell `tank_twin_shot_delay_seconds` later, and
the minigun's whole burst, one bullet every `minigun_bullet_delay_seconds`,
burst after burst while the key is held; collisions with static terrain,
water and every other hull (exact for the statics, the server's answer wins
for the hulls). **Not predicted:** damage, knockback, ram, pickups, buffs, the
laser's beam (an instant hit, so its length is a hit test the client does not
run - and the weapon lag compensation is for, decision 9), the missile pod
(a volley the seeker steers), and the flamethrower's cone (a stream the
server caps at the first solid tile; the replica draws it from the `FLAME`
flag, `tick_presentation` rebuilding the jet from the hull). Everyone else
stays interpolated.

**The protocol.** `Intent.tick` is the client's own count of the packets it
has sent, one intent per packet (decision 15). The server holds each seat's
intents in a mailbox, an ordered buffer keyed by that tick, and applies one
per tick, oldest first; a tick that finds it empty repeats the last and
counts a starvation. `Snapshot.acked[seat]` is the tick of the intent
**applied** on that tick, not the newest received, because that is what a
replay is measured from. `Snapshot.mailbox[seat]` is how deep the buffer
stood after the read, with bit 7 set if the read starved - the reading the
client's lead is steered by.

**The sandbox and the loop.** The client owns a second `Game`, built by
`net::apply::welcome` exactly as the replica is, so the walls, obstacles and
deep-water boxes a replay steps against are the server's by construction.
`Game::predict_seat` is the only thing ever called on it - `drive_tank` and
one solver step, no RNG, nothing ages, nothing fires. Per packet: stamp,
send, step the sandbox, remember the input (the last `HISTORY_TICKS`). On
each snapshot: **the whole snapshot is applied to the sandbox** through
`net::apply::snapshot`, the same call the replica takes, so the sandbox is a
projection of the server's world rather than one of its own - every other
hull, a destroyed wall, a speed boost, a spent pickup, all of it by one
already-tested path (decision 10, amended: this replaced the kinematic
stand-ins, which had the predicted tank driving through hulls that had moved
and stopping against hulls that were gone). Then every input after `acked`
is replayed and the result compared with what was drawn: under `IGNORE_PX`
(half a pixel, the wire's own quarter-pixel rounding) nothing; up to
`SNAP_PX` (48, most of a hull) the difference becomes a visual offset decayed
over `NUDGE_SECONDS`; past it the hull snaps, which is what a teleport is.
The drawn pose is written into the replica between the snapshot and
`tick_presentation`, never after, so the presentation pass eases the facing
and presses the tread marks of the pose that is actually drawn.

**The lead.** The client sends one packet per `PHYSICS_FIXED_DT` of real time
(`app::StepClock`'s discipline) and the room reads one per tick, so on a
steady link the mailbox holds nothing or one intent after each read and
nobody waits. Jitter is what the depth absorbs: a packet late by a tick is
covered by the one already waiting. The client keeps a small, adaptive lead
in that depth rather than in the stamp: on a reported starvation it sends
one extra packet (the next tick's intent early, so the buffer is one deeper
from then on and the sandbox one tick further ahead); when the reported
depth has sat above `LEAD_DEPTH_MAX` for a `LEAD_WINDOW` with no starvation
it skips one (nothing sent, nothing stepped, the tap carried to the next).
At most one adjustment per window, so the loop cannot hunt; `BUFFER_MAX`
bounds what a runaway client can pile up. Each adjustment costs one tick of
prediction distance and nothing else - the replay runs the same inputs the
room will.

**Provisional shots.** A shot is drawn from the predicted muzzle on the
packet its press travels on, gated the way the server gates it: the
sandbox's seat carries the server's active weapon and ammo as of `acked`
(they are on the wire), the cooldown is the weapon's own
(`player_fire_interval`, the minigun's burst cooldown), and the ammo still to
be confirmed is subtracted, so a press the server would refuse draws
nothing. Shells and plasma fire on the press edge, the minigun while held,
the same rule `drive_player` applies. *As first built*, a provisional flew
by dead reckoning, met nothing and was retired when the room's shot
appeared in its place; stage 4 (4.16) replaced that: the provisional is
the one drawn copy for the shot's whole life, meets the drawn world, and
the room's copy is paired with it by `Fired::input_tick` and hidden, so
nothing is ever swapped. A shot no
`Fired` ever claims was one the server refused and goes quietly after
`PROVISIONAL_MS`; the count of those is the reading that says the local gate
is looser than the server's. The room's `Fired` also seeds the local
cooldown for a shot this client did not predict (prediction off, a press
from before the sandbox existed), measured back from `acked`, so the gate
agrees with the server's from then on.

**What still costs a correction**, deliberately: firing, since
`apply_recoil` pushes the shooter and the sandbox only drives (one nudge,
bounded by the recoil over one interval); a teleport (a snap, on purpose).

**Measured, in the rig and on the dev server.** `status.round.prediction`
(docs/dev-server-design.md) reads the predictor's counters: corrections
under `IGNORE_PX`, nudges, snaps, the distribution of the correction
distance (buckets at a quarter, a half, two, eight and forty-eight pixels),
the largest, shots drawn and shots refused, the inputs in flight (the
replay's length, which is the lead plus the round trip in ticks), and the
lead's own adjustments; `status.round.interpolation` reads the delay in
force, the jitter it is covering, the measured cadence and how many frames
ran on extrapolation. The rig's tests assert on the same counters, so the
numbers phase 4's exit test asks for - the mass under a quarter pixel, the
tail at contacts - are pinned rather than eyeballed.

**Under prediction** your hull is drawn at the present while others are the
delay in the past. Ramming a friend looks like arriving a beat early and
being settled back; an enemy's hit is judged against where you were on the
server. A provisional shot is drawn at the present against tanks drawn the
delay in the past, and stops where it meets one in the picture (4.16).
**Lag compensation** (decision 9) - the server rewinding targets to the
shooter's view - is what makes the room judge the shot against that same
picture rather than against where the tanks had since moved, and it is
built (4.16): each intent carries the tick the client was drawing
(`IntentMsg::view_tick`), the room keeps the last 250 ms of enemy and frog
hit boxes (`HitBoxHistory`), and a seat's shots and beams are swept against
the boxes of that tick (`sweep_rewound`). The measurement that checks it:
every provisional `advance_shots` stops against a drawn tank or frog is a
*crossing*; its paired room copy bursting within `HIT_MATCH_PX` (40 px) of
that stop makes it `crossings_hit`, and its copy flying on past
`MISS_MARGIN_PX` or bursting anywhere else `crossings_missed`, the room's
shot then shown in its place. `status.round.prediction` carries all three;
a missed rate above a few per cent says the rewind is wrong, and the laser
is the weapon that would complain first.

### 4.14 Stage 3: the client owns its hull

*Built 2026-09-28 on `feature/coop-ng-2`, behind `online_client_hull` (on
by default), stage 2's path kept under the same knob for the comparison
phase 8 asks for.* Stage 2 keeps the server the owner of
every hull and makes the local one *feel* owned by predicting it; stage 3
makes it owned. This is co-op against the AI: there is nobody to cheat,
and a hull position the client reports is as true as any the server
computes from that client's inputs a round trip later. The server stays
the referee for everything the players compete *with* - enemies, waves,
hits, damage, pickups, the frog, props, fires - and stops simulating the
one thing each client already simulates better.

**The model, as built.** Each client runs its own seat exactly as a local
round does: the sandbox `Game` stage 2 already builds drives it with
`drive_tank` and one solver step per tick against the map's statics and
the drawn hulls (`Game::predict_seat`), and a snapshot never moves it -
`Predictor::reconcile` in owned mode writes the room's world around the
hull and puts the hull back where it was, replaying nothing. Every tick
the packet carries the pose that tick produced (`IntentMsg::with_pose`:
`owned`, `x, y` in quarter pixels, `dir`, `vx, vy`), and the room puts
the seat there before the tick runs (`Game::accept_seat_pose`): the body
is placed at the pose with the reported velocity, so `combat::ram` reads
impact speed off it as before and the solver's contact response pushes
the dynamic enemies out of its way, and `drive_player` leaves the seat
alone for that tick (`Game::seat_owned`). A packet with no pose releases
the seat to the room's own driving. **Validation, not simulation**: the
step from where the room has the hull is bounded by the chassis's top
speed over the ticks the mailbox read vouches for
(`Mailbox::pose_reach_ticks`: the driving since the last pose, held to the
room's own ticks since it, never fewer than `POSE_REACH_TICKS`) plus
`POSE_REACH_SLACK_PX`; the pose must lie inside the field, off a solid
tile and out of deep water; a wreck and a seat still rolling in through a
gate own nothing. **The one correction path is `Placed`**: a refused pose
holds the hull where it was and answers with `Placed { seat, x, y, dir }`,
and so does a tick that moved an owned hull further than a contact could
(`PLACED_PX`) - a portal, a gate, the round's end; the client snaps to it
the way a teleport snaps (`Predictor::place_own`, also on this seat's
`Teleported` and `TankEntered`). **Shoves are carried**: the next tick's
pose would overwrite any push the room put on an owned hull, so the room
sends the velocity change it applied - a hit's knockback, a blast, a ram,
a missile launch's recoil - as `Shoved { seat, vx, vy }`, and the client
adds it to its own body the moment the snapshot arrives
(`Predictor::shove`), so the poses that follow carry it; a shell's, bolt's
or bullet's recoil is the client's own at the launch and never echoed. The
client's sandbox takes the room's world, its `Fired` and its shoves
whether or not the prediction is drawn (`online_predict_own_tank` off only
stops the drawing), since every pose comes from it. The water's current
the client already runs itself.

**Shots** are spawned where the client had the hull, by construction: the
server fires from the pose it just applied, so the shell leaves the muzzle
the client drew it from and there is nothing provisional to swap. The
client still draws its own shot on the press (the stage 2 press logic,
minus the retirement dance - the server's copy *is* that shot, keyed by
the tick it was fired on) and the laser is drawn at once to the first
solid tile, drawn tank or frog in the client's own picture, with damage the
server's. Whether the shot *hit* an enemy drawn the delay in the past was
decision 9's question; stage 4's lag compensation (4.16) answers it on the
room, which judges a seat's shots against the tick that seat was drawing.

**What it makes redundant**, and phase 9 removes once the comparison is
in: `reconcile`'s replay, the pose offset and its nudge and snap, the lead
controller and `Snapshot::mailbox`, the provisional retirement dance,
`Snapshot::acked` (the mailbox stays, as the ordered buffer it is, but a
starvation now means only a hull that held still for a tick on the
server, which nobody sees). What it added: seven bytes on the intent, the
validator, `Placed`. The `online_client_hull` knob (Restart) keeps both
models in the build until a real-link session has compared them; the
room needs no knob, since it honours whatever each packet says.

**Costs and limits.** A client's own hull costs one predicted tick a tick
(the table in section 3, 2 to 12 µs) and nothing per snapshot. The link
adds nothing to the own hull's response, ever: on a 300 ms link the tank
still turns on the next frame, only the enemies are 300 ms old. Two
players ramming each other resolve on each client against the *drawn*
other hull, so the two see slightly different contacts - acceptable for
friends against the AI, and the reason free-for-all (section 9) would put
the server back in charge of hulls, which the knob's two paths keep
possible. A client with a broken clock or a modified build could report
impossible poses; the validator bounds it to the chassis's speed and the
field, which is all a co-op round needs.

**What the field does** (the 2026-09-28 research, references in section
10). Destiny's engineering lead: "the server is authoritative over how the
game progresses, and each player is authoritative over their own movement
and abilities"; Bungie left Halo: Reach's lockstep co-op because paying a
round trip between the trigger and the shot was unacceptable for a game
meant to feel single-player. Unity's netcode guidance calls client
authority with server range checks acceptable for most PvE games and names
the two costs this section accepts - a contact judged on a different
version of the world, and overlaps where owner-driven hulls meet
server-driven bodies (their fix is what stage 3 does: the non-owner's copy
goes kinematic, gameplay collisions are the server's). Fiedler's co-op
physics hands authority to whoever last touched a thing and warns it is
for cooperative play only. Shipped prediction schemes - Half-Life,
Fiedler's, Rocket League, Lightyear - reconcile only on a mismatch against
a saved history and predict the local entity against a static world, which
is the fallback below; Rocket League's slides put the cost of the other
way at "200 ms ping, 120 Hz = 24 correction frames". On the delay: no
source shows anything near 33 ms holding up over TCP on phones - a lost
segment stalls every later snapshot for a round trip or a retransmit
timeout - so the adaptive widening and the four intervals of extrapolation
are load-bearing, and the own hull's feel must not depend on the delay at
all, which client ownership is the only way to guarantee. On shots, the
pattern is the client's fake projectile adopted by the server and
fast-forwarded a capped one-way latency toward it, damage the server's -
decision 18 with the fast-forward as an option once measured. Two things
the research reopened for the horizon: rapier's `enhanced-determinism`
feature claims identical results across platforms including wasm, which
weakens section 1's case against lockstep if the game's own float code
followed suit; and WebTransport now ships in every major browser
(Safari 26.4), so the datagram transport of 4.6 is buildable.

**If stage 3 is not taken**, the cheap path for stage 2 is written down
here so it is not lost: keep a ring of the sandbox's predicted poses by
tick, compare the server's pose at `acked` against the ring instead of
replaying, and replay only on a misprediction past `IGNORE_PX`. In steady
state that is one comparison per snapshot and no solver steps at all, and
it is what every shipped prediction implementation does. Stage 3 makes it
unnecessary.

### 4.15 The clock: measured, not guessed

*Built 2026-09-28.* A `Ping` carries the client's own milliseconds; the
room server echoes it as a `Pong` straight from the connection task with
its clock (`Hub::now_ms`, the one epoch every room's snapshots are
stamped with too), and `net::clock::RttClock` keeps a ten-second window:
the round trip as median, p95 and floor, and server-minus-local from the
**fastest** probe (Cristian: the probe that spent least time queued has
the tightest midpoint). `interp::ServerClock` stays what it is good for -
the snapshot buffer's depth, which is arrival-relative by design - and
`RttClock` answers "what is the server's time". Both are fed the instant
a message came **off the socket** (`Transport::drain_stamped`: the native
socket thread, the browser's callback, the loopback link's due time), not
the frame that drained it: stamped at the drain, every arrival read up to
a frame late, which the probe measured as round trip (22 ms on loopback,
2 ms after) and the interpolator as jitter. `status.round.rtt` and the
status line show it.

### 4.16 Stage 4: present-time co-op

*Designed 2026-09-28 from the research sweep and a 53-finding code audit
verified by two skeptics each; built on `feature/coop-ng-2` - each piece
below says how far it got.* Stage
3 made the own hull local; stage 4 makes everything the player acts on
local-feeling while keeping the server the referee. The organising rule:
**what the player controls lives in the present, what the player reacts
to lives in the present on its approach, everything else is interpolated
in the past - and the seams between them are hidden rather than ignored.**

The audit's worst findings, in the order they are fixed:

1. **Own shots jump backwards** 45-90 px when the provisional shell is
   swapped for the server's: the server's shell sits 183 ms at the muzzle
   in `Fire0..Fire2` and is drawn in the past, the provisional flies from
   the press in the present.
2. **The lead controller ratchets the mailbox** to about two ticks and
   never trims it, so every server-side action of an owned seat runs
   17-42 ms late; the owned hull only moves on packet frames and Extra or
   Skip double-step or freeze it.
3. **The kill spectacle never reaches a replica** - no fireball, mushroom
   cloud, shockwave, shake or flash online - because those are pushed from
   inside `Game::update`, which a replica never runs.
4. **Enemy shells are drawn ~110 ms in the past** against an own hull in
   the present: shells visibly miss and hit, or explode behind the tank.
5. **No lag compensation**: own shots are judged against enemies ~110 ms
   ahead of what was aimed at; the laser is a full round trip late.
6. **The playout clock follows every arrival** (an EMA), modulating the
   picture's speed every frame; one lost TCP segment freezes, rewinds and
   slow-motions the round for seconds; missiles are not interpolated;
   teleports are lerped across the map.
7. **Knockback, blasts, rams and recoil are erased** on an owned hull.

The pieces, each with its owner module:

- **Measurement first: `netlab`** (its own crate). The in-process room
  server behind an impairment proxy that models TCP rather than packets
  (delay; jitter that never reorders; loss as a hold of that segment and
  everything behind it until a retransmit time; a Nagle switch), N
  headless clients running the window's real `OnlineRound` over the real
  `NativeTransport`, all on one process clock so end-to-end latencies are
  exact; a **local twin** - the same scripted inputs through a local
  `Game` at the same frame rate - as the reference "feels local" is
  measured against; and a remote mode that points the same clients at a
  real server (a PR preview). It reports the own-input latency, the lag
  and pacing of remote motion (per-frame displacement: stalls, jumps,
  backward steps, their spread against the local twin), a shot ledger
  (press, provisional drawn, `Fired` handed over, server shot drawn,
  impact, hit) with the hand-off gap in pixels, how far an enemy shell was
  from the drawn hull when it hit, corrections, round trip, snapshot
  inter-arrival and bandwidth.
  *Built* (`netlab/`, `just netlab`, `just netlab-suite`); `netlab replay
  F --explain` re-measures a recorded run and names what each shot
  appearance was paired with.
- **The owned hull on its own clock** (`net::round`, `net::predict`): the
  sandbox steps on its own fixed-step clock, one packet per sandbox tick,
  no lead controller in owned mode, so nothing double-steps or freezes.
  *Built*. It is drawn at its **newest** tick, as a local round draws its
  newest step: drawing it between its last two ticks (the design) showed
  every input up to a tick late, and netlab measured two frames of own
  input at the median against the local twin's one. Recoil is applied on
  the press (`Game::seat_recoil`), once per shot whether drawn or not.
- **Owned poses played out on the room's clock** (`net::mailbox`, the
  room). Designed as newest-wins - each tick takes the newest pose - and
  built so first; netlab then showed the room's copy of an owned hull, the
  one every other seat draws, stalling and doubling whenever packets
  straddled a tick (3.8 % stalls and 2.3 % jumps on a LAN, against the
  twin's 0). The mailbox keeps a *play point* instead: the client tick the
  room applies, moved one tick a room tick, each read taking every intent
  at or before it (pose from the newest, the trigger merged press for
  press - a press whose release hid in an earlier merge is delivered up
  then down), a missing tick dead-reckoned. A once-a-second controller
  holds the point a tick after two late ticks in a window and moves it two
  after a window with two ticks to spare, so it settles a tick or so
  behind the latest arrival. The pose's reach is the driving since the
  last pose held to the room's own ticks since it, so a client stamping
  its packets far apart gains nothing. The room's tick is on the wall
  clock (`TickClock`: catch-up up to four ticks, a restart past that),
  serves a due tick before commands and one command between back-to-back
  late ticks. *Built*; LAN drive 0.4 % stalls, 0 % jumps, the room's copy
  about a tick further behind than newest-wins had it.
- **Own shots on the present timeline** (`net::predict`): the provisional
  runs the real state machine (the muzzle hold, then flight) from the
  sandbox's muzzle, stops at the replica's walls and at drawn hulls with
  its impact drawn at once (damage stays the room's), and is never swapped
  for the server's: the room's copies of this seat's shots
  (`ShotState::owner`) are hidden and paired by `Fired::input_tick`, and
  only a server outcome that disagrees (a hit the client did not draw, a
  drawn hit the room's copy flies on past or bursts away from, a refusal)
  corrects it. The laser and each shot's muzzle ripple are drawn on the
  press, and the replica skips the room's own for the presses it drew
  (`apply::Show::OwnShotsDrawn { seat, beams }`: each laser `Fired` claims
  the beam drawn for its press, and one no drawn press claims - the local
  gate refused what the room fired - is the room's beam to draw). *Built*,
  with one rule netlab added: **a shot outlives its own drawing** - kept
  off the picture until its room copy has come and gone, since a shell
  that bursts near the muzzle has played out long before its copy arrives
  a round trip later, and that copy was being paired with the next shot
  (which leapt hundreds of pixels to the first one's impact) or shown on
  its own (the shot drawn twice) on every link. Its converse: a shot whose
  copy was due in the picture and never came (the snapshot carrying it
  stepped over, the copy dead between two) is orphaned - left out of the
  pairing, so the next shot's copy is not taken for it - and goes as soon
  as it is off the picture.
- **Lag compensation, favor the shooter** (the simulation's hit test):
  each intent carries the tick the client was drawing
  (`IntentMsg::view_tick`); the room keeps the last 250 ms of enemy and
  frog hit boxes (`HitBoxHistory`), and a seat's shots and beams are
  swept against the boxes at the tick that seat saw (`weapons::Rewind`,
  `sweep_rewound`). In co-op the AI does not feel it, so it is strictly a
  gain. *Built*; a round with no seat views rewinds nothing.
- **Incoming fire in the present** (`net::round`): enemy and teammate
  shots are drawn forward along their straight path by the local lead,
  starting at the drawn muzzle and catching up over ~120 ms, clipped at
  walls; one that reaches the own drawn hull shows its impact at once
  (health stays the room's). *Built*: `incoming_lead_ticks` and
  `draw_incoming_in_present`.
- **A controlled playout clock** (`net::interp`): render time in ticks,
  monotone, steered at a bounded rate (while it runs past the newest
  snapshot with the target behind it, bounded by the full `RATE_FAR` (10 %)
  rather than the error-ramped bound, without the deadband), the delay
  from a lateness percentile with
  head-of-line stalls ridden out on extrapolation; per-entity error
  offsets that decay instead of snapping; missiles blended; hulls and
  shots extrapolated along their paths; teleports snapped. *Built*, with
  one change netlab forced: **the link is measured on the wall clock**.
  The clock read tick time less arrival, so a waiting room's welcome -
  tick 0 of a round that starts a tenth of a second later - opened every
  round with the clock that far ahead (seconds of extrapolation, a 150 ms
  delay on a 30 ms link), and a room that dropped ticks read as a late
  link. It takes `server_ms` less arrival now, which only the link moves,
  and the round's anchor (`server_ms - tick time`) turns it into tick time;
  a welcome restarts the anchor, and an anchor that jumps forward holds the
  picture for as long as the room stood still.
- **The spectacle on replicas** (`net::apply`): fireball, mushroom,
  shockwave, flash, scorch and thrown parts from `Wreck`; blast effects
  from `Blast` and `MissileBlast`; cook-offs; impact flashes from `Hit`,
  `Deflected`, `Ricochet`, `ShellsCollided` and a shielded laser hit;
  muzzle ripples from `Fired`; rubble from `ObstacleDestroyed`. Each
  cause's cosmetic half is one `Game` show both a local round and a
  replica call, so they cannot drift. *Built*; the predictor's sandbox
  takes snapshots quietly (`Show::Quiet`).
- **Shoves on owned hulls** (`Event::Shoved`): the room sends the velocity
  changes it put on an owned hull - knockback, blasts, rams, a missile
  launch's recoil - and the owner applies them to its own body; a shell's,
  bolt's or bullet's recoil is the client's own at the launch and never
  echoed. *Built*.
- **Measuring a browser**: a dev-tools web build (every PR preview) exports
  `bb_net_stats` (`OnlineRound::stats_json`, the same readings the native
  dev server shows) and `bb_input` (a scripted seat), so a browser tab
  against a deployed room is driven and measured by a script. *Built.*

**Measured** (netlab `suite --quick`, 8 s runs at 60 fps, the client
owning its hull; *before* is stage 3 at `8c99fdf`, *after* is stage 4 as
merged; profiles are one-way delay ± jitter with loss held as TCP holds
it - lan 0 ms, typical 40 ± 10 ms, bad 100 ± 40 ms with 3 % loss):

| run | own input p50 | remote lag p50 | remote stall / jump % | own shot hand-offs (gap p95) | drawn twice | verdict |
|---|---|---|---|---|---|---|
| lan drive, before | 16 ms | 66 ms | 0.0 / 0.0 | - | - | local |
| lan drive, after | 12 ms | 60 ms | 2.0 / 1.5 (0.3 / 0.2 alone) | - | - | close |
| typical drive, before | 17 ms | 167 ms | 0.3 / 0.0 | - | - | local |
| typical drive, after | 13 ms | 156 ms | 0.0 / 0.0 | - | - | local |
| bad drive, before | 0 ms | 515 ms | 14.4 / 5.5 | - | - | far |
| bad drive, after | 18 ms | 458 ms | 11.8 / 4.7 | - | - | far |
| typical shoot, before | 17 ms | - | - | every shot (96 px) | - | far |
| typical shoot, after | 22 ms | - | - | 1 (89 px) | 0 | far |
| lan duel, before | 17 ms | 73 ms | 1.3 / 0.0 | every shot (64 px) | - | far |
| lan duel, after | 17 ms | 62 ms | 0.8 / 0.0 | 1 (40 px) | 0 | far |
| typical duel, before | 17 ms | 168 ms | 1.3 / 3.3 | every shot (97 px) | - | far |
| typical duel, after | 16 ms | 154 ms | 2.0 / 1.0 | 0 | 0 | close |

The local twin reads 17 ms of own input (one frame) and no stalls. The
hand-offs left are the room's outcome disagreeing with the drawn one - a
hit the picture did not draw, or a drawn hit the room missed - one or two
in an 8 s run of fifteen presses. Over the internet to the preview server
(`wss://rooms.bongbong.io/pr-48`, a 28-33 ms round trip at the median
and ~110 ms at the 95th) before the clock and playout fixes: own input
one frame, remote lag ~120 ms, the picture 150 ms behind and
extrapolating - the reading the two fixes were made against.

Deferred, written down: WebTransport datagrams for the snapshot and
intent streams (HOL blocking is the one link effect no client-side trick
removes); sub-tick rendering of the local round; the mobile lifecycle.

### 4.13 Running it locally

The developer loop needs no cluster: the server is a `cargo run`, the client
points at it with one override, and the container is the same binary. The
flags land with phase 2; the rig of phase 1 needs no server at all.

```
# the room server on loopback: plain ws://, every room in this one process
just run-server            # cargo run -p bongbong-server -- --listen 127.0.0.1:4848 --insecure

# a host: creates a room over the socket, prints and shows the code
BONGBONG_ROOMS=ws://127.0.0.1:4848 cargo run -- --host

# a second client on the same machine, joining with that code
BONGBONG_ROOMS=ws://127.0.0.1:4848 cargo run -- --join CK7QX --nick second

# the web build against the same server: the page takes the room and the override from its own URL
just serve-web-dev         # host: http://localhost:4321/?rooms=ws://127.0.0.1:4848
                           # join: http://localhost:4321/?join=CK7QX&rooms=ws://127.0.0.1:4848
                           # (the site's own /j/CK7QX route is the Worker's; the preview serves ?join=)

# the container, exactly what the cluster runs (k8s/README.md)
just rooms-image           # docker buildx build --platform linux/amd64, tagged from `git describe`
just rooms-image-run       # docker run --rm -p 4848:4848 ... --listen 0.0.0.0:4848 --insecure

# the offline rig: an authoritative Game on a thread, the replica in the window, no server
cargo run -- --rig --delay 80 --jitter 20 --loss 0.02
```

- `--insecure` says plain `ws://` is expected, with no TLS terminator in front,
  and skips Turnstile. It is recorded in the start-up line; deployed, the
  ingress holds the certificate and the flag stays off.
- `BONGBONG_ROOMS` (or `--rooms`) overrides the rooms host, default
  `wss://rooms.bongbong.io`. There is one URL rule and the override does not
  change its shape: the host's own `/ws`, for creating a room and for joining
  one alike.
- Two clients on one machine are two seats because the native device token
  lives per nickname (`--nick`); the web build crosses a device id in
  localStorage with a tab id in sessionStorage, so a reload reclaims the seat
  and a second tab is a second player.
- Behind `dev-tools`, `--dev-port 4747` exposes the dev server for the newest
  room, so `just mcp-call status`, `snapshot`, `events`, `history` and
  `terrain` inspect a live round; `screenshot` refuses, since there is no
  renderer.
- In the *window*, those tools follow the round on screen: in an online round
  they describe the room's replica and `status.round` names the room, the
  seat, the buffer and the server's tick, while everything that would write -
  `step`, `restart`, `teleport`, the builder tools - refuses, since only the
  server simulates that round (docs/dev-server-design.md section 4.2).
  `net::rig::Lockstep` is how a test steps a networked round instead: the rig
  with no thread and no clock, the replica catching up to the last snapshot
  each `step` earned.
- `cargo test -p bongbong-server` starts the server on an ephemeral port and
  plays a round through a headless client; it is the CI check for the
  protocol.
- Rehearsing the drain needs no cluster: run the image, start a round, then
  `docker kill -s TERM` it. `/health` turns 503 at once, the container stays
  up, and the round plays to its result before the process exits - which is
  the whole contract `terminationGracePeriodSeconds` is sized against. In a
  cluster it is `kubectl rollout restart deployment/rooms -n bongbong-prod`
  and the same wait.

## 5. Numbers (order of magnitude)

| Item | Value |
|---|---|
| Intent up, 60 Hz, one intent per packet (decision 15) | 0.4–0.8 KB/s payload, ~3 KB/s framed; stage 3's pose adds about 8 bytes a packet |
| Snapshot down, 60 Hz, 8 tanks + 24 shots, delta | ~6–8 KB/s payload, ~12 KB/s framed (1.7 KB/s was measured on a real two-seat round at 30 Hz, before the cadence doubled) |
| Worst case, ~40 tanks in a wave round (`wave_max_alive`'s cap of 31 plus eight seats) | ~10 KB/s payload |
| `Welcome`, deflated | ~4 KB once |
| Server tick, 12 enemies + 1 player (measured, section 3) | ≈ 230 µs on a 2.1 GHz Xeon vCPU, ≈ 95 µs of it per-frame fixed cost |
| Server tick, 12 enemies + 8 seats (projected) | ≈ 320 µs; ≈ 550 µs at `wave_max_alive` |
| CPU per room at 60 Hz | ≈ 2 % of a core at 8 seats, ≈ 3 % worst case; 30–50 rooms per vCPU of that class, more on a desktop core |
| Memory per room | < 2 MB |
| Client replay per snapshot, stage 2 | < 0.2 ms |
| Own-tank latency, stage 1 (prediction off) | ≈ 80–160 ms on a clean link (≤16 sample + 20–60 up + ≤16 tick + ≤17 cadence + 20–60 down + 33 interpolation), plus the jitter margin the adaptive delay adds on a dirty one |
| Latency to a tank a player *aims at*, stage 2 | the same ≈ 80–160 ms. Prediction carries the local hull only, so what remains is paid by every other hull; the two terms that are a choice rather than a cost - the cadence and the interpolation delay - are 50 ms of it, down from 150 at 20 Hz |
| Own-tank latency, stage 2 | the next frame |

## 6. Phased plan

Sizes: S days, M a week or two, L several weeks. Phases 0 to 3 are stage 1 and
ship a complete co-op game; 4 and 5 are stage 2.

0. **Headless simulation crate (M).** Replace the seven `sola_raylib::Vector2`
   imports with a local `math::Vec2`; move each entity file's `draw_*` and
   `*Textures` behind a `render` feature (on by default for every client
   build) or into `*_draw.rs` siblings; generalise `Input` to N seats; a fixed
   60 Hz accumulator in `app.rs`. Done when `probe` and `cargo test --lib`
   build with `--no-default-features` and no C compiler, `just
   probe-fixtures` is byte-identical, and the windowed game plays exactly as
   before. Bonus: CI builds the crate for `wasm32-unknown-unknown`.
1. **Replica and protocol, offline (L).** The state audit made real:
   `Snapshot`, `Welcome`, the intent packet, delta encoding;
   `encode_snapshot`/`apply_snapshot`; `tick_presentation`; the cosmetic
   halves of `wreck_fx`, `apply_blast`, `obstacle_died` callable from events;
   `Ricochet` (iron and the barrel bounce) and whatever else the audit finds
   silent; the `Online` driver in `Session`. The rig: an authoritative `Game`
   on a thread, a replica in the window, an in-process transport with dialled
   delay, jitter and loss. Done when the round looks and feels like the local
   game at 80 ms and 2 % loss with every effect present, a snapshot round-trip
   test sits next to `determinism_tests`, and local play is untouched.
2. **Room server, container, join links (L).** `bongbong-server` and its
   image; the emscripten socket and the tungstenite thread behind the
   `Transport` trait; the image, the single-replica Deployment, TLS and the
   drain on the Hetzner cluster; five-letter codes that name a room; the
   Worker's join page and well-known files; the ONLINE dialog, the lobby, the code and QR. Done
   when a link from a phone puts a laptop in the same wave round, a closed tab
   rejoins the same seat, and a deploy mid-round lets the round finish before
   the replacement starts.
3. **Co-op polish (M).** Seats beyond two, the compact HUD, N-seat spawns,
   re-entry with the next wave, end screen and rematch, host hand-over,
   builder maps in rooms, clock sync and adaptive interpolation delay. Done
   when four friends on four platforms finish a wave round, one drops and
   rejoins, and they rematch without a new link. **Stage 1 ships.** *Built*
   (v0.2.0), except host hand-over and the adaptive delay, which landed with
   phase 5b below.
4. **Prediction: sandbox and reconciliation (L).** `set_velocity` and
   kinematic stand-ins; statics from a map on a bare world; input history and
   the client tick with adaptive lead; the server's ordered buffer and
   `acked`; rewind and replay; the visual correction with nudge and snap
   thresholds; the metrics. In the rig first. Done when at 120 ms RTT with
   jitter and loss the tank answers on the next frame, the error histogram
   sits under a quarter pixel away from contacts, and a wall stop matches the
   server's to the pixel. *Built*: the sandbox as a projection rather than
   with stand-ins (decision 10), the mailbox and `acked`, nudge and snap;
   the lead and the metrics readout landed with phase 5b.
5. **Prediction: shots, contacts, maybe lag compensation (M).** Provisional
   shells; predicted cooldown and ammo; full-auto visual streams; stand-ins
   and ram behaviour; the rewind ring and lag-compensated hit test if decision
   9 says yes. Feel pass on real links. **Stage 2 ships.**
   - 5a *built*: the provisional shell, on by default, retired against the
     room's `Fired`.
   - 5b *built* (2026-09-26): the shot gated by the sandbox's weapon and
     ammo, the cooldown seeded from the room, the twin's second shell and
     the minigun's bursts, retirement at the frame the server's shot
     appears; the adaptive interpolation delay; the client's lead as a
     mailbox depth; the metrics on `status.round`; the flamethrower's cone
     drawn on a replica at all.
   - 5c *open*: the feel pass on real links, and decision 9's reading -
     the instrument is on `status.round.prediction`. Lag compensation
     only if it says so.
7. **The field's fixes (S).** `TCP_NODELAY` on both ends; the laser beam as
   an event; a slow frame sends every tick it covered; `-sASYNCIFY=1` off.
   *Built* 2026-09-28 on `feature/coop-ng-2`, all but the last, which
   needs a browser to check. Done when a phone at 30 fps and a laptop on a
   real link show the delay at the floor, no lead adjustments, no
   starvations, and the laser.
8. **Stage 3: the client owns its hull (M).** Behind `online_client_hull`:
   the pose on the intent, the placed seat body and the validator,
   `Placed`, the shot fired from the applied pose. *Built* 2026-09-28
   (`server/tests/round.rs`: the room's snapshot follows the owned hull
   within a tenth of a pixel with zero corrections); `Shoved` and the
   laser drawn on the press came with stage 4. Done when a real-link
   session with the knob on shows no corrections at all on the own hull,
   the shot leaving from where it was drawn, and the same round with the
   knob off for comparison.
8b. **Stage 4: present-time co-op (L).** 4.16: netlab first, then the
   owned hull on its own clock, the play point, own shots on the present
   timeline, lag compensation, incoming fire in the present, the playout
   clock on a wall-measured link, the spectacle on replicas, shoves.
   *Built* 2026-09-28 on `feature/coop-ng-2` (PR #48), measured in 4.16.
   Open: the one or two own-shot corrections an 8 s run still makes (the
   room's hit against the drawn one), phantom strikes of incoming fire (a
   strike drawn on the hull the room judged a miss, one or two a run), and
   the browser reading (`bb_net_stats` on a visible tab against the PR's
   room). Done when netlab's typical profile reads `local` or `close` on
   every scenario and a real-link session agrees.
9. **Stage 3 takes over (S).** Once 8's comparison says so: delete the
   stage-2 reconciliation's replay, the lead and the ordered mailbox path,
   keeping the sandbox, which drives the owned hull; the protocol loses
   `mailbox`. Then the feel pass on real links on the simpler machine.
10. **Horizon.** Distribution: more than one instance, with the directory the
   replay log makes possible rather than a routing letter in the code (4.8).
   Persistence: the replay log (crash recovery, true blue/green deploys),
   records and results links; projectiles as events; datagram transport; more
   regions; accounts and friends on the device token; public rooms; replays
   from the log; a desktop installer with the URL scheme; the parked modes.

## 7. Decisions and recommendations

1. Room host: a VM, Cloudflare Durable Objects, or the Kubernetes cluster?
   **The Hetzner Kubernetes cluster, as a container** (a requirement of
   2026-09-19); the Worker keeps the join page; keep the DO route alive with a
   bare-wasm CI build.
2. Transport v1: WebSockets, or WebRTC data channels from day one?
   **WebSockets**; revisit only if lossy mobile links stutter, then
   WebTransport.
3. Identity: nickname plus device token, or accounts? **No accounts in v1.**
4. Room size: up to 8 seats on the shared 34×17 field? **8**, 60 Hz tick,
   and a snapshot every tick - it started at 20 Hz, went to 30, and the
   cadence was raised to 60 so the interpolation delay could come down
   with it (two intervals, whatever the interval is); expect four seats to
   be common.
5. Desktop deep links: type the code, or an installer registering
   `bongbong://`? **Type the code, or `--join`.**
6. Crate split: a `bongbong-sim` workspace crate, or a `render` feature?
   **Feature first**, on by default for clients; promote when the server image
   wants a slimmer graph.
7. A wrecked player in a wave round: watch, or re-enter with the next wave?
   **Re-enter through a gate, no penalty**; band rounds keep today's rule.
   *Built in phase 3c* - see 4.11 for the four rules that hold it together.
8. Interpolation delay: fixed 100 ms, or adaptive? **Fixed in phases 1–2,
   adaptive in 3.** Built 2026-09-26 (4.5): the knob is the floor, the
   link's jitter widens it, slewed rather than jumped.
9. Lag compensation: rewind to the shooter's view, or judge at server time?
   **Server time through stage 2, then measure.** Only the laser will
   complain, and in co-op the "shot behind cover" complaint has no victim with
   a grudge.
10. Other tanks in the sandbox: kinematic stand-ins, or ignore? **Stand-ins**
    was the answer here, and it was overtaken: the sandbox is a whole `Game`
    that takes every snapshot through `net::apply`, so every other hull is
    where the server has it by the same path the replica uses (4.12). No
    stand-in code exists; without the projection a predicted tank drove
    through a friend for a round trip and snapped back, which is the case
    this decision was about.
11. Persistence in v1: replay log and records, or nothing? **Nothing** (a
    requirement of 2026-09-19): rooms are memory in one process, a crash ends
    its rounds, a planned deploy drains.
12. Client runtime: tokio everywhere, or threads? **Threads**: blocking
    `tungstenite` on a std thread natively, emscripten's WebSocket API on the
    web; tokio is server-only; the protocol codec is the shared part (4.6).
13. Local play: keep it in the same binary, or make online the game? **Keep
    it untouched** (a requirement): Online is a third `Session` driver; Play,
    Build and every offline tool are unchanged.
14. How many server instances in v1: one, or several behind routing?
    **One** (a requirement of 2026-09-24). Every room is in its memory, so a
    code names a room and nothing else, and one URL reaches them all. The
    several-instance design - stable names, a path per instance, a routing
    letter in the code - is written down in 4.8 and built when the capacity
    asks for it, by which time persistence may offer a directory instead.
15. Intents: the last three per packet, or one? **One.** The three-per-packet
    redundancy was written for a datagram transport; on a WebSocket nothing
    is lost or reordered, so the copies would only cost bytes. The lead the
    stamp was meant to carry lives in the mailbox's depth instead (4.12),
    which a datagram transport could keep unchanged.
16. Nagle's algorithm on the sockets? **Off, on both ends, always** (a rule
    from 2026-09-28): a sixty-a-second stream of small frames is exactly
    what Nagle and delayed acknowledgements turn into round-trip-paced
    bursts, and no test on a loopback can catch it. Every socket the game
    or the server opens sets `TCP_NODELAY`; a new transport inherits the
    rule.
17. Who owns a seat's hull in co-op: the server, predicted on the client,
    or the client, validated on the server? **The client** (stage 3, 4.14),
    because there is nobody to cheat in a round against the AI and the
    machine that hides server ownership is the most complex and the most
    link-sensitive thing in the tree. Kept behind `online_client_hull`
    until a real-link comparison is in; the parked versus modes would put
    the server back in charge.
18. Shots: provisional on the client and swapped for the server's, or
    fired by the server from the client's own pose? **From the client's
    pose** once stage 3 owns the hull: the server's shot then *is* the one
    the client drew, and the swap goes.
19. `-sASYNCIFY=1` in the web build? **Off.** Nothing in the loop yields,
    there is no audio, and the instrumentation halves the client's speed on
    the platform that has the least to spare.
20. An owned seat's poses at the room: the newest waiting each tick, or
    played out? **Played out** (4.16), one client tick a room tick with a
    margin a controller keeps: newest-wins is a tick less lag and a hull
    every other seat sees stall and double whenever packets straddle a
    tick, which on a real link is every second. Measured, not argued.
21. The client's clock: tick time less arrival, or the room's wall clock?
    **The wall clock**, with a per-round anchor turning it into ticks: the
    link is one thing and the round's tick numbering another, and reading
    them as one made a waiting room's welcome, a round start and a dropped
    tick all look like the link.
22. When does a provisional shot retire: when its drawing ends, or when
    its room copy does? **When its room copy does** (or never comes): the
    copy arrives a round trip after a near-muzzle burst, and a copy with no
    shot of its own is either drawn twice or taken for the next shot.

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
  route the few transcendentals in `drive_tank` through `trig.rs`, which
  already gives bit-identical `sin`/`cos`/`atan2` on every platform, without
  lockstep-grade effort.
- **Tuning agreement.** Online rounds apply the `Welcome` diff and lock the
  panel; the sandbox reads the same table; a mismatch shows as systematic
  prediction error in the metrics.
- **No persistence.** A crash or an unplanned restart ends every live round,
  and a rejoin while the server drains fails. Accepted for v1; the replay log
  is the fix, and the room task already holds everything it would write.
- **One instance is one ceiling and one failure domain.** Every room is in
  one process, so a crash takes them all and a deploy blocks new rooms until
  the rounds in progress finish. Accepted deliberately (decision 14). The
  stop condition is `bongbong_tick_microseconds` nearing 16 ms or the room
  count nearing `--max-rooms`: either is the signal to build the distribution
  4.8 defers, before players meet the ceiling.
- **Cloudflare in front of the socket.** A proxied `rooms.bongbong.io` adds a
  hop and a 100 s idle limit; keep it DNS-only unless the proxied path
  measures fine.
- **Web build.** Retire `-sASYNCIFY=1` before phase 2; the emscripten socket
  does not need it and it inflates the wasm on the invite's critical path.
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
- **Trusting the client's hull (stage 3).** A modified client could report
  poses it never drove to; the validator bounds every step to the
  chassis's top speed and refuses walls, deep water and the field's edge,
  which bounds the damage to "a friend who teleports a little". Accepted
  for co-op; any versus mode brings server ownership back (decision 17).
- **Two clients, two pictures of a ram.** With client-owned hulls, two
  players colliding each resolve against the other's *drawn* hull, so the
  contact differs by the delay on each screen. Bounded by hull size and
  the delay; the server's ram damage is judged once, on the reported
  velocities.
- **The loopback lies about the link.** The rig and the server tests never
  see Nagle, delayed acknowledgements, a phone's radio or a proxy; a
  transport-level change is not verified until a real link has carried it.
  Phase 7's "done when" is measured on production or a PR preview, never
  on `127.0.0.1`.

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

- Bongbong online co-op, the design page (2026-09-19): this design with
  diagrams, the measurements and the open checklist; it supersedes the
  "Bongbong Online" page (rev 2, 2026-09-15).
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

Research behind stage 3 (2026-09-28; what each says that matters):

- Bungie on Destiny 2 (kotaku.com/bungie-explains-how-theyre-improving-destiny-2-servers-1795587013):
  players authoritative over their own movement and abilities, the server
  over the game's progress.
- Truman, GDC 2015, Destiny's networking (archive.org/stream/GDC2015Truman):
  lockstep co-op costs a round trip per shot; AI on the physics host.
- Unity Netcode for GameObjects, "Dealing with latency" and "Physics"
  (docs.unity3d.com/Packages/com.unity.netcode.gameobjects): client
  authority with range checks is acceptable for PvE; non-authority bodies
  kinematic; collision gameplay on the server.
- Fiedler, "Networked physics in virtual reality" (gafferongames.com):
  authority by interaction, cooperative only, 60 packets a second.
- Bernier, "Latency compensating methods" (Half-Life): predict only the
  local player, replay unacknowledged commands, 100 ms interpolation.
- Fiedler, "Networked physics (2004)": replay only on a significant
  difference; owned objects against a static world.
- Cone, GDC 2018, "It IS rocket science" (Rocket League): 120 Hz, predict
  everything, compare against history, correct only on a large difference,
  server input buffer with speed-up/slow-down.
- Unity Netcode for Entities, "Prediction": ~22 resimulation frames at
  300 ms; keep most ghosts interpolated.
- Lightyear (cbournhonesque.github.io/lightyear): rollback only on
  mismatch by default; hash-matched pre-spawned projectiles.
- Fiedler, "Snapshot interpolation" and "State synchronization": the
  delay should survive two lost packets, 85 ms at 60 a second; a 4-5 frame
  jitter buffer.
- Riot, "Peeking into Valorant's netcode": 128 Hz over UDP with about one
  frame of buffering - the number 33 ms is compared against.
- Reitich, projectile prediction in Unreal (sreitich.github.io): the fake
  projectile on the press, the real one fast-forwarded halfway, damage the
  server's.
- Jangda et al., USENIX ATC 2019, "Not so fast": wasm 45-55 % slower than
  native on average.
- emscripten, "Asyncify": about 50 % overhead in size and speed.
- rapier, "Determinism": `enhanced-determinism` across platforms including
  wasm.
- Fiedler, "UDP vs TCP"; Voidgun's devlog on WebRTC; kernel tcp-thin; RFC
  8985: head-of-line blocking, 50-200 ms stalls per loss at 45 Hz over
  WebSocket, no fast retransmit under four packets in flight.
- WebKit, "WebKit features for Safari 26.4": WebTransport with an HTTP/2
  fallback.

