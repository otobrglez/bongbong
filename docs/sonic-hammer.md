# Sonic hammer

BB-37, the first of the six weapons of BB-36. A special weapon from its own
crate (`pickup = "sonic_hammer"`), one at a time like every other
(`Tank::take_weapon`): a crate loads `sonic_ammo_per_pickup` (7) blasts, a
re-pick refills to seven, another weapon's crate replaces it. Seats and
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
  (`drive_player`'s `fire_pressed`); `sonic_reload_seconds` (0.8) between
  blasts (`Tank::fire_cooldown`). A seat's press goes off at once - no
  wind-up. An enemy's goes off after its tell (§3.2).
- **The cone.** From the hull's centre (`Tank::position`, the turret's
  pivot), along the hull's cardinal facing (`Dir::from_rotation`), out to
  `sonic_reach_px` (208, six and a half cells), `sonic_half_angle_deg` (46)
  either side of the facing. No misfire skew: the hammer is never fired off-aim.
  It is judged from the pivot so a tank pressed against a pane still
  shatters it; it is drawn from there too, the arcs leaving the dish on the
  roof (the dish is a few pixels ahead of the pivot, inside the first arc).
- **What shadows it: a ray cast on the map-cell grid.** At the press the
  cone is cast once as a fan of rays (`sonic::SonicCone::cast`), one every
  `SONIC_RAY_ARC_PX` (4) px of arc at full reach (85 rays at the
  defaults), each walked cell by cell (Amanatides-Woo, the
  `weather::Occluders` walk) over the map's cells (`map::world_to_cell`: centres on multiples of
  32). A ray stops on entering the first cell holding a tile that
  `Material::blocks_sound`: every wall material - brick, iron, wood and
  glass - the three towers, a volcano's cone and a training door; its reach
  is the distance to that entry. It also stops at the field's edge. Props,
  drums, range boards, trees, lamp posts, pickups, water, lava, grass and
  tanks do not stop it. A destroyed tile is not there. The cast records, per ray, its
  reach, and, per cell any ray entered, the least distance at which a ray
  entered it. "In the cone" from then on means: a point within the half
  angle whose distance is no more than the reach of the rays either side
  of its bearing (the lesser of the two), or, for a cell, a cell some ray
  entered. Nothing of the cast is re-done as the world changes under the
  wave: a wall that falls mid-wave lets nothing more through, a tank that
  drives behind a wall mid-wave is shadowed by the rays already cast.
