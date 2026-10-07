# EMP burst

BB-38, the second of the six weapons of BB-36. A special weapon from its own
crate (`pickup = "emp_burst"`), one at a time like every other
(`Tank::take_weapon`): a crate loads `emp_charges_per_pickup` (3) pulses, a
re-pick refills to three, another weapon's crate replaces it. Seats and
enemies alike: an enemy takes the crate while it fires shells
(`Tank::wants_pickup`) and uses it by its own rule (§4).

A ring of pale blue blocks runs out five cells from the hull, and
everything electric it crosses dies for a while: an enemy's brain stops and
the tank coasts on its last intent, turrets sag, headlights and the light
layer go out, live rainbow shields pop, towers go offline, missiles in the
air fall dead. The frog does not care - it is organic. The cost: the
shooter's own special goes offline for as long, and its HUD says
`WPN OFFLINE`. At night the pulse puts out every lamp post on the map for
ten seconds. Counter: range - keep five cells from a tank that just took a
crate.

The sonic hammer's doc (docs/sonic-hammer.md §3) is the shared path this
plugs into: the AI hook, the enemy tell, the online press show, the
probe's `--crate`, the spawn swap, `spawn_pickup` and the armory. This PR
adds three things the later weapons build on (§3.3): the general
**disabled** state of a tank and a tower, which the FPV drones (BB-40) and
the gravity well (BB-42) read; the split of what a tank **carries** from
what its trigger **fires** (`Tank::special` and `Tank::active_weapon`);
and the AI's **dangers**, the places an enemy keeps out of, to which the
rail's lane (BB-39) and the rod's circle (BB-41) add their shapes.

## 1. How it plays

### The pulse

- **One per press.** The trigger fires on the press edge, like a shell
  (`drive_player`'s `fire_pressed`). A seat's press goes off at once - no
  wind-up. An enemy's goes off after its tell (§3.2 of the hammer's doc,
  `emp_tell_seconds`).
- **Its cost, which is also its reload.** The press takes the shooter's
  own special offline for `emp_disable_seconds` (3):
  `Tank::special_offline` (§3.3). While it runs the trigger fires shells,
  exactly as it does once a special runs dry (`Tank::active_weapon`), and
  the HUD's weapon readout says `WPN OFFLINE` (§5). There is no second
  reload knob: a second pulse needs the first's offline over.
  `fire_cooldown` is set to `player_fire_interval`, the shell's own pace,
  so the press is not followed by a shell on the next frame.
- **The ring.** From the hull's centre (`Tank::position`, the pivot), all
  round, out to `emp_radius_px` (160, five cells). Measured to the
  **nearest point** of what it strikes: a hull's box
  (`Tank::hull_bbox_world`), a tower's cell box (32 x 32), a missile's
  ground point. A big hull half inside the ring is reached by its nearer
  edge. The shooter is never reached.
- **Nothing stops it.** Walls - brick, iron, wood, glass -, towers, trees,
  water, lava and tanks let it through: an EMP is not sound, and its
  counter is range. It is not cut at the field's edge (nothing stands
  there but rolling-in tanks, which are not on the field and are never
  struck), and it does not pass through portals (it is not a shot).
- **The ring travels.** What the ring reaches, it strikes when its front
  gets there - the sonic wave's rule, so the picture and the effect agree:
  the front leaves the pivot at the press and runs out at
  `emp_ring_speed` (800 px/s - the full radius in a fifth of a second).
  Each frame (`Game::tick_emp_pulses`) everything whose reach distance
  falls between the front's last radius and its new one is struck, once.
  The pulse is done with once its front is at the radius; its picture
  lingers `emp_ring_seconds` (0.4) longer.
- **Events**: `Event::Fired { weapon: "emp_burst" }` then
  `Event::EmpPulse { slot, x, y }` (the pivot) on the press - in that
  order, in the same tick, which the online claim reads (hammer §3.3) -
  then `Event::Disabled { slot, x, y }` per tank and
  `Event::TowerDisabled { x, y }` per tower the front reaches, the
  ordinary `ShieldBroken` per shield it pops, and `Event::MissileDud
  { x, y }` where a missile it killed lands.

### What "everything electric" means

The front strikes, in this order: tanks (the seats in index order, then the
enemies by slot), towers (cell order), missiles (id order).

