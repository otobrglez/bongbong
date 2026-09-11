# PRD: the flamethrower

Status: agreed with the author 2026-09 (the interview answers are in section
2), mechanics sketched in the "Flame Lab" artifact from the session that
wrote this, **implemented on the same branch** (`simulation/flame.rs`, tests
in `simulation/flame_tests.rs`). Section 14 lists where the build departs
from the text below. Builds on the
ground-fire system the barrel work added (docs/barrel-explosion-variety.md
section B1, `simulation/props.rs`'s `tick_fires`), which is what makes a
flamethrower that burns the *environment* a weapon rather than a particle
effect.

Contents

1. Why
2. The interview: what was decided
3. The weapon
4. What the stream does to a tank
5. What the stream does to the world
6. What the fire leaves behind
7. Enemies, risk and the AI
8. Presentation
9. HUD, pickup, map and tools
10. Knobs
11. Tests
12. Non-goals and open questions
13. Phases

## 1. Why

Every weapon in the roster fires a projectile at a point: shells, minigun
bullets, plasma bolts and the instant laser all resolve as one hit on one
target. None of them changes the battlefield. The barrel work gave the game
fire as a *state* - a burning cell hurts whatever drives over it, lights
what stands beside it and blocks the AI's nav grid while it burns - but the
only way to start one is to find a drum. The flamethrower is the first
weapon the player *aims at the ground*: it clears cover, fences off a
corridor, drives enemies out of a cluster and sets the map's own props
against them. It is also the first weapon that can hurt its user.

## 2. The interview: what was decided

| Question | Decision |
|---|---|
| How is it obtained and spent? | **A pickup with a fuel tank.** A new pickup like the laser or minigun; it grants a tank of fuel measured in seconds of burn. Holding fire drains it; the slot empties when it runs out. |
| What does the stream do to a tank? | **A short cone with burn-over-time.** About two and a half cells long, roughly a cell wide at the end. A tank inside takes steady damage and keeps burning for a couple of seconds after it leaves the stream, so a touch is enough. |
| What does it set alight? | **All of it.** The ground under the stream burns like an oil pool; tall grass in a burning cell chars and is gone for the round; wood, trees, oil trails and drums ignite; sandbags and fences take burn damage and collapse; and walls and ground keep burn marks the way the barrel mechanics leave them. |
| Enemies and risk? | **Player only, fire is neutral.** Only the player finds the pickup. Ground fire it starts burns anyone, the player included. |

## 3. The weapon

- **Slot and queue.** `ActiveWeapon::Flamethrower`, granted by
  `PickupKind::Flamethrower`. It joins `Tank::weapon_queue` through
  `enqueue_weapon` like the other three, so the FIFO rule holds: a pickup
  collected while the minigun is live waits its turn.
- **Fuel.** `Tank::flame_fuel: f32`, seconds of burn. A pickup adds
  `flame_fuel_per_pickup` (9 s); a second pickup stacks. The weapon counts
  as stocked while `flame_fuel > 0` (`weapon_ammo` reports the fuel
  rounded up, so the last fraction of a second still fires).
- **Trigger.** Full-auto while held, like the laser and minigun, but with
  no `fire_cooldown` pacing: every frame the key is held and fuel remains,
  the tank emits a `FlameJet` for that frame and burns `dt` of fuel. A
  twin-barrel chassis has one nozzle; the flamethrower ignores
  `tank_barrel_lateral_offset`.
- **The cone.** From the muzzle (the shell's own spawn point), along the
  facing, `flame_range` (164 px, five cells) long, half
  angle `flame_half_angle_deg` (21.4 degrees, about 64 px each side at the
  end). The centre line is swept through `Terrain::sweep` like a laser
  beam, and the first solid tile it meets caps the effective range for that
  frame - a stream does not pass through a wall, though it does reach the
  wall (section 5).
- **Fuel gauge.** The HUD slot shows whole seconds left (rounded up), in a
  fourth weapon slot with an orange accent.

## 4. What the stream does to a tank

- **In the cone.** Any live tank other than the shooter whose hull centre
  is inside the cone (distance along the facing within the effective range,
  lateral offset within the cone's half width at that distance) takes
  `flame_damage_per_second` (20) scaled by `dt`, through `take_damage` so
  a shield blocks it, and is `mark_hit` so its ring shows. An enemy is
  `notify_hit`, which alerts it exactly as a shell would. Damage is a
  fixed rate, not a roll: **the flamethrower draws no RNG at all.**
- **Afterburn.** Contact sets `Tank::burn_timer` to
  `flame_afterburn_seconds` (2.75). While it runs the tank takes
  `flame_afterburn_dps` (4) per second and draws burning. Re-entering the
  cone resets the timer rather than stacking it. A tank killed by
  afterburn goes on `Frame::kills` and explodes like any other kill.
- **Frogs.** Either frog inside the cone takes
  `flame_frog_damage_per_second` (10). It does not hop: a hop's landing
  spot is an RNG draw, and the weapon draws none (section 14).
- **The shooter.** Never damaged by its own stream. It *is* damaged by the
  ground fire the stream leaves, like everyone else (section 7).
- **Events.** `Event::Fired { weapon: "flamethrower" }` once per press, not
  per frame; `Event::Hit { target, damage: 0, killed }` the first frame a
  target enters the cone during a hold (the running damage is a rate, not
  a series of hits, so the log records contact); `Event::Ignited { x, y,
  what }` when the stream lights a cell or a tile.

## 5. What the stream does to the world

The stream heats what it touches. Everything below is driven by one
mechanism: **exposure**. Each frame, the cells along the centre line (one
sample per cell from a quarter cell past the muzzle out to the effective
range, so a standing tank never lights the cell it is in) and the lateral
cells inside the cone at the far end accumulate `dt` of heat, decaying at
`flame_heat_decay` (1.0 per second) when the stream is elsewhere. A cell or
tile catches when its heat passes `flame_ignite_seconds` (0.35 s). A quick
sweep therefore scorches without lighting; holding on a spot for a third of
a second sets it alight. Heat is stored per cell in `Game::heat` for the
ground and in `Obstacle::heat` for tiles, both cleared by `init`.

| Target | Rule | Reuses |
|---|---|---|
| Ground cell | Catches at the ignition threshold: `light_cell` for `flame_ground_seconds` (2.0) as a pool cell. Burns anyone over it, blocks the nav grid, lights its own neighbours the way any burning cell does. | `tick_fires` unchanged |
| Tall grass | Every tuft in a burning cell chars (`GrassTuft::burnt`) and stays gone for the round, and the cell leaves `grass_cells`, so it no longer conceals. **Applies to every burning cell**, so a drum's pool clears grass too. | `tick_fires`, `grass::conceals` |
| Wood tile, tree | Ignites at the threshold whatever its `flammable` roll: `health = 0, burning = true`, then the ordinary burn-out. A flamethrower is the one thing that lights a plank that was rolled to break. | `Obstacle::tick_burn` |
| Oil trail cell | Lights at the threshold; the fire runs the trail. | `light_cell`, trail spread |
| Barrel (either drum) | Fused at the threshold with `fire_fuse_factor` and the drum's own factor, `from` = the muzzle, so a chained fuel drum launches *away from the shooter*. | `arm_fuse` |
| Sandbag | Takes heat until `flame_sandbag_seconds` (1.7) and collapses through `damage_obstacle` with a new `DamageCause::Fire`. | existing death path |
| Fence | Same, at `flame_fence_seconds` (1.2): a fence burns faster than a bag of sand. | existing death path |
| Brick, glass | Never destroyed by fire. The face toward the muzzle is sooted (`scorched`) on first contact. | `face_toward`, the soot bands |
| Iron | Sooted like brick. | |
| Pickups | Untouched: a health pack in a fire is a decision, not a casualty. | |
| Wrecks | Untouched by the stream (they already burn); a wreck in a burning cell is unchanged. | |

## 6. What the fire leaves behind

All of it exists already and is reused as is:

- A burnt-out ground cell darkens its ground tint for the round
  (`GroundGrid::darken_cell`) and leaves charred rubble.
- Grass cells that burned stay bare; the charred tufts draw as stubs for
  the round rather than vanishing, so a burnt meadow reads as burnt.
- Walls keep their soot bands on the faces the stream reached.
- A dead wood tile leaves the charred rubble row; a popped drum leaves what
  a drum leaves.

## 7. Enemies, risk and the AI

- **Enemies never carry it.** No enemy rolls the pickup and no AI code
  aims a cone. The `Fired` event names the slot, so a future enemy version
  needs only the AI side.
- **Neutral fire.** Ground fire started by the stream damages the shooter
  through the existing pool rule. Driving forward into the strip you just
  laid costs health; that is the cost of the weapon and is deliberate.
- **The AI reacts through what it already has.** A burning cell is a wall
  in the nav grid, so enemies route around a strip of fire, which is the
  area-denial use. A stream hit alerts the enemy like a shell. No new
  behaviour tree node. Enemies do not flee a burning tank state; a
  burning enemy keeps fighting, which is fine for two seconds.
- **Probe fixtures are unaffected**: no fixture places the pickup, the
  weapon is player-only, and nothing it does draws RNG.

## 8. Presentation

- **The stream** is particles, not a sprite: `fx.rs` samples
  `Game::flames()` (this frame's jets) and emits `flame_particle_rate`
  (90 per second) embers and fire motes along the cone with velocity down
  the facing plus a fan of spread, in the fire ramp, with smoke off the
  far end. This matches how every other short-lived thing is drawn and
  needs no sheet. The cone's reach is what the particles show, so the
  particle spawn spans the *effective* range - a stream on a wall stops
  at the wall.
- **Muzzle.** An additive `pixel_disc` glow at the nozzle while firing and
  a muzzle-flash shimmer pushed every `flame_shimmer_every_frames` (11)
  frames, not every frame, so the ripple list does not flood.
- **A burning tank** draws embers and smoke sampled from
  `Game::burning_tanks()` (tanks with `burn_timer > 0`) the way a burning
  wreck does, plus the hit flash on contact.
- **Lit ground** is the existing ground-fire loop and glow.
- **The pickup icon** is `static/pickups/flamethrower.png`, a 32 px nozzle
  with a tongue of flame, loud orange like the other pickups, written by
  `tools/gen_flamethrower_pickup.py` in the raw-PNG convention.
- No shader work: nothing here needs a GLSL ES port.

## 9. HUD, pickup, map and tools

- **HUD.** A fourth weapon slot in both `SLOTS_ONE` and `SLOTS_TWO`, the
  slot width narrowed so four fit before the gauges; `hud_tests` re-pinned.
  The count is fuel in whole seconds; the accent is orange
  (`weapon_color`). Two players: the pair rule (`3|0`) applies as for the
  other weapons.
- **Pickup.** `PickupKind::Flamethrower` (`pickup = "flamethrower"` in a
  map), a `Tool::Pickup` in the builder's PICKUP category, a texture in
  `EditorTextures`/`Textures`, and a row in the dev server's `map_get`
  description.
- **Dev tools.** `set_tank { flame_fuel }`, `flame_fuel` in `snapshot` and
  the probe's tank rows, `step { fire: true }` holds the trigger (it is
  full-auto, so lockstep works unchanged). `events` carries the new kinds.

## 10. Knobs

One new `tunables!` group, `flamethrower`:

| Knob | Default | Meaning |
|---|---|---|
| `flame_fuel_per_pickup` | 9.0 s | fuel a pickup grants; stacks |
| `flame_range` | 164 px | cone length from the muzzle |
| `flame_half_angle_deg` | 21.4 | cone half angle |
| `flame_damage_per_second` | 20 | to a tank inside the cone |
| `flame_afterburn_seconds` | 2.75 | how long a touched tank keeps burning |
| `flame_afterburn_dps` | 4 | damage per second while it burns |
| `flame_frog_damage_per_second` | 10 | to a frog inside the cone |
| `flame_ignite_seconds` | 0.35 | exposure before a cell or tile catches |
| `flame_heat_decay` | 1.0 /s | how fast unheated exposure fades |
| `flame_ground_seconds` | 2.0 | how long a lit ground cell burns |
| `flame_sandbag_seconds` | 1.7 | exposure that collapses a sandbag |
| `flame_fence_seconds` | 1.2 | exposure that snaps a fence |
| `flame_particle_rate` | 436 /s | stream particles (cosmetic) |
| `flame_shimmer_every_frames` | 11 | muzzle ripple cadence (cosmetic) |

## 11. Tests

`mechanics_tests`/`props_tests`, one per promised rule, headless:

1. A pickup grants fuel and queues the weapon; holding fire drains it at
   one second per second; the slot empties at zero.
2. An enemy directly ahead inside the range takes damage; one behind, or
   beside the cone, takes none; one past a wall takes none.
3. A touched enemy keeps burning for `flame_afterburn_seconds` after the
   stream moves away, and afterburn can kill.
4. Holding on a cell for the ignition time lights it; a quick sweep does
   not; the lit cell blocks the nav grid and burns out on schedule.
5. Grass in a lit cell is gone and the cell no longer conceals the player.
6. A wood tile rolled non-flammable still ignites under the stream.
7. A sandbag collapses after its seconds, a fence after fewer.
8. A drum in the cone gets a fuse; a fuel drum launches away from the
   shooter.
9. An iron tile facing the muzzle is sooted and undamaged.
10. The shooter takes no damage from its stream and does take damage from
    driving into the strip it lit.
11. A round with no flamethrower pickup draws exactly the RNG it did
    before (the prop-free replay guard, extended).

## 12. Non-goals and open questions

Out of scope, on purpose:

- Enemies wielding it (section 7).
- Grass *spreading* fire to neighbouring grass cells. A wildfire across a
  meadow would clear every hiding place in a map at once; it is a knob
  worth adding later (`grass_fire_spread_cells_per_second`, default 0),
  not a default.
- Fire on the frog's or the player's *hull* spreading to tiles they drive
  past. A burning tank is a timer, not a source.
- Sound. There is no audio in the game.

Two choices made without asking, easy to change:

- The **ignition delay** (0.35 s of exposure). Without it every frame of a
  sweep lights a cell and a half-second pass carpets a corridor; with it
  the player has to *hold* the stream to start a fire, which is the
  difference between a flamethrower and a paint roller.
- **Fuel shown as seconds**, not a percentage bar: it sits in the weapon
  slot beside the laser's charges and reads the same way.

## 14. As built

Where the implementation departs from the sections above, and why:

- **Afterburn does not run while the stream is still on the hull.** The
  cone charges `flame_damage_per_second`; the afterburn's lower rate
  applies only on frames the stream is elsewhere, so the two never stack
  and a held stream deals exactly its rate.
- **Frogs are damaged, not hopped** (section 4). The shell's hop draws the
  landing spot from the round RNG; keeping the flamethrower RNG-free was
  worth more than the hop.
- **The shooter's own cells never heat.** "A quarter cell past the
  muzzle" was not enough: `tick_fires` burns a hull whose box *touches* a
  burning cell, so a stream that lit the cell in front of the tracks lit
  the shooter. The rule is now the same box test in reverse - a cell the
  shooter's hull is over or touching is skipped - and the strip starts one
  cell out. Driving forward into it still costs health (test 10).
- **Hull slack.** A tank counts as in the cone when its centre is within a
  quarter hull of it, not exactly inside: a jet that visibly washes over a
  flank and does nothing read as a miss.
- **The dry tank is not the end of the hold.** Fuel running out mid-hold
  releases `flame_held`, so the next press with a fresh tank logs a new
  `Fired`.
- **Cone width fills from the start.** The particles are seeded across
  the cone's width along its whole length (`fx::Fx::flame_mote`); at the
  first default of 90 a second the stream read as a dotted line.
- **The defaults were re-tuned by the author in the panel** after the
  first play-test: fuel 9 s, range 164 px at a 21.4 degree half angle,
  afterburn 2.75 s, sandbag 1.7 s, 436 particles a second, a shimmer
  every 11 frames. The knob table in section 10 carries the live values.
- **HUD, two players.** The fourth slot fits by setting the two-player
  weapon pairs in the 10 px font (`hud::CHAR_W_SMALL`); the single-player
  bar shifts the heart, HP and shells left instead.
- The builder's short label for the pickup is `flame` (the cursor readout
  has no room for `flamethrower`); the tool's name stays `flamethrower`.

## 13. Phases

1. Weapon and cone: the enum, the pickup, fuel, the jet, damage and
   afterburn, HUD slot, icon, tools (tests 1 to 3 and 11).
2. Environment: exposure, ground, grass, wood, drums, sandbags, fences,
   soot (tests 4 to 10).
3. Presentation: stream particles, muzzle glow and shimmer, burning tanks,
   charred grass stubs.
4. Docs: CLAUDE.md module map, docs/sandbags-barrels-fences.md's as-built
   notes for the fire rules, this file's status line.
