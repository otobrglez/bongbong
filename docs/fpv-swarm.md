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
  ripple (`SHOCK_DRONE`, 0.15 of a tank dying), the impact flash, a small
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
three sparks, `fx::ImpactKind::Dud`), removed, and nothing else - no blast,
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
| `src/air.rs` (new) | Air targets (§3.3): `AirTarget`, `AirKey`, `AirStrike`, `strike_box`, `drawn`; the rules every reader keeps, in its doc |
| `src/fpv.rs` (new) | The weapon's headless half. `Drone` (id, owner, stage, ground point, height, heading, speed, age, stage time, lock, aim, hits; `advance(dt, wind)`, `commit`, `down(by)`, `launch_path`, `in_air`, `as_target`), `DroneStage`, `DroneLock`, `AirWant`, `halo_slot`, `halo_drawn` (the slots as drawn: bob, settle, fall), `pick_lock` (pure, over plain candidates - shared by the simulation and the AI's sense), `crown_box`, `cover_spot` and `tree_spot` (the AI's searches, pure over a grid and closures), the composers `compose_drone`, `compose_shadow`, `compose_halo` (pure, `pyro::Shape`s), `module_cell` |
| `src/simulation/fpv.rs` (new) | The world half. `fire_fpv` (the dispatch arm's body), `launch_drone`, `guide_drones` (locks and aims, before the step), `resolve_drones(f, live)` (bursts and crashes), `drone_burst`, `drone_show` (the cosmetic half - fireball, ripple, scorch, leaves - which a replica's `DroneBurst` calls too), `Game::{air_targets, strike_air, canopy_over, drones}`, `fpv_field`/`fpv_sense` (what the AI is handed, §4), `air_threats` (§4), `seat_drone_slot` |
| `src/simulation/hits.rs` | `ShellTarget::Air(AirKey)`; `sweep_rewound` takes `strikes_air` and meets opposing air targets; `HitBoxFrame::air` (lag compensation); the air candidates in `pierce_rewound` |
| `src/simulation/weapons.rs` | The `ActiveWeapon::FpvSwarm` dispatch arm; `Projectile::strikes_air` (bullets) |
| `src/simulation/combat.rs`, `src/simulation/missiles.rs` | `BlastParams::drone`; `side_blast_sparing` (`side_blast` with a list of hulls and frogs left out) |
| `src/simulation/mod.rs` | `Frame::pending_drones`; `spawn_pending` ids; `guide_drones` beside `guide_missiles`; drones advanced in `step_world`'s fixed-step loop beside the missiles; `resolve_drones` after `resolve_missiles` (both branches); `resolve_projectiles::<Bullet>`'s air hits; `enemy_phase` (the senses, the threats, `Ai::air_want` handed to the tank before `dispatch_fire`); `tick_presentation` (a replica's drones age, fall and spin; the module flash); the swap's table entry; `Event::{DroneLaunched, DroneBurst, DroneDowned, DroneCrashed, DroneLockLost}` |
| `src/simulation/towers.rs` | The tesla's air arc (`Tower::air_cooldown`), the gun tower's air pick and lead; `box_allows` for an air target |
| `src/simulation/emp.rs` | The drones' arm in `tick_emp_pulses`, after the missiles |
| `src/simulation/sonic.rs` | The drones' arm in `tick_sonic_waves` |
| `src/simulation/gauss.rs` | Drones in the pierce walk (`Pierced::Drone`) |
| `src/simulation/present.rs` | `PresentWorld::shot_contact` meets drawn opposing drones for a bullet (`Contact::Air`); `rail_trace` pierces them; `Game::draw_press_show`'s drone arm; `Game::{add_provisional_drone, hide_drones}` |
| `src/simulation/replica.rs` | `DrawableDrone`; `DrawableTank::fpv` |
| `src/tank.rs` | `fpv_drones`, `fpv_flash`, `fpv_want`; `ActiveWeapon::FpvSwarm` (`name`, `full_load`, `tell_seconds` none, `trigger` `Press`), `SPECIAL_WEAPONS`; `weapon_ammo`/`take_weapon`/`empty_stock`/`wants_pickup`; `kick_fpv`; the module's cells in `module_cols` |
| `src/tower.rs` | `Tower::air_cooldown` |
| `src/pickup.rs` | `PickupKind::FpvSwarm` (`fpv_swarm`, row 16, its ink, cooks off) |
| `src/ai.rs` | `SpecialSense::Fpv(FpvSense)`, `fpv_rule`, `SpecialUse::Launch`, `AirWant`, `generic_fire(FpvSwarm)`; `AirThreat`, the `air` tier and `act_air`; `Ai::{air_want, cover_spot}`; `SEEK_SPECIALS` gains the swarm; `AiSnapshot::{air, threat}` |
| `src/indicators.rs` | `ArrowKind::Drone`, the incoming drones' arrows (never merged, never dropped); `Scene::drones` |
| `src/hud.rs` | `HUD_FPV_COLOR`, the `weapon_color`/`weapon_pickup` arms |
| `src/game.rs`, `src/render/game.rs` | The halo in its tank's place of the standing walk; drones' shadows after the floor marks; drones over everything standing; their lamps in the glowing pass; the dev overlay's strike boxes and aims |
| `src/fx.rs`, `src/burst.rs` | Rotor wash and buzz off drones; a downed drone's spark and smoke; the crash (`ImpactKind::Dud`); leaves off a crown burst; the halo's fall on a wreck |
| `src/weather.rs` | A drone's lamp light |
| `src/fish.rs` | `DroneBurst` and `DroneCrashed` among the scares |
| `src/net/wire.rs` | `WeaponKind::FpvSwarm` (`drawn_on_press`), `DroneState`, `Snapshot::drones`, `drone_stage`, `drone_lock`, `AirStrike` |
| `src/net/delta.rs` | `drones`, `drones_moved`, `drones_gone` |
| `src/net/events.rs` | `WireEvent::{DroneLaunched, DroneBurst, DroneDowned, DroneCrashed}`, the `press_show` arm, `drone_lock_lost` on `NOT_SENT` |
| `src/net/encode.rs`, `src/net/apply.rs` | `drones(game)`, `apply_drones`; the bursts' and crashes' shows; the `kick_turret` arm |
| `src/net/interp.rs` | Drones blended and carried on like missiles |
| `src/net/predict.rs`, `src/net/round.rs` | `PressShow::Drone`, the eased launch (§8), the shown seat's halo count |
| `src/simulation/debug.rs`, `src/devserver.rs` | `set_tank`'s `fpv_drones`; the snapshot's `fpv`, `drones`, `air`, `threat`; `spawn_pickup {kind: "fpv_swarm"}` |
| `src/bin/probe.rs` | The tank line's `fpv=`/`out=`, the fire tuple, the drone locks off the box, the holds |
| `src/editor/mod.rs` | `Tool::Pickup(PickupKind::FpvSwarm)` (`fpv_swarm`) |
| `maps/armory.toml` | Its crates, two pairs of trees and a player gun tower (§3.4) |
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
3. The dispatch arm in `weapons::dispatch_fire_from` (`fire_fpv`); the
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
cover spot and a tree spot are kept out of them, `Brain::out_of_danger`),
the press show's claim by input tick, `spawn_pickup {kind: "fpv_swarm"}`,
the probe's `--crate fpv_swarm`, `fx::ImpactKind::Dud` (the EMP's, for a
crash).

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
  `Ai::air_want` right before `dispatch_fire`; the launch takes it (and
  clears it), `None` reading as `Nearest`.
