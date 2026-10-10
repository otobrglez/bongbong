# Mission maps: drive across the field

Status: **draft, 2026-10-10** (BB-72; this document is BB-73). Nothing is
implemented. Section 13 lists the decisions still to make; the rest is the
proposal they are written against, with a recommendation at each fork.

## 1. Purpose

Every level today is fought in one place: an arena, or a field map's fort
that the waves come to. A **mission map** is a journey instead. The seats
start at one end of a long map and have to reach the other, and the fight
is whatever stands or turns up on the way:

- a **garrison** that sleeps until the seats come near, then wakes;
- an **ambush** that rolls in through the field's edge when a seat reaches
  a spot;
- a **fortification** of enemy towers guarding a choke point;
- a **blockade**, a door across the road that opens once its guards are down;
- a **hazard**: a drum depot, an oil trail, a ford, a volcano, the weather.

The round is won when a seat's hull crosses the **exit**, and lost when
every seat is wrecked and no lives are left. On a wreck the seat comes back
at the last **checkpoint** passed, for as long as the team's lives last.

What a player sees: the banner (`BREAK THROUGH!`), the road heading east,
the exit's arrow on the edge of the screen, an outpost's tanks starting to
move as the hull comes over the rise, the amber gate arrows flashing on the
flank as an ambush rolls in, `CHECKPOINT` once a blockade falls, the lives
beside the mission word, and the end screen the moment the hull crosses
the exit line.

## 2. What is already built

Boot Camp (docs/training-stage.md) is this genre in all but name: a
60 x 24 field map with one road, a script of beats whose triggers roll
tanks in and open doors, and a comeback at the last door opened. It is
written as training (never lost, one seat, the TRAINING words, the frog's
voice), and that is most of what has to change.

| A mission map needs | What exists | Where |
|---|---|---|
| A long map the camera follows | Field maps up to 250 cells a side: the follow camera, the couch split, off-screen arrows, the minimap, the establishing shot | `framing.rs`, `follow.rs`, `indicators.rs`, `minimap.rs`, `establish.rs`; docs/large-maps-follow-camera.md |
| Enemies that wait for the seats | A far enemy nothing has woken does not think at all; a seat within `enemy_far_px` (1200 px), a hit or an alert wakes it. Each keeps within `enemy_leash_px` (640 px) of its home, the spot it first stood on, and alerts pass along a chain (`enemy_alert_chain_px`, 800 px) with no line of sight | `simulation/field.rs`, `ai.rs` (`FieldMind`, `Brain::home_leash`) |
| Fair fire | An enemy fires at a seat only from inside that seat's sight box (+-368 x +-240 px) | `ai::in_sight_box` |
| "When this happens, do that" | The beat script: triggers `flags`, `collect`, `destroyed`, `destroyed_all`, `past_col`, `frog_full`, `wrecks`; starts `roll_in`, `drop`, `shoot_frog`. No RNG of its own | `training.rs`, `simulation/training.rs` |
| Blockades and checkpoints | `door` cells that open when their beat is done; a wrecked seat comes back after `training_respawn_seconds` at the middle of the last door opened | `simulation/training.rs` (`open_doors`, `bring_back_seats`, `last_door_opened`) |
| Tanks arriving | The roll-in: gates, `RollIn`, the `Rejoin` role, `free_lanes`, the `spawn_in_band` fallback when a lane stays busy, the chassis tiers, enemy wrecks fading so a lane never stays blocked | `simulation/waves.rs` (`training_roll_in`, `pick_gate`), `level.rs` (`Tier`) |
| Fixed defences | Gun tower, tesla and bio slush with `side = "enemy"` | `towers.rs`; docs/defence-towers-prd.md |
| Set pieces | Drums and oil trails, fences, sandbags, water and fords, lava, volcanoes, portals, lamps, the weather | `props.rs`, `ground.rs`, `volcano.rs`, `weather.rs` |
| Being a level | `levels.toml`, the opening banner and title, the end screen, the level select, progress | `levels.rs`, `mode.rs`; docs/levels.md |
| Checking it | The probe, `mechanics_tests`, the map linter (which already judges a course with every door open, `Game::open_every_door`), mapshot thumbnails | `bin/probe.rs`, `maplint.rs`; docs/tooling.md |

