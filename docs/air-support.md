# Air support

BB-119. Research and proposed design; nothing here is built yet. The open
questions for Oto are in §9. Until they are answered, the "Proposal" lines are
recommendations and not decisions.

The issue: a supply crate shows a helicopter or a plane. A seat that picks it
up starts a 15 s countdown, shown in a small box under the HUD's ammo section.
At zero an aircraft flies in: an Apache-like helicopter or an A-10
Warthog-like plane, drawn about 10% bigger than a tank. It drops its payload
on enemies near the seat and flies away. A marker on the map shows where it is.

Most of this already exists. The rod from god (docs/rod-from-god.md) has a
countdown on the round clock, a call carried whole on the wire, a minimap mark
and an off-screen arrow. The FPV swarm (docs/fpv-swarm.md) has flying hecs
entities with ground + height, a shadow, and the air-target list. Missiles
have `side_blast`, the side-aware area hit. `TowerPack` and `FrogHealth` are
crates that are not a tank weapon. What is new: two aircraft, the call that
belongs to a seat, the HUD box, and the sprites.

## 1. How it plays (proposal)

- **The crate.** Two kinds, `PickupKind::AirHeli` and `PickupKind::AirJet`
  (`pickup = "air_heli"` / `"air_jet"`). Each shows its aircraft's
  silhouette, so a player can see which one is coming. Only seats take
  them: `Tank::wants_pickup` returns `false` for enemies, as it does for
  Flamethrower, HeatShield and Grenades. Neither is a weapon
  (`PickupKind::weapon()` is `None`), so collecting one does not replace the
  special weapon the seat carries.
- **The call.** Collecting the crate calls the aircraft. The seat's HUD
  shows a small box with the aircraft's glyph and a count from 15
  (`air_support_delay_seconds`). Calls stack as a queue: while a call is
  running, a second crate queues another sortie, which starts its own count
  when the first aircraft leaves (§9 Q4).
- **The arrival.** At zero the aircraft enters from the field edge
  nearest a point behind the caller (hashed from the call id, so not random
  and not RNG), flying towards the caller. A short "AIR SUPPORT INBOUND"
  line shows in the prompt slot.
- **The target picture.** It is taken when the aircraft arrives, not when the
  crate is collected. The targets are enemies inside the caller's sight box
  (`ai::in_sight_box`, the screen the follow camera shows round the seat),
  nearest first, ties broken by slot. If the caller is wrecked, the strike
  centres on where the wreck is. No enemies in the box: the aircraft makes
  one pass over the caller and leaves without firing, and the crate is spent.
- **Helicopter (Apache).** Slow, and it hovers. It flies to a standoff
  point `air_heli_standoff_cells` from the first target. It turns to face
  each of up to `air_heli_targets` (3) targets in turn and fires a pair of
  rockets at each. The rockets are missile-like projectiles, each ending in
  a `side_blast`. Then it climbs out the way it came. On the field for about
  6 s.
- **Plane (Warthog).** Fast, in a straight line. It lines up through the
  densest cluster of targets in the box: the line from the entry point to the
  centroid of the nearest three. It makes one gun run, a stripe of
  `air_jet_gun_bursts` small blasts along the ground track ahead of it
  (GAU-8 BRRRT). It drops two bombs at the centroid as it passes, then
  leaves off the far edge. On the field for about 3 s.
- **Friendly fire.** None. Every hit goes through
  `side_blast`/`explosion_hit`, which skip `Owner::same_side`, so seats,
  friendly frogs and the seat's towers are safe. Tiles do take damage
  (`damage_obstacle` with falloff), as they do for missiles.
- **Counterplay.** The aircraft is not a ground body: nothing collides
  with it, and it ignores the nav grid and physics. Whether enemies can
  shoot it down is §9 Q3. Proposal: the helicopter is an air target (only
  bullets hit it, and enemies only from inside the caller's sight box, as
  for the FPV swarm), and the plane is too fast to hit.
- **The marker.** The minimap shows the aircraft as a small glyph in the
  caller's team colour, and the strike point blinks while the call counts
  down. The indicators show an off-screen arrow to an inbound or active
  aircraft (a new `ArrowKind::Aircraft`). The minimap is never drawn on
  phones, so there the arrow is the only marker. The aircraft is also drawn
  on the field (§5).