- **`Ai::think`** takes `threat: Option<AirThreat>` beside `sense` and
  `dangers`, kept on the `Brain` (§4); the `air` tier reads it.
- **`ArrowKind::Drone { diving }`**, put with the never-dropped kinds
  (`Teammate`, `Frog`, `Volcano`, `Tell`): never merged, never left out past
  `indicator_max_arrows` (§5).
- **`PressShow::Drone(DronePress)`** and the round's own list of drawn
  launches, which live past the frame they were pressed in (§8); the
  replica's `presses_drawn` claim gains the `DroneLaunched` arm through
  `WireEvent::press_show`.
- **The strike walks of weapons 1-3** gain an air arm each, after
  everything they already strike: `tick_sonic_waves` (drones of the
  opposing side, after the grenades), `tick_emp_pulses` (every drone, after
  the missiles - the EMP's doc keeps the place), the rail's pierce walk
  (`Terrain::pierce_rewound`'s air candidates and `Pierced::Drone`, every
  drone in the lane).
- **`side_blast_sparing(f, center, owner, params, spared)`** - the
  missiles' and grenades' `side_blast` with a list of hulls and frogs left
  out; `side_blast` is it with none.
- **`Projectile::strikes_air()`** - true for bullets, false for shells and
  plasma; `resolve_projectiles` passes it to the sweep.

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
- **The one setter**: `Game::strike_air(f, key, by, at) -> bool` - counts a
  bullet's hit or downs the target outright (every other cause), logs
  `Event::DroneDowned`, and answers whether it went down. Nothing else
  changes an air target.
- **The hit test**: `Terrain::sweep_rewound` takes `strikes_air`; with it
  set, every air target not of the shooter's side (for a tower's shot, not
  of the tower's) is a candidate at its `strike_box` grown by the shot's
  half extent - and by `player_shot_hit_pad_px` for a seat's shot, the pad
  every enemy box takes - as `ShellTarget::Air(key)`. Ranks for an exact
  tie: seats, enemies, **air**, frogs, tiles, walls (every existing pair
  keeps its order). Air targets are read from the world at the sweep, as
  the tanks are, not from the frame's `Terrain`, so a bullet meets a drone
  where this tick's step left it.
- **Lag compensation**: `HitBoxHistory::record` keeps every air target's
  strike box beside the enemies' boxes (`HitBoxFrame::air`, sorted by key),
  and a seat's shot rewound to its view tick meets the enemy drones where
  its client drew them; one with no entry is met where it is.
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
  drawn air targets, `shot_contact` meets an opposing one for a bullet
  (`Contact::Air { key }`), so a client's provisional bullet stops at a
  drawn drone; `rail_trace` pierces them.
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
(3,3 and 34,3) for air defence, the lake and the lava to fly over, the trees
in the grass patch (16,11 and 10,13).

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
11   ...........wwwwwT..........g........
12   ..F....r...wwwww..............R.....
13   ..........Twwwww...WWWWWWWW.b.......
14   .......r...wwwww...WWWWWWWW.b.....G.
15   .....Q.....wwwww...WWWWWWWW...D.....
16   .a.................WWWWWWWW....m....
17   ...................WWWWWWWW.........
```

(`r` the column still reserved for weapons 5 and 6.) Checked again in
Phase 2 against the linter (connectivity, the band's capacity - the player
gun tower's reach is a detour the band routes round, `player_tower_reach`),
and the armory's CPU thumbnail pin is re-baselined for the crates, the
trees and the tower.

## 4. AI

An enemy carrying drones uses them by `fpv_rule`, never through the generic
tiers (`generic_fire(FpvSwarm)` is false): attack still lines up and settles
but never launches, `Brain::wants_breach` never latches with drones (they fly
over the wall; its stuck escape gets it unstuck), and it fires no shells
while it carries them - as with the hammer and the rail, until its six are
spent. And every enemy, whatever it carries, reacts to a seat's drone
diving at it (the `air` tier).

### What it is handed

`fpv_field` is built once per frame in `enemy_phase`, only when some live
enemy carries online drones: every seat (position, hull box, on the field,
concealed, `Game::sight_on` at it, under canopy and by which tree), the
players' frog (position, alive, under canopy), the standing trees' crowns,
each enemy's drones in the air (by owner slot), the frame's `Terrain` and
grid. `fpv_sense` then gives each thinking drone tank an `FpvSense`:

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

- **The cover spot** (`fpv::cover_spot`): over the nav cells within
  `fpv_ai_cover_px` (5 cells) of the tank: open, connected to it
  (`Grid::connected`), the cell's centre inside the seat's sight box (so it
  can still launch from there), at least `fpv_ai_min_range_px` from the
  seat, not inside a danger the tank does not own (`out_of_danger`), and
  with no line of sight from the seat to it (a wall between); the nearest
  to the tank, ties to the lower cell index. It is latched on the `Ai`
  (`Ai::cover_spot`) and searched again only when there is none, it no
  longer hides from the seat, the seat changed, or `fpv_ai_cover_seconds`
  (4) passed - at most one search per tank per latch, about 121 cells and a
  line of sight each.
- **The back-off point**: `fpv_ai_min_range_px` plus a cell from the seat
  along the line from the seat through this tank, held a half cell inside
  the field, and moved out of any danger it lands in.
- **The sight box binds by construction**: `at_seat` and `canopy` name a
  seat only if this tank stands inside its box, measured in the simulation
  against the seat's real position, and the tank is not a training dummy
  (`Ai::frog_only`); the launch checks it again (§1).

### The rule (`fpv_rule`), in priority order

1. **A training dummy** (`Ai::frog_only`): `None`.
2. **Too close**: a `back_off` point, the tank healthy (under
   `enemy_flee_damage`) and not a guard that holds: `Approach { to:
   back_off }` (`"back off"`) - it keeps its distance; drones do not need
   it close.
3. **Into cover**: `exposed`, a `cover` spot, healthy, not a guard that
   holds, none of its drones in the air: `Approach { to: cover }`
   (`"cover"`) - "preferably from behind a wall". Arrived (within half a
   cell), it is no longer exposed and the next arm launches.
4. **Launch, one at a time** - none of its drones in the air:
   1. a seat in `at_seat`: `Launch { want: Seat(s), at_seat: Some(s), why:
      "cover" | "open" }`;
   2. a seat in `canopy`: `Launch { want: Point(crown), at_seat: Some(s),
      why: "canopy" }` - the drone bursts in the leaves over the seat,
      three of them fell a broadleaf, and the seat is out in the open;
   3. a hunter's `quarry`: `Launch { want: Frog, at_seat: None, why:
      "frog" }`.
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
or turns its minigun on it if it carries one". `air_threats` is built once
per frame in `enemy_phase`, only when a seat's drone is in the air, and
hands each enemy a seat's drone has locked an `AirThreat`:

```rust
pub struct AirThreat {
    /// The nearest seat's drone locked on this tank, past its climb, within
    /// this tank's sight (`Game::enemy_sight`): its ground point and its
    /// seconds to arrive at its speed.
    pub drone: Position,
    pub eta: f32,
    /// This tank stands inside the sight box of the seat that sent it:
    /// it may shoot at the drone (§1, "Air defence keeps the sight box").
    pub may_shoot: bool,
    /// The nearest tree spot (`fpv::tree_spot`): a nav cell within
    /// `fpv_ai_tree_px` (5 cells), open, connected, out of every danger,
    /// where this tank's hull would be under canopy; ties to the lower
    /// cell index.
    pub tree: Option<Position>,
}
```

The `air` tier (1.45, after the wreck check, before `special`):
`condition(b.threat.is_some() && b.me.windup().is_none())`, `action("air",
act_air)` - a tell or a charge in progress commits, as everywhere; anything
else is dropped for the drone. `act_air`, in order:

1. **Flak**: it carries an online minigun with rounds and `may_shoot` - it
   faces the drone (`Dir::toward` its ground point, the larger offset's
   cardinal), holds still and holds the trigger while the drone is within
   `fpv_ai_flak_range_px` (192) and its ground point within
   `fpv_ai_flak_align_px` (20) of the facing's line (`"flak"`). The
   minigun's own bursts and cooldown pace it. No `shot_at_seat`: a drone is
   not a seat.
2. **Into the trees**: a `tree` spot - steer to it (`"tree"`), and once
   under canopy hold there (`"canopy"`). The drone loses its lock the tick
   it gets there.
3. **Break**: none of those, the drone within `fpv_ai_break_px` (112):
   drive across its line - the cardinal perpendicular to the drone's
   bearing whose way is open (`walls_ahead` and the grid), the clockwise
   one first, then the other (`"break"`). Moving at the commit is what makes
   a dive miss.
4. Otherwise (the drone still far, nothing to do yet): `Failure`, the tree
   goes on.

`AiSnapshot::air` names the arm, `AiSnapshot::threat` the drone. No RNG,
ties on cell index and `Dir::ALL` order; a seat's drone is the only threat
(an enemy does not shy from its own side's).

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
  drawn at its ground point lifted by its height:
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
- **Draw order**: shadows after the floor marks; the drones themselves after
  everything standing, trees included - a drone flies over the crowns -
  in the lit pass (so at night only the lamps glow, the quads are lit by
  what is around them); the lamps' blocks and their light in the glowing
  pass.
- **The halo** (`fpv::compose_halo`), drawn with its tank in the y-sorted
  standing walk, over the hull: each occupied slot a drone at
  `FPV_HALO_HEIGHT_PX`, bobbing one block on a 1.3 s cycle phased by slot
  (drawn only), rotors turning, lamps blinking at `fpv_lamp_hz` phased by
  slot; their shadows with the tank's shadow. Disabled, they drop to the
  ground over 0.25 s and lie there, rotors still, lamps `#373737`, and lift
  over 0.4 s when it ends (eased from `Tank::disabled`, so a replica draws
  the same). On a wreck, each tumbles to the ground over
  `FPV_HALO_FALL_SECONDS` from `Tank::wreck_timer` 0 - the falling
  silhouettes below - and lies dark beside the wreck until its fire is out
  (`wreck_timer` at `wreck_burn_seconds`), then is drawn no more.
- **Rotor wash and buzz** (`fx.rs`, particles, so `rand::rng()`): a drone
  in the air below `FPV_WASH_HEIGHT_PX` (24) - the climb and the dive -
  throws `ParticleKind::Dust` off the ground under it at `fpv_wash_rate`
  (30 a second) in the ground's dust (`pyro::DUST` by the material under
  it; `BLUE_PALE` spray over water); in the cruise it sheds a pair of
  `#C1C1C1` specks every 0.1 s that hang and fade in eighths - the buzz a
  silent game can show. A launch puts a ring of wash round its slot.