- **A tank** - live, with a body, on the field (not rolling in through a
  gate), not the shooter, **of either side** (§12, decision 3) - is
  disabled: `Tank::disable(emp_disable_seconds)` (§3.3) sets
  `Tank::disabled` to at least that (a second pulse refreshes it, never
  adds), and on that frame:
  - **its live rainbow shield pops**: `shield_hp`, `shield_timer` and
    `shield_recharge_delay` to 0 and `shield_broke` set, so
    `drain_shield_breaks` logs the ordinary `ShieldBroken` and its ripple
    later in the same frame. A shattered shield never comes back
    (`Tank::tick_shield`'s rule), so this is not "for a while": it is
    gone, as one spent is. The heat shield is a coat, not a field, and
    stays (decision 8).
  - **its tell lapses** (any weapon's: a hammer's wind-up, and the later
    weapons' charges and call-ins): `Tank::tell = None`, no `Fired`.
  - **its special goes offline** for as long: while `disabled > 0`
    `Tank::active_weapon` is the shell cannon whatever it carries, and
    what it carries is kept (decision 5). A minigun burst and a missile
    volley in progress stop short (`minigun_burst`, `missile_volley` to
    `None`) keeping the rounds they had not fired - both are counted a
    round at a time; a twin plasma gun's second bolt (`pending_plasma_shot`,
    paid on the press) fizzles; a held flamethrower's jet goes out
    (`flame_held = false`) and the held trigger then fires nothing until
    the next press, which is a shell. A twin gun's second **shell**
    (`pending_shot`) still leaves: shells are not electric.
  - **its lights go out** for as long: no headlight cones, no spotlights,
    no hull glow (`weather::lights_in`), no light layer
    (`tank::draw_tank_glow`), the lamps drawn dark by day (§5), and its
    hull crackles with sparks the whole time (§5).
  - **an enemy's brain stops** (below). A seat's does not - a seat has a
    driver (below, "On a seat").
- **A tower** - standing, either side - goes offline for
  `emp_tower_seconds` (6): `Tower::disable` sets `Tower::disabled` to at
  least that and on that frame drops what it was doing - the tesla's
  charge to 0 and its target (the charge is lost: a coil discharged), the
  gun tower's burst in progress (`burst_left` 0), the bio slush's aim.
  While `disabled > 0`, `tower_phase` runs its upkeep (a burning tower
  burns on: fire is not electric) and nothing else: no charge, no turn,
  no burst, no lob, the bio slush's reload held where it stands. Its
  lights (the tesla's coil, the mortar's loaded mouth, `TOWER_GLOW_COL`)
  are out and its top layer sags (§5). Repairing it with a tower pack
  mends its health and does not bring it back sooner (decision 19).
- **A missile** in the air - either side, its ground point within reach,
  any stage - dies: `Missile::kill` puts it in `MissileStage::Dead` - no
  lock, no tracking, no exhaust, no trail. It keeps its ground heading,
  losing speed (`emp_missile_drag` per second, exponential), and falls
  from the height it had (`emp_missile_gravity`, from rest). Where it
  reaches the ground it is a **dud**: `resolve_missiles` removes it,
  logs `Event::MissileDud { x, y }` (a puff of dust and a few sparks,
  §5) and nothing else - no blast, no damage, no shove, no scorch
  (decision 6).
- **Lamp posts, at night** (the "at 11"): a pulse fired while the sky in
  force is night's (`Game::is_night`: `Weather::Night` or `Storm`, which
  is what `nightfall` turns a round into) puts out **every lamp post on
  the map** at once for `emp_lamp_seconds` (10): `Game::lamps_out`. While
  it runs, `Game::lit_lamp_posts()` is empty - the posts still stand, take
  shots and block as tiles; they throw no light (`weather::lights_in`)
  and light nobody up for an enemy (`Game::is_lit`, so `Game::sight_on`
  falls to the night's range by them). A second pulse refreshes it. By
  day a pulse does nothing to them: they are not lit by day.
- **What it leaves alone**: the frogs (both sides - organic), lanterns
  (a flame, not a circuit - the seats' light when the posts go dark),
  crates and breakable crates, portals, grenades (a burning fuse, its lamp
  that fuse's glow), shells, bullets, plasma bolts and laser beams already
  fired, flying drums, lava bombs and globs, drum fuses, ground fires,
  oil, ooze, the volcano and the lava, water and ice, trees, tall grass,
  tiles of every kind (a range board too), wrecks (their embers glow on),
  fish, tread marks.

### An enemy struck: brain off

- **It coasts on its last intent with the trigger released** - the
  far-tank `Coast` of `simulation/field.rs` is the model: in
  `enemy_phase`'s collect pass, a tank with `disabled > 0` takes the branch
  before `field::mind` and drives `Intent { fire: false,
  fire_aim_offset: 0.0, ..ai.last_intent() }` - the heading it was
  driving keeps it driving, a hold keeps it holding - through
  `tank.control`, `tick_queued_shots` and the deferred drive, like a far
  tank between thinks. It does not think: no `Ai::think`, no tree, no
  steering, so it does not brake for the tank ahead (the personal-space
  brake is the tree's), does not route round anything and can drive into
  a wall, a ford, the lava's bank or another tank and ram it.
- **Its brain is frozen, not paid back.** None of `Ai`'s clocks run while
  it is off (`fire_timer`, `hit_alert_timer`, `dir_hold`, the dodge, the
  snipe, the grudge, the breach): they pick up where they stood. There is
  no `think_debt`: the first think after covers its own tick only. A hit
  while it is off still calls `Ai::notify_hit`, so it reboots knowing it
  was shot.
- **The reboot.** The first tick it is no longer disabled,
  `Ai::reboot()` clears what the coast made a lie of - the stuck clock's
  baseline (`last_position`, `progress_avg`, `stuck_timer`, `motion`),
  the heading commitment and `dir_hold`, the dodge, `yield_timer`,
  `wall_ahead_timer`, a latched breach and `aim_settle` - and keeps what
  is about the fight: the role, alerts, the target seat, the fire timer,
  retreat, the escape count. So a tank that coasted into a wall is not
  "stuck" the moment it wakes, and does not re-latch a breach on evidence
  it never gathered. `Ai::down` marks the brain off for tooling
  (`AiSnapshot::down`) and is what `enemy_phase` reads to call
  `reboot` once.
- **Its senses and its radio are dead.** While disabled it neither spots
  a seat for the arena's shared alert nor sees, relays or receives on a
  field map's alert chain (`Game::field_alerts` treats it as it treats a
  wreck, its own alert still ageing), is skipped by the retarget pass
  (`Ai::target_player` holds), and takes no order from the commander
  (§4). It keeps its engagement slot (the ring is sticky; a three-second
  outage is not worth a reshuffle of the pack) and stays in the motion
  snapshot others avoid by.
- **A field map's far tank** that is disabled coasts the same way and is
  neither woken nor counted as lost by `field::mind` (it is not called).
  A disabled tank is never re-rolled as a straggler
  (`Game::reroll_stragglers` skips it).
- **Its turret sags** (drawn only, §5) and its sparks crackle until it
  reboots.

### On a seat (an enemy's EMP, or a teammate's)

A seat is not an AI, so its brain is not switched off. A seat the front
reaches is disabled the same way and loses, for `emp_disable_seconds`:

- **its special weapon**: offline - `Tank::active_weapon` is the shell
  cannon, the stock is kept, a burst, a volley or a flame in progress stops
  (above). **The stick and shells still work**: it drives, turns and fires
  shells from its magazine, sets lanterns down and picks crates up. A
  crate taken while disabled is offline too until the timer runs out.
- **its shield**: popped, `ShieldBroken`, gone.
- **its headlights and its light layer**: out, so at night its screen
  goes dark round it but for its sparks, its lanterns and whatever else
  burns.
- **its side's towers in the ring**: offline for `emp_tower_seconds`, as
  every tower in the ring is.

Its HUD's weapon readout flickers `WPN OFFLINE` while it carries a special
(§5); with no special there is nothing to take offline, and the readout
shows the shells as ever. Its turret does not sag - the gun is a seat's,
and it still fires.

### The disabled state, for the weapons after this one

Two timers on `Tank` and one on `Tower`, each with one setter (§3.3):

| Field | Set by | What reads it |
|---|---|---|
| `Tank::disabled: f32` | `Tank::disable(seconds)` (the EMP; later the drones' and the well's reactions decide their own) | `Tank::active_weapon` (offline special), `enemy_phase` (brain off, coast, no spotting), `Ai::reboot`, `weather::lights_in` and `render/game.rs` (lights out, sparks), the turret's sag, `indicators` (no lane warning), `hud::WeaponSlot`, the C2 commander (never ordered), `Game::reroll_stragglers`, the probe's holds, the wire (`TankState::disabled`) |
| `Tank::special_offline: f32` | the EMP's own press (`fire_emp`) | `Tank::active_weapon`, `hud::WeaponSlot`, the module's offline cell, the predictor, the wire (`TankState::offline`) |
| `Tower::disabled: f32` | `Tower::disable(seconds)` | `tower_phase` (no weapon), `Game::tower_views` (`TowerView::disabled`), the tower's drawing and light, `Game::player_tower_reach` (an offline player tower is no detour), the hammer's trouble cells, the EMP's own AI value, the wire (`tile_flags::DISABLED`) |

`Tank::special_down()` is `disabled > 0 || special_offline > 0`: the one
predicate "the special does not fire now". The FPV drones read
`special_down` to keep a disabled tank from launching, and their own
drones in a pulse's reach fall dead through the strike walk's arm they add
(`tick_emp_pulses`, after the missiles); the well collapses early through
its own arm there. Neither needs a new timer on `Tank`.

### On the end screen

The pulses already out finish their picture (`tick_emp_pulses(f,
false)`): nothing is struck, as a blast on the end screen hurts nobody.
The timers already running keep counting down (`tick_timers` runs on both
branches), so sparks fade and lamps come back on as the restart counts
down.

## 2. Where it lives

| File | What |
|---|---|
| `src/emp.rs` (new) | The weapon's headless half. `EmpPulse` (origin, owner, age, `swept` radius, `live`, the slots `struck`), `box_reach(origin, centre, half) -> f32`, `seat_value` (the AI's pure scoring, §4), the composers `ring`, `sparks`, `tell` and `lamp_sparks` (pure, `pyro::Shape`s) with `sparking`/`spark_seed` (when a hull's sparks fly, the light `weather.rs` gives them) and `lamp_lit` (a post's flicker back), `droop_side(slot) -> f32`, `module_cell` |
| `src/simulation/emp.rs` (new) | The world half. `fire_emp` (the dispatch arm's body), `resolve_emp` (the frame's presses into pulses, the lamps at night), `tick_emp_pulses(f, live)` and its one strike walk `strike_emp` (tanks, towers, missiles, in that order), `tick_emp_pictures` (a replica's rings), `emp_show` (the cosmetic half - the ring on the list, the ripple, the module's flash - which a replica's `EmpPulse` and a client's press call too), `EmpSeat` and `emp_senses` (what the AI is handed, §4), `emp_dangers` and `danger_route_cells` (§4), `seat_emp`, `any_emp`, `Game::{is_night, lit_lamp_posts, lamps_out}` |
| `src/simulation/weapons.rs` | The `ActiveWeapon::Emp` dispatch arm |
| `src/simulation/mod.rs` | `Frame::pending_emp`; `Game::{emp_pulses, lamps_out}`; the phase calls (`resolve_emp` and `tick_emp_pulses(f, true)` after the hammer's, before `step_world`; `tick_emp_pulses(f, false)` on the end screen); the disabled coast branch, the reboot, the spotters and the retarget pass in `enemy_phase`, and the dangers it builds; `Pending::{disabled, clearing}`; `tick_timers` (`disabled`, `special_offline`, `emp_flash`, `lamps_out`); `tick_presentation` (the cosmetic pulses, the tanks' and towers' timers, `lamps_out`); `Event::{EmpPulse, MissileDud, Disabled, TowerDisabled}`; the swap's table entry (hammer §3.4) |
| `src/simulation/field.rs` | `field_alerts`: a disabled tank neither sees nor relays nor receives |
| `src/simulation/waves.rs` | `reroll_stragglers` skips a disabled tank |
| `src/simulation/towers.rs` | `tower_phase`'s offline branch; `tower_views`' `disabled` and `droop`; `player_tower_reach` leaves offline towers out |
| `src/simulation/missiles.rs`, `src/missile.rs` | `MissileStage::Dead`, `Missile::{kill, dud}`, the fall; `guide_missiles` skips the dead; `resolve_missiles`' dud; `Game::missiles` carries `dead` |
| `src/simulation/sonic.rs` | The swap's table entry; the hammer's interactions (§12): `hammer_senses`' trouble leaves offline towers and disabled enemies out |
| `src/simulation/nav.rs` | `route_grid_on` surcharges the cells inside a seat's danger (`enemy_danger_route_cost`) |
| `src/simulation/command.rs`, `src/simulation/comms.rs` | `UnitView::{busy, clearing, dodging}` with `command::Busy` (`Disabled`; the later weapons add their states), the `clear_rings` producer (issues the existing `Order::Nudge`), `deconflict`'s two rules for a busy unit and the already ordered, `gives_way`'s first step (a busy unit never gives way), `Skipped::deaf` |
| `src/tank.rs` | `emp_charges`, `disabled`, `special_offline`, `emp_flash`, `droop`; `ActiveWeapon::Emp` (`name`, `full_load`, `tell_seconds`), `SPECIAL_WEAPONS`; `special`, `active_weapon`, `special_down`, `disable`, `kick_emp`, `ease_droop`; `weapon_ammo`/`take_weapon`/`empty_stock`/`wants_pickup`; the module's cells in `module_cols`, the sag in `turret_placement`, `draw_tank_dark`, `draw_tank_glow`'s skip |
| `src/tower.rs` | `Tower::{disabled, droop, disable}`, `TowerView::{disabled, droop}`, `draw_tower`'s glow off and sag |
| `src/pickup.rs` | `PickupKind::Emp` (`emp_burst`, row 14, its ink, spills) |
| `src/ai.rs` | `SpecialSense::Emp(EmpSense)`, `emp_rule`, `generic_fire(Emp)`, `SpecialUse::Clear`; `Danger`, `DangerShape`, the `dodge` tier, `Brain::{danger_here, out_of_danger, way_out, can_reach, steer_out, in_danger, close_spot}`, `act_attack`'s wait outside a danger, `nearest_pickup` leaving crates inside a danger out; `Ai::{down, clearing, dodging, dodge_exit, kept_out, reboot}`; `SEEK_SPECIALS` gains the EMP; `hammer_rule`'s closer keeps out of dangers; `AiSnapshot::{down, dodging, kept_out}` |
| `src/lamp.rs` | `draw_post`'s dark glass and its flicker back; `lamp_sparks` |
| `src/weather.rs` | `lights_in`: a disabled hull's spark light for its headlights and glow, an offline tower's lights out, dark posts, a dead missile's exhaust out; `Game::is_lit` reads the lit posts |
| `src/hud.rs` | `HUD_EMP_COLOR`, the `weapon_color`/`weapon_pickup` arms, `WeaponSlot::offline`, `WEAPON_SLOT_W`, `offline_lines` |
| `src/render/hud.rs` | The offline readout; `corner_tests` pins `WEAPON_SLOT_W` to the slot table |
| `src/render/game.rs` | The rings, sparks, tells and lamp sparks in the glowing pass; the dark lamps over a disabled hull in the lit pass; the light layer skipped; the dev stats arm |
| `src/render/missile.rs` | A dead missile's sprite with no exhaust, nose tipping into the fall |
| `src/fx.rs` | A dud's dust and cobalt sparks off `MissileDud` (particle bursts, no `ImpactKind` of its own), a splash where it lands wet |
| `src/indicators.rs` | `Scene::of` reads each tank's outage beside its facing: no lane warning from a disabled enemy |
| `src/pyro.rs` | The `EMP` ramp |
| `src/net/wire.rs` | `WeaponKind::Emp` (`drawn_on_press`), `TankState::{disabled, offline, shells}`, `MissileState::dead`, `tile_flags::DISABLED`, `RoundState::lamps_out` |
| `src/net/events.rs` | `WireEvent::{EmpPulse, MissileDud}`, the `press_show` arm, `Disabled` and `TowerDisabled` on `NOT_SENT` |
| `src/net/encode.rs`, `src/net/apply.rs` | The new fields; `EmpPulse`'s show; the `kick_turret` arm |
| `src/net/predict.rs`, `src/net/round.rs` | `PressShow::Emp`, `Predictor::offline_left`, the shown seat's offline |
| `src/simulation/present.rs` | `draw_press_show(seat, weapon, origin, facing)`, the press shows by weapon (the hammer's wave, the EMP's ring) with the seat's module kicked; `hold_seat_offline(seat, seconds)`, the predicted offline on the drawn seat |
| `src/simulation/replica.rs` | `DrawableTank::{disabled, offline}`, `DrawableMissile::dead`, `DrawableTile::offline`, `DrawableState::lamps_out` - booleans, so the room and a replica that runs its timers down agree |
| `src/simulation/debug.rs`, `src/devserver.rs` | `set_tank`'s `emp_charges`, `disabled`, `special_offline`; the snapshot's `emp`, `disabled`, `offline`, `down`, `dodging`, `towers` (`TowerDebug`), `lamps_out`, the rings as `emp_pulse` projectiles |
| `src/bin/probe.rs` | The tank line's `emp=`/`dis=true`, the fire tuple; `HOLDS` gains `kept out`, and `OUT_OF_ITS_HANDS` (new) holds `disabled` |
| `src/editor/mod.rs` | `Tool::Pickup(PickupKind::Emp)` (`emp_burst`) |
| `maps/armory.toml` | Its crates and lamp posts (§3.4) |
| `src/tuning.rs` | The `emp` group (§6), two rows in `enemies` |
| `lang/en.ftl`, `lang/sl.ftl`, `src/text.rs` | §7 |
| `tools/punypalette.py`, `tools/spritegen/gen_crates.py`, `tools/spritegen/tankdesign/{kit,export,render,lines/vanguard}.py` | The art (§5); writes `static/crates_sheet.png`, `pickup_glyphs.png`, `tank_modules.png`, `tank_modules_glow.png`, `src/tank_art.rs` |
| `src/thumbnail.rs` | The armory's pin, re-baselined |
| `docs/` | This, `CRATES_SPEC.md`, `SPRITESHEET_SPEC.md`, `effects.md` (the `EMP` ramp, who draws what); `CLAUDE.md` |

## 3. The shared path

### 3.1 What this uses as the hammer laid it down

Each item of the hammer's checklist (docs/sonic-hammer.md §3.0) gets its
EMP arm, in the same places, nothing beside them:

1. `PickupKind::Emp` (`#[serde(rename = "emp_burst")]`, appended to `ALL`,
   row 14, `ink`, `cooks_off` false), `PickupKind::weapon`
   (`Some(ActiveWeapon::Emp)`), `name`.
2. `ActiveWeapon::Emp` (`name` "emp_burst", `full_load` =
   `emp_charges_per_pickup`, `tell_seconds` = `emp_tell_seconds`),
   appended to `SPECIAL_WEAPONS`; `Tank::emp_charges` with its arms in
   `weapon_ammo`, `take_weapon`, `empty_stock`, `module_cols`.
3. The dispatch arm in `weapons::dispatch_fire_from`; the press edge in
   `drive_player`'s `should_fire` (with the shells, plasma, grenades and
   the hammer).
4. `pickup_phase` needs nothing (`PickupKind::weapon`).
5. `ai::SEEK_SPECIALS = [SonicHammer, Emp]`; `SpecialSense::Emp`, the
   `special_rule` arm, `generic_fire(Emp) == false` (§4).
6. `hud::weapon_color` (`HUD_EMP_COLOR`), `hud::weapon_pickup`.
7. `gen_crates.py` (`KINDS`, `GLYPHS`), `PICKUP_INK['emp_burst']`; the
   tankdesign module `emp` and its anchor `tank_art::EMP_COIL` (§5).
8. `editor::TOOLS` and `Tool::name`; `tool-emp_burst`,
   `tool-short-emp_burst` in both catalogues.
9. `WeaponKind::Emp` (appended to `ALL`, both `From`s,
   `drawn_on_press` true), `Predictor::seed_gate`'s arm,
   `apply::write_tank`'s ammo arm, `kick_turret` (the module's pulse cell,
   `Tank::kick_emp`) and `drawn_muzzle` (`None`: no muzzle).
10. `debug::{TankDebug, TankPatch}`, `set_tank`'s schema, `TankSnapshot`,
    the probe's tank line and fire tuple, `render::game::draw_tank_stats`.
11. The armory's crate (§3.4); `SPAWN_SWAPS` gains
    `(ActiveWeapon::Emp, |t| t.enemy_special_weapon_emp_share)` after the
    hammer's; the tuning group; `PROTOCOL_VERSION`.

And, as they are: the tell (`Tank::tell`, its countdown, its hold, its
wire, its off-screen arrow - `ArrowKind::Windup { weapon: Emp, .. }` drawn
in `HUD_EMP_COLOR` rimmed hostile red), the press show's claim by input
tick (hammer §3.3),
`spawn_pickup {kind: "emp_burst"}`, the probe's `--crate emp_burst`, and
`pyro::Shape::Arc` for the ring.

### 3.2 What this extends

- **`SpecialUse::Clear { face, radius }`** - hold facing `face` and ask
  the commander (§4) to clear the tank's ring of `radius` before it fires.
  `act_special` applies it as `Hold` and records `Ai::clearing =
  Some(radius)` for the collect pass to put on the tank's `UnitView`. A
  rule that offers it does so only while `c2_enabled`.
- **`PressShow::Emp(EmpPress { origin })`**, the predictor's arm and
  `Game::draw_press_show`'s, and `WireEvent::press_show`'s `EmpPulse` arm
  (§8).
- **`Ai::think`** takes `dangers: &[Danger]` beside its `sense` (§4),
  kept on the `Brain`.
- **`Predictor::offline_left`** - the client's own count of its special's
  offline, so a press made in the round trip after a pulse is predicted as
  the shell it will be (§8).

### 3.3 What this adds that the later weapons use

- **The disabled state** (§1's table): `Tank::disabled`,
  `Tank::special_offline`, `Tower::disabled`, and their setters:

  ```rust
  impl Tank {
      /// The special this tank carries - the one with ammo left
      /// (`SPECIAL_WEAPONS` order), offline or not; `None` on shells alone.
      /// What the HUD, the turret's module, the wire and the crate rule read.
      pub fn special(&self) -> Option<ActiveWeapon>;

      /// What the next trigger pull fires: the special it carries while
      /// that is online, else the shell cannon.
      pub fn active_weapon(&self) -> ActiveWeapon;

      /// Whether its special is down: disabled, or offline on its own.
      pub fn special_down(&self) -> bool;

      /// Its electrics out for at least `seconds` (never shortened): the
      /// live shield pops, the tell lapses, a burst, a volley, a twin
      /// bolt and a held flame stop. True when it was not disabled before.
      pub fn disable(&mut self, seconds: f32) -> bool;
  }
  impl Tower {
      /// Offline for at least `seconds`: the tesla's charge and target
      /// dropped, the gun's burst stopped.
      pub fn disable(&mut self, seconds: f32);
  }
  ```

  `active_weapon` keeps its meaning for every reader that asks what the
  trigger fires (`dispatch_fire`, `drive_player`, `seat_arms`, the AI's
  tiers and `generic_fire`, the tell's start). The readers that mean what
  the tank **carries** move to `special()`: `wants_pickup` (an enemy whose
  special is offline still carries it, and does not trade it for a crate),
  `module_cols`, `hud::WeaponSlot`, the wire's `TankState::weapon`, the
  spawn swap ("a tank that drew a special").
- **The pulse's strike walk** (`strike_emp`, called by
  `tick_emp_pulses` with the band the front swept this tick): tanks,
  towers, missiles, each its own block; the drones (BB-40) and the well
  (BB-42) add their blocks after the missiles, on the same band.
- **The disabled state's readers are open lists**: the commander's
  `UnitView::busy` is an `Option<command::Busy>` (`Disabled` here; a
  weapon that takes a tank out of the commander's hands adds a variant),
  and the probe has two tables - `HOLDS` (a hold the AI chose: asleep, a
  tell, a skid, kept out of a danger) and `OUT_OF_ITS_HANDS` (a state the
  tank's motion is not its own in: disabled), each a row per state.
- **`Predictor::offline_left`**, read by `pull_trigger`'s one gate
  (`Game::seat_arms(seat, offline)`), so any weapon that takes the seat's
  special offline on the press sets it and every weapon's arm sees it.
- **Dangers** (`ai.rs`): the places an enemy keeps out of, built once a
  frame by `enemy_phase` and read by every enemy's `think`:

  ```rust
  /// A place an enemy keeps out of this tick, because a weapon could go
  /// off on it there. Empty unless a weapon makes one, so a round without
  /// one decides exactly what it did.
  #[derive(Clone, Copy, Debug, PartialEq)]
  pub struct Danger {
      pub shape: DangerShape,
      /// The owner slot of the tank whose weapon it is: that tank never
      /// shies from it. `None` for one nobody owns (the rod's call
      /// circle), which every enemy keeps out of.
      pub owner: Option<usize>,
      /// How deep its outer band runs in which a tank only stops rather
      /// than backing out: the berth of a danger over in a moment (an
      /// ally's crackle), where a turn round and back would spin it. 0
      /// backs out from the edge.
      pub slack: f32,
  }

  #[derive(Clone, Copy, Debug, PartialEq)]
  pub enum DangerShape {
      /// Everything within `radius` of `at` (an armed EMP, an ally's
      /// crackle; the rod's circle).
      Disc { at: Position, radius: f32 },
  }

  impl Danger {
      /// How far `p` stands inside (negative: outside).
      pub fn depth(&self, p: Position) -> f32;
      /// The points `clear` px outside it nearest `p`, best first: the
      /// nearest, then ones turned further and further from it (a disc:
      /// 30, 60, 90 and 120 degrees each way, never back through the
      /// middle), along the line through `from` when `p` is the middle.
      pub fn exits(&self, p: Position, clear: f32, from: Position, facing: Dir) -> [Position; 9];
      /// `exits`' first.
      pub fn exit(&self, p: Position, clear: f32, from: Position, facing: Dir) -> Position;
      /// The points `clear` px outside it on the axes through its middle:
      /// where a tank fighting what stands there waits lined up on it.
      pub fn posts(&self, clear: f32) -> [Position; 4];
      /// The point it is drawn round.
      pub fn middle(&self) -> Position;
  }
  ```

  A new shape adds its arm to those four; nothing else matches on it.

  The `dodge` tier (1.6, after `special`, before `flee`):
  `condition(b.danger_here().is_some())`, `action("dodge", act_dodge)`.
  `danger_here` is the deepest danger this tank is inside and does not
  own, ties to the earlier in the list - latched by `Ai::dodging` until
  the tank stands `enemy_danger_clear_px` (24) outside it, so the edge is
  never a place to jitter. In a passing danger's slack band it stops.
  Otherwise `act_dodge` resets the aim and walks for its way out, a cell
  past the clear margin: the exit it chose (`Ai::dodge_exit`, a point,
  kept while it stays out of every danger and walkable), else the first
  of `exits(me, enemy_danger_clear_px + a cell)` that is on the field, in
  a usable cell, out of every danger and joined to where it stands **on
  foot** (`Brain::can_reach`, `Grid::next_step_walking`) - a seat by the
  field's edge has its nearest exit off the field, and the exits turn with
  the tank's bearing from the danger's middle, so one chosen afresh every
  tick slides off a shore or the field's edge as the tank walks to it and
  the tank turns back and forth between it and the next. Steering drives
  it (`Brain::steer_out`: the walking route - never through a portal,
  whose far end a route hands out as its first step from the near one's
  edge -, avoidance, the stuck escape, and no wander, which draws on the
  round's RNG), and the one turn steering never takes, round from a
  heading that leads away from the way out, is put on its heading first:
  a dodge is most often that. And so the rest of the tree never steers
  back in, the points it steers at are kept out of every danger the tank
  does not own (`Brain::out_of_danger(p)`: the point itself, or its
  `way_out` from the first danger holding it, at the dodge's own
  distance): the engagement point (`engage_point`, which chase and
  attack's reposition read), patrol's alert point, and every pickup it
  heads for (one inside a danger is neither sought nor fled to:
  `Brain::seek`, `nearest_pickup`). The seat itself, steered at with no
  slot, is its own danger's middle, so its way out is a `post`, inside
  the seat's sight box first (a shot is fired from there). Attack's
  reposition that has reached a point moved out of a danger waits there
  facing the fight (`Ai::kept_out`, a probe hold) instead of driving on
  past it, which the commitment would, into the danger. The frame's route
  grid surcharges the cells inside a seat's danger
  (`enemy_danger_route_cost`, 2), so a route goes round it rather than
  through it. Aim and fire still read the real target: a tank keeps
  shooting shells at the seat from outside the disc. The rod (BB-41)
  adds its circle as a `Disc` with no owner; the rail (BB-39) adds
  `DangerShape::Lane` if its surcharge alone does not do.
- **`TankState::{disabled, offline, shells}`**, `tile_flags::DISABLED`
  and `WeaponSlot::offline` (§5, §8) - any later weapon that takes a
  special offline reads and writes the same fields.

### 3.4 The armory's EMP

Into `maps/armory.toml` (hammer §3.5), nothing of the hammer's moved:

| Mark | Cell | Why |
|---|---|---|
| EMP crate | 7,4 | The reserved column's first cell, three cells from the start |
| EMP crate | 29,7 | On the enemy side, so an enemy on shells collects it and its rule shows with no tuning |
| Missiles crate | 31,16 | An enemy on shells takes it and fires volleys the seat's pulse kills in the air |
| Lamp posts | 11,4 and 25,10 | Beside the one at 17,8, so the at-11 reads across the map (`--weather night`) |

The towers it acts on are there (the player's tesla at 3,4, the enemies'
tesla at 34,3 and gun tower at 34,14), and the shield inside the glass
house is the shield it pops. The map lints clean (`supported_maps_no_new_errors`)
and the armory's CPU thumbnail pin is re-baselined for the crates and
posts.

## 4. AI

An enemy carrying the EMP uses it by `emp_rule`, never through the generic
tiers (`generic_fire(Emp)` is false). While its EMP is offline -
after its own pulse, or disabled and rebooted with the offline still
running - `active_weapon` is the shell cannon, `special_rule` has no arm
for it, and the generic tiers fight with shells: an EMP tank that has just
pulsed shoots shells into what it disabled.

### What it is handed

`Game::emp_senses` runs once per frame in `enemy_phase`, only when some
tank on the field carries an EMP (`any_emp`), over the seats as the
enemies see them (`EmpSeat`: seat, position, hull box, live, concealed,
shielded, carrying an online special, carrying an armed EMP, disabled),
the standing online towers (cell box, side), the live enemies with a body
that are not disabled (hull box now, and where its motion carries it by
the time a pulse decided now lands: the crackle and the ring's run out)
and `Game::is_night`. It gives each live enemy carrying an online EMP an
`EmpSense`:

```rust
pub struct EmpSense {
    /// What a pulse from where this tank stands is worth (`emp::seat_value`
    /// over the seats it would reach, plus the player towers it would):
    /// each seat in reach that this tank stands in the sight box of and
    /// that is not hidden from it (`concealed` and not hit-alerted - the
    /// attack tier's rule) and not already disabled is worth
    /// `emp_ai_seat_value`, plus `emp_ai_shield_value` with a live shield,
    /// `emp_ai_special_value` carrying an online special, and
    /// `emp_ai_night_value` at night; each standing, online player tower in
    /// reach `emp_ai_tower_value`.
    pub value: i32,
    /// The seat the pulse is used on: the most valuable in reach, ties to
    /// the lower seat. What `Ai::shot_at_seat` records.
    pub at_seat: Option<u8>,
    /// A seat in reach would see this tank from outside its sight box.
    pub off_box: bool,
    /// A fellow enemy - live, not disabled, not this one - whose hull is,
    /// or by the time a pulse decided now lands will be, within
    /// `emp_radius_px + emp_ai_friend_margin_px`.
    pub friends: bool,
    /// A standing, online enemy tower within reach.
    pub friendly_tower: bool,
    /// The seat this tank fights, valued alone (`seat_value`, whatever the
    /// range), and whether a fellow enemy's hull stands within
    /// `emp_radius_px` of it - what the approach weighs.
    pub target_value: i32,
    pub target_crowded: bool,
    /// One of the `emp_ai_closers` enemies with an online EMP nearest the
    /// seat this tank fights (ties on slot): the ones that may close in.
    pub closer: bool,
}
```

"In reach" is the pulse's own measure (`emp::box_reach`, the nearest point
of the hull box or the cell). `off_box` counts every live seat in reach,
hidden or not: with the defaults a hull inside the ring always has its
attacker inside its sight box (160 plus a hull is under the box's 240 px
half height), and the flag keeps the rule true on any knobs.

### The rule (`emp_rule`), in priority order

1. **The tell holds** (`special_rule`'s first arm, hammer §3.1).
2. **A training dummy** (`Ai::frog_only`) never uses it: `None`. Every
   pulse reaches whatever seat stands in the ring.
3. **`off_box`**: `None`. The sight box binds the pulse whole.
4. **It pays** - `value >= emp_ai_fire_value` (1: any seat in reach, or
   a player tower, is enough; the weights pick `at_seat` and rank):
   1. a standing friendly tower in reach: `None` (it never puts its own
      side's tower out, and nothing can move a tower out of the way);
   2. a friend in reach: `Clear { radius: emp_radius_px +
      emp_ai_friend_margin_px }` - it never fires the EMP into an ally;
      it records the ring (`Ai::clearing`) and fights on through the tiers
      below, and on the next frame the ring is a danger its allies back out
      of (below), and with `c2_enabled` the commander nudges them out too.
      Once none is in reach, the pulse goes off. It asks for
      `emp_ai_clear_patience_seconds` (3) at most (`Ai::clear_waited`,
      running while an ally stays in reach): past that it asks no more -
      `None`, so its ring is no danger - until its reach is clear, and an
      ally that cannot get out (penned, wedged against another tank) is
      not kept backing out for ever while the pack waits on one ring;
   3. otherwise `Fire { face: facing, at_seat, why: "pulse" }` - the
      facing it has (a pulse is all round), through the tell.
5. **Approach**: a `closer`, under `enemy_flee_damage`, not a guard that
   holds, not hunting the frog, whose seat is within `attack_range`, in
   its line of sight, not hidden from it, worth a detour alone
   (`target_value >= emp_ai_approach_value`, 2) and not crowded
   (`!target_crowded` - it would only hold beside an ally there), closes
   in to its own spot on the line from the seat out to its engagement
   slot, `emp_ai_close_share` (three fifths) of the ring out
   (`Brain::close_spot`; decision 32),
   and holds there facing the seat (`Hold { why: "close" }`). One closer
   per seat (`emp_ai_closers`) keeps a pack of EMP tanks from
   piling onto one seat; the rest keep their slots and use it when the
   seat comes into their ring.
6. Otherwise `None`: the tree goes on (attack lines up and settles but
   never fires the EMP; chase, patrol and the seek tiers as ever).

**What it means at the defaults.** Any seat that comes into the ring is
pulsed, by day a bare one too (worth 1) - the issue's "fires when it pays:
at least one seat, or a player tower, inside the ring" - and so is a player
tower alone. The weights are preference: they pick the seat the pulse is
used on (`at_seat`) and decide what is worth going after. A closer closes
in only on a seat worth `emp_ai_approach_value` (2): one with a live shield
or an online special - or any seat at night (the night's +1), when killing
its headlights is worth the charge and the EMP tank goes looking for it, the
issue's "prefers night". By day a bare seat is pulsed where it stands in an
EMP tank's ring, never hunted down for it.

**Pacing**: `emp_ai_fire_interval` (3.5) between an enemy's decisions, on
top of the offline it costs itself (`act_special`'s fire timer). With the
timer running and the pulse paying, it holds facing as it is (hammer §3.1).

**The tell**: `emp_tell_seconds` (0.5) of crackling arcs on the module
(§5) before the pulse, the tank held facing as it is. The decision is the
moment the sight box is checked and `shot_at_seat` recorded. A tell
commits: a seat that drives out of the ring during it has dodged.

### With the commander (`c2_enabled`)

The ring is cleared by the dangers without the commander (an ally keeps out
of a clearer as of a crackle); with it, the clearing is also an order
(docs/enemy-command-and-control-prd.md):

- **`UnitView`** gains `clearing: Option<f32>` (the tank's `Ai::clearing`
  this tick, from its rule's `Clear`) and `busy: Option<command::Busy>`
  (`Busy::Disabled` here - an open list the later weapons add their
  states to).
- **`clear_rings`**, a producer `plan` runs **before** `deconflict`: for
  each unit with `clearing = Some(radius)`, in slot order, every other
  unit - not a wreck, not busy, not a seat, not already backing out of a
  danger on its own (`UnitView::dodging`: a clearer's ring is one from the
  frame after it asks), not already ordered this frame - whose centre
  stands within `radius` plus its own avoidance radius of the
  clearer is given `Order::Nudge { dir }`: the cardinal away from the
  clearer along the larger of the two offsets, then along the smaller,
  then the other two in `Dir::ALL` order, the first `ctx.blocked` does not
  wall; none open, no order (`Skipped::no_free_lane`). `Nudge` already
  exists - reflex, applied to this frame's intent - so the tree is left
  alone (the commander speaks `Intent`, never `Ai`). The ring clears over
  a few ticks; the clearer fights on until it has, then pulses.
- **`deconflict`** skips a yielder that already holds an order this
  frame, so a cleared unit is not also slowed; and **a busy unit never
  gives way** - it cannot hear: `gives_way`'s first step, and it is never
  handed an order (`Skipped::deaf` counts the would-be yields).
- **C2 off is untouched**: `plan` returns before any producer, as ever.
  With C2 on and no EMP on the field, `clear_rings` finds no clearer and
  `deconflict` sees an empty `orders` map before it runs - exactly its
  old input.

### Reacting to the EMP: dangers

A seat has no tell (it fires on the press), so "seeing the tell" is seeing
the weapon: the EMP's module on the turret, lit while armed. `emp_dangers`
(built in `enemy_phase` only when some tank on the field carries an EMP)
makes a `Danger` of:

- **every live seat on the field carrying an online EMP** (charges left,
  `!special_down()`) **that is not concealed**: a `Disc` at the seat,
  radius `emp_radius_px + emp_ai_berth_px` (208) to the enemy's centre, so
  a hull at the disc's edge stands clear of the ring. A seat hiding in
  grass with an EMP is a trap the pack walks into. The moment its EMP is
  offline - it just pulsed, or an enemy pulsed it - the disc is gone and
  the pack may close in, which is the window the EMP's cost opens.
- **every live enemy with an EMP tell running, or that held its pulse for
  an ally on its last think** (`Ai::clearing`): the same disc round it,
  owned by it - its allies back out of the ring once they see the
  crackle, or the clearer waiting on them, and it does not. In a
  crackle's berth (`slack`, `emp_ai_berth_px`) an ally only stops - the
  crackle is over in half a second, and turning round and back there spun
  tanks in the probe; a clearer's ring it backs out of whole. Two clearers
  in each other's ring each back out of the other, so a pack spreads
  until one has a clear ring - or a clearer's patience runs out, when its
  ring is no danger until it is clear (above). Built, a clearer that held
  still for its ring was the probe's worst: a pack of EMP tanks round a
  seat, each holding for the others, pressed in on one another and
  stalled.

Every enemy keeps out (`dodge`, §3.3): one inside backs out to the clear
margin, and its chase, attack reposition, alert and seeks aim outside, so
it fights a seat carrying an armed EMP with shells from beyond the ring
(the engagement ring's 272 px already stands outside 208). A disabled
enemy does not dodge (it does not think) - which is the point of the
pulse. The special tier sits above `dodge`, so an EMP enemy's approach
still walks in on a seat armed with its own EMP: whoever pulses first
wins the duel. The danger reads no RNG, and its exits break ties in
`Dir::ALL` order.

### What changes for enemies that carry something else

- A disabled enemy coasts (§1); its tell, of any weapon, has lapsed.
- The hammer's rule (§12): its closer does not approach a seat inside a
  danger; an offline enemy tower and a disabled enemy are no trouble to
  shove a seat into.
- `wants_pickup` reads `special()`: a disabled enemy coasting over a
  weapon crate does not take it while it still carries one.

### Off the field and asleep

A field map's far tank coasting on its last intent has no sense; its EMP
tell, if it had one, still counts down and goes off (the hammer's rule). A
far tank that is disabled coasts by the EMP's branch instead (§1).

## 5. Drawing

All of it in the effects language (docs/effects.md): whole 2 px blocks,
ramp steps, Bayer fades; composed at draw time as pure functions of the
pulse, the tank or the tower and its age, hashed from slots and cells,
never rolled.

- **A new ramp, `pyro::EMP`**: `#1D6071` (`BLUE_DEEP`, the tank kit's deep
  water step), `#038AAB` (`BLUE_DK`), `#27D8C5` (`BLUE_BRIGHT`), `#93ECE2`
  (`BLUE_PALE`), `#FFFFFF` - the palette's own blues, the Armory scene's
  colours, on the palette (effects.md's ramp table gains its row).
- **The ring** (`emp::ring`), in `render/game.rs`'s glowing pass
  (fast and bright, rule 7; read at night): three full circles of
  `pyro::Shape::Arc` - the front at `emp_ring_speed * age`, two blocks
  thick in `BLUE_PALE`; a one-block `WHITE` circle 6 px inside it; a
  one-block `BLUE_BRIGHT` circle 12 px inside it (none under 8 px) - the
  front holding at `emp_radius_px` once there; `cover` falling from 1 over
  the last half of the linger. Crackle along the front: every 1/20 s a
  hashed set of eight short zigzags (`pyro::block_line`, three
  segments of 4-8 px off the circle, `BLUE_PALE` with a `WHITE` head
  block). One glow over the pivot (`pyro::glow`, `BLUE_PALE`, 40 px,
  falling to nothing over the first 0.3 s). Nothing throws light on the
  ground: the ring is gone in half a second.
- **The ripple**: `Shockwave::scaled(pivot, emp_shock)` (0.25 of a tank
  dying) through `shockwave.rs` - a soft bend and shake, none under
  reduced motion.
- **Sparks on a disabled hull** (`emp::sparks`), glowing pass, for
  the whole time it is disabled: bursts of 0.2 s twice a second, phased
  by slot (the Armory scene's rhythm); in a burst, every 1/12 s a fresh
  hashed set of four zigzags from points on the hull box's edge outward
  6-16 px, two or three segments each, `BLUE_PALE` with a `WHITE` head,
  and one `BLUE_BRIGHT` glow of 20 px over the hull; over the last second
  of the timer, one zigzag fewer every quarter second. At night a
  burst throws a little light (`weather::lights_in`, an unshadowed point
  of 28 px in `EMP`'s pale blue at `emp_spark_light`, on the frames
  `emp::sparking` says a burst is on - the drawing's own rhythm).
- **Lights out.** A disabled tank throws no headlight, spotlight or hull
  glow (`lights_in`), and its light layer is not drawn at night
  (`draw_tank_glow` skipped). By day, where the paint already carries the
  lamps at their daylight colours, `tank::draw_tank_dark` draws the light
  layer's cells again over the paint in black at `emp_dark_alpha` (0.75) -
  the game's shadow convention - so the lamps, the accent strips and the
  sensor eye read dark. At night the same overlay keeps the paint's
  lamps from glowing through the ambient multiply. A tower: no glow
  overlay (`draw_tower`'s `glow` 0), and no light from the tesla's coil
  or the mortar's mouth.
- **The sag.** A disabled **enemy's** turret and its modules are drawn
  `emp_droop_deg` (15; 8 read only under a close zoom) off their aim, to the side `emp::droop_side(slot)`
  hashes, eased in over `emp_droop_seconds` (0.4) and out over half that
  once it reboots (`Tank::droop`, eased with the turret's own angle in
  `ease_turret_visual_rotation`, so a replica sags the same). A seat's
  turret does not sag. An offline gun tower's and bio slush's top layer
  sags the same way (`Tower::droop`, eased in `tick_tower_effects`). Drawn
  only: no shot reads it.
- **The tell** (`emp::tell`), glowing pass: arcs jumping round the
  module's coil (`tank_art::EMP_COIL`): every 1/15 s a hashed set of two
  to four zigzags from the coil's rim outward, 4 px growing to 12 px with
  the tell's progress, `BLUE_PALE` with `WHITE` heads, dissolving in from
  `cover` 0.4 to 1; one small glow on the coil pulsing quicker as it
  nears. The module alternates its two crackle cells at `8 + 16 *
  progress` Hz.
- **A dead missile** (`render/missile.rs`): no exhaust flame, its nose
  easing toward the way it falls (`Missile::facing` follows the drawn path
  as ever, and the path now drops), no smoke trail (`fx.rs` skips it), no
  pool of light on the ground, and its sprite cut above the flame the
  sheet bakes into every frame (`render/missile.rs`: the body's rows
  alone, so it is drawn where a live one is). Its
  landing (`MissileDud`, `fx.rs`): particles, not a composed burst - a
  small puff of dust and a few sparks in `EMP`'s pale blue and white
  thrown up, and a splash where it lands in water. No scorch, no
  fireball.
- **Lamp posts going out** (`lamp::draw_post(.., lit)`, `lit` from
  `emp::lamp_lit(at, lamps_out)`): dark - the glass `SMOKE[1]`, no
  flame - while it runs; in its first 0.25 s a little burst of `WHITE` and `BLUE_PALE`
  blocks off each lantern (`emp::lamp_sparks`, glowing pass); in its last
  0.6 s each post flickers back in two hashed blinks before it steadies.
  `lights_in` follows the drawing (a post throws light on the frames it
  is drawn lit); the rule (`is_lit`) is exact - no post counts while
  `lamps_out > 0`.
- **The module** (`tankdesign`, `lines/vanguard.py`, `module_fn('emp')`):
  an EMP projector on the roof - the missiles' hardpoint, shared, since a
  tank carries one special at a time (`hp.get('emp', hp['missiles'])`): a
  squat gunmetal plinth (7 x 6, chamfered - 5 x 5 left no room for the
  coil's rim on the light chassis), on it a toroid coil - a ring of brass
  windings shaded by `rim_shade` (the kit's round-rim shading; `'cylv'`
  shades a cylinder side-on and read as a barrel) round a dark core - with
  a steel emitter stub forward. Five cells, `TANK_MODULE_EMP_COL` = 28..32 (`tank_modules.png`
  grows from 28 to 33 columns, 1320 x 480): 0 armed, the core's lamp dim
  (`DIM_ION`, `BLUE_DK`); 1 and 2 the crackle, the left and then the
  right half of the windings lit in the light layer (`'ion'`: `BLUE_PALE`
  over `BLUE_BRIGHT`); 3 the pulse, the whole ring lit `'white'` and the
  core `'ion'`; 4 offline, the windings scorched (`RUST` step 0 for
  brass) and the core dark, no lamp. `module_cols` (now eight entries): cell 3 while
  `Tank::emp_flash` (`emp_flash_seconds`, set by `kick_emp` on every pulse
  - the room's, a replica's `Fired`, a client's press), 1 and 2 through a
  tell, 4 while `special_down()`, else 0. A disabled tank carrying any
  other special shows that module's idle cell (its armed cell needs the
  weapon live), which is its lights out. Anchor: `tank_art::EMP_COIL` (the
  coil's centre, turret frame), written by `export.py` from the module's
  `meta['coil']`. `render.SHOWN_TOGETHER` leaves it out with the grenade
  launcher and the hammer.
- **The crate**: row 14 of `gen_crates.py`'s sheets (`crates_sheet.png`
  280 x 600, `pickup_glyphs.png` 24 x 360). Its symbol, 10 x 10 design
  px: the power-off sign, the ring in the ink's base and its bar (`o`) in
  the ink's light - not a bolt, which is the speed crate's:

  ```
  '....oo....',
  '.X..oo..X.',
  'XX..oo..XX',
  'X...oo...X',
  'X...oo...X',
  'X........X',
  'X........X',
  'XX......XX',
  '.XX....XX.',
  '..XXXXXX..',
  ```

  Ink (`punypalette.PICKUP_INK['emp_burst']`, admitted on the crate sheets
  alone like the others): cobalt - shade `#2433A6`, base `#4F6BFF`, light
  `#B3C2FF` - the deep electric blue the inks leave free between the
  hammer's sky blue and the tower pack's periwinkle. To be shown beside
  the other fourteen in a screenshot before it is settled (§12).
- **The HUD**: `hud::WeaponSlot::of` gives the carried special
  (`Tank::special`), its count and `offline` (`special_down()` with a
  special carried). Online it is drawn as ever, in `HUD_EMP_COLOR`
  (`#4F6BFF`, the ink's base). Offline, the weapon slot
  (`V_WEAPON`..`V_SPEED`, `hud::WEAPON_SLOT_W` = 64 pt) alternates at
  `emp_hud_flicker_hz` (3.5) on the round's clock between the words and
  the readout: the words `WPN OFFLINE` split at the first space into two
  lines (`hud::offline_lines`), each centred in the slot at
  `HUD_LABEL_SIZE`, in `RED_BRIGHT` (`#FF421A`); the readout the
  special's symbol unlit and its count in `DIM`. The pips under a seat's
  ring are drawn in `DIM` while offline.
- **The off-screen tell** is the hammer's arrow (`ArrowKind::Windup`) in
  `HUD_EMP_COLOR`, rimmed hostile red. A disabled enemy raises no lane
  warning (`Scene::of` reads each tank's outage beside its facing: it
  cannot fire), so an off-screen arrow never warns of a tank that is
  coasting.

## 6. Tuning

New group `emp` (every row live unless marked), plus three rows in the
enemies' group:

| Row | Default | Range | Doc |
|---|---|---|---|
| `emp_charges_per_pickup: i32` | 3 | 1..=20 | Pulses one EMP crate loads. One per press. |
| `emp_radius_px` | 160 | 32..=480 | How far the ring reaches from the hull's centre (px; five cells), to the nearest point of what it strikes. |
| `emp_ring_speed` | 800 | 60..=5000 | How fast the ring runs out (px/s): what it reaches it strikes when the front gets there. |
| `emp_disable_seconds` | 3 | 0..=20 | How long a tank the ring reaches stays disabled - an enemy's brain off, its special offline, its lights out - and how long the shooter's own special stays offline. |
| `emp_tower_seconds` | 6 | 0..=30 | How long a tower the ring reaches stays offline. |
| `emp_lamp_seconds` | 10 | 0..=60 | At night, how long every lamp post on the map stays dark after a pulse. |
| `emp_tell_seconds` | 0.5 | 0..=3 | An enemy's crackle before its pulse; 0 pulses on the decision. |
| `emp_ring_seconds` | 0.4 | 0.05..=3 | How long the ring's picture lingers once its front is out. |
| `emp_flash_seconds` | 0.25 | 0..=2 | The module's pulse cell. |
| `emp_shock` | 0.25 | 0..=2 | The screen ripple and shake against a tank dying's. |
| `emp_droop_deg` | 15 | 0..=45 | How far a disabled enemy's turret, and an offline tower's top, sags off its aim (drawn only). |
| `emp_droop_seconds` | 0.4 | 0.05..=3 | How long the sag takes; it comes back in half that. |
| `emp_dark_alpha` | 0.75 | 0..=1 | How dark a disabled tank's lamps and strips are drawn over its paint. |
| `emp_spark_light` | 0.6 | 0..=2 | The light a disabled hull's sparks throw at night, against a headlight's. |
| `emp_missile_gravity` | 600 | 50..=5000 | How fast a dead missile falls (px/s²). |
| `emp_missile_drag` | 2.5 | 0..=20 | How fast a dead missile loses its ground speed (per second). |
| `emp_hud_flicker_hz` | 3.5 | 0.5..=20 | How often `WPN OFFLINE` flickers. |
| `emp_ai_fire_value: i32` | 1 | 1..=20 | What a pulse has to be worth before an enemy fires it: at 1, any seat or player tower in reach. |
| `emp_ai_approach_value: i32` | 2 | 1..=20 | What the seat an enemy fights has to be worth alone before its closer closes in to pulse it: at 2, a seat with a shield or an online special, or any seat at night. |
| `emp_ai_close_share` | 0.6 | 0.1..=1 | How far from the seat a closer waits to pulse it, as a share of the ring's reach. |
| `emp_ai_seat_value: i32` | 1 | 0..=10 | A seat in reach, not already disabled. |
| `emp_ai_shield_value: i32` | 1 | 0..=10 | More for a seat with a live shield. |
| `emp_ai_special_value: i32` | 1 | 0..=10 | More for a seat carrying an online special. |
| `emp_ai_night_value: i32` | 1 | 0..=10 | More for each seat at night, when its headlights matter. |
| `emp_ai_tower_value: i32` | 1 | 0..=10 | A standing, online player tower in reach. |
| `emp_ai_friend_margin_px` | 16 | 0..=128 | How far past the ring an ally still holds an enemy's pulse. |
| `emp_ai_berth_px` | 48 | 0..=256 | How far past the ring an enemy keeps from a seat carrying an armed EMP, or from an ally's crackle. |
| `emp_ai_fire_interval` | 3.5 | 0.1..=20 | Seconds between an enemy's decisions to pulse. |
| `emp_ai_clear_patience_seconds` | 3 | 0..=30 | How long an enemy holds its pulse for allies in its ring - the ring a danger they back out of - before it stops asking until none is in it. |
| `enemy_danger_clear_px` (`enemies`) | 24 | 0..=128 | How far outside a danger an enemy backs before it turns back to the fight. |
| `enemy_danger_route_cost: usize` (`enemies`) | 2 | 0..=64 | Route surcharge on every cell inside a seat's danger, so enemies go round an armed EMP rather than through it; 0 turns it off. Low for the tower reach's reason (`route_tower_cost`). |
| `enemy_special_weapon_emp_share` (`enemies`, `@ Restart`) | 0 | 0..=1 | The share of special-carrying enemies that spawn with the EMP instead, decided by a hash of the spawn point and the slot - never the round's RNG - so at 0 nothing changes. |

Constants (geometry, not feel) in `emp.rs`: `RING_INNER_PX` (6, 12),
`CRACKLE_HZ` (20), `SPARK_HZ` (12), `TELL_HZ` (15), `LAMP_SPARK_SECONDS`
(0.25), `LAMP_RETURN_SECONDS` (0.6).

## 7. Text

| Key | en | sl |
|---|---|---|
| `hud-weapon-offline` (`keys::HUD_WEAPON_OFFLINE`) | WPN OFFLINE | IZPAD OROŽJA |
| `tool-emp_burst` | emp burst | emp sunek |
| `tool-short-emp_burst` | emp | emp |

`IZPAD OROŽJA` ("weapon outage") folds to `IZPAD OROZJA`. The budget:
`every_language_fits_every_budget` gains the offline words, split at the
first space (`hud::offline_lines`), each line within `WEAPON_SLOT_W - 4`
(60) at `HUD_LABEL_SIZE` - `WPN` 21, `OFFLINE` 44, `IZPAD` 31, `OROZJA`
40 at the font's own widths. The tool names are measured by the existing
tool budgets (144 and 48). Both languages fit (the budget test runs
over every shipped language).

## 8. Wire

Protocol 17 (from the hammer's 16), once in the PR.

- `WeaponKind::Emp`, appended to `ALL`; `drawn_on_press` true.
- `WireEvent::EmpPulse { slot: u16, x: i16, y: i16 }` - the pivot in
  quarter pixels; mirrors `Event::EmpPulse { slot, x, y }`. Its
  `press_show` is `(seat, WeaponKind::Emp)`.
- `WireEvent::MissileDud { x: i16, y: i16 }`; mirrors
  `Event::MissileDud`.
- `Event::Disabled { slot, x, y }` and `Event::TowerDisabled { x, y }`
  are on `NOT_SENT`: the state is what draws.
- `TankState::weapon` is the special the tank **carries**
  (`Tank::special`, `Shell` with none) and `ammo` its rounds - the same
  values as before whenever nothing is offline. Three new bytes:
  `disabled: u8` and `offline: u8` (`Tank::disabled` and
  `Tank::special_offline` in tenths, `quantise_seconds`) and `shells: u8`
  (the magazine whatever is carried - what the trigger fires while the
  special is offline, which the predictor gates a press against).
  `tank_flags` stays a byte.
- `MissileState::dead: bool`.
- `tile_flags::DISABLED` (bit 3): a tower tile is listed while its
  `Tower::disabled > 0`, at full health too, with the flag set.
- `RoundState::lamps_out: u8` (tenths).
- **What a replica draws**: on an `EmpPulse`, `emp_show` - a cosmetic
  pulse (`live` false: it strikes nothing) on `Game::emp_pulses`, the
  ripple, the module's flash through `Fired`'s `kick_turret`. From state:
  `write_tank` sets `disabled`, `special_offline` and `shells_ammo` (and
  the carried stock as ever), and `tick_presentation` runs both timers
  down between snapshots; a tile entry's `DISABLED` sets
  `Tower::disabled` to `emp_tower_seconds` when it was 0 and holds it,
  the flag going off (or the entry going) clears it - the heat shield's
  `set_timer` pattern -, and `tick_presentation` drains it; `dead` puts a
  replica's missile in `MissileStage::Dead`; `lamps_out` sets
  `Game::lamps_out`, run down between snapshots. `MissileDud` starts its
  burst in `fx.rs`.
- **What is drawn at once** (decision 3 of BB-36, hammer §3.3): the
  shooter's press. `Predictor::pull_trigger`'s EMP arm, on the press edge,
  with charges less the owed presses, the local gate open and
  `offline_left` 0: takes the sandbox's pivot (`Game::seat_emp(seat)`),
  sets the gate to `player_fire_interval`, sets `offline_left` to
  `emp_disable_seconds`, owes the press and, while presses are drawn,
  queues `PressShow::Emp(EmpPress { origin })` and the drawn press.
  `fly_own_shots` draws it through `Game::draw_press_show` - `emp_show`
  on the replica. While `offline_left > 0` (counted down each tick) the
  arm reads the weapon as the shell cannon, with the sandbox's `shells`,
  so a press made in the round trip before the room's offline arrives is
  predicted as the shell it will be. `seed_gate`'s EMP arm (a room's
  `Fired` for a press the client did not draw) sets `offline_left` and the
  gate; a drawn press nobody claims within the refusal wait is dropped and
  clears `offline_left`. **The HUD at once**: each frame `OnlineRound`
  writes `max(the snapshot's special_offline, offline_left)` into the
  shown seat's tank, where it writes the drawn pose, so `WPN OFFLINE`
  shows on the press frame.
- **What is claimed**: the room's `EmpPulse` for this seat, by the input
  tick its `Fired` names (`presses_drawn`, one pending claim per kind): not
  drawn again, not handed on to `game.events`.
- **What stays the room's**: who is disabled (`TankState::disabled`), the
  shields (`ShieldBroken` and `shield`), the towers (`DISABLED`), the
  missiles (`dead`, `MissileDud`), the lamps (`lamps_out`). Nothing is
  shoved, so a client-owned hull has nothing to hear of and the validator
  is unchanged.
- `delta.rs` needs nothing new (the fields are inside their families and
  `RoundState`); its random snapshots fill them, and the size bounds are
  re-measured.

## 9. Determinism

- **The EMP draws no RNG at all.** No roll anywhere: the reach is a
  distance, every effect a timer, the dud a fall. Its walk is fixed: per
  pulse in the order fired, the seats in index order then the enemies by
  slot, then towers by cell, then missiles by id.
- **The AI**: `emp_rule` chooses by fixed priority and integer values,
  ties to the lower seat; the closer ties on slot; `emp_dangers` is built
  in seat then slot order, `danger_here` picks the deepest then the
  earliest, exits break ties in `Dir::ALL` order; `clear_rings` walks
  clearers and units in slot order. The coast replays the last intent;
  the reboot clears to fixed values.
- **The swap** is the hammer's hash (§3.4 of its doc); the new entry runs
  only with its share above 0.
- **A round without the EMP replays byte for byte**: no crate kind is
  rolled anywhere, every share defaults to 0, no enemy carries one, so
  there is no pulse, no disabled tank, no offline special
  (`active_weapon` is `special().unwrap_or(Shell)` exactly as before), no
  danger (the `dodge` tier is a `false` with no state touched, and
  `out_of_danger` returns its point), no sense, no clearer; the spotters,
  the retarget pass, `field_alerts` and `reroll_stragglers` filter only
  disabled tanks; `lit_lamp_posts` is `lamp_posts` while `lamps_out` is 0.
  `determinism_tests`' pinned streams, the probe fixtures' ceilings and
  every thumbnail pin but the armory's stay as they are.

## 10. Tests

**Scenarios** (`simulation/emp_tests.rs`, headless, on the default 34 x 17
field with the seat at (3, 6) facing east and enemies placed by hand):

- `an_emp_crate_arms_the_burst_and_replaces_the_special_carried` - three
  charges, another special emptied, a second crate refills to three.
- `the_emp_fires_on_the_press_and_takes_its_own_special_offline` - one
  pulse per press; `Fired` then `EmpPulse` in one tick; `special_offline`
  running, `active_weapon` the shell while `special` stays the EMP; a
  press within it fires a shell; after it the next press pulses.
- `the_ring_reaches_far_things_later_and_stops_at_its_reach` - a near hull
  struck before a far one; a hull whose box's nearest point is past the
  reach is not.
- `walls_do_not_stop_the_pulse` - an enemy behind iron struck.
- `a_range_board_in_the_ring_is_left_alone` - an enemy behind a board
  struck, the board whole.
- `a_struck_enemy_coasts_on_its_last_intent_and_reboots` - the intent it
  had when the ring reached it, the trigger released, `AiSnapshot::down`,
  its fire timer frozen; after the outage it thinks again with no stuck
  evidence from the coast.
- `a_disabled_enemy_spots_nobody` - the arena's shared alert skips it.
- `a_live_shield_pops_with_shield_broken`.
- `an_enemys_tell_lapses_when_it_is_struck` - a hammer tell: no
  `SonicBlast`.
- `a_burst_and_a_volley_stop_short_and_keep_their_rounds` - and a held
  flame goes out, a twin's second shell still leaves, a second strike
  never shortens the outage.
- `a_struck_seat_drives_and_fires_shells_with_its_special_offline`.
- `towers_in_the_ring_go_offline_and_come_back` - a tesla's charge lost
  and no strike for `emp_tower_seconds`, then it strikes again.
- `an_offline_player_tower_is_no_detour` - `player_tower_reach` leaves it
  out.
- `a_missile_in_the_ring_falls_dead_and_lands_a_dud` - a `MissileDud` and
  no blast where it fell; one outside the ring flies on.
- `at_night_the_lamp_posts_go_out_and_come_back` - `is_lit` false and
  `sight_on` short by a post for `emp_lamp_seconds`, then lit.
- `by_day_the_lamp_posts_are_left_alone`.
- `the_shooter_keeps_its_lights_and_its_shield`.
- `the_pulse_disables_a_teammate_and_fellow_enemies` - a seat's pulse a
  teammate, an enemy's pulse the enemy beside it, never the shooter.
- `a_wreck_is_not_struck`.
- `the_ring_finishes_on_the_end_screen_and_strikes_nothing`.
- `a_round_with_the_emp_replays_bit_for_bit` - at night, with a tower and
  enemies, the drawable state frame by frame.
- `the_spawn_swap_hands_out_the_emp_by_its_share`.
- `an_enemy_takes_a_crate_only_on_shells_and_not_while_its_special_is_offline`.
- AI on a whole round: `an_emp_enemy_pulses_a_seat_in_its_ring_after_its_crackle`
  (the crackle `emp_tell_seconds` before the pulse, used on the seat),
  `the_emp_holds_for_an_ally_and_for_its_own_tower`,
  `a_training_dummy_never_pulses`,
  `the_closer_leaves_a_bare_seat_by_day_and_closes_in_at_night`,
  `enemies_keep_out_of_an_armed_seats_ring` (one inside backs out and
  never comes back within the ring's reach, crossing back over the berth
  at most once), `a_seats_emp_is_a_danger_only_while_armed`,
  `an_ally_backs_out_of_a_held_pulse_and_then_it_goes_off`,
  `an_emp_enemy_stops_asking_for_its_ring_after_its_patience`,
  `a_hurt_enemy_does_not_flee_for_a_crate_inside_an_armed_seats_ring`,
  `a_tank_backing_out_walks_out_rather_than_through_a_portal` (the exit
  past a wall, reachable only through a portal it cannot hop yet; it takes
  one on its own side and keeps to it as its bearing turns).
- The disabled brain: `a_disabled_enemy_spots_nobody` (the arena's shared
  alert), `a_disabled_enemy_neither_spots_nor_relays_on_a_field_map`,
  `a_disabled_enemy_keeps_its_target_until_it_reboots` (the retarget
  pass), and in `mechanics_tests` the straggler's exemptions
  (`a_guard_keeping_its_frog_a_burning_hull_and_a_disabled_one_are_never_rolled_in_again`);
  each fails with its rule taken out.
- The rest: `a_disabled_enemy_has_no_lane_warning` (the indicators),
  `the_weapon_slot_flickers_offline_while_the_special_is_down` (the HUD's
  slot and `offline_lines`), `an_offline_tower_is_no_trouble_for_the_hammer`.

**The rule and the dangers** (`ai::emp_rule_tests`, a made-up `EmpSense`
and dangers on an open field): `a_seat_in_the_ring_is_pulsed`,
`it_holds_off_box_beside_its_tower_into_an_ally_and_as_a_dummy`,
`the_generic_tiers_never_fire_the_emp`,
`a_closer_approaches_only_a_seat_worth_it` (not a crowded one, not one
hidden from it unless it just shot it),
`a_tank_backs_out_of_a_danger_not_its_own` (its own ignored, one nobody
owns kept out of, the latch), `a_tank_backing_out_keeps_the_exit_it_chose`
(the point held while out of every danger and walkable, chosen again
past the field's edge or inside the danger, let go when clear),
`a_tank_in_a_crackles_slack_band_stops_and_deeper_backs_out`,
`a_discs_exits_and_posts`.

**The commander** (`simulation::command`): `a_clearer_nudges_its_allies_out_of_its_ring`
(the larger offset, round a wall; a busy unit, a wreck and a seat left
alone), `a_disabled_unit_never_gives_way`.

**Pure parts**: `emp.rs` - `box_reach_is_the_distance_to_the_nearest_point`,
`seat_value_adds_its_parts`, `a_pulse_runs_out_to_its_reach_and_is_gone_after_its_linger`,
`the_ring_is_on_the_grid_in_its_ramp_and_gone_by_its_end`,
`the_ring_never_draws_past_its_reach`, `the_sparks_are_hashed_and_thin_at_the_end`,
`the_tell_is_on_the_grid_and_pure`, `lamp_sparks_are_gone_in_their_time_and_the_post_comes_back`,
`the_droop_side_is_hashed_and_both_sides_come_up`; `missile.rs` -
`a_dead_missile_falls_and_arrives_a_dud`; `lamp.rs` - `a_dark_post_draws_no_flame`.

**Online**: `net::apply` - `an_emp_pulse_reaches_the_replica`,
`an_emp_pulse_this_client_drew_is_not_drawn_again`,
`what_an_emp_leaves_reaches_the_replica_and_ends_with_the_rooms` (a
disabled enemy, an offline special, an offline tower, a dead missile and
the posts out, compared on the drawable state, and their end);
`net::predict` - `an_emp_pulse_is_drawn_on_the_press_and_takes_the_special_offline`,
`a_refused_pulse_gives_the_special_back`; `net::rig` -
`a_seats_pulse_is_drawn_on_the_press_and_only_once` (a whole
`OnlineRound` over a 60 ms link: the ring and the special offline on the
press frame, the room's pulse never drawn again),
`a_seats_pulse_reaches_the_replica_once` (through `Lockstep`),
`an_enemys_emp_reaches_the_replica` (crackle, ring, the seat drawn
disabled); `net::events` - every new variant sent or on `NOT_SENT`, and
round-tripped; `net::delta` - the new fields in the random snapshots, the
size bound re-measured (a full snapshot 478 B from 456: three bytes a
tank and one for the round).

**Tools and text**: `devserver::set_tank_arms_the_emp_and_takes_it_offline`;
the editor's PICKUP category counts fifteen; `every_language_fits_every_budget`
measures each line of the offline words; `corner_tests` pins
`WEAPON_SLOT_W`; the armory's thumbnail pin is re-baselined.

Not written as tests, and read off the code and the probe instead: a
rolling-in tank not
struck (`strike_emp` skips a `RollIn`), frogs, lanterns, crates, grenades
and shots left alone (nothing in the strike walk reaches them), a
burning tower burning on while offline (`tower_upkeep` runs before the
offline branch).

## 11. Probe

- **Defaults first**: `just probe-fixtures` and `just probe-fields`
  unchanged, passing their recorded ceilings untouched.
- **With the crate**: the same two sweeps with `--crate emp_burst`. AFK:
  enemies on shells collect it and pulse the AFK seat when it comes into
  their ring; by day their closers do not close in on the bare seat
  (worth 1, under the approach's 2), which is the rule.
- **Armed enemies**: the same sweeps with `--tuning armed.json`,
  `{"enemy_special_weapon_chance": 1.0,
  "enemy_special_weapon_emp_share": 1.0}`; then again at night (`armed`
  plus `"weather_override": 1`), where every closer closes in on the bare
  seat to pulse it, and with the commander (`armed` plus `"c2_enabled":
  true`), where the clearing is an order too.
- The probe's tank line gains `emp=` (charges) and ` dis=true` (while
  disabled); its fire tuple counts the charges, so a pulse is a trigger
  pull for `FIRED_RECENTLY_FRAMES`. A disabled tank is out of its own
  hands (`OUT_OF_ITS_HANDS`, `TankSnapshot::disabled`): no anomaly reads
  it while it is, and every window over its motion starts over where it
  stands when it reboots (`TankTrack::rejoin`) - the coast is the pulse's
  doing, not the AI's. A tank waiting outside a danger (`kept out`,
  `TankSnapshot::kept_out`) is a deliberate hold in `HOLDS`.
- **The bar**: `offbox-fire` 0 in every run; an exceedance is read round
  by round from its `ANOMALY` lines, and one the EMP's own action causes
  is fixed, not re-baselined.

**Recorded 2026-10-07, after review** (debug build, `--rounds 10 --seed
1000`; the fixtures at 1800 frames, the fields at 3600; totals over the 9
fixtures and the 7 field maps, minutes of round in brackets; "as built" is
the branch before its review, rebased on the hammer's reviewed head):

| Run | Fixtures | Fields |
|---|---|---|
| Defaults | border-stuck 4, jitter 32, spin 3, churn 34, clustering 10, pile-up 6 - every map's output byte for byte the hammer's (`15808dc`) | border-stuck 11, jitter 108, spin 24, churn 83, clustering 12, wall-grind 1, pile-up 8 - byte for byte the hammer's |
| `--crate emp_burst` | as the defaults (no weapon slot) | spin 21, clustering 37, pile-up 23, tank-grind 3, never-arrived 1 (24.4 min); as built stall 3, low-progress 4, tank-grind 3, never-arrived 1, spin 27 (24.9) |
| Mix: chance 0.5, EMP share 0.5 | spin 8, low-progress 2, tank-grind 1, clustering 38, pile-up 23 (16.4) | stall 1, low-progress 1, never-arrived 1, tank-grind 2, spin 18 (23.7) |
| Every enemy armed, by day | stall 2, tank-grind 1, spin 24 (42.4); as built low-progress 10, spin 13 | stall 2, low-progress 5, never-arrived 5, tank-grind 9, spin 79 (60.6); as built low-progress 7, never-arrived 4, tank-grind 12, spin 74 |
| Every enemy armed, night | stall 1, low-progress 2, never-arrived 2, tank-grind 2, spin 19 (40.7) | low-progress 1, never-arrived 4, tank-grind 2, spin 79 (52.9) |
| Every enemy armed, commander on | stall 1, low-progress 2, never-arrived 3, tank-grind 2, spin 12 (42.8) | low-progress 4, never-arrived 2, tank-grind 13, spin 62 (61.9) |

`offbox-fire` 0 in every run, and no shot, missile lock or hit on a seat
from off its box.

**The yardstick for long rounds.** A pack whose trigger is the EMP fires
nothing at a bare seat outside every ring, so its rounds run to the frame
cap, and every kind is counted per tank per stretch of time. The fair
comparison is the hammer's: a shells pack over as long - `--mission
destroy` and `player_armor_factor` 0.1 - against the same with every enemy
on the EMP:

| Sweep | min | spin | stall | stale-start | never-arrived | tank-grind | low-progress |
|---|---|---|---|---|---|---|---|
| Fixtures, shells pack | 44.1 | 22 | 0 | 0 | 7 | 12 | 8 |
| Fixtures, EMP pack, as built | 45.0 | 26 | 1 | 0 | 7 | 3 | 11 |
| Fixtures, EMP pack, review | 45.0 | 28 | 1 | 0 | 7 | 3 | 3 |
| Fields, shells pack | 66.6 | 44 | 1 | 0 | 1 | 18 | 8 |
| Fields, EMP pack, as built | 70.0 | 47 | 1 | 0 | 3 | 4 | 12 |
| Fields, EMP pack, review | 70.0 | 53 | 1 | 0 | 3 | 7 | 1 |

**Read round by round.** Each run was repeated with a probe that adds to
every `ANOMALY` line the AI tiers its tank ran over the 180 frames before
(and the EMP rule's arm, the dodge, a wait outside a danger, a disabled
brain) - a scratch build, not in the tree:

- **The EMP's own bugs, fixed in review** (decisions 29 to 31): as built,
  low-progress, grinds and never-arriveds were tanks stuck dodging - turning
  up and down on the spot between two exits by archipelago's lake, turning
  left and right on a portal's edge it could not hop through, pressing for
  seconds into a tank in its way, sliding its exit off the field's edge,
  driving away from its way out with the ally it backed from in front of
  it - and a hurt tank fleeing in and out of an armed seat's ring for a
  crate. After review, in the crate, mix, armed and long runs every stall,
  low-progress, tank-grind and never-arrived is the tree's own tiers -
  flee into wrecks and dead ends, guards on their beat, chase and attack
  in corridors, patrol - as in the shells pack, but one never-arrived on
  props (0x3f1) whose last three seconds were the attack tier.
- **What remains with a dodge in it** is spins: 1 of 24 with the crate, 4
  of 26 in the mix, 21 of 103 armed, 30 of 81 in the long rounds, 32 of 98
  armed at night. A tank backs out of an ally's crackle or held pulse and
  comes back to the fight - a reversal and a reversal back, which the
  probe counts as a turn of 360 degrees - or circles a fellow enemy on its
  way out. That is decision 26 at work in a pack where every enemy carries
  the EMP, the armed sweep's extreme (question 3); per minute of round the
  field maps' spins run at 0.76 against the shells pack's 0.66.
- **Night and the commander** each leave a few. At night every seat is
  worth the approach (the night's +1), and on maze (0x3f0) an EMP closer
  ground for three seconds behind an ally holding a one-cell corridor - the
  corridor jam the tree's tiers have. With the commander on, its nudge
  and the dodge both moved an ally out of a ring and in a crowd disagreed
  (grinds and low-progress with a dodge in their window, fixed in review:
  decision 34); after it, every low-progress is the tree's, and the three
  grinds with a dodge in their window are mostly attack and flee. The
  commander is off by default.
- **Clustering and pile-up** rise with the EMP's design, as they did as
  built: an EMP carrier with nothing in its ring holds its attack post
  lined up and does not fire (by day a bare seat is not worth the
  approach), so the pack behind it stacks up (question 1).

## 12. Interactions, decisions, what is left out

### Interactions with what ships

| With | What happens |
|---|---|
| Rainbow shield | Popped (`ShieldBroken`), gone, as one spent is - on any side the ring reaches |
| Heat shield, speed boost, ooze coat | Left alone |
| Portals | The ring does not pass through; a coasting enemy that drives into one teleports (`on_teleported` clears its place memory as ever) |
| Water, ice | Crossed; a coasting enemy wades a ford, slides on ice, stops at a deep shore |
| Rain, sandstorm gusts | Nothing to the pulse; a coasting hull drives in the gust's frame like any |
| Night, storm | Lights out matter: a disabled seat's screen goes dark but for its sparks and lanterns; every lamp post dark for ten seconds; the AI's +1 |
| Dusk, fog | Not night: the posts stay lit |
| Lamp posts, lanterns | Posts dark at night for `emp_lamp_seconds`; lanterns lit |
| Towers | Offline for `emp_tower_seconds`, either side; fire burns on; a player tower offline is no detour for the enemies; an enemy tower in reach keeps an enemy from pulsing |
| Missiles | Fall dead as duds |
| Shells, bullets, plasma, the laser, the flamethrower's jet already out, grenades | Left alone; a held flame goes out on its tank |
| Frogs (both sides) | Left alone: organic |
| Crates, drums, fires, oil, ooze, glass, walls, trees, grass | Left alone |
| Range boards (docs/range-target-prd.md) | Left alone: a board is wood and paint, nothing electric, and the ring passes it as it passes every tile |
| Lava, the volcano | Crossed; a coasting enemy can drive into a lava ford or a hot bank and burn |
| Field maps | The sight box binds the pulse whole; a disabled far tank coasts, is not woken and is not rerolled; the director feels a popped shield as a seat's loss |
| Waves | A wave tank rolling in is not struck; a disabled wave tank still counts toward the wave |
| The couch and its split | A seat's pulse disables its teammate in the ring (decision 3) |
| Training | `drop = ["emp_burst"]` works by its name; a dummy never pulses |
| The C2 commander | Clears an ally out of an EMP tank's ring first; never orders a disabled tank |
| Online | §8 |

### Interactions with the sonic hammer (built here)

| Hammer | EMP |
|---|---|
| An enemy's hammer tell | Lapses when the ring reaches it: no blast |
| A seat's or an enemy's hammer | Offline for `emp_disable_seconds`: the trigger fires shells, `WPN OFFLINE`; the dish's module shows its idle cell |
| A hammer's shove on a disabled enemy | Skids as any hull; once the skid ends it goes on coasting on its last intent |
| The hammer's trouble (`hammer_senses`, `lands_in_trouble`) | An offline enemy tower's reach and a disabled enemy's lane are no trouble |
| The hammer's closer (`Approach`) | Does not close in on a seat inside a danger (one carrying an armed EMP) |
| A hammer enemy near an EMP enemy's crackle | Backs out like any ally |
| The wave and the ring | Independent: neither stops, shadows nor triggers the other |
| The online claim | One pending claim per kind: a seat's hammer and EMP presses claim their own events |
| The probe's holds | A tell, a skid and a wait outside a danger are deliberate holds (`HOLDS`); the disabled state is out of the tank's hands (`OUT_OF_ITS_HANDS`) |

### Decisions taken

1. **The ring travels** at `emp_ring_speed`, striking as its front
   arrives - the sonic wave's rule, so what is drawn is what is ruled.
   Faster than sound (a fifth of a second to full reach): an EM pulse is
   not slow, but an instant one would strike before its picture reached
   the hull. Rejected: instant.
2. **Nothing stops it** - walls, iron, towers, water. An EMP is not sound,
   and the issue's counter is range; it also gives the EMP the use the
   hammer lacks, behind cover, and keeps the AI's measure a distance.
   Rejected: iron as a Faraday cage. *For Oto.*
3. **The pulse is blind**: it disables every tank and tower in reach
   whatever its side, the shooter alone excepted - a seat's pulse takes a
   teammate's special and shield and its own side's towers in reach, as an
   enemy's takes its allies'. The issue's "friendly fire is real" read as
   a property of the pulse, not of the AI. Rejected: sparing the seats'
   own side. *For Oto.*
4. **The shooter is the eye of the storm**: its special goes offline for
   `emp_disable_seconds` and nothing else does - lights, shield, turret
   and drive stay. That offline is the EMP's reload.
5. **"Offline" means the trigger falls back to shells**, the stock kept,
   as when a special runs dry - read from "the stick and shells still
   work". A burst, a volley and a held flame stop; a twin plasma's paid
   second bolt fizzles; a twin's second shell leaves. Rejected: a dead
   trigger (no shells either).
6. **Missiles fall dead as duds** - no blast. "Fall dead" read as dead,
   and a pulse is the counter to a volley. Rejected: bursting where they
   land (which would rain the volley on the shooter). *For Oto.*
7. **The at 11 is night only, the posts only**: `Night` and `Storm` (what
   `nightfall` becomes), every lamp post on the map dark for ten seconds;
   by day they are not lit. Lanterns are flames and stay lit - the seats'
   light when the posts go out. *For Oto.*
8. **Only the rainbow shield pops** - an energy field. The heat shield is
   a coat, the speed boost a burn of fuel, ooze a slime.
9. **Grenades, plasma bolts, portals, crates, frogs and fish are left
   alone**: a burning fuse, a bolt already loosed, an anomaly, wood,
   organic.
10. **A disabled enemy's brain is frozen, not paid back**: no thinking, no
    clocks, no think debt; the reboot clears the stuck clock and the
    heading, so a coast into a wall reads as nothing. Rejected: the
    far-tank debt (a three-second think after reboot would read the coast
    as stuck evidence).
11. **A disabled enemy is deaf and blind**: it spots, relays and takes
    orders from nobody, and keeps right of way with the commander.
12. **The sag is an enemy's** - a seat's gun still fires; drawn only.
13. **Lights out is drawn by day too** (the lamps darkened in black, the
    shadow convention), so the state reads without night.
14. **The AI fires on any seat in the ring and goes after the ones worth
    it** (integers: a seat 1, a shield, a special, night and a player tower
    1 each; fire at 1, approach at 2): "at least one seat, or a player
    tower" is the condition to fire, the weights its preference - they pick
    the seat a pulse is used on and gate the closer's approach, so by day
    a bare seat is pulsed where it stands but not hunted down, and at night
    every seat is. Never beside its own tower, never into allies (it has
    its ring cleared - they keep out of it, and the commander nudges them
    out), never from off any box in reach. Rejected: firing only at 2 (an EMP enemy would be a quiet tank
    against a bare seat in daylight).
15. **Dangers, with a light surcharge under them**, for the AI's
    reaction: an armed seat's disc and an ally's crackle are kept out of
    by a tier and by keeping every steering target outside, which also
    pushes out a tank already inside; the rod and the rail add their
    shapes. Built, that alone hunted the edge: a route to a point outside
    still cut across the disc, the dodge (a reversal) never turned the
    tank round under steering's commitment, and the seat steered at with
    no slot - the disc's own middle - had its way out swing with the
    tank's facing. So the dodge drives straight onto its route out, a
    way out is the first reachable exit (never one off the field), the
    seat's is a post on its axes inside its sight box, a tank that
    reached its moved point waits there (`kept_out`), and the cells
    inside a seat's disc cost `enemy_danger_route_cost` (2) more to the
    router. In the test that found it, an enemy that crossed back over the
    berth four times, once to 32 px from the seat, now never comes back
    within the ring's reach. Rejected: a surcharge alone (it never moves a
    tank standing inside).
16. **A seat's EMP is a danger only while armed and seen**: offline, it
    is none - the window its cost opens; hidden in grass, it is a trap.
17. **The crate spills** rather than cooks off.
18. **Crate ink cobalt** (`#4F6BFF`). *For Oto*, with a screenshot of the
    crate beside the other fourteen in Phase 2.
19. **A tower pack mends, it does not reboot.**
20. **`WPN OFFLINE` in the slot**, two lines at the font's own size,
    alternating with the dimmed readout: the slot holds 64 pt and the
    words do not fit in one line at any size the font draws.
21. **The wire names what a tank carries** (`TankState::weapon` = the
    special) plus its magazine and both timers, so the HUD, the module
    and the predictor all read the truth during an offline.
22. **The name is `emp_burst`** (crate, `Fired`, `--crate`, `drop`), as
    the hammer is `sonic_hammer`.
23. **A disabled tank keeps its engagement slot**: an outage of three
    seconds is not worth reshuffling the pack's ring.
24. **`Danger::owner` is an `Option`**: `Some(slot)` is a danger its own
    tank ignores (a seat's EMP, an ally's crackle), `None` one every enemy
    keeps out of, the caller included (the rod's call circle, BB-41).
25. **The disabled state's readers are open lists**: the commander's
    `UnitView::busy` (`command::Busy::Disabled`), the probe's `HOLDS` and
    `OUT_OF_ITS_HANDS`, `Predictor::offline_left` read by the one gate in
    `pull_trigger`, and the strike walk's blocks - each a row or a variant
    a later weapon adds rather than a new code path.
26. **A pulse held for allies clears its ring with or without the
    commander, and the clearer fights on meanwhile**: its held pulse is a
    danger its allies keep out of. Built as the doc first had it - with
    the commander off, the tank simply held its fire; with it on, it held
    still for the nudges - a pack of EMP tanks round a seat each held for
    the others, pressed in and stalled, and two clearers in each other's
    ring would never have moved. Rejected: firing into allies (decision 3
    keeps the pulse blind; the AI is the one that avoids it).
27. **An ally's crackle has a slack band** (`Danger::slack`, its berth): a
    tank in it stops rather than turning round, since the crackle is over
    in half a second.
28. **The sag is 15 degrees, not 8**: at 8 the screenshots showed it only
    under a close zoom.
29. **The dodge walks out through steering, to an exit held as a point**
    (review). Built, it committed straight to its route's first step every
    tick, which read round by round as most of the EMP's own anomalies: an
    exit chosen afresh each tick flipped between two whenever the nearer
    sat on the edge of a reachable cell (a tank turning up and down for
    three seconds by archipelago's lake), and slid off the field's edge as
    the tank walked to it, the exits turning with its bearing; a route
    through a portal the tank could not hop yet handed out the far portal
    from the near one's edge; and with no avoidance or stuck escape a tank
    pressed into a tank in its way for seconds. Now the exit is a point
    kept while it stays out of every danger and walkable on foot, steering
    drives it (`Brain::steer_out`), and only the turn from a heading that
    leads away from the way out is put on its heading. Rejected: a
    sharper version of the direct commit (each fix re-opened another of
    the cases).
30. **Flee and retreat leave a crate inside a danger alone**, as the seek
    tiers do: a hurt tank whose crate stood in an armed seat's ring drove
    in, was backed out with its heading committed away and ran to the far
    edge of the field before coming round again.
31. **A pulse held for allies is held for `emp_ai_clear_patience_seconds`
    (3) and no longer** until the ring is clear: an ally that cannot get
    out (penned, wedged against another tank) was kept backing out for
    ever while the pack waited on the one ring. Past it the rule asks
    nothing - it still never fires into an ally (decision 3). Rejected:
    firing once the patience runs out; waiting for ever.
32. **The EMP keeps the closer's spot it had** (`Brain::close_spot`, on
    the line from the seat out to its engagement slot): the hammer's
    closers moved to `closer_spots` (square on the seat, out of each
    other's cones) in that weapon's review, but a pulse is a disc and
    `emp_ai_closers` is one, so any side serves and there is no other
    closer to keep clear of. Its approach reads the seat's concealment as
    the attack tier does.
33. **The armory's player tesla, iron block and brick stub start a row or
    two lower** (tesla 3,4; iron 20..21 x 3..4; brick 26,3..5): in a 1152
    x 576 window the HUD's corner clusters cover rows 0 to 3 on the west
    and 0 to 2 on the east, and the tower a seat's own pulse puts out
    stood under the vitals. Every reserved cell stays where it was.
34. **The commander leaves an ally backing out on its own**
    (`UnitView::dodging`): it nudges only the allies in a clearer's ring
    that are not yet dodging it - on the frame the clearer first asks, the
    ring is a danger only from the next - since the nudge's way and the
    dodge's route disagreed in a crowd and ground tanks into each other.

### Questions for Oto

1. **An EMP enemy is quiet by day against a seat that keeps away.** With
    the approach at 2, an EMP carrier holds its attack post outside a
    bare seat's ring and fires nothing (its trigger is the EMP), so a
    player who never comes within five cells by day meets a tank that
    neither shoots nor closes. That is the rule as reviewed (fire at 1,
    approach at 2); `emp_ai_approach_value` 1 makes every closer go
    looking. Keep 2?
2. Decisions 2, 3, 6, 7 and 18 (no Faraday cage, a blind pulse, duds,
    the posts only at night, the cobalt crate - see
    `09-crates-row.png`) stand as reviewed; the crate screenshot is the
    one decision 18 asked for.
3. **A pack that all carries the EMP dances.** Each enemy with a seat in
    its ring and an ally in it holds its pulse and its ring is a danger
    the allies back out of; with every enemy armed, the backing out and
    the coming back to the fight read as spins on the field maps (§11).
    That is decision 26 working; a pack of three or four EMP tanks is
    the armed sweep's extreme, not a round the defaults or the crate
    produce. Keep, or have allies keep out only of the pulse's own reach?
4. **On a phone the HUD covers the armory's top five rows** (an 844 x 390
    touch window: the corner plates stand 90 pt tall over an arena drawn
    at two thirds): the glass house and its shield, this crate at 7,4, the
    health crate, the player tesla, the lamp post at 11,4, the iron block
    and the enemy tesla - and the cells weapons 3 to 6 reserved at 29,2
    and 30,2, which a 1152 x 576 window already half covers. Lay the
    armory's north band out again for phones (a follow-up), or accept a
    play-test map that reads best on a desktop?
5. **An offline tesla reads brighter during its spark bursts**: the
    sparks' glow over its cyan dome. A dimmer glow over a tower, or none?

### Not in this PR

- Placing the EMP in the shipped levels - level design, a follow-up.
- A mark on the minimap for a disabled tank, an offline tower or the ring.
- Sound effects - the game has no audio yet.
- The AI using a pulse against a missile volley coming at an ally, and
  weighing the lamp posts it would put out at night (which hides the seats
  by them).
- A struck seat's own instruments going dark (its minimap, its arrows) - a
  HUD design of its own.
- Portals, plasma bolts in flight and grenade fuses going dead - each a
  mechanic beyond the issue.
- The drones' fall and the well's early collapse - BB-40 and BB-42 add
  their arms to the strike walk.
- The armory's layout on a phone (question 4).

### What this changed in the shared path

What this design took from the hammer's implementation beyond what its doc
gave - all of it done in this PR:

1. **The tell goes off only on a live trigger**: the tell's go-off check
   reads `tank.active_weapon() == tell.weapon` (with the EMP: the weapon
   online), not merely its ammo, so a special taken offline mid-tell
   lapses. The EMP also clears the tell when it strikes; the rail's and the
   rod's tells on a seat need the general check.
2. **"Carries" and "fires" through two accessors**: the hammer's new
   readers that mean what a tank carries (its `TankState::weapon`,
   `module_cols`' presence, `wants_pickup`, `WeaponSlot`, the swap's "drew
   a special") read one accessor this PR turns into `Tank::special`, and
   those that mean what the trigger fires read `active_weapon` - so the
   offline lands in one place.
3. **`pyro::Shape::Arc` draws a whole turn** without a seam (`to - from >=
   TAU`), for the ring.
4. **`Game::draw_press_show(seat, weapon, origin, facing)`**: a press
   show knows its seat and weapon, so the EMP's flashes its own tank's
   module (the pulse cell), and `Game::hold_seat_offline` writes the
   predicted offline into the drawn seat.
5. **`Predictor::pull_trigger` picks its arm after a per-weapon gate**, so
   the EMP's arm can read the weapon as the shell while `offline_left`
   runs - the top of the function reads the sandbox's arms through one
   place this PR adds the local offline to.
6. **`enemy_phase`'s collect pass has one spot before `field::mind`** for a
   branch that bypasses thinking (the disabled coast), sharing the coast
   branch's `control`/`tick_queued_shots`/`Pending` push rather than
   copying it.
7. **`act_special` applies a `Hold` without touching the fire timer**, so
   `Clear` (a `Hold` that also records `Ai::clearing`) holds for as many
   ticks as the clearing takes.
8. **The armory's reserved cell 7,4** for this crate, and the map's
   free cells 29,7, 31,16, 11,4 and 25,10 - now the EMP's.
9. **`SPAWN_SWAPS`** gains its entry, and the protocol goes to 17.
10. **The armory's north pieces a row or two lower** (decision 33), the
    hammer's doc and the thumbnail pin with them.