## 2. Where it lives

This follows the special weapons' split:

- `src/air_support.rs`, the headless half: `AirCall`, `Aircraft`, `Sortie`
  (Heli/Jet), the stage machine (`Inbound`, `Attack`, `Egress`), pure
  flight (`Aircraft::advance`), target and approach geometry, the
  `compose_*` drawing that is generic over `canvas::Canvas` (the shadow,
  the strike marker), and its tuning group.
- `src/simulation/air_support.rs`, the world half: `call_air_support` (from
  `pickup_phase`), `tick_air_calls` (arrivals), `fly_aircraft`
  (steering, firing), `air_support_show` (the cosmetics), and the senses if
  enemies get a reaction.
- `src/render/air_support.rs`: the sprite blit and the tinted-silhouette
  shadow (docs/sprite-shadows-design.md; the shape of `render/missile.rs`).
- `src/simulation/air_support_tests.rs`, plus `mechanics_tests` cases for
  every promise in §1.

### State

- `Game::air_calls: Vec<AirCall>`, sorted by id: `{ id, seat: u8, sortie,
  arrive_at: Option<f32> }`. These are round-clock times like `Zone::until`, not a
  timer on the `Tank`. Tank timers go on the wire only as flag bits, and
  `net/apply.rs` `set_timer` restarts them at full on a client, which is
  wrong for a 15 s count a player watches. A queued call has
  `arrive_at = None` until the previous sortie of that seat leaves (if Q4
  goes the queue way).
- `Aircraft` is a hecs component (spawned alone, like `fpv::Drone`):
  `{ id, owner: Owner::Player(seat), sortie, stage, ground: Position, height,
  heading, speed, targets: SmallVec<slot>, shots_left, stage_time }`. It is
  drawn at `ground` lifted by `height`, with the shadow at `ground`.
- Ids come from `Game::take_shot_id()`.

### Update phases

`pickup_phase` → `call_air_support` (pushes `Event::AirSupportCalled`).
Then, beside `place_calls` in `spawn_pending`, `tick_air_calls` spawns the
`Aircraft` whose `arrive_at <= time` (`Event::AirSupportArrived`). Its
rockets are pushed as pending missiles with `Owner::Player(seat)`.
`fly_aircraft` runs in `step_world`'s fixed-dt loop beside
`advance_drones`. The jet's gun stripes and bombs resolve beside
`resolve_missiles` (`Event::AirStrike`, a `side_blast` each, then
`air_support_show`). The end-screen branch repeats the cosmetic halves with
`live = false`, like every other weapon.

## 3. Pieces it reuses

