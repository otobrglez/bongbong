# Sonic hammer

BB-37, the first of the six weapons of BB-36. A special weapon from its own
crate (`pickup = "sonic_hammer"`), one at a time like every other
(`Tank::take_weapon`): a crate loads `sonic_ammo_per_pickup` (6) blasts, a
re-pick refills to six, another weapon's crate replaces it. Seats and
enemies alike: an enemy takes the crate while it fires shells
(`Tank::wants_pickup`) and uses it by its own rule (§4).

A cone of sound from the turret. Very little damage and a very big shove:
it knocks tanks off their tracks, shatters glass, flattens tall grass so
whoever hid in it shows, throws drums at whoever stands behind them,
scatters the fish and stuns frogs. Walls stop it. Counter: a wall between,
distance, a heavy chassis.

This doc is also where the **shared path** the five later weapons build on
is specified (§3): the AI hook for specials, the enemy tell, the online
press show, the probe's `--crate` and the spawn swap, the `spawn_pickup`
tool and the `armory` map.

## 1. How it plays

### The blast

- **One per press.** The trigger fires on the press edge, like a shell
  (`drive_player`'s `fire_pressed`); `sonic_reload_seconds` (1.2) between
  blasts (`Tank::fire_cooldown`). A seat's press goes off at once - no
  wind-up. An enemy's goes off after its tell (§3.2).
- **The cone.** From the hull's centre (`Tank::position`, the turret's
  pivot), along the hull's cardinal facing (`Dir::from_rotation`), out to
  `sonic_reach_px` (160, five cells), `sonic_half_angle_deg` (35) either
  side of the facing. No misfire skew: the hammer is never fired off-aim.
  It is judged from the pivot so a tank pressed against a pane still
  shatters it; it is drawn from there too, the arcs leaving the dish on the
  roof (the dish is a few pixels ahead of the pivot, inside the first arc).
- **What shadows it: a ray cast on the map-cell grid.** At the press the
  cone is cast once as a fan of rays (`sonic::SonicCone::cast`), one every
  `SONIC_RAY_ARC_PX` (4) px of arc at full reach (50 rays at the
  defaults), each walked cell by cell (Amanatides-Woo, the
  `weather::Occluders` walk) over the map's cells (`map::world_to_cell`: centres on multiples of
  32). A ray stops on entering the first cell holding a tile that
  `Material::blocks_sound`: every wall material - brick, iron, wood and
  glass - the three towers, a volcano's cone and a training door; its reach
  is the distance to that entry. It also stops at the field's edge. Props,
  drums, trees, lamp posts, pickups, water, lava, grass and tanks do not
  stop it. A destroyed tile is not there. The cast records, per ray, its
  reach, and, per cell any ray entered, the least distance at which a ray
  entered it. "In the cone" from then on means: a point within the half
  angle whose distance is no more than the reach of the rays either side
  of its bearing (the lesser of the two), or, for a cell, a cell some ray
  entered. Nothing of the cast is re-done as the world changes under the
  wave: a wall that falls mid-wave lets nothing more through, a tank that
  drives behind a wall mid-wave is shadowed by the rays already cast.
- **The wave travels.** What the cone reaches, the wave reaches when its
  front gets there: the front leaves the pivot at the press and runs out at
  `sonic_wave_speed` (640 px/s - the full reach in a quarter second). Each
  frame (`Game::tick_sonic_waves`) everything whose distance falls between
  the front's last radius and its new one is struck, once. A wave is done
  with when its front is past the longest ray; its picture lingers
  `sonic_wave_seconds` (0.6) longer.
- **Falloff.** Everything a wave does scales with `falloff(d) = 1 - (1 -
  sonic_edge_falloff) * d / reach` - 1 at the pivot, `sonic_edge_falloff`
  (0.35) at the rim.

### What it does to what it reaches

- **Tanks (live, with a body, not the shooter).** A hull is reached when
  the front passes the nearest of five points of its hull box (the centre
  and the four corners, `Tank::hull_bbox_world`) that is in the cone - so a
  big tank half in the cone is reached by its nearer edge. Then:
  - *the shove*: along the line from the pivot to the hull's centre, at
    `v = min(sonic_shove_max_speed, sonic_shove_speed * falloff /
    m^sonic_mass_exponent)`, where `m` is the chassis's mass factor
    (`Tank::mass() / scale^2`, `tank_mass_factor`). An impulse, so it adds
    to the hull's own motion: a tank driving at the shooter is shoved less
    far than one standing. At the defaults (380, 420, 1.5) a standard
    chassis at point blank slides about 3.5 cells, a scout about 4 (capped),
    a titan half a cell - "heavy chassis barely slide".
  - *the skid* (`Tank::skid`, §3.6): the hull is knocked off its tracks for
    `min(v / skid_friction, sonic_skid_max_seconds)` seconds. While it
    skids its own drive does nothing - the stick still turns it and still
    fires - and its motion, relative to the ground's flow, falls by a
    constant `skid_friction = sonic_skid_decel * max(grip,
    sonic_skid_grip_floor)` (`grip` the footing's: wet ground, a ford and
    ice slide further, never more than `1 / sonic_skid_grip_floor` times
    as far). It ends when the timer runs out or the hull's speed falls
    under `tank_decel_snap_px` (a wall stops it, and the tank drives
    again at once). So the slide is `v^2 / (2 * skid_friction)` whichever
    way the hull faces - the plain drive model stops a hull shoved along
    its axis in a few pixels and lets one shoved across it slide several
    times as far, which would make a head-on shove, the common one, a nudge
    (decisions, below).
  - *damage*: only to the side opposing the shooter
    (`Owner::same_side`), `sonic_damage` (4) times the falloff, no roll,
    through `Tank::take_damage` (a rainbow shield soaks it), `mark_hit`,
    `credit` (the kill credit and `hit_by_seat`), an `Event::Hit` where it
    landed, and `Ai::notify_hit` on a surviving enemy - it knows it was
    hit. A teammate, a fellow enemy and the shooter's own side take none:
    they are only shoved.
  - A wreck is not moved (as no blast moves one), a tank rolling in through
    a gate is not on the field, and the shooter is never reached.
- **Glass and lamp posts** (`Material::breaks_by_sound`): a glass cell some
  ray entered, and a lamp post on a cell some ray entered, die when the
  front reaches the cell's entry distance (`damage_obstacle` with the
  tile's whole health, `DamageCause::Shot` along the ray - the ordinary
  death path: `ObstacleDestroyed`, the glass's rubble row, the edge masks).
  Glass stops the rays that meet it as it shatters - the wave does not
  pass through any wall - so what stood behind a pane is spared until the
  next shout. A lamp post is a pole and stops nothing.
- **Lanterns** set down by a seat (`Game::lanterns`) in the cone are broken
  (`Event::LanternBroken`), as a blast breaks them.