- **The burst** (`drone_show`): a small fireball leaning down the dive,
  `BlastFx::shaped(at, BlastKind::Oil, BlastShape::Shot { dir })` at
  `fpv_blast_fx_scale` (0.35) - the missile's burst, smaller -, the ripple
  `Shockwave::scaled(at, SHOCK_DRONE)` (0.15), the impact flash, a scorch
  at that scale on dry ground, the grass round it flattened. In a crown:
  the same fireball among the leaves and a spray of leaf chips
  (`ParticleKind::Chip` in `pyro::dust_of(Material::Tree)`, the tree's own
  colours - vegetation keeps its greens), no scorch.
- **Shot down**: on `DroneDowned`, the cause's own hit at the drone as drawn
  - a bullet's spark star (`ImpactKind::Bullet`), the tesla's ring
  (`ImpactKind::Tesla`, its bolt drawn from `TeslaStrike`), the EMP's sparks,
  the rail's pierce burst - then **the spinning fall**: the quad alternates
  its two silhouettes, the frame as an X and as a + (a 45-degree step),
  `fpv_fall_spin_hz` (6) times a second, drifting with what is left of its
  ground speed, its lamp dark and a `#373737` smoke puff off it every
  0.08 s (`ParticleKind::Smoke`). The crash: `ImpactKind::Dud` (a dust puff
  and three white sparks) where it lands.