| Need | Existing piece | Where |
|---|---|---|
| Crate kind, sprite row, glyph | `PickupKind` (+ `ALL`, `row`, `name`/`parse`, `weapon`, `ink`, `cooks_off`, all exhaustive) | `src/pickup.rs:23-325` |
| Seat-only crate | `Tank::wants_pickup` → `false` | `src/tank.rs:1271` |
| Effect outside the tank | the `TowerPack`/`FrogHealth` arms after the tank borrow | `simulation/mod.rs` `pickup_phase` (~3330-3400) |
| Clock-based countdown on the wire | `Zone::until` pattern, `ZoneState.until` in ticks | `src/zone.rs`, `net/wire.rs:826` |
| Flying entity | `fpv::Drone` (ground + height, `advance`, `clamp_inside`) | `src/fpv.rs:141-436`, `simulation/fpv.rs` |
| Shoot-down | `AirTarget`, `AirKey`, `Game::strike_air` (one mutator) | `src/air.rs:38-124`, `simulation/fpv.rs:412` |
| Side-aware area damage | `side_blast`, `BlastParams` | `simulation/missiles.rs:166`, `combat.rs:32` |
| Rocket | missile + `guide_missiles` target pick | `simulation/missiles.rs:57` |
| Enemies near a seat | `ai::in_sight_box`, sorted-by-slot pattern of `fpv::pick_nearest` | `src/ai.rs:3219`, `src/fpv.rs:516` |
| Cosmetics shared with replicas | `*_show(&mut Spectacle)` | `simulation/mod.rs:1570`, `net/apply.rs:420-440` |
| Shadow | tinted silhouette | `tank::draw_tank_shadow` (`tank.rs:3114`), `render/missile.rs:32-50` |
| Minimap mark | `Marks::gather` / `picture` (the rod's blinking frame) | `src/minimap.rs:606-771` |
| Off-screen arrow | `Scene::of`, `Awareness::frame`, `ArrowKind` | `src/indicators.rs:1012-1780` |

There is no shared "enemies near a seat, nearest first" helper; every weapon
writes its own. Write one here: `Game::enemies_near_seat(seat) ->
Vec<(slot, Position)>` (sight box, nearest first, ties to the lower slot).
The FPV and rod senses can move to it in a separate issue.

## 4. HUD: the countdown box

- `PlayerHud` (`src/hud.rs:185`) gets `air: Option<AirHud { sortie,
  seconds_left, queued }>`, gathered in `PlayerHud::gather` from
  `game.air_calls` for that seat. The seconds come from
  `time + zone_lead`, the client's present, as the rod's count does.
- **Placement.** The weapon column of the block's second row: x at
  `block.x + V_WEAPON` (108), `WEAPON_SLOT_W` (64) wide, one `ROW_H` (32)
  tall, top at `block.y + row_h + ROW_GAP`. That is directly under the ammo
  slot. The lamp row (`CornerShape::lamp_row()`, `hud.rs:815`) already opens
  this row for lanterns and the heat shield, and both use other columns
  (x 6..60 and `V_SHIELD`). Proposal: widen the condition to
  `lanterns || heat_shield || air_call` and add `Corners::air_box`, one
  rectangle that both the painter and the tests read (the one-geometry
  rule), pinned in the `corner_tests` shapes table.
- **Look.** A plate with the sortie's glyph from `Sheet::PickupGlyphs` and the
  whole seconds left (`ceil`). The last 3 s blink at `indicator_pulse_hz`.
  When the aircraft arrives the plate shows the glyph alone until it leaves.
  A queued sortie adds a small "+1".
- **Layouts.** `One` and `Two` (couch: each block has its own box). In
  `Compact` (online, or three or more seats) only the local seat's box is
  shown. Other seats' calls appear on the minimap only.
- **Text.** Digits are `format!`ed, like the weapon count. New
  `text::keys`: `HUD_AIR_INBOUND` (the prompt line), `PICKUP_AIR_HELI` /
  `PICKUP_AIR_JET` if the armory or editor names crates. Each key goes in
  `lang/en.ftl` and `lang/sl.ftl` with a budget, so
  `every_language_fits_every_budget` covers it.

## 5. Drawing and sprites

- **Size.** A tank's sprite cell is `TANK_SPRITE_SIZE` 40 design px at
  `Tank::scale` 2 = 80 px on the field, and its gameplay frame is 64 px.
  10% bigger: an **88 px** cell (44 design px). The plane is long and thin
  (wingspan 44, length ~40). The heli has a 44 px rotor disc over a ~36 px
  fuselage.
- **Sheet.** A new generator `tools/spritegen/gen_aircraft.py`, in the style
  of `gen_missiles.py` (fixed seed, `SPRITE_OUT=static`, its own
  `docs/AIRCRAFT_SPEC.md`), importing the tankdesign `kit.py` palette.
  `export.py` is built around the 33×60 tank layout, so the aircraft do not
  go through it. Frames: heli body, rotor in 4 rotation frames, tail rotor
  in 2, a damage overlay if Q3 makes it shootable; jet body, plus
  afterburner/engine glow for the glow pass. Top-down, rotation 0 = facing
  up like everything else. Run `just check-sheets` after.
- **Crate art.** Two new rows in `gen_crates.py` (`KINDS`, a design dict
  entry each, an ink in `punypalette.PICKUP_INK`). `docs/CRATES_SPEC.md` §1
  still says 280×640 / 16 rows; the real sheet is 280×760 with 19 rows. Fix
  the spec in the same change.
- **Paint order.** The shadow after the floor, like `paint_drone_shadows`.
  The aircraft over everything standing, tree crowns included (like
  `paint_drones`). Nav/landing lights in the glow pass, so it reads at night
  and in fog. The shadow's offset grows with `height`, as the missile's does.
- **Effects** (docs/effects.md): rocket trails and blasts reuse the
  missile's. The gun run is a row of small `BlastFx` + scorches, hashed from
  id and index. Rotor wash flattens grass under the heli
  (`Spectacle` grass flatten). Everything is cosmetic and hashed, with no RNG.

## 6. Online

- `PROTOCOL_VERSION` 28 → 29: new `PickupKind` variants shift its wire
  enum, and a new snapshot family is added.
- **Calls:** `Snapshot::air_calls: Vec<AirCallState { id, seat, sortie,
  arrive: u32 ticks }>`. Added/gone like zones, so a client's HUD counts
  against its own present. (Alternative: a `ZONE_AIR` kind on `ZoneState`.
  A zone has a centre and an AI `danger()` disc, which a countdown that has
  no target yet does not need. Proposal: its own small family.)
- **Aircraft:** `AircraftState { id u16, x, y, height, heading u8, sortie,
  stage, owner }` (quantised like `DroneState`), with encode, delta
  (added/moved/gone), interp (like drones) and apply arms, and a
  `DrawableState::aircraft`.
- **Events:** `AirSupportCalled`, `AirSupportArrived`, `AirStrike` are each
  mirrored in `WireEvent::from_event`, which is exhaustive. `AirStrike` runs
  `air_support_show` on the replica.
- Difficulty per seat count (`tuning_patch`) is untouched. A room of one
  stays byte-identical to the offline round.

## 7. Determinism

- The call, arrival, entry edge, flight and target pick draw **no RNG**:
  the entry side is hashed from the call id, and targets are sorted by
  distance then slot.
- The only draws are `explosion_hit`'s damage rolls, which happen only in
  a round where a seat took the crate. A round without the crate keeps its
  stream, so `the_one_and_two_seat_streams_are_pinned` stays as it is.
- If air crates join the random bonus rolls (Q5), the gate is checked
  **before** the chance draw (the frog-pack rule), so maps without them
  keep their stream.
- Transcendentals (heading, orbit) go through `math::`.
- Walks go seats in index order, then enemies.

## 8. Tests and probe

- `mechanics_tests`:
  - an enemy does not take the crate;
  - the special weapon a seat carries survives the pickup;
  - nothing arrives before 15 s and it arrives at 15 s;
  - only enemies inside the caller's sight box are struck;
  - no seat, friendly frog or friendly tower takes damage;
  - with no enemies in the box, nothing fires and the crate is spent;
  - a wrecked caller's strike still lands;
  - a queued sortie waits for the first one.
- `air_support_tests`: the stage machine, the entry edge, the jet's run line
  through the centroid, heli standoff.
- `determinism_tests`: a seeded round with the crate replays bit for bit.
- `corner_tests`: the `air_box` rectangle in every shape.
- The net tests: the round trip of `AirCallState`/`AircraftState`, and the
  `NOT_SENT` list.
- Probe: the shared `--crate air_heli|air_jet` path, plus a map fixture in
  `maps/test/` with an air crate next to the seat and a pack of enemies, run
  under `just probe-fixtures`.
- Dev server: a `debug_call_air_support {seat, sortie}` mutator for
  screenshots. Rendering is checked by eye on a screenshot.

## 9. Open questions for Oto

1. **One crate or two?** Two kinds (heli crate, jet crate), so the symbol
   says which aircraft comes (proposal). Or one "air support" crate that
   picks heli or jet (hashed from the slot).
2. **Payloads.** Is the proposal right: heli = rockets at up to 3 targets
   with a hover, jet = one gun run + two bombs on the cluster? Should a
   single sortie be able to wreck a full-health tank, or only soften it?
3. **Can it be shot down?** Proposal: heli yes (an air target, so enemy
   bullets and towers can hit it), jet no. Or both invulnerable for v1.
4. **A second crate while counting.** Queue a second sortie (proposal),
   restart the count, or the crate stays on the ground (`wants_pickup` false
   while a call runs)?
5. **Where it spawns.** Only where a map places it (`pickup = "air_heli"`
   in the editor's crate tool), or also as a rare random bonus roll? And
   should enemies ever get their own air support (enemy-owned sorties
   against seats)? Proposal: map slots only, seats only.
6. **"Marker on map".** The minimap mark plus an off-screen arrow
   (proposal), or also a target marker drawn on the field where it will
   strike?
7. **Enemy reaction.** Should enemies scatter from an inbound strike, the
   way they flee the rod's zone? Proposal: no for v1. Enemies do not see
   the aircraft coming.