- **Drums** (`Material::is_explosive`) on a cell some ray entered are
  **thrown** when the front reaches them - whether or not a fuse is already
  burning on them, which is how it "sets fuses off early": a thrown drum
  goes up where it lands. The throw runs along the line from the pivot
  through the drum, `max(1, round(sonic_drum_throw_cells * falloff))` cells
  far. If a live tank (any side but the shooter's own hull) stands beyond
  the drum within `sonic_drum_aim_deg` (25) of that line and within the
  throw's reach plus one cell, the drum is thrown onto the nearest such
  tank's cell instead (ties on slot) - "pushed toward whoever stands behind
  it". A landing cell a solid tile holds is walked back toward the drum a
  cell at a time; the field's border cells are never landed on
  (`sonic::drum_landing`, pure, shared with the AI). The drum goes through
  the existing launch: the tile dies with no blast (`ObstacleDestroyed`), a
  `FlyingDrum` carries it on its arc (`debris_flight_seconds`) and it goes
  off where it lands as a chained blast of its own kind (`tick_launches`) -
  an oil drum's pool and all. No `fuel_launch_chance` roll: the throw is
  the hammer's, not a fuse's.
- **Tall grass**: a grass cell some ray entered is flattened when the front
  reaches it. For `sonic_grass_flat_seconds` (6) it hides nobody
  (`Game::grass_flat`, which `Game::cover_cells` takes out of the cells
  `Terrain::build` and the indicators read), and its tufts lie flat for as
  long (`GrassTuft::pinned`), then stand back up over the ordinary
  `grass_crush_recover_seconds`. A seat or an enemy in it is in plain
  sight - the AI's concealment rule and `Terrain::conceals` both answer
  false - until the cell hides again.
- **Frogs** (either side's) whose centre is in the cone are **stunned** for
  `sonic_frog_stun_seconds` (1.5): `Frog::stun_timer`; while it runs the
  frog neither hops (`can_hop`) nor bites (`can_attack`) - no evasive hop
  away from a shell, no bite on a tank beside it, no training walk. It takes
  no damage. Your own frog is stunned too: a stunned frog cannot hop away
  from the enemy's shells.
- **Grenades** (on the ground or in the air) whose centre is in the cone are
  pushed along the line from the pivot at `sonic_grenade_push_speed` times
  the falloff (their velocity, the ball's own physics after).
- **Fish** in a lake the wave crosses dart away from the front
  (`fish::scares`, read from the waves on the field), and the "at 11": a
  fish the front passes within `sonic_fish_throw_px` (40) of dry ground,
  along the line from the pivot, is thrown onto the bank and flops there for
  `sonic_fish_flop_seconds` (2.5) before hopping back into the nearest deep
  cell - at most `sonic_fish_throw_max` (3) a wave, nearest the shore
  first. Presentation only (`fish.rs`), hashed, nothing on the wire: a
  replica throws the same fish off the same wave.
- **Trees** stop nothing; their crowns lean away from the pivot for a moment
  as the front passes (cosmetic, `sonic::tree_push`, added to
  `obstacle::tree_lean` at draw time).
- **Rubble** lying in the cone (landed decals) is picked up and thrown again
  a little along the wave, as a blast rethrows it (cosmetic, hashed).
- **What it leaves alone**: shots in flight (shells, bullets, plasma,
  missiles, flying drums, lava bombs, globs - a shell outruns sound), wrecks,
  crates (no damage, they do not break), towers (they stop it and take
  nothing), sandbags and fences (it passes over them), oil, ground fires,
  ooze, scorches, tread marks.
- **Portals** do not carry the wave (it is not a shot - `portal_shots`
  covers shells, bullets, plasma and the laser). A tank shoved into a
  portal's trigger radius goes through it as any tank does
  (`portal_phase`), and the teleport ends its skid (`place_tank` zeroes the
  velocity; it zeroes `skid` too).
- **Water and lava**: the wave crosses both. A shove ends at a deep lake's
  or deep lava's shore (both are colliders) and carries a hull into a ford
  as far as the skid goes; a lava ford burns it the whole way
  (`lava_damage_per_second`), a bank's heat too.
- **The shooter**: a small kick back along the facing (`apply_recoil`,
  `sonic_recoil_speed`/`sonic_recoil_max_speed`), not on `Frame::shoves` -
  a client that owns its hull kicks it itself on the press (§3.3). No
  muzzle flash (that is fire): the dish's firing cell (§5), the wave and a
  weak screen ripple (`sonic_shock`).
- **Events**: `Event::Fired { weapon: "sonic_hammer" }` then
  `Event::SonicBlast { slot, x, y, dir }` (the pivot and the facing) on the
  press - in that order, in the same tick, which the online claim reads
  (§3.3) - and the ordinary events of what it does (`Hit`,
  `ObstacleDestroyed`, `DrumLaunched`, `LanternBroken`, `Shoved`).

### On the end screen

The waves already out finish their picture (`tick_sonic_waves(f, false)`):
nothing is shoved, hurt, shattered, thrown or stunned, as a blast on the end
screen hurts nobody.

## 2. Where it lives

| File | What |
|---|---|
| `src/sonic.rs` (new) | The weapon's headless half. `SonicCone` (`cast` over a `blocks: Fn((i32, i32)) -> bool` and the field size, `reaches(p) -> Option<f32>`, `cells() -> &[((i32, i32), f32)]` sorted by entry distance then cell), `SonicWave` (origin, facing, owner slot, cone, age, `swept` radius, `struck` hulls), `falloff`, `shove_speed(mass_factor, d)`, `slide(v, grip)` (the skid's slide, what the AI predicts with), `drum_landing`, `tree_push`, the composers `compose_wave` and `compose_tell` (pure, `pyro::Shape`s), `module_cell` |
| `src/simulation/sonic.rs` (new) | The world half. `fire_sonic` (called from the dispatch arm), `resolve_sonic` (casts the frame's presses into waves), `tick_sonic_waves(f, live)` (the strike walk), `Game::knock` (the shove and skid any later weapon may use), `throw_drum`, `sonic_show` (the cosmetic half - the wave on the list, the ripple, the dish's flash - which a replica's `SonicBlast` and a client's press call too), `hammer_field` / `hammer_sense` (what the AI is handed, §4), `seat_sonic` |
| `src/simulation/weapons.rs` | The `ActiveWeapon::SonicHammer` dispatch arm |
| `src/simulation/mod.rs` | `Frame::pending_sonic`; `Game::{sonic_waves, grass_flat, seat_knock}`, `cover_cells`; the phase calls (`resolve_sonic` and `tick_sonic_waves` after `resolve_flames`, before `step_world`, so the shoves land in this tick's solver step; `tick_sonic_waves(f, false)` on the end screen); the tell in `enemy_phase` (§3.2); the skid in `drive_tank_with`, `tick_timers` and `predict_seat`; `accept_seat_pose`'s knock allowance; the spawn swap in `roll_enemy_tank` (§3.4); the `pickup_phase` arm; `tick_presentation` (waves, tells, skids, frog stuns, flattened grass); `Event::{SonicBlast, TellStarted}`, `Event::Shoved::skid`, `Event::DrumLaunched::drum` |
| `src/tank.rs` | `sonic_ammo`, `ActiveWeapon::SonicHammer` (`name`, `full_load`, `tell_seconds`), `SPECIAL_WEAPONS`, `weapon_ammo`/`take_weapon`/`empty_stock`/`wants_pickup`, `Tell`/`tell`, `skid`/`skid_speed`, `sonic_flash`/`kick_sonic`, the module's cell in `module_cols` |
| `src/pickup.rs` | `PickupKind::SonicHammer` (`sonic_hammer`, row 13, its ink, spills rather than cooks off), `PickupKind::{name, parse, weapon}` |
| `src/obstacle.rs` | `Material::blocks_sound`, `Material::breaks_by_sound` |
| `src/ai.rs` | The special hook (§3.1): `SpecialSense`, `HammerSense`/`HammerAim`, `SpecialUse`, `special_rule` / `hammer_rule`, `act_special`, `generic_fire`, the `special` tier, the `seek_special` tier, the stuck and breach clocks paused under a skid, `AiSnapshot::special` |
| `src/frog.rs` | `stun_timer`, `stun`, `is_stunned`, the gates in `can_hop`/`can_attack`, `stun_marks` |
| `src/grass.rs` | `GrassTuft::pinned`, `pin` |
| `src/fish.rs` | The waves among the scares; `Flop` (the fish on the bank) |
| `src/pyro.rs` | `Shape::Arc`, `block_arc` (shared, §5) |
| `src/indicators.rs` | `TankView::tell`, `ArrowKind::Tell`, the hit arc from a sonic blast |
| `src/hud.rs` | `HUD_SONIC_COLOR`, the `weapon_color`/`weapon_pickup` arms |
| `src/game.rs`, `src/render/game.rs` | Trees leaning, frog stun marks, the waves, dust and tells in their passes (§5), the dev stats arm |
| `src/fx.rs` | Dust off a skidding hull |
| `src/net/wire.rs` | `WeaponKind::SonicHammer`, `WeaponKind::drawn_on_press`, `TankState::{tell, skid}`, `frog_flags::STUNNED` |
| `src/net/events.rs` | `WireEvent::SonicBlast`, `Shoved::skid`, `DrumLaunched::drum`, `WireEvent::press_show`, `tell_started` on `NOT_SENT` |
| `src/net/encode.rs`, `src/net/apply.rs` | The new fields; `Show::OwnShotsDrawn { seat, presses }`, `presses_drawn`; `SonicBlast`'s spectacle |
| `src/net/predict.rs`, `src/net/round.rs` | The press show (§3.3), `Predictor::shove(dv, skid)` |
| `src/simulation/present.rs` | `Game::{draw_press_show, seat_kick}` |
| `src/simulation/replica.rs` | `DrawableTank::{tell, skid}`, `DrawableFrog::stunned` |
| `src/simulation/debug.rs`, `src/devserver.rs` | `spawn_pickup`, `set_tank`'s `sonic_ammo`, the snapshot's `sonic`, `tell`, `skid`, `stun` and `sonic_waves` |
| `src/bin/probe.rs` | `--crate`, the tank line's `sonic=`, the holds |
| `src/editor/mod.rs` | `Tool::Pickup(PickupKind::SonicHammer)` (`sonic_hammer`) |
| `src/map.rs`, `maps/armory.toml` | The armory (§3.5) in `SHIPPED_MAPS` |
| `src/tuning.rs` | The `sonic` group (§6), `enemy_special_weapon_sonic_share` |
| `lang/en.ftl`, `lang/sl.ftl` | §7 |
| `tools/punypalette.py`, `tools/spritegen/gen_crates.py`, `tools/spritegen/tankdesign/{kit,export,render,lines/vanguard}.py` | The art (§5); writes `static/crates_sheet.png`, `pickup_glyphs.png`, `tank_modules.png`, `tank_modules_glow.png`, `src/tank_art.rs` |
| `src/maplint.rs`, `src/thumbnail.rs` | The armory among `SUPPORTED_MAPS`, its pinned thumbnail |
| `justfile` | `probe-fixtures`/`probe-fields` pass extra arguments through (§11) |
| `docs/` | This, `CRATES_SPEC.md`, `SPRITESHEET_SPEC.md`, `effects.md`; `CLAUDE.md` |

## 3. The shared path

What weapon 1 lays down for weapons 2-6. Each later weapon adds its own arm
to each of these; none of them needs a second mechanism.

### 3.0 A special weapon's checklist

The grenade launcher's PR (`0a142ff`) touched these for one kind; every
BB-36 weapon does the same, in the same places:

1. `PickupKind::X` (explicit `#[serde(rename)]`, appended to `ALL`, its
   `row`, `ink`, `cooks_off`), `PickupKind::weapon` (`Some(ActiveWeapon::X)`)
   and `name` (the serde spelling).
2. `ActiveWeapon::X` (`name`, `full_load`, `tell_seconds`), appended to
   `SPECIAL_WEAPONS`; the ammo field on `Tank` with its arms in
   `weapon_ammo`, `take_weapon`, `empty_stock`, `wants_pickup`
   (`active_weapon() == Shell` for an enemy, the one rule), `module_cols`.
3. The dispatch arm in `weapons::dispatch_fire_from`; the fire edge or hold
   in `drive_player`'s `should_fire`.
4. `pickup_phase`: `PickupKind::weapon` gives `take_weapon` its weapon, so a
   new kind needs no arm of its own there.
5. `ai::SEEK_SPECIALS` (§3.1) and, if the AI uses it by a rule,
   `SpecialSense::X`, `special_rule`'s arm and `generic_fire`.
6. `hud::weapon_color` (`HUD_X_COLOR`, the crate ink's base) and
   `hud::weapon_pickup`.
7. `gen_crates.py` (`KINDS`, `GLYPHS`), `punypalette.PICKUP_INK`; the
   tankdesign module (`kit.WEAPONS`/`WEAPON_STATES`, the `module_fn`, the
   anchor `export.py` writes into `tank_art.rs`, `render.SHOWN_TOGETHER`).
8. `editor::TOOLS` and `Tool::name`; `tool-X`/`tool-short-X` in both
   catalogues.
9. `WeaponKind::X` (appended to `ALL`, both `From`s, `drawn_on_press`),
   `Predictor::seed_gate`'s arm, `apply::write_tank`'s ammo arm,
   `kick_turret`/`drawn_muzzle`.
10. `debug::{TankDebug, TankPatch}`, `set_tank`'s schema, `TankSnapshot`,
    the probe's tank line and fire tuple, `render::game::draw_tank_stats`.
11. The armory's crate; the spawn swap's table; the tuning group;
    `PROTOCOL_VERSION`.

### 3.1 The AI hook for special weapons

An enemy's tree already fires whatever it carries through the generic tiers
(`hold_and_fire` in attack, snipe and grudge; `act_breach`). A BB-36 weapon
is used by a rule of its own instead. The hook, in `ai.rs`:

```rust
/// What `enemy_phase` measured for the special a tank carries, for that
/// weapon's rule: one variant per special whose use needs more than the
/// tree's own perception. `None` for every other tank.
pub enum SpecialSense { None, Hammer(HammerSense) /* , Emp(..), ... */ }

/// What a special's rule asks of the tank this tick.
pub(crate) enum SpecialUse {
    /// Face `face` and pull the trigger (the simulation runs the weapon's
    /// tell first, §3.2). `at_seat` is the seat it is used on - what
    /// `Ai::shot_at_seat` records - and `why` the arm, for the trace.
    Fire { face: Dir, at_seat: Option<u8>, why: &'static str },
    /// Hold still facing `face`.
    Hold { face: Dir },
    /// Close in on `to`, to bring a short-range weapon to bear.
    Approach { to: Position },
}

/// The rule of the special the tank carries, `None` for no use this
/// tick. First, for every weapon: a tell in progress holds the tank
/// facing the way it goes off. Then one arm per weapon.
fn special_rule(b: &Brain) -> Option<SpecialUse>;

/// Whether the tree's generic tiers - attack, snipe, grudge, breach - may
/// pull the trigger on `weapon`: false for a special whose rule owns it.
fn generic_fire(weapon: ActiveWeapon) -> bool;
```

- **Where the tree consults it**: a `special` tier at 1.5, after the wreck
  check and before flee (`build`): `condition(special_rule(b).is_some())`,
  `action("special", act_special)`. `act_special` applies the use: `Fire`
  faces, commits the heading (`Ai::commit`), resets the aim settle and,
  while `Ai::fire_timer` allows, sets `intent.fire`, sets
  `Ai::fire_timer` to the weapon's own interval (`special_fire_interval`,
  the hammer's `sonic_ai_fire_interval`) and records `at_seat` in
  `Ai::shot_at_seat` and `why` in `Ai::special_why`
  (`AiSnapshot::special`); with the timer running it holds instead.
  `Hold` faces; `Approach` steers (`Brain::steer`). Above flee because a
  hurt tank shoving away the seat in its face is the use; a rule offers
  `Approach` only to a tank under `enemy_flee_damage`, and never to a guard
  whose `guard_holds`.
- **How a weapon adds its arm**: a `SpecialSense` variant, filled in
  `enemy_phase` (below), one match arm in `special_rule`, its own
  `fn x_rule(b, &sense) -> Option<SpecialUse>`, and its `generic_fire`
  answer. Nothing else in the tree changes.
- **The generic tiers defer**: `hold_and_fire` and `act_breach` set no
  `intent.fire` while `!generic_fire(active_weapon)` (they still face and
  settle), and `Brain::wants_breach` never latches a breach with such a
  weapon - a tank carrying the hammer does not stand shouting at brick; its
  own rule breaks glass (§4).
- **What the rule is handed**: `Ai::think` takes one more argument,
  `sense: &SpecialSense`, kept on the `Brain`. `enemy_phase` builds it in
  the collect pass for a tank that thinks this tick and carries a weapon
  with a sense, from a per-frame snapshot of the world built before the
  pass - only when some enemy carries one, so a round without costs
  nothing and a `Brain` never reaches into the world (the AI reasons over
  snapshots). The hammer's is `hammer_field` + `hammer_sense` (§4).
- **The sight box binds by construction**: a sense names a seat only if
  this tank stands inside that seat's sight box
  (`ai::in_sight_box_of`, measured in the simulation against the seat's
  real position) and the tank is not a training dummy (`Ai::frog_only`),
  so no arm can use a weapon on a seat from off the box; the probe's
  `offbox-fire` reads the same `shot_at_seat` it always has.
- **No RNG**: a rule chooses by fixed priority, then the facing the tank
  already has, then `Dir::ALL` order, then owner slot. `hold_and_fire`'s
  friendly-fire hold roll and the misfire roll are not on this path.
- **Seeking**: one `seek_special` tier after `seek_minigun`, walking
  `ai::SEEK_SPECIALS: &[PickupKind]` (the BB-36 kinds the AI collects, in
  the order it wants them - `[PickupKind::SonicHammer]` here) for the first
  kind with `Tank::wants_pickup` and `Brain::seek`. The existing four tiers
  keep their names and order, so the trace and every replay are untouched.

### 3.2 The enemy tell

Every enemy use of a BB-36 weapon has a wind-up a player can read before it
lands. On `Tank`:

```rust
/// The wind-up an enemy shows before a special with one goes off
/// (`ActiveWeapon::tell_seconds`): which weapon, the seconds left, its
/// whole length (the drawing's progress) and the facing it goes off along.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tell { pub weapon: ActiveWeapon, pub left: f32, pub total: f32, pub facing: Dir }

pub tell: Option<Tell>,
```

- **Who sets it**: the simulation, on the AI's word. In `enemy_phase`'s
  collect pass, where `dispatch_fire` is called today: a tank with no tell
  whose intent fires, cooldown out, starts one if its weapon has a
  `tell_seconds` (`Event::TellStarted { slot, weapon }`), else fires as
  now. A tank with a tell counts it down by the tick's `dt` and, at zero,
  clears it and fires (`dispatch_fire`) - if it is not a wreck, still
  carries `tell.weapon` with ammo, and its cooldown is out; otherwise the
  tell lapses. While it runs the tank's intent is overridden before
  `tank.control`: no movement, `face = tell.facing`, no new trigger - the
  tank holds its aim (and the rule's first arm holds it too). The far
  tanks of a field map that coast between thinks count their tell down in
  the coast branch the same way. A teleport cancels it (`portal_phase`);
  `init` has none. A tell commits: a seat that steps out of the cone during
  it is the dodge the tell is for.
- **A seat has no tell**: a seat's press fires on the press. A later
  weapon whose own mechanic is a charge (the Gauss rail) or a call-in (the
  rod) may give a seat a `Tell` of its own; that is its PR's to wire.
- **How it is drawn**: per weapon, `render/game.rs` asks the weapon's
  composer for every tank with a tell (`sonic::compose_tell` for the
  hammer, §5), in the glowing pass, so it reads at night; the module shows
  its wind-up cells (`module_cols`).
- **How it travels**: `TankState::tell` - the seconds left in tenths, 0 for
  none; the weapon is `TankState::weapon`, the facing `TankState::dir`. A
  replica writes `Tank::tell` from it (`apply::write_tank`, `total` the
  weapon's `tell_seconds`) and runs it down between snapshots
  (`tick_presentation`), as it runs a grenade's fuse. `TellStarted` is on
  `NOT_SENT`: the state is what draws.
- **Off screen**: `indicators::TankView::tell` (the progress, 0..1) from
  `Tank::tell`, so it works on a replica; an enemy with a tell, off the
  screen and not concealed, gets an `ArrowKind::Tell { weapon, progress }`
  arrow, put with the teammates, frogs and volcanoes - never merged, never
  left out past `indicator_max_arrows`. A tell also reveals its tank as
  firing does (`Awareness::fired`). The arrow is drawn in the weapon's HUD
  accent (`hud::weapon_color`), rimmed near-black, blinking at
  `indicator_pulse_hz * (1 + 2 * progress)`.

### 3.3 Online: the shooter's press is drawn at once

Decision 3 of BB-36: the shooter's client draws its press's show from its
predicted pose on the press frame, and the replica does not draw it again
when the room's event arrives. The room stays the authority on damage,
shoves and tile deaths. The laser does this today for its beam; it becomes
the general case:

- **Which weapons**: `WeaponKind::drawn_on_press(self) -> bool` - the
  laser and the hammer here; each later weapon whose press has a show says
  so. The show event each one claims: `WireEvent::press_show(&self) ->
  Option<(u8, WeaponKind)>` - a `LaserBeam` with `leg: 0` for the laser
  (legs past a portal stay the room's), a `SonicBlast` for the hammer - its
  seat and weapon.
- **Predictor side** (`net/predict.rs`): `beams_owed` becomes
  `owed_presses: VecDeque<(WeaponKind, u32)>` (ammo the presses not yet
  answered will spend, by kind), `drawn_beams` becomes `drawn_presses:
  VecDeque<(WeaponKind, u32, f32)>` (kind, input tick, seconds since), and
  `beams: Vec<BeamPress>` becomes `shows: Vec<PressShow>`:

  ```rust
  pub enum PressShow { Beam(BeamPress), Sonic(SonicPress) /* , ... */ }
  pub struct SonicPress { pub origin: Position, pub facing: Dir }
  ```

  `pull_trigger`'s hammer arm: on the press edge, with ammo less what is
  owed and the local gate open, it takes the sandbox's pose
  (`Game::seat_sonic(seat) -> (Position, Dir)`), sets the gate to
  `sonic_reload_seconds`, owes the press and, while presses are drawn,
  queues `PressShow::Sonic` and the drawn press. An owned hull is kicked
  on the press, drawn or not (`Game::seat_kick`, the general form of
  `seat_recoil`). `take_beams` becomes `take_press_shows`; `confirm_beam`
  becomes `confirm_press(kind, input_tick) -> bool` - the last drawn press
  of that kind at or before the tick is the room's, the earlier ones the
  room refused; none claimed seeds the local gate (`seed_gate`, which gains
  the hammer's reload). `note_fired` drops the owed presses of a
  drawn-on-press kind up to the tick.
- **Round side** (`net/round.rs`): `confirm_beams` becomes
  `confirm_presses(frame) -> u8`: bit `k` for this seat's `k`th `Fired` of
  a drawn-on-press kind in the snapshot, set when `confirm_press` claims
  it. `fly_own_shots` draws each `PressShow` against the drawn world: a
  beam as now, a sonic press through `Game::draw_press_show` - the same
  `sonic_show` a replica puts on for the room's `SonicBlast` (the wave
  cast against the replica's tiles, the dish's flash, the ripple).
- **Replica claim side** (`net/apply.rs`): `Show::OwnShotsDrawn { seat,
  beams }` becomes `{ seat, presses }`; `beams_drawn` becomes
  `presses_drawn(s) -> BTreeSet<usize>`: walking the events, a seat's
  drawn-on-press `Fired` whose bit is set claims the seat's next press-show
  event of that kind (the room logs the `Fired` first, in the same tick),
  one pending claim per kind. A claimed event is neither drawn nor handed on
  to `game.events` (so `fx` and the indicators do not see it twice). The
  `Fired`'s own muzzle ripple and turret kick are already skipped for the
  seat (`client_drew`); for the hammer that is the dish's flash.
- **What is keyed by the input tick**: only the claim. The room's `Fired`
  carries the input tick of the press it answered (`authority::
  stamp_presses`, unchanged); everything else of the show is the event's.
- **Refusal and timeouts**: a drawn press nobody claims within the refusal
  wait (`set_refusal_after`: the round trip plus the picture's delay plus
  `REFUSAL_MARGIN_SECONDS`) is dropped, owed and drawn alike - the client
  showed a shout the room never made, and the picture already faded; a
  room `Fired` the client never drew (its local gate refused) claims
  nothing, so the room's show is drawn and the gate is seeded from it.
- **What stays the room's**: the shoves (a client-owned hull hears of its
  own through `Shoved`, now with its skid, §3.6), the damage (`Hit`), tile
  deaths (`ObstacleDestroyed`), thrown drums (`DrumLaunched`, now naming
  the drum), frog stuns (`FrogState`), lanterns (`LanternBroken`). The
  client's own wave flattens its picture's grass and scatters its fish at
  once, which is presentation.

### 3.4 The probe's `--crate` and the spawn swap

- **`--crate <kind>`** (`src/bin/probe.rs`): before the round, every map
  pickup slot whose kind is a special weapon's (`PickupKind::weapon` is
  `Some`) becomes `<kind>` (`PickupKind::parse`, the serde spelling). The
  slots and their order stay, so `respawn_from_slots` draws exactly what it
  drew - the respawn picks a slot, not a kind, and only a Health slot rolls
  bonuses - and only what is taken differs. Echoed in the run's header and
  `--json-out`.
- **The spawn swap** (`roll_enemy_tank`): after the draws that choose an
  enemy's special (laser, plasma or minigun, unchanged), a tank that drew
  one may have it swapped for a BB-36 weapon:

  ```rust
  /// The BB-36 weapons' place in the enemy spawn roll, in order: each
  /// with the knob that is its share.
  const SPAWN_SWAPS: &[(ActiveWeapon, fn(&Tuning) -> f32)] =
      &[(ActiveWeapon::SonicHammer, |t| t.enemy_special_weapon_sonic_share)];
  ```

  For each entry with a share above 0, `pyro::unit(blast::seed_at(spawn,
  SWAP_SALT ^ slot), i)` - a hash of the spawn point, the owner slot and
  the entry's index, never a draw from the round's RNG - below the share
  swaps the special (`take_weapon`) and ends the walk. A tank that drew no
  special keeps none. Every share defaults to 0, so at the defaults no kit
  and no draw changes; the knob (`@ Restart`, the `enemies` group, beside
  `enemy_special_weapon_*_share`) is the play-tester's lever.

### 3.5 `spawn_pickup` and the armory

- **`spawn_pickup {kind, x, y}`** (dev server, one `ToolSpec` row and a
  `dispatch` arm; `Game::debug_spawn_pickup`): puts a crate of `kind` (the
  serde spelling) down at the map cell nearest (`x`, `y`), in its air drop
  (`Pickup::dropped(.., Some(time))`). Refused outside the field, on a solid
  tile and where a pickup already stands. Not a slot - it never respawns.
  No RNG. A `GAME_ONLY_TOOLS` and `ONLINE_REFUSED_TOOLS` member. Replies
  `{kind, x, y}`. `set_tank` takes `sonic_ammo` (and every later weapon's
  field) the way it takes `grenade_ammo`: above 0 arms it in place of the
  special carried.
- **`maps/armory.toml`**, appended to the free-play maps of `SHIPPED_MAPS`
  (after `longwater`), so the web preview's LOAD list and the lobby's map
  stepper reach it. An arena (36 x 18, shown whole), `mission.kind =
  "protect"`, a band of four, clear sky. Each later weapon adds its crate
  to the reserved column and whatever its interactions need; nothing here
  moves. Every level is untouched.

  ```
       0         1         2         3
       012345678901234567890123456789012345
   0   ................................L...
   1   .+..........ggggg...............L...
   2   ............g...g...II....b.....L...
   3   ...P........g.s.g...II....b.....L.E.
   4   .......r....g...g.........b.....L...
   5   ............ggggg...........zzz.L...
   6   .......r.............o..........L...
   7   ................................L...
   8   ....S..H.........*...f%%%...........
   9   ....................................
  10   .......r.............o........H.....
  11   ...........wwwwwT...................
  12   ..F....r...wwwww....................
  13   ..........Twwwww...WWWWWWWW.b.......
  14   .......r...wwwww...WWWWWWWW.b.....G.
  15   ...........wwwww...WWWWWWWW.........
  16   .a.................WWWWWWWW.........
  17   ...................WWWWWWWW.........
  ```

  | Mark | Cells | Why |
  |---|---|---|
  | `S` start | 4,8 | West, facing the field |
  | `H` sonic hammer | 7,8 and 30,10 | One three cells from the start; one on the enemy side, which an enemy on shells collects, so an armed enemy shows up with no tuning |
  | `r` reserved | 7,4 7,6 7,10 7,12 7,14 | The crate column weapons 2-6 fill, one each |
  | `+` `a` | 1,1 and 1,16 | Health and ammo, out of the way |
  | `g` glass house | the ring 12..16 x 1..5 | Shatters; it stops the wave as it does; the AI's glass breach |
  | `s` shield | 14,3 | Inside the glass, what an enemy seeks (tier 5.9) and has to break in for |
  | `P` player tesla | 3,3 | A player tower the wave stops at |
  | `F` the frog | 2,12 | The stun; the hunters (Protect) come for it, so the frog-pin arm shows |
  | `I` iron | 20..21 x 2..3 | The counter: cover the wave cannot pass |
  | `b` brick | 26,2..4 and 28,13..14 | Walls stop it too, not only iron |
  | `*` lamp post | 17,8 | Put out by the wave (try `--weather night`) |
  | `o` `f` drums | oil 21,6 and 21,10, fuel 21,8 | Thrown at whoever stands east of them |
  | `%` oil trail | 22..24,8 | A thrown drum's blast lights it |
  | `w` tall grass | 11..15 x 11..15 | Hide in it; a shout flattens it and shows you |
  | `T` trees | 16,11 and 10,13 | Crowns that sway as the wave passes; stop nothing |
  | `W` lake | 19..26 x 13..17 | Fish to scatter and throw onto its north bank (row 12) |
  | `L` lava ford | 32,0..7 | Trouble: what the AI shoves a seat into, and where a seat shoves enemies |
  | `E` enemy tesla | 34,3 | An enemy tower's reach beside the lava: more trouble |
  | `G` enemy gun tower | 34,14 | Trouble on the south-east |
  | `z` sandbags | 28..30,5 | The wave passes over them |

  It lints with no error (`maplint`, added to `SUPPORTED_MAPS`; the shield
  inside the glass is a `gated-pickup` warning), its CPU thumbnail is
  pinned beside the others', and the cells are checked again in Phase 2
  against the linter's band capacity and the fish's lake size.

### 3.6 The knock: a shove that takes a hull off its tracks

`knock(physics, f, tank, dir, speed)` (`simulation/sonic.rs`) is the shove
any later weapon that throws tanks about uses (the gravity well):
an impulse of `speed` along `dir` scaled by the mass as above, then
`Tank::skid` and `Tank::skid_speed` (the hull's speed after it, for the
pose validator). `drive_tank_with` reads `skid` (the skid model of §1);
`tick_timers` counts it down for every tank, and `predict_seat` for the
sandbox's seat. Online:

- `Event::Shoved { seat, vx, vy, skid }`: `Shoves::push` takes the skid
  (0 for every other shove), the wire carries it in tenths
  (`WireEvent::Shoved::skid: u8`), and `Predictor::shove(dv, skid)` puts
  the sandbox's seat into the same skid. An owned reconciliation keeps the
  seat's `skid` with its pose, as it keeps the pose.
- `TankState::skid` (tenths) carries every hull's skid, so a stage-2
  replay skids where the room does and a replica's `fx` puts dust off a
  skidding hull.
- The validator (`accept_seat_pose`) allows a shoved hull its speed: a
  shove logged for an owned seat sets `Game::seat_knock[seat]` to its speed
  and the frame `POSE_KNOCK_GRACE_TICKS` (60, server policy beside
  `POSE_REACH_SLACK_PX`) past the skid's end, and the reach uses
  `max(effective_speed, knock speed)` until then - the client's skid starts
  a link's delay after the room's.

## 4. AI

An enemy carrying the hammer uses it by `hammer_rule`, never through the
generic tiers (`generic_fire(SonicHammer)` is false). `hammer_field` is
built once per frame in `enemy_phase` when any enemy carries the hammer:
the cells that stop sound (from `f.terrain`'s boxes), the cover cells, the
drums, every live tank (slot, position, mass factor, side, weapon), the
frogs (position, side, stunned), the seats (position, concealed, the
alert), the trouble cells (below) and the footing's grip per cell.
`hammer_sense` then gives each thinking hammer tank a `HammerSense`:

```rust
pub struct HammerSense {
    /// One per facing, `Dir::index` order: what a blast that way would do.
    pub aims: [HammerAim; 4],
    /// This tank is the nearest hammer-armed enemy to the seat it fights
    /// (ties on slot): the one that may close in (`Approach`).
    pub brawler: bool,
}
pub struct HammerAim {
    pub trouble: Option<u8>,  // a seat the shove would land in trouble
    pub drum: Option<u8>,     // a seat a thrown drum would land on
    pub flush: Option<u8>,    // the hidden seat whose grass it would flatten
    pub breaker: Option<u8>,  // a seat within `sonic_ai_breaker_px`
    pub frog: bool,           // the hunter's quarry, not stunned
    pub friend: bool,         // a live fellow enemy in the cone
}
```

Each aim casts the cone (`SonicCone::cast`) from the tank as it stands,
facing that way. A seat counts in any field only if it is in that cone, not
a wreck, on the field, and this tank stands inside its sight box (§3.1); for
`trouble`, `drum` and `breaker` it must also not be hidden from this tank
(`concealed` and not hit-alerted - the attack tier's rule). `hammer_rule`,
in priority order:

1. **The tell holds** (`special_rule`'s first arm): `Hold { face:
   tell.facing }`.
2. **Breach glass at once.** Commanding movement into glass
   (`walls_ahead[last_move_dir]` is glass) for `sonic_ai_glass_after_seconds`
   (0.1) - instead of `enemy_breach_after_seconds` - fires that way
   (`why: "glass"`, no seat). Its own clock, `Ai::wall_ahead_timer`,
   already counts this.
3. Then for each facing - the tank's own first, then `Dir::ALL` - skipping
   any whose `friend` is set (it never shoves a fellow enemy):
   1. **Shove into trouble** (`"trouble"`): a seat in the cone whose
      predicted slide meets trouble. The prediction is the rule's own
      model, so it is exact on dry ground: the shove `v` at the seat's
      distance and chassis (`sonic::shove_speed`), the slide `v^2 / (2 *
      skid_friction)` at the seat's footing (`sonic::slide`), along the line
      from this tank through the seat, sampled every `SONIC_TROUBLE_STEP_PX`
      (8) and cut at the first cell that stops a hull (a tile, deep water,
      deep lava) or the field's edge. Trouble at a sample: heat at or over
      `heat_hurt_from` (`Game::heat_at` - lava and its banks); a burning
      ground cell (`Game::fires`); an ooze puddle (`Game::ooze`); inside a
      standing enemy tower's reach (`TowerKind::range`); or lined up for
      another live enemy (not this one): on its row or column within
      `enemy_fire_align_px`, inside `enemy_attack_range`, with the line of
      sight clear (`Terrain::line_of_sight`) and that enemy inside the
      landing point's sight box. The trouble cells (all but the lanes, which
      depend on the shooter) are gathered into `hammer_field` once.
   2. **Throw a drum** (`"drum"`): a drum in the cone whose
      `sonic::drum_landing` - the simulation's own function - lands on a
      seat's cell.
   3. **Flush the grass** (`"flush"`): the seat this tank fights is
      hidden from it (`target_concealed`, not hit-alerted) and its alert
      (the shared one on an arena, its own on a field map,
      `Brain::alert`) lies in the cone within `OBSTACLE_GRID_SIZE` of a
      cover cell the cone reaches. Aimed at the alert point - what the tank
      believes - never at where the seat really is; the sight box is
      checked against the seat's real position, like every other use.
   4. **Breaker** (`"breaker"`): a seat in the cone within
      `sonic_ai_breaker_px` (112).
   5. **Pin the frog** (`"frog"`): a hunter (`Brain::hunting_frog`) with
      its quarry in the cone, not stunned. A hammer cannot hurt a frog; a
      stunned one cannot hop away from the pack's shells. Bounded: six
      blasts, then shells.
4. **Hold for the timer**: a fire arm that matched while `Ai::fire_timer`
   runs holds facing it instead (`act_special`).
5. **Approach**: a brawler (`HammerSense::brawler`) under
   `enemy_flee_damage`, not a guard that holds, whose target - the seat it
   fights, or a hunter's quarry - is within `attack_range`, in its line of
   sight and not hidden from it, and is not yet a breaker (or frog) in the
   cone of `Dir::toward(target)`, closes in on it (`Approach { to: target
   }`); the personal-space brake (`crowded_ahead`) stops it short of the
   hull. One brawler per seat keeps a pack of hammer tanks from piling
   onto one seat; the rest keep their ring slots and use the hammer only
   when an arm above offers.
6. Otherwise `None`: the tree goes on (attack still lines up and settles
   but never fires the hammer; chase, patrol and the seek tiers as ever).

**The tell** (§3.2): every fire arm goes through it - `sonic_tell_seconds`
(0.55) of the dish pulsing and arcs gathering into it (§5) before the
blast. The arm's decision is the moment the sight box is checked and
`shot_at_seat` recorded.

**Pacing**: `sonic_ai_fire_interval` (2.5) between an enemy's decisions,
on top of the reload.

**A shoved enemy is not stuck.** While `me.skid > 0`, `Ai::think` neither
advances nor resets `stuck_timer`, `progress_avg` or `wall_ahead_timer`,
and still updates `last_position`, so the slide it did not ask for is not
evidence of anything; the stuck escape and the breach latch carry on from
where they stood once it ends. A tank in a tell commands no movement, which
resets the stuck clock as any deliberate hold does.

**Reacting to a player's hammer**: nothing beyond the physics and the
skid exemption above.

**Off the field and asleep**: a field map's far tank coasting on its last
intent has no sense; its tell, if it had one, still counts down and goes
off.

## 5. Drawing

All of it in the effects language (docs/effects.md): whole 2 px blocks,
ramp steps, Bayer fades; composed at draw time as pure functions of the
wave or the tank and its age, hashed, never rolled.

- **A new primitive, shared**: `pyro::Shape::Arc { center, radius, width,
  from, to, color, cover }` and its painter `pyro::block_arc` - the blocks
  whose centres lie within `width / 2` of the radius and between the two
  angles, dissolving through the Bayer pattern to half at `cover` 0.5 and in
  eighths below (`dither_disc`'s rule). The EMP's ring and the well's
  spirals draw with it too.
- **The wave** (`sonic::compose_wave`), in `render/game.rs`'s glowing pass
  (fast and bright, rule 7, and readable at night): `sonic_wave_rings` (3)
  arcs, the front at `sonic_wave_speed * age` and each one
  `sonic_ring_gap_px` (14) behind, none under 12 px; each arc split into
  the runs of rays whose reach is at least its radius, so it stops at a
  wall's face and shows the shadow; in the stone ramp, `#F0F0F0` leading
  (two blocks thick), `#DADADA`, then `#C1C1C1` (one block); `cover`
  falling with the radius (`1 - 0.6 * (r / reach)^2`) and over the last
  third of the wave's life. No fire colours, no light thrown.
- **Dust where it passes**, in the lit pass (lingering, shaded): every
  `sonic_dust_spacing_px` (10) along the front at hashed offsets, a
  `pyro::dust_puff` in the `DUST` ramp off dry ground, rising and leaning
  with the wind (`pyro::smoke_lean`), gone in `sonic_dust_seconds` (0.5);
  over water `BLUE_PALE` and white chop marks; over lava `SMOKE` puffs.
- **Shards**: a pane or a lamp the wave shatters throws eight marks,
  `BLUE_PALE` and `#F0F0F0` alternating, hashed within 40 degrees of the
  wave's line, 16-40 px over `sonic_dust_seconds`, falling as they go; the
  ordinary rubble and collapse burst come from the tile's death.
- **The ripple**: `Shockwave::scaled(pivot, sonic_shock)` (0.35 of a tank
  dying) through `shockwave.rs` - a weak bend and shake, none under reduced
  motion.
- **The tell** (`sonic::compose_tell`), glowing pass: three arcs within the
  cone's angles centred on the dish (`tank_art::SONIC_MUZZLE`), contracting
  from 40 px into the dish once every 0.2 s, `#C1C1C1` far to `#F0F0F0`
  near, dissolving in from `cover` 0.3 to 1 as the tell runs; the module
  pulsing its two wind-up cells, quicker as it nears.
- **The skid**: dust off a skidding hull's tracks (`fx.rs`, from
  `Tank::skid`), and the tread marks already follow the travel heading, so
  a sideways slide leaves sideways marks.
- **Trees** lean `sonic_tree_lean_px` (4) away from the pivot as the front
  passes and swing back over a quarter second (whole 2 px bands, as a ram's
  lean). **Frogs** stunned: the hurt clip held on its first frame and three
  `#F0F0F0` blocks, each with a `#C1C1C1` block under it, circling over the
  head (`frog::stun_marks`, the stone ramp). **Fish** on the bank: the fish's own blocks
  (`fish::compose`) lying on the grass, flipping heading every 0.15 s with
  a block's hop.
- **The module** (`tankdesign`, `lines/vanguard.py`, `module_fn('sonic')`):
  an LRAD dish on the roof - the missiles' hardpoint, shared, since a tank
  carries one special at a time (`hp.get('sonic', hp['missiles'])`): a
  round gunmetal mount (4 x 4, chamfered), a shallow dish across its front
  7 design px wide whose horns curve forward a pixel, a face of
  transducer dots (steel and dark, `'grille'`) and a steel rim. Four cells,
  `TANK_MODULE_SONIC_COL` = 24..27 (`tank_modules.png` grows from 24 to 28
  columns, 1120 x 480): 0 idle, the face dark; 1 and 2 the wind-up, the
  inner and then the outer ring of dots lit in the light layer (`'white'`
  role: white over `STONE_PALE`); 3 the blast, the whole face lit and the
  rim pushed forward a pixel. `module_cols` (now seven entries): cell 3
  while `Tank::sonic_flash` (`sonic_flash_seconds`, set by `kick_sonic` on
  every blast - the room's, a replica's `Fired`, a client's press), 1 and 2
  alternating at `4 + 10 * progress` Hz through a tell, else 0. Anchor:
  `tank_art::SONIC_MUZZLE` (the dish's face, turret frame), generated by
  `export.py` from the module's `meta['muzzle']` like `GRENADE_MUZZLE`.
  `render.SHOWN_TOGETHER` leaves it out with the grenade launcher.
- **The crate**: row 13 of `gen_crates.py`'s sheets (`crates_sheet.png` 280
  x 560, `pickup_glyphs.png` 24 x 336). Its symbol, 10 x 10 design px: a
  speaker's cone on the left and two arcs of sound to its right, the arcs
  (`o`) in the ink's light:

  ```
  '....X.....',
  '...XX..o..',
  '..XXX...o.',
  'XXXXX.o..o',
  'XXXXX.o..o',
  'XXXXX.o..o',
  'XXXXX.o..o',
  '..XXX...o.',
  '...XX..o..',
  '....X.....',
  ```

  Ink (`punypalette.PICKUP_INK['sonic_hammer']`, admitted on the crate
  sheets alone like the others): sky blue - shade `#1E7FB8`, base
  `#46C3F2`, light `#A8E6FF` - the one hue the inks leave free between the
  plasma's teal and the tower pack's periwinkle. To be shown beside the
  other crates in a screenshot before it is settled (§12).
- **The HUD**: nothing new to lay out. `hud::WeaponSlot::of` gives the
  count in `HUD_SONIC_COLOR` (`#46C3F2`, the ink's base) and
  `hud::weapon_pickup` the glyph; the ring's ammo pips measure against
  `full_load` (6).

## 6. Tuning

New group `sonic` (every row live unless marked), plus one row in the
enemies' group:

| Row | Default | Range | Doc |
|---|---|---|---|
| `sonic_ammo_per_pickup: i32` | 6 | 1..=40 | Blasts one sonic hammer crate loads. One per press. |
| `sonic_reload_seconds` | 1.2 | 0..=10 | Seconds between two blasts. |
| `sonic_reach_px` | 160 | 32..=480 | How far the cone reaches from the pivot (px; five cells). |
| `sonic_half_angle_deg` | 35 | 5..=90 | The cone's half angle either side of the facing. |
| `sonic_edge_falloff` | 0.35 | 0..=1 | The share of the shove and the damage left at the rim, falling linearly from 1 at the pivot. |
| `sonic_wave_speed` | 640 | 60..=5000 | How fast the front runs out (px/s): what it reaches, it reaches when the front gets there. |
| `sonic_shove_speed` | 380 | 0..=500 | The shove at the pivot (px/s) against the chassis-free mass. |
| `sonic_shove_max_speed` | 420 | 0..=500 | The most any shove gives (px/s); the wire's shove reaches 508. |
| `sonic_mass_exponent` | 1.5 | 0..=4 | How hard a heavy chassis resists: the shove over the mass factor to this power. |
| `sonic_skid_decel` | 650 | 50..=5000 | A skidding hull's friction on dry ground (px/s^2): its slide is `v^2 / (2 * this * grip)`. |
| `sonic_skid_grip_floor` | 0.4 | 0.05..=1 | The least of the ground's grip the skid keeps: ice, a ford and wet ground slide further, at most `1 / this` times as far. |
| `sonic_skid_max_seconds` | 1.2 | 0..=5 | The longest a hull is off its tracks. |
| `sonic_damage` | 4 | 0..=50 | Damage at the pivot to the side opposing the shooter, falling off like the shove; no roll. |
| `sonic_recoil_speed`, `sonic_recoil_max_speed` | 30, 40 | 0..=200, 0..=400 | The shooter's kick. |
| `sonic_grass_flat_seconds` | 6 | 0..=60 | How long a cell of flattened tall grass hides nobody before it stands back up. |
| `sonic_drum_throw_cells` | 4 | 1..=12 | Cells a drum at the pivot is thrown, scaled by the falloff, at least one. |
| `sonic_drum_aim_deg` | 25 | 0..=90 | A tank within this of a thrown drum's line, beyond it, draws the drum onto its cell. |
| `sonic_frog_stun_seconds` | 1.5 | 0..=10 | How long a frog the wave reaches neither hops nor bites. |
| `sonic_grenade_push_speed` | 260 | 0..=1000 | A grenade's push at the pivot (px/s), falling off like the shove. |
| `sonic_tell_seconds` | 0.55 | 0..=3 | An enemy's wind-up before its blast; 0 fires on the decision. |
| `sonic_wave_seconds` | 0.6 | 0.05..=3 | How long the wave's picture lingers after its front is out. |
| `sonic_wave_rings` | 3 | 1..=6 | Arcs in the wave, the front and those behind it. |
| `sonic_ring_gap_px` | 14 | 2..=64 | Px between two arcs. |
| `sonic_dust_spacing_px`, `sonic_dust_seconds` | 10, 0.5 | 2..=64, 0.05..=3 | Dust along the front: one every this many px, gone in this long (0 spacing turns it off). |
| `sonic_shock` | 0.35 | 0..=2 | The screen ripple and shake against a tank dying's. |
| `sonic_flash_seconds` | 0.2 | 0..=2 | The dish's firing cell. |
| `sonic_tree_lean_px` | 4 | 0..=16 | How far a crown leans as the front passes. |
| `sonic_fish_throw_px`, `sonic_fish_throw_max`, `sonic_fish_flop_seconds` | 40, 3, 2.5 | 0..=200, 0..=16, 0..=20 | A fish this close to dry ground along the wave's line is thrown onto the bank, at most this many a wave, flopping this long. |
| `sonic_ai_breaker_px` | 112 | 0..=480 | An enemy shouts at a seat this close in its cone, whatever lies behind it. |
| `sonic_ai_glass_after_seconds` | 0.1 | 0..=5 | How long an enemy drives into glass before it shouts it down (the breach's `enemy_breach_after_seconds` for every other tile). |
| `sonic_ai_fire_interval` | 2.5 | 0.1..=10 | Seconds between an enemy's decisions to shout. |
| `enemy_special_weapon_sonic_share` (`enemies`, `@ Restart`) | 0 | 0..=1 | The share of special-carrying enemies that spawn with the sonic hammer instead, decided by a hash of the spawn point and the slot - never the round's RNG - so at 0 nothing changes. |

Constants (geometry, not feel): `SONIC_RAY_ARC_PX` (4) and
`SONIC_TROUBLE_STEP_PX` (8) in `sonic.rs`, `POSE_KNOCK_GRACE_TICKS` (60)
beside `POSE_REACH_SLACK_PX`.

## 7. Text

Data names by family (`named("tool", ..)`), so no `text::keys` constant;
both inside their budgets (`every_language_fits_every_budget`), checked in
Phase 2 - `zvočni top` stands by if `zvočno kladivo` overflows a row.

| Key | en | sl |
|---|---|---|
| `tool-sonic_hammer` | sonic hammer | zvočno kladivo |
| `tool-short-sonic_hammer` | sonic | zvok |

The HUD shows the glyph and a count; the weapon has no other words.

## 8. Wire

Protocol 15 (from 14), once in the PR.

- `WeaponKind::SonicHammer`, appended to `ALL`.
- `WireEvent::SonicBlast { slot: u16, x: i16, y: i16, dir: u8 }` - the
  pivot in quarter pixels, the facing as `dir_index`; mirrors
  `Event::SonicBlast { slot, x, y, dir: &'static str }`.
- `WireEvent::Shoved { seat, vx, vy, skid: u8 }` - the skid in tenths.
- `WireEvent::DrumLaunched { x, y, to_x, to_y, drum: Drum }` - a thrown oil
  drum flies as an oil drum (the replica assumed fuel, the only drum a fuse
  ever launched).
- `TankState::tell: u8` (tenths left, 0 none) and `TankState::skid: u8`
  (tenths). `tank_flags` stays a byte.
- `frog_flags::STUNNED` (bit 4). A replica holds `Frog::stun_timer` up while
  the flag is set and lets it run out when it clears.
- `Event::TellStarted` is on `NOT_SENT`.
- **What a replica draws**: on a `SonicBlast`, `sonic_show` (the wave cast
  against its own tiles - the pane the wave shatters is still standing
  then, the death comes in a later snapshot - the ripple, the dish's flash),
  whose cosmetic sweep flattens its picture's grass and scatters its fish as
  the front passes (`tick_presentation`); an enemy's tell from
  `TankState::tell`; skids from `TankState::skid`; the rest is state and
  the existing events.
- **What is predicted**: the seat's own press show (§3.3) and its recoil on
  an owned hull; its own skid once the room's `Shoved` says so. Nothing
  else - the shoves of others, damage, tiles, drums and stuns are the
  room's.
- `delta.rs` needs nothing new (the fields are inside `TankState`); its
  random snapshots fill them, and the size bounds are re-measured.

## 9. Determinism

- **The hammer draws no RNG at all.** No damage roll, no misfire, no
  launch roll (the throw is computed), no spread. Its walk is fixed: per
  wave in the order they were fired, the seats in index order then the
  enemies by slot, then frogs (player's, enemy's), cells by entry distance
  then cell, lanterns by id, grenades by id. A thrown drum's blast draws its
  damage rolls as every blast does, where it lands.
- **The swap** is a hash (§3.4); the AI's choices are fixed priorities with
  slot tie-breaks (§3.1); the tell, the skid, the stun and the flattened
  grass are timers.
- **A round without the hammer replays byte for byte**: no crate kind is
  rolled anywhere (slots are fixed and the respawn picks a slot), every
  share defaults to 0, no enemy carries one, so there is no wave, no tell,
  no skid and no sense; `SpecialSense::None` makes the `special` tier a
  `false` with no state touched; `seek_special` finds no crate;
  `cover_cells` is `grass_cells` while `grass_flat` is empty; `knock` is
  never called. `determinism_tests`' pinned streams, the probe fixtures'
  ceilings and every thumbnail pin but the armory's new one stay as they
  are.

## 10. Tests

`mechanics_tests` (headless, tiny inline maps):

- `a_sonic_crate_arms_the_hammer_and_replaces_the_special_carried` - a
  crate gives six, empties another special, a second crate refills to six.
- `the_hammer_fires_on_the_press_and_spends_one_blast` - one blast per
  press, none while held, the reload holds the next; `Fired` then
  `SonicBlast` in one tick.
- `the_cone_shoves_an_enemy_ahead_and_not_one_beside` - ahead slides away,
  at 90 degrees untouched.
- `the_wave_reaches_far_things_later` - a near and a far enemy start moving
  on the frames the front reaches them.
- `the_shove_falls_off_with_distance` - the near one slides further.
- `a_heavy_chassis_barely_slides` - a titan's slide under a quarter of a
  scout's at the same distance.
- `a_skidding_tank_cannot_drive_and_drives_again_when_it_stops` - a held
  stick does not move it along the stick during the skid, does after.
- `the_skid_slides_as_far_whichever_way_the_hull_faces` - the same shove
  head-on and broadside, the same slide within a pixel.
- `ice_lengthens_the_skid` - under snow, on a frozen lake, the slide is
  `1 / sonic_skid_grip_floor` of the dry one.
- `iron_brick_and_towers_shadow_the_cone` - an enemy behind each untouched,
  one beside the shadow reached.
- `glass_shatters_when_the_wave_reaches_it_and_shadows_what_stands_behind`.
- `a_lamp_post_is_put_out_and_a_lantern_broken`.
- `grass_the_wave_crosses_stops_concealing_for_a_while` - `Terrain::conceals`
  true, false after the front, true again after `sonic_grass_flat_seconds`;
  the AI sees the seat in between.
- `a_drum_is_thrown_onto_the_tank_behind_it` - `DrumLaunched` to the tank's
  cell, the blast there.
- `a_drum_with_no_tank_behind_it_is_thrown_along_the_wave` and
  `a_fused_drum_is_thrown_too`.
- `a_frog_the_wave_reaches_is_stunned_and_neither_hops_nor_bites`.
- `the_hammer_hurts_only_the_other_side_and_shoves_everyone` - two seats:
  the teammate shoved and whole, the enemy hurt by `sonic_damage` times the
  falloff.
- `a_shield_soaks_the_damage_not_the_shove`.
- `a_grenade_in_the_cone_is_pushed_away`.
- `a_wreck_shots_in_flight_and_crates_are_left_alone`.
- `a_tank_shoved_into_a_portal_goes_through_and_stops_skidding`.
- `the_wave_finishes_on_the_end_screen_and_moves_nothing`.
- `an_owned_hull_is_allowed_its_skid` - `accept_seat_pose` takes a pose a
  skid carried past the chassis's speed, within the grace.
- `a_round_with_the_hammer_replays_bit_for_bit`.
- `the_spawn_swap_is_off_at_zero_and_draws_nothing` - shares 0: every kit
  and the RNG state after `init` as before; share 1: every enemy that drew
  a special holds the hammer, and the RNG state is the same.
- `an_enemy_collects_the_crate_only_on_shells`.

AI (`ai.rs` unit tests on a `Brain` with a made-up `HammerSense`, and
`mechanics_tests` on a whole round):

- `the_hammer_shoves_a_seat_into_trouble`, `..._throws_a_drum_onto_a_seat`,
  `..._flushes_the_grass_at_the_alert`, `..._breaks_a_seat_in_its_face`,
  `..._pins_the_hunters_frog` - each arm fires facing its way with its seat.
- `the_hammer_never_fires_through_a_friend`.
- `the_hammer_holds_while_its_tell_runs`.
- `the_generic_tiers_never_fire_the_hammer` - lined up at 300 px: no fire.
- `only_one_brawler_closes_in_on_a_seat`.
- `the_predicted_slide_is_the_skid` (`sonic::slide` against a stepped
  `Game` within 2 px).
- `an_enemy_never_shouts_at_a_seat_from_outside_its_sight_box` (whole
  round: a seat in the cone, the tank outside its box: no `TellStarted`).
- `an_enemy_shouts_down_glass_in_its_way` - driving into glass: a tell
  within `sonic_ai_glass_after_seconds` plus the tell, the pane gone.
- `an_enemys_tell_runs_before_its_blast` - `TellStarted`, then `Fired` and
  `SonicBlast` `sonic_tell_seconds` later, facing held, no movement.
- `a_wrecked_tanks_tell_never_goes_off`.
- `a_shoved_enemy_does_not_count_as_stuck` - `stuck_timer` unchanged
  through a skid it commanded against.

Shared path and presentation:

- `sonic::tests`: `the_cone_reaches_its_reach_on_open_ground`,
  `a_blocking_cell_cuts_the_rays_that_meet_it`,
  `glass_is_entered_and_stops_its_rays`,
  `the_cone_never_leaves_the_field`, `drum_landing_takes_the_nearest_tank_behind_then_the_slot`,
  `drum_landing_walks_back_off_a_tile`; the composers
  `the_wave_is_on_the_grid_in_its_ramp_and_gone_by_its_end`,
  `the_wave_stops_at_a_walls_face`, `the_tell_is_on_the_grid_and_pure`.
- `pyro`: `block_arc_stays_on_the_grid_and_within_its_angles`.
- `tank` (`weapon_inventory_tests`): the hammer in `take_weapon`,
  `full_load`, `active_weapon`, the module's cells.
- `pickup`: `name`/`parse` round-trip every kind; `weapon`.
- `hud_tests`: the hammer's slot, colour and glyph.
- `frog`: `a_stunned_frog_cannot_hop_or_bite`.
- `grass`: `a_pinned_tuft_stays_flat_then_recovers`.
- `fish::tests`: `a_wave_scares_the_fish_it_passes`,
  `a_shout_at_the_shore_throws_a_fish_onto_the_bank_and_back`.
- `indicator_tests`: `a_tell_off_screen_gets_an_arrow_the_cap_never_drops`,
  `a_sonic_hit_points_the_arc_at_its_pivot`.
- `devserver`: `spawn_pickup_puts_a_crate_down` (and its refusals: off the
  field, on a tile, on a crate, online, in build), `set_tank_arms_the_sonic_hammer`,
  the PICKUP category's count (13 to 14).
- `editor`/`chrome_tests`: the tool in `TOOLS`.
- `thumbnail`: the armory's pin. `maplint`: the armory in `SUPPORTED_MAPS`.
- `text_tests`: every budget.

Wire:

- `events.rs`: the samples gain `SonicBlast`, `TellStarted` (not sent),
  `Shoved` with a skid and `DrumLaunched` with a drum; the variant count.
- `apply.rs`: `a_sonic_blast_reaches_the_replica` (a seat armed by
  `debug_set_tank`, pressing: the replica gets one wave per blast and
  draws the same picture after every apply), `an_enemys_tell_reaches_the_replica`,
  `a_stunned_frog_reaches_the_replica`,
  `a_sonic_blast_this_client_drew_is_not_drawn_again` (`OwnShotsDrawn`
  with the press's bit: no wave, not handed on; without: a wave),
  `the_beam_claim_still_holds` (the existing beam tests on `presses`).
- `predict.rs`: `a_sonic_press_is_drawn_at_once_and_claims_the_rooms_show_once`,
  `a_sonic_press_waits_for_the_reload`, `a_shove_with_a_skid_skids_the_sandbox`.
- `round.rs`: `the_rooms_sonic_blast_is_left_out_only_for_a_press_this_client_drew`
  (the beam test's twin), `an_owned_hull_skids_on_the_rooms_shove`.
- `rig.rs` (`Lockstep`): `a_seats_sonic_blast_is_drawn_once_on_the_replica`
  (one wave per press, never two) and
  `an_enemys_sonic_blast_reaches_the_replica_after_its_tell` (an enemy
  armed through `authority_mut`, a seat in its cone: the replica's tank
  shows the tell, then the wave).
- `delta.rs`: the random snapshots and the size bounds.
- The room server's `cargo test -p bongbong-server` as it stands.

## 11. Probe

- **Defaults first**: `just probe-fixtures` and `just probe-fields`
  unchanged, passing their recorded ceilings untouched.
- **With the crate**: the same two sweeps with `--crate sonic_hammer` - the
  recipes take extra arguments and pass them to every run
  (`just probe-fixtures --crate sonic_hammer`). AFK: enemies on shells
  collect it and use it.
- **Armed enemies**: the same sweeps with `--tuning armed.json`,
  `{"enemy_special_weapon_chance": 1.0, "enemy_special_weapon_sonic_share":
  1.0}` - every enemy spawns with the hammer.
- The probe's tank line gains `sonic=`; its fire tuple counts the hammer's
  ammo, so a blast is a trigger pull for `FIRED_RECENTLY_FRAMES`; a tank in
  a tell or a skid is a deliberate hold (`TankSnapshot::{tell, skidding}`),
  not a stall, low progress, jitter or grind.
- **The bar**: every crate and armed run within the defaults' ceilings,
  `offbox-fire` 0. An exceedance is read round by round from its
  `ANOMALY` lines; one the hammer's own action causes - a tank stranded,
  spinning, piling up or firing from off the box - is fixed, not
  re-baselined. Recorded here in Phase 2: the totals at the defaults, with
  the crate and armed, and what moved.

## 12. Interactions, decisions, what is left out

### Interactions with what ships

| With | What happens |
|---|---|
| Rainbow shield | Soaks the damage; the shove lands whole |
| Heat shield | Nothing (it is not heat) |
| Portals | The wave does not pass through; a shoved tank goes through and stops skidding |
| Water | Crossed; a shove stops at a deep shore, wades into a ford; dust is chop |
| Ice (snow) | Longer skids (the grip floor); no fish to scatter under ice |
| Rain | Wet grip: a slightly longer skid |
| Sandstorm gusts | The skid runs in the gust's frame, as the drive does |
| Night, fog | The wave and the tell drawn unlit, so they read; the AI's sight rules as ever |
| Lava, the volcano | Crossed; a cone cell stops it; a shove into a ford or onto a hot bank burns - the AI's trouble |
| Towers | Stop the wave, take nothing; an enemy tower's reach is trouble |
| Frogs | Stunned (either side's, your own too); Hunt's enemy frog stunned is a shot frog |
| Crates | Untouched, not broken |
| Drums, oil, ground fires | Drums thrown and going off where they land (fused or not); oil and fires untouched; a thrown oil drum leaves its pool |
| Trees, grass | Trees sway and stop nothing; grass flattened and hides nobody for a while |
| Glass, lamp posts, lanterns | Shattered, put out, broken |
| Grenades | Pushed |
| Missiles, shells, bullets, plasma, the laser, the flamethrower | Untouched |
| Field maps | The sight box binds every use; a far coasting tank's tell still goes off; waves, the director, stragglers as ever |
| The couch and its split | Teammates are shoved (never hurt); nothing else |
| Training | `drop = ["sonic_hammer"]` works by its name; the training dummy never uses it on a seat |
| Online | §3.3, §8 |
| Earlier BB-36 weapons | None: it is the first |

### Decisions taken

1. **A skid** ("knocked off its tracks", §3.6) **rather than a plain
   impulse.** The drive model brakes a hull along its axis exponentially
   (at about 20/s for a standard chassis) and across it at a constant
   `tank_turn_grip_force / mass` - so one impulse of `v` px/s slides a
   broadside hull `v / 50` times as far as a head-on one (six times at
   300 px/s, more the harder the shove), and head-on is the common case in
   an axis-aligned shoot-out. The skid makes the slide the same whichever way
   the hull faces, predictable for the AI (`sonic::slide`), and gives the
   gravity well its tool. Rejected: the explosion's knockback (capped by
   `knockback_max_speed` at 60 px/s: a nudge), and an impulse with the cap
   lifted (the anisotropy above).
2. **The wave travels**, striking things as its front reaches them, rather
   than everything on the press. Things happen as the arcs pass - panes
   shatter and grass falls in a sweep - and a far target has a quarter
   second to see it coming. Rejected: instant (cheaper, but the picture
   arrives after what it shows).
3. **Glass stops the wave as it shatters**: "it does not pass through any
   wall" read strictly - a pane is a one-shout shield. Rejected: the wave
   carrying on through the glass it breaks. *For Oto.*
4. **Shadows are a fan of rays on the map-cell grid**, cast once at the
   press: walls fill their cells, a cell walk is cheap and exact for them,
   and the arcs can be clipped to the same rays, so what is drawn is what
   is ruled. Rejected: a segment test per target (no picture of the
   shadow, and a target's edge needs several tests anyway) and a walk on the
   2 px grid (16 times the steps for the same walls).
5. **No RNG**: fixed damage with falloff and a computed drum throw, so a
   round with a hammer in it stays a pure function of its inputs and its
   online picture is the room's. Rejected: a damage roll like a shell's.
6. **Drums are thrown**, through the existing launch, since a tile cannot
   be pushed as a body; a thrown drum always goes off where it lands, which
   is "sets fuses off early". Rejected: lighting fuses in place only (no
   "toward whoever stands behind").
7. **Only the opposing side is hurt, everyone is shoved**, frogs of both
   sides stunned - the side blast's rule (grenades, missiles).
8. **The AI owns the trigger**: an enemy carrying the hammer never fires it
   through the generic tiers; it fires by its rule, approaches as one
   brawler per seat, and otherwise keeps its place. Rejected: letting the
   generic attack fire it at 340 px (wasted), and letting an enemy fire
   shells while it carries it (breaks the one-trigger rule).
9. **A hunter pins the frog** rather than doing nothing with it - six
   blasts, then shells. *For Oto.*
10. **The tell commits**: it goes off along its facing even if the seat
    stepped out, which is the dodge.
11. **The special tier sits above flee**: a hurt tank's breaker is the
    weapon's best use; the approach is only for a healthy one.
12. **One `seek_special` tier** for the BB-36 kinds rather than a tier per
    kind, so a new weapon is one list entry and the existing tiers (and
    every trace) are untouched.
13. **The arcs and the tell are drawn in the glowing pass** (unlit, for
    fairness at night) and the dust in the lit pass. Sound throws no light.
14. **The crate spills** rather than cooks off - it is a speaker, not
    ordnance.
15. **The skid and the tell travel as tenths in `TankState`**, not flags:
    a stage-2 replay needs how long, and a replica counts both down.
16. **The press show generalises the beam's claim** (`presses` for
    `beams`) rather than adding a second claim beside it.
17. **Crate ink sky blue** (`#46C3F2`). *For Oto*, with a screenshot of the
    crate beside the other twelve in Phase 2.

### Not in this PR

- Placing the hammer in the shipped levels - level design, a follow-up.
- A "SPOTTED" label over a seat the wave uncovers (the armory scene's) -
  the flattened grass is the reveal; a word over the field is HUD design.
- A tell's pulse on the minimap - the arrow is the warning; the minimap
  shows terrain and marks, a follow-up if play-tests want it.
- Sound effects - the game has no audio yet.
- Shoving wrecks, deflecting shots in flight, putting fires out, breaking
  crates, stunning towers (the EMP's) - each would be a mechanic of its own
  beyond the issue.
- A probe scenario in which the seat fires its special - the AFK and
  advance scenarios measure the enemies, which is what the probe is for.