- **The wave travels.** What the cone reaches, the wave reaches when its
  front gets there: the front leaves the pivot at the press and runs out at
  `sonic_wave_speed` (640 px/s - the full reach in about a third of a second). Each
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
    landed with `cause: HitCause::Sonic` (§5: a hull flash and dust, never
    a shell's fire), and `Ai::notify_hit` on a surviving enemy - it knows
    it was hit. A teammate, a fellow enemy and the shooter's own side take none:
    they are only shoved.
  - A wreck is not moved (as no blast moves one), a tank rolling in through
    a gate is not on the field, and the shooter is never reached.
- **Glass** (`Material::breaks_by_sound`, glass alone): a glass cell some
  ray entered dies when the front reaches the cell's entry distance
  (`damage_obstacle` with the tile's whole health, `DamageCause::Shot`
  along the ray - the ordinary death path: `ObstacleDestroyed`, the
  glass's rubble row, the edge masks). Glass stops the rays that meet it as
  it shatters - the wave does not pass through any wall - so what stood
  behind a pane is spared until the next shout.
- **Lamp posts** are left standing: a pole stops nothing, and a lamp post
  is a map fixture whose light is a rule (`lamp_reveal_px`) - putting the
  lights out is the EMP's (BB-38).
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
  fish the front passes within `sonic_fish_throw_px` (40) of dry ground on
  the map, along the line from the pivot, is thrown onto the bank and flops
  there for `sonic_fish_flop_seconds` (2.5) before hopping back to where it
  was - at most `sonic_fish_throw_max` (3) a wave, nearest the pivot first.
  Never past the map's edge: a lake painted to it runs on, and the margin
  past an arena's field is no bank. Presentation only (`fish.rs`), hashed,
  nothing on the wire: a replica throws the same fish off the same wave.
- **Trees** stop nothing; their crowns lean away from the pivot for a moment
  as the front passes (cosmetic, `sonic::tree_push`, added to
  `obstacle::tree_lean` at draw time).
- **What it leaves alone**: shots in flight (shells, bullets, plasma,
  missiles, flying drums, lava bombs, globs - a shell outruns sound), wrecks,
  crates (no damage, they do not break), towers (they stop it and take
  nothing), sandbags, fences and range boards (it passes over them), lamp
  posts, oil, ground fires, ooze, scorches, rubble, tread marks.
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
| `src/sonic.rs` (new) | The weapon's headless half. `SonicCone` (`cast` from a pivot and facing over a `Block` per cell - open, a wall, glass - and a `Floor` per cell, inside the field; `rays`, the `cells` it entered in entry order with their floor, the glass `panes` it struck; `reaches(p)`, `entered(cell)`, `nearest_reached(points)`, `longest`), `SonicWave` (cone, owner, age, `swept` radius, `struck` hulls, a seed), `falloff`, `shove_speed(mass_factor, d)`, `skid_friction`, `skid_seconds`, `slide(v, grip)` (the skid's slide, what the AI predicts with), `drum_landing`, `tree_push`, the composers `wave_arcs`, `wave_dust` and `tell_arcs` (pure, `pyro::Shape`s), the `STONE` ramp, `module_cell` |
| `src/simulation/sonic.rs` (new) | The world half. `fire_sonic` (called from the dispatch arm), `resolve_sonic` (casts the frame's presses into waves), `tick_sonic_waves(f, live)` (the strike walk), `knock` (the shove and skid any later weapon may use), `throw_drum`, `sonic_show` (the cosmetic half - the wave on the list, the ripple, the dish's flash - which a replica's `SonicBlast` and a client's press call too), `tick_sonic_pictures` (a replica's waves and flattened grass), `Game::hammer_senses` with `lands_in_trouble` and `closer_spots` (what the AI is handed, §4), `seat_sonic`, `swap_spawn_special` (§3.4), `debug_spawn_pickup` |
| `src/simulation/sonic_tests.rs` (new) | The scenario tests (§10) |
| `src/simulation/weapons.rs` | The `ActiveWeapon::SonicHammer` dispatch arm |
| `src/simulation/mod.rs` | `Frame::pending_sonic`; `Game::{sonic_waves, grass_flat, seat_knock}` (`SeatKnock`, §3.6), `cover_cells`; the phase calls (`resolve_sonic` and `tick_sonic_waves` after `resolve_flames`, before `step_world`, so the shoves land in this tick's solver step; `tick_sonic_waves(f, false)` on the end screen); the tell in `enemy_phase` (§3.2); the skid in `drive_tank_with`, `tick_timers` and `predict_seat`; `accept_seat_pose`'s knock allowance; the spawn swap in `roll_enemy_tank` (§3.4); a hammer hunter woken and leashed as any tank (`field::mind`'s `hunting`, `TankSnapshot::leashed`), `TankSnapshot::guarding`; the `pickup_phase` arm; `tick_presentation` (waves, tells, skids, frog stuns, flattened grass); `Event::{SonicBlast, TellStarted}`, `Event::Hit::cause` (`HitCause`), `Event::Shoved::skid`, `Event::DrumLaunched::drum` |
| `src/tank.rs` | `sonic_ammo`, `ActiveWeapon::SonicHammer` (`name`, `full_load`, `tell_seconds`), `SPECIAL_WEAPONS`, `Tank::special` (§3.0), `Windup`/`Tank::windup` (§3.2), `weapon_ammo`/`take_weapon`/`empty_stock`/`wants_pickup`, `Tell`/`tell`, `skid`/`skid_speed`, `sonic_flash`/`kick_sonic`, the module's cell in `module_cols` |
| `src/pickup.rs` | `PickupKind::SonicHammer` (`sonic_hammer`, row 13, its ink, spills rather than cooks off), `PickupKind::{name, parse, weapon}` |
| `src/obstacle.rs` | `Material::blocks_sound`, `Material::breaks_by_sound` |
| `src/ai.rs` | The special hook (§3.1): `SpecialSense`, `HammerSense`/`HammerAim`, `SpecialUse`, `special_rule` / `hammer_rule`, `act_special`, `generic_fire`, the `special` tier, the `seek_special` tier, the stuck and breach clocks paused under a skid, `AiSnapshot::special`, `Ai::holds_beat` |
| `src/frog.rs` | `stun_timer`, `stun`, `is_stunned`, the gates in `can_hop`/`can_attack`, `stun_marks` |
| `src/grass.rs` | `GrassTuft::pinned`, `pin` |
| `src/fish.rs`, `src/ground.rs` | The waves among the scares; `Flop` (the fish on the bank), never off the map (`WaterLayout::contains`) |
| `src/pyro.rs` | `Shape::Arc`, `block_arc` (shared, §5) |
| `src/indicators.rs` | `TankView::windup`, `ArrowKind::Windup`, the hit arc from a sonic blast |
| `src/hud.rs`, `src/render/hud.rs` | `HUD_SONIC_COLOR`, the `weapon_color`/`weapon_pickup` arms; the weapon slot's readout drawn by one function (`draw_weapon_readout`), which a weapon whose readout is a gauge or words replaces by its weapon |
| `src/game.rs`, `src/render/game.rs` | Trees leaning, frog stun marks, the waves, dust and tells in their passes (§5), the dev stats arm |
| `src/fx.rs` | Dust off a skidding hull; a sonic hit's flash and dust, no fire |
| `src/net/wire.rs` | `WeaponKind::SonicHammer`, `WeaponKind::drawn_on_press`, `TankState::{tell, skid}`, `frog_flags::STUNNED` |
| `src/net/events.rs` | `WireEvent::SonicBlast`, `Shoved::skid`, `DrumLaunched::drum`, `WireEvent::press_show`, `tell_started` on `NOT_SENT` |
| `src/net/encode.rs`, `src/net/apply.rs` | The new fields; `Show::OwnShotsDrawn { seat, presses }`, `presses_drawn`; `SonicBlast`'s spectacle |
| `src/net/predict.rs`, `src/net/round.rs` | The press show (§3.3), `Predictor::shove(dv, skid)` |
| `src/simulation/present.rs` | `Game::{draw_press_show, seat_kick}` |
| `src/simulation/replica.rs` | `DrawableTank::{tell, skid}`, `DrawableFrog::stunned` |
| `src/simulation/debug.rs`, `src/devserver.rs` | `spawn_pickup`, `set_tank`'s `sonic_ammo`, the snapshot's `sonic`, `tell`, `skid`, `stun` and `sonic_waves` |
| `src/bin/probe.rs` | `--crate`, the tank line's `sonic=`, the holds, a guard on its beat no never-arrived, a hold just ended no stale-start (§11) |
| `src/editor/mod.rs` | `Tool::Pickup(PickupKind::SonicHammer)` (`sonic_hammer`) |
| `src/editor/chrome.rs` | The folded TOOLS palette: a category with more tools than the room is wide for (PICKUP's fourteen on a phone) runs on into a second row, and where the rows would not fit the room's height the cells shrink, never under `UI_TOUCH_PT` |
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
   (`special().is_none()` for an enemy, the one rule), `module_cols`.
   **Carries or fires**: `Tank::special()` is the special a tank carries
   with ammo left, `None` on shells alone, and `active_weapon()` what its
   trigger fires (`special()` or else shells). A reader asking what a tank
   *carries* takes `special()` - its module (`module_cols`), the wire's
   `TankState::weapon`, the HUD's slot (`hud::WeaponSlot`), whether it
   takes a weapon crate, the spawn swap's "drew a special"; one asking what
   it *fires* takes `active_weapon()` - the dispatch, the tell, the AI's
   rule and `generic_fire`. The two agree today; a weapon carried that
   cannot fire (a charge, a cooldown of its own) is where they part.
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
    /// Hold still facing `face`; `why` for the trace.
    Hold { face: Dir, why: &'static str },
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
  snapshots). The hammer's is `Game::hammer_senses` (§4).
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

/// A wind-up in progress, whatever winds it up: what the AI's rule holds a
/// tank to, the off-screen arrow and the drawing read. A tell is one; a
/// later weapon's charge joins `Tank::windup` as another.
pub struct Windup { pub weapon: ActiveWeapon, pub progress: f32, pub facing: Dir }
pub fn windup(&self) -> Option<Windup>;
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
  tank holds its aim (and the rule's first arm holds it too). With the
  command layer on (`c2_enabled`), whose apply pass replays the commander's
  orders over the collect pass's intents, the override is applied again
  after `Commander::apply` (`commanded_intent`: the orders, then the hold),
  so it is the last word on a tank in a tell. The far tanks of a field map
  that coast between thinks count their tell down the same way: a tank
  that does not think this tick goes through `coast_enemy` - the tell's
  hold, `control`, `tick_queued_shots`, `enemy_trigger` with the trigger
  released, its `Pending` for the apply pass - and a later weapon that
  takes a tank's thinking away (a stun) does so at the one spot in
  `enemy_phase` before the field map's own choice (`field::mind`), through
  the same helper. A teleport cancels it (`portal_phase`), and so does
  the round's end, with no blast (`end_round`, as it lapses every charge),
  so nothing winds up on the end screen; `init` has none. A tell commits: a seat that steps out of the cone during
  it is the dodge the tell is for.
- **A seat has no tell**: a seat's press fires on the press. A later
  weapon whose own mechanic is a charge (the Gauss rail) or a call-in (the
  rod) may give a seat a `Tell` of its own; that is its PR's to wire.
- **How it is drawn**: per weapon, `render/game.rs` asks the weapon's
  composer for every tank with a tell (`sonic::tell_arcs` for the
  hammer, §5), in the glowing pass, so it reads at night; the module shows
  its wind-up cells (`module_cols`).
- **How it travels**: `TankState::tell` - the seconds left in tenths, 0 for
  none; the weapon is `TankState::weapon`, the facing `TankState::dir`. A
  replica writes `Tank::tell` from it (`apply::write_tank`, `total` the
  weapon's `tell_seconds`) and runs it down between snapshots
  (`tick_presentation`), as it runs a grenade's fuse. `TellStarted` is on
  `NOT_SENT`: the state is what draws.
- **Off screen**: `indicators::TankView::windup` (the weapon and the
  progress, 0..1) from `Tank::windup`, so it works on a replica and a later
  weapon's charge feeds it too; an enemy winding up, off the screen and not
  concealed, gets an `ArrowKind::Windup { weapon, progress }`
  arrow, put with the teammates, frogs and volcanoes - never merged, never
  left out past `indicator_max_arrows`. A tell also reveals its tank as
  firing does (`Awareness::fired`). The arrow is drawn in the weapon's HUD
  accent (`hud::weapon_color`), rimmed hostile red as the enemy frog's is -
  an accent can be a seat's own colour (the hammer's sky blue is player
  1's), and this is no teammate - blinking at
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
  beam as now, a sonic press through `Game::draw_press_show(seat, origin,
  facing)` (the seat's tank is the wave's owner) - the same
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
  "protect"`, a band of four, clear sky - the one shipped map that is not
  a field map, which `map::toml_tests::every_shipped_map_is_a_field_map`
  names, since the BB-36 weapons' cells are laid out on its 36 x 18. Each later weapon adds its crate
  to the reserved column and whatever its interactions need. Every level
  is untouched.

  ```
       0         1         2         3
       012345678901234567890123456789012345
   0   ................................L...
   1   ............ggggg...............L...
   2   ............g...g...............L...
   3   ............g.s.g...II....b.....L.E.
   4   .+.P...r....g...g...II....b.....L...
   5   ............ggggg.........b.zzz.L...
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
  | `+` `a` | 1,4 and 1,16 | Health and ammo, out of the way (not 1,1, which the HUD's vitals cover on a whole-field view) |
  | `g` glass house | the ring 12..16 x 1..5 | Shatters; it stops the wave as it does; the AI's glass breach |
  | `s` shield | 14,3 | Inside the glass, what an enemy seeks (tier 5.9) and has to break in for |
  | `P` player tesla | 3,4 | A player tower the wave stops at |
  | `F` the frog | 2,12 | The stun; the hunters (Protect) come for it, so the frog-pin arm shows |
  | `I` iron | 20..21 x 3..4 | The counter: cover the wave cannot pass |
  | `b` brick | 26,3..5 and 28,13..14 | Walls stop it too, not only iron |
  | `*` lamp post | 17,8 | One the wave leaves standing (the EMP's to put out, BB-38); try `--weather night` |
  | `o` `f` drums | oil 21,6 and 21,10, fuel 21,8 | Thrown at whoever stands east of them |
  | `%` oil trail | 22..24,8 | A thrown drum's blast lights it |
  | `w` tall grass | 11..15 x 11..15 | Hide in it; a shout flattens it and shows you |
  | `T` trees | 16,11 and 10,13 | Crowns that sway as the wave passes; stop nothing |
  | `W` lake | 19..26 x 13..17 | Fish to scatter; a shout from the north bank (row 12) by a corner throws the fish by the west or east shore onto that bank (it runs to the map's south edge, past which nothing lands) |
  | `L` lava ford | 32,0..7 | Trouble: what the AI shoves a seat into, and where a seat shoves enemies |
  | `E` enemy tesla | 34,3 | An enemy tower's reach beside the lava: more trouble |
  | `G` enemy gun tower | 34,14 | Trouble on the south-east |
  | `z` sandbags | 28..30,5 | The wave passes over them |

  The HUD's corner clusters stand over the top rows on a whole-field view
  (rows 0 to 3 on the west and 0 to 2 on the east in a 1152 x 576 window),
  so nothing a weapon needs stands there but the lava ford's head.

  It lints with no error (`maplint`, added to `SUPPORTED_MAPS`; the shield
  inside the glass is a `gated-pickup` warning), its CPU thumbnail is
  pinned beside the others', and the cells are checked again in Phase 2
  against the linter's band capacity and the fish's lake size.

### 3.6 The knock: a shove that takes a hull off its tracks

`knock(physics, f, tank, dir, speed)` (`simulation/sonic.rs`) is the shove
any later weapon that throws tanks about uses (the gravity well):
an impulse of `speed` along `dir` scaled by the mass as above, then
`Tank::skid` and `Tank::skid_speed` (the hull's speed after it against
the ground's flow, which the skid's length is worked out from).
`drive_tank_with` reads `skid` (the skid model of §1);
`tick_timers` counts it down for every tank, and `predict_seat` for the
sandbox's seat.

**It stops at a wall's face.** While a hull skids its contacts look a
step's travel ahead (`sonic::skid_look_ahead`, run for every hull before
every solver step, the room's and a sandbox's: `Physics::set_look_ahead`,
rapier's soft continuous collision detection, at `Physics::max_step_travel`),
so the solver stops it flush against a tile, deep water or the field's
edge. Without it a knock lands a hull a step inside what it meets: rapier
measures in metres while the world is in pixels, so it looks 0.02 px ahead
and pushes a body out of an overlap at 3 px/s - a hull thrown at the edge
at 400 px/s went 5.5 px into the boundary, its skid ended as the solver
stopped it, and it crept back out over two seconds, part-way off the
field. A hull on its tracks is never touched, so a round with no knock
steps exactly as before. Online:

- `Event::Shoved { seat, vx, vy, skid }`: `Shoves::push` takes the skid
  (0 for every other shove), the wire carries it in tenths
  (`WireEvent::Shoved::skid: u8`), and `Predictor::shove(dv, skid)` puts
  the sandbox's seat into the same skid. An owned reconciliation keeps the
  seat's `skid` with its pose, as it keeps the pose.
- `TankState::skid` (tenths) carries every hull's skid, so a stage-2
  replay skids where the room does and a replica's `fx` puts dust off a
  skidding hull.
- The validator (`accept_seat_pose`) allows a knocked hull the knock and
  no more (`SeatKnock`): a knock logged for an owned seat lets each pose
  go the knock's speed further than the chassis's reach, and all of them
  together no further than the knock's slide on the slipperiest ground
  (`sonic::slide` at `sonic_skid_grip_floor`, plus `POSE_REACH_SLACK_PX`),
  until `POSE_KNOCK_GRACE_TICKS` (60, server policy) past the skid's end -
  the client's skid starts a link's delay after the room's. Two knocks add
  their slides; one that lapsed adds nothing. The allowance is set only by
  a knock the room itself logged for that seat, so a client cannot claim
  one, and spending it is what a modified client could do with it: one
  slide's worth of extra ground, never a window of free speed.

## 4. AI

An enemy carrying the hammer uses it by `hammer_rule`, never through the
generic tiers (`generic_fire(SonicHammer)` is false). `Game::hammer_senses`
runs once per frame in `enemy_phase`, before the collect pass, when any
live enemy carries the hammer; it gathers the cells that stop sound, the
cover cells, the drums, every live tank, the enemy towers and their reach,
the hunter's quarry and the seats (position, hull points, mass factor,
concealed) once, hands out the closers' spots (`closer_spots`), and gives
each hammer tank a `HammerSense`:

```rust
pub struct HammerSense {
    /// One per facing, `Dir::index` order: what a blast that way would do.
    pub aims: [HammerAim; 4],
    /// Where this tank closes in to, one of the `sonic_ai_closers` (2)
    /// hammer tanks nearest the seat it fights (`closer_spots`, below);
    /// `None` for every other tank.
    pub spot: Option<Position>,
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
(`concealed` and not hit-alerted - the attack tier's rule). A fellow enemy
(`friend`) counts by its hull's centre and corners and by where its motion
carries it by the time a blast started now would reach it (the tell and
the wave's run out), so one driving into the cone is not shouted at either.
`hammer_rule`, in priority order:

1. **A wind-up holds** (`special_rule`'s first arm, `windup_rule`, on
   `Tank::windup`): `Hold { face: windup.facing }` - every wind-up so far
   is a tell; a later weapon's charge joins `Tank::windup` and gives
   `windup_rule` its own arm.
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
      deep lava) or the field's edge. Trouble on the way, at any sample:
      heat at or over `heat_hurt_from` (`Game::heat_at` - lava and its
      banks), a burning ground cell (`Game::fires`), an ooze puddle
      (`Game::ooze`). Trouble where the slide comes to rest, when the seat
      does not stand in it already: inside a standing enemy tower's reach
      (`TowerKind::range`), or lined up for another live enemy (not this
      one) - on its row or column within `enemy_fire_align_px`, inside
      `enemy_attack_range`, with the line of sight clear
      (`Terrain::line_of_sight`) and that enemy inside the resting point's
      sight box. A slide across a lane leaves the seat in none, and a seat
      already in one is not shoved "into" it - that is the breaker's arm,
      at the breaker's range, not a reason to shout from the rim.
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
      `sonic_ai_breaker_px` (144, about seven tenths of the reach, so the
      shove there is about half the shove at the pivot).
   5. **Pin the frog** (`"frog"`): a hunter (`Brain::hunting_frog`) with
      its quarry in the cone, not stunned. A hammer cannot hurt a frog; a
      stunned one cannot hop away from the pack's shells. Bounded: seven
      blasts, then shells.
4. **Hold for the timer**: a fire arm that matched while `Ai::fire_timer`
   runs holds facing it instead (`act_special`).
5. **Close in**: a closer (`HammerSense::spot` set) under
   `enemy_flee_damage`, not a guard that holds, whose seat is alive,
   within `attack_range` and in its line of sight (not hidden from it),
   heads for its spot and holds there facing the seat (`Hold`, `why:
   "close"`), where the breaker arm takes over. The spots are the
   simulation's (`sonic::closer_spots`): the `sonic_ai_closers` (2) hammer
   tanks fighting a seat nearest it, healthy enough to close in (ties on
   slot), are its closers; the seat's spots are the four on its row and
   column `sonic_ai_breaker_px` less half a cell out, and a spot holds
   where a hull can be routed (`Grid::usable`), a blast from it at the
   seat reaches the seat's hull within `sonic_ai_breaker_px`, no tank but a
   closer stands on it and no fellow enemy but a closer stands in that
   blast's cone. In slot order each closer takes the spot that holds
   nearest it, reachable from where it stands (`Grid::connected`), where
   neither its blast nor that of a closer before it would reach the
   other's hull: across the seat from the first at the defaults - the spot
   beside it stands 45 degrees off the first's facing, inside a cone of
   46 either side, and the one across is 256 px off, past the reach - and
   beside the first under a cone narrow enough for the hulls. So two close
   in square on the seat - its
   centre in the middle of their cones - never in each other's way, and a
   closer finds a spot or is left to the tree. The rest keep their ring
   slots by the attack tier and use the hammer only when an arm above
   offers.
6. Otherwise `None`: the tree goes on (attack still lines up and settles
   but never fires the hammer; no sniping - `can_snipe_player` is false
   with a weapon its rule owns -; chase, patrol and the seek tiers as
   ever).

**A hunter carrying the hammer fights the seat** (`enemy_phase`'s
`target_of`): the hammer cannot hurt a frog, and a frog hops away from a
tank closing in on it, so a hunter chasing its quarry with it only drove
round after a frog it could never kill. It takes the seat's ring like
everyone else until its blasts are spent, and the frog arm still pins the
frog when the frog stands in its cone.

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
- **The wave** (`sonic::wave_arcs`), in `render/game.rs`'s glowing pass
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
- **Shards**: a pane the wave shatters throws eight marks,
  `BLUE_PALE` and `#F0F0F0` alternating, hashed within 40 degrees of the
  wave's line, 16-40 px over `sonic_dust_seconds`, falling as they go; the
  ordinary rubble and collapse burst come from the tile's death.
- **A hull the wave hurts** (`Event::Hit` with `HitCause::Sonic`): the
  white hit flash over the hull (`fx::Flash`) and a few dust motes in the
  `DUST` ramp and stone chips (`fx.rs`), never a shell's sparks, impact
  flash, impact burst or pool of light - a replica pushes no impact flash
  for it either (`apply_spectacle`). No `ImpactKind` of its own: the wave's
  own dust is the rest of the picture.
- **The ripple**: `Shockwave::scaled(pivot, sonic_shock)` (0.35 of a tank
  dying) through `shockwave.rs` - a weak bend and shake, none under reduced
  motion.
- **The tell** (`sonic::tell_arcs`), glowing pass: three arcs within the
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
  `full_load` (7).

## 6. Tuning

New group `sonic` (every row live unless marked), plus one row in the
enemies' group:

| Row | Default | Range | Doc |
|---|---|---|---|
| `sonic_ammo_per_pickup: i32` | 7 | 1..=40 | Blasts one sonic hammer crate loads. One per press. |
| `sonic_reload_seconds` | 0.8 | 0..=10 | Seconds between two blasts. |
| `sonic_reach_px` | 208 | 32..=480 | How far the cone reaches from the pivot (px; six and a half cells). |
| `sonic_half_angle_deg` | 46 | 5..=90 | The cone's half angle either side of the facing. |
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
| `sonic_ai_breaker_px` | 144 | 0..=480 | An enemy shouts at a seat this close in its cone, whatever lies behind it; a closer's spot stands this less half a cell off the seat. About seven tenths of `sonic_reach_px`: two closers across a seat stand twice this less a cell apart, which has to clear the reach. |
| `sonic_ai_closers: i32` | 2 | 0..=8 | How many hammer tanks close in on one seat, the nearest first; the rest hold their slots of its ring. |
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

Protocol 16 (from 15), once in the PR.

- `WeaponKind::SonicHammer`, appended to `ALL`.
- `WireEvent::SonicBlast { slot: u16, x: i16, y: i16, dir: u8 }` - the
  pivot in quarter pixels, the facing as `dir_index`; mirrors
  `Event::SonicBlast { slot, x, y, dir: &'static str }`.
- `WireEvent::Shoved { seat, vx, vy, skid: u8 }` - the skid in tenths.
- `WireEvent::Hit { .., cause: HitCause }` - what landed: `Shell`,
  `Bullet`, `Plasma`, `Laser`, `Flame`, `Tesla` or `Sonic`
  (`Event::Hit::cause`, set by `Projectile::hit_cause` and
  `HitEffects::cause`), so a picture is chosen by its cause; every cause
  but `Sonic` draws exactly what a hit drew before.
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

`simulation/sonic_tests.rs` (headless, the default 34 x 17 field, the
seat at cell (3, 6) facing east, parked enemies placed by hand):

- The weapon: `a_sonic_crate_arms_the_hammer_and_replaces_the_special_carried`
  (seven a crate, another special emptied, a refill to seven, an enemy takes
  one only on shells), `the_hammer_fires_on_the_press_and_spends_one_blast`
  (`Fired` then `SonicBlast` in one tick, none while held).
- The cone and the wave: `the_cone_shoves_an_enemy_ahead_and_not_one_beside`,
  `the_wave_reaches_far_things_later`,
  `the_shove_falls_off_with_distance_and_a_heavy_chassis_barely_slides`,
  `walls_and_towers_shadow_the_cone_and_glass_shatters_as_it_does` (iron,
  brick, a tower, glass: behind untouched, beside reached, only glass
  gone), `a_range_board_lets_the_wave_through_and_takes_nothing`.
- The skid: `a_skidding_tank_cannot_drive_and_slides_as_far_whichever_way_it_faces`
  (head-on and broadside within 3 px, a held stick ignored during it and
  obeyed after), `the_predicted_slide_is_the_skid` (`sonic::slide`
  against the stepped round), `ice_lengthens_the_skid` (`1 /
  sonic_skid_grip_floor` within 0.3), `a_shoved_enemy_is_not_counted_stuck`,
  `an_owned_hull_is_allowed_its_knock` (the room logs the knock for an
  owned seat, and the slide past the chassis's reach is taken),
  `a_knock_allows_its_slide_and_no_more` (`SeatKnock`: a pose its speed
  past the reach, the poses together the slide and no more, nothing past
  the grace, a lapsed knock adding nothing).
- What it meets: `a_lantern_is_broken_and_a_lamp_post_left_standing`,
  `grass_the_wave_crosses_stops_concealing_for_a_while`,
  `a_drum_is_thrown_onto_the_tank_behind_it`,
  `a_drum_with_no_tank_behind_it_is_thrown_along_the_wave`,
  `a_fused_drum_is_thrown_too`,
  `a_frog_the_wave_reaches_is_stunned_and_neither_hops_nor_bites`,
  `the_hammer_hurts_only_the_other_side_and_shoves_everyone` (a sonic `Hit`
  says `HitCause::Sonic`), `a_shield_soaks_the_damage_not_the_shove`,
  `a_grenade_in_the_cone_is_pushed_away`, `a_wreck_is_left_alone`,
  `a_crate_in_the_cone_is_left_alone`,
  `a_tank_shoved_into_a_portal_goes_through_and_stops_skidding`,
  `the_wave_finishes_on_the_end_screen_and_moves_nothing`.
- Determinism: `a_round_with_the_hammer_replays_bit_for_bit`,
  `the_spawn_swap_is_off_at_zero` (a drawn special swapped at share 1, a
  tank that drew none kept on shells; the swap is a hash, so the RNG
  stream is the same either way).
- The enemy: `an_enemys_tell_runs_before_its_blast` (`TellStarted`, then
  the blast `sonic_tell_seconds` later, the aim held, facing the seat),
  `a_wrecked_tanks_tell_never_goes_off`, `a_tell_holds_under_c2` (the
  commander's nudge reaches a tank with no tell, not one in a tell -
  `commanded_intent`), `the_rule_reads_where_the_knock_would_slide_the_seat`
  (an enemy tower past the seat makes it "trouble", none leaves it
  "breaker"), `a_seat_already_in_a_lane_is_not_shoved_into_it`,
  `hammer_tanks_close_in_round_the_seat_not_onto_it`,
  `closer_spots_stand_square_on_the_seat_out_of_each_others_way` (each on
  the side nearest it clear of the first's cone - under a cone narrowed to
  35 degrees beside it for a small hull, across the seat for a large one;
  across for a small one under the default cone -, a third left out, a wall
  on the only clear side leaving the second none).

`ai::hammer_tests` (the rule on a made-up `HammerSense`):
`every_arm_fires_the_way_it_names` (trouble, drum, flush, breaker, frog -
each fires facing its way, then holds while `sonic_ai_fire_interval` runs),
`the_arms_go_in_order_and_its_own_facing_first`,
`it_never_fires_where_a_fellow_enemy_stands`,
`the_generic_tiers_never_fire_the_hammer` (lined up at 200 px: the shells
tank fires, the hammer tank never),
`a_closer_closes_in_to_its_spot_and_waits_there`,
`a_tell_holds_the_tank_facing_its_way`, `glass_in_its_way_is_shouted_down`.

The sight box: at the defaults the cone's 208 px lies inside every seat's
box (+-368 x +-240 px), so no blast reaches a seat from outside its box. A
thrown drum can land past it - a drum six cells up a column goes two more,
256 px from the thrower - so `Game::hammer_senses` checks the box for every
seat on every arm, and the probe's `offbox-fire` (budget 0) counts a hammer
used on a seat from outside it, since `act_special` records
`Ai::shot_at_seat`.

Shared path and presentation:

- `sonic::tests`: the cone (`the_cone_reaches_its_reach_on_open_ground`,
  `a_blocking_cell_cuts_the_rays_that_meet_it`,
  `glass_is_struck_and_stops_its_rays`, `the_cone_never_leaves_the_field`,
  `cells_are_in_entry_order`), the drum's landing
  (`drum_landing_takes_the_nearest_tank_behind_then_the_slot`,
  `drum_landing_walks_back_off_a_tile`),
  `the_skid_is_v_squared_over_twice_the_friction`, the composers
  (`the_wave_is_on_the_grid_in_its_ramp_and_gone_by_its_end`,
  `the_wave_stops_at_a_walls_face`, `the_tell_is_on_the_grid_and_pure`) and
  `pyro::block_arc` (`block_arc_stays_on_the_grid_and_within_its_angles`,
  `block_arc_draws_a_whole_turn_without_a_seam`).
- `pickup::kind_tests::every_kind_is_named_as_serde_spells_it`;
  `hud_tests::the_trigger_readout_is_the_special_carried_else_shells` (the
  hammer's slot, colour and symbol); `fx_tests::a_sonic_hit_starts_no_fire`
  (no spark, ember, smoke or impact burst; the flash and the dust);
  `grass::tests::a_pinned_tuft_lies_flat_until_the_pin_runs_out`;
  `fish::tests::{a_wave_scares_the_fish_it_passes,
  a_shout_at_the_shore_throws_a_fish_onto_the_bank_and_back,
  no_fish_is_thrown_off_the_map}`;
  `indicator_tests::{a_tell_off_the_screen_has_an_arrow_whatever_the_cap,
  through_the_round_a_sonic_hit_arcs_toward_the_blast}` and
  `picture_tests::every_kind_of_arrow_wears_its_colour` (the wind-up's
  accent rimmed hostile);
  `devserver::tests::set_tank_arms_the_hammer_and_spawn_pickup_drops_its_crate`
  and the PICKUP category's count (14); `chrome_tests` with the wrapping
  palette; the armory's thumbnail pin and its place in `SUPPORTED_MAPS`;
  `text_tests`' budgets.

Wire:

- `events.rs`: the samples carry `SonicBlast`, a sonic `Hit`, `Shoved`
  with a skid and `DrumLaunched` with a drum; `tell_started` on
  `NOT_SENT`.
- `apply.rs`: `a_sonic_blast_reaches_the_replica` (the same cone from the
  same pivot, run out on the replica's clock),
  `a_sonic_blast_this_client_drew_is_not_drawn_again`,
  `an_enemys_tell_reaches_the_replica`, `a_stunned_frog_reaches_the_replica`,
  `a_sonic_hit_flashes_no_impact_on_the_replica`, and the beam tests on
  `presses`.
- `predict.rs`: `a_sonic_blast_is_drawn_on_the_press` (from the predicted
  pivot, owed until its `Fired`, claimed once), `a_knock_skids_the_owned_hull`.
- `rig.rs`: `a_seats_blast_reaches_the_replica_once` and
  `an_enemys_hammer_reaches_the_replica` (the tell, then the wave) through
  `Lockstep`; `a_seats_blast_is_drawn_on_the_press_and_only_once` through
  the threaded rig and a whole `OnlineRound` (the seat drives onto the
  crate, presses: one wave on that frame, never two).
- `delta.rs`: the random snapshots and the size bounds (a full snapshot
  456 B, two bytes a tank more).
- The room server's `cargo test -p bongbong-server`, its dev tools too.

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
- Three more deliberate states, which a pack that shoves an AFK seat about
  rather than killing it shows only because its rounds outlast the
  budgets: a guard keeping its beat while the seat is far from its frog
  (`TankSnapshot::guarding`) is no never-arrived, as a leashed or sleeping
  tank is not; a hunter carrying the hammer, which fights the seat rather
  than the frog, is leashed and woken as any tank (`leashed`,
  `field::mind`); and a hold within the last half second
  (`HOLD_REACTION_FRAMES`) vetoes stale-start, since a tank that held a
  firing solution at its spawn until its seat was knocked off the line is
  reacting, not stale. Each only exempts, and the defaults' recorded counts
  of both kinds are 0.
- **The bar**: every crate run within the defaults' ceilings, `offbox-fire`
  0. An exceedance is read round by round from its `ANOMALY` lines; one the
  hammer's own action causes - a tank stranded, spinning, piling up or
  firing from off the box - is fixed, not re-baselined.

### Results (debug build, seed 1000, the recipes' budgets)

Measured at the hammer's first numbers (six blasts a crate, 1.2 s
between them, a 160 px reach, 35 degrees either side), not re-run at the
current ones; the defaults' rows do not depend on them.

Fixtures: the nine `maps/test/` maps, ten 30 s rounds each. Fields: the
seven field maps, ten 60 s rounds each. Anomaly totals (kinds at 0 left
out); mixed is `enemy_special_weapon_chance` 0.5 with
`enemy_special_weapon_sonic_share` 0.5:

| Sweep | Result | Totals |
|---|---|---|
| Fixtures, defaults | pass | border-stuck 4, churn 34, clustering 10, jitter 32, pile-up 6, spin 3 |
| Fixtures, `--crate` | pass, the same | the fixtures carry no weapon crate, so `--crate` swaps nothing |
| Fixtures, mixed | over on four of nine | border-stuck 1, churn 61, clustering 15, jitter 38, pile-up 12, spin 5, tank-grind 1 |
| Fixtures, armed | over on all nine | border-stuck 16, churn 200, clustering 69, jitter 138, never-arrived 2, pile-up 30, spin 15, tank-grind 1 |
| Fields, defaults | pass | border-stuck 11, churn 83, clustering 12, jitter 108, pile-up 8, spin 24, wall-grind 1 |
| Fields, `--crate` | over on two of seven | border-stuck 12, churn 96, clustering 30, jitter 137, never-arrived 1, pile-up 17, spin 25, wall-grind 2 |
| Fields, mixed | pass | border-stuck 15, churn 83, clustering 7, jitter 102, pile-up 6, spin 16, wall-grind 2 |
| Fields, armed | over on four of seven | border-stuck 21, churn 196, clustering 95, jitter 213, low-progress 5, never-arrived 5, pile-up 69, spin 58, tank-grind 8, wall-grind 4 |

`offbox-fire` is 0 in every sweep. The defaults are the recipes' own,
unchanged: a round without the hammer runs exactly as before.

**Why the ceilings are the wrong yardstick for an armed pack.** The
recipes' ceilings were measured on rounds an AFK seat loses in 4 to 26 s.
A pack that shoves the seat about rather than killing it keeps the round
going to the frame cap, and every kind but `invariant` is counted per
tank per stretch of time, so the same AI over three times the time trips
them. The fair yardstick is a shells pack over as long: the same sweeps
with `--mission destroy` (no frog to lose the round by) and
`player_armor_factor` 0.1 (a seat that outlives the cap), with and
without the hammer. Totals over the same minutes of play (fixtures 44,
fields 67 and 70):

| Sweep | spin | stall | stale-start | never-arrived | tank-grind | low-progress |
|---|---|---|---|---|---|---|
| Fixtures, shells pack, long | 22 | 0 | 0 | 7 | 12 | 8 |
| Fixtures, hammer pack, long (as built) | 16 | 5 | 0 | 4 | 9 | 10 |
| Fixtures, hammer pack, long (review) | 24 | 0 | 0 | 3 | 7 | 8 |
| Fields, shells pack, long | 44 | 1 | 0 | 1 | 18 | 8 |
| Fields, hammer pack, long (as built) | 45 | 18 | 0 | 7 | 9 | 4 |
| Fields, hammer pack, long (review) | 57 | 1 | 0 | 2 | 18 | 9 |

So a pack carrying the hammer strands, stalls and grinds no more than a
pack of shells over the same time; spins on the field maps run a fifth
higher (0.81 a minute against 0.66), the closers chasing a seat their
partner keeps shoving to a new spot. Read round by round (`--log-every
1` and the AI's tier and arm each frame):

- **The hammer's own bugs, fixed in review.** Every armed stall was a
  closer holding "close" where it could never fire: at a spot drawn in
  from its ring slot that stood in a wall's shadow, past a fellow enemy
  or in the other closer's cone, or that another tank held. Most grinds,
  low progress and a third of the spins were closers dithering round a
  spot another tank stood on, backing away from a seat they already stood
  next to, or crossing the other closer's path. The closers' spots are
  the simulation's now (§4, `closer_spots`). Armed, before -> after:
  fixtures spin 19 -> 15, stall 1 -> 0, tank-grind 5 -> 1, low-progress
  4 -> 0; fields stall 11 -> 0, tank-grind 21 -> 8, spin 61 -> 58.
- **The probe misreading a deliberate state** (§11 above): never-arrived
  were guards keeping their beat by the enemy frog (archipelago) and
  hammer hunters, which fight the seat, read as never leashed (hedge-maze);
  the one stale-start was a tank holding a firing solution at its spawn
  until its seat was knocked off the line two frames before the check.
  Armed fields never-arrived 11 -> 5, the five left the field AI's own
  in a long round (below).
- **The tree's own tiers in a long round**, the same in the shells
  pack's long rounds: chase, flee and patrol loops (flee spins in a dead
  end, a chaser running north and south beside choke's wall, patrollers
  on hedge-maze's far side), the attack tier's ring repositioning. About
  three in four of the remaining spins and nearly every grind and
  never-arrived are these; the rest are closers following a seat their
  partner just shoved.
- **The skid** strands nothing: the stuck and breach clocks stand still
  while a hull slides, a slide ends at the first wall, and no anomaly
  read was a hull mid-skid.

What stays over the ceilings with the hammer in play is crowding and
weaving - churn, jitter, clustering, pile-up - from a pack that fights
at 100 px rather than 300: two closers on a seat shove it to and fro and
follow it, the rest stand on its ring. **With the crate** two field maps
go over as they did as built: archipelago's jitter by one (31) and
harbor-lights' jitter (34), clustering (12) and one never-arrived, a
shells tank fighting on the far side through a 48 s round; one or two
hammer tanks keep the AFK seat alive longer, and the jitter is mostly the
guard and chase tiers' (the hammer's tier is in six of the 65 jitter
events on the two maps). How such a pack should play is a question for Oto (§12).

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
| Range boards (docs/range-target-prd.md) | Passed over as a fence is, and left whole: a board stops no sound and the wave breaks only glass; what stands behind one is shoved |
| Drums, oil, ground fires | Drums thrown and going off where they land (fused or not); oil and fires untouched; a thrown oil drum leaves its pool |
| Trees, grass | Trees sway and stop nothing; grass flattened and hides nobody for a while |
| Glass, lanterns, lamp posts | Shattered; broken; left standing |
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
   shatter and grass falls in a sweep - and a far target has about a third
   of a second to see it coming. Rejected: instant (cheaper, but the picture
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
   through the generic tiers; it fires by its rule, two per seat close in,
   and the rest keep their places on the seat's ring. The probe chose the
   two: one brawler left the others standing at range with a weapon they
   could not use (stalls, rounds where a tank never came within range);
   every hammer tank closing in piled them round the seat (clustering and
   pile-ups doubled); two closers measured best (§11), at spots the
   simulation hands them square on the seat (decision 24). Rejected: letting the
   generic attack fire it at 340 px (wasted), and letting an enemy fire
   shells while it carries it (breaks the one-trigger rule). A visible
   change in how such an enemy plays: while it carries the hammer it is a
   close-range threat only - it never fires the hammer at range and fires
   no shells - until its seven blasts are spent.
9. **A hunter carrying the hammer fights the seat**, pinning the frog only
   when the frog stands in its cone - seven blasts, then shells and the frog
   again. *For Oto.*
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
17. **A sonic hit says so** (`HitCause` on `Event::Hit` and the wire)
    rather than being a new event: everything that reads a hit - the round's
    stats, the hit arc, the hull flash, the fish - keeps reading it, and
    only what would draw fire asks the cause.
18. **Lamp posts stand**: a lamp post's light is a rule, and putting it out
    is the EMP's. Only glass breaks by sound; a seat's lanterns, which a
    blast breaks too, break.
19. **Crate ink sky blue** (`#46C3F2`). *For Oto*, with a screenshot of the
    crate beside the other twelve in Phase 2.
20. **The builder's palette wraps**: the fourteenth pickup made the folded
    TOOLS palette's PICKUP row wider than a phone's room (72 + 14 x 48 pt
    against 704). A category longer than the room is wide runs on into a
    second row, and the cells shrink to fit the room's height, never under
    a finger's 44 pt. Rejected: paging the palette (a hidden page on a
    touch screen) and a second PICKUP category (two names for one kind of
    tool).
21. **`Tank::special` beside `active_weapon`**: one question per reader -
    what a tank carries or what its trigger fires (§3.0) - so the BB-36
    weapons that carry without firing need no second rule.
22. **A pack of hammer tanks juggles**: two closers a seat, the rest on
    its ring, measured best of the four rules tried (§11), but a whole pack
    armed with it still crowds a seat and keeps an AFK one alive two to
    three times as long. Rejected for now: letting the tanks that are not
    closing in fire shells (the one-trigger rule), and one closer (more
    crowding on the ring, not less). *For Oto*: whether a pack should play
    so, before the share knob goes above 0 anywhere.
23. **The crate's blue sits between plasma's teal and the tower pack's
    periwinkle** in the crate row (screenshot in the PR); *For Oto* if it
    reads too close to either.
24. **A closer's spot is the simulation's, square on the seat** (review):
    the Brain drew its spot in from its ring slot, which put it on a cell
    another tank held, behind a wall's shadow, past a fellow enemy or across
    the other closer's path - the armed runs' stalls (a closer holding
    "close" where it could never fire), grinds and most of its extra spins.
    `closer_spots` sees the whole world: it offers only spots from which a
    blast reaches the seat, with nobody on them or in their cone, and keeps
    the two closers out of each other's cones (across the seat from each
    other at the default cone; beside each other for small hulls under a
    narrower one). `sonic_ai_breaker_px` moves with `sonic_reach_px`, at
    seven tenths of it: the spots stand that less half a cell out, and two
    across the seat stay out of each other's reach only while twice that
    clears the reach. A seat square in the cone is
    also the shove a player reads best: straight along a row or a column.
    Rejected: keeping the ring-slot spot and checking it (it still crosses
    the other closer's path), and a spot behind the tank on its own bearing
    (two closers on one side end in each other's cones).
25. **"Trouble" is where the slide comes to rest** for a lane or a tower's
    reach (review): a slide across a lane leaves the seat in none, and a
    seat already in one is not shoved into it - before, a seat lined up for
    any enemy was "trouble" from the cone's rim, so hammer tanks shouted
    from the rim at seats they were only breaking off.
26. **A knock's allowance is a distance, not a window** (review): the
    validator gives each pose the knock's speed past the chassis's reach,
    but all of them together only the knock's slide on the slipperiest
    ground - a modified client gains one slide's worth of ground per knock
    the room gave it, never two seconds of free speed - and adds the
    chassis's own motion to the knock's rather than taking the larger, so a
    seat knocked while fleeing the shooter is not snapped back.
27. **The wind-up arrow is the weapon's accent rimmed hostile red**
    (review): the accent names the weapon, and the red rim, the enemy
    frog's, says it is no teammate - the hammer's sky blue is player 1's
    own colour.
28. **A range board lets the wave through** (docs/range-target-prd.md): it
    is a prop - a plank on an easel, seam-closed like a fence - so the wave
    passes it as it passes sandbags and fences, and breaks nothing but
    glass, so the board takes nothing. *Alternative*: stopping the wave as
    the board stops sight, which would make every board a sound wall - but
    `blocks_sight` is the AI's line of fire, and a board is no wall.

### Not in this PR

- Placing the hammer in the shipped levels - level design, a follow-up.
- A "SPOTTED" label over a seat the wave uncovers (the armory scene's) -
  the flattened grass is the reveal; a word over the field is HUD design.
- A tell's pulse on the minimap - the arrow is the warning; the minimap
  shows terrain and marks, a follow-up if play-tests want it.
- Sound effects - the game has no audio yet.
- Breaking lamp posts - a lamp post's light is a rule
  (`lamp_reveal_px`), and putting the lights out is the EMP's (BB-38's "at
  11"); the hammer leaves them standing.
- Throwing the rubble in the cone again, as a blast does - unasked for, and
  it would change round state (`Game::decals`) for a cosmetic.
- Shoving wrecks, deflecting shots in flight, putting fires out, breaking
  crates, stunning towers (the EMP's) - each would be a mechanic of its own
  beyond the issue.
- A probe scenario in which the seat fires its special - the AFK and
  advance scenarios measure the enemies, which is what the probe is for.
