# FPV swarm

BB-40, the fourth of the six weapons of BB-36. A special weapon from its own
crate (`pickup = "fpv_swarm"`), one at a time like every other
(`Tank::take_weapon`): a crate loads `fpv_drones_per_pickup` (6) drones, a
re-pick refills to six, another weapon's crate replaces it. Seats and
enemies alike: an enemy takes the crate while it carries no special
(`Tank::wants_pickup`) and uses it by its own rule (§4).

Six quadcopters hover in a halo over the tank. Each press sends one up, out
and over the walls at the nearest enemy inside the seat's sight box - or,
with none there, at the aim point ahead - and it dives on it and bursts:
about 8 damage, a small blast. They are the first **air targets** in the
game: a minigun's bullets, a tesla coil's arc and a gun tower's bursts bring
them down; shells and plasma pass under them. A tank under a tree is hidden
from the dive. In co-op each seat's drones blink in its team colour.
Counter: shoot them; stand under trees. The "at 11" is built: enemies carry
drones too, launching from behind cover one at a time, and the gun tower
becomes anti-air.

The sonic hammer's doc (docs/sonic-hammer.md §3), the EMP's
(docs/emp-burst.md §3.3) and the gauss rail's (docs/gauss-rail.md §3) lay
down the shared path this plugs into: the AI hook, the tell and its arrow,
the online press show claimed by input tick, the trigger kinds, the
carried/fired split, the disabled state, the dangers, the probe's
`--crate`, the spawn swap, `spawn_pickup` and the armory. This PR adds
**air targets** (§3.3) - the general piece that the hit test, the towers,
the EMP's ring, the hammer's wave, the rail's slug and the gravity well
(BB-42) read for anything in the air - and an **eased launch** for a press
whose show outlives its frame (§8).

## 1. How it plays

### The halo

- **The halo is the stock.** `Tank::fpv_drones` is the count, and the halo
  is its picture: `fpv_drones` of `full_load` slots on a ring round the
  hull. The drones in it are not in the world - no entity, no body, no hit
  box, no wire entry beyond the count `TankState::ammo` already carries -
  so nothing strikes a hovering drone and nothing collides with one
  (§12, decision 1).
- **Fixed slots.** Slot `k` of `n = full_load` stands at the bearing
  `FPV_HALO_START_DEG + k * 360 / n` (30, 90, 150, ... at six; 0 = up,
  clockwise), `FPV_HALO_RADIUS_FRACTION` (0.55) of the tank's sprite size
  from its centre (44 px for a standard chassis, outside the hull) and
  `FPV_HALO_HEIGHT_PX` (14) above the ground: `fpv::halo_slot(centre,
  sprite_size, k, n)`, a pure function of the hull's position. The ring
  is fixed in the world: it neither orbits nor turns with the hull. The
  drones in slots `0..fpv_drones` hover there; each bobs a block up and
  down and its rotors turn (drawn only, §5).
- **The halo empties from the top slot down**: a launch takes slot
  `fpv_drones - 1`, so the gap that opens is the drone that left.
- **A wreck takes its halo with it.** The frame a tank carrying drones is
  wrecked, the drones left in its halo fall (drawn, from the wreck's
  carried count and its own clock `Tank::wreck_timer`: each tumbles from
  its slot to the ground over `FPV_HALO_FALL_SECONDS` (0.5) and lies dark
  after; §5). Nothing is launched and nothing bursts.
- **Disabled by an EMP**: the special is down (`Tank::special_down`,
  docs/emp-burst.md §3.3), so nothing launches, and the halo settles - each
  drone drops to the ground under its slot, rotors stopped, lamp dark - and
  lifts again the frame the disable runs out. The stock is kept, as every
  special's is through an offline.