- **The light** (`weather::lights_in`): a drone in the air throws an
  unshadowed point light in its lamp's colour, 18 px, at `fpv_lamp_light`
  (0.35) on the frames its lamp is on - so an incoming drone reads in the
  dark, the tell is fair at night; the burst throws the missile burst's
  light. The halo throws none (the hull's own lamps already do).
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
  to a body, the rotors' hubs and the body (`o`) in the ink's light:

  ```
  'XX......XX',
  'XoX....XoX',
  '.XX....XX.',
  '...X..X...',
  '....oo....',
  '....oo....',
  '...X..X...',
  '.XX....XX.',
  'XoX....XoX',
  'XX......XX',
  ```

  Ink (`punypalette.PICKUP_INK['fpv_swarm']`, admitted on the crate sheets
  alone like the others): signal crimson - shade `#A3133A`, base `#FF2D5F`,
  light `#FF9AB0` - the one gap on the hue wheel that is not green: between
  the laser's hot pink (330 degrees) and the health cross's red (5), at
  345, a blue-red against the cross's orange-red and darker and redder than
  the laser's pink; far from the hammer's sky blue `#46C3F2`, the EMP's
  cobalt `#4F6BFF` and either of the rail's candidates (jade `#36E07A`, hot
  magenta `#FF3DD8`). The symbol's shape - an X with four discs, unlike any
  other - carries as much of it as the hue. To be shown beside the other
  sixteen in a screenshot before it is settled (§12); a two-tone "carbon
  and crimson" (the frame `#30343E` with crimson rotor hubs, the heat
  shield's two-tone convention) stands by if crimson reads as the health
  cross.
