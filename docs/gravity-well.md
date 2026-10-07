# Gravity well

BB-42, the sixth and last of the six weapons of BB-36. A special weapon from
its own crate (`pickup = "gravity_well"`), one at a time like every other
(`Tank::take_weapon`): a crate loads `well_per_pickup` (3) wells, a re-pick
refills to three, another weapon's crate replaces it. Seats and enemies
alike: an enemy takes the crate while it carries no special
(`Tank::wants_pickup`) and uses it by its own rule (§4).

A press fires a slow violet orb down the gun line. A second press anchors it
where it is; left alone it anchors itself where it meets something or at the
end of its flight. With a snap a ring forms round a black core, and for four
seconds everything within four cells falls toward it: tanks of both sides
(the shooter's too) roll toward the core along their tracks while their
tracks bite sideways, so a scout is dragged and a titan braces; shells,
bullets, plasma bolts, missiles and drones curve toward the core and what
reaches it is swallowed; grenades roll in and orbit on the ring; drums are
lifted out of their cells and circle the core; crates and the frog slide in.
Then the well collapses: everything it holds is flung outward, the other
side takes a hit, and the drums it gathered go off together. The field
looks like it is going down a drain - tread marks swirl into it and the
grass leans in. Counter: drive across the pull, not away from it.

The sonic hammer's doc (docs/sonic-hammer.md §3), the EMP's
(docs/emp-burst.md §3.3), the gauss rail's (docs/gauss-rail.md §3), the FPV
swarm's (docs/fpv-swarm.md §3) and the rod's (docs/rod-from-god.md §3) lay
down the shared path this plugs into: the AI hook, the wind-up, the online
press show, the knock and its skid (`knock_from`), the carried/fired split,
the disabled state and the EMP's strike walk, the dangers and the herd, the
trigger kinds, air targets, **zones**, the probe's `--crate`, the spawn
swap, `spawn_pickup` and the armory. This PR adds the **well field** (§3.3):
a sum of pulls, a pure function of the wells standing at a round tick and a
point, which the drive, the projectiles' fixed-step loop, the grenades, the
missiles, the drones, the frog, the crates and the cosmetic layers read, and
which the client's sandbox, its provisional shots and its incoming fire read
on the same ticks the room does - so a bent shot is drawn where the room
flies it and a pulled hull is predicted where the room accepts it.

## 1. How it plays

### The orb

- **One per press.** The trigger is `Trigger::Press` (docs/gauss-rail.md
  §3.3). A press edge with no orb of this tank's in flight, a well left,
  the special up (`!special_down()`) and `Tank::fire_cooldown` out
  **launches** an orb: one well spent, `fire_cooldown =
  well_reload_seconds` (1.5), `Event::Fired { weapon: "gravity_well" }`.
  No recoil - the projector lets go of it.
- **From the gun line, along the facing.** The orb leaves the gun line's
  muzzle (`Tank::gun_line_muzzle`) along the hull's cardinal facing
  (`Dir::from_rotation`) at `well_orb_speed` (150 px/s) - slower than a
  seat drives (210) and about an enemy's pace - never off aim: no misfire skew, no spread. It is a shot
  (`well::Orb`, a `Projectile`): an id from the round's projectile counter
  (`Game::spawn_pending`), its own two states (`OrbState::Launch`, 0.1 s
  swelling at the muzzle, then `Flying`), integrated in `step_world`'s
  fixed-step loop with the shells, swept every tick.
- **What stops it** - where it **anchors by contact**: the first thing its
  sweep (`Terrain::sweep_rewound`, half extent `well_orb_half_px` (6), a
  seat's rewind as every seat's shot has it) enters - a live tank's hull or
  turret of either side but its shooter's (a seat's orb grows an enemy's
  boxes by `player_shot_hit_pad_px`, as a seat's shell does), a frog, a tile
  that blocks sight (`Material::blocks_sight`: every wall, glass, trees,
  towers, drums, a volcano's cone, a training door) or the field's edge. It
  floats over sandbags and fences (knee-high, the gun tower's rule) and
  past a lamp post's pole - the tiles that do not block sight, handed to
  its sweep as passed over - and over crates, grass, water, lava, oil,
  fires and wrecks (see-through to every shot). No roll anywhere: no pass-over, no
  deflection. It anchors at its entry point backed off `well_orb_half_px +
  2` px along its heading, so the core never sits in a wall or a hull. It
  hurts nothing.
- **The second press anchors it where it is.** While the tank's orb flies
  (`Tank::orb`), the trigger's next press edge anchors it on that tick at
  the orb's position - **whatever the reload** (`fire_cooldown` is not
  consulted: the reload gates launches) and with no well spent. It needs
  the special up: an EMP'd tank's press fires a shell, as every offline
  special's does, and its orb flies on (below).
- **Left alone it anchors itself** at `well_orb_range_px` (320, ten cells,
  two seconds) flown - "what if never anchored": it always becomes a well.
- **A press meant for it**: a press within `well_anchor_grace_seconds` (0.4)
  after this tank's orb anchored by itself (contact or range) does nothing -
  the player pressed as it struck - so a late anchor press online or
  locally never fires a second orb by accident.
- **Other wells bend it** like a plasma bolt (below), and one that reaches
  another well's core is swallowed (`Event::Swallowed { what: Orb }`): no
  well. An EMP's ring reaching it fizzles it (`Event::OrbFizzled`): no well.
  A portal leaves it alone - it is not one of the shots `portal_shots`
  carries - and it flies over the anchor.
- **The shooter wrecked or EMP'd with an orb in flight**: the orb is the
  world's. It flies on and anchors by contact or at its range; the well is
  its shooter's (the kill credit), a dead shooter's too. A disabled shooter
  cannot press it down; a wrecked one cannot press at all.
- **Events**: `Fired` on the launch (then the orb is a shot on the wire,
  §8); `Event::WellAnchored { id, slot, x, y, by }` on the anchor (`by`:
  `Press`, `Contact` or `Range`), where the zone begins.

### The well

- **A zone.** The anchor puts a `Zone` (docs/rod-from-god.md §3.3) on the
  field: its id the orb's, kind `ZoneKind::Well(WellZone)`, its owner the
  shooter (`Owner`, and its seat if it is one), its centre the anchor point,
  its radius `well_radius_px` (128, four cells), `until` the end of its
  current stage on the round clock. The orb is gone; the zone is what every
  reader reads from here on.
- **Forming** (`well_form_seconds`, 0.4): the snap and the ring forming -
  the tell. Nothing is pulled yet.
- **Pulling** (`well_pull_seconds`, 4.0): the well field (§3.3) pulls
  everything below. `until` is the collapse.
- **The collapse** (below), on the tick `until` is reached - or the tick an
  EMP's ring reaches its centre. The zone goes.

**The strength** at a point `d` px from the centre, `R = well_radius_px`,
`c = well_core_px` (12):

```
s(d) = 0                       d >= R
       (R - d) / (R - c)       c <= d < R     1 at the core's rim, 0 at the edge
       d / c                   d < c          down to 0 at the centre
```

so the pull is strongest a core's width out and gentle at the edge, and the
centre itself is a still point a pulled thing settles on rather than
overshoots. Its direction is toward the centre. Several wells sum (§3.3).

### What it pulls

**Hulls** - every live tank with a body on the field, either side, the
shooter included; not a wreck, not a tank rolling in through a gate. The
pull goes through the drive (`drive_tank_with`, §3.3), as the water's
current does, and it treats the hull's two axes differently, because a
tracked hull does: **along its tracks it rolls, across them it bites.**

- *Along the tracks* (the hull's facing axis, `Tank::facing_along_x`): the
  well is a **current** of `u = well_current_speed * s(d) / m^well_mass_exponent`
  px/s toward the core, and the drive runs in its frame - a ford's current's
  rule (`Footing::flow`). A hull with no command settles to the current and
  rolls toward the core at `u`; one driving straight away nets its own speed
  less `u`; one driving at the core nets its own speed plus `u`.
- *Across the tracks*: the well is a **side pull** of `a = well_side_pull *
  s(d) / m^well_mass_exponent` px/s² toward the core that the tracks' grip
  (`tank_turn_grip_force * footing.grip / mass`) cancels up to its limit: a
  hull broadside to the pull holds where its grip beats `a` and slides
  sideways where it does not.
- `m` is the chassis's mass factor (`tank_mass_factor`, 1 for the standard
  chassis). At the defaults (320 px/s, 650 px/s², exponent 2), at the
  strongest point: a scout (0.77) feels a current of 540 px/s and a side
  pull of 1090 px/s² against a grip of 650 - it is dragged whichever way it
  faces; a standard chassis 320 px/s and 650 against 500; a breaker (1.26)
  200 and 410 against 400 - it holds broadside almost to the core; a titan
  (1.89) 90 and 180 against 265 - it holds broadside anywhere, and stopped
  facing the core it creeps in at walking pace.
- **Drive across the pull, not away from it** - true by the model, and held
  by §10's `deep_in_driving_across_escapes_and_driving_away_does_not` (the
  numbers here are its, from a model of `drive_tank_with` at 60 Hz). Deep
  in, a hull driving straight away has the whole current along its tracks:
  a standard enemy (160 px/s) 50 px from the core is carried into it in
  under two seconds at full throttle, and a standard seat (210 px/s) 40 px
  out stands still flooring it. Turned across, the current along its tracks
  is next to nothing and its grip holds most of the side pull, so it keeps
  its speed and leaves along a chord: the same enemy is out in under two
  seconds, the seat in a second and a quarter. A scout seat driving away is
  carried in from anywhere within 70 px; across, it is out from 50 px.
  Nearer the edge, where the current is weak, either way out works and away
  is the shorter; the deeper in, the more only across does - and right at
  the core nothing does but a heavy chassis. Stopping broadside is how a
  heavy chassis stands its ground: a breaker or a titan stopped across the
  pull holds, one stopped facing the core rolls in.
- **Knocked off its tracks** (a skid - the hammer's knock, the rail's
  recoil, a rod's shove, the well's own collapse): the tracks neither roll
  nor bite, so the whole of the pull is a current and the skid runs in its
  frame (the skid branch takes `current - flow - well current`).
- **Ground and sky** keep their say: a ford's pace and grip, ice's grip
  (`ice_grip_factor`), rain's (`rain_grip_factor`) all weaken what the
  tracks hold, so the well drags harder on wet ground and ice; a gust adds
  its flow.
- A hull pressed against a wall or another hull by the pull scrapes there as
  the drive presses any hull - the solver's contact, no damage; tanks in a
  clump at the core are held against each other until the collapse.
- **A portal** in the pull: a hull dragged into its trigger radius goes
  through (`portal_phase`), as any hull does, and is out of the pull.

**Shots in flight** - shells, bullets, plasma bolts and orbs (their own
included, either side's): each fixed step, before it moves, a shot's
velocity takes `well_shot_pull * s(d)` px/s² toward each well's core over the
step and is set back to its own speed - **the heading bends, the speed does
not**, so its sprite and tracer turn with it (`Projectile::bend`). The sweep
still checks each tick's straight stretch, so nothing tunnels. At the
defaults (3600 px/s²) a shell aimed past the core at one cell's offset turns
about 60 degrees, at two cells 80, at three about 40, at three and a half
about 10; one aimed within about 20 px of the centre falls in. A slower shot
bends more (a bullet, faster, a little less; an orb, at 150 px/s, always
falls in).
- **Swallowed**: a shot whose stretch of the tick passes within
  `well_core_px` of a pulling well's centre, before its sweep met anything,
  is gone there (`Event::Swallowed { what, x, y }`): no hit, no burst, a
  violet pop (§5). A hull clumped at the core is met by the sweep first, so
  a shot aimed into a clump hits it.
- **Ricochets and shields** as ever: a shell off iron or a shot turned by a
  shield flies on from there, and bends from there.
- **Not bent**: the laser's beam and the rail's slug (instant traces - no
  flight to bend), the tesla's arc, the flamethrower's jet (a cone rule),
  and everything whose arc is a function of its age: flying drums, lava
  bombs, the bio slush's globs.

**Missiles and drones** (in the air, either side's): their ground velocity
takes `well_air_pull * s(d)` px/s² toward the core each fixed step
(`Missile::drag`, `Drone::drag`, docs/fpv-swarm.md §3.3), and their
guidance steers back toward their aim - a missile curves, a diving drone is
dragged off its point. One whose ground point passes within `well_core_px`
of the centre is swallowed: a missile is removed with no blast
(`Swallowed { what: Missile }`), a drone is downed (`Game::strike_air(..,
AirStrike::Well, ..)`, falling as a dud). Its burst, if it lands first, is
its own.

**Grenades** (on the ground or aloft, either side's): their ground velocity
takes `well_grenade_pull * s(d)` px/s² toward the core each tick (in
`roll_grenades`, before the roll), against the ground's drag - they roll in.
One within `well_ring_px` (24) of the centre is **captured**: from then its
ground point is on the ring at its bearing, turning at `well_orbit_speed`
(3 rad/s) clockwise as drawn, its height 0, its roll and lamp drawn as it
goes (`Grenade::orbit: Option<GrenadeOrbit>`, `roll` skipped while set). Its
fuse burns on: one that runs out on the ring bursts there, on the clump. Six
grenades lobbed into a well circle it together.

**Drums** (the tiles of `Material::is_explosive`): on the pull's first tick,
every standing drum whose cell centre lies within `R` of the centre - fused
or not - is **lifted**: the tile dies with no blast (`ObstacleDestroyed`,
its nav cell opens, the hammer's throw's way) and becomes a **held drum**
(`well::HeldDrum`: id, the well's id, its cell, its `Drum`, its fuse if one
burned, the tick it was lifted). It spirals in from its cell to the ring
over `well_capture_seconds` (0.8) and then circles with the grenades, drawn
lifted over its shadow (the flying drum's convention) - its position a pure
function of the round clock (`well::held_at`), so a replica draws it from
three numbers. Held drums are not tiles: shots pass them. A held drum's fuse
burns on; **any blast whose reach covers a held drum's ground point sets it
off at once** (the chain), and so does the collapse - "drums pulled together
go off together" (below).

**Crates** (every pickup on the field, a whole crate or loose contents):
each slides toward the core at `well_crate_speed * s(d)` (60 px/s) as an
offset from where it lay (`Pickup::drift`), stopping where its centre would
enter a solid cell, deep water, deep lava or the field's half-cell inset. It
keeps its slot: whoever touches it where it now is takes it, the slot
respawns as ever, and the map's slots and the round's respawn draws are
untouched. It does not break.

**Frogs** (either side's, alive): a frog in the pull is **pinned** - it
neither hops (its evasive hop, its training walk) nor hops back - and slides
toward the core at `well_frog_speed * s(d)` (50 px/s) wherever
`Terrain::frog_fits` lets it (its static collider moved with it, as a hop
moves it). It bites as ever. `Frog::pulled` is the flag.

**Fish** in a lake the pull reaches swim to the deep cell nearest the
core while it pulls (an attractor among `fish.rs`'s scares; cosmetic).

**The "at 11"** (built, §12 decision 21): **tread marks** within `R` creep
toward the core at `well_mark_speed * s(d)` (30 px/s) and turn about it at
`well_mark_twist * s(d)` (1.2 rad/s) clockwise, so the marks round a well
are left as a swirl; a mark that reaches the core is gone. **Grass tufts**
within `R` lean toward the core's side by `well_grass_lean_px * s(d)` and
are pressed half flat, and whip the other way at the collapse. Both are
cosmetic steps (`Game::drain_marks`, `Game::lean_grass`) run by the well's
phase and by a replica's `tick_presentation` alike, hashed and with no RNG -
nothing reads them back. **Particles** - smoke, dust, sparks - within `R`
spiral in (`fx.rs`, presentation).

**What it leaves alone**: wrecks (settled hulks, `settle_wreck`'s damping,
as no blast moves one - §12 decision 7), tanks rolling in, every tile but a
drum (walls, glass, props, trees, towers, lamp posts, cones, doors),
lanterns, fires, oil, ooze, scorches, rubble, the water and the lava, the
laser, the rail, the tesla, flames, flying drums, lava bombs, globs,
portals.

### The collapse

On the tick a well's `until` is reached (or an EMP's ring reached its
centre this tick), `Game::collapse_well` walks, in this order:

1. **The drums go off together**: every drum the well holds, in id order,
   goes off where it is on the ring - `Event::Blast { chained: true, drum }`
   and a `PendingBlast` with `BlastShape::Chained { from: centre, .. }`,
   the flying drum's landing path (`tick_launches`): its kind's blast, an oil
   drum's pool, a fuel drum's fire. Packed within a ring of 24 px, they
   overlap on whatever the well holds at its core.
2. **Hulls are flung**: every live hull on the field whose box's nearest
   point is within `R` is knocked out from the centre (`Game::knock_from`,
   the rod's: `well_fling_speed` (400) at the centre falling linearly to 0
   at `R`, over `m^well_fling_mass_exponent` (1.5), at most
   `well_fling_max_speed` (480)) - the hammer's knock and skid, either side,
   the shooter too; a seat's on `Frame::shoves` for an owned hull. A hull
   whose centre stands within 1 px of the centre is flung back along its
   own facing. **The side opposing the well's owner** takes
   `well_collapse_damage` (10) times `1 - d/R`, no roll, through
   `Tank::take_damage` (a rainbow shield soaks it), credited to the owner,
   `Event::Hit { cause: HitCause::Well }`; its own side is flung unhurt.
3. **Frogs hop out**: a frog in `R` hops along the line out from the centre
   `max(1, round(well_fling_frog_cells * (1 - d/R)))` cells
   (`well::frog_fling_target`: the farthest cell on the line it fits, walked
   back a cell at a time, no jitter) through `Frog::start_hop`; the frog of
   the side opposing the owner takes the collapse damage too
   (`Frog::damage`).
4. **Grenades are thrown out in a ring**: every grenade the well holds
   leaves the ring along its own radial at `well_fling_grenade_speed` (220)
   with a lob (`climb` = `well_fling_grenade_climb`), its fuse as it was.
5. **What flies is turned out**: every shot, orb, missile and drone within
   `R` has its heading turned straight out from the centre at its own
   speed; missiles and drones steer back after.
6. **Crates slide out**: a crate in `R` gets a slide of
   `well_fling_crate_speed * (1 - d/R)` (240) out from the centre, braking at
   `well_crate_friction` (600 px/s²), stopping as its pull did.
7. **The fish** within `R` are thrown onto the bank (`fish::throw_from`, the
   rod's, at most `well_fish_throw_max`), from the event; the grass whips
   out; the marks stay swirled.
8. `Event::WellCollapsed { id, x, y, early }`; the zone goes; the show (§5).

The collapse moves nothing on the end screen and hurts nobody (below).

### An EMP, a wreck, a teleport, a swap

| What happens | To an orb in flight | To an anchored well |
|---|---|---|
| An EMP's ring reaches it (its centre) | Fizzles: no well (`OrbFizzled`) | **Collapses at once** - the whole collapse, `early: true` (docs/emp-burst.md's strike walk, the block after the drones) |
| The EMP reaches its shooter | No anchor press (special down); the orb anchors itself | Nothing - the well is the world's |
| Its shooter is wrecked | Flies on, anchors by itself; its well is the wreck's | Pulls on; the collapse is credited to the dead shooter |
| Its shooter teleports | Flies on; a press still anchors it | Nothing |
| Its shooter takes another weapon's crate | No anchor press (its trigger is another weapon's); it anchors itself | Nothing |
| A well crate | Refills; a press still anchors it | Nothing |
| The round ends | Flies on and anchors as a show | Forms, swirls and collapses as a show (below) |
| The round restarts | `init` clears it | `init` clears every zone and held drum |

### Several wells at once

Wells are independent: each its own zone, ring and collapse. Their pulls sum
at every point (§3.3), so two wells close together pull harder between them
and bend a shot from both. A thing in reach of two at the moment one would
capture it (a drum on the pull's first tick, a grenade reaching a ring) goes
to the lower id; a thing a well holds stays its until it is set off or
thrown. A shot, a missile or a drone is swallowed by whichever core it
reaches. A well may be anchored inside another's pull; an orb fired through
a pulling well bends, and is swallowed if it falls in.

### On the end screen

`player_phase` and `enemy_phase` do not run, the physics does not step and
nothing is driven. Orbs in flight fly on and anchor as a show; wells form,
swirl and collapse as shows (`Game::well_phase(f, false)`): nothing is
pulled, captured, flung, hurt or swallowed; held drums go off at the
collapse as blasts that hurt nobody (`explosions(f, false)`), as every blast
on the end screen does.

## 2. Where it lives

| File | What |
|---|---|
| `src/well.rs` (new) | The weapon's headless half. `Orb` (the shot: id, owner, position, prev, velocity, rotation, state, timer, flown, done - a `Projectile`), `OrbState` (`Launch`, `Flying`; `col`/`from_col`), `WellZone` (stage, the anchor's `AnchorBy`, the shooter's seat, `emp_collapse`), `WellStage` (`Forming`, `Pulling`), `AnchorBy`; **the field** (§3.3): `strength(d)`, `WellSource`, `WellField` (`at`, `pull`, `hull_pull`, `shot_accel`, `core_hit`), `bend(velocity, accel, dt)`, `HullPull` and its axis split; `HeldDrum` and `held_at`, `GrenadeOrbit` and `orbit_at`, `frog_fling_target`, `holds_broadside`, `escape_dir`, `curved_streak`; the composers `compose_orb`, `compose_snap`, `compose_well`, `compose_swirl`, `compose_collapse`, `compose_swallow`, `compose_held_drum_glow` (pure, `pyro::Shape`s); `module_cell` |
| `src/simulation/well.rs` (new) | The world half. `fire_well` (the dispatch arm: launch or anchor), `anchor_orb`, `resolve_orbs` (contact, range, swallowed), `well_field` (the frame's field), `well_phase(f, live)` (stages, the drums' lift, the frog and the crates, the marks and the grass, every collapse), `collapse_well`, `well_anchor_show`, `well_collapse_show`, `swallow_show` (the cosmetic halves, which a replica's events and a client's own press call too), `tick_well_pictures` (a replica's stages, held drums, marks, grass), `Game::{held_drums, hull_footing, drain_marks, lean_grass}`, `well_senses` and `pull_senses` (§4), `seat_orb`, `debug_well` |
| `src/simulation/well_tests.rs` (new) | The scenario tests (§10) |
| `src/simulation/weapons.rs` | The `ActiveWeapon::GravityWell` dispatch arm; `Projectile::bend` (a default for every shot kind: turn the velocity, keep the speed, set the rotation) and `Projectile::swallowed`; `advance_projectiles` taking the frame's field |
| `src/simulation/mod.rs` | `Frame::{wells, pending_orbs}`; `Game::{held_drums, anchor_grace}`; `well_field` built after `tick_timers`; `well_phase` after `resolve_zones` (the rod's), before `step_world`, and `well_phase(f, false)` on the end screen; `resolve_orbs` after `shell_vs_shell`; `Footing::well` and the axis split in `drive_tank_with`, the skid branch in the whole current; `hull_footing` in `player_phase`, the enemy apply pass and `predict_seat`; the anchor press before the cooldown gate in `drive_player` and `enemy_trigger`; `resolve_projectiles`' swallow; `accept_seat_pose`'s well drift; `tick_presentation` (the wells' stages, held drums, marks, grass); `Event::{WellAnchored, WellCollapsed, Swallowed, OrbFizzled}`, `HitCause::Well`; the swap's table entry |
| `src/simulation/missiles.rs`, `src/missile.rs` | `Missile::drag`, the swallow in `resolve_missiles` |
| `src/simulation/fpv.rs`, `src/air.rs` | `Drone::drag`, `AirStrike::Well` |
| `src/simulation/grenades.rs`, `src/grenade.rs` | The pull and the capture in `roll_grenades`, `Grenade::orbit` (`roll` skipped while set), the fling |
| `src/simulation/props.rs` | A blast setting off held drums (`apply_blast`), the lift (`lift_drum`: the tile's death with no blast) |
| `src/simulation/emp.rs` | The strike walk's well block (after the drones): an orb fizzles, a well collapses |
| `src/zone.rs` | `ZoneKind::Well(WellZone)`, its `danger()` (`None`), `route_cells()`, `left()`, the `ZONE_WELL` tag |
| `src/simulation/nav.rs` | A pulling or forming well's cells surcharged in `route_grid_on` (`well_ai_route_cost`) |
| `src/simulation/engage.rs` | The herd built round a well's centre (`EngageCtx::herd`, the rod's) |
| `src/simulation/command.rs` | `command::Busy::{Pulled, Anchoring}` |
| `src/simulation/present.rs` | `PresentWorld::wells` (the field on this client's present ticks); `ProvisionalShot::advance` bending on the tick grid and swallowed; the incoming carry stepped through the field; `Game::{draw_press_show}`'s anchor arm, `add_provisional_zone` for a well, `seat_orb` |
| `src/simulation/replica.rs` | `DrawableZone`'s stage, `DrawableHeldDrum`, `DrawableCrate::drift`, `DrawableFrog::pulled` |
| `src/tank.rs` | `wells`, `orb`, `well_flash`; `ActiveWeapon::GravityWell` (`name`, `full_load`, `tell_seconds` none, `trigger` `Press`), `SPECIAL_WEAPONS`; `kick_well`, `anchor_press`; `weapon_ammo`/`take_weapon`/`empty_stock`/`wants_pickup`; the module's cells in `module_cols` |
| `src/pickup.rs` | `PickupKind::GravityWell` (`gravity_well`, row 18, its ink, spills); `Pickup::{drift, slide, at}` |
| `src/frog.rs` | `Frog::pulled`, the gate in `can_hop` |
| `src/grass.rs`, `src/track.rs` | The lean into the drain (`GrassTuft::drain`), a mark's swirl (`Track::drain`) |
| `src/fish.rs` | The attractor (a well's core among the scares, reversed) |
| `src/ai.rs` | `SpecialSense::Well(WellSense)`, `WellPlan`, `well_rule`, `SpecialUse::{Orb, Anchor}`, `generic_fire(GravityWell)`; `PullSense`, the `pull` tier, `act_pull`, `Ai::{well_plan, pull_escape, bracing}`; `special_rule` and the `air` tier yielding inside a pull; `Brain::seek` and `nearest_pickup` leaving a crate inside a pull alone; `SEEK_SPECIALS` gains the well; `AiSnapshot::{well, pull}` |
| `src/indicators.rs` | `ArrowKind::Zone { kind: Well, .. }`'s arm |
| `src/minimap.rs` | A well's mark |
| `src/hud.rs`, `src/render/hud.rs` | `HUD_WELL_COLOR`, the `weapon_color`/`weapon_pickup` arms; the anchor prompt (`hud::well_prompt`, the rod's `Corners::prompt` slot) |
| `src/game.rs`, `src/render/game.rs` | Orbs, wells, held drums, swallows and the collapse in their passes (§5); bent shots' curved tracers; the dev stats arm |
| `src/render/well.rs` (new) | The orb's and the core's drawing that takes a draw handle |
| `src/shockwave.rs`, `src/render/shockwave.rs`, `static/shockwave.fs`, `static/web/shockwave.fs` | The inward ring (`Shockwave::inward`, the shader's `starts`/`signs` arrays) |
| `src/weather.rs` | The ring's and the orb's light |
| `src/fx.rs`, `src/burst.rs` | Particles within a pull spiralling in; a well `Hit` draws its flash and violet motes, no fire; the swallow's pop (`ImpactKind::Swallow`) |
| `src/pyro.rs` | The `VOID` ramp |
| `src/net/wire.rs` | `WeaponKind::GravityWell`, `ShotKind::Orb`, `ZoneState`'s well kind and stages, `WellDrumState`, `Snapshot::well_drums`, `CrateState::{dx, dy}`, `frog_flags::PULLED`, `HitCause::Well`, `AirStrike::Well`, `SwallowedKind` |
| `src/net/delta.rs` | `well_drums`, `well_drums_gone` |
| `src/net/events.rs` | `WireEvent::{WellAnchored, WellCollapsed, Swallowed, OrbFizzled}` |
| `src/net/encode.rs`, `src/net/apply.rs` | The family, the fields, the shows; `Show::OwnShotsDrawn`'s anchor claim by orb id |
| `src/net/predict.rs`, `src/net/round.rs`, `src/net/interp.rs` | `ProvisionalKind::Orb`, the anchor press, the sandbox's field on the room's tick, the provisional well paired by id; a crate's drift blended between snapshots |
| `src/net/authority.rs` | Nothing new: the validator's allowance is `accept_seat_pose`'s |
| `src/simulation/debug.rs`, `src/devserver.rs` | `set_tank`'s `wells`; the snapshot's `wells`, `orb`, `held_drums`, a crate's `drift`, a frog's `pulled`, `pull` and `bracing`, an enemy's `well` plan; `spawn_pickup {kind: "gravity_well"}`; the `well_at` tool (§3.4) |
| `src/bin/probe.rs` | The tank line's `well=`, the fire tuple; `OUT_OF_ITS_HANDS` gains `pulled`, `HOLDS` gains `bracing` |
| `src/editor/mod.rs` | `Tool::Pickup(PickupKind::GravityWell)` (`gravity_well`) |
| `maps/armory.toml` | Its crates and a grenade crate (§3.4) |
| `src/tuning.rs` | The `well` group (§6), one row in `enemies` |
| `lang/en.ftl`, `lang/sl.ftl`, `src/text.rs` | §7 |
| `tools/punypalette.py`, `tools/spritegen/gen_crates.py`, `tools/spritegen/tankdesign/{kit,export,render,lines/vanguard}.py` | The art (§5); writes `static/crates_sheet.png`, `pickup_glyphs.png`, `tank_modules.png`, `tank_modules_glow.png`, `src/tank_art.rs` |
| `src/thumbnail.rs`, `src/maplint.rs` | The armory's pin re-baselined; the armory as it lints |
| `docs/` | This, `CRATES_SPEC.md`, `SPRITESHEET_SPEC.md`, `effects.md` (the `VOID` ramp, who draws what); `CLAUDE.md` |

## 3. The shared path

### 3.1 What this uses as weapons 1-5 laid it down

Each item of the checklist (docs/sonic-hammer.md §3.0) gets its well arm:

1. `PickupKind::GravityWell` (`#[serde(rename = "gravity_well")]`,
   appended to `ALL`, row 18, `ink`, `cooks_off` false - a projector and a
   field coil, no explosive), `PickupKind::weapon`
   (`Some(ActiveWeapon::GravityWell)`), `name`.
2. `ActiveWeapon::GravityWell` (`name` "gravity_well", `full_load` =
   `well_per_pickup`, `tell_seconds` none - the slow orb and the forming
   ring are its tell -, `trigger` `Trigger::Press`), appended to
   `SPECIAL_WEAPONS`; `Tank::wells` with its arms in `weapon_ammo`,
   `take_weapon`, `empty_stock`, `module_cols`. `wants_pickup` reads
   `special()`.
3. The dispatch arm (`fire_well`: a launch, or the anchor while an orb
   flies); the press edge in `drive_player`'s `match weapon.trigger()`
   (`Press`), with the anchor press taken before the cooldown gate (§3.2).
4. `pickup_phase` needs nothing.
5. `ai::SEEK_SPECIALS = [SonicHammer, Emp, GaussRail, FpvSwarm,
   RodFromGod, GravityWell]`; `SpecialSense::Well`, the `special_rule` arm,
   `generic_fire(GravityWell) == false` (§4).
6. `hud::weapon_color` (`HUD_WELL_COLOR`), `hud::weapon_pickup`.
7. `gen_crates.py` (`KINDS`, `GLYPHS`, the two-tone painting),
   `PICKUP_INK['gravity_well']`; the tankdesign module `well` and its anchor
   `tank_art::WELL_CRADLE` (§5).
8. `editor::TOOLS` and `Tool::name`; `tool-gravity_well`,
   `tool-short-gravity_well` in both catalogues.
9. `WeaponKind::GravityWell` (appended to `ALL`, both `From`s,
   `drawn_on_press` false - its launch is a shot, drawn as provisional
   shots are, and its anchor is claimed by the orb's id, §8),
   `Predictor::seed_gate`'s arm (`well_reload_seconds`),
   `apply::write_tank`'s ammo arm, `kick_turret` (`Tank::kick_well`, the
   module's launch cell) and `drawn_muzzle` (the gun line's muzzle).
10. `debug::{TankDebug, TankPatch}`, `set_tank`'s schema, `TankSnapshot`,
    the probe's tank line and fire tuple, `render::game::draw_tank_stats`.
11. The armory's crates (§3.4); `SPAWN_SWAPS` gains
    `(ActiveWeapon::GravityWell, |t| t.enemy_special_weapon_well_share)`
    after the rod's; the tuning group; `PROTOCOL_VERSION`.

And, as they are: **zones** (`Zone`, `Game::zones`, the family, the arrow
`ArrowKind::Zone`, the minimap's mark, `add_provisional_zone`,
`hide_zones`, `Predictor::pair_zone` by id); the **knock** and its skid
(`Game::knock`, `knock_from`, `Event::Shoved::skid`, `seat_knock`); the
**herd** (`EngageCtx::herd`); the EMP's **strike walk** and `special_down`;
**air targets** (`Game::air_targets`, `strike_air`); the hammer's and the
rod's **fish on the bank** (`fish::throw_from`); the **press show's claim**
(`presses_drawn`, `Show::OwnShotsDrawn`); the **provisional shots** of
docs/online-coop-prd.md §4.16 (`seat_shot`, `ProvisionalShot`, pairing by
`Fired`, `Live::finished`); the drum's **chained blast** (`tick_launches`'
path); `Tank::special`/`active_weapon`; `HitCause` on `Event::Hit`;
`spawn_pickup {kind: "gravity_well"}`; the probe's `--crate gravity_well`;
`pyro::Shape::Arc`; the HUD's prompt line (`Corners::prompt`).

### 3.2 What this extends

- **`Zone`** gains its second kind: `ZoneKind::Well(WellZone)`.

  ```rust
  /// A well standing on the field (`well.rs`): what stage it is in, how it
  /// was anchored, and whether an EMP has called its collapse.
  #[derive(Clone, Copy, Debug, PartialEq)]
  pub struct WellZone {
      pub stage: WellStage,
      pub by: AnchorBy,
      /// The seat that fired it, for the anchor prompt and the claim.
      pub seat: Option<u8>,
      /// An EMP's ring reached its centre this tick: `well_phase`
      /// collapses it before anything else.
      pub emp_collapse: bool,
  }

  pub enum WellStage { Forming, Pulling }
  pub enum AnchorBy { Press, Contact, Range }
  ```

  `Zone::until` is the end of the **current stage** - the pull's start
  while forming, the collapse while pulling - so a client never needs the
  room's `well_pull_seconds` to know when either comes. `Zone::radius` is
  `well_radius_px`. `danger()` is `None`: a well is not a place to back out
  of the nearest way (the dodge's), which is radial - the wrong way (§4).
  `route_cells()` is the nav cells whose centre lies inside the radius, at
  `well_ai_route_cost`, while forming or pulling. `left(now)` the seconds
  to the collapse while pulling, none while forming.
- **`Footing`** gains a per-hull term, `well: HullPull` (§3.3): the current
  and the side pull at the hull's centre, already scaled by its mass.
  `Footing::at` stays position-only; `Game::hull_footing(tank)` is
  `Footing::at(..)` with the frame's field's `hull_pull(pos, mass_factor)`,
  and is what `player_phase`, the enemy apply pass and `predict_seat` hand
  `drive_tank_with`. `drive_tank_with` splits it after `tank.control`
  (the facing this tick decides the axis): the current's on-axis part is
  added to the frame the on-axis velocity is measured in (with
  `footing.flow`'s), the side pull's off-axis part is added to `current_off`
  before the grip's clamp (so the grip cancels it up to `max_off`); the
  skid branch takes the whole current as its frame. Every hull's one
  impulse a tick is still the drive's one impulse.
- **`accept_seat_pose`'s drift** gains the well's: the current's speed at
  the room's copy of the hull plus what its side pull builds over
  `WELL_SIDE_REACH_SECONDS` (0.5, server policy beside
  `POSE_REACH_SLACK_PX`), so an owned hull pulled faster than its top speed
  is placed where its client puts it.
- **`Projectile`** gains `bend(&mut self, accel: Vec2, dt: f32)` - turn
  the velocity by `accel * dt`, set it back to its speed, set `rotation`
  to the new heading - with one default body for every kind, and
  `advance_projectiles` calls it with the frame's field before `advance`.
  `resolve_projectiles` ends a shot whose tick passes the core before its
  sweep met anything (`WellField::core_hit`, §3.3): `detonate` is not
  called - the shot is despawned and `Event::Swallowed` logged.
- **`ProvisionalKind::Orb`** and **`ShotKind::Orb`**: the orb is a shot on
  the wire (§8) and a provisional shot on its shooter's client, by the
  machinery shells use (`Game::seat_shot`'s orb arm, `ProvisionalShot`'s
  orb arm of the state machine, pairing by `Fired`). It differs in two
  places: `foreign_flying_shots` leaves orbs out (an enemy's orb is drawn
  where the room has it, beside the zone it becomes, not carried into the
  present - it hits nothing to judge in the present), and no portal check
  takes it.
- **`ProvisionalShot::advance(dt, wells)`** steps on the tick grid: it
  keeps the seconds into its current tick and the round tick its flight
  stands on, and at each tick boundary the frame crosses it bends its
  velocity by `world.wells(tick).shot_accel(position, &t)` - the room's Euler
  step on the room's ticks, so a bent provisional and the room's copy agree to the
  quarter pixel whatever the frame rate. A swallow is a stop
  (`ShotStop::Swallowed`), handed on as a struck shot is: the shot is kept
  off the picture until its room copy is gone, and shown again if that copy
  flies on past it.
- **The incoming carry** (`round.rs`'s `draw_incoming_in_present`) steps a
  foreign shot through the field tick by tick when the field is not empty
  (`well::carry(position, velocity, ticks, field)`), stopping at the first
  static contact on each stretch, swallowed at a core as above; straight,
  as now, when it is empty.
- **`SpecialUse::Orb { face, anchor_px, at_seat, why }`** - `Fire` that
  also records `Ai::well_plan = Some(anchor_px)` (how far along its flight
  to anchor) - and **`SpecialUse::Anchor { face }`** - pull the trigger this
  tick without touching `Ai::fire_timer` or `Ai::shot_at_seat` (the press
  that anchors).
- **`Ai::think`** takes `pull: Option<PullSense>` beside `sense`, `dangers`
  and `threat`, kept on the `Brain`; the tree gains the `pull` tier (§4).
- **The anchor press is taken before the cooldown gate**: `drive_player`,
  `enemy_trigger` and `predict_seat` ask `Tank::anchor_press()` (an orb of
  its own in flight, the special up) first, and a press edge then anchors
  whatever `fire_cooldown` says.
- **`Show::OwnShotsDrawn`** gains `anchors: BTreeSet<u16>`: the orb ids
  whose anchor this client drew; the room's `WellAnchored` with such an id
  is neither drawn again nor handed on to `game.events` - a claim by
  payload id, the rod's and the swarm's kind (their need 7).
- **`Shockwave`** gains `inward: bool` and a start radius: an inward ring
  starts at `start` and runs in to the centre at the ripple's speed; the
  shader takes `starts[4]` and `signs[4]` beside `times[4]` (both
  dialects, §5). An outward ripple is `start` 0, sign 1 - every shipped
  ripple, so the frame is what it was.
- **The EMP's strike walk** gains its last block (after the drones): an
  orb in flight whose position the front reaches fizzles; a forming or
  pulling well whose centre it reaches gets `emp_collapse`.
- **`command::Busy::{Pulled, Anchoring}`**: a tank in the `pull` tier or
  holding for its orb is never ordered and keeps right of way.
- **`Pickup::drift` and `Pickup::slide`**: a pickup's offset from where it
  lay and its slide speed; `Pickup::at()` (position plus drift) is what
  `in_reach`, the drawing, the AI's pickups and the wire read. Zero for
  every pickup no well has touched, so every reader reads what it did.
- **`HitCause::Well`**, **`AirStrike::Well`**, **`Event::Swallowed`**.

### 3.3 What this adds: the well field

A well is a zone the AI reads like any (§4), but what it does it does
through one pure function every mover asks - in the room, in the
client's sandbox, in its provisional shots and in its incoming carry -
on the same round ticks.

```rust
/// The wells pulling at one round tick: their centres, by id. Built from
/// `Game::zones` - every well pulling at that tick, its stage read off the
/// clock - so a replica, a sandbox and a room build the same field from
/// the same zones.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WellField { pub sources: Vec<WellSource> }

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WellSource { pub id: u32, pub centre: Position }

impl WellField {
    /// The field at round tick `tick` from `zones`, the stage read off the
    /// clock rather than the stored stage: a forming well whose `until`
    /// is at or before `tick` pulls, a pulling one whose `until` is at or
    /// before `tick` does not (its collapse is due). The room builds it at
    /// its own tick; a client at the tick its present stands on, which
    /// runs past the room's. Compared in whole ticks (`until` over
    /// `PHYSICS_FIXED_DT`, rounded - the wire's own unit for it), so the
    /// room and a client agree on the tick a stage turns.
    pub fn at(zones: &[Zone], tick: u32) -> WellField;
    pub fn is_empty(&self) -> bool;
    /// The sum over the sources of the unit vector toward each centre
    /// times `strength(d)`: 0 outside every well. Summed in id order.
    pub fn pull(&self, p: Position) -> Vec2;
    /// A hull's current and side pull at `p` for a chassis of mass factor
    /// `m`: `pull(p)` times `well_current_speed` and `well_side_pull`, over
    /// `m^well_mass_exponent`.
    pub fn hull_pull(&self, p: Position, m: f32, t: &Tuning) -> HullPull;
    /// A shot's acceleration at `p`: `pull(p) * well_shot_pull`.
    pub fn shot_accel(&self, p: Position, t: &Tuning) -> Vec2;
    /// The first source whose core (`well_core_px`) the stretch `p0..p1`
    /// passes through, and where along it (0..1) it first comes within the
    /// core's radius: a swallow.
    pub fn core_hit(&self, p0: Position, p1: Position, t: &Tuning) -> Option<(u32, f32)>;
}

/// What a well does to one hull this tick, in the world's axes.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HullPull {
    /// The current (px/s): the frame its tracks roll in, along their axis.
    pub current: Vec2,
    /// The side pull (px/s²): what its tracks' grip holds, across their axis.
    pub side: Vec2,
}

/// `velocity` turned by `accel` over `dt` and set back to its own speed.
pub fn bend(velocity: Vec2, accel: Vec2, dt: f32) -> Vec2;
```

- **Who reads it**, all from `Frame::wells` (built once a tick after
  `tick_timers` by `WellField::at(zones, frame)` - the stage by the clock,
  so a well whose pull starts on this tick pulls in it, and one whose
  collapse falls on this tick does not: it flings): the drive (`hull_footing`), the
  fixed-step loop (`Projectile::bend` for shells, bullets, plasma and orbs;
  `Missile::drag`; `Drone::drag`), `roll_grenades`, `well_phase` (the
  frog, the crates, the marks, the grass), `fx.rs` (particles,
  presentation), `fish.rs` (cosmetic).
- **Arithmetic only**: square roots and divisions, and a hull's mass factor
  raised to the exponent once per hull (the hammer's shove's own `powf`);
  no trigonometry; summed in source id order.
- **Empty in a round with no well pulling**, and every reader skips it
  then: `bend` is not called, `HullPull` is zero and `drive_tank_with`
  adds nothing, so every existing round is byte for byte what it was.
- **On a client**: `PresentWorld::wells(tick)` is `WellField::at` over the
  replica's zones plus this seat's provisional well (§8) at the round tick
  the client's present stands on - its sandbox's input tick mapped to the
  room's by the incoming lead's arithmetic (`newest.tick + (input -
  acked)`); provisional shots and the incoming carry ask it per tick, and
  `predict_seat` builds the sandbox's field from its own zones (the room's,
  through `apply::snapshot`) the same way.

**Held things** are the field's other half: what a well captures follows
a pure function of the round clock from the moment it was captured, so the
room moves it and a replica draws it from the same few numbers:

```rust
/// A drum a well lifted out of its cell (`Game::held_drums`, by id).
pub struct HeldDrum { pub id: u32, pub well: u32, pub cell: (i32, i32), pub drum: Drum, pub fuse: Option<f32>, pub lifted_at: f32 }

/// Where it is at round time `now` round a well at `centre`: from its cell
/// spiralling in to the ring over `well_capture_seconds` (eased out), then
/// on the ring at the bearing it arrived at, turning clockwise at
/// `well_orbit_speed`; the height it is drawn lifted by.
pub fn held_at(drum: &HeldDrum, centre: Position, now: f32, t: &Tuning) -> (Position, f32);

/// A grenade on the ring: the bearing it was caught at and when.
pub struct GrenadeOrbit { pub well: u32, pub bearing: f32, pub since: f32 }
pub fn orbit_at(orbit: &GrenadeOrbit, centre: Position, now: f32, t: &Tuning) -> Position;
```

- **Pure in the clock, so no wire for their motion**: a held drum's
  position comes from its family entry (§8) and its well's centre; a
  grenade on the ring travels in `GrenadeState` as every grenade does. The
  room moves a captured grenade by `orbit_at` and its fuse as ever.
- **Captured by one well**: a drum within reach of two on its lift goes to
  the lower id; so does a grenade reaching two rings on one tick.

### 3.4 The armory's well

Into `maps/armory.toml` (docs/sonic-hammer.md §3.5 and the four docs
after it); nothing placed by weapons 1-5 moves:

| Mark | Cell | Why |
|---|---|---|
| `V` well crate | 7,14 | The reserved column's last cell, six cells south of the start |
| `V` well crate | 32,13 | On the enemy side, so an enemy that carries no special collects it and its rule shows with no tuning |
| `n` grenade crate | 9,14 | Two cells east of the well crate: anchor a well, take the grenades, lob them in - the armory scene's combo (a couch's second seat does it with the first's well) |

The rest it needs is there: **the drum column** (oil 21,6 and 21,10, fuel
21,8, two cells apart) - a well anchored by it lifts all three and they go
off together at its core - with **the oil trail** (22..24,8) their blasts
light; **the lava ford** (32,0..7) and **the lake** (19..26 x 13..17, deep
in the middle) an enemy's well pulls a seat toward; **the players' frog**
(2,12), four and a half cells from the start (4,8), which an enemy's well
pulls its guard off; **the iron** (20..21 x 2..3, 33..34 x 8..9) and
**brick** an orb anchors against; **the grass** (11..15 x 11..15) that leans
into a drain and **the trees** that stand; **the portals'** absence (the
pull's portal case is tried on `portals`); **the towers** whose bullets
bend; the other weapons' crates, each with its interaction (§12).

```
     0         1         2         3
     012345678901234567890123456789012345
 0   ................................L...
 1   .+..........ggggg...............L...
 2   ............g...g...II....b..pp.L...
 3   ...P........g.s.g...II....b.....L.E.
 4   .......e...*g...g.........b.....L...
 5   ............ggggg...........zzz.L...
 6   ...tp..R.............o..........L...
 7   .............................e..L...
 8   ....S..H.........*...f%%%..b.....II.
 9   ...........................b.....II.
10   .......D.............o...*.d..H.....
11   ...........wwwwwT..........g....X...
12   ..F....X...wwwww..............R.....
13   ..........Twwwww...WWWWWWWW.b...V...
14   .......V.n.wwwww...WWWWWWWW.b.....G.
15   .....Q.....wwwww...WWWWWWWW...D.....
16   .a.................WWWWWWWW....m....
17   ...................WWWWWWWW.........
```

The reserved column is full. Checked again in Phase 2 against the armory
as weapons 3-5 leave it in code (the linter, the band's capacity), and the
armory's CPU thumbnail pin is re-baselined for the crates.

**The dev server**: `spawn_pickup {kind: "gravity_well", x, y}`,
`set_tank`'s `wells` (above 0 arms it in place of the special carried), and
one tool of the well's own, `well_at {x, y, enemy}` - a well anchored at
once at the point (forming), player 1's or with `enemy: true` an enemy's
(`Game::debug_well`, no RNG; a `GAME_ONLY_TOOLS` and `ONLINE_REFUSED_TOOLS`
member, refused off the field or inside a solid cell; replies `{id, x, y,
until}`), so a pull, a capture and a collapse are tried in lockstep without
steering an orb. `status`/`snapshot` carry each tank's `wells` and `orb`
(its orb's id and how far it has flown), every zone's well stage, the held
drums (id, well, position), each crate's `drift`, each frog's `pulled`, each
tank's `pull` (the current and side pull on it) and `bracing`, and an
enemy's `well` plan (`AiSnapshot::well`: the arm, the facing, the anchor
distance).

## 4. AI

An enemy carrying wells uses them by `well_rule`, never through the generic
tiers (`generic_fire(GravityWell)` is false): attack still lines up and
settles but never fires the well, `Brain::wants_breach` never latches with
it, and it fires no shells while it carries wells - as with every BB-36
weapon, until its three are spent. And every enemy, whatever it carries,
reacts to every well whose pull reaches it (the `pull` tier, below).

### What it is handed

`Game::well_senses` runs once per frame in `enemy_phase`, before the collect
pass, only when some live enemy on the field carries an online well or has
an orb out. It gathers once: every seat on the field (position, hull box,
concealed, hit-alerted for each enemy, `Game::sight_on` at it, its facing
and whether its trigger's weapon could reach this far - the shield arm's),
the players' frog (position, alive), every live enemy (slot, hull box,
velocity), the enemies' frog, the standing drums (cell), the trouble cells
(heat at or over `heat_hurt_from` - lava and its banks -, burning ground,
ooze, water of any depth, every live rod call's kill circle), the live
wells (centre), the frame's `Terrain` and the field. Then, in owner-slot
order, each well tank that thinks this tick gets a `WellSense`:

```rust
pub struct WellSense {
    /// What a launch now would be for, `None` when nothing qualifies.
    pub plan: Option<WellPlan>,
    /// Its orb in flight: how far it has flown and whether, where it
    /// stands now, anchoring would drag more allies than seats.
    pub orb: Option<(f32, bool)>,
}

pub struct WellPlan {
    /// The facing to launch along and how far along it to anchor (px from
    /// the gun line's muzzle).
    pub face: Dir,
    pub anchor_px: f32,
    /// The seat the well is used on - the lowest seat it counts - what
    /// `Ai::shot_at_seat` records at the launch.
    pub at_seat: Option<u8>,
    /// "clump", "trouble", "guard" or "shield": the trace's word.
    pub why: &'static str,
}
```

**The candidates.** For each facing - the tank's own first, then
`Dir::ALL` - the orb's line from the gun line's muzzle along it, the points
every `WELL_AI_STEP_PX` (16) from `well_ai_min_px` (160: past its own
pull's reach, the tank's hull never inside `R`) to the first of
`well_orb_range_px` and the orb's own stopper on that line (the first tile
that blocks sight, the field's edge, a live hull or a frog - the orb's
sweep, `Terrain::sweep`), the stopper's anchor point itself included. Each
candidate `P` is a well centre; "in the pull" means a hull box's nearest
point within `well_radius_px` of `P` (a frog's centre).

**Who counts.** A seat counts only if it is in the pull, live, on the
field, not hidden from this tank (`concealed` and not hit-alerted - the
attack tier's rule), within its sight under the sky (`Game::sight_on`), and
this tank stands inside its sight box (`ai::in_sight_box_of`) - so no well
is used on a seat from off its box. An ally is every live enemy - this one
included, any role, disabled or not - in the pull now or where its velocity
carries it by the time the pull would start (the orb's flight to `P` plus
`well_form_seconds`), with `well_ai_friend_margin_px` (16) added to the
radius; and the enemies' own frog in the pull.

**An anchor qualifies** by the first of these arms, in priority order:

1. **Clump** (`"clump"`): two or more seats count - it pulls two seats
   together.
2. **Trouble** (`"trouble"`): a seat counts and its straight line to `P`
   (sampled every 8 px, the hammer's `SONIC_TROUBLE_STEP_PX`) crosses a
   trouble cell, or a standing drum lies in the pull - a seat pulled into
   drums, lava, the water or a rod's circle.
3. **Guard** (`"guard"`): the players' frog is alive, a seat counts that
   stands within `well_ai_guard_px` (160) of it, the frog is out of the pull
   (its centre farther than `well_radius_px + well_ai_friend_margin_px` from
   `P`) and `P` is farther from the frog than that seat - a seat pulled off
   the frog it is guarding.
4. **Shield** (`"shield"`): this tank is hit-alerted (`Ai::is_hit_alerted`:
   it is being shot at), a seat counts by the rules above but for the pull -
   it is lined up on this tank (on its row or column within
   `enemy_fire_align_px`, facing it, with a line of sight) - the candidate
   lies on that line between them at `well_ai_min_px`, and the seat stands
   farther than `well_radius_px + well_ai_friend_margin_px` from `P`: the
   seat's shells fly into a well that swallows them (§1).

and passes every one of:

- **Never on its own group**: the allies in the pull are no more than the
  seats in it (for the shield, none at all) - "no anchor whose pull would
  drag more allies than seats". The tank itself never is (the candidates
  start past its reach).
- **Not twice**: no live well's centre within `2 * well_radius_px` of `P`.
- **One well per seat**: a seat a lower-slot well tank already planned for
  this frame is not counted again.

The first qualifying anchor in arm order, then facing order, then nearest
candidate, is the plan. A training dummy (`Ai::frog_only`) gets none. No
RNG anywhere: fixed priority, the facing it has, `Dir::ALL` order, distance,
the lower seat, the lower slot.

### The rule (`well_rule`), in priority order

1. **Its orb in flight** (`Tank::orb`): the rule owns the trigger until it
   anchors. `Anchor { face }` on the tick the orb has flown
   `Ai::well_plan`'s distance or more and anchoring there drags no more
   allies than seats (`WellSense::orb`'s flag); else `Hold { face, why:
   "orb" }` - the trigger released, so the next press is an edge. An orb
   whose plan never holds again flies on and anchors at its range or on
   what it meets - the room's rule, not the AI's.
2. **A training dummy**: `None`.
3. **Cooling** (`fire_cooldown > 0` or `Ai::fire_timer` running): `None` -
   the tree goes on.
4. **A plan**: `Orb { face, anchor_px, at_seat, why }`: it faces, fires, and
   sets `Ai::fire_timer` to `well_ai_fire_interval` (8).
5. Otherwise `None`: the tree goes on (chase, attack's repositioning,
   patrol and the seeks; never a shot).

**The tell** (§1): the orb's flight - at 150 px/s, at least a second from a
launch to an anchor 160 px out - and the snap and the forming ring
(`well_form_seconds`) before anything is pulled; off the screen, the zone's
arrow from the snap on (§5). No wind-up before the launch (the swarm's rule,
docs/fpv-swarm.md decision 12): the slow orb is the warning. The decision to
launch is the moment the sight box is checked and `shot_at_seat` recorded;
the anchor press records nothing.

**The hold-still clocks**: a tank holding for its orb commands no movement
(the stuck clock resets, as any deliberate hold), and the C2 commander does
not order it while it holds (`command::Busy::Anchoring`).

### The group fires into the clump

While a seat's centre is inside a pulling well owned by an enemy, the seat's
engagement ring (when it has two or more engaged) is built round **the
well's centre** with `EngageCtx::herd = Some(well_ai_herd_px)` (176) - the
rod's herd (docs/rod-from-god.md §4): the firing slots on the axes at that
distance from the core, outside the pull, facing in. The pack lines up round
the well and shoots into the clump the well holds; shots fired along an axis
at the core are not bent (they are aimed at it) and meet the clumped hulls
before the core. The moment the seat is out of the pull its ring is built
round the seat again.

### Reacting to a well (every enemy): the `pull` tier

`Game::pull_senses` runs once per frame in `enemy_phase`, only when a well
is forming or pulling, and hands every live enemy whose centre lies inside a
forming or pulling well's radius - its own included - a `PullSense`:

```rust
pub struct PullSense {
    /// The centre of the well whose pull is strongest on it (the lower id
    /// on a tie) and the field's pull there (`WellField::pull`, unit
    /// toward the core times the strength) - at the pull's start for a
    /// forming well.
    pub core: Position,
    pub pull: Vec2,
    /// It is heavy (`tank_mass_factor` at or over `well_ai_heavy_mass`,
    /// 1.2: breaker, longbow, obelisk, titan, leviathan) and its tracks
    /// hold broadside where it stands (`well::holds_broadside`: the side
    /// pull on it under its grip on this ground - the drive's own numbers,
    /// rain and ice included).
    pub brace: bool,
}
```

The `pull` tier (1.65, after `dodge`, before `flee`):
`condition(b.pull.is_some() && b.me.tell.is_none())`, `action("pull",
act_pull)`. Inside a pull **a special's use yields** - `special_rule`'s
first arm (`windup_rule`) answers a charge in progress (a rail, a reticle)
`Drop`, every other arm of every weapon answers `None`, the swarm's `air`
tier yields - the rod's rule for a call (docs/rod-from-god.md §3.2) - unless
the tank braces: a heavy chassis keeps its special's use (the special tier
comes first) and braces when its rule has none. A tell, half a second long,
commits. `act_pull`:

1. **Brace** (`brace`): it turns broadside - of the two cardinals across
   the pull, the one nearer its bearing to the seat it fights (ties to the
   one nearer its facing) - and holds still (`Ai::bracing`, `why:
   "brace"`): its tracks hold, and the current along them is nothing. A
   seat lined up along that facing is fired on as the attack tier fires
   (`hold_and_fire`, by `generic_fire`'s rule for what it carries), so a
   braced titan is still a gun. A heavy chassis stands its ground until the
   collapse or until the pull on it grows past its grip (it is dragged in,
   or a second well sums onto it), when it escapes as below.
2. **Across** (`"across"`): it drives the cardinal **perpendicular** to its
   bearing from the core (the larger offset's axis turned a quarter) on its
   own side of the core (the sign of the smaller offset; on the axis
   itself, the clockwise one), so its chord runs away from the core - if its
   way is open (`walls_ahead` and the next cell usable); else the other
   perpendicular; else straight away; else `Failure`. The way is latched
   (`Ai::pull_escape`) until the tank stands `enemy_danger_clear_px`
   outside every pull, and chosen again only when it is blocked. Steering
   holds the heading (`Ai::commit`); a perpendicular is never a turn round,
   so it never spins.

No RNG, ties on `Dir::ALL` order. `AiSnapshot::pull` names the arm. A
disabled enemy (the EMP) does not think and so does not react: it coasts on
its last intent and is dragged - the EMP-then-well combo. A far tank
coasting on its last intent is dragged the same way.

**Routing**: every nav cell of a forming or pulling well costs
`well_ai_route_cost` (24) more, saturating (`route_grid_on`, the zones'
`route_cells`), so routes go round a well rather than through it; it is no
danger (the dodge's way out would be radial - the wrong way) and no edge
hold. `Brain::seek` and `nearest_pickup` leave a crate inside a pull alone.

### With the commander (`c2_enabled`)

A tank in the `pull` tier is `command::Busy::Pulled` and one holding for its
orb `command::Busy::Anchoring` (the EMP's open list): never ordered, never
asked to give way. C2 off is untouched.

### What changes for enemies that carry something else

- The `pull` tier, the route surcharge and the herd, above.
- **The hammer's rule**: a seat whose predicted slide ends inside a pulling
  enemy well is in trouble (`lands_in_trouble`) - a hammer enemy shoves a
  seat into a well; a hammer closer's spot is not taken inside a pull.
- **The EMP's rule**: a seat carrying an online well is worth
  `emp_ai_special_value`, like any seat with an online special.
- **The rail's, the swarm's and the rod's rules**: a charge or a watch
  inside a pull yields as every wind-up does (above); the rod's pick
  counts a seat held in a well as standing still (it is - `SeatMotion`
  reads its centre), which is the rod-on-the-clump combo.

### Off the field and asleep

A field map's far tank coasting on its last intent has no sense: it fires
no well, and an orb it has out anchors itself. A sleeping tank starts
nothing. A disabled tank's orb, if it had one out, anchors itself.

## 5. Drawing

All of it in the effects language (docs/effects.md): whole 2 px blocks,
ramp steps, Bayer fades; composed at draw time as pure functions of the orb,
the zone, the held drum, the tank and their age, hashed from ids and cells,
never rolled. What warns - the orb, the snap, the ring, the swirl, the
rim - is drawn in the glowing pass, unlit, so it reads at night and in fog
(rule 7, and fair); what lingers on the ground is shaded in the lit pass.

- **A new ramp, `pyro::VOID`**, the Armory scene's violets, dark to bright:
  `#2C1A5C` `#6B3CC9` `#9A5CF0` `#C98CFF` `#F3DCFF` - off-palette energy,
  admitted as the plasma's and the laser's ramps are (code draws them; no
  sheet carries them). It is the plasma purple's family on purpose, a step
  deeper at the dark end, so the ring round the core reads as the rim of a
  hole. The core itself is `SMOKE[0]` (`#252525`, the palette's black).
  `effects.md`'s ramp table gains its row; `pyro`'s ramp test gains it.
- **The orb** (`well::compose_orb(orb, t)`), glowing pass: a disc of
  `VOID[1]` of 6 px with a `VOID[3]` disc of 4 px inside it and a `VOID[4]`
  glint block up and left; two `VOID[3]` blocks circling it 8 px out, a turn
  every half second, phased by its id; a trail of four blocks behind it
  along its heading, `VOID[2]` down to `VOID[0]`, the last two dissolving
  through the Bayer pattern; one stepped glow (`pyro::glow`, `VOID[2]`, 20
  px). In `Launch` the discs swell from 2 px at the muzzle. Online, a seat's
  own orb is its provisional shot (§8), drawn by the same composer.
- **The snap** (`well_anchor_show`, from `WellAnchored` - the room's for its
  tank, a replica's, a client's own on its press): a ring of `VOID[4]`
  blocks closing from 32 px onto the anchor point over 0.12 s and a white
  block at the point; the module's anchor cell; and **the ripple pinched
  inward** - `Shockwave::inward(centre, well_radius_px, well_snap_shock)`,
  a ring of bent image running in from the reach to the core over
  `well_form_seconds` (below): the reach drawn by the scene itself bending,
  before anything moves.
- **The well** (`well::compose_well(zone, now, t)`), glowing pass, by its
  stage:
  - *forming*: the core grows from one block to its full disc and the ring's
    arcs dissolve in from `cover` 0.3 to 1 over `well_form_seconds`; the rim
    dots appear;
  - *the core*: a disc of `SMOKE[0]` of 7 px inside a disc of `VOID[4]` of
    10 px - the bright rim of a black hole;
  - *the accretion ring*: two arcs of `pyro::Shape::Arc` at 13 px, two
    blocks wide - `VOID[2]` over 4.6 radians from `4t`, `VOID[1]` over the
    other 1.6 - turning clockwise at 4 rad/s (the scene's);
  - *the swirl* (`well::compose_swirl`): `well_particles` (48) blocks
    spiralling in - each at phase `ph = (0.7 t + hash(id, i)) mod 1`, radius
    `0.6 * well_radius_px * (1 - ph) + 8`, bearing `hash(id, 3i) * 2pi + 5 ph`
    (clockwise in), the vertical squashed to 0.8 (the ground's tilt);
    `VOID[0]` past 0.7 of the way in, `VOID[1]` past 0.4, `VOID[2]` before;
  - *the rim*: a dot of `VOID[1]` every 12 px of arc at `well_radius_px`, at
    `cover` 0.4, turning a turn every eight seconds - the reach, readable
    while it pulls;
  - *one glow* (`pyro::glow`, `VOID[2]`, 40 px) over the ring;
  - *the warning*: over its last second the core beats between 7 and 8 px at
    6 Hz and the swirl runs half again as fast.
- **Bent shots** turn with their heading: `Projectile::bend` sets the
  rotation their sprites are drawn by, so a shell visibly curves. Inside a
  pull a shot's tracer (`render/shot_fx.rs`'s streak) is drawn curved:
  `well::curved_streak(position, velocity, field)` steps back along the
  path it came by - four ticks through the field in reverse - and lays the
  streak's blocks along it in the tracer's own colours. Outside every pull
  the streak is as it ever was.
- **The swallow** (`Swallowed`, `well::compose_swallow`): a ring of
  `VOID[3]` blocks closing onto the point over 0.15 s and one `VOID[4]`
  block - a burst `burst.rs` composes (`ImpactKind::Swallow`), started by
  `fx.rs` off the event: no fire, no sparks, no pool of light, no impact
  flash.
- **Held drums** (`well::held_at`): each drum's sprite drawn lifted by its
  height over its shadow where the flying drum's is (the `FlyingDrum`
  drawing in `render/blast.rs`, by its kind), turning with its bearing, a
  fused one with its fuse's glow; a grenade on the ring is drawn as every
  grenade is, rolling round.
- **What the pull moves** is drawn where it is: a hull, a crate at
  `Pickup::at()`, a frog sliding in its hurt clip held on its first frame
  (pinned, flailing), a missile or a drone dragged. Nothing is drawn moved
  that is not.
- **The drain** (the "at 11"): tread marks and tufts drawn as ever, where
  `drain_marks` and `lean_grass` left them - the marks swirled toward the
  core, the tufts bowed into it; **particles** within a pulling well
  (`fx.rs`) take `WELL_FX_PULL` (90 px/s) times the field's pull and as
  much again clockwise round the core, so smoke off a burning hull, dust
  and sparks spiral in, and one that reaches the core is spent.
- **The collapse** (`well_collapse_show`, from `WellCollapsed`, staged
  through `Game::show` like the rod's impact):
  - *the implosion*: for 0.06 s the core shrinks to a block and a disc of
    `VOID[4]` of 24 px with a white core flashes over it;
  - *the fling*: `Shockwave::scaled(centre, well_shock)` (0.6 of a tank
    dying) - the outward ring; the screen flash at `well_screen_flash`
    (`flash_screen_with`, 0.6 of a drum's);
  - *the rings*: two rings racing out over 0.6 s - two blocks of `VOID[2]`
    at 220 px/s, one block of `VOID[0]` at 160 px/s - dissolving through the
    Bayer pattern (the scene's);
  - *one glow* (`pyro::glow`, `VOID[1]`, 80 px) gone in a third of a second;
  - *dust* (lit pass): ten `pyro::dust_puff`s thrown out from half the
    reach to the reach and leaning with the wind, gone in 0.8 s; the
    ground's dust (`DUST`, `SMOKE` over lava, `BLUE_PALE` chop over water,
    white over snow);
  - the drums' fireballs, the hulls' skid dust, the frogs' hops, the fish
    and the grass are their own shows.
  No ripple, flash or shake under reduced motion, and all of it scaled by
  `screen_fx_intensity`.
- **The shader**: `static/shockwave.fs` and its GLSL ES 100 twin
  `static/web/shockwave.fs` take `uniform float starts[4]` and `uniform
  float signs[4]`; a ripple's ring stands at `max(starts[i] + signs[i] *
  times[i] * speed, 0.0)` and is drawn as now (the same band, the same
  wobble, the same fade). An outward ripple is start 0, sign 1, which is
  every ripple that ships, so every frame without a well is the bits it
  was. `render::shockwave::RippleFx` packs the two arrays from `Shockwave`
  (`start`, `inward`) in ripple units; an inward ring counts in the camera
  shake as any ripple does. The two shaders are changed by hand, line for
  line (CLAUDE.md, the web section).
- **The light** (`weather::lights_in`): at night the ring throws an
  unshadowed point light of `VOID[2]`, 48 px, at `well_light` times the
  forming's progress; an orb one of 20 px; the core none - drawn in
  `SMOKE[0]` in the glowing pass, it is black on any ground under any sky.
- **The module** (`tankdesign`, `lines/vanguard.py`, `module_fn('well')`):
  a singularity projector on the roof - the missiles' hardpoint, shared, as
  a tank carries one special at a time (`hp.get('well', hp['missiles'])`):
  a squat gunmetal gimbal ring (6 x 5 design px, chamfered) cradling a dark
  sphere (`BLACK` with a `STONE_DARKEST` rim), a small lamp either side of
  the ring and a steel collar forward. Four cells, `TANK_MODULE_WELL_COL`
  = the four columns after the rod's (49..52 if the rod's lands at 44..48;
  `tank_modules.png` grows by four columns, 2120 x 480): 0 armed, the lamps
  dim (`DIM_ION`), the sphere seated; 1 launch, the cradle empty and its
  ring lit in the light layer (`'white'`); 2 anchor, both lamps lit
  `'white'`, the sphere back; 3 offline, the ring scorched (`RUST_DK` for
  steel's light steps), no lamps. The violet is the effects' alone; the
  module stays on the palette and its light is the kit's `'white'` role, so
  `just check-sheets` needs no new admission. `module_cols` (now twelve
  entries): 1 for `well_flash_seconds` after a launch (`Tank::kick_well` -
  the room's, a replica's `Fired`, a client's press), 2 for as long after
  an anchor (`Tank::kick_well_anchor` - the room's, a replica's
  `WellAnchored`, a client's press), 3 while `special_down()`, else 0.
  Anchor: `tank_art::WELL_CRADLE` (the sphere's centre, turret frame),
  written by `export.py` from the module's `meta['cradle']` - drawn only:
  the orb leaves the gun line's muzzle. `render.SHOWN_TOGETHER` leaves it
  out with the other roof modules. Three chassis in a screenshot before it
  is settled (§12).
- **The crate**: row 18 of `gen_crates.py`'s sheets (`crates_sheet.png`
  280 x 760, `pickup_glyphs.png` 24 x 456). Its symbol, 10 x 10 design px,
  **two-tone** like the heat shield's: a black hole - its void (`v`, painted
  in the ink's shade), ringed (`X`, the base), crossed by its accretion disc
  (`o`, the light):

  ```
  '....XX....',
  '..XXvvXX..',
  '.XvvvvvvX.',
  '.XvvvvvvX.',
  'oooooooooo',
  '.XvvvvvvX.',
  '.XvvvvvvX.',
  '..XXvvXX..',
  '....XX....',
  '..........',
  ```

  `gen_crates.py` paints a `v` in the ink's shade on the crate and the
  token alike (the token's top-lit and bottom-shaded steps apply to `X`
  and `o` only), so the void stays black. Ink
  (`punypalette.PICKUP_INK['gravity_well']`, admitted on the crate sheets
  alone like the others): shade `#1C1033` (the void), base `#E6A8FF` (a
  pale ultraviolet lilac), light `#FFF0FF`. No hue that is not green is
  free (the rod's doc measured it), so the pick is by colour distance and by
  the void: the base is 30 in CIELAB from its nearest ink (the tower pack's
  periwinkle) and 31 from the shield's lavender - wider than the
  health/heat-shield (16) and shield/EMP (21) pairs that ship - and the
  only crate with a black symbol. Stand-by: `#F0C0FF` (29 from the tower
  pack). To be shown beside the other eighteen in a screenshot before it is
  settled, after the rod's and the rail's final picks (§12).
- **The HUD**: `hud::WeaponSlot::of` gives the wells in `HUD_WELL_COLOR`
  (`#E6A8FF`, the ink's base) and the glyph; offline, `WPN OFFLINE`; the
  ring's ammo pips are the wells left against `full_load` (3). **The anchor
  prompt** (`hud::well_prompt`): while a seat on this screen has an orb in
  flight, one line under that seat's block (the rod's `Corners::prompt`
  slot), centred, at `UI_SMALL_TEXT`, in the ink's light: `hud-well-anchor`
  (§7). A couch's second seat's under its own block.
- **Off the screen** (`indicators.rs`): `ArrowKind::Zone { kind: Well, left
  }` for every forming or pulling well off this screen and not owned by a
  seat on this screen: a notched arrowhead in `VOID[3]`, rimmed near-black,
  the whole seconds to the collapse on its plate while it pulls, blinking
  at 2 Hz and at 6 Hz in its last second; never merged, never left out past
  `indicator_max_arrows` (the rod's rule). An orb needs no arrow: an
  enemy's launcher stands inside the sight box of the seat it is used on,
  which is always on that seat's screen.
- **The minimap**: a well is a mark - a ring of `VOID[3]` texels round its
  cell, `VOID[1]` while forming, pulsing at 2 Hz.
- **The dev overlay** (`Overlays::engage`): a well's reach and its herd
  ring.

## 6. Tuning

New group `well` (every row live unless marked), plus one row in the
enemies' group:

| Row | Default | Range | Doc |
|---|---|---|---|
| `well_per_pickup: i32` | 3 | 1..=10 | Wells one crate loads. One per launch; the anchor costs nothing. |
| `well_reload_seconds` | 1.5 | 0..=10 | After a launch, seconds before the next launch. The anchor press is never held back by it. |
| `well_orb_speed` | 150 | 20..=1000 | How fast the orb flies (px/s): slower than a seat drives, about an enemy's pace. |
| `well_orb_range_px` | 320 | 32..=1000 | How far it flies before it anchors itself. |
| `well_orb_half_px` | 6 | 1..=16 | Half its width, what its sweep meets things by. |
| `well_anchor_grace_seconds` | 0.4 | 0..=2 | A press this soon after an orb anchored by itself fires nothing: it was meant for the orb. |
| `well_form_seconds` | 0.4 | 0..=3 | From the anchor to the pull: the snap and the ring forming, nothing pulled. |
| `well_pull_seconds` | 4.0 | 0.5..=15 | How long it pulls before it collapses. |
| `well_radius_px` | 128 | 32..=480 | How far it reaches (px; four cells). |
| `well_core_px` | 12 | 2..=48 | The core: the pull is strongest at its rim and fades to nothing at the centre; a shot, missile, drone or orb that passes inside it is swallowed. |
| `well_current_speed` | 320 | 0..=1000 | Along a hull's tracks, the current at the strongest point for the chassis-free mass (px/s): the drive runs in its frame. |
| `well_side_pull` | 650 | 0..=5000 | Across a hull's tracks, the pull at the strongest point (px/s²) the tracks' grip holds up to its limit. |
| `well_mass_exponent` | 2.0 | 0..=4 | How hard a heavy chassis resists: the current and the side pull over the mass factor to this power. |
| `well_shot_pull` | 3600 | 0..=20000 | The pull on shells, bullets, plasma bolts and orbs at the strongest point (px/s²): it turns their heading, never their speed. |
| `well_air_pull` | 1800 | 0..=20000 | The pull on a missile's or a drone's ground motion (px/s²). |
| `well_grenade_pull` | 600 | 0..=5000 | The pull on a grenade's ground motion (px/s²). |
| `well_ring_px` | 24 | 8..=64 | The ring captured grenades and held drums circle on. |
| `well_orbit_speed` | 3.0 | 0..=20 | How fast they circle it (rad/s, clockwise). |
| `well_capture_seconds` | 0.8 | 0.05..=4 | How long a lifted drum takes to spiral from its cell to the ring. |
| `well_crate_speed` | 60 | 0..=400 | How fast a crate slides in at the strongest point (px/s). |
| `well_frog_speed` | 50 | 0..=400 | How fast a frog slides in at the strongest point (px/s). |
| `well_fling_speed` | 400 | 0..=508 | The collapse's shove at the centre (px/s, chassis-free mass), falling to nothing at the reach. |
| `well_fling_max_speed` | 480 | 0..=508 | The most any fling gives; the wire's shove reaches 508. |
| `well_fling_mass_exponent` | 1.5 | 0..=4 | How hard a heavy chassis resists the fling. |
| `well_collapse_damage` | 10 | 0..=100 | The collapse's damage at the centre to the side opposing its owner, falling to nothing at the reach; no roll. |
| `well_fling_grenade_speed` | 220 | 0..=1000 | How fast the collapse throws a grenade off the ring. |
| `well_fling_grenade_climb` | 160 | 0..=2000 | And how high it lobs it (the grenade's `climb`). |
| `well_fling_crate_speed` | 240 | 0..=1000 | How fast it slides a crate out at the centre. |
| `well_crate_friction` | 600 | 1..=5000 | How hard a sliding crate brakes (px/s²). |
| `well_fling_frog_cells: i32` | 3 | 1..=8 | How far it hops a frog out from the centre, falling off with distance, at least one. |
| `well_fish_throw_max: i32` | 6 | 0..=32 | Fish in its reach thrown onto the bank at the collapse, nearest first. |
| `well_mark_speed` | 30 | 0..=400 | Tread marks in its reach creep toward the core at this at the strongest point (px/s; cosmetic). |
| `well_mark_twist` | 1.2 | 0..=10 | And turn about it at this (rad/s; cosmetic). |
| `well_grass_lean_px` | 6 | 0..=16 | Tall grass in its reach leans into it by this at the strongest point (cosmetic). |
| `well_particles: i32` | 48 | 0..=200 | Blocks in its swirl. |
| `well_snap_shock` | 0.35 | 0..=2 | The inward ripple at the anchor, against a tank dying's. |
| `well_shock` | 0.6 | 0..=2 | The collapse's ripple and shake, against a tank dying's. |
| `well_screen_flash` | 0.6 | 0..=8 | The collapse's screen flash, against a drum's. |
| `well_flash_seconds` | 0.25 | 0..=2 | The module's launch and anchor cells. |
| `well_light` | 0.6 | 0..=2 | The light the ring throws at night, against a headlight's. |
| `well_ai_fire_interval` | 8 | 0.1..=60 | Seconds between an enemy's decisions to launch. |
| `well_ai_min_px` | 160 | 64..=1000 | The nearest an enemy anchors a well, from its gun line's muzzle - past its own pull and the friend margin -, and where its shield stands. |
| `well_ai_guard_px` | 160 | 0..=480 | A seat this close to the players' frog is its guard, which an enemy pulls off it. |
| `well_ai_friend_margin_px` | 16 | 0..=128 | Added to the reach when an enemy counts the allies a pull would drag. |
| `well_ai_heavy_mass` | 1.2 | 0.1..=5 | A chassis this heavy (mass factor) braces broadside in a pull while its tracks hold. |
| `well_ai_route_cost: usize` | 24 | 0..=255 | Extra route cost on every cell of a forming or pulling well; 0 switches it off. |
| `well_ai_herd_px` | 176 | 0..=480 | How far from a well's centre the pack's firing slots stand while the seat they fight is in its pull. |
| `enemy_special_weapon_well_share` (`enemies`, `@ Restart`) | 0 | 0..=1 | The share of special-carrying enemies that spawn with the gravity well instead, decided by a hash of the spawn point and the slot - never the round's RNG - so at 0 nothing changes. |

Constants (geometry and policy, not feel): in `well.rs` `WELL_AI_STEP_PX`
(16), `WELL_FX_PULL` (90), the composers' sizes and rates above (the orb's
6 and 4 px, the ring's 13 px and 4 rad/s, the snap's 32 px and 0.12 s, the
collapse's rings); `WELL_SIDE_REACH_SECONDS` (0.5) beside
`POSE_REACH_SLACK_PX` (server policy, the validator's).

## 7. Text

| Key | en | sl |
|---|---|---|
| `hud-well-anchor` (`keys::HUD_WELL_ANCHOR`) | FIRE AGAIN TO ANCHOR | ZNOVA SPROŽI ZA SIDRO |
| `tool-gravity_well` | gravity well | gravitacijska jama |
| `tool-short-gravity_well` | well | jama |

`FIRE` names the trigger on a keyboard and the fire half on a touch screen
alike, so the prompt needs no `-touch` twin (the rod's `LET GO`). `ZNOVA
SPROŽI ZA SIDRO` ("fire again for the anchor") folds to `ZNOVA SPROZI ZA
SIDRO`; `gravitacijska jama` ("gravity pit"). **The budget**:
`every_language_fits_every_budget` gains the prompt at `UI_SMALL_TEXT`
within the block's width less 8 (`Corners::prompt`, the one-seat block on
the smallest area) - the rod's measure; the tool names are measured by the
existing tool budgets (144 and 48). Measured in Phase 2; `SPROŽI ZA SIDRO`
stands by for Slovene if the line runs over, and `gravitacijski vodnjak`
is not used (too long for the 144). The countdown on the well's arrow is
digits: no key.

## 8. Wire

Protocol 20 (from the rod's 19), once in the PR.

- `WeaponKind::GravityWell`, appended to `ALL`; `drawn_on_press` false: its
  launch is a shot, drawn as provisional shots are, and its anchor is
  claimed by the orb's id (below).
- **`ShotKind::Orb`**, appended: the orb travels as a `ShotState` - its id,
  position, heading, `state` (0 launch, 1 flying), `variant` 0, `owner` its
  seat or `NO_SEAT`. A replica spawns a `well::Orb` from it, its velocity
  its heading times `well_orb_speed`, interpolated as a shell is.
- **Bent shots already travel**: `ShotState` carries every shot's position
  and heading each snapshot, and a replica's velocity is its heading times
  its kind's speed - exact, since a bend keeps the speed. A shot whose
  heading changed travels whole rather than as a `Moved` (a byte or two
  more, only while it bends). Between two snapshots the interpolator blends
  positions linearly - the chord of two ticks of curve, under half a pixel
  off at these bends - and the heading snaps, as it always does.
- **`ZoneState`** (the rod's family): `kind` 1 is a well (`ZONE_WELL`);
  `stage` 0 forming, 1 pulling; `until` the stage's end, round-clock ticks;
  `owner` its owner's slot. A well's entry changes once (the stage turns)
  and goes at the collapse.
- **A new family, `Snapshot::well_drums`**, keyed by id:

  ```rust
  /// A drum a well holds (`well::HeldDrum`), by its id. Never moves: where
  /// it is is `well::held_at` of its well's centre and the round clock.
  pub struct WellDrumState {
      pub id: u16,
      /// The well's zone id.
      pub well: u16,
      /// The cell it was lifted from, `row * cols + col`.
      pub cell: u16,
      /// `Drum`, its kind.
      pub drum: u8,
      /// Its fuse is burning (the replica runs it down).
      pub fused: bool,
      /// The tick it was lifted, round clock.
      pub tick: u32,
  }
  ```

  `delta.rs`: `well_drums`, `well_drums_gone`. An entry travels once (and
  once more if its fuse is lit) and goes when the drum does.
- **`CrateState`** gains `dx: i16, dy: i16` (the drift, quarter pixels); a
  crate is listed while it is not whole **or drifted**. The interpolator
  blends a listed crate's drift between snapshots (keyed by its cell), so a
  slide is smooth.
- **`frog_flags::PULLED`** (bit 5): a replica holds `Frog::pulled` while it
  is set.
- `GrenadeState`, `MissileState` and `DroneState` need nothing: a captured
  grenade's, a dragged missile's and a dragged drone's positions travel as
  they always do. `TankState` needs nothing: `weapon` is the special
  carried, `ammo` the wells; the module's launch cell rides `Fired`'s
  `kick_turret`, its anchor cell `WellAnchored`.
- **Events** (`net/events.rs`, mirrors of the simulation's):
  `WireEvent::WellAnchored { id: u16, slot: u16, seat: u8, x: i16, y: i16,
  by: u8 }` (`seat` `NO_SEAT` for an enemy's; `by` `AnchorBy`),
  `WellCollapsed { id: u16, x: i16, y: i16, early: bool }`, `Swallowed {
  what: SwallowedKind, x: i16, y: i16 }` (`SwallowedKind`: shell, bullet,
  plasma, orb, missile, drone) and `OrbFizzled { id: u16, x: i16, y: i16 }`.
  `HitCause::Well` and `AirStrike::Well`, appended.
- **What a replica draws**: wells from the zones family (`apply_zones`:
  added, turned, dropped by id) - the core, the ring, the swirl and the rim
  by `compose_well` from the stage and `until`; its field
  (`WellField::at` on the replica's clock) for the cosmetic drain alone
  (`tick_well_pictures`: the marks, the grass, the particles, the fish) -
  a replica pulls no hull, shot or crate: those arrive in the snapshots;
  held drums from their family (`held_at`); orbs from the shots; on
  `WellAnchored`, `well_anchor_show` and the module's anchor cell; on
  `WellCollapsed`, `well_collapse_show` and the fish thrown
  (`fish::throw_from`); on `Swallowed` and `OrbFizzled`, the pop. The
  drums' blasts, the hits, the shoves, the frogs' hops and the tile deaths
  come from their own events and state, as every blast's do.
- **What is drawn at once** (decision 3 of BB-36, the hammer's §3.3):
  - *the orb, on the press*: `Predictor::pull_trigger`'s well arm, on the
    press edge, with no orb of this seat's in flight (a provisional one, or
    the room's copy of one), wells left less the owed, the gate open and
    the special up (`offline_left` 0): a provisional shot
    (`Game::seat_shot(seat, ProvisionalKind::Orb, ..)` - the gun line's
    muzzle off the sandbox's pose, along its facing), owed, the gate set to
    `well_reload_seconds`, the module's launch cell (`kick_seat`). It flies
    by the orb's own state machine on the tick grid (§3.2), bent by the
    present field, swept against the drawn world (`shot_contact`), and is
    paired with the room's orb by the `Fired` that names its input tick, as
    every provisional shot is; the room's copy is kept off the picture.
  - *the anchor, on the second press*: a press edge while this seat's orb
    flies anchors the provisional where it stands on that tick - the room
    anchors its copy where it stands on the tick it applies the press, and
    an owned seat's intents are played out one a tick, so the two stand
    within a tick's flight (2.5 px) - and puts on a **provisional well**
    (`Game::add_provisional_zone`: a forming well there, its `until` the
    press's round tick plus `well_form_seconds`) with `well_anchor_show`
    and the module's anchor cell, recording the anchor against the orb
    (`Predictor::drawn_anchors`). No well spent, no gate. A provisional orb
    whose sweep meets something in the drawn world, or that reaches its
    range, anchors there the same way (`Contact`, `Range`).
  - *the claim*: under `Show::OwnShotsDrawn`, the room's `WellAnchored`
    whose id is an orb of this seat's with a drawn anchor (the orb's room
    id, known from its pairing by `Fired`, which is always handed over
    first) is neither drawn again nor handed on to `game.events`, and
    `Predictor::pair_zone` (the rod's) pairs the provisional well with the
    family's well of that id: the provisional goes and the room's is shown,
    on the same spot within a tick's flight - or, where the room's orb met
    something the drawn world did not have, where the room has it. Until
    then the family's wells of this seat no anchor pairs are kept off the
    picture (`hide_zones`). A drawn anchor nobody claims within the refusal
    wait goes with its provisional well; a room anchor this client never
    drew (the room's orb met something first; its range) is drawn from the
    event and the family. A press that reached the room within
    `well_anchor_grace_seconds` of an anchor of its own (§1) fires nothing
    there, so a late anchor press never costs a well.
  - *the pull on its own hull*: the sandbox's footing (`predict_seat`'s
    `hull_footing`) reads the field built from the sandbox's zones - the
    room's, taken with the snapshot by `apply::snapshot` - plus this seat's
    provisional well, on the round tick its input lands on (§3.3), so an
    owned hull is pulled from the tick the room pulls it and every pose
    carries the pull; the room's validator allows the drift
    (`accept_seat_pose`, §3.2). A stage-2 replay pulls the same way from the
    room's exact count.
  - *its own shots bent, and everyone else's*: this seat's provisional
    shots bend by the present field on the tick grid and the incoming carry
    steps foreign shots through it (§3.2), so every shot is drawn on the
    room's curve. **The pairing is by `Fired`, never by position**, so a
    bent room copy stays paired with its provisional and hidden - it is
    not "missed". Where the two part - a well this client did not know of
    yet, anchored within its round trip - the room's copy bursts away from
    the provisional's stop or flies on past it, and is shown instead
    (`crossings_missed`, the existing rule); a provisional swallowed in the
    drawn world is kept off the picture until its copy goes, as a struck one
    is.
  - *the collapse*: the room's, drawn when its snapshot is handed over. The
    fling reaches an owned hull as `Shoved` with its skid (the hammer's
    rule) and the validator allows it (`seat_knock`); the sandbox stops
    pulling the hull on the collapse's tick, which the zone's `until`
    tells it. An EMP's early collapse reaches it a round trip late (the
    zone gone, `WellCollapsed { early }`): for that round trip its sandbox
    pulls a hull the room has stopped pulling, so a pose within the
    chassis's own reach is taken and one past it is refused and answered
    with `Placed` - the validator's ordinary correction, and the round
    trip of pull it undoes.
- **What stays the room's**: the orb's flight and what it meets, where and
  when it anchors, the stages, every capture, every swallow, the collapse
  and all it does (`Hit`, `Wreck`, `Shoved` with skids, `Blast`, the frogs'
  hops, the crates' drifts), the drums' blasts.
- `delta.rs`: the family's two lists; its random snapshots fill it and the
  crates' drift, and the size bounds are re-measured.

## 9. Determinism

- **The well draws no RNG of its own.** The orb flies by arithmetic and
  its sweep rolls nothing (no pass-over, no deflection); the anchor is a
  position; the field, the bend and every drag are arithmetic; a capture is
  a distance; the lift and the orbit are functions of the round clock; the
  collapse's damage has no roll; the fling is a knock; a frog's hop out is
  computed (`frog_fling_target`), with no jitter. What it sets off draws as
  it always does: a held drum's blast its damage rolls and its pool's
  chance, a grenade's burst its rolls - where they go off.
- **Fixed walks**: wells in id order, the field's sources in id order; a
  collapse's walk as §1 (drums by id; hulls - seats by index, then enemies
  by slot; frogs, the players' then the enemies'; grenades, shots (shells,
  bullets, plasma, orbs), missiles by id; drones by key; crates by cell);
  the lift in cell order; ties to the lower well.
- **Rapier's order.** The pull adds no body, no collider and no call of its
  own to a hull: it is part of the one impulse the drive puts on each hull
  each tick (`drive_tank_with`'s `apply_impulse`), worked out in one
  expression, in the drive's fixed order (seats by index in `player_phase`,
  then the enemies' apply pass by slot). The collapse's knocks go through
  `knock` in `knock_from`'s walk (seats, then enemies by slot). A frog's
  collider moves by `set_position` in `well_phase`, frogs in their fixed
  order. Rapier's step is deterministic for the same sequence of calls on
  the same world - its sets are arenas, its islands and its solver iterate
  in insertion order - and impulses on different bodies commute, so the
  order of the calls is the one thing to fix, and it is fixed. No call
  depends on a `HashMap`'s order.
- **Platform**: the field and the bend are square roots and divisions
  (exact in IEEE), summed in a fixed order; a hull's mass exponent is the
  `powf` the hammer's shove already takes, and the orbit and the swirl use
  `sin`/`cos` as every rotation in the simulation already does. The room is
  the authority online, and the probe's bit-exact pair runs on one
  platform.
- **The AI**: `well_rule` and `well_senses` choose by fixed priority, then
  facing order, then distance, then the lower seat and slot; `pull_senses`
  by strength, then id; `act_pull` in `Dir::ALL` order.
- **The swap** is the hammer's hash; the new entry runs only with its share
  above 0.
- **A round without the well replays byte for byte**: no crate kind is
  rolled anywhere, every share defaults to 0, no enemy carries one, so no
  orb flies and no zone is a well; `Frame::wells` is empty, and every
  reader skips it (no `bend` is called, `drive_tank_with` does not add the
  term at all - not even a zero, which could turn a `-0.0` into `+0.0` -,
  `roll_grenades`, the drag and the drain do nothing); no drum is lifted,
  no crate drifts (`Pickup::at()` is its position), nothing is swallowed;
  the `pull` tier is a `false` with no state touched; no surcharge, no
  herd; every ripple's start is 0 and its sign 1, so the shader's ring is
  `times * speed` as it was. `determinism_tests`' pinned streams, the probe
  fixtures' ceilings and every thumbnail pin but the armory's stay as they
  are.

## 10. Tests

`simulation/well_tests.rs` (headless, tiny inline maps or the default
field, the seat at (3, 6) facing east, parked enemies placed by hand; the
`well_at` tool's `Game::debug_well` to stand a well without steering an
orb):

- The weapon: `a_well_crate_arms_the_well_and_replaces_the_special_carried`
  (three a crate, another special emptied, a refill to three),
  `the_press_fires_a_slow_orb_and_spends_one_well` (`Fired` and the orb
  in one tick, at `well_orb_speed` along the facing from the gun line's
  muzzle), `a_second_press_anchors_the_orb_where_it_is` (the zone at the
  orb's point, `WellAnchored { by: Press }`, no well spent),
  `the_anchor_press_is_not_held_back_by_the_reload`,
  `an_orb_anchors_on_the_first_wall_tank_or_frog_it_meets_and_floats_over_sandbags_and_fences`
  (backed off its half width; `by: Contact`; the shooter's own hull never
  met), `an_orb_never_anchored_anchors_itself_at_its_range`,
  `a_press_just_after_an_orb_anchored_itself_fires_nothing`,
  `an_emped_shooter_cannot_anchor_and_its_orb_anchors_at_range` (the press
  fires a shell), `a_wrecked_shooters_orb_flies_on_and_its_well_is_its_own`
  (the collapse's kill credited to the wreck).
- The stages: `the_ring_forms_then_the_pull_runs_then_it_collapses` (no
  pull while forming; `until` the stage's end; the zone gone at the
  collapse), `strength_is_strongest_at_the_cores_rim_and_nothing_past_the_reach`.
- Hulls: `a_stopped_hull_rolls_with_the_current_along_its_tracks` (facing
  the core: in at the current's speed),
  `a_hull_broadside_holds_while_its_grip_beats_the_side_pull` (and slides
  where it does not), **`deep_in_driving_across_escapes_and_driving_away_does_not`**
  (a standard enemy at 50 px and a standard seat at 40: away into the core
  or held, across out - the §1 numbers, run through `drive_tank_with` at
  60 Hz), `a_scout_is_dragged_and_a_titan_stands`,
  `the_pull_reaches_both_sides_and_the_shooter_and_never_a_wreck_or_a_rolling_in_tank`,
  `a_skidding_hull_is_carried_by_the_whole_current`,
  `rain_and_ice_loosen_the_bite` (a standard hull broadside that holds dry
  slides on ice), `a_hull_dragged_into_a_portal_goes_through_and_is_out_of_the_pull`,
  `the_pull_runs_once_a_tick_in_the_drives_order` (the impulse sequence
  logged in a test build: seats by index, then enemies by slot).
- Shots: `shells_bullets_and_plasma_bend_toward_the_core_at_their_own_speed`
  (speed unchanged to the bit, heading turned, rotation set),
  `a_shot_that_reaches_the_core_is_swallowed` (`Swallowed`, no `Hit`, no
  burst), `a_shot_bent_into_a_tank_hits_it`,
  `a_clump_at_the_core_is_hit_before_the_core_swallows`,
  `the_laser_and_the_rail_are_not_bent`,
  `flying_drums_lava_bombs_and_globs_are_left_alone`,
  `an_orb_in_another_wells_core_is_swallowed`.
- The air: `a_missile_is_dragged_and_one_reaching_the_core_is_swallowed_without_a_blast`,
  `a_drone_is_dragged_off_its_dive_and_downed_at_the_core`
  (`DroneDowned { by: Well }`).
- What it gathers: `grenades_roll_in_and_circle_the_ring`,
  `a_captured_grenade_whose_fuse_ends_bursts_on_the_ring`,
  `the_collapse_throws_the_grenades_out_in_a_ring` (each along its own
  radial, fuses kept),
  `drums_in_reach_are_lifted_and_go_off_together_at_the_collapse` (the
  tiles dead with no blast, `ObstacleDestroyed`; the blasts on one tick, in
  id order, where `held_at` puts them),
  `a_blast_over_a_held_drum_sets_it_off` (one fused drum's takes the rest),
  `a_fused_drum_is_lifted_with_its_fuse`,
  `crates_slide_in_and_out_and_keep_their_slot` (taken where it lies; the
  slot respawns; `respawn_from_slots` draws what it drew),
  `a_crate_stops_at_a_wall_deep_water_and_the_edge`,
  `the_frog_slides_in_cannot_hop_and_is_flung_out` (no jitter: the hop's
  target `frog_fling_target`'s), `a_wreck_a_tile_and_a_lantern_are_left_alone`.
- The collapse: `the_collapse_flings_hulls_hurts_only_the_other_side_and_skids`
  (a seat's fling on `Frame::shoves` with its skid),
  `a_shield_soaks_the_collapse_not_the_fling`,
  `what_flies_in_the_reach_is_turned_out`,
  `an_emp_collapses_an_anchored_well_and_fizzles_an_orb` (`early: true`),
  `two_wells_pull_together_and_each_keeps_what_it_captured`,
  `the_well_on_the_end_screen_pulls_nothing_and_its_drums_hurt_nobody`.
- The drain: `tread_marks_swirl_in_and_grass_leans_in` (deterministic, on
  the room and through `tick_presentation` alike).
- Determinism: `the_well_draws_no_rng` (the RNG's state after a well that
  pulls hulls, bends shots and collapses, with no drum, frog or grenade:
  unchanged), `a_round_with_the_well_replays_bit_for_bit`,
  `the_spawn_swap_hands_out_the_well_by_its_share_and_draws_nothing`,
  `an_enemy_takes_the_crate_only_with_no_special`.
- Online in the simulation: `an_owned_hull_is_allowed_the_pull`
  (`accept_seat_pose` takes a pulled pose past the top speed and refuses
  one past the allowance), `the_sandbox_pulls_its_own_hull_on_the_rooms_tick`.

AI (`ai.rs` unit tests on a `Brain` with a made-up `WellSense`/`PullSense`,
and scenario tests on a whole round):

- `an_enemy_anchors_where_two_seats_are_pulled_together`,
  `an_enemy_anchors_to_pull_a_seat_into_drums_lava_or_water`,
  `an_enemy_anchors_to_pull_a_guard_off_its_frog`,
  `an_enemy_under_fire_shields_itself_on_the_seats_line`,
  `an_enemy_never_anchors_where_its_pull_drags_more_allies_than_seats`
  (itself, a fellow enemy, one driving in, the enemies' frog),
  `an_enemy_presses_the_anchor_at_its_planned_distance` (released the
  ticks between, so the anchor is an edge),
  `one_well_per_seat_and_never_twice_on_a_spot`,
  `an_enemy_never_uses_a_well_on_a_seat_from_outside_its_sight_box` (whole
  round: `offbox-fire`'s reading), `a_seat_hidden_in_grass_is_not_counted`,
  `a_training_dummy_never_fires_a_well`, `the_generic_tiers_never_fire_the_well`.
- `an_enemy_in_a_pull_drives_across_on_its_own_side` (the perpendicular
  away from the core's axis; the other when walled; latched; out past the
  clear margin), `a_heavy_enemy_braces_broadside_and_escapes_once_dragged`,
  `a_light_enemy_never_braces`,
  `an_enemy_charging_a_rail_in_a_pull_drops_it_and_escapes`,
  `a_tell_in_a_pull_commits`, `a_disabled_enemy_in_a_pull_is_dragged`,
  `routes_go_round_a_well`, `the_pack_fires_into_a_clump_held_by_a_well`
  (the ring round the core at `well_ai_herd_px`, every slot outside the
  pull), `the_hammer_shoves_a_seat_into_a_well`.

Shared path and presentation:

- `well::tests`: `strength_peaks_at_the_cores_rim_and_falls_to_the_edge`,
  `the_field_sums_in_id_order`, `bend_keeps_the_speed_to_the_bit`,
  `core_hit_meets_the_core_between_ticks`, `hull_pull_splits_by_the_axis`,
  `holds_broadside_is_the_drives_grip`,
  `escape_dir_is_across_on_the_hulls_side`, `held_at_spirals_in_then_circles`,
  `orbit_at_is_on_the_ring`, `frog_fling_target_walks_back_to_a_fit`,
  `curved_streak_follows_the_bend`; the composers
  `the_orb_is_on_the_grid_and_pure`, `the_well_draws_its_stages`,
  `the_swirl_is_hashed_and_spirals_in`, `the_collapse_is_gone_by_its_end`.
- `pyro`: `VOID` in the ramps' luminance test.
- `shockwave`: `an_inward_ripple_contracts_and_an_outward_one_is_unchanged`
  (`RippleFx`'s packed arrays: start 0 and sign 1 for every shipped kind).
- `pickup`: `name`/`parse`; `weapon`; `at_is_position_plus_drift`.
- `tank` (`weapon_inventory_tests`): the well in `take_weapon`, `full_load`,
  `special`, its trigger; `anchor_press`; the module's cells.
- `frog`: `a_pulled_frog_cannot_hop`. `fish::tests`:
  `a_well_draws_the_fish_and_the_collapse_throws_them`.
- `weather`: `the_ring_throws_violet_light_and_the_core_none`.
- `hud_tests`: the well's slot, colour and glyph;
  `the_anchor_prompt_shows_while_the_orb_flies`.
- `indicators`: `a_well_off_screen_gets_an_arrow_the_cap_never_drops`,
  `a_seats_own_well_gets_none`.
- `minimap_tests`: `a_well_is_a_mark`.
- `engage`: `a_herd_round_a_well_stands_outside_its_pull`.
- `devserver`: `set_tank_arms_the_well`, `well_at_stands_a_well`, the
  snapshot's new fields, `spawn_pickup` with `gravity_well`, the PICKUP
  category's count (18 to 19).
- `editor`/`chrome_tests`: the tool in `TOOLS`.
- `thumbnail`: the armory's pin. `maplint`: the armory as it lints.
- `text_tests`: every budget, the prompt in both languages.

Wire:

- `events.rs`: the samples gain `WellAnchored`, `WellCollapsed`,
  `Swallowed`, `OrbFizzled`, a well `Hit`, a well `DroneDowned`; the
  variant count.
- `delta.rs`: `well_drums_round_trip_through_the_delta`,
  `a_drifted_crate_round_trips`, the random snapshots and the size bounds.
- `apply.rs`: `a_well_reaches_the_replica_and_collapses_with_its_show`,
  `held_drums_reach_the_replica_and_a_joiner` (at `held_at`'s places),
  `an_orb_reaches_the_replica_as_a_shot`,
  `an_anchor_this_client_drew_is_not_drawn_again` (`OwnShotsDrawn` with the
  orb's id: no snap, not handed on; without: drawn),
  `a_pulled_frog_and_a_drifted_crate_reach_the_replica`.
- `predict.rs`: `the_orb_is_drawn_on_the_press`,
  `the_anchor_is_drawn_on_the_second_press_where_the_rooms_will_be` (within
  a tick's flight), `a_provisional_shot_bends_as_the_rooms_copy_does` (the
  same positions on the tick grid at 30, 60 and 144 Hz frames),
  `a_provisional_shot_swallowed_is_kept_off_until_its_copy_goes`,
  `the_sandbox_pulls_the_owned_hull_from_the_rooms_tick`.
- `round.rs`: `a_bent_room_shot_is_paired_and_hidden_not_missed`,
  `an_incoming_shot_is_carried_through_a_well`,
  `the_rooms_well_is_hidden_until_paired_with_the_drawn_one`.
- `rig.rs` (`Lockstep`): `a_seats_orb_and_anchor_are_drawn_once`,
  `an_enemys_well_reaches_the_replica` (the orb, the snap, the pull on the
  seat, the collapse), `an_owned_hull_in_a_well_is_placed_where_its_client_says`;
  through the threaded rig and a whole `OnlineRound`:
  `a_seats_well_is_drawn_on_the_press_and_only_once`.
- `server/tests/round.rs`: `a_well_anchored_through_the_mailbox_lands_on_the_clients_tick`:
  through whole `OnlineRound`s over `NativeTransport`, a seat presses
  twice: one `Fired`, one zone in the room within a tick's flight of the
  client's drawn anchor, the replica's own snap drawn once; and its owned
  hull, pulled by its own sandbox, never refused.
- The room server's `cargo test -p bongbong-server` as it stands.

## 11. Probe

- **Defaults first**: `just probe-fixtures` and `just probe-fields`
  unchanged, passing their recorded ceilings untouched.
- **With the crate**: the same two sweeps with `--crate gravity_well`. AFK:
  enemies that carry no special collect the crate and use it on the AFK
  seat - no clump against one seat, so the trouble arm where the seat stands
  by water, lava or drums, the guard arm on the Protect maps (the seat
  starts by its frog), and the shield arm when the seat's shells come their
  way (an AFK seat fires none: rarely).
- **Armed enemies**: the same sweeps with `--tuning armed.json`,
  `{"enemy_special_weapon_chance": 1.0, "enemy_special_weapon_well_share":
  1.0}` - every enemy that would carry a special spawns with wells. A
  second armed run with `--players 2` on the fixtures (the clump arm needs
  two seats; the probe's script for every seat past the first).
- The probe's tank line gains `well=` (wells left) and ` orb=` (an orb in
  flight); its fire tuple counts the wells, so a launch is a trigger pull
  for `FIRED_RECENTLY_FRAMES`.
- **Out of its hands**: a tank in a forming or pulling well's reach whose
  footing carries a pull (`TankSnapshot::pulled`) is in `OUT_OF_ITS_HANDS`
  beside `disabled`: no anomaly reads it while it is, and every window over
  its motion starts again where it stands when it leaves the pull
  (`TankTrack::rejoin`) - the drag is the well's doing, not the AI's. So a
  clump at a core is no `clustering` or `pile-up`, a hull pressed into a
  wall by the pull no `wall-grind`, and its crawl no `stall` or
  `low-progress`.
- **Deliberate holds**: a heavy enemy bracing (`TankSnapshot::bracing`)
  and one holding for its orb (`TankSnapshot::special_why == "orb"`) are in
  `HOLDS` - not a stall, a stale start, low progress or jitter. An escape
  across is driving where it asked.
- **Stranding is the bar**: windows restart when the pull ends, so a tank
  the collapse leaves wedged, or one that cannot get going after it, is
  counted by `stall`, `low-progress` and `never-arrived` as ever. The
  armed and crate runs are read for exactly that round by round: a tank
  within a former well's reach three seconds after its collapse with a
  stall or low-progress line is the well's bug, not a ceiling.
- **`offbox-fire`** reads `shot_at_seat` at the launch, as ever: a well
  used on a seat from outside its sight box is one. A seat the pull
  catches beyond the one it was used on (collateral) is not a shot at it.
  Budget 0.
- **What to watch** in the armed and crate runs: `spin` from the escape
  (a perpendicular is never a turn round, so a spin there is a bug),
  `jitter` at a pull's edge (the escape's latch), `pile-up`/`clustering`
  round a herd (the herd's slots stand 176 px out), `never-arrived` on the
  field maps (a wave's route round a well), `border-stuck` (a pull dragging
  a hull against the field's edge: out of its hands while pulled; after, a
  real strand).
- **The bar**: every crate and armed run within the defaults' ceilings,
  `offbox-fire` 0. An exceedance is read round by round from its `ANOMALY`
  lines; one the well's own action causes - a tank stranded after a
  collapse, a tank spinning or jittering on its escape, a herd piling up, a
  well used from off the box - is fixed, not re-baselined. Recorded here in
  Phase 2: the totals at the defaults, with the crate and armed, the long
  rounds against a shells pack (the hammer's yardstick for a pack that
  keeps an AFK seat alive), and what moved.

## 12. Interactions, decisions, what is left out

### Interactions with what ships

| With | What happens |
|---|---|
| Rainbow shield | Soaks the collapse's damage and the drums' blasts; the pull and the fling land whole; a shot a shield turns bends from there |
| Heat shield | Nothing |
| Speed boost, ooze coat | Change the hull's own speed and so its escape - a boosted hull drives out better, an oozed one worse; the current is not scaled by them |
| Portals | The pull does not reach through one; a hull dragged into a trigger goes through it and out of the pull; a shot bent into a portal goes through as ever and bends again past it if a well stands there; the orb flies over them |
| Water | A ford's pace and grip let the pull drag a wading hull harder; deep water is a wall it presses hulls against (a core over a lake pins them on its shore); fish drawn in and thrown onto the bank at the collapse; a grenade rolls in slower through water's drag |
| Ice (snow) | The tracks bite less (`ice_grip_factor`): a broadside hull that holds on dry ground slides; grenades roll in faster; no fish |
| Rain, storm | Wet grip: the side pull drags harder |
| Sandstorm gusts | The gust's flow and the well's current add |
| Night, storm, fog | The orb, the snap, the ring, the swirl and the rim are drawn unlit, the ring throws violet light and the core stays black; the AI counts a seat within its sight under the sky |
| Lamp posts, lanterns | Stand: a post is a tile, a lantern a fixture the pull leaves alone |
| Towers | Never moved; their bullets bend (a gun tower's burst curves round a well); the tesla arcs drones in a pull as ever; an offline tower is no different |
| Frogs | Either side's slides in pinned, hops out at the collapse; the opposing side's takes the collapse's damage; Hunt's enemy frog counts as an ally to the AI's rule |
| Crates | Slide in and out, keep their slot, never break by the pull; with `crate_breakable` a held drum's blast breaks them as any blast |
| Drums, oil, fires | Drums lifted and set off together; oil and fires stay, and the drums' blasts light a trail in reach |
| Trees, grass | Trees stand and stop the orb; grass leans in (cosmetic) and hides as ever |
| Glass, walls, sandbags, fences | Stand; the orb anchors on walls and glass and floats over sandbags and fences |
| Lava, the volcano | A lava ford's pace and grip, and its heat: a hull pulled across a ford burns; deep lava is a wall; a cone stops the orb; lava bombs untouched |
| Shells, bullets, plasma | Bent; swallowed at a core |
| The laser, the flamethrower | Untouched |
| Missiles | Dragged, steering back; swallowed at a core with no blast |
| Grenades | Roll in, circle the ring, thrown out in a ring |
| Wrecks | Untouched |
| Gates, waves | A tank rolling in is off the field; arriving into a pull, it reacts like any |
| Field maps | The sight box binds every use; a far coasting tank is dragged and does not react; a well's arrow reaches a seat wherever it is; a wave's route goes round a well |
| The couch and its split | Both seats pulled (the pull is blind) - the clump arm's case; each half draws the wells of its world; the prompt under each seat's own block |
| Training | `drop = ["gravity_well"]` works by its name; a dummy never fires one |
| The C2 commander | Never orders a pulled or anchoring tank |
| Online | §8 |

### Interactions with the sonic hammer (built here)

| Hammer | Well |
|---|---|
| The knock and its skid | The collapse's fling is the hammer's knock from the core (`knock_from`): one skid model, one validator allowance |
| A shove on a hull in a pull | Off its tracks, the whole current carries it: a shove toward the core slides it in faster, one outward fights the current |
| The wave and a well | Independent - sound is not pulled; the wave pushes a captured grenade off the ring, which is free again with the push |
| Held drums | Not tiles: the wave cannot throw them |
| The hammer's AI | A seat whose slide ends inside a pulling enemy well is in trouble (`lands_in_trouble`): a hammer enemy shoves a seat into a well |
| A hammer tell in a pull | Commits for its half second, the tank dragged meanwhile; then the `pull` tier |
| The online claim | One pending claim per kind; the anchor's by the orb's id |

### Interactions with the EMP (built here)

| EMP | Well |
|---|---|
| The ring reaches an anchored well's centre | It collapses at once - the whole collapse, `early` |
| The ring reaches an orb | It fizzles: no well |
| The ring reaches a well tank | Its special is down: no launch and no anchor press; an orb it has out anchors itself |
| A disabled enemy in a pull | Coasts, does not react, is dragged - the EMP-then-well combo |
| An EMP enemy's value of a seat carrying a well | `emp_ai_special_value` |
| The predictor's `offline_left` | A press during it is a shell, neither a launch nor an anchor |
| `WPN OFFLINE` | The well's slot reads it |

### Interactions with the gauss rail (built here)

| Rail | Well |
|---|---|
| A slug through a well | Not bent (an instant trace), not swallowed: it pierces whatever the well holds, a clump at the core included |
| A charging enemy in a pull | Drops its charge (`windup_rule`'s `Drop`) and escapes across |
| A charging seat in a pull | The charge holds; the crawl (a fifth of its speed) cannot fight the current, so it is dragged while it charges |
| The rail's recoil in a pull | A knock: off its tracks, carried by the whole current |
| Lanes and the AI | The rail's edge hold and lane dangers as ever; the well adds only its route surcharge |

### Interactions with the FPV swarm (built here)

| Swarm | Well |
|---|---|
| A drone in a pull | Dragged off course and steering back; a diving one dragged off its point; downed at the core (`AirStrike::Well`); turned out at the collapse |
| The halo | Not in the world: not pulled |
| The `air` tier in a pull | Yields to the `pull` tier, as every special's use does |
| The online claim | The anchor's claim by id is the swarm's payload claim |

### Interactions with the rod from god (built here)

| Rod | Well |
|---|---|
| A seat held at a core | Stands still by `SeatMotion`: a rod enemy's camper - an enemy's rod lands on the clump the well holds, the combo |
| A rod's impact in a well's reach | Its shove is a knock, carried by the whole current while it skids; its crush kills what the well holds at its core; held drums in its break radius go off (a blast over a held drum) |
| A call's circle and a pull | The well AI's trouble: a seat pulled into a live call; an enemy in both dodges the circle first (the `dodge` tier sits before `pull`) |
| A crater in a pull | Its pace (`rod_crater_pace`) weakens a hull's escape |
| Zones | One family, one arrow kind, one mark per kind, one provisional pairing by id |

### Decisions taken

1. **The pull goes through the drive: along a hull's tracks it is a
   current, across them a side pull the grip holds.** The drive brakes and
   drives the axis a hull faces with a velocity controller (an engine of
   1050 px/s² and a brake that closes over a quarter of the gap a tick for
   a standard chassis) and holds the other axis with a constant grip of
   500 px/s². A plain force put on every hull each tick (`apply_impulse`,
   the issue's "body force") is cancelled whole along the axis up to the
   engine, and drags hardest across it, so the escape would be to drive
   away - the opposite of BB-42's counter - and a hull driving away would
   not feel the pull at all below the engine. The water's current (one
   frame for both axes) is isotropic: away is always the shortest way out.
   A tracked hull rolls along its tracks and bites across them; a current
   along and a side pull across is that, and it makes "drive across, not
   away" true where it matters (§1's numbers, §10's test) while a heavy
   chassis braces broadside. Mass comes in as the hammer's exponent. It is
   still a body force through `physics.rs`: the drive's one impulse.
   Rejected: a plain force, a uniform current, a skid (the hull's choice is
   the whole counter). *For Oto.*
2. **The orb is a shot, not a moving zone.** As a shot it is interpolated,
   drawn on the press as a provisional and paired by `Fired` - machinery
   that exists; the zone begins at the anchor. The rod's doc imagined the
   orb as the zone's first stage; zones are neither interpolated nor
   predicted.
3. **It anchors on contact and at its range**: walls, glass, trees, towers,
   hulls of either side, frogs and the field's edge stop it; it floats over
   knee-high props; no roll. Fired at an enemy, it sticks to it - a slow
   sticky singularity, the counter to it being the slowness.
4. **The anchor press is never held back by the reload**, and a press just
   after the orb anchored itself fires nothing: a press meant for the orb.
5. **Shots bend; speed never changes**; instant traces (laser, rail,
   tesla), cones (the flame) and arcs that are a function of age (flying
   drums, lava bombs, globs) are left alone. *For Oto*: a laser bent by the
   well (lensing) would be beautiful and would make a beam a polyline the
   client must draw on the press.
6. **What reaches the core is swallowed** - shells, bullets, plasma, orbs,
   missiles, drones: that is the shield arm, and the core reads as a hole.
   Rejected: shots orbiting for the pull (a shot circling a core for four
   seconds is a mine nobody laid), shots passing through. *For Oto.*
7. **Wrecks are not pulled** - settled hulks, as no blast moves one;
   flinging wrecks at the collapse would be ram damage nobody aimed. *For
   Oto.*
8. **Tiles stay; drums are lifted.** A tile cannot move (BB-42's own
   words); a drum is the loose thing on the field, lifted with the
   hammer's death-without-a-blast and held as a function of the clock.
9. **Drums pulled together go off together**: all on the collapse's tick,
   where they circle the core - a ring of blasts on whatever the well
   gathered. Rejected: thrown out and going off where they land (the
   hammer's way - apart, not together). *For Oto*: a well by a drum cluster
   is the deadliest thing on the field.
10. **Grenades are captured into the ring, fuses burning, and thrown out in
    a ring** - the armory's combo, built.
11. **Crates drift and keep their slot**: the slots, the respawn and the
    wire's slot bitmask stay as they are; only an offset moves.
12. **The frog is pinned and slides**, hopping out at the collapse by a
    computed hop with no jitter.
13. **The collapse hurts the side opposing the owner a little** (10 at the
    core), so a well alone pays something; the fling is blind. *For Oto*:
    0 makes it pure control.
14. **An EMP collapses a well early with the whole collapse**, rather than
    switching it off: one collapse path, and an EMP becomes a detonator -
    the counter-play reads (pulse it, it blows). It fizzles an orb.
15. **The pull is blind**: both sides, the shooter's own hull; the AI is
    what keeps its allies out.
16. **Several wells sum**; a captured thing belongs to the lower id.
17. **The AI's four arms only** - clump, trouble, guard, shield - in the
    issue's words; no well on a lone seat in the open (a pull toward
    nothing).
18. **The reaction is across, and heavies brace** - decided by the drive's
    own numbers (`holds_broadside`) above a mass threshold, latched, never a
    turn round. A well is no danger (the dodge's way out is radial) and no
    edge hold: a route surcharge.
19. **The dodge comes before the pull**: a rod's circle kills, a well
    drags.
20. **`until` is the end of the stage**, so a client never needs the
    room's `well_pull_seconds`.
21. **The "at 11" is built**: marks swirl, grass leans, particles spiral -
    cosmetic steps run by the room and by a replica alike (state the wire
    does not carry), and the particle layer's presentation.
22. **The ripple pinched inward is an inward ring** through the existing
    ripple pass (two uniform arrays and a sign, both dialects), at the
    anchor; the collapse's ring runs out. Rejected: a lens over the whole
    pull (every hull drawn off its true position for four seconds - the
    aim would fight the picture) and blocks alone (they cannot bend the
    scene). *For Oto.*
23. **The `VOID` ramp is the Armory scene's violets**; the module stays on
    the palette (its light is `'white'`).
24. **Crate ink two-tone, a void and a lilac** (`#1C1033` / `#E6A8FF`): no
    free hue is left that is not green, so the pick is by distance (30 from
    the nearest ink) and by the one black symbol among the crates. *For
    Oto*, with a screenshot of the crate beside the other eighteen in
    Phase 2, after the rail's and the rod's final inks.
25. **The crate spills**: a projector and a coil, no explosive.
26. **The client pulls its own hull** in its sandbox on the room's tick,
    and the room's validator allows the drift - a pull sent as a `Shoved`
    every tick would arrive a round trip late, every tick.
27. **Bent provisional shots step on the tick grid** and incoming fire is
    carried through the field a tick at a time, so a curve is drawn where
    the room flies it at any frame rate.
28. **The anchor is claimed by the orb's id**, not by an input tick: an
    anchor spends nothing and logs no `Fired`.
29. **An enemy's orb is drawn where the room has it**, not carried into
    the present: it hits nothing to judge there, and the zone it becomes
    appears where it is drawn.
30. **No wind-up before a launch**: the slow orb and the forming ring are
    the tell (the swarm's rule).
31. **An enemy anchors no nearer than `well_ai_min_px`** (160 from its
    muzzle, so its own hull stands past the reach and the friend margin);
    its shield stands there.

### Not in this PR

- Placing the well in the shipped levels - level design, a follow-up.
- A lens over the whole pull (decision 22).
- Bending the laser, the rail or the flamethrower's jet (decision 5).
- Pulling wrecks (decision 7), lanterns, rubble, scorches, fires and oil.
- The AI pulsing an EMP to collapse a seat's well that holds it, using a
  well against drones, missiles or towers, or anchoring one on a lone seat.
- A drawn guide of a shot's curve ahead of it, and a swirl on the minimap.
- A well's pull through a portal.
- An off-screen arrow for an orb (§5: its launcher is on the screen of the
  seat it is used on).
- Sound effects - the game has no audio yet.
- A probe scenario in which the seat fires its special.

### Needs from the shared path

What this design takes from weapons 1-5's implementations beyond what
their docs give:

1. **`ZoneKind` open to a second kind with a stage**: `Zone::danger`
   answering `None` for a kind, `route_cells` per kind, `ZoneState::stage`,
   `add_provisional_zone`, `hide_zones` and `Predictor::pair_zone` keyed by
   id, `ArrowKind::Zone` and the minimap's mark by kind (the rod's).
2. **`Footing` open to a per-hull term** and `drive_tank_with` to split it
   by the axis after `control`; the skid branch's frame; `predict_seat`
   taking its footing through one function and the round tick its input
   lands on.
3. **`accept_seat_pose`'s drift open to a per-hull term.**
4. **`Projectile` open to `bend`**, `advance_projectiles` to the frame's
   field, and `resolve_projectiles` able to end a shot that is not a hit.
5. **`ShotKind`/`ProvisionalKind` open to a fourth kind** with its own state
   machine: `seat_shot`'s arm, `foreign_flying_shots` and the portal checks
   able to leave a kind out, `ProvisionalShot::advance` stepping on the tick
   grid with a field, `ShotStop` open to `Swallowed`.
6. **The incoming carry able to step a shot** rather than extrapolate it
   in one straight stretch.
7. **`Game::knock_from`** (the rod's) with a mass exponent per caller.
8. **The EMP's strike walk open to a block after the drones** (the EMP's
   doc keeps the place).
9. **`Drone::drag` and `AirStrike::Well`** (the swarm's §3.3 keeps the
   place), and a `Missile` open to a drag.
10. **`fish::throw_from`** (the rod's) and the shoal's scares open to an
    attractor.
11. **`SpecialUse` open to `Orb` and `Anchor`** - a press that touches
    neither the fire timer nor `shot_at_seat` -; `Ai::think` open to one
    more perception (`pull`); the tree open to a tier after `dodge`;
    `special_rule`'s and the `air` tier's yield (the rod's, for a call) open
    to a second cause.
12. **`EngageCtx::herd`** (the rod's) built round any zone's centre.
13. **`command::Busy` open** to `Pulled` and `Anchoring`.
14. **`Show::OwnShotsDrawn` and `presses_drawn` open to claims keyed by a
    payload id** (the rod's need 7).
15. **`Shockwave` and the ripple shader open to an inward ring** (`starts`,
    `signs`, both dialects).
16. **The probe's `OUT_OF_ITS_HANDS` and `HOLDS`** (the EMP's tables) open
    to `pulled` and `bracing`, with the clustering and pile-up checks
    reading them.
17. **`Corners::prompt`** (the rod's) open to a second weapon's prompt.
18. **`SPAWN_SWAPS` and `SEEK_SPECIALS`** as tables.
19. **The armory**: the reserved cell 7,14, and 9,14 and 32,13 left free by
    weapons 1-5.
20. **`CrateState` open to more fields**, and the interpolator to blending
    a listed crate's drift.
21. **The rod's impact's drum block** reading held drums.
22. **`Grenade` open to a captured state** that skips `roll`, and
    `roll_grenades` to a pull before the roll.

### Questions for Oto

1. **The pull's model** (decision 1): a current along the tracks and a side
   pull across them make "drive across" the way out deep in a well, away
   still fine near its edge, and a heavy chassis brace broadside. Is that
   the feel you meant?
2. **The core swallows** shots, missiles and drones (decision 6), which is
   what makes the AI's shield work. Keep, or let shots orbit and pass?
3. **The collapse's damage** (decision 13): 10 to the other side at the
   core, or none (pure control)?
4. **Drums go off together on the ring** (decision 9): a well over a drum
   cluster is lethal to everything it gathered, the shooter included if it
   stood in the pull. Too strong?
5. **Wrecks stay put** (decision 7), and so do lanterns. Fine?
6. **The ink** (decision 24): a black void ringed in pale lilac - the one
   black crate symbol - to be settled on a screenshot after the rail's and
   the rod's picks.
7. **No lens over the whole pull** (decision 22): the scene bends only as
   the forming ring runs in and as the collapse's ring runs out.
8. **The orb sticks to the first hull it meets** (decision 3), so it can be
   fired at an enemy as well as anchored in the open. Keep?