- **A teleport** moves the halo with the hull (it is a function of the
  hull's position). **A weapon swap** - another special's crate - replaces
  the stock, and the halo is gone. **A tank rolling in through a gate**
  carrying drones (an enemy that spawned with them) shows its halo as it
  rolls; it launches nothing until it has an `Ai`, on arrival. A seat that
  comes back through a gate is a fresh tank with no special.

### The launch

- **One per press.** The trigger is `Trigger::Press` (docs/gauss-rail.md
  §3.3): it fires on the press edge, with a drone in the halo, the special
  up and `Tank::fire_cooldown` out; `fpv_reload_seconds` (0.4) before the
  next. A seat empties its halo in a little over two seconds. A seat's
  press goes off at once; an enemy's has no wind-up either (§4, "The
  tell").
- **What a launch does**: one drone off the count (`fpv_drones - 1`), the
  target picked now (below), and a `Drone` put in the world at its slot's
  ground point at `FPV_HALO_HEIGHT_PX`, in its first stage. The module's
  launch cell flashes (`Tank::kick_fpv`, `fpv_flash_seconds`). No recoil:
  a quadcopter lifts itself. `Event::Fired { weapon: "fpv_swarm" }` then
  `Event::DroneLaunched { id, slot, x, y, target, frog }` - in that order,
  in the same tick, which the online claim reads (§8).
- **Ids** come from `Game::take_shot_id`, the one per-round projectile
  counter shells, bullets, missiles and grenades share, so a drone's id is
  its wire key and its walk order.

### Which target

The target - the drone's **lock** - is picked at the launch and kept; a
drone never seeks a new one (§12, decision 3).

- **A seat's drone** locks the nearest opposing tank inside the seat's
  sight box: a live enemy on the field (with an `Ai`, not rolling in, not a
  wreck) whose centre lies in the box round the seat
  (`ai::in_sight_box_of`) and that is not under canopy (§1, "Trees").
  Nearest by the distance between the seat's centre and the enemy's
  (`Tank::position`), ties to the lower owner slot. Grass does not hide a
  tank from the air: only trees do.
- **An enemy's drone** locks what its AI asked for (`AirWant`, §4): a seat
  - the one its rule chose, checked again here: live, on the field, the
  launcher standing inside that seat's sight box, not under canopy - or
  the players' frog (a hunter's quarry, alive and not under canopy), or no
  lock and a point (a tree's crown, §4). A choice that fails the check
  launches with no lock, at the aim point.
- **No lock: the aim point.** `fpv_aim_px` (192, six cells) ahead of the
  launcher's centre along its hull's facing, held a half cell inside the
  field; for an enemy's point, that point. The drone flies there and
  dives on it.
- **What a lock follows**: the locked tank's centre, or the frog's. The
  lock is **lost** - the aim stays where the target last stood and the
  drone dives there - the tick the target is a wreck or gone, goes under
  canopy, or teleports (`Event::Teleported` for its slot this frame): a
  portal is a way to shake a drone. `Event::DroneLockLost { id, why }`
  (not sent; the trace and the tests read it).

### The flight

A drone works on its **ground point** and a separate **height**, the
missile's and the flying drum's convention (`missile.rs`): the simulation
moves the ground point, the drawing lifts the drone by its height over its
shadow. It flies over everything - walls, props, trees, towers, tanks,
water, lava - and touches nothing on the way (§12, decision 4). Its ground
point is held inside the field. `Drone::advance(dt, wind)` is pure: no RNG,
no world.

1. **Launch** (`fpv_launch_seconds`, 0.35): up out of its slot along the
   slot's outward bearing at `fpv_launch_speed` (70 px/s), the height
   easing out from `FPV_HALO_HEIGHT_PX` to `fpv_cruise_height` (36). The
   climb is the same whatever the lock - `fpv::launch_path(slot, out, age)`,
   a function of where it left and its age alone - which is what lets a
   client draw it on the press (§8).
2. **Cruise**: at `fpv_cruise_height`, accelerating at `fpv_accel` (700) to
   `fpv_speed` (230 px/s), turning toward its aim at `fpv_turn_rate_deg`
   (300 degrees a second) from the outward bearing it climbed on. Faster
   than any tank on its own engine (a seat's 210, an enemy's 160 give or
   take a quarter), so a running tank is caught; a turning circle of about
   44 px, inside `fpv_commit_px`, so it never orbits its target. The aim is
   the lock's centre, refreshed every tick (`Game::guide_drones`).
3. **Dive**: the tick its ground point is within `fpv_commit_px` (64) of
   the aim, it **commits**: the aim freezes where the target stands then,
   and the drone comes down on it in `dive_seconds = distance /
   fpv_dive_speed` (320 px/s; 0.2 s from the commit distance), its height
   falling linearly to 0 over that time. A tank that moves in those fifth
   of a second moves off the point; a drone does not follow it down. A
   drone still in its climb when the target is within reach commits as it
   reaches the top.
4. **Battery out**: `fpv_max_flight_seconds` (6) after its launch, a drone
   still cruising commits a dive on the point `fpv_commit_px` ahead along
   its heading, the missile's rule - so a drone whose target keeps slipping
   it still ends.
5. **The burst** at the dive's end (below).

A **downed** drone is a fifth stage, **Falling** (below, "Shot down").

### The burst: only the dive hits

When a dive's time is up the drone bursts where its ground point is
(`Game::resolve_drones`), and nothing else a drone does touches anything:

- **In a crown.** A burst point inside a standing tree's crown - the tree's
  cell grown by `fpv_canopy_px` (12) on every side - bursts in the leaves:
  the tree takes `fpv_tree_damage` (4) through `damage_obstacle`
  (`DamageCause::Blast` with no falloff, from the drone's heading), which
  leaves a flammable tree burning when it kills it, as every killing blow
  does. Nothing under it is touched. A broadleaf (12) takes three drones, a
  pine (9) three. Two crowns: the nearer tree's centre, then cell order.
- **On the ground.** Anywhere else, a small side blast
  (`Game::side_blast_sparing`, §3.2): the missiles' and grenades' rule at
  `BlastParams::drone(owner)` - radius `fpv_blast_radius_px` (44), damage
  `fpv_damage` (8) at the centre falling linearly to nothing at the radius,
  **no roll** (`BlastParams::roll_damage` draws nothing when its range is a
  point), knockback `fpv_knockback_speed` (30). So: the side opposing the
  launcher takes the damage through `Tank::take_damage` (a rainbow shield
  soaks it; the kill is credited to the launcher, `BlastParams::by`),
  every live tank in reach is shoved a little (a seat's shove on
  `Frame::shoves` for an owned hull), the opposing side's frog is hurt
  (8 at the centre; a frog has 40), tiles crack and drums react as to any
  blast (`damage_obstacle` with falloff - a fence's one-shot odds and a
  drum's fuse are their own rules), breakable crates break
  (`blast_crates`). **Hulls and frogs under canopy are spared** - left out
  of the blast whole (§1, "Trees"). A dive on a tank that stood still lands
  on its centre: 8. One that moved 30 px off the point: about 2.5.
- `Event::DroneBurst { id, slot, x, y, crown }` and the ordinary events of
  what the blast did (`Wreck`, `ObstacleDestroyed`, `Blast`, `ShieldBroken`,
  `Shoved`); the show (§5): a small fireball leaning down the dive, a weak
  ripple (`fpv_shock`, 0.15 of a tank dying), the impact flash, a small
  scorch on dry ground (none in a crown, none on water), the grass round it
  flattened.

### Trees

"Trees hide a tank from the dive", made exact with one box: a tree's
**crown** is its 32 px cell grown by `fpv_canopy_px` (12) on each side - the
48 px crown it is drawn with, and a few pixels more. A tank whose hull box
(`Tank::hull_bbox_world`) overlaps a standing tree's crown is **under
canopy**, and so is a frog whose centre is inside one
(`Game::canopy_over(box) -> Option<Entity>`, the tree, nearest first).
Then:

1. it is never locked (§1, "Which target");
2. a drone locked on it loses the lock the tick it goes under, and dives
   on the spot it last saw it;
3. a dive that ends in a crown bursts in the leaves and hurts only the
   tree (above);
4. a burst on the ground leaves it out whole.

A tank cannot drive into a tree's cell, so being under canopy means parking
against a tree - within 12 px of its cell, or among a grove. Trees are
destructible (a drone's burst takes 4 off one, a shell's damage more), so
the cover lasts until somebody shreds it; the enemies' rule does that on
purpose (§4, "canopy"). A burning tree still shelters until it dies. Tall
grass hides nothing from the air.

### Shot down: air targets

A drone in its launch, cruise or dive is an **air target** (§3.3). These
strike it there:

- **Bullets** - a minigun's, either side's tank's, and a gun tower's: a
  bullet whose sweep enters an **opposing** drone's strike box (the column
  from its shadow up to its body, §3.3) stops there, and the drone counts a
  hit; at `fpv_drone_hits` (1) it is downed. A bullet passes its own side's
  drones. No damage number and no roll: a drone has hits, not health.
- **A tesla coil's arc**: an opposing drone within `tesla_range` of the
  coil (to its ground point) is struck by an arc of its own, with no charge,
  every `tesla_air_gap_seconds` (0.5) - beside the coil's charge on a tank,
  which it neither spends nor interrupts (§4, "Air defence by the towers").
- **A gun tower's bursts**: it turns on an opposing drone in reach before
  any tank, leads it fully (`gun_tower_air_lead`, 1.0) and fires its bursts
  of minigun bullets at it, which strike it as above.
- **The EMP's ring** (docs/emp-burst.md): every drone whose ground point
  the front reaches falls dead, any side - the ring is blind.
- **The sonic hammer's wave** (docs/sonic-hammer.md): every drone of the
  side opposing the shooter whose ground point is in the cone
  (`SonicCone::reaches` - walls shadow it as they shadow everything) is
  knocked down when the front reaches it; its own side's ride it out (the
  hammer's "only the other side is hurt").
- **The gauss rail's slug** (docs/gauss-rail.md): every drone whose strike
  box its lane crosses before the stopper is pierced and downed, any side,
  as it goes through every tank in the lane; the slug keeps its damage past
  a drone (`gauss_pierce_keep` is not applied - a drone is not armour).

Nothing else touches a drone in the air: shells, plasma bolts, laser beams,
the flamethrower's jet, missiles, grenades, globs, lava bombs, flying drums,
blasts and fires below it, other drones (§12, decisions 7 and 8).

**Air defence keeps the sight box.** A tower or an enemy tank engages a
seat's drone - turns to it, fires at it, arcs it - only from inside that
seat's sight box, the box it may engage the seat itself from
(`towers::box_allows`, the AI's flak, §4). A bullet already in flight hits
whatever drone it meets, as it hits whatever tank. An enemy's drone may be
engaged from anywhere.

**Downed.** `Game::strike_air` puts the drone in **Falling**:
`Event::DroneDowned { id, x, y, height, by }` (`by` the `AirStrike`:
`Bullet`, `Tesla`, `Emp`, `Sonic`, `Rail`). It keeps its ground velocity,
losing it at `fpv_fall_drag` (3) a second; its height falls from rest under
`fpv_fall_gravity` (500 px/s²); it spins (§5). Where it reaches the ground
it is a **dud**: `Event::DroneCrashed { id, x, y }` (a puff of dust and
pale sparks, the missile dud's), removed, and nothing else - no blast,
no damage (§12, decision 8). A falling drone is no longer an air target.

### Weather, portals, the edge, the end

- **A sandstorm's gust** carries a cruising or diving drone: its ground
  point drifts by `weather::gust_at` times `fpv_gust_factor` (0.5) - a pure
  function of the round's clock - so a dive in a gust lands downwind of
  its aim. The climb is left alone, so the press's drawing stays exact
  (§8). Rain, snow, fog and night change nothing it does; at night its lamp
  throws a little light (§5).
- **Portals**: a drone flies over them; a locked target that teleports is
  lost (above).
- **Water and lava**: flown over; a burst on water splashes rather than
  scorches.
- **The field's edge**: the ground point is held inside the field and every
  aim a half cell inside it.
- **The launcher wrecked mid-flight**: its drones fly on and burst; their
  kills are its.
- **On the end screen**: drones in the air fly out their flight and burst,
  hurting nobody (`resolve_drones(f, false)`, the missiles' rule); towers
  do not run; a bullet passes through a drone (air targets are struck only
  while the round is live). The halo stays drawn.

### A wreck, an EMP, a swap, a teleport

| What happens | To the halo | To drones in the air |
|---|---|---|
| Its tank is wrecked | Falls with it (drawn) | Fly on, burst, credited to it |
| The EMP's ring reaches its tank | Settles on the ground, stock kept, nothing launches | Those whose ground point the front reaches fall dead |
| Its tank teleports | Moves with it | Unaffected |
| Its target teleports | - | Lock lost: dives where it went in |
| Another special's crate | Gone | Unaffected |
| An FPV crate | Refilled to six | Unaffected |
| The round ends | Drawn as ever | Fly out and burst harmlessly |

## 2. Where it lives

| File | What |
|---|---|
| `src/air.rs` (new) | Air targets (§3.3): `AirTarget` (`strike_box`, `drawn`), `AirKey`, `AirStrike` (the wire's order), `strike_box`; the rules every reader keeps, in its doc |
| `src/fpv.rs` (new) | The weapon's headless half. `Drone` (id, owner, stage, ground point, height, heading, speed, ages, launch slot, lock, aim, the dive's plan, hits, the fall; `launch`, `advance(dt, wind, field)`, `commit`, `down(by)`, `in_air`, `tracking`, `velocity`, `as_target`, `drawn`, `heading_degrees`/`heading_of`, `age_by`), `DroneStage`, `DroneLock` (`code`, `LOCK_NONE`/`LOCK_FROG`/`LOCK_ENEMY_FROG`), `AirWant`, `halo_slot`, `launch_path`, `FPV_LAUNCH_HANDOVER_SECONDS`, `pick_nearest`, `crown_box`, `boxes_overlap`/`box_holds`, `lamp_color`/`lamp_on`, `module_cell`, the composers `compose_drone`, `compose_shadow`, `compose_halo`, `look_of`, `drone_block` |
| `src/simulation/fpv.rs` (new) | The world half. `fire_fpv` (the dispatch arm's body), `launch_drones`, `pick_lock`, `guide_drones` (locks and aims, before the step), `advance_drones` (in the fixed step), `resolve_drones(f, live)` (bursts and crashes), `drone_show` (the cosmetic half, which a replica's `DroneBurst` calls too), `spared_by_canopy`, `count_drones_out`, `Game::{air_targets, strike_air, drones, standing_trees, canopy_over}`, `fpv_senses` (what the AI is handed, §4, with the cover search), `air_threats` (§4, with the tree search), `seat_drone_slot` |
| `src/simulation/hits.rs` | `Terrain::sweep_air` (a bullet's sweep against the air targets, lag-compensated) and `air_along` (the rail's lane); `HitBoxFrame::air` |
| `src/simulation/weapons.rs` | The `ActiveWeapon::FpvSwarm` dispatch arm; `Projectile::strikes_air` (bullets) |
| `src/simulation/missiles.rs` | `side_blast_sparing` (`side_blast` with a list of hulls and frogs left out); `BlastParams::drone` lives in `simulation/fpv.rs` |
| `src/simulation/mod.rs` | `Frame::pending_drones`; `launch_drones` at the end of `spawn_pending`; `guide_drones` beside `guide_missiles`; `advance_drones` in `step_world`'s loop; `resolve_drones` after `resolve_missiles` (both branches); `resolve_projectiles`' air sweep; `enemy_phase` (the senses, the threats, `Ai::air_want` handed to `Tank::fpv_want` before `enemy_trigger`); `tick_presentation` (a replica's drones age); `Event::{DroneLaunched, DroneBurst, DroneDowned, DroneCrashed, DroneLockLost}`; `TankSnapshot::{fpv_drones, fpv_out, air_hold}` |
| `src/simulation/towers.rs` | The tesla's air arc (`tesla_air`, `Tower::air_cooldown`), the gun tower's air pick and lead (`pick_air`, `Tower::air_target`), `air_candidates`/`air_box_allows` |
| `src/simulation/emp.rs`, `sonic.rs`, `gauss.rs` | The drones' arm in `tick_emp_pulses` (after the missiles), in the sonic wave's cone, and in the slug's lane (`Pierced::Drone`); `SPAWN_SWAPS` gains the swarm (`sonic.rs`) |
| `src/simulation/present.rs` | `PresentWorld::air_contact` (a client's own bullets stop at drawn drones), `flash_seat_fpv`, `remove_drones`, `add_drone`, `set_seat_lifting` |
| `src/simulation/replica.rs` | `DrawableDrone`, `DrawableState::drones` |
| `src/simulation/debug.rs`, `src/devserver.rs` | `set_tank`'s `fpv_drones`; the snapshot's `fpv`/`fpv_out` and `drones`; `spawn_pickup {kind: "fpv_swarm"}` |
| `src/tank.rs` | `fpv_drones`, `fpv_out`, `fpv_want`, `fpv_flash`, `fpv_lifting`; `ActiveWeapon::FpvSwarm` (`name`, `full_load`, no tell, `Trigger::Press`), `SPECIAL_WEAPONS`; `weapon_ammo`/`take_weapon`/`empty_stock`/`wants_pickup`; `kick_fpv`; the module's cells in `module_cols` |
| `src/tower.rs` | `Tower::{air_cooldown, air_target}` |
| `src/pickup.rs` | `PickupKind::FpvSwarm` (`fpv_swarm`, row 16, its ink, cooks off) |
| `src/ai.rs` | `SpecialSense::Fpv(FpvSense)`, `fpv_rule`, `SpecialUse::Launch` (and `Approach`'s `why`), `generic_fire(FpvSwarm)`; `AirThreat`, the `air` tier and `act_air`; `Ai::{air_want, cover_spot, place_waited, air_threat, air_hold}`; `SEEK_SPECIALS` gains the swarm; `AiSnapshot::air` |
| `src/indicators.rs` | `ArrowKind::Drone`, `Scene::drones`, the arrow's X (`DRONE_X`) |
| `src/hud.rs` | `HUD_FPV_COLOR`, the `weapon_color`/`weapon_pickup` arms; the slot's count less `Tank::fpv_lifting` |
| `src/game.rs`, `src/render/game.rs` | `halo_of` and the halo with its tank in the standing walk; `paint_drone_shadows` after the floor, `paint_drones` over everything standing, `drone_lamps` in the glowing pass with their light; the dev overlay's strike boxes and aims; `draw_tank_stats`'s `FPV` line |
| `src/fx.rs` | Rotor wash, buzz and a falling drone's smoke; a launch's ring of dust, a burst's sparks (leaves in a crown), a downed drone's sparks, a crash's dud |
| `src/weather.rs` | A drone's lamp light |
| `src/fish.rs` | `DroneBurst` and `DroneCrashed` among the scares |
| `src/net/wire.rs`, `src/net/mod.rs` | `WeaponKind::FpvSwarm` (`drawn_on_press`), `DroneState`, `Snapshot::drones`; `PROTOCOL_VERSION` 18 and the measured sizes |
| `src/net/delta.rs` | `drones`, `drones_moved`, `drones_gone` |
| `src/net/events.rs` | `WireEvent::{DroneLaunched, DroneBurst, DroneDowned, DroneCrashed}` (`NO_TARGET`), the `press_show` arm, `drone_lock_lost` on `NOT_SENT` |
| `src/net/encode.rs`, `src/net/apply.rs` | `drones(game)`, `apply_drones`; the burst's show; the `kick_turret` arm; `Show::presses_drawn` read by the round |
| `src/net/interp.rs` | Drones blended and carried on like missiles |
| `src/net/predict.rs`, `src/net/round.rs` | `PressShow::Drone`/`DronePress`, `Predictor::refusal_after`; `OwnDrone`, `fly_own_drones` (the eased launch and its handover, §8), the halo's `fpv_lifting`; a drawn bullet's air contact |
| `src/bin/probe.rs` | The tank line's `fpv=`/`out=`, the fire tuple, the drone locks on and off the box, the `air` hold, a launch at a seat as arrival |
| `src/editor/mod.rs` | `Tool::Pickup(PickupKind::FpvSwarm)` (`fpv_swarm`) |
| `maps/armory.toml` | Its crates, a grove, two pines and a player gun tower (§3.4) |
| `src/tuning.rs` | The `fpv` group (§6), two rows in `towers`, one in `enemies` |
| `lang/en.ftl`, `lang/sl.ftl` | §7 |
| `tools/punypalette.py`, `tools/spritegen/gen_crates.py`, `tools/spritegen/tankdesign/{kit,export,render,lines/vanguard}.py` | The art (§5); writes `static/crates_sheet.png`, `pickup_glyphs.png`, `tank_modules.png`, `tank_modules_glow.png` |
| `src/thumbnail.rs`, `src/maplint.rs` | The armory's pin re-baselined; the armory as it lints |
| `docs/` | This, `CRATES_SPEC.md`, `SPRITESHEET_SPEC.md`, `effects.md` (who draws what), `TOWERS_SPEC.md` (anti-air); `CLAUDE.md` |

## 3. The shared path

### 3.1 What this uses as weapons 1-3 laid it down

Each item of the checklist (docs/sonic-hammer.md §3.0) gets its swarm arm:

1. `PickupKind::FpvSwarm` (`#[serde(rename = "fpv_swarm")]`, appended to
   `ALL`, row 16, `ink`, `cooks_off` true - six charges, like the
   missiles' and grenades' crates), `PickupKind::weapon`
   (`Some(ActiveWeapon::FpvSwarm)`), `name`.
2. `ActiveWeapon::FpvSwarm` (`name` "fpv_swarm", `full_load` =
   `fpv_drones_per_pickup`, `tell_seconds` none - the flight is its tell,
   §4 -, `trigger` `Trigger::Press`), appended to `SPECIAL_WEAPONS`;
   `Tank::fpv_drones` with its arms in `weapon_ammo`, `take_weapon`,
   `empty_stock`, `module_cols`. `wants_pickup` reads `special()`: an enemy
   takes the crate only while it carries no special.
3. The dispatch arm in `weapons::dispatch_fire` (`fire_fpv`); the
   trigger in `drive_player` by `ActiveWeapon::trigger` (`Press`).
4. `pickup_phase` needs nothing.
5. `ai::SEEK_SPECIALS = [SonicHammer, Emp, GaussRail, FpvSwarm]`;
   `SpecialSense::Fpv`, the `special_rule` arm, `generic_fire(FpvSwarm) ==
   false` (§4).
6. `hud::weapon_color` (`HUD_FPV_COLOR`), `hud::weapon_pickup`.
7. `gen_crates.py` (`KINDS`, `GLYPHS`), `PICKUP_INK['fpv_swarm']`; the
   tankdesign module `fpv` (§5). No anchor: the drones leave the halo, not
   the module.
8. `editor::TOOLS` and `Tool::name`; `tool-fpv_swarm`,
   `tool-short-fpv_swarm` in both catalogues.
9. `WeaponKind::FpvSwarm` (appended to `ALL`, both `From`s,
   `drawn_on_press` true), `Predictor::seed_gate`'s arm
   (`fpv_reload_seconds`), `apply::write_tank`'s ammo arm, `kick_turret`
   (`Tank::kick_fpv`) and `drawn_muzzle` (`None`).
10. `debug::{TankDebug, TankPatch}`, `set_tank`'s schema, `TankSnapshot`,
    the probe's tank line and fire tuple, `render::game::draw_tank_stats`.
11. The armory's crates (§3.4); `SPAWN_SWAPS` gains
    `(ActiveWeapon::FpvSwarm, |t| t.enemy_special_weapon_fpv_share)` after
    the rail's; the tuning group; `PROTOCOL_VERSION`.

And, as they are: the special hook's tier and `act_special` (extended
below), `Tank::special`/`active_weapon`/`special_down`, the dangers (a
cover spot and a back-off point are kept out of every danger the tank
does not own, `Danger::depth`), the press show's claim by input tick
(`Show::presses_drawn`), `spawn_pickup {kind: "fpv_swarm"}`, the probe's
`--crate fpv_swarm`, the EMP's dud (a crash is the missile dud's dust and
pale sparks).

### 3.2 What this extends

- **`SpecialUse::Launch { want, at_seat, why }`** - the hook's use for a
  weapon that sends something after a target of the rule's choosing.
  `act_special` applies it as `Fire` (face as it faces, the trigger pulled
  while `Ai::fire_timer` allows, `at_seat` into `Ai::shot_at_seat`, `why`
  into `Ai::special_why`, the timer set to `fpv_enemy_gap_seconds`) and
  records `want` in `Ai::air_want`. With the timer running it holds.

  ```rust
  /// What an enemy's drone is to lock (`SpecialUse::Launch`): the seat its
  /// rule chose, the players' frog, or a point - a tree's crown - with no
  /// lock. A seat's press asks for `Nearest`.
  pub enum AirWant { Nearest, Seat(u8), Frog, Point(Position) }
  ```

  In `enemy_phase`'s collect pass, where the trigger meets the simulation
  (docs/gauss-rail.md, "Needs" 8), the tank's `fpv_want` is set from
  `Ai::air_want` right before `enemy_trigger`; the launch takes it (and
  clears it), `None` reading as `Nearest`.
- **`SpecialUse::Approach { to, why }`** gains its `why` ("approach" for
  the EMP's and the hammer's closers, "to cover" and "back off" for the
  swarm's), which `Ai::special_why` reports.
- **`Ai::air_threat`**: `enemy_phase` puts each enemy's `AirThreat` on its
  `Ai` before it thinks (§4); the `air` tier reads it.
- **`ArrowKind::Drone { diving }`**, put with the never-dropped kinds
  (`Teammate`, `Frog`, `Volcano`, `Windup`): never merged, never left out
  past `indicator_max_arrows` (§5).
- **`PressShow::Drone(DronePress)`** and the round's own drawn launches
  (`OwnDrone`), which live past the frame they were pressed in (§8); the
  round reads the replica's claim (`Show::presses_drawn`), which gains the
  `DroneLaunched` arm through `WireEvent::press_show`.
- **The strike walks of weapons 1-3** gain an air arm each, after
  everything they already strike: the sonic wave (drones of the opposing
  side in its cone), `tick_emp_pulses` (every drone in the ring, after the
  missiles - the EMP's doc keeps the place), the rail's slug
  (`Terrain::air_along` up to where it stops, `Pierced::Drone`, every
  drone in the lane).
- **`side_blast_sparing(f, center, owner, params, spared)`** - the
  missiles' and grenades' `side_blast` with a list of hulls and frogs left
  out; `side_blast` is it with none.
- **`Projectile::strikes_air()`** - true for bullets, false for shells and
  plasma; `resolve_projectiles` sweeps the air (`Terrain::sweep_air`) only
  for a kind that does.

### 3.3 What this adds: air targets

Until now everything a shot could hit stood on the ground, and everything in
the air (missiles, flying drums, lava bombs, globs, grenades aloft) was
struck by nothing. **An air target** is something in the air that can be
struck there. The drones are the first; the gravity well (BB-42) pulls them,
and a later weapon that puts something in the air makes it one by adding a
key. In `src/air.rs`:

```rust
/// Something in the air that can be struck there: what the bullets' hit
/// test, the towers, the EMP's ring, the hammer's wave, the rail's slug and
/// the gravity well read (`Game::air_targets`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AirTarget {
    /// Which one - the walk order and, for a drone, its wire key.
    pub key: AirKey,
    /// Whose it is: who may strike it (`Owner::same_side`).
    pub owner: Owner,
    /// The point on the ground under it, and its height over that point.
    pub ground: Position,
    pub height: f32,
    /// Its ground velocity (px/s): what a gun tower leads it by.
    pub velocity: Vec2,
    /// Half its body's width (px).
    pub half: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum AirKey {
    Drone(u32),
}

/// What struck an air target (`Event::DroneDowned::by`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AirStrike { Bullet, Tesla, Emp, Sonic, Rail }

impl AirTarget {
    /// The box a shot strikes it in: a column from its shadow up to its
    /// body, `half` either side and `half` past each end - centre
    /// `(ground.x, ground.y - height / 2)`, half-extents
    /// `(half, height / 2 + half)`. A shot crossing the drone, its shadow
    /// or the line between them strikes it, which is what the picture
    /// shows: a drone is drawn lifted by its height over its shadow.
    pub fn strike_box(&self) -> (Position, Position);

    /// Where it is drawn: the ground point lifted by the height.
    pub fn drawn(&self) -> Position;
}
```

- **The list**: `Game::air_targets() -> Vec<AirTarget>`, every drone in its
  launch, cruise or dive (a falling drone is not one), sorted by key. Empty
  in a round with no drone in the air, so every reader costs nothing there.
- **The one setter**: `Game::strike_air(f, key, by) -> bool` - counts a
  bullet's hit or downs the target outright (every other cause), logs
  `Event::DroneDowned` at the drone's ground point and height, and answers
  whether it went down. Nothing else changes an air target.
- **The hit test**: for a kind that `strikes_air`, `resolve_projectiles`
  sweeps the frame's movement against every air target not of the
  shooter's side (for a tower's shot, not of the tower's) with
  `Terrain::sweep_air` - its `strike_box` grown by the shot's half extent
  and by `player_shot_hit_pad_px` for a seat's shot, the pad every enemy
  box takes - beside the ground sweep, which keeps its own ranks
  untouched. The air target is struck when it is met first, or at the
  same point as anything but a tank (a tank met at the same point keeps
  the shot). The list is read from the world before the loop, as the
  tanks are, and a drone downed by one bullet leaves it, so a bullet meets
  a drone where this tick's step left it and the next bullet flies on.
- **Lag compensation**: `HitBoxHistory::record` keeps every air target
  beside the enemies' boxes (`HitBoxFrame::air`, sorted by key), and a
  seat's shot rewound to its view tick meets the enemy drones where its
  client drew them (`sweep_air`'s `past`); one with no entry is met where
  it is.
- **What strikes them, and the rules every reader keeps** (the module's
  doc):
  1. only a shot that `strikes_air` hits one - bullets; shells, plasma and
     beams pass under;
  2. a side never strikes its own side's air targets - but the EMP's ring,
     which is blind (docs/emp-burst.md decision 3), and the rail's slug,
     which goes through everything in its lane, teammates included;
  3. a tower or an enemy decides to engage a seat's air target only from
     inside that seat's sight box (the seat's own rule, applied to what
     the seat sent);
  4. nothing on the ground collides with one - walls, tiles, tanks and
     frogs pass under it - and its own landing is its owner's business
     (a drone's burst);
  5. walks go by key.
- **The drawn world** (`simulation::present`): `PresentWorld` carries the
  drawn air targets and `air_contact` meets an opposing one for a bullet,
  so a client's provisional bullet stops at a drawn drone (whether it
  comes down is the room's word). A rail drawn on the press does not
  pierce them: the drones it went through come down on the room's
  `DroneDowned`.
- **For the gravity well (BB-42)**: it reads `air_targets` and moves one
  through a mover of its own (`Drone`'s ground velocity), as it moves
  grenades and drums; a drone pulled off course re-steers to its aim, a
  diving one is dragged off its point. That arm is its PR's.

### 3.4 The armory's swarm

Into `maps/armory.toml` (docs/sonic-hammer.md §3.5, docs/emp-burst.md §3.4,
docs/gauss-rail.md §3.4); nothing placed by weapons 1-3 moves:

| Mark | Cell | Why |
|---|---|---|
| `D` FPV crate | 7,10 | The reserved column's third cell, two cells south-east of the start |
| `D` FPV crate | 30,15 | On the enemy side, so an enemy on shells collects it and its rule shows with no tuning |
| `t` `p` a tree and a pine | 3,6 and 4,6 | A grove two cells from the start: a seat parks between them to hide from enemy drones, and watches a drone break a crown |
| `p` two pines | 29,2 and 30,2 | Trees on the enemy side, behind the brick stub, that an enemy breaks toward from a seat's drones |
| `Q` player gun tower | 5,15 | Anti-air on the players' side: guards the frog (2,12) from a hunter's drones and the south-west from the band's |

The rest it needs is there: cover to launch from behind (the brick stub at
26,2..4, the screen at 27,8..11, the brick at 28,13..14, the iron at
33..34,8..9, the glass house), the enemy gun tower (34,14) and both teslas
(3,4 and 34,3) for air defence, the lake and the lava to fly over, the trees
in the grass patch (16,11 and 10,13).

```
     0         1         2         3
     012345678901234567890123456789012345
 0   ................................L...
 1   .+..........ggggg...............L...
 2   ............g...g...II....b..pp.L...
 3   ............g.s.g...II....b.....L.E.
 4   ...P...e...*g...g.........b.....L...
 5   ............ggggg...........zzz.L...
 6   ...tp..R.............o..........L...
 7   .............................e..L...
 8   ....S..H.........*...f%%%..b.....II.
 9   ...........................b.....II.
10   .......D.............o...*.d..H.....
11   ...........wwwwwT..........g........
12   ..F....r...wwwww..............R.....
13   ..........Twwwww...WWWWWWWW.b.......
14   .......r...wwwww...WWWWWWWW.b.....G.
15   .....Q.....wwwww...WWWWWWWW...D.....
16   .a.................WWWWWWWW....m....
17   ...................WWWWWWWW.........
```

(`r` the column still reserved for weapons 5 and 6.) The linter passes it
(`supported_maps_no_new_errors`): one warning, the glass house's sealed
inside, and the corridor notes it had; the band routes round the player
gun tower's reach (`player_tower_reach`). The armory's CPU thumbnail pin
is re-baselined for the crates, the trees and the tower.

## 4. AI

An enemy carrying drones uses them by `fpv_rule`, never through the generic
tiers (`generic_fire(FpvSwarm)` is false): attack still lines up and settles
but never launches, `Brain::wants_breach` never latches with drones (they fly
over the wall; its stuck escape gets it unstuck), and it fires no shells
while it carries them - as with the hammer and the rail, until its six are
spent. And every enemy, whatever it carries, reacts to a seat's drone
diving at it (the `air` tier).

### What it is handed

`Game::fpv_senses` runs once per frame in `enemy_phase`, only when some
live enemy carries online drones (`any_fpv`), over every seat (`FpvSeat`:
position, on the field, concealed, `Game::sight_on` at it), the players'
frog, the standing trees, the frame's `Terrain`, the grid and the
dangers, and gives each drone tank an `FpvSense`:

```rust
pub struct FpvSense {
    /// The seat a drone launched now would go for: the nearest live seat
    /// on the field whose sight box this tank stands in, that it knows of
    /// - within its sight under the sky (`Game::sight_on`) and not hidden
    /// from it (concealed and not hit-alerted, the attack tier's rule) -
    /// and that is not under canopy. Ties to the lower seat. What
    /// `Ai::shot_at_seat` records.
    pub at_seat: Option<(u8, Position)>,
    /// The nearest seat it knows of in the same way that *is* under
    /// canopy, and the centre of the tree's crown over it.
    pub canopy: Option<(u8, Position)>,
    /// A hunter's quarry - the players' frog, alive, within this tank's
    /// sight and not under canopy.
    pub quarry: Option<Position>,
    /// The seat in `at_seat` (or `canopy`) has a line of sight to this
    /// tank (`Terrain::line_of_sight`): it stands in the open.
    pub exposed: bool,
    /// Where to launch from instead, while exposed: the cover spot
    /// (`fpv::cover_spot`, below), latched on the `Ai`.
    pub cover: Option<Position>,
    /// A seat it knows of is within `fpv_ai_min_range_px` with a line of
    /// sight to it: the point to back off to.
    pub back_off: Option<Position>,
    /// Its own drones in the air.
    pub in_air: u8,
}
```

- **The cover spot** (`fpv_cover_spot`): over the map cells within
  `fpv_ai_cover_px` (5 cells) of the tank: usable, connected to it
  (`Grid::connected`), the cell's centre inside the seat's sight box (so it
  can still launch from there), at least `fpv_ai_min_range_px` from the
  seat, not inside a danger the tank does not own, and with no line of
  sight from the seat to it (a wall between); the nearest to the tank,
  ties to the lower cell index. It is latched on the `Ai`
  (`Ai::cover_spot`, with its age) and searched again only when there is
  none, it no longer hides the tank from the seat, it is no longer joined
  to it, or `fpv_ai_cover_seconds` (4) passed - at most one search per tank
  per latch, about 121 cells and a line of sight each.
- **The back-off point**: `fpv_ai_min_range_px` plus a cell from the seat
  along the line from the seat through this tank, held a half cell inside
  the field, and moved out of any danger it lands in.
- **The sight box binds by construction**: `at_seat` and `canopy` name a
  seat only if this tank stands inside its box, measured in the simulation
  against the seat's real position, and the tank is not a training dummy
  (`Ai::frog_only`); the launch checks it again (§1).

### The rule (`fpv_rule`), in priority order

1. **A training dummy** (`Ai::frog_only`): `None`.
2. **Too close**: a `back_off` point it can drive to (`Brain::can_reach`),
   the tank healthy (under `enemy_flee_damage`) and not a guard that holds:
   `Approach { to: back_off, why: "back off" }` - it keeps its distance;
   drones do not need it close.
3. **Into cover**: `exposed`, a `cover` spot, healthy, not a guard that
   holds, none of its drones in the air: `Approach { to: cover, why: "to
   cover" }` - "preferably from behind a wall". Arrived (within half a
   cell), it is no longer exposed and the next arm launches.

   **Patience**: backing off and making for cover together get
   `fpv_ai_cover_seconds` (4) since its last launch (`Ai::place_waited`);
   out of it the tank launches from where it stands. In a hedge maze the
   way to a cover spot can leave the seat's box, and a tank went back and
   forth between the cover and the chase with its drones unflown; on an
   archipelago a tank backed off into a shore for four seconds (§11). The
   patience starts over on a launch and whenever the tank stands in cover
   from the seat, not when the seat is lost for a moment.
4. **Launch, one at a time** - none of its drones in the air:
   1. a seat in `at_seat`: `Launch { want: Seat(s), at_seat: Some(s), why:
      "cover" | "open" }`;
   2. a seat in `canopy`: `Launch { want: Point(crown), at_seat: Some(s),
      why: "canopy" }` - the drone bursts in the leaves over the seat,
      three of them fell a broadleaf, and the seat is out in the open;
   3. a hunter's `quarry`: `Launch { want: Frog, at_seat: None, why:
      "frog" }` - `quarry` is set for a hunter alone; a hunter carrying the
      swarm fights the seat like everyone else (`generic_fire` false keeps
      it off its frog's ring), and sends its drones at the frog when no
      seat is to be had.
   With `Ai::fire_timer` running (`fpv_enemy_gap_seconds`, 3.0, from the
   last launch) `act_special` holds instead.
5. **Watch**: one of its drones in the air and a seat (or the quarry) to
   work on, healthy: `Hold { face, why: "watch" }` - it stands where it
   launched from while the drone works. Hurt, it leaves this to `flee`.
6. Otherwise `None`: the tree goes on (chase, patrol and the seeks find it a
   seat; attack never launches).

**Spaced out**: a launch waits for none of its drones to be in the air
**and** for `fpv_enemy_gap_seconds` since the last - so a seat meets them
one at a time and can shoot each down, rather than six at once. A flight
from the distance it keeps lasts about a second and a half, so at the
defaults the gap is what paces it: a drone every three seconds, six in
about eighteen.

**The tell**: no wind-up (`tell_seconds` none, the rail's "the charge is its
tell"): the drone itself is the warning. From the distance an enemy keeps
(`fpv_ai_min_range_px` and more) it takes the climb and a cruise - over a
second from the launch to its burst - with its lamp
blinking in the enemy's red, quicker in the dive, its rotor wash and buzz
(§5), and off the screen an `ArrowKind::Drone` for every enemy drone locked
on this seat or on the players' frog (§5). The decision to launch is the
moment the sight box is checked and `shot_at_seat` recorded, and the launch
is that tick.

**What an enemy's drone goes after** is what the rule chose, nothing else
(§1): a seat hidden in grass is not launched at (the AI does not know it is
there), though grass would not hide it from the air.

**The hold-still clocks**: a tank holding to watch commands no movement (the
stuck clock resets, as any deliberate hold), and one moving to cover or
backing off is driving where it asked to.

### Reacting to a seat's swarm: the `air` tier

BB-40's "an enemy targeted by a diving drone breaks toward the nearest tree
or turns its minigun on it if it carries one". `Game::air_threats` runs
once per frame in `enemy_phase` (empty with no seat's drone past its
climb), and puts on the `Ai` of each enemy a seat's drone is coming at -
locked on it, or with no lock going for a point within its blast's reach
of the tank (`fpv_blast_radius_px` and half a cell), so a tank whose
drone lost its lock to a tree keeps standing under it - an `AirThreat`
(`Ai::air_threat`):

```rust
pub struct AirThreat {
    /// The nearest such drone within this tank's sight
    /// (`Game::enemy_sight`): its ground point and its seconds to arrive.
    pub drone: Position,
    pub eta: f32,
    /// This tank stands inside the sight box of the seat that sent it:
    /// it may shoot at the drone (§1, "Air defence keeps the sight box").
    pub may_shoot: bool,
    /// Its hull is under a tree's crown already: it stands.
    pub covered: bool,
    /// The nearest tree within `fpv_ai_tree_px` (5 cells) it can get under
    /// (`fpv_tree_spot`): the usable cell nearest that tree joined to where
    /// it stands - then nearest the tank, then the lower cell index - and
    /// the tree's centre.
    pub tree: Option<(Position, Position)>,
}
```

The cells round a tree are mostly inside the nav grid's clearance, so a
cell under the crown is seldom open: the tank drives to the open cell
nearest the tree and from there straight at the tree's centre until its
hull is under the crown, against the trunk if need be.

The `air` tier (1.45, after the wreck check, before `special`):
`condition(b.ai.air_threat.is_some() && b.me.windup().is_none())`,
`action("air", act_air)` - a tell or a charge in progress commits, as
everywhere; anything else is dropped for the drone. `act_air`, in order:

1. **Flak**: it carries an online minigun and `may_shoot` - it faces the
   drone (`Dir::toward` its ground point), holds still and holds the
   trigger while the drone is ahead, within `fpv_ai_flak_range_px` (192)
   and its ground point within `fpv_ai_flak_align_px` (20) of the facing's
   line (`"flak"`). The minigun's own bursts and cooldown pace it. No
   `shot_at_seat`: a drone is not a seat.
2. **Under the crown**: `covered` - stand (`"canopy"`). The drone loses its
   lock the tick the hull gets there.
3. **Into the trees**: a `tree` - steer to its cell on foot
   (`Brain::steer_out`), a heading held from the fight that does not lead
   there giving way as `act_dodge`'s does, then drive at the tree
   (`"tree"`).
4. **Break**: none of those, the drone within `fpv_ai_break_px` (112):
   drive across its line - the cardinal perpendicular to the drone's
   bearing whose way is open (`walls_ahead` and the grid), the clockwise
   one first, then the other (`"break"`). Moving at the commit is what
   makes a dive miss.
5. Otherwise (the drone still far, nothing to do yet): `Failure`, the tree
   goes on.

`AiSnapshot::air` names the arm. No RNG, ties on cell index and `Dir`
order; a seat's drone is the only threat (an enemy does not shy from its
own side's). Standing for a drone - `"watch"`, `"canopy"` - is a
deliberate hold (`Ai::air_hold`, the probe's `air` row).

### Air defence by the towers

Both sides' towers, in `tower_phase` (`towers.rs`), an air target treated as
the tower's rule treats a tank, with the sight-box rule applied to the
seat that sent it (`box_allows` for an air target reads its owner seat's
tank):

- **The tesla coil**: besides its charge on a tank, which runs as ever, an
  opposing air target within `tesla_range` of the coil (to its ground
  point) - the nearest, ties to the lower key - is struck by an arc of its
  own every `tesla_air_gap_seconds` (0.5, `Tower::air_cooldown`, its own
  clock): `Event::TeslaStrike { x0, y0, x1, y1, chained: false }` to the
  drone as drawn (lifted by its height), then `strike_air(.., Tesla, ..)`.
  No charge spent, no chain, no damage roll; no line of sight needed - the
  drone is in the air. An offline coil (`Tower::disabled`) arcs nothing; a
  burning one arcs on its fire factor's pace, as it charges.
- **The gun tower**: while an opposing air target is in `gun_tower_range`
  (to its ground point) with a line of sight to its ground point (the
  bullets fly level, and a wall in between would take them), it fights
  that **before** any tank - picked as it picks a tank, the nearest, kept
  until another is `tower_switch_margin_px` nearer; it leads it by
  `gun_tower_air_lead` (1.0) of its ground velocity over the bullets'
  flight, turns at its rate, fires its bursts within its cone, holds while
  a friend is in the line - every rule of its tank fight. Its bullets
  strike the drone through the sweep.
- **The bio slush** lobs at the ground and ignores the air.

### What changes for enemies that carry something else

- The `air` tier, above, for every enemy a seat's drone locks.
- **The EMP's rule**: a seat carrying online drones is worth
  `emp_ai_special_value` like any seat with an online special - the pulse
  grounds its halo and every drone it has in reach.
- **The hammer's and the rail's rules**: nothing new (their AIs do not
  aim at drones; their strikes down them when they meet them, §1).

### Off the field and asleep

A field map's far tank coasting on its last intent has no sense and no
threat: it launches nothing (its trigger released) and does not react. A
drone already in the air flies on whatever its launcher does. A disabled
enemy neither launches nor reacts (it does not think).

## 5. Drawing

All of it in the effects language (docs/effects.md): whole 2 px blocks,
ramp steps, Bayer fades; composed at draw time as pure functions of the
drone, the tank and their age, hashed from ids and slots, never rolled.

- **A drone** (`fpv::compose_drone`), a 4 x 4-block quad on the 2 px grid,
  drawn at its ground point lifted by its height - **in blocks twice the
  size, 4 px, above `FPV_WASH_HEIGHT_PX`** (`fpv::drone_block`: up at
  cruise height it is nearer the eye, as a missile's sprite grows with its
  height, and an 8 px quad over a 40 px tank read as a speck, not the
  threat it is - settled on the screenshots), so a launch grows as it
  climbs past 24 px and a dive shrinks as it comes down:
  - the frame: the two diagonals - the corner blocks and the middle four -
    in `#252525` (`SMOKE[0]`, the tanks' outline);
  - the body: the middle 2 x 2 in `#5A5A5A` (`SMOKE[2]`), its top-left block
    lit `#7E7E7E` (`SMOKE[3]`);
  - the rotors: at each corner a block of `#C1C1C1` (`SMOKE[5]`) that
    steps between the corner and its outer neighbour every
    `FPV_ROTOR_FRAME_SECONDS` (0.05), the four phased by quarter turns and
    each drone by its id, so the props read as turning;
  - the lamp: the body's bottom-right block, in the launcher's colour while
    it blinks on - a seat's `tank::team_color(seat)` (in co-op each seat's
    own), an enemy's `indicators::HOSTILE` - and `#373737` while off;
    `fpv_lamp_hz` (3) blinking in the launch and the cruise, `fpv_dive_lamp_hz`
    (10) in the dive. Drawn in the glowing pass, so it reads at night;
  - in the dive, two `#9E9E96` (`SMOKE[4]`) blocks streaking a block and two
    behind it along its heading.
- **Its shadow** (`fpv::compose_shadow`), on the ground under it, drawn in
  the lit pass after the floor marks, under the tanks: a 3 x 3-block square
  of black at `fpv_shadow_opacity` (0.3), a block smaller from 24 px up,
  shifted 4 px along `shadow_dir` (the missiles' shadow rule).
- **Draw order**: shadows after the floor and the fish
  (`game::paint_drone_shadows`); the drones themselves after everything
  standing, trees included - a drone flies over the crowns - in the lit
  pass (`game::paint_drones`, so at night only the lamps glow, the quads
  are lit by what is around them); the lamps' blocks, the halos' among
  them, and their light in the glowing pass (`game::drone_lamps`).
- **The halo** (`fpv::compose_halo`), drawn with its tank in the y-sorted
  standing walk, over the hull: each occupied slot a drone at
  `FPV_HALO_HEIGHT_PX`, bobbing one block on a 1.3 s cycle phased by slot
  (drawn only), rotors turning, lamps blinking at `fpv_lamp_hz` phased by
  slot; their shadows with the tank's shadow (`game::halo_of`, drawn with
  its tank by `draw_one_tank`). Disabled, they drop to the ground over
  0.25 s from the EMP's start (eased from `Tank::disabled` against
  `emp_disable_seconds`, so a replica draws the same) and lie there,
  rotors still, lamps `#373737`; they are back up the frame it ends. On a
  wreck, each tumbles to the ground over `FPV_HALO_FALL_SECONDS` from
  `Tank::wreck_timer` 0 - the falling silhouettes below - and lies dark
  beside the wreck until its fire is out (`wreck_timer` at
  `wreck_burn_seconds`), then is drawn no more. A client drawing its own
  launch draws the halo without the drones it has in the air that the
  room's count does not know of yet (`Tank::fpv_lifting`).
- **Rotor wash and buzz** (`fx.rs`, particles, so `rand::rng()`): a drone
  in the air below `FPV_WASH_HEIGHT_PX` (24) - the climb and the dive -
  throws `ParticleKind::Dust` off the ground under it at `fpv_wash_rate`
  (30 a second); in the cruise it sheds a `#C1C1C1` speck ten times a
  second that hangs and fades - the buzz a silent game can show; a falling
  one puts up a smoke puff 12.5 times a second. A launch puts a ring of
  dust round its slot.
- **The burst** (`drone_show`): a small fireball leaning down the dive,
  `BlastFx::shaped(at, BlastKind::Oil, BlastShape::Shot { dir })` at
  `fpv_blast_fx_scale` (0.35) - the missile's burst, smaller -, the ripple
  `Shockwave::scaled(at, fpv_shock)` (0.15), the impact flash, a scorch at
  that scale on dry ground, the grass round it flattened; sparks and a
  puff in `fx.rs`. In a crown: the same fireball among the leaves and a
  spray of leaf chips (`ParticleKind::Chip` in the leaves' colours -
  vegetation keeps its greens), no scorch.
- **Shot down**: the cause's own hit - a bullet's impact where it crossed
  the drone's column, the tesla's bolt to the drone as drawn with its ring
  (`TeslaStrike`), the EMP's ring, the rail's slug - and on `DroneDowned`
  sparks at the drone as drawn in the cause's colour; then **the spinning
  fall**: the quad alternates its two silhouettes, the frame as an X and as
  a + (a 45-degree step), `fpv_fall_spin_hz` (6) times a second, drifting
  with what is left of its ground speed, its lamp dark, a smoke puff off it
  every 0.08 s. The crash (`DroneCrashed`): the missile dud's dust puff and
  pale sparks where it lands, a splash on water.
- **The light** (`weather::lights`): a drone in the air throws an
  unshadowed point light in its lamp's colour, 18 px, at `fpv_lamp_light`
  (0.35) on the frames its lamp is on - so an incoming drone reads in the
  dark, the tell is fair at night; the burst's fireball throws a blast's
  light. Under a dark sky every lamp block - the halos' too - also lays a
  small stepped glow (`ground_light`, 10 px) in the glowing pass.
- **The module** (`tankdesign`, `lines/vanguard.py`, `module_fn('fpv')`): a
  ground-control relay on the roof - the missiles' hardpoint, shared, since
  a tank carries one special at a time (`hp.get('fpv', hp['missiles'])`): a
  squat gunmetal box (5 x 4, chamfered) with a dark screen panel on its top
  (3 x 2, the FPV feed), a steel panel antenna across its front (a 5 x 1 bar
  on a one-pixel mast, `'cylv'`) and a link lamp on the antenna's tip. Four
  cells, `TANK_MODULE_FPV_COL` = 40..43 (`tank_modules.png` grows from the
  rail's 40 to 44 columns, 1760 x 480): 0 armed, the screen dark and the
  link lamp dim (`DIM_LINK`, `BLUE_DK`); 1 linked, the screen lit `'ion'`
  and the lamp `'white'`; 2 launch, the screen and the antenna `'white'`;
  3 offline, the screen dark, the antenna scorched (`RUST_DK`), no lamp.
  `module_cols` (now ten entries): 2 while `Tank::fpv_flash`
  (`fpv_flash_seconds`, set by `kick_fpv` on every launch - the room's, a
  replica's `Fired`, a client's press), 3 while `special_down()`, 0 and 1
  alternating at 4 Hz while one of its drones is in the air (the owner slot
  on `DroneState`, so a replica knows), else 0. `render.SHOWN_TOGETHER`
  leaves it out with the grenade launcher, the hammer and the EMP, which
  share the roof. Three chassis in a screenshot before it is settled (§12).
- **The crate**: row 16 of `gen_crates.py`'s sheets (`crates_sheet.png` 280
  x 680, `pickup_glyphs.png` 24 x 408). Its symbol, 10 x 10 design px: a
  quadcopter from above - four rotor discs at the corners on diagonal arms
  to a body, the rotors' hubs and the body (`o`) lit as its lamps:

  ```
  '.X......X.',
  'XoX....XoX',
  '.XXX..XXX.',
  '..XXXXXX..',
  '...XooX...',
  '...XooX...',
  '..XXXXXX..',
  '.XXX..XXX.',
  'XoX....XoX',
  '.X......X.',
  ```

  Ink (`punypalette.PICKUP_INK['fpv_swarm']`, admitted on the crate sheets
  alone like the others): **two-tone, as the heat shield is** - a warm
  ivory quadcopter (shade `#BFA77A`, base `#FFF0C8`) with crimson lamps
  (`#FF2D5F`, the "light", which the generator uses for the hubs and the
  body) - settled by rendering every candidate beside the sixteen crates
  as they now stand (the hammer's sky blue, the EMP's cobalt and the
  rail's magenta taken). Every single loud hue left sits on a neighbour:
  crimson alone read as the health cross a crate away (its red is 20
  degrees off), violet as the grenades and the shield, azure as the
  hammer, vermilion as the flamethrower, and greens are out (objects stay
  off the grass's colour, Oto's art direction). A white X with red lights
  is a pattern no other crate has and reads as a drone with its lamps on;
  ivory rather than silver keeps it off the minigun's grey-blue. The
  crimson is the HUD's accent too. `target/devshots/fpv-crate-row.png` is
  the seventeen side by side.
- **The HUD**: `hud::WeaponSlot::of` gives the drones in the halo (less
  `Tank::fpv_lifting`) in `HUD_FPV_COLOR` (`#FF2D5F`, the ink's crimson)
  and the glyph; offline, the
  EMP's `WPN OFFLINE`. The ring's ammo pips are the drones left against
  `full_load` (6). Nothing new to lay out.
- **Off the screen** (`indicators.rs`): `ArrowKind::Drone { diving }` for
  every opposing drone locked on this seat - or on the players' frog - off
  the screen (`Scene::drones`, from the drones' `lock`, so a replica draws
  them): a notched arrowhead in `HOSTILE`, rimmed near-black, a 3 x 3-block
  X (the drone, `DRONE_X`) at its tail, blinking at `fpv_lamp_hz`, at
  `fpv_dive_lamp_hz` and the X white once it dives. Never merged, never
  left out past `indicator_max_arrows` (the tells' rule). A seat's own
  drones and its teammates' get none; a couch's shared screen shows the
  first seat's, as it shows the first seat's enemies.
- **The dev overlay** (`Overlays::projectiles`): every air target's strike
  box and each drone's line to its aim, the lock's slot by it.

## 6. Tuning

New group `fpv` (every row live unless marked), two rows in `towers`, one
in `enemies`:

| Row | Default | Range | Doc |
|---|---|---|---|
| `fpv_drones_per_pickup: i32` | 6 | 1..=12 | Drones one FPV crate loads into the halo. One per press. |
| `fpv_reload_seconds` | 0.4 | 0..=10 | Seconds between two launches. |
| `fpv_launch_seconds` | 0.35 | 0.05..=2 | How long a drone climbs out of the halo before it turns for its aim. |
| `fpv_launch_speed` | 70 | 0..=400 | How fast it drifts outward from its slot while it climbs (px/s). |
| `fpv_cruise_height` | 36 | 8..=120 | How high it flies (px): what lifts it over its shadow and how tall its strike box is. |
| `fpv_speed` | 230 | 20..=1000 | Its top ground speed (px/s), faster than any tank on its own engine. |
| `fpv_accel` | 700 | 1..=10000 | How fast it reaches it (px/s²). |
| `fpv_turn_rate_deg` | 300 | 1..=3600 | How fast it turns toward its aim (degrees a second). |
| `fpv_commit_px` | 64 | 0..=400 | How near its aim it commits to the dive, which then follows nothing. |
| `fpv_dive_speed` | 320 | 20..=2000 | How fast it comes down from the commit to the point (px/s). |
| `fpv_max_flight_seconds` | 6 | 0.5..=30 | How long after its launch a cruising drone dives where it is going, battery out. |
| `fpv_aim_px` | 192 | 16..=1000 | With nothing to lock, how far ahead of the launcher it dives. |
| `fpv_damage` | 8 | 0..=100 | Its burst's damage at the centre to the side opposing the launcher, falling to nothing at the radius; no roll. |
| `fpv_blast_radius_px` | 44 | 0..=200 | Its burst's reach (px). |
| `fpv_knockback_speed` | 30 | 0..=400 | Its burst's shove at the centre (px/s). |
| `fpv_tree_damage` | 4 | 0..=100 | What a burst in a tree's crown takes off the tree. |
| `fpv_canopy_px` | 12 | 0..=32 | How far past its cell a tree's crown shelters (px): a hull overlapping it is under canopy, a dive landing in it bursts in the leaves. |
| `fpv_drone_hits: i32` | 1 | 1..=10 | Bullets that bring a drone down; every other strike downs it at once. |
| `fpv_hit_half_px` | 6 | 1..=24 | Half a drone's width (px), the strike box's. |
| `fpv_fall_gravity` | 500 | 50..=5000 | How fast a downed drone falls (px/s²). |
| `fpv_fall_drag` | 3 | 0..=20 | How fast it loses its ground speed (per second). |
| `fpv_fall_spin_hz` | 6 | 0..=30 | How often its silhouette turns as it falls. |
| `fpv_gust_factor` | 0.5 | 0..=2 | How much of a sandstorm's gust carries a drone in the air. |
| `fpv_flash_seconds` | 0.2 | 0..=2 | The module's launch cell. |
| `fpv_lamp_hz`, `fpv_dive_lamp_hz` | 3, 10 | 0.5..=20, 0.5..=30 | How fast its lamp blinks, and in the dive. |
| `fpv_lamp_light` | 0.35 | 0..=2 | The light its lamp throws at night, against a headlight's. |
| `fpv_blast_fx_scale` | 0.35 | 0.1..=2 | Its burst's fireball against a missile's. |
| `fpv_shock` | 0.15 | 0..=2 | Its burst's ripple and shake against a tank dying's. |
| `fpv_shadow_opacity` | 0.3 | 0..=1 | Its shadow. |
| `fpv_wash_rate` | 30 | 0..=200 | Dust blocks a second off the ground under a low drone. |
| `fpv_enemy_gap_seconds` | 3.0 | 0.1..=20 | Seconds between an enemy's launches; it also waits for its last drone to come down. |
| `fpv_ai_min_range_px` | 160 | 0..=600 | How close an enemy with drones lets a seat it can see come before it backs off. |
| `fpv_ai_cover_px` | 160 | 0..=480 | How far it looks for cover to launch from. |
| `fpv_ai_cover_seconds` | 4 | 0.5..=20 | How long it keeps a cover spot before looking again. |
| `fpv_ai_flak_range_px` | 192 | 0..=600 | How near a drone diving at an enemy with a minigun has to be before it fires at it. |
| `fpv_ai_flak_align_px` | 20 | 0..=64 | How far off its facing's line the drone may be. |
| `fpv_ai_tree_px` | 160 | 0..=480 | How far an enemy looks for a tree to hide under from a drone. |
| `fpv_ai_break_px` | 112 | 0..=480 | With no tree and no minigun, how near the drone is before it breaks across its line. |
| `tesla_air_gap_seconds` (`towers`) | 0.5 | 0..=10 | Seconds between two of a tesla coil's arcs at drones. |
| `gun_tower_air_lead` (`towers`) | 1.0 | 0..=2 | How much a gun tower leads a drone. |
| `enemy_special_weapon_fpv_share` (`enemies`, `@ Restart`) | 0 | 0..=1 | The share of special-carrying enemies that spawn with the FPV swarm instead, decided by a hash of the spawn point and the slot - never the round's RNG - so at 0 nothing changes. |

Constants (geometry, not feel) in `fpv.rs`: `FPV_HALO_START_DEG` (30),
`FPV_HALO_RADIUS_FRACTION` (0.55), `FPV_HALO_HEIGHT_PX` (14),
`FPV_HALO_FALL_SECONDS` (0.5), `FPV_ROTOR_FRAME_SECONDS` (0.05),
`FPV_WASH_HEIGHT_PX` (24), `FPV_LAUNCH_HANDOVER_SECONDS` (0.25, §8); the
burst's ripple is the `fpv_shock` row; `game.rs`'s `HALO_SETTLE_SECONDS`
(0.25, §5).

## 7. Text

Data names by family (`named("tool", ..)`), so no `text::keys` constant.

| Key | en | sl |
|---|---|---|
| `tool-fpv_swarm` | fpv swarm | roj dronov |
| `tool-short-fpv_swarm` | fpv | dron |

Both inside the tool list's 144 pt and the short budget's 48 at the font's
own widths; measured in Phase 2 by `every_language_fits_every_budget`. The
HUD shows the glyph and a count; the weapon has no other words.

## 8. Wire

Protocol 18 (from the rail's 17), once in the PR.

- `WeaponKind::FpvSwarm`, appended to `ALL`; `drawn_on_press` true.
- **A new family, `Snapshot::drones`**, keyed by the drone's id:

  ```rust
  /// One FPV drone in the air (`fpv::Drone`). A replica never flies one,
  /// so where it is, how high, which way and in what stage travel, with
  /// the halo slot it left and whose it is.
  pub struct DroneState {
      pub id: u16,
      /// Quarter pixels (`quantise_pos`): the point on the ground under it.
      pub x: i16,
      pub y: i16,
      /// Height above the ground in quarter pixels.
      pub height: i16,
      /// `quantise_heading` of its heading over the ground.
      pub heading: u8,
      /// `fpv::DroneStage::code`: 0 launch, 1 cruise, 2 dive, 3 falling.
      pub stage: u8,
      /// The halo slot it left: its rotors' and lamp's salt.
      pub halo: u8,
      /// The launcher's owner slot: the lamp's colour (a seat's team
      /// colour below the room's first enemy slot, else an enemy's red)
      /// and its relay module's link cell.
      pub owner: u16,
      /// What it is locked on (`fpv::DroneLock::code`): a tank's owner
      /// slot, `LOCK_FROG`, `LOCK_ENEMY_FROG` or `LOCK_NONE` - what the
      /// incoming arrow reads.
      pub lock: u16,
  }
  ```

  `Snapshot::normalise` sorts and dedups it; `delta.rs` treats it as a
  positioned family (`drones`, `drones_moved`, `drones_gone`). The
  measured sizes (`src/net/mod.rs`): a full snapshot 487 B, a moving delta
  172 B, a busy one 259 B, an idle one the 52 B header - three bytes more
  each than the rail's, the new family's three list lengths. Like
  `MissileState`, no speed, aim, stage timer or hits travel.
- **Events** (`net/events.rs`, mirrors of the simulation's):
  `WireEvent::DroneLaunched { id: u16, slot: u16, x: i16, y: i16, target:
  u16, frog: bool }` (the launch point; `target` an owner slot or
  `NO_TARGET`), `DroneBurst { id, slot, x, y, crown }`, `DroneDowned { id,
  x, y, height, by: AirStrike }` (the drone's ground point), `DroneCrashed
  { id, x, y }`. `Event::DroneLockLost` is on `NOT_SENT` (the `lock` field
  is the state). `AirStrike` is `air::AirStrike` itself, serialised in its
  variant order. The tesla's arc at a drone travels as the `TeslaStrike` it
  already is.
- `TankState` needs nothing: `weapon` is the special carried, `ammo` the
  drones in the halo; the module's flash rides `Fired`'s `kick_turret`, its
  link cell the drones' `owner` (`count_drones_out` on the replica).
- **What a replica draws** (`apply_drones`): drones from the family,
  spawned, moved and dropped by id, their lock rebuilt from `lock`; a stage
  newly reached starts its clock over and `tick_presentation` runs their
  ages, which the rotors, the lamps and a falling drone's spin read.
  Interpolated like missiles (`interp.rs`: position and height linearly,
  the heading by `lerp_heading`, carried on past the newest snapshot along
  its heading at the speed it covered). On `DroneBurst`, `drone_show`
  leaning down the replica's copy's heading; the launch, the downing and
  the crash put up their particles off the events (`fx.rs`). The halo from
  the tank's count and position. `DrawableState::drones` is what the
  round-trip tests hold the two sides to (`fpv_drones_reach_the_replica`).
- **What is drawn at once** (decision 3 of BB-36, the hammer's §3.3): the
  shooter's launch, **eased onto the room's timeline**:
  - *On the press* (`Predictor`'s swarm arm: the press edge, drones left
    less the owed above 0, the local gate open, the special not down): it
    sets the gate to `fpv_reload_seconds`, owes the drone and, while
    presses are drawn, queues `PressShow::Drone(DronePress { slot, origin,
    out })` and the drawn press. `slot`, `origin` and `out` are the halo
    slot the room will launch from (the top one, `fpv_drones - 1` less the
    owed), its ground point round the hull as the sandbox has it and its
    outward bearing (`Game::seat_drone_slot`).
  - *The eased climb* (`round.rs`, `OwnDrone`, `fly_own_drones`): the round
    flashes the module (`flash_seat_fpv`) and keeps the launch with Δ, how
    far ahead of the picture the press lands on the room's clock - the
    incoming fire's lead, `incoming_lead_ticks` as last measured
    (docs/online-coop-prd.md §4.16). τ seconds after the press it draws
    the drone in the replica (`add_drone`, an id past the wire's,
    `OWN_DRONE_ID_BASE`) at `fpv::launch_path(origin, out, a(τ))` with
    `a(τ) = min(τ * h / (h + Δ), h)` (`h` = `FPV_LAUNCH_HANDOVER_SECONDS`,
    0.25, held to at most `fpv_launch_seconds` so the handover always falls
    in the climb). The room's copy appears in the picture when render time
    reaches its launch tick - τ = Δ - and the round, reading the replica's
    claim (`Show::presses_drawn` on the seat's `DroneLaunched`), pairs it
    with the oldest launch waiting and keeps it off the picture
    (`remove_drones`) until it is `h` into its own climb; then the client's
    goes and the room's is the drone, on the same point of the same path.
    The drone lifts off on the press frame, climbs a little slower than the
    room's for its first `h + Δ`, and is never drawn twice
    (`a_launch_is_drawn_on_the_press_and_handed_to_the_rooms_copy`). The
    climb is the lock's business at no point (§1), so nothing the client
    does not know enters it; a difference between the drawn hull and the
    room's at the press is a pixel or two at the handover.
  - *The halo*: the seat's halo and HUD count lose the drone on the press
    frame - `Tank::fpv_lifting`, the launches drawn that the room's count
    does not know of yet, set every frame.
  - *Claimed by input tick*: the room logs `Fired` then `DroneLaunched`
    for the seat in one tick; `confirm_presses` claims the drawn press for
    the `Fired`, and `presses_drawn` leaves the seat's `DroneLaunched` out
    of the replica's events.
  - *Refused*: a drawn launch the room never makes is dropped after the
    refusal wait (`Predictor::refusal_after`) plus `h`; the halo has its
    drone back. A room `Fired` the client never drew (its local gate
    refused, or prediction is off) claims nothing: the room's drone is
    drawn as it comes, and `seed_gate` seeds the local gate.
- **What is not predicted, and why**: everything after the climb - the
  cruise, the lock, the dive, the burst, being shot down - is drawn from the
  room's snapshots on the picture's timeline, the drone and what it dives
  at together. A drone is meant to be shot down at the last moment by the
  very tank it dives at; a client drawing it ahead of the room's word would
  draw a burst the room never had whenever that happens, which is the
  counter's whole point (§12, decision 15).
- **What stays the room's**: the lock, the flight, the burst and its
  damage, shoves (`Shoved` to an owned hull), tile deaths, crown damage,
  every strike in the air (`DroneDowned`), the crash.
- **This seat's bullets at drones**: a provisional bullet meets a drawn
  enemy drone (`PresentWorld::air_contact`) and plays its impact there at
  once; the room judges the bullet against the drones rewound to this
  client's view (`HitBoxFrame::air`), so the hit it drew is the room's hit
  too, and the drone falls when the room's `DroneDowned` is handed over.
- The rig: `a_seats_drone_reaches_the_replica_once` and
  `an_enemys_drone_reaches_the_replica_locked_on_the_seat` (`rig::Lockstep`).

## 9. Determinism

- **The swarm draws no RNG of its own.** The lock is a distance with
  slot ties, the flight a pure function of its state, the aim and the
  round clock (the gust), the burst a side blast whose damage range is a
  point (`roll_damage` draws nothing for it), a bullet's or an arc's strike
  a count. What its burst sets off draws as it always does - a fence's
  one-shot odds, a drum's fuse where its rules roll.
- **Walk orders**: launches in the order the triggers were pulled (seats in
  index order, then enemies in the collect pass's order, which the RNG
  already depends on); `guide_drones` and `resolve_drones` by id;
  `air_targets` by key; the towers' arcs and picks in cell order, targets
  by distance then key; the strike walks of the EMP, the hammer and the
  rail put air targets after everything they already strike, by key.
- **The AI**: `fpv_rule` chooses by fixed priority; `pick_lock`,
  `cover_spot` and `tree_spot` tie on slot or cell index; `act_air`'s break
  ties in `Dir::ALL` order; `air_threats` is built in slot order.
- **The swap** is the hammer's hash; the new entry runs only with its share
  above 0.
- **A round without the swarm replays byte for byte**: no crate kind is
  rolled anywhere, every share defaults to 0, no enemy carries drones, so
  no drone is ever in the air: `air_targets` is empty (the sweep's air
  candidates, the towers' air arms, the strike walks' air arms and the
  history's air list are all empty), `air_threats` is empty (the `air` tier
  a `false` with no state touched), no sense is built, `side_blast` is
  `side_blast_sparing` with nothing spared, the shared id counter is drawn
  only by what it always was. The rank renumbering in the sweep keeps every
  existing tie's order. `determinism_tests`' pinned streams, the probe
  fixtures' ceilings and every thumbnail pin but the armory's stay as they
  are.

## 10. Tests

`simulation::fpv_tests` (headless, tiny inline maps; whole rounds):

- The weapon: `an_fpv_crate_arms_the_swarm_and_replaces_the_special_carried`,
  `a_press_launches_one_drone_and_spends_one`,
  `the_drone_locks_the_nearest_enemy_in_the_seats_sight_box`,
  `with_no_enemy_in_the_box_it_dives_on_the_aim_point`,
  `the_drone_flies_over_walls_and_dives_on_its_target`,
  `only_the_opposing_side_is_hurt`, `a_shield_soaks_the_burst`,
  `a_disabled_tank_launches_nothing_and_keeps_its_drones`,
  `the_drones_fly_out_on_the_end_screen_and_hurt_nobody`.
- Trees: `a_tank_under_a_tree_is_never_locked`,
  `a_drone_loses_its_lock_when_its_target_goes_under_a_tree`,
  `a_dive_into_a_crown_hurts_only_the_tree`.
- Air targets: `a_minigun_bullet_brings_an_opposing_drone_down`,
  `shells_pass_under_a_drone`, `a_tesla_arcs_a_drone_in_reach_without_charging`,
  `an_offline_tesla_arcs_no_drone`, `a_gun_tower_brings_a_drone_down`,
  `a_seats_own_towers_leave_its_drones_alone`, `strike_air_downs_a_drone_once`,
  `a_seats_drawn_bullet_stops_at_an_enemy_drone_not_its_own`.
- Determinism: `the_swarm_draws_no_rng`, `a_round_with_drones_replays_bit_for_bit`.
- The AI: `the_swarm_launches_at_a_seat_in_its_box_without_line_of_sight`,
  `an_enemy_never_launches_at_a_seat_from_outside_its_sight_box`,
  `the_swarm_launches_one_at_a_time_with_its_gap`,
  `a_hunter_sends_its_drones_at_the_frog`,
  `the_swarm_never_launches_at_a_seat_hidden_in_grass`,
  `a_training_dummy_never_launches`, `the_generic_tiers_never_launch_a_drone`,
  `the_swarm_breaks_the_crown_over_a_hidden_seat`,
  `an_enemy_that_cannot_reach_cover_launches_from_the_open`,
  `an_enemy_with_a_minigun_shoots_down_the_drone_diving_at_it`,
  `an_enemy_breaks_toward_the_nearest_tree` (under the crown before the
  dive, nothing taken), `an_enemy_with_no_tree_breaks_across_the_drones_line`,
  `an_enemy_drone_locked_on_the_seat_is_in_its_scene` (and its arrow).

Headless halves:

- `air::tests`: `the_strike_box_is_the_column`, `drawn_is_lifted_by_the_height`.
- `fpv::tests`: `halo_slots_are_fixed_round_the_hull`,
  `the_launch_path_is_the_same_whatever_the_lock`,
  `a_drone_climbs_cruises_dives_and_arrives`,
  `the_turning_circle_is_inside_the_commit` (no orbit at the defaults),
  `a_downed_drone_falls_and_lands`, `a_gust_carries_a_dive_off_its_point`,
  `pick_nearest_ties_on_the_lower_key`; the composers
  `a_drone_is_on_the_grid_and_pure`, `a_drone_up_high_is_drawn_twice_the_size`,
  `the_lamp_blinks_at_its_rate`, `the_rotors_turn`,
  `a_falling_drone_alternates_its_silhouettes`,
  `the_halo_settles_when_disabled_and_falls_on_a_wreck`.
- `indicators`: `a_drone_coming_at_the_seat_off_the_screen_has_an_arrow_whatever_the_cap`,
  `a_drone_arrow_has_an_x_at_its_tail`.
- `devserver`: the PICKUP category's count (16 to 17), the compact
  snapshot's size (`fpv`, `fpv_out` and `drones` left out while empty).
- `thumbnail`: the armory's pin. `maplint`: the armory as it lints.

Wire:

- `events.rs`: the samples gain the four drone events and
  `DroneLockLost` (not sent); the variant count.
- `wire.rs`: `normalise_sorts_and_dedups_every_family` covers the drones.
- `delta.rs`: the random snapshots carry drones through the round trip;
  the size bounds re-measured.
- `apply.rs`: `fpv_drones_reach_the_replica` - a seat and the enemies armed
  through `debug_set_tank`: every drone the room flies, the seat's and the
  enemies', in every stage, is on the replica the same picture after every
  apply, and re-encoding the replica gives the room's bytes.
- `round.rs`: `a_launch_is_drawn_on_the_press_and_handed_to_the_rooms_copy`
  - on the press frame one drone up and one fewer in the halo, the room's
  copy kept off the picture until the handover, never two drawn.
- `rig.rs` (`Lockstep`): `a_seats_drone_reaches_the_replica_once`,
  `an_enemys_drone_reaches_the_replica_locked_on_the_seat`.
- The room server's `cargo test -p bongbong-server` as it stands: drones
  are one more family in the snapshot it encodes once for everyone.

## 11. Probe

- **Defaults first**: `just probe-fixtures` and `just probe-fields`
  unchanged, passing their recorded ceilings untouched.
- **With the crate**: the same two sweeps with `--crate fpv_swarm`. AFK:
  enemies collect the crates and launch at the seat from inside its box,
  from cover where they find it.
- **Armed enemies**: the same sweeps with `--tuning armed.json`,
  `{"enemy_special_weapon_chance": 1.0, "enemy_special_weapon_fpv_share":
  1.0}` - every enemy that would carry a special spawns with drones.
- The probe's tank line gains `fpv=` (drones in the halo) and `out=` (in the
  air); its fire tuple counts the drones, so a launch is a trigger pull for
  `FIRED_RECENTLY_FRAMES`. A tank holding to watch its drone (`"watch"`) or
  under canopy from one (`"canopy"`) is a deliberate hold
  (`TankSnapshot::special_why`, `TankSnapshot::air`), not a stall, a stale
  start, low progress or jitter - beside the hammer's tell and skid, the
  EMP's disabled state and the rail's charge and edge hold. Moving to cover
  or backing off is driving it asked for.
- **`offbox-fire` gains the drones' lock**, beside the missiles':
  `drone_locks_offbox` counts every enemy `DroneLaunched` locked on a seat
  whose launcher stood outside that seat's box that frame (`--json-out`
  `fire`), and each is an `offbox-fire` anomaly; `shots_at_seats_offbox`
  reads `shot_at_seat` at the decision as ever. `seat-hits-offbox` (a seat's
  hit on an enemy outside the seat's box - informational) may move with the
  crate: a seat's drone chases its lock out of the box, which is the seat's
  own drone.
- The `air` tier is not exercised by the probe's scenarios (no scenario
  fires the seat's special); it is held by its `mechanics_tests`.
- **The bar**: every crate and armed run within the defaults' ceilings,
  `offbox-fire` 0. An exceedance is read round by round from its `ANOMALY`
  lines; one the swarm's own action causes - a tank stranded on its way to
  cover, holding where it can never launch, spinning between back-off and
  cover, piling up at a spot, launching from off the box - is fixed, not
  re-baselined. Recorded here in Phase 2: the totals at the defaults, with
  the crate and armed, and what moved.

## 12. Interactions, decisions, what is left out

### Interactions with what ships

| With | What happens |
|---|---|
| Rainbow shield | Soaks a burst's damage; does not stop the drone |
| Heat shield, speed boost, ooze coat | Nothing; a boosted tank still does not outrun a drone in the cruise |
| Portals | Flown over; a locked target that teleports is lost |
| Water, ice | Flown over; a burst on water splashes and scorches nothing; wash is spray over water |
| Night, storm, fog | The lamps throw light, so a drone reads in the dark; an enemy launches only at a seat within its sight under the sky |
| Rain, snow | Nothing |
| Sandstorm gusts | Carry drones in the cruise and the dive: dives land downwind |
| Towers | The tesla arcs drones on its own clock; the gun tower turns on drones first, fully led; the bio slush ignores them; a burst damages a tower in reach like any tile; an offline tower shoots nothing |
| Frogs | An enemy hunter sends drones at the players' frog; a burst hurts the opposing side's frog; a frog under canopy is hidden like a tank |
| Crates | Flown over; a burst breaks breakable crates in reach; the FPV crate cooks off |
| Drums, oil, fires | A burst sets drums off by their rules; oil and fires untouched |
| Trees, grass | Trees hide what is under them and take crown bursts; grass hides nothing from the air and is flattened round a burst |
| Glass, walls, lamp posts | Flown over; a burst cracks tiles in reach |
| Lava, the volcano | Flown over; bombs and eruptions do nothing to drones |
| Shells, plasma, lasers, flames, missiles, grenades | Pass under; none touch a drone |
| Wrecks | A wreck's halo falls; a drone locked on a tank that is wrecked dives where it was |
| Field maps | The sight box binds every launch and every air defence decision; a far coasting tank launches nothing and does not react; a wave tank that spawned with drones carries its halo in |
| The couch and its split | Each seat's drones lock in its own box and blink in its colour; drawn in both halves' worlds; incoming arrows shared |
| Training | `drop = ["fpv_swarm"]` works by its name; a dummy never launches |
| The C2 commander | Nothing: drones are not units; a tank holding to watch is an ordinary unit |
| Online | §8 |

### Interactions with the sonic hammer (built here)

| Hammer | Swarm |
|---|---|
| The wave's front reaches a drone in the cone | The opposing side's drones are knocked down (`by: Sonic`); the shooter's side's ride it out |
| A drone's burst on a hammer tank in its tell | Hurts it; the tell holds (a tell commits) |
| A sonic shove on a tank with a halo | The halo moves with the hull |
| The `air` tier and a hammer tell | A tank in a tell does not break for a drone |
| The online claim | One pending claim per kind: a seat's hammer blast and drone launch claim their own events |
| The probe's holds | The tell, the skid, the watch and the canopy hold are each deliberate |

### Interactions with the EMP (built here)

| EMP | Swarm |
|---|---|
| The ring reaches a drone | It falls dead, any side (`by: Emp`) - the blind ring's rule |
| The ring reaches a tank carrying drones | Its special is down: the halo settles, nothing launches, the stock is kept; `WPN OFFLINE` on a seat |
| An enemy EMP's value of a seat with drones | `emp_ai_special_value`, like any online special |
| A disabled enemy targeted by a seat's drone | Does not react (it does not think) |
| The predictor's `offline_left` | A press during it is not a launch |
| The dangers | A cover spot and a tree spot are kept out of every danger |

### Interactions with the gauss rail (built here)

| Rail | Swarm |
|---|---|
| A slug's lane crosses a drone's strike box | It is pierced and downed (`Pierced::Drone`, `by: Rail`), any side; the slug keeps its damage past it |
| A drone's burst on a charging tank | Hurts it; the charge holds (a hit does not lapse it) |
| The `air` tier and a charge | A charging tank does not break for a drone |
| The client's drawn slug | Pierces the drawn drones (`rail_trace`); the room's `DroneDowned` is the drone's fate |
| A charging rail's lane and a cover spot | A cover spot inside a lane danger is not taken |

### Decisions taken

1. **The halo is the stock, drawn, not in the world**: no body, no hit box,
   no wire entry beyond the count. Rejected: six hit-testable drones round
   every hull - a bullet aimed at a tank would be stopped by its halo (a
   shield nobody asked for), and every armed tank would put six more
   entries on the wire to say what its count already says. *For Oto.*
2. **Fixed slots, no orbit**: the halo is a function of the hull's position
   alone, so the launch point is the same in the room and on every client,
   and a settled or falling halo stays put. Rejected: a ring orbiting on the
   round's clock (the launch point would depend on time, and a disabled or
   wrecked halo would slide along the ground).
3. **The lock is picked at the launch and kept**: "each press sends one at
   the nearest enemy". Rejected: a seek at the top of the climb, like the
   missiles' (the seat would not know what it sent the drone at).
4. **Only the dive's end bursts, as a small side blast** (the missiles' and
   grenades' rule, fixed damage, no roll) - no contact in the air. Rejected:
   a direct hit only (a drone that misses by a pixel would do nothing) and
   bursting on any hull met on the way down (a friend driving under your
   dive would soak it).
5. **Trees are one box with four rules** (never locked, the lock lost,
   a crown burst hurts the tree, the spared hull) - a hard counter that
   lasts as long as the tree, which the enemies' rule shreds on purpose.
   *For Oto.*
6. **Grass hides nothing from the air**; the AI still launches only at a
   seat it knows of, so grass keeps hiding a seat from an enemy's decision.
7. **Air targets are struck by bullets, the tesla, the gun tower, the EMP,
   the hammer and the rail - not lasers.** "The minigun and the tesla can
   shoot drones down; shells and plasma cannot" read as the anti-air being
   the weapons that spray or arc; the laser is a gun-line beam like a
   shell. *For Oto.*
8. **One bullet brings a drone down, and a downed drone is a dud** (no
   blast): a drone is fragile, and shooting one down over your own head
   should not hurt you. Rejected: drone health against bullet damage (a
   number nobody sees) and a downed drone bursting where it falls (the
   defender would be punished for the counter). *For Oto.*
9. **The strike box is the column from the shadow to the body**: a drone
   is drawn lifted by its height, and a bullet crossing what the player
   sees - the drone, its shadow or between - should hit it.
10. **Air defence keeps the sight box**: a tower or an enemy engages a
    seat's drone only from inside that seat's box, so every bullet a seat
    can be hit by while defending is fired from where it can see.
11. **The gun tower turns on drones first, fully led; the tesla arcs them
    on a clock of its own, without charge.** "The gun tower becomes
    anti-air" made a priority, and a coil that needs 1.3 s to charge would
    never catch a drone crossing its 112 px.
12. **No pre-launch tell**: the drone's flight - over a second, blinking,
    with an arrow off the screen - is the warning before anything lands
    (the rail's "the charge is its tell"). Rejected: a wind-up before the
    launch (a delay on top of a flight that is already the warning).
13. **The enemy launches one at a time with a gap, from cover where it can,
    keeps its distance and watches its drone**, and fires no shells while it
    carries drones.
14. **The reaction is flak, then a tree, then a break across the line**, in
    a tier of its own above the special tier that gives way to a tell or a
    charge in progress (those commit).
15. **The press is drawn at once and eased onto the room's timeline;
    everything after the climb is the room's.** The climb runs a little slow
    for the lead plus a quarter second and hands over at equal age, so no
    jump and no double. Rejected: a provisional drone for its whole flight,
    the shells' model, and the room's copy carried into the present, the
    incoming fire's - both draw the drone ahead of the room's word, and a
    drone shot down in its last fifth of a second (the counter, used as
    intended) would be drawn bursting on a tank it never reached. *For
    Oto.*
16. **Drones are a family of their own on the wire**, keyed by id like the
    missiles, with their lock - what the incoming arrow needs.
17. **The climb is the same whatever the lock** (straight out from the slot),
    which is what makes 15 exact.
18. **The sonic wave downs only the opposing side's drones; the EMP downs
    every drone; the rail every drone in its lane** - each weapon's own rule
    of who it hurts. *For Oto.*
19. **The launcher wrecked mid-flight: its drones fly on** and their kills
    are its (the missiles').
20. **A teleporting target is lost**: a portal is a way to shake a drone.
21. **A gust carries drones**: the sandstorm's one effect on them.
22. **The crate cooks off**: six charges.
23. **Crate ink signal crimson** (`#FF2D5F`), the one non-green gap left.
    *For Oto*, with a screenshot of the crate beside the other sixteen in
    Phase 2; the two-tone stands by.
24. **The module is a relay on the roof hardpoint**, which it shares with
    the missiles, the grenade launcher, the hammer and the EMP.
25. **A disabled tank's halo settles and lifts again, its stock kept**: the
    EMP's offline rule.
26. **Ids from the one projectile counter**, as the missiles and grenades
    take theirs.
27. **A seat's press with nothing lockable dives on the aim point**, even
    with a tree-covered enemy in the box - BB-40 read literally; the
    seat aims at the tree itself to break it.

### Not in this PR

- Placing the swarm in the shipped levels - level design, a follow-up.
- Drones on the minimap.
- A hit arc for a drone's burst (the missiles' burst has none either).
- The hammer's AI shouting down a drone diving at it - the `air` tier's
  flak is the minigun's; a hammer tank would need the tell's timing against
  the drone's arrival.
- Enemies using their miniguns or hammers to defend another tank from a
  drone.
- Drones fighting drones, blasts and fires touching drones in the air, and
  lasers striking them.
- A seat's press breaking a tree over a hidden enemy by itself.
- The incoming fire this client draws in the present stopping at its own
  drones, which are drawn on the picture's timeline: a flak stream and the
  drone it downs can stand a few pixels apart for a frame; the drone falls
  on the room's word.
- A departing animation for a halo replaced by another crate.
- Sound effects - the game has no audio yet.
- The gravity well's pull on drones - BB-42 adds its arm on the air
  targets (§3.3).
- A probe scenario in which the seat fires its special.

### Needs from the shared path

What this design takes from the hammer's, the EMP's and the rail's
implementations beyond what their docs give:

1. **`SpecialUse` open to a `Launch` with a want**, `act_special` applying
   it as `Fire` and recording `Ai::air_want`, and the one place in
   `enemy_phase`'s collect pass where a trigger meets the simulation (the
   rail's need 8) handing it to the tank before `dispatch_fire`.
2. **`Ai::think` taking one more per-frame perception** (`threat`) beside
   `sense` and `dangers`, kept on the `Brain`, and the tier list open to a
   tier above `special` that defers to `Tank::windup`.
3. **`Trigger::Press` for the swarm** (the rail's `Trigger`), and
   `Tank::special_down` gating a launch the way it gates every special.
4. **`presses_drawn` claiming per kind through `WireEvent::press_show`**
   (the swarm's is `DroneLaunched` with a seat), and the round able to read
   a claimed event's payload (the drone's id) - `presses_drawn` returning
   the claimed events' indices with their kinds.
5. **`PressShow` open to a show that lives past its frame** - the round
   keeps a drawn launch until its handover - and `Game::draw_press_show(seat,
   ..)` for the launch's one-frame part (the wash ring and the module's
   flash).
6. **The strike walks open to an air arm after their last**:
   `tick_sonic_waves` (after the grenades), `tick_emp_pulses` (after the
   missiles - the EMP's doc already keeps the place), the rail's pierce
   walk (`Terrain::pierce_rewound`'s candidates and `Pierced`), and
   `PresentWorld::rail_trace`.
7. **`ArrowKind`'s never-dropped set** (the tell's) open to `Drone`.
8. **The probe's holds and its off-box readings** open to the swarm's
   holds and its lock (`TankSnapshot::special_why` among them).
9. **The predictor's local offline** (the EMP's `offline_left`) read by
   every arm's gate, so a press in an offline is not a launch.
10. **The armory's reserved cell 7,10** for this crate, and the free cells
    30,15, 3,6, 4,6, 29,2, 30,2 and 5,15 left free.