- **The HUD**: `hud::WeaponSlot::of` gives the drones in the halo in
  `HUD_FPV_COLOR` (`#FF2D5F`, the ink's base) and the glyph; offline, the
  EMP's `WPN OFFLINE`. The ring's ammo pips are the drones left against
  `full_load` (6). Nothing new to lay out.
- **Off the screen** (`indicators.rs`): `ArrowKind::Drone { diving }` for
  every opposing drone locked on this seat - or on the players' frog - off
  the screen (`Scene::drones`, from the drones' `lock`, so a replica draws
  them): a notched arrowhead in `HOSTILE`, rimmed near-black, a 3 x 3-block
  X (the drone) at its tail, blinking at `fpv_lamp_hz`, at
  `fpv_dive_lamp_hz` and white-cored once it dives. Never merged, never
  left out past `indicator_max_arrows` (the lane threats' and tells' rule).
  A seat's own drones and its teammates' get none; a couch shares them
  (`shared_arrows`).
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
`FPV_WASH_HEIGHT_PX` (24); `SHOCK_DRONE` (0.15) beside `SHOCK_MISSILE`; in
`net::round`, `DRONE_HANDOVER_SECONDS` (0.25, §8).

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
  /// One drone in the air (`fpv.rs`), by `Drone::id`. Only what the
  /// drawing and the indicators need: a replica never flies one.
  pub struct DroneState {
      pub id: u16,
      /// The ground point under it, quarter pixels (`quantise_pos`).
      pub x: i16,
      pub y: i16,
      /// Its height, quarter pixels.
      pub height: i16,
      /// Its ground heading (`quantise_heading`): the dive's streak, the
      /// carry past the newest snapshot.
      pub heading: u8,
      /// `drone_stage`: 0 launch, 1 cruise, 2 dive, 3 falling.
      pub stage: u8,
      /// The launcher's owner slot: the lamp's colour (a seat's team
      /// colour, an enemy's red) and its module's link cell. Never changes.
      pub owner: u16,
      /// What it is locked on (`drone_lock`): a tank's owner slot,
      /// `PLAYER_FROG`, `ENEMY_FROG`, or `NONE` - what the incoming arrow
      /// reads.
      pub lock: u16,
  }
  ```

  `Snapshot::normalise` sorts and dedups it; `delta.rs` treats it as a
  positioned family (`drones`, `drones_moved`, `drones_gone`); a drone's
  height changes most snapshots, so it travels whole most of the time, a
  dozen bytes - and there are rarely more than a few in the air.
  `MissileState`'s precedent: no speed, aim, stage timer or hits travel.
- **Events** (`net/events.rs`, mirrors of the simulation's):
  `WireEvent::DroneLaunched { id: u16, slot: u16, x: i16, y: i16, target:
  u16, frog: bool }` (the launch point; `target` an owner slot or
  `drone_lock::NONE`), `DroneBurst { id: u16, slot: u16, x: i16, y: i16,
  crown: bool }`, `DroneDowned { id: u16, x: i16, y: i16, height: i16, by:
  AirStrike }`, `DroneCrashed { id: u16, x: i16, y: i16 }`.
  `Event::DroneLockLost` is on `NOT_SENT` (the `lock` field is the state).
  `AirStrike` mirrors `air::AirStrike`. The tesla's arc at a drone travels
  as the `TeslaStrike` it already is.
- `TankState` needs nothing: `weapon` is the special carried, `ammo` the
  drones in the halo; the module's flash rides `Fired`'s `kick_turret`, its
  link cell the drones' `owner`.
- **What a replica draws**: drones from the family (`apply_drones`: spawned,
  moved, dropped by id; a falling one spins and drifts from its own age,
  which `tick_presentation` runs), interpolated like missiles (`interp.rs`:
  position and height linearly, the heading by `lerp_heading`, carried on
  past the newest snapshot along its heading at the speed it covered); on
  `DroneLaunched`, the wash ring and the module flash (`kick_turret`); on
  `DroneBurst`, `drone_show`; on `DroneDowned`, the cause's hit; on
  `DroneCrashed`, the dud. The halo from the tank's count and position. A
  drone seen in the family for the first time puts nothing up (its launch
  came with its event).
- **What is drawn at once** (decision 3 of BB-36, the hammer's §3.3): the
  shooter's launch, **eased onto the room's timeline**:
  - *On the press* (`Predictor::pull_trigger`'s swarm arm: the press edge,
    drones left less the owed above 0, the local gate open, the special not
    down - the EMP's `offline_left` included): it sets the gate to
    `fpv_reload_seconds`, owes the drone and, while presses are drawn,
    queues `PressShow::Drone(DronePress { slot, from, out, lead })` and the
    drawn press. `slot` and `out` are the halo slot the room will launch
    from (`fpv_drones - 1` less the owed) and its outward bearing; `from`
    its ground point round the hull **as drawn** (the predicted hull,
    `Game::seat_drone_slot`); `lead` = Δ, how far ahead of the picture this
    press lands on the room's clock - exactly the incoming fire's lead
    (`incoming_lead_ticks(newest.tick, acked, the press's input tick,
    render)` in seconds, §4.16 of docs/online-coop-prd.md): the room
    applies one input a tick from `acked`, so this press's launch is room
    tick `newest.tick + (press - acked)`, and the picture stands `lead`
    behind it. The shown seat's halo loses the drone that frame (the count
    written is the snapshot's less the owed, where `OnlineRound` writes the
    drawn pose), and the module's launch cell flashes.
  - *The eased climb*: `round.rs` keeps each drawn launch as a provisional
    drone in the replica (`Game::add_provisional_drone`, an id in the
    provisional band) and draws it, τ seconds after the press, at
    `fpv::launch_path(from, out, a(τ))` with `a(τ) = τ * h / (h + Δ)` for
    `τ <= h + Δ` (`h` = `DRONE_HANDOVER_SECONDS`, 0.25, held to at most
    `fpv_launch_seconds` so the handover always falls in the climb). The
    room's copy of the drone appears in the picture when render time
    reaches its launch tick - τ = Δ - and from then stands at age `τ - Δ`;
    at `τ = h + Δ` the two ages are equal (`a = h`), so the provisional goes
    and the room's copy - kept off the picture until then
    (`Game::hide_drones`) - is shown from where the provisional stood. The
    drone lifts off on the press frame, climbs a little slower than the
    room's for its first `h + Δ` (between half and two thirds of the pace
    at the links played), and never jumps or doubles. The climb is the
    lock's business at no point (§1), so nothing the client does not know
    enters it. What is left - a difference between the drawn hull and the
    room's (a stage-2 correction), or a tick either way of the room's
    playout (`Mailbox`'s controller) - is a pixel or two, blended out
    linearly over the same span.
  - *The room's copy late*: if at `τ = h + Δ` the room's copy is not in the
    picture yet (a late snapshot), the provisional stays on the room's
    timeline - age `τ - Δ` - to the top of the climb and waits there; the
    copy, when it comes, is shown at its own age.
  - *Claimed by input tick*: the room logs `Fired` then `DroneLaunched`
    for the seat in one tick; `presses_drawn` (the hammer's §3.3) claims
    the seat's next `DroneLaunched` for a drawn `Fired` - it is neither
    drawn again (no second wash ring, no second module flash) nor handed to
    `game.events` - and `round.rs` reads the claimed event's `id` to pair
    the provisional with the room's drone (`Predictor::pair_drone`).
  - *Refused*: a drawn launch nobody claims within the refusal wait
    (`set_refusal_after`) settles back into its slot over 0.25 s - the
    climb drawn backwards - and the owed drone is given back to the halo.
    A room `Fired` the client never drew (its local gate refused, or
    prediction is off) claims nothing: the room's drone and its launch
    show are drawn, and `seed_gate` seeds the local gate.
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
  enemy drone (`Contact::Air`) and plays its impact there at once; the room
  judges the bullet against the drones rewound to this client's view
  (`HitBoxFrame::air`), so the hit it drew is the room's hit too, and the
  drone falls when the room's `DroneDowned` is handed over.
- `delta.rs`: the new family's three lists; its random snapshots fill
  them, and the size bounds are re-measured.

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

`mechanics_tests` (headless, tiny inline maps):

- `an_fpv_crate_arms_the_swarm_and_replaces_the_special_carried` - six
  drones, another special emptied, a second crate refills to six.
- `a_press_launches_one_drone_and_spends_one` - one per press, none while
  held, `fpv_reload_seconds` before the next; `Fired` then `DroneLaunched`
  in one tick; the drone at the top slot's ground point.
- `the_drone_locks_the_nearest_enemy_in_the_seats_sight_box` - two enemies
  in the box, the nearer locked; one nearer but outside the box not.
- `a_tie_goes_to_the_lower_slot`.
- `with_no_enemy_in_the_box_it_dives_on_the_aim_point`.
- `the_drone_flies_over_walls_and_dives_on_its_target` - a brick wall
  between: the enemy hurt by `fpv_damage`, the wall whole.
- `the_dive_commits_and_a_tank_that_moves_slips_most_of_it` - the target
  teleported off the point at the commit: the burst on the point, the
  damage the falloff's.
- `only_the_opposing_side_is_hurt_and_everyone_is_shoved` - two seats and an
  enemy at the burst: the teammate shoved and whole.
- `a_shield_soaks_the_burst`.
- `a_tank_under_a_tree_is_never_locked`.
- `a_drone_loses_its_lock_when_its_target_goes_under_a_tree` - and dives
  where it was.
- `a_dive_into_a_crown_hurts_only_the_tree` - the tree takes
  `fpv_tree_damage`, the tank beside it nothing; three fell a broadleaf.
- `a_burst_beside_a_tree_spares_the_hull_under_it`.
- `grass_does_not_hide_a_tank_from_a_seats_drone`.
- `a_teleporting_target_is_lost`.
- `a_minigun_bullet_brings_an_opposing_drone_down` - `DroneDowned { by:
  Bullet }`, the bullet stopped, the drone falls and crashes as a dud,
  nobody hurt.
- `a_bullet_passes_its_own_sides_drone`.
- `shells_and_plasma_pass_under_a_drone` (and a laser beam).
- `the_strike_box_runs_from_the_shadow_to_the_body` - a bullet across the
  shadow, one across the body, one across the line between: each strikes;
  one a block beside the column does not.
- `a_tesla_arcs_a_drone_in_reach_without_charging` - and its charge on a
  tank goes on; one arc a `tesla_air_gap_seconds`.
- `a_gun_tower_turns_on_a_drone_before_a_tank_and_brings_it_down`.
- `an_enemy_tower_engages_a_seats_drone_only_from_inside_the_seats_box`.
- `an_offline_tower_shoots_no_drone`.
- `the_drones_launcher_wrecked_mid_flight_still_scores`.
- `battery_out_dives_where_it_is_going`.
- `a_gust_carries_a_drone_off_its_point` (sandstorm).
- `a_wrecked_tank_launches_nothing` and `a_disabled_tank_launches_nothing_and_keeps_its_drones`.
- `the_drones_fly_out_on_the_end_screen_and_hurt_nobody`.
- `a_round_with_drones_replays_bit_for_bit`.
- `the_swarm_draws_no_rng` - the RNG state after a launch, a flight and a
  burst on open ground with no fence, drum or portal is the state before.
- `the_spawn_swap_hands_out_the_swarm_by_its_share_and_draws_nothing`.
- `an_enemy_takes_the_crate_only_with_no_special`.

AI (`ai.rs` unit tests on a `Brain` with a made-up `FpvSense` or
`AirThreat`, and `mechanics_tests` on a whole round):

- `the_swarm_launches_at_a_seat_in_its_box_without_line_of_sight` -
  behind a wall: `Launch` with the seat, `"cover"`.
- `the_swarm_goes_to_cover_when_exposed_and_launches_from_there`.
- `the_swarm_backs_off_a_seat_too_close`.
- `the_swarm_launches_one_at_a_time_with_its_gap` - no launch while one of
  its drones flies, none within `fpv_enemy_gap_seconds`.
- `the_swarm_holds_to_watch_its_drone`.
- `the_swarm_breaks_the_crown_over_a_hidden_seat`.
- `a_hunter_sends_its_drones_at_the_frog`.
- `the_swarm_never_launches_at_a_seat_hidden_in_grass`.
- `a_training_dummy_never_launches`.
- `the_generic_tiers_never_launch_a_drone`.
- `an_enemy_never_launches_at_a_seat_from_outside_its_sight_box` (whole
  round: `offbox-fire`'s two readings, `shot_at_seat` and the lock).
- `an_enemy_with_a_minigun_shoots_down_the_drone_diving_at_it` (whole
  round: the seat's drone downed by the enemy's bullets).
- `an_enemy_breaks_toward_the_nearest_tree` - and is under canopy before
  the dive, and takes nothing.
- `an_enemy_with_no_tree_breaks_across_the_drones_line`.
- `an_enemy_in_a_tell_or_a_charge_does_not_break_for_a_drone`.
- `an_enemy_flaks_a_seats_drone_only_from_inside_the_seats_box`.
- `fpv::tests`: `cover_spot_is_hidden_from_the_seat_and_nearest`,
  `tree_spot_puts_the_hull_under_canopy`, `pick_lock_ties_on_slot`.

Shared path and presentation:

- `air::tests`: `the_strike_box_is_the_column`, `drawn_is_lifted_by_the_height`.
- `fpv::tests`: `halo_slots_are_fixed_round_the_hull`,
  `the_launch_path_is_the_same_whatever_the_lock`,
  `a_drone_climbs_cruises_dives_and_arrives`,
  `the_turning_circle_is_inside_the_commit` (no orbit at the defaults),
  `a_downed_drone_falls_and_lands`; the composers
  `a_drone_is_on_the_grid_and_pure`, `the_lamp_blinks_at_its_rate`,
  `the_rotors_turn`, `a_falling_drone_alternates_its_silhouettes`,
  `the_halo_settles_when_disabled_and_falls_on_a_wreck`.
- `hits`: `the_sweep_meets_an_opposing_air_target_only_when_asked`,
  `an_air_target_is_rewound_for_a_seats_shot`,
  `the_rank_renumbering_keeps_every_tie`.
- `tank` (`weapon_inventory_tests`): the swarm in `take_weapon`,
  `full_load`, `special`; its trigger; the module's cells.
- `tower`: `a_tesla_arcs_drones_on_its_own_clock`.
- `weather`: `a_drone_throws_its_lamps_light_when_it_is_on`.
- `hud_tests`: the swarm's slot, colour and glyph.
- `indicators`: `an_incoming_drone_off_screen_gets_an_arrow_the_cap_never_drops`,
  `a_drone_at_the_frog_gets_one_too`, `own_drones_get_none`.
- `fx` tests: `a_low_drone_throws_wash`, `a_crash_is_a_dud`.
- `fish::tests`: `a_drone_burst_scares_the_fish`.
- `pickup`: `name`/`parse`; `weapon`.
- `devserver`: `set_tank_arms_the_swarm`, the snapshot's `drones`,
  `spawn_pickup` with `fpv_swarm`, the PICKUP category's count (16 to 17).
- `editor`/`chrome_tests`: the tool in `TOOLS`.
- `thumbnail`: the armory's pin. `maplint`: the armory as it lints.
- `text_tests`: every budget.

Wire:

- `events.rs`: the samples gain the four drone events and
  `DroneLockLost` (not sent); the variant count.
- `delta.rs`: `drones_round_trip_through_the_delta`, the random snapshots
  and the size bounds.
- `apply.rs`: `drones_reach_the_replica` (a seat armed through
  `debug_set_tank`, pressing: every drone the room flies is on the replica
  between its snapshots, the same picture after every apply),
  `a_downed_drone_falls_on_the_replica`, `a_drone_burst_puts_on_the_same_show`,
  `a_drone_launch_this_client_drew_is_not_drawn_again` (`OwnShotsDrawn`
  with the press's bit: no wash ring, not handed on; without: drawn).
- `interp.rs`: `drones_are_interpolated_and_carried_on`.
- `predict.rs`: `a_drone_press_is_drawn_at_once_and_claims_the_rooms_launch_once`,
  `a_drone_press_waits_for_the_reload`, `a_refused_drone_settles_back`.
- `round.rs`: `the_eased_climb_meets_the_rooms_drone_at_the_handover` -
  the provisional's drawn point and the room's copy's within a pixel at
  `h + Δ` on a link of 80 ms, and no frame drawing both;
  `the_shown_halo_loses_the_drone_on_the_press_frame`.
- `rig.rs` (`Lockstep`): `a_seats_drone_is_drawn_once_on_the_replica`,
  `an_enemys_drone_reaches_the_replica_and_bursts_on_the_seat` (an enemy
  armed through `authority_mut`, a seat in its box: the replica shows the
  drone locked on the seat and the burst), `a_drone_shot_down_falls_on_the_replica`.
- `server/tests/round.rs`: `a_drone_launched_through_the_mailbox_is_drawn_once`
  - through whole `OnlineRound`s over `NativeTransport`, a seat presses:
  one `Fired`, one drone in the room, the replica's own launch drawn once.
- The room server's `cargo test -p bongbong-server` as it stands.

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