## 3. The mission

```rust
pub enum Mission { Protect, Hunt, Destroy, Breakthrough }
```

| Mission | Frogs | Lost when | Won when | Banner |
|---|---|---|---|---|
| Breakthrough | none | every seat a wreck with no life left | a live seat's hull reaches an exit cell | `BREAK THROUGH!` |

- **The mission rule stands on its own.** A breakthrough map with no script
  plays: placed garrisons, towers and a road. A script plays under any
  mission: a Protect field map can use one for its set pieces. Only a
  `[training]` script makes a round a training round (section 4.1).
- Losing takes precedence over winning on the same frame, as it does today
  (`check_round_end`).
- No frog. The escort variant, where the players' frog walks the route, is
  section 10.
- `Mission::name` is `breakthrough`; `has_player_frog` and `has_enemy_frog`
  are false. The variant goes at the end of the enum so the existing
  discriminants keep their order, but `Mission` travels on the wire (a
  room's `Create` message, `WireOverrides`), so this is still a
  `PROTOCOL_VERSION` bump.

### 3.1 The exit

`kind = "exit"`, multi-instance: a run of exit cells is one finish line,
and a map may have more than one (two roads in). Not solid and **not an
`Obstacle`**, for the same reason as tall grass and flags: the nav grid and
the linter treat every obstacle as a wall. A seat reaches it when its hull
box, grown by `mission_exit_reach_px`, touches an exit cell (the flag's
test, `take_flags`). Reaching it pushes `Event::ExitReached { seat }`.

Drawn generic over `canvas::Canvas` (as `training::draw_flag` is), so the
CPU thumbnails show it: a chequered band of whole 2 px blocks across the
cell in two palette steps (docs/PALETTE.md), dark and light, with a pennant
at each end of the run. The off-screen arrows gain `ArrowKind::Exit`, never
merged and never left out (like a teammate's), and the minimap marks the
exit cells.

### 3.2 Garrisons: enemies placed on the map

`kind = "enemy", tank = "longbow"`. Optional keys: `ai` (as a roll-in's),
`facing` (`north`/`east`/`south`/`west`, default facing the start).

- **Placed at `init`** after the seats and before the band, in cell order
  (`iter_cells` sorts), each with the next owner slot and the same rolls a
  band tank gets (`roll_enemy_tank`: weapon, shield). A map with no enemy
  cells draws nothing new, so every existing replay stays as it is.
- **On a field map they are the field's enemies with an authored home:**
  they sleep until a seat is within `enemy_far_px` (about 37 cells), keep
  within `enemy_leash_px` (20 cells) of their cell, chain their alert
  `enemy_alert_chain_px` (25 cells) and fire only from inside a sight box.
  None of that is new code; the cell only decides where home is.
- **On an arena** they are band tanks at fixed spots, under the arena's
  shared alert.
- **`tanks` still adds band tanks** on top; a mission map writes
  `tanks = 0`.
- **They count against `wave_max_alive`** (31 live enemies, garrisons and
  roll-ins together). A garrison asleep is still live; the linter warns
  when the placed count leaves too little room for the script's roll-ins.
- **Wrecks fade** after `mission_wreck_seconds`, longer than training's 4 s
  but not forever: a wreck has a body, and on a two-lane road a garrison's
  wrecks would close it. The scorch decals stay.

Held-back garrisons (`beat = N` on an enemy cell, placed only once beat N
starts and only while the cell is outside every seat's sight box) are left
open (section 13, item 6).

### 3.3 Named gates

`kind = "gate", id = "north-bridge"`. Gate cells sharing an id are one
group of lanes. A `roll_in` that names a gate rolls in through a free lane
of that group: the same `free_lanes` test, the lane marked used, the round
RNG drawn only to choose among more than one free lane. A named roll-in
skips `field::prefer_gates`, because the author has already chosen. A
roll-in that names none picks as today. Gates still sit on the map's edge;
on a strip 24 to 32 rows tall the north and south edges run the whole
route, so a flank gate beside any trigger box is always available, and a
tank rolling in from row 0 toward a seat on row 14 is about 450 px off,
outside the sight box's 240 px.

The amber gate arrow (`ArrowKind::Gate`) and the minimap's gate flash
already telegraph a roll-in, so an ambush announces itself for free.

## 4. The script

### 4.1 Training and missions share one engine

`MapFile` gains `script: Option<Script>` beside `training`. Both parse
into the same type (`Training` renamed `Script`) and run on the same beat
runner. What is training-only stays keyed to the `[training]` table: never
lost, the frog's voice and walk, one seat, the TRAINING words, the held
shell refill. A `[script]` map keeps every seat the round has, can be lost
by its mission, and is hostable once section 8's online step lands
(unhostable until then). A map with both tables is a parse error.

Boot Camp's file and every player map in the map store parse as before,
and Boot Camp replays as before, beat for beat (`simulation::training`'s
tests). One table with a `kind` key is the alternative (section 13, item 5).

Beats stay **sequential**, the route's spine: a segment of the road is a
beat, and a door is the blockade between two of them. Skipping ahead is
harmless because the spatial triggers latch (below).

### 4.2 New triggers (`done`)

- **`reach = [c0, r0, c1, r1]`**: a live seat's hull centre has been inside
  this cell box *at any time this round*. Latched like `flags`, so a seat
  that races past an ambush's box before its beat comes up still sets it
  off the moment the beat starts. Any seat counts, on the couch and in a
  room alike.
- **`cleared = [c0, r0, c1, r1]`**: no live enemy, asleep or awake, stands
  inside the box. "The guards are down" in any order and at any time,
  which `wrecks` (counted from the beat's start) cannot say.
- **`after = 12.0`**: seconds since the beat began. Alone it is a timed
  beat ("hold here"); with others it is a minimum.
- `past_col` stays, for Boot Camp.

### 4.3 New starts and beat fields

- **`roll_in[].gate = "<id>"`**: section 3.3.
- **`wake = [c0, r0, c1, r1]`**: every enemy inside the box is alerted at
  once, as a hit would: the base hears the shot.
- **`checkpoint = [c, r]`** on a beat: once this beat is done, a wrecked
  seat comes back here. Without it, the middle cell of the last door
  opened, as in training, else the seat's own start.
  `Event::CheckpointReached { beat }`, and a short `CHECKPOINT` banner.

### 4.4 Lives and coming back

- `script.lives = 3` is a team pool; absent, the comeback is unlimited.
  `lives = 0` means no comeback at all: a wreck is out, as on a band level.
- A wrecked seat waits `mission_respawn_seconds`, spends a life and comes
  back at the checkpoint as a fresh tank of its own chassis, keeping its
  shells: training's `respawn_seat`. `Event::LifeSpent { seat, left }`.
- With no life left a wreck stays, and once every seat is a wreck the
  round is lost.
- A seat comes back wherever the checkpoint is, enemies or not, so a
  checkpoint belongs on ground the fight has left behind; the linter warns
  when a checkpoint cell lies inside a garrison's leash.
- A lost level offers `PLAY AGAIN` from the start, not from the
  checkpoint (section 13, item 8).

### 4.5 Knobs and events

A `mission` tuning group in `tunables!`: `mission_respawn_seconds`,
`mission_wreck_seconds`, `mission_exit_reach_px`,
`mission_checkpoint_banner_seconds`. The new events (`ExitReached`,
`CheckpointReached`, `LifeSpent`) go on `NOT_SENT` with the training events
until the online step: `WireEvent::from_event` is an exhaustive match, so
the build fails until each is placed.

## 5. The map file

```toml
# a strip 128 x 28: four segments east along one road (section 9)
version = 1
size = [128, 28]
tanks = 0
tank = "assault"
mission.kind = "breakthrough"
spawn.kind = "band"

cells."3,14"    = { kind = "start" }
cells."3,16"    = { kind = "start2" }
cells."125,13"  = { kind = "exit" }
cells."125,14"  = { kind = "exit" }
cells."125,15"  = { kind = "exit" }

# segment 2: an outpost by the bridge
cells."34,9"    = { kind = "enemy", tank = "longbow" }
cells."37,18"   = { kind = "enemy", tank = "scout", facing = "west" }
cells."40,11"   = { kind = "gun_tower", side = "enemy" }

# segment 3: the flank gate of the ambush
cells."58,0"    = { kind = "gate", id = "north-bridge" }
cells."59,0"    = { kind = "gate", id = "north-bridge" }

# the blockade between segments 3 and 4
cells."68,13"   = { kind = "door", beat = 2 }
cells."68,14"   = { kind = "door", beat = 2 }
cells."68,15"   = { kind = "door", beat = 2 }

[script]
lives = 3

[[script.beat]]
id = "bridge"
done = { reach = [52, 0, 62, 27] }

[[script.beat]]
id = "ambush"
start = { roll_in = [{ tank = "scout", gate = "north-bridge" },
                     { tank = "assault", gate = "north-bridge", after = 2.0 }] }
done = { cleared = [52, 0, 67, 27] }
checkpoint = [70, 14]

[[script.beat]]
id = "depot"
start = { wake = [80, 0, 100, 27] }
done = { reach = [100, 0, 108, 27] }
```

The level tables as dotted keys (CLAUDE.md, `map.rs`): a `[mission]` or
`[spawn]` header would swallow every later `cells.` line. The `[script]`
tables come after the cells and are written last, as `[training]` is.

## 6. The rest of the game

**Words** (`text::keys`, `lang/en.ftl` and `lang/sl.ftl`, held by
`every_language_fits_every_budget`): `mission-breakthrough` (BREAKTHROUGH;
sl PREBOJ), `mission-breakthrough-banner` (BREAK THROUGH!; sl PREBIJ SE!),
`banner-checkpoint` (CHECKPOINT; sl KONTROLNA TOČKA, which may need
shortening to fit), the lives readout. BREAKTHROUGH is twelve letters in
the HUD's mission title box, which also has to hold the lives now; if it
does not fit, the HUD word is the first thing to change (section 13,
item 1).

**HUD** (`hud.rs`; docs/hud-and-builder-layout-design.md): the lives as a
count beside the mission word in the info cluster, in `corners()` so the
painter and the hit tests read one geometry.

**Camera**: the establishing shot draws the whole field through one
texture at most `establish::MAX_TEXELS` (4096) a side, so a field over 128
cells long opens on the follow view with no shot. Either a mission map
keeps to 128 columns, which is a 4,096 px route and about 20 s of straight
driving, or the shot learns to take a long map at a lower texel rate, or
it becomes a pan along the road to the exit and back. The last suits a
mission best and is the most work.

**Minimap**: one texel per cell, so a 128 x 28 strip is a thin bar under
the corner cluster; check its fit at phone-tablet sizes before settling on
strip proportions. Phones have no minimap, which leaves the exit arrow.

**The couch**: two seats as in any round. `reach` takes either seat, the
lives are shared, and each seat comes back on its own. The seat-count cap
in `Game::init` and the hidden players button key off `[training]` only.

**Thumbnails**: an exit or enemy cell on a shipped map changes its CPU
render hash; re-pin it in the same change.

## 7. Builder, linter, probe, tests

**Builder** (docs/game-editor-fusion.md, docs/map-editor-design.md). First
version: the script is written by hand; the builder draws doors and flags
already and round-trips the tables on a re-save (dropping header comments,
as always). Then: tools for the exit (a run), the enemy (with the chassis
picker the start uses), gate ids (named by letter, A, B, ...), a door's
beat; a script panel that lists the beats and outlines each beat's boxes
and checkpoint on the canvas. **PLAY HERE** on a mission map should start
at the nearest checkpoint behind the cursor with every earlier beat done
and its doors open, so a level is tested a segment at a time.

**Linter** (`maplint.rs`):

- error: a breakthrough map with no exit; no route from the start to an
  exit with every door open; a named gate no cell has; a `checkpoint`,
  `reach` or `cleared` box off the field or with no open cell; more
  garrisons than `wave_max_alive`;
- warning: an exit on a map whose mission is not breakthrough; a garrison
  inside the start's sight box; a checkpoint inside a garrison's leash; a
  pickup an enemy wants within a garrison's leash (Longwater's lesson:
  the garrison leaves its post to fetch it); a `cleared` box no enemy can
  ever stand in;
- info: garrisons within `enemy_alert_chain_px` of one another, grouped,
  so the author sees which outposts wake together.

**Probe**: a `route` scenario where the seat drives the nav route to the
nearest exit (`Grid::next_step`, as the frog walks) and fires when lined
up, reporting the time to the exit, wrecks per beat and where it stalled,
with `ANOMALY kind=stall` and `kind=never-reached`. A `route-defend`
variant adds the perfect defence of `--scenario defend`, to read the
pacing alone (the time per segment and the lulls). Fixtures in
`maps/test/` (a 60 x 20 strip with one of each encounter), recorded
ceilings, `just probe-missions`.

**Tests** (`mechanics_tests`, `simulation::training`'s tests, the beat
runner's own):

- a garrison sleeps until a seat comes within `enemy_far_px`, then wakes
  and keeps to its leash;
- a `reach` box crossed before its beat still fires that beat once it
  comes up;
- a named roll-in comes through only that gate;
- a `cleared` box opens its door whichever guard goes last;
- a wreck comes back at the checkpoint and spends a life;
- a wreck with no life left loses the round;
- reaching the exit wins it, and a loss on the same frame beats the win;
- Boot Camp plays through as before, and a map with neither new cells nor
  a script replays bit for bit (`determinism_tests`'
  `the_one_and_two_seat_streams_are_pinned` stays as it is).

## 8. Online

Rooms come last. `MapFile::hostable` refuses a scripted map until then.
Hosting one means:

- mirroring `BeatDone`, `DoorOpened`, `CheckpointReached`, `ExitReached`
  and `LifeSpent` as `WireEvent`s;
- the round state carrying the beat, the lives and the latched boxes;
- a replica that opens doors and shows checkpoints from those, never by
  running the beats itself;
- a `PROTOCOL_VERSION` bump.

Difficulty follows the seat count as every room's does (`tuning_patch`),
and a room of one plays the offline round down to the bytes. The authority's
`room_events` on the room server's dev surface is where a co-op beat is
checked.

## 9. Laying out a mission map

**The numbers to lay out against.**

| Measure | Value | In cells |
|---|---|---|
| A monitor's follow view | about 1280 x 720 px | 40 x 22.5 |
| A seat's sight box | +-368 x +-240 px | 23 x 15 |
| A garrison wakes | `enemy_far_px` 1200 px | 37 |
| Alerts chain | `enemy_alert_chain_px` 800 px | 25 |
| A garrison's leash | `enemy_leash_px` 640 px | 20 |
| Player top speed | `tank_speed` 210 px/s | 6.6 a second |
| Enemy speed | `enemy_speed` 160 px/s | 5 a second |

A 128-column strip is 4,096 px, about 20 s at full speed with nothing in
the way, so the encounters set how long the level takes. Aim for 3 to 6
minutes in four or five segments.

**Shape.** A strip, 100 to 128 long by 24 to 32 (longer once the
establishing shot can take a long map, section 6), or a road that switches
back across a squarer map, 100 x 70 for example. A switchback gives more flank edges and better landmarks but
a harder route for the AI. Keep corridors at least 2 cells wide (the
pathfinder's rule) and 3 where hulls meet, since a turning hull slides
26 to 75 px past its turn.

**Segments, a string of pearls.** Each segment is about a screen, 25 to 40
columns: a stretch of road, then a pocket where something happens, then a
breather with a crate or two. Each segment gets its own encounter, mixed
from garrison, ambush, fortification, blockade, gauntlet (towers along a
road you have to run) and hazard. The difficulty climbs across segments,
and the last is a set piece, a fort before the exit.

**Spacing.**

- Garrisons sit more than 25 cells apart unless they are meant to wake
  together.
- An ambush's gate is on the flank edge level with or ahead of its `reach`
  box, so the tanks come in front of the seat.
- What an enemy would turn off its road for (laser, plasma, minigun,
  missiles, speed, shield, health) lies beyond every garrison's leash.
- A checkpoint follows every blockade, roughly every 60 to 90 s of play,
  on ground no garrison's leash reaches.

**Pacing by hand.** The wave director's cycle is the template: build-up
(the road in, the first sight of the outpost), peak (the fight), relax
(the breather pocket). Do not put two peaks back to back without a
breather; a mission map has no director to stretch one.

**Reading the map.** A screen has to say where you are. Give each segment
its own palette of materials and one landmark (a bridge, a mill, a depot,
a town square) on the minimap, use roads and rivers as the route's edges,
and let the exit be visible from the last segment's entrance.

## 10. Variants for later

- **Escort**: Protect on a route. The players' frog walks the road
  (`walk_frog` already routes it along the nav cache and waits at a closed
  door), the beats follow the frog's progress, and the round is lost if the
  frog dies.
- **Raid**: reach a `cleared` objective at the far end, then come back to
  an exit by the start.
- **Hunt at the end**: an enemy frog in the fort at the far end. This is
  playable today as a Hunt map shaped as a strip, though the hunters
  roll for the players' frog left behind at the start.
- **Against the clock**: a beat with `after` as a limit that loses the
  round when it runs out. This needs a lose rule the script does not have.

## 11. Prototype before building

Two ways to play-test a layout today, no code:

1. **A training script on a strip with `mission.kind = "destroy"`**: beats,
   doors and roll-ins run, with no frog. But the round cannot be lost, it
   says TRAINING and FOLLOW THE FROG!, it seats one, and the game picks
   the gates. Good enough for the layout and the pacing. Unverified: no
   shipped map combines a script with Destroy.
2. **A Hunt map shaped as a strip**: band tanks placed out of sight by
   walk (`field::spawn_cells`) and asleep until woken, the enemy frog at
   the far end. Good for reading how difficult the road is.

## 12. Build order

Each step leaves the game playable and the tests green, and updates
`docs/codebase.md`'s entries and this document's status.

1. **One engine, two tables**: `[script]` beside `[training]`, `Training`
   renamed `Script`, training-only rules keyed to `[training]`. Boot Camp
   unchanged; scripted maps unhostable.
2. **The mission**: `Mission::Breakthrough`, the `exit` cell and its
   drawing, the round's end, the words, `ArrowKind::Exit`, the minimap
   mark. Playable as a strip of towers and band tanks.
3. **Garrisons**: the `enemy` cell, placed at `init`, wrecks fading.
4. **The script's new triggers and starts**: latched `reach`, `cleared`,
   `after`, named gates, `wake`, `checkpoint`.
5. **Lives and comebacks**, and their HUD readout.
6. **Linter rules, the probe's `route` scenarios, fixtures**, then the first
   shipped mission level. The level select holds sixteen tiles
   (`SELECT_TILES`) and `levels.toml` fills them, so the first new level
   needs a second page or takes an existing level's place.
7. **The builder**: the new tools, the script panel, PLAY HERE from a
   checkpoint.
8. **Online**: section 8.

## 13. Open decisions

| # | Question | Recommendation |
|---|---|---|
| 1 | The mission's name, data and words | `breakthrough`, BREAKTHROUGH, BREAK THROUGH!; ADVANCE if the HUD box is too narrow (avoiding `advance` as the data name, which the probe's scenario already uses) |
| 2 | Lives | A team pool of 3 with checkpoint comebacks; `lives = 0` for a map that wants one-wreck arcade rules |
| 3 | A co-op win | Any live seat reaching the exit |
| 4 | The script's shape | Sequential beats with latched spatial triggers (Boot Camp's model); independent once-only triggers only if a map needs side events |
| 5 | The table | `[script]` beside `[training]`, so Boot Camp and saved maps stay untouched; one table with a `kind` key is the alternative |
| 6 | Held-back garrisons (`beat = N` on an enemy cell) | Out of the first version; roll-ins cover reinforcements |
| 7 | The first shipped mission level | A second page of the level select rather than dropping a level |
| 8 | A lost level restarts from | The start (arcade); a checkpoint restart is a later option |
| 9 | Long maps and the establishing shot | Keep the first mission maps to 128 columns; the pan along the route as a follow-up |
| 10 | Online in the first version | No: solo and the couch first, rooms as step 8 |
