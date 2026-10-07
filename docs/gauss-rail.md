# Gauss rail

BB-39, the third of the six weapons of BB-36. A special weapon from its own
crate (`pickup = "gauss_rail"`), one at a time like every other
(`Tank::take_weapon`): a crate loads `gauss_slugs_per_pickup` (4) slugs, a
re-pick refills to four, another weapon's crate replaces it. Seats and
enemies alike: an enemy takes the crate while it fires shells
(`Tank::wants_pickup`) and uses it by its own rule (§4).

Hold the trigger and the rail charges for a second and a half while the tank
slows to a crawl and the charge glows. Let go and a slug crosses the whole
field down the gun line in one tick, through brick, wood, glass, props,
trees, towers and every tank in the lane, until iron, a volcano's cone, a
training door or the field's edge stops it. The recoil throws the shooter
back about a cell. Let go early and nothing fires. Hold too long and it
vents. Counter: the glow gives it away - step out of the lane, or put iron
between.

The sonic hammer's doc (docs/sonic-hammer.md §3) and the EMP's
(docs/emp-burst.md §3.3) lay down the shared path this plugs into: the AI
hook, the enemy tell and its off-screen arrow, the online press show
claimed by input tick, the knock and its skid, the probe's `--crate`, the
spawn swap, `spawn_pickup`, the armory, the carried/fired split
(`Tank::special`/`active_weapon`), the disabled state and the AI's dangers.
This PR adds the **charge-and-hold firing pattern** (§3.3) - a trigger that
builds while held and fires on its release, on seats and enemies, locally
and online - which the rod from god's press-hold-release reticle (BB-41)
reuses, and the **lane** among the dangers an enemy keeps out of.

## 1. How it plays

### The charge

- **Press to start.** On the trigger's press edge, with a slug in the rail
  and its cooldown out (`Tank::fire_cooldown`), a charge starts
  (`Tank::charge`, §3.3). A seat's charge is its own tell: the ring, the
  motes drawn in to the muzzle and the muzzle's glow (§5) show from the
  press. Online the shooter's client draws it on the press frame (§8).
- **Held, it builds.** Each tick the trigger stays down adds a tick. It is
  **full** at `gauss_charge_seconds` (1.5) held, **overcharged**
  `gauss_overcharge_seconds` (1.0) past full, and it **vents**
  `gauss_hold_seconds` (2.0) past full: 1.5 s, 2.5 s and 3.5 s held at the
  defaults. Counted in whole ticks (`Charge::ticks`), so a room and a
  client agree to the tick (§3.3).
- **The crawl.** While a charge runs the hull's top speed is
  `gauss_crawl_pace` (0.2) of its own (`Tank::charge_pace`): 42 px/s for a
  seat's standard chassis, 32 for an enemy's. It is a lower top speed, not
  a lazier engine - applied where `drive_tank_with` scales the target by
  `intent.slow` - and `Tank::throttle` records it, so the probe's
  commanded-versus-achieved reads it as asked for. The stick still turns
  the hull: a seat may crawl round to a new lane mid-charge, and the lane
  turns with it. The crawl starts the tick after the press (the drive runs
  before the trigger in `drive_player`, as it does today) and ends on the
  release tick.
- **Released before full: nothing.** The charge **fizzles**
  (`ChargeEnd::Fizzled`): no slug, no slug spent, no cooldown, a little
  fizzle at the muzzle (§5). The next press charges from zero. Why nothing
  rather than a weak shot: §12, decision 1.
- **Released at full: the slug** (below), one slug spent,
  `gauss_reload_seconds` (0.6) before the next charge may start.
- **Released overcharged: the "at 11".** The slug cuts iron too, and the
  recoil throws the shooter twice as far and spins it round (below). An
  enemy never releases overcharged (§4).
- **Held past the vent: it vents** (`ChargeEnd::Vented`). No slug and none
  spent, a burst of steam off the module (§5), and the rail is hot for
  `gauss_vent_cooldown_seconds` (1.2): no charge starts until it cools.
  It never fires by itself - a charge that went off on its own would be a
  trap for a player walking into a teammate. The trigger, still held after
  a vent, does nothing: a new charge needs a new press.
- **It lapses** - no slug, none spent, nothing drawn - when the tank
  becomes a wreck, when the EMP disables it (docs/emp-burst.md, "its tell
  lapses"), when the trigger stops being the rail's (a crate of another
  weapon taken mid-charge, the special taken offline), and when the round
  ends. The first three clear the charge where they happen with no event;
  a trigger that is no longer the rail's ends it in `step_charge` with
  `ChargeEnded { end: Lapsed }`. A crate of the rail taken mid-charge
  refills and keeps the charge.
- **It holds through** a teleport (the charge is the trigger's, held by
  whoever holds it, and nothing about it is tied to where the hull stands -
  a seat crawling into a portal comes out with its finger on the trigger),
  a sonic hammer's shove (the skid carries the hull, its facing kept; no
  crawl while it skids, since its own drive does nothing then), a hit, a
  shield breaking, a speed boost (the crawl scales the boosted top speed),
  ooze (the crawl times the coat's pace) and a ford (times its pace).

### The slug

- **One instant trace.** On the release tick (`gauss::fire_charge`, queued
  like a laser shot and resolved by `Game::resolve_rails` right after
  `resolve_lasers`, before `step_world`), the slug is judged from the gun
  line's muzzle (`Tank::gun_line_muzzle`) along the hull's facing - never
  off-aim: no misfire skew for an enemy, no spread - and drawn from the
  rail module's muzzle (`tank_art::RAIL_MUZZLE`), the laser's rule. It
  runs `laser_reach(field)` px: every arena and most fields are crossed
  corner to corner, a larger field too.
- **The pierce list.** The trace is `Terrain::pierce_rewound` (§3.2): every
  candidate box it enters, in order along the line, up to and including the
  first **stopper**. Each thing appears once, at the first of its boxes the
  line enters. Order: entry distance, then the sweep's rank (seats > enemies
  > frogs > tiles > the edge), then owner slot or cell. The candidates are
  the sweep's own, with its own boxes: every seat's and every live enemy's
  hull and turret, grown by `gauss_half_width` (4) - and a seat's slug grows
  an enemy's by `player_shot_hit_pad_px` besides, as `sweep_rewound` does -,
  the frogs, every tile, the field's four walls. The shooter is never in it,
  even on a leg a portal brings back. A wreck is see-through, as to every
  shot; a tank rolling in through a gate is off the field.
- **What stops it**: iron, a volcano's cone and a training door
  (`Material::is_permanent`), and the field's edge. Overcharged, iron no
  longer stops it: the slug passes through, the iron unharmed (it is
  permanent, and the nav grid's layer and the linter's breach grid keep
  their meaning); the cone, a door and the edge still stop it.
- **What it does to each thing, in order**, with the slug's damage `d`
  starting at `gauss_damage` (120) for a seat's slug and
  `gauss_enemy_damage` (60) for an enemy's:
  - *A tank* (any side but the shooter's own hull): `d`, no roll, through
    `Tank::take_damage` - so a player's hull armour applies, and a rainbow
    shield soaks the whole of it: `spend_shield(d)` empties a shield of up
    to `d` and, a hit bigger than what is left being absorbed in full, the
    hull takes nothing, the shield almost always shatters (`ShieldBroken`)
    and the slug flies on. Seat on seat is scaled by
    `friendly_fire_damage_factor` as every hit is (`apply_hit`'s rule); a
    fellow enemy takes an enemy's slug whole, as it takes its shells.
    `mark_hit`, `credit` (the kill and `hit_by_seat`), `Event::Hit` with
    `cause: HitCause::Rail` at the entry point, a kill onto `f.kills`, and
    `Ai::notify_hit` on a surviving enemy. No knockback (the slug is
    through before it pushes) and no hop. Then `d *= gauss_pierce_keep`
    (0.8). At the defaults a seat's slug wrecks an enemy at full health even
    through three tiles of cover (120 x 0.95^3 = 103), takes 96 off the
    tank behind it and 77 off the third; an enemy's slug takes 46 off a seat
    (60 through the 0.77 armour).
  - *A frog* (either side's): `gauss_frog_damage` (20) times the slug's
    keep so far, no roll, `Frog::damage`, `Event::Hit`; no evasive hop
    (an instant shot is not dodged - the laser's rule, and the hop would draw
    RNG). Then `d *= gauss_pierce_keep`. Two slugs kill a frog.
  - *A tower* (any side's): `d` through `damage_obstacle` with
    `DamageCause::Pierce { dir }` - no deflect roll, the slug is not turned.
    A tesla (120) dies to a seat's first slug; a bio slush (130) and a gun
    tower (150) survive it and die to the next (`tower_died`). Then `d *=
    gauss_pierce_keep`.
  - *Brick, wood, glass, sandbags, fences, trees and lamp posts* die
    outright, whatever `d` is left: `DamageCause::Pierce` puts the tile's
    whole health on it through `Obstacle::damage`, which leaves a flammable
    plank or tree burning rather than broken, as every killing blow does.
    No fence one-shot roll, no sandbag pass-over roll. The ordinary death
    path: `ObstacleDestroyed`, the rubble thrown along the slug's line
    (`BlastShape::Shot { dir }`), the edge masks. A tile already burning,
    already destroyed this tick, or a drum with a fuse burning is passed
    through untouched. Then `d *= gauss_tile_keep` (0.95).
  - *A drum*: dies at once, so it goes off where it stands (a direct hit's
    pop: `obstacle_died` queues its blast, `Event::Blast`); a fuel drum's
    neighbours chain as ever. Then `d *= gauss_tile_keep`.
  - *Iron, overcharged*: nothing (permanent); it is on the list for its
    sparks (§5).
  - *The stopper*: an `Event::Hit` with no damage (`target` the tile or
    `HitTarget::Wall`, `cause: Rail`), which flashes it.
- **What it leaves alone**: crates and breakable crates (the slug flies
  over them, as shells do), wrecks, a seat's lanterns, shells, bullets,
  plasma, missiles, grenades, globs, lava bombs and flying drums in the
  air, ground fires, oil, ooze, water and lava (it crosses both; deep water
  and deep lava stop hulls, never a shot), portals' anchors, scorches,
  rubble, tread marks. Fish dart away from its line (§5); the tall grass
  along it lies flat for a moment (cosmetic, §5).
- **Portals, leg by leg** (`portal_shots`, docs/teleporting.md): the laser's
  walk. A leg is judged only up to the point it goes into a portal
  (`portals::shot_entry`, `portal_shot_radius`; a slug fired from a tank
  standing on a portal is leaving it); the exit is one draw from the round's
  RNG (`draw_shot_exit`, only where a slug goes in), `ShotTeleported { id:
  None }` is logged and the next leg leaves the exit at the same offset on
  the same heading with the reach it has left, the damage `d` carried over,
  at most `portal_shot_max_passes` legs. Each leg pierces its own list.
- **The recoil**: on the release, the shooter is knocked back along its
  facing (`gauss::recoil_hull` in `resolve_rails`, before `step_world`,
  through `sonic::knock_hull` - the hammer's shove-and-skid,
  docs/sonic-hammer.md §3.6) at
  `v = sqrt(2 * skid_friction(1) * 32 * gauss_recoil_cells) / m^gauss_recoil_mass_exponent`,
  where `skid_friction(1)` is the skid's friction on dry ground
  (`sonic_skid_decel`) and `m` the chassis's mass factor - so on dry ground
  a standard chassis slides `gauss_recoil_cells` (1) cell, a scout 41 px, a
  titan 17 px. It is a skid, so the slide is the same whichever way the
  hull faces and its drive does nothing until it ends; wet ground, a ford
  and ice slide it further (the skid's grip floor). The shooter's own
  recoil is **not** put on `Frame::shoves` (a client that owns its hull
  kicks it itself on the release, §8, as it does a shell's), but the room
  still allows the owned hull the knock's speed (`Shoves::allow_knock`, the
  validator's `SeatKnock`).
- **Overcharged** (the "at 11"): the slide is `gauss_overcharge_recoil_factor`
  (2) times as far (the speed times its square root) and the hull **spins
  round**: its facing turns half a turn (`Tank::rotation`), and for as long
  as the skid runs (`Tank::spin`) the stick neither turns nor drives it;
  the hull and the turret swing round at their own turn speeds
  (`ease_visual_rotation`), so the spin is seen. After it the tank faces
  back the way the slug came from.
- **Events**, all on the release tick: `Event::Fired { weapon: "gauss_rail" }`
  (from the trigger, `gauss::charge_trigger` in `drive_player` or the enemy
  collect pass), then per
  leg `Event::RailSlug { slot, seat, leg, x0, y0, x1, y1, portal,
  overcharged, pierced }` (`resolve_rails`: drawn start, end, whether the
  leg ended in a portal, and every pierce point with what it was), and the
  ordinary events of what it did (`Hit`, `ObstacleDestroyed`, `Blast`,
  `ShieldBroken`, `ShotTeleported`, `Wreck`). `Fired` first, `RailSlug`
  after, in one tick - which the online claim reads (§8). A charge's start
  and end log `Event::ChargeStarted { slot, weapon }` and
  `Event::ChargeEnded { slot, weapon, end }`.

### A wreck, an EMP, a shove, a teleport mid-charge

| Mid-charge | What happens |
|---|---|
| The tank is wrecked | The charge is cleared the tick it is a wreck (`tick_timers`): no slug. A dead hand releases nothing |
| The EMP's ring reaches it | `Tank::disable` clears the charge; its special is offline for `emp_disable_seconds`, so no new charge starts until it is back |
| A sonic hammer's shove | The charge holds; the skid carries the hull, facing kept; the crawl resumes when the skid ends |
| A teleport | The charge holds (a seat's and an enemy's); the lane is wherever it now faces |
| Another weapon's crate | The charge lapses next tick (the trigger is no longer the rail's: `ChargeEnded { end: Lapsed }`) |
| A rail crate | Refills the slugs; the charge holds |
| A hit, a shield breaking | The charge holds |
| The round ends | `end_round` clears every charge; nothing fires on the end screen |

### On the end screen

`player_phase` and `enemy_phase` do not run, and `end_round` has lapsed
every charge, so nothing charges or fires. The slugs already fired finish
their picture (`tick_effects`), and the bursts they started play out.

## 2. Where it lives

| File | What |
|---|---|
| `src/gauss.rs` (new) | The weapon's headless half: `RailSlug` (a leg as drawn: start, end, portal, overcharged, pierces, age, seed), `Pierce`/`Pierced`, `ChargeEndFx`, `muzzle` (the module's bore, `tank_art::RAIL_MUZZLE`), `damage(owner, t)`, `recoil_speed(t, mass_factor, overcharged, friction)`, `module_cell`, and the composers `compose_charge`, `compose_slug` (glowing), `compose_slug_lit` (the chips), `compose_end` (pure, `pyro::Shape`s) |
| `src/simulation/gauss.rs` (new) | The world half: `charge_trigger` (the pattern's room side, seats and enemies, §3.3), `fire_charge`, `PendingRail`, `recoil_hull`, `Game::resolve_rails` (the legs, the pierce walk, the events, the recoil), `stops_slug`, `pierce_hit`, `rail_recoil`, `rail_show` and `charge_end_show` (the cosmetic halves, which a replica's events and a client's own release call too), `any_rail_charging`, `rail_lanes` (`RailLane`, every charging rail's lane this frame), `rail_dangers`, `gauss_senses` (`GaussSeat`; what the AI is handed, §4), `set_seat_hold`/`seat_hold_report`, `SeatCharge` |
| `src/simulation/hits.rs` | `Terrain::pierce_rewound` (every box entered, in order, to the first stopper), `Terrain::rail_tiles` (the tiles a slug enters and its stop, for the drawn world) |
| `src/simulation/weapons.rs` | The `ActiveWeapon::GaussRail` dispatch arm (empty: a charge weapon fires through `fire_charge`) |
| `src/simulation/props.rs` | `DamageCause::Pierce { dir }` |
| `src/simulation/mod.rs` | `Frame::{pending_rails, charge_ends}`; `Game::{rail_slugs, charge_ends, seat_hold}`; `resolve_rails` after `resolve_lasers`; `drive_player`'s trigger by `ActiveWeapon::trigger`; `enemy_trigger` routing a charge weapon's trigger every tick (its press edge from `Tank::trigger_held`); `coast_enemy` keeping a charging trigger down; the crawl in `drive_tank_with`; `tick_timers` (`spin`, `rail_flash`, a wreck's charge); `predict_seat_with` (the charge step and its recoil); `end_round`; `tick_effects`/`tick_presentation` (slugs, charge ends, a replica's charges); `Event::{RailSlug, ChargeStarted, ChargeEnded}`, `HitCause::Rail`; `Pending::charging`; `TankSnapshot::{gauss_slugs, charging}` |
| `src/simulation/command.rs` | `Busy::Charging`: never ordered, keeps right of way |
| `src/simulation/sonic.rs` | The hammer's trouble: a charging enemy rail's lane (§12); `knock_hull`/`knock_with`; the swap's table entry |
| `src/simulation/present.rs` | `PresentWorld::rail_trace` (`RailTrace`: the drawn world's stop and what a slug goes through), `Game::draw_rail_press`, `Game::{seat_charge, set_seat_charge}` |
| `src/simulation/replica.rs` | `DrawableTank::charge` |
| `src/tank.rs` | `gauss_slugs`, `charge`, `spin`, `rail_flash`, `trigger_held`; `Trigger`, `Charge`, `ChargeRule`, `ChargeStage` (`name`), `ChargeEdge`, `ChargeEnd`, `ticks_of`, `CHARGE_HOLD_SPARE_TICKS`; `ActiveWeapon::GaussRail` (`name`, `full_load`, `tell_seconds` none, `trigger`, `charge_rule`), `SPECIAL_WEAPONS`; `step_charge`, `lapse_charge`, `charge_pace`, `windup`, `kick_rail`; `weapon_ammo`/`take_weapon`/`empty_stock`/`wants_pickup`; the spin lock in `control`; the module's cells in `module_cols`; `Dir::opposite` |
| `src/pickup.rs` | `PickupKind::GaussRail` (`gauss_rail`, row 15, its ink, cooks off) |
| `src/ai.rs` | `SpecialSense::Gauss(GaussSense)`, `GaussLane` (`score`, `counts`, `target_along`), `gauss_rule`, `gauss_charge_rule`, `windup_rule`'s charge arm, `SpecialUse::{Charge, Release}`, `generic_fire(GaussRail)`; `SEEK_SPECIALS` gains the rail; `DangerShape::Lane` (`depth`, `exits`, `posts`, `middle`, `Danger::is_lane`, `lane_offsets`); the edge hold (`enters_lane`, `Ai::kept_out`) |
| `src/indicators.rs` | A charging rail's lane through cover (`TankView::lane`), `ArrowKind::Windup { lane }` and its ring, the hit arc down a slug (`hit_from`) |
| `src/hud.rs` | `HUD_GAUSS_COLOR`, the `weapon_color`/`weapon_pickup` arms, `WeaponSlot::charge` (`ChargeGauge`), `CHARGE_BLINK_HZ` |
| `src/render/hud.rs` | The charge gauge in the count's place (`draw_weapon_readout`) |
| `src/render/game.rs` | Charges, slugs and charge ends in their passes (§5); the dev stats lines |
| `src/weather.rs` | The charge's, the white frame's and the stop's light |
| `src/fx.rs` | Sparks out of the bore and off the stop of a slug; a rail `Hit` draws only its flash |
| `src/grass.rs` | `flatten_along` |
| `src/fish.rs` | The slug's legs among the scares |
| `src/pyro.rs` | The `RAIL` ramp, `RAIL_LIGHT` |
| `src/net/wire.rs` | `WeaponKind::GaussRail` (`drawn_on_press`), `TankState::charge`, `RailPierce` |
| `src/net/events.rs` | `WireEvent::{RailSlug, ChargeEnded}`, the `press_show` arm, `charge_started` on `NOT_SENT` |
| `src/net/encode.rs`, `src/net/apply.rs` | The charge field; `RailSlug`'s and `ChargeEnded`'s shows; this seat's `ChargeEnded` left out under `OwnShotsDrawn`; a rail hit's impact flash left out; the `kick_turret`/`drawn_muzzle` arms |
| `src/net/mailbox.rs`, `src/net/authority.rs`, `src/net/rig.rs`, `server/src/room.rs` | The hold report (§3.3): `Mailbox::hold_ticks`, `authority::take_hold` |
| `src/net/predict.rs`, `src/net/round.rs` | The charge in the sandbox (`charge_gate_open`, `charge_edge`, the history's trigger edges), `PressShow::{Rail, ChargeEnd}` (`RailPress`, `ChargeEndPress`), the shown seat's charge |
| `src/simulation/debug.rs`, `src/devserver.rs` | `set_tank`'s `gauss_slugs` and `charge`; the snapshot's `gauss` and `charge`; `spawn_pickup {kind: "gauss_rail"}` |
| `src/bin/probe.rs` | The tank line's ` rail=`/` chg=true`, the fire tuple, the `charge` hold |
| `src/editor/mod.rs` | `Tool::Pickup(PickupKind::GaussRail)` (`gauss_rail`) |
| `maps/armory.toml` | Its crates, a screen of cover and an iron block (§3.4) |
| `src/tuning.rs` | The `gauss` group (§6), one row in `enemies` |
| `lang/en.ftl`, `lang/sl.ftl` | §7 |
| `tools/punypalette.py`, `tools/spritegen/gen_crates.py`, `tools/spritegen/tankdesign/{kit,export,render,lines/vanguard}.py` | The art (§5); writes `static/crates_sheet.png`, `pickup_glyphs.png`, `tank_modules.png`, `tank_modules_glow.png`, `src/tank_art.rs` |
| `src/thumbnail.rs`, `src/maplint.rs` | The armory's pin re-baselined; the armory as it lints |
| `docs/` | This, `CRATES_SPEC.md`, `SPRITESHEET_SPEC.md`, `effects.md` (the `RAIL` ramp); `CLAUDE.md` |

## 3. The shared path

### 3.1 What this uses as the hammer and the EMP laid it down

Each item of the checklist (docs/sonic-hammer.md §3.0) gets its rail arm,
in the same places:

1. `PickupKind::GaussRail` (`#[serde(rename = "gauss_rail")]`, appended to
   `ALL`, row 15, `ink`, `cooks_off` true - slugs and charged capacitors,
   like the laser's and the plasma's crates), `PickupKind::weapon`
   (`Some(ActiveWeapon::GaussRail)`), `name`.
2. `ActiveWeapon::GaussRail` (`name` "gauss_rail", `full_load` =
   `gauss_slugs_per_pickup`, `tell_seconds` none - the charge is its tell),
   appended to `SPECIAL_WEAPONS`; `Tank::gauss_slugs` with its arms in
   `weapon_ammo`, `take_weapon`, `empty_stock`, `module_cols`.
   `wants_pickup` reads `special()`: an enemy takes the crate only while it
   carries no special (the EMP's rule).
3. The dispatch arm (empty; §3.3 `fire_charge`); the trigger in
   `drive_player` by `ActiveWeapon::trigger` (§3.3).
4. `pickup_phase` needs nothing.
5. `ai::SEEK_SPECIALS = [SonicHammer, Emp, GaussRail]`; `SpecialSense::Gauss`,
   the `special_rule` arm, `generic_fire(GaussRail) == false` (§4).
6. `hud::weapon_color` (`HUD_GAUSS_COLOR`), `hud::weapon_pickup`.
7. `gen_crates.py` (`KINDS`, `GLYPHS`), `PICKUP_INK['gauss_rail']`; the
   tankdesign module `gauss` and its anchor `tank_art::RAIL_MUZZLE` (§5).
8. `editor::TOOLS` and `Tool::name`; `tool-gauss_rail`,
   `tool-short-gauss_rail` in both catalogues.
9. `WeaponKind::GaussRail` (appended to `ALL`, both `From`s,
   `drawn_on_press` true), `Predictor::seed_gate`'s arm
   (`gauss_reload_seconds`), `apply::write_tank`'s ammo arm,
   `kick_turret` (`Tank::kick_rail`, the module's fire cell) and
   `drawn_muzzle` (none: the slug's first leg puts on its own muzzle
   ripple, `Game::rail_show`).
10. `debug::{TankDebug, TankPatch}`, `set_tank`'s schema, `TankSnapshot`,
    the probe's tank line and fire tuple, `render::game::draw_tank_stats`.
11. The armory's crates (§3.4); `SPAWN_SWAPS` gains
    `(ActiveWeapon::GaussRail, |t| t.enemy_special_weapon_gauss_share)`
    after the EMP's; the tuning group; `PROTOCOL_VERSION`.

And, as they are: the special hook's tier and `act_special` (extended
below), the wind-up's off-screen arrow (`ArrowKind::Windup`, fed by a
charge as by a tell through `Tank::windup`, below), the press show's claim
by input tick, the knock and its skid (`sonic::knock_hull`),
`Tank::special`/`active_weapon`, `Tank::disable`, the dangers and the
`dodge` tier (its held exit point and `Brain::steer_out`),
`spawn_pickup {kind: "gauss_rail"}`, the probe's `--crate gauss_rail`,
`HitCause` on `Event::Hit`, `pyro::Shape::Arc` (the charge's ring).

### 3.2 What this extends

- **`SpecialUse::Charge { face, why }` and `SpecialUse::Release { face,
  at_seat, why }`** - the hook's two uses for a charge weapon.
  `act_special` applies `Charge` as: face `face`, commit the heading, hold
  the trigger (`intent.fire = true`), no movement - a charging enemy
  stands its ground (§12, decision 23); `Release` as: face `face`, let the
  trigger go (`intent.fire = false` - the simulation fires the charge if it
  is ready), record `at_seat` in `Ai::shot_at_seat`; both record `why` in
  `Ai::special_why`. A `Charge` that would *start* a charge while
  `Ai::fire_timer` or the weapon's cooldown runs faces and holds with the
  trigger up (the hammer's pacing rule); one that starts it sets
  `Ai::fire_timer` to the weapon's own interval (`gauss_ai_fire_interval`).
  Once a charge runs, the timer is not consulted.
- **`special_rule`'s first arm** (the hammer's "the tell holds") becomes
  "a wind-up in progress belongs to its weapon's rule": a tank with a tell
  holds; a tank with a charge (`Tank::charge`) is answered by its weapon's
  arm, which never answers `None` while it runs. This is what keeps a
  charge from being released by accident: every other tier leaves
  `intent.fire` false, which the trigger would read as a release.
- **`DangerShape::Lane { at, from, dir, length, half_width }`** - the
  EMP's dangers gain the lane, as its doc anticipated (§4, "Reacting to a
  rail"): everything within `half_width` of the line from `from` (the
  charging tank's gun-line muzzle; `at` is its hull) along the cardinal
  `dir`, `0..=length` along it. `depth(p)` is `half_width - |p's offset
  across the line|` for a point along it and minus its distance past the
  nearer end otherwise (`lane_offsets`). `exits(p, clear, from, facing)`
  are nine points `half_width + clear` off the line: on the side the tank
  faces across to - a tank crossing the lane goes on over rather than
  turning back - else, for one facing along it, the near side (`p`'s, else
  `from`'s), at `p`'s distance along, a cell and two either way along it,
  then the far side the same. The dodge walks to the first one it can
  reach on foot out of every danger and holds it (`Ai::dodge_exit`,
  `Brain::steer_out`, as for the EMP's discs). `posts` stand beside and
  behind the charging hull, never ahead of it.
- **The edge hold**, for a lane: in `Ai::think`, after the tree, a step
  whose point `Tank::avoidance_radius + enemy_danger_clear_px` ahead along
  `move_dir` lies in a lane this tank does not own, while its own centre
  is in none, is not taken - `move_dir` is cleared, the hull keeps its
  facing and `Ai::kept_out` is set (`TankSnapshot::kept_out`, a hold the
  probe excuses) (`enters_lane`). A route across a lane that spans the
  field would otherwise walk a tank in, the dodge would push it back to the
  nearer side, and it would walk in again until the charge ended; this way
  it waits at the edge. The EMP's discs keep their own slack band. The
  dodge tier's own steering out of a danger is never held.
- **`Event::Hit::cause`** gains `HitCause::Rail`: `fx` draws such a hit's
  flash (`fx::Flash`, hull or tile) and nothing else - the slug's own
  picture carries its bursts (§5), so a hit is never burst twice.
- **`PressShow::Rail(RailPress)` and `PressShow::ChargeEnd(ChargeEndPress)`**,
  the predictor's arms, drawn by `Game::draw_rail_press` and
  `Game::charge_end_show`, and `WireEvent::press_show`'s `RailSlug` arm
  (§8).
- **`Terrain::pierce_rewound(world, players, shooter, p0, p1, half, past,
  iron_stops) -> Vec<(ShellTarget, f32)>`** beside `sweep_rewound`: the
  same candidates, boxes, pads and rewind, but every one the segment
  enters, sorted by (entry `t`, rank, owner slot or cell), cut after the
  first stopper (a permanent tile - iron only while `iron_stops` - or a
  wall). Every box is first tested against the segment's bounding box
  grown by the half width, so a cardinal slug pays the segment test only
  for the boxes along its row or column. `Terrain::rail_tiles(p0, p1,
  half, iron_stops)` is the tiles alone with the stop, which
  `PresentWorld::rail_trace` builds the drawn world's trace from.

### 3.3 The charge-and-hold pattern

BB-39 asks for charge-and-hold firing to be general. The pattern is a
**sibling of the tell, not a tell.** A tell is the simulation's countdown on
an enemy's decision: it commits, holds the tank still and goes off by
itself at zero. A charge is the trigger's own state: it builds for as long
as whoever holds the trigger - a seat's player or an enemy's rule - keeps
it down, the tank may crawl and turn while it builds, and it fires only on
the release. They share how they are drawn per weapon, how they lapse
(a wreck, the EMP) and the off-screen arrow (`Tank::windup`), and a tank
never has both.

**The trigger kinds.** Every weapon says how its trigger works:

```rust
/// How a weapon's trigger fires it (`ActiveWeapon::trigger`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Trigger {
    /// Once on the press edge: shells, plasma, grenades, the hammer, the EMP.
    Press,
    /// Every tick it is held, paced by the cooldown: the laser, the minigun,
    /// the missile pod.
    Auto,
    /// A stream while held: the flamethrower.
    Stream,
    /// Built while held, fired on the release (`Charge`): the gauss rail,
    /// the rod's reticle (BB-41).
    Charge,
}
```

`drive_player`'s `should_fire` match becomes `match weapon.trigger()`,
the same answers for every weapon that ships (a pure rewrite), and a seat
whose trigger is `Charge` - or whose tank holds a charge - calls
`gauss::charge_trigger` every tick whatever the trigger is (its release is
the trigger going *up*).

**The state, on `Tank`:**

```rust
/// A charge built on a held trigger.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Charge {
    /// The weapon it is for: a charge whose weapon is no longer the
    /// trigger's lapses.
    pub weapon: ActiveWeapon,
    /// Seconds the trigger has been down, the press tick included: one
    /// `PHYSICS_FIXED_DT` a tick in a round, the frame's time on a replica.
    /// A room's is the client's count (the hold report, below).
    pub held: f32,
    /// The ticks the round has stepped the charge itself, whatever a
    /// report said: what a report is held to.
    pub stepped: u32,
}

/// A charge weapon's timings in seconds held (`ActiveWeapon::charge_rule`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ChargeRule {
    /// Ready: a release fires.
    pub full: f32,
    /// Past this a release fires overcharged; `None`, never.
    pub overcharge: Option<f32>,
    /// Past this the charge is lost (`ChargeEnd::Vented`).
    pub vent: f32,
    /// The share of its top speed a charging hull keeps.
    pub crawl: f32,
    /// After a vent, how long before a charge may start again.
    pub vent_cooldown: f32,
}

pub enum ChargeStage { Charging, Full, Overcharged }
pub enum ChargeEnd { Fizzled, Vented, Lapsed }
pub enum ChargeEdge { None, Started, Held, Released(ChargeStage), Ended(ChargeEnd) }

pub charge: Option<Charge>,
```

`Charge::ticks()` is `held` over `PHYSICS_FIXED_DT`, rounded, and every
threshold compares ticks - `ChargeRule::{full, overcharge, vent}_ticks()`,
the knob over the step, rounded - so float drift never costs a tick, and a
room and a client agree to the tick. `Charge::{stage, progress}` (progress
0..1 to full) read the rule.

**The one state machine:**

```rust
impl Tank {
    /// One tick of a charge-and-hold trigger. `fire` is the trigger this
    /// tick, `pressed` its press edge, `open` whether a press may start a
    /// charge now (the caller's gate: the room's `fire_cooldown`, a
    /// client's local gate), `report` a room's count of the ticks the
    /// trigger has been down (the hold report, below). Pure state: no
    /// world, no RNG.
    pub fn step_charge(&mut self, fire: bool, pressed: bool, dt: f32, open: bool, report: Option<u32>) -> ChargeEdge;

    /// Lose a charge without firing; true if there was one.
    pub fn lapse_charge(&mut self) -> bool;

    /// The share of its top speed this hull keeps: its charge's crawl
    /// while one runs, else 1.
    pub fn charge_pace(&self) -> f32;

    /// The wind-up it shows - a tell's or a charge's: the weapon, its
    /// progress 0..1 and the facing it goes off along.
    pub fn windup(&self) -> Option<Windup>;
}
```

In order: a charge whose weapon is no longer `active_weapon()` lapses
(`Ended(Lapsed)`). With no charge, a press on an open gate of a charge
weapon starts one at `held = dt` (`Started`); anything else is `None`.
With a charge and the trigger down, `held += dt` and `stepped += 1` - with
a report, `held` set to it, held within `CHARGE_HOLD_SPARE_TICKS` of
`stepped` - and past `vent` it vents (`Ended(Vented)`), else `Held`. With a
charge and the trigger up (with a report, the count taken the same way):
before `full` it fizzles (`Ended(Fizzled)`), else `Released(stage)`. The
charge is gone after every `Ended` and `Released`.

**Who calls it** - `gauss::charge_trigger(f, entity, tank, owner, fire,
pressed, report)`, the room side, one function for a seat and an enemy:

- `drive_player`, for a seat whose trigger is `Charge` or whose tank holds
  a charge (which lapses there if the trigger is no longer its weapon's),
  with `pressed` its `fire_pressed`, `open` = `fire_cooldown <= 0`,
  `report` this tick's hold report for the seat
  (`Game::seat_hold_report`, below).
- `enemy_trigger`, for an enemy whose trigger is `Charge` or which holds a
  charge, every tick it thinks or coasts (where a tell starts and
  `dispatch_fire` is called for every other weapon), with `pressed` from
  `Tank::trigger_held` (the trigger it handed the simulation last tick),
  no report. A tank with a charge coasting far from every seat
  (`coast_enemy`) coasts with its trigger *down*, so a far tank's charge
  never goes off on a coast: it vents at worst. A disabled tank's coast
  has nothing to release - the EMP cleared it.
- `Game::predict_seat_with` for the sandbox's seat (§8), with the
  predictor's local gate as `open`.

`charge_trigger` acts on the edge: `Started` logs `ChargeStarted`;
`Released(stage)` calls `fire_charge` (a slug spent, `PendingRail` queued,
`fire_cooldown = gauss_reload_seconds`, `kick_rail`, `Fired`), and
`resolve_rails` traces it and recoils the shooter the same update;
`Ended(Vented)` sets `fire_cooldown` to the rule's `vent_cooldown`; every
`Ended` logs `ChargeEnded`, and a fizzle or a vent stages its picture
(`ChargeEndFx`). The other lapses - a wreck, `disable`, `end_round` -
clear the charge where they happen, with no event and nothing drawn.

**The crawl**: `drive_tank_with` multiplies the commanded top speed by
`tank.charge_pace()` where it applies `intent.slow` (the target only, not
the acceleration), and `tank.throttle` records `intent.speed_scale() *
charge_pace()`. The predictor's sandbox runs the same `drive_tank`, so a
client's own hull crawls on its own prediction.

**The hold report** - the room counts the client's ticks, not its own.
Online, the room's count of a held trigger and the client's can differ by a
tick or two: the room's play point steps twice or holds once now and then
(`Mailbox`'s playout controller), a stalled read repeats the trigger, a
burst merges several intents into one tick. At the full threshold a tick is
the difference between a slug and a fizzle, and the client draws its slug
on its own count (§8). So the room takes the client's count, which it can
read off the intents' own ticks, the way it believes an owned pose for the
driving its intents cover (`Mailbox::pose_reach_ticks`):

- `Mailbox::hold_ticks() -> Option<u32>`: the client's own ticks of
  trigger held, counted along every intent taken in tick order - dropped
  ones folded in - from the tick a hold started on to the newest it was
  still down on (`Inner::track_hold`). A read that delivers the trigger
  down reports the hold running (for a tap merged into one read, the one
  that just ended); one that delivers it up reports the hold that last
  ended; a starved read reports nothing, and the round counts its own
  tick.
- `net::authority::take_hold(game, seat, ticks)` puts it on the seat before
  the tick (`Game::set_seat_hold`, valid for that update alone, as
  `seat_owned` is) - in the room server's tick beside `take_pose`, and in
  the rig's.
- `step_charge` takes the report within `tank::CHARGE_HOLD_SPARE_TICKS`
  (6, 100 ms) of the ticks the round has stepped the charge itself
  (`Charge::stepped`), either way: a client stamping its intents further
  apart than it held gains six ticks a press at most - full six ticks
  early, or held six ticks past the vent. The spare is measured against
  the round's own count, never against the last report, which would let
  it compound a tick at a time into a full charge in a fraction of a
  second or one that never vents
  (`the_hold_reports_spare_is_a_presss_never_a_ticks`).

A local round has no report; its count is its own, exact by construction.
With the report the room's release decision is the client's to the tick,
in owned mode and in stage 2's ordered mailbox alike.

**How it is drawn**: per weapon, from the state - `render/game.rs` asks the
weapon's composer for every tank with a charge (`gauss::compose_charge`),
in the glowing pass, so it reads at night; the module shows its charge
cells (`module_cols`); the HUD's weapon slot shows the gauge
(`WeaponSlot::charge`, §5).

**How it travels**: `TankState::charge: u16` - 0 for none, else the whole
ticks held, at least 1 (exact, so a stage-2 replay starts from the room's
count); the
weapon is `TankState::weapon`. A replica writes `Tank::charge` from it
(`apply::write_tank`) and counts `held` up between snapshots
(`tick_presentation`, never past the vent: a replica's charge ends only when
the room's does). The shooter's own client draws its predicted charge
instead (§8). `ChargeStarted` is on `NOT_SENT`; `ChargeEnded` is sent, for
the fizzle and the vent's picture.

**How the AI drives it**: `SpecialUse::Charge`/`Release` and the first arm
of `special_rule` (`windup_rule`, §3.2); `Tank::trigger_held` gives
`enemy_trigger` the press edge; the coast holds a charging trigger down.

**Off screen**: `indicators::TankView::windup` (weapon and progress from
`Tank::windup`), so a charging enemy off the screen gets the wind-up's
arrow (`ArrowKind::Windup { weapon, progress, lane }`, never merged, never
dropped past the cap) in its weapon's accent rimmed hostile red; for the
rail it also carries the lane warning (§5).

**For the rod (BB-41)**: it is `Trigger::Charge` with its own `ChargeRule`
(`full` = the reticle settled, `overcharge` none, `vent` its own), the same
`Charge`, `step_charge`, release edge, wire field, hold report, AI uses and
arrow; its reticle - moved by the stick while the trigger is held, with
`crawl` 0 so the hull stands - is its own state beside the charge.

### 3.4 The armory's rail

Into `maps/armory.toml` (docs/sonic-hammer.md §3.5, docs/emp-burst.md
§3.4); nothing placed by the hammer or the EMP moves:

| Mark | Cell | Why |
|---|---|---|
| `R` rail crate | 7,6 | The reserved column's second cell, two cells north-east of the start |
| `R` rail crate | 30,12 | On the enemy side, so an enemy on shells collects it and its rule shows with no tuning |
| `b` brick | 27,8 and 27,9 | A screen of cover across the start's rows, between the seats and the band: a slug goes through it, a shell does not |
| `d` wood | 27,10 | Splinters, or catches fire if the plank is flammable |
| `g` glass | 27,11 | The screen's pane |
| `I` iron | 33,8 34,8 33,9 34,9 | Stops a slug down rows 8 and 9 - the cover that counts; overcharged, it is cut |

```
     0         1         2         3
     012345678901234567890123456789012345
 0   ................................L...
 1   .+..........ggggg...............L...
 2   ............g...g...............L...
 3   ............g.s.g...II....b.....L.E.
 4   ...P...e...*g...g...II....b.....L...
 5   ............ggggg.........b.zzz.L...
 6   .......R.............o..........L...
 7   .............................e..L...
 8   ....S..H.........*...f%%%..b.....II.
 9   ...........................b.....II.
10   .......r.............o...*.d..H.....
11   ...........wwwwwT..........g........
12   ..F....r...wwwww..............R.....
13   ..........Twwwww...WWWWWWWW.b.......
14   .......r...wwwww...WWWWWWWW.b.....G.
15   ...........wwwww...WWWWWWWW.........
16   .a.................WWWWWWWW....m....
17   ...................WWWWWWWW.........
```

(`e` the EMP's crates, `m` its missiles crate, `*` the lamp posts, `r` the
column still reserved for weapons 4-6.) The lanes it shows from the west:
row 8 fells the lamp post, sets the fuel drum off (whose blast lights the
oil trail), goes through the brick and stops at the iron; row 9 is the clean
one, brick then iron; row 10 sets off the oil drum, fells the second lamp
post and goes through the wood to the edge; row 11 lays the grass flat,
fells or lights the tree and breaks the pane. The screen leaves rows 7 and
12 open, so the field stays one piece. The linter finds no new error or
warning: two more `narrow-corridor` infos, at 26,6 and 27,6 - the pass
between the brick stub (26,3..5, where the EMP's review moved it off the
HUD's rows) and the screen. The armory's CPU thumbnail pin is re-baselined
for the crates and the walls.

## 4. AI

An enemy carrying the rail uses it by `gauss_rule`, never through the
generic tiers (`generic_fire(GaussRail)` is false): attack still lines it
up and settles but never fires it, `Brain::wants_breach` never latches with
it (a rail tank does not stand charging slugs into brick to get unstuck;
its stuck escape does that), and it fires no shells while it carries the
rail - as with the hammer, until its four slugs are spent.

### What it is handed

`Game::gauss_senses` runs once per frame in `enemy_phase`, only when some
live enemy carries the rail, over every seat (`GaussSeat`: position, on
the field, concealed, `Game::sight_on` at it). It gives each rail tank a
`GaussSense`, one trace per facing through the frame's `Terrain` by the
room's own `pierce_rewound`, each from the gun-line muzzle that facing
would have, `laser_reach` long, with this tank's own stop rule (iron stops
it - an enemy never overcharges):

```rust
pub struct GaussSense {
    /// One per facing, `Dir::index` order: what a slug from where this
    /// tank stands, that way, would go through.
    pub lanes: [GaussLane; 4],
}

pub struct GaussLane {
    /// The nearest seat along the lane that counts (below), and how far
    /// along it is: the seat the slug is used on (`Ai::shot_at_seat`).
    pub at_seat: Option<(u8, f32)>,
    /// Seats that count.
    pub seats: u8,
    /// Standing player towers it would go through.
    pub towers: u8,
    /// The players' frog it would go through, and how far along - read
    /// only for a hunter, whose quarry it is.
    pub quarry: Option<f32>,
    /// A live fellow enemy, a standing enemy tower or the enemies' own
    /// frog it would go through.
    pub friend: bool,
}
```

**A seat counts** in a lane when the slug would go through it (its box is
on the pierce list, it is no wreck and on the field), this tank stands in
that seat's sight box (`in_sight_box_of`, against the seat's real position
- §3.1 of the hammer's doc), the seat is within this tank's sight
(`Game::sight_on`: the attack tier's range under the sky; a lamp or a hot
bank lights it up), and it is not hidden from this tank (concealed and not
hit-alerted - the attack tier's rule). No line of sight is asked for: brick,
wood and glass between do not hide it.

That is the issue's "a seat it knows is behind them (alerted, last-known
position in the lane)" made exact: the alert an enemy keeps is a distance
rule with no line of sight (the arena's shared alert, a field map's chain),
refreshed from the seat's real position every tick an enemy is in sight of
it, so a seat in sight range and not in grass *is* known where it stands -
and the lane is tested against where it stands. A seat in tall grass is not
known (its alert has gone stale), and a seat beyond the sky's range at night
is not either.

**The lane's score** (`GaussLane::score`): `2 * seats + 2 * quarry +
towers`. A lane counts (`counts`) when it scores at least 2, holds a seat
or the quarry to fire at (`target_along`: how far along that seat, else
the quarry, stands) and no `friend` before it - a friend beyond the target
is the slug's to go on through (decision 24).

### The rule (`gauss_rule`), in priority order

1. **A charge in progress** (`special_rule`'s first arm, §3.2) is answered
   here and nowhere else, never `None`:
   1. charging, not full yet: `Charge { face: the facing }`;
   2. full, not overcharged: the lane along its facing counts and its
      `at_seat` is still a seat that counts (or, for a hunter whose lane
      holds its quarry, the quarry) - **`Release { face, at_seat, why }`**
      (`why`: `"rail"`, `"rail-two"` with two seats, `"rail-tower"` with a
      tower, `"rail-quarry"`); otherwise `Charge { face }` (`"rail-wait"`) -
      it waits, holding, for a seat to step back into the lane;
   3. overcharged: `Charge { face }` (`"rail-vent"`), to the vent. It never
      releases overcharged, so it never cuts iron and iron stays the cover
      that holds against enemies. A seat that stepped out of the lane and
      stays out has wasted the charge.
2. **A training dummy** (`Ai::frog_only`): `None`.
3. **The rail is cooling** (`fire_cooldown > 0`: its reload or a vent):
   `None` - the tree goes on, the trigger released.
4. **Charge the best lane**: of the four lanes that count whose seat is
   settled - inside its sight box and this tank's sight by
   `gauss_ai_box_margin_px` (32) besides (`GaussLane::settled`, decision
   27) - the highest
   score - two seats beat a seat and a player tower beat a seat alone - ties
   to the facing it has, then `Dir::ALL` order: `Charge { face }`. With
   `Ai::fire_timer` running, `act_special` holds facing it instead.
5. Otherwise `None`: the tree goes on (attack lines up and settles but
   never fires the rail; chase, patrol, the seeks as ever).

**It stands its ground.** A charging enemy commands no movement: the
issue's "a charging enemy also crawls" is the crawl a seat gets, and an
enemy that crept along its lane while it charged and was thrown back a
cell by each release kept moving the cell it stood in - which, with the
probe's armed sweeps, swung a tank chasing a seat behind it to and fro for
the whole round (§12, decision 23). The charge's glow is the tell.

**The tell**: the charge itself - the ring, the motes and the glow, its
1.5 s before the slug at the earliest - and off the screen the tell's arrow
with the lane warning (below). The decision to start and the release are
both moments the sight box is checked; `shot_at_seat` is recorded at the
release, the frame the slug goes, which is the frame the probe's
`offbox-fire` reads.

**What else the slug goes through.** A counted seat is the one the slug is
used on. The slug goes on through whatever else is in the lane, as every
shot does - a second seat further down the lane whose sight box this tank
stands outside among it (§12, decision 2). That seat is warned: the tell's
arrow and the lane ring point at the tank while it charges.

**Pacing**: `gauss_ai_fire_interval` (2.0) between an enemy's decisions to
charge, on top of the reload.

**The hold-still clocks**: a charging tank commands no movement, so the
stuck clock resets, as for any deliberate hold, and the probe reads it as
one (`TankSnapshot::charging` in `HOLDS`).

### Reacting to a rail: the lanes

"The AI already prices the cells down a player's barrel": it does, and a
charging seat counts - `route_lane_cost` (3) on `route_lane_cells` (8)
cells ahead of every live seat - but that lane stops at the first blocked
cell, is a nudge, and never moves a tank standing in it. A charging rail's
lane goes through cover to the far side of the field. Two things answer
it, both built only while some tank on the field charges a rail
(`Game::rail_lanes`: each charging tank's lane - from its gun-line muzzle
along its facing to its slug's stop with its current stage's rule, half
width `battlefield::max_tank_clearance_half_extent() + gauss_half_width`,
so any hull centred inside it could be pierced):

1. **A danger** (`rail_dangers`, beside the EMP's `emp_dangers`): a
   `DangerShape::Lane` per charging tank, owned by it, seats in index order
   then enemies by slot. Every enemy inside one it does not own backs out
   on foot to the nearest of its exits it can reach (the `dodge` tier, its
   latch, its held exit point, `enemy_danger_clear_px`), and its chase,
   attack reposition, alert and seeks aim outside (`out_of_danger`). So
   enemies step out of a charging seat's lane, and a charging enemy's
   allies step out of its lane - the slug would go through them too.
2. **The edge hold** (§3.2): a tank whose next step enters a lane waits at
   its edge until the charge ends.

No route surcharge: a lane runs to a permanent tile or the field's edge
and lives a second or two, so pricing its cells only flipped the flow field
between a detour and the straight route as charges came and went (§12,
decision 22).

A charging seat hidden in tall grass still makes its lane: the glow is
drawn over the tufts and lights its lane (§12, decision 7); the seat itself
stays hidden from every enemy's targeting. A seat's frog and its towers
read no dangers - the frog's evasive hop is its own, towers stand - and an
enemy's rail rule counts them as reasons to fire, not dangers.

The special tier sits above `dodge`: an enemy charging its own rail (or in
a tell) in a seat's lane keeps charging - whoever lets go first wins.
A disabled enemy does not dodge (it does not think). A far tank that
coasts or sleeps does not dodge either.

### With the commander (`c2_enabled`)

`Busy::Charging` (`UnitView::busy`, from `Tank::charge`): a charging unit
is never given an order - a `Nudge` would turn its hull off its lane - and
keeps right of way in `deconflict`, as the EMP's disabled unit does. C2
off is untouched; with C2 on and no rail charging, nothing is.

### Off the field and asleep

A field map's far tank coasting on its last intent keeps a charging trigger
down (§3.3) - it never fires on a coast - and has no sense; a sleeping tank
starts nothing.

## 5. Drawing

All of it in the effects language (docs/effects.md): whole 2 px blocks,
ramp steps, Bayer fades; composed at draw time as pure functions of the
slug, the charge or the tank and their age, hashed from slots and
positions, never rolled.

- **A new ramp, `pyro::RAIL`**: `#04A0B4` (`BLUE_MD`), `#1EB3AE`
  (`BLUE_LT`), `#27D8C5` (`BLUE_BRIGHT`), `#93ECE2` (`BLUE_PALE`), `#FFFFFF`
  - the palette's own blues, the Armory scene's colours; effects.md's ramp
  table gains its row. Paler at its bright end and greener at its dark end
  than the EMP's ramp, so a slug's trail and an EMP's ring do not read as
  one weapon. Its glows - added light - are `pyro::RAIL_LIGHT`, a cold blue
  `#4C7EFF` off the palette: the ramp's teal added over the grass read as
  the grass's own green in the screenshots (an object is never the
  ground's colour, docs/PALETTE.md), so the light is bluer than the blocks.
- **The charge** (`gauss::compose_charge(centre, muzzle, charge, seed,
  time)`), glowing pass:
  - a ring round the hull, `pyro::Shape::Arc` a whole turn, one block wide,
    in `BLUE_PALE`, its radius falling from `CHARGE_RING_PX` (34) to 22 px
    as the charge fills and its `cover` rising with the progress (dissolving
    in through the Bayer pattern);
  - `CHARGE_MOTES` (10) motes drawn in to the muzzle
    (`tank_art::RAIL_MUZZLE`): each a block on a hashed angle, from 40 px
    out to the muzzle over half a second and round again, phased per mote,
    `BLUE_PALE` with a `WHITE` block for its last 8 px;
  - a glow on the muzzle in `RAIL_LIGHT`, `8 + 12 * progress` px, its
    strength the progress;
  - at full the ring holds at 22 px and beats at 6 Hz (`cover` 1 and 0.6),
    the glow's core `WHITE`;
  - overcharged the ring is `WHITE` and jumps a block either way on a hash
    every 1/20 s, and two zigzags crackle off the bore every 1/15 s (two
    runs, 4-8 px in all, `BLUE_PALE` with a `WHITE` head);
  - over the last `VENT_WARN_SECONDS` (0.6) before the vent the ring
    flickers (`cover` stepping 1 and 0.5 at 10 Hz) and wisps of white steam
    begin off the module (`pyro::Puff` in `SMOKE`'s two lightest steps).
  The module shows its charge cells (below).
- **The slug** (`gauss::compose_slug(slug)`), per leg from
  `Game::rail_slugs`:
  - **the white frame**: for `gauss_flash_seconds` (0.05, three frames), a
    band three blocks wide of `WHITE` from the leg's start to its end
    (`pyro::block_line`), with `BLUE_PALE` blocks one more block out either
    side - glowing pass, the one bright instant (rule 7);
  - **the ionised trail**: from then to `gauss_trail_seconds` (1.6), one
    block wide along the leg, `BLUE_PALE` while its life `a` (1 falling to
    0) is over 0.6, then `BLUE_LT`, wobbling across the line by
    `round_to_block(sin(along * 0.3 + age * 6) * 2 * (1 - a))` and
    dissolving through the Bayer pattern to half, then in eighths (rule 3);
    beside it a sparse row of `#F0F0F0` ions two blocks off the line, every
    third block, kept where the Bayer value is under `0.8 a`. Glowing pass: the
    trail is ionised air, its own light, and it throws none on the ground;
  - **the pierce bursts**, composed with the slug from its age over
    `PIERCE_SECONDS` (0.5), so a replica and a client draw them alike: a
    spray of sparks out of each pierce's far side along the slug's line
    (glowing pass) - six `WHITE`-and-`BLUE_PALE` off a hull with a white
    ring of blocks opening round the entry, six in `SHIELD`'s steps with the
    ring off a shield, four `BLUE_PALE` off a frog, six white-and-grey off
    iron an overcharge cuts, three off any other tile; and in the lit pass
    (`compose_slug_lit`) a tile's chips - its material's dust
    (`pyro::dust_of`), glass's pale shards - thrown 8-24 px out of its far
    side and falling as they go, and two dark flecks of armour off a hull.
    The tile's own collapse cloud comes from its death (`ObstacleDestroyed`,
    `fx.rs`);
  - **the stop**: a spark star of `WHITE` and `BLUE_PALE` thrown back along
    the line and a `RAIL_LIGHT` glow fading over 0.3 s; nothing at a leg's
    end in a portal (the portal's own flare, `ShotTeleported`, marks it);
  - **the ripple**: `Shockwave::scaled(muzzle, gauss_shock)` (0.4 of a tank
    dying) through `shockwave.rs` - the bend and the shake, none under
    reduced motion;
  - **the grass** along every leg lies flat for a moment
    (`grass::flatten_along`, the tufts within 12 px of the line given a
    hull's `crush`, recovering over `grass_crush_recover_seconds`) -
    cosmetic, concealment untouched;
  - **the fish** dart away from every leg (`fish::scares`, a scare every
    cell along it, as from a laser beam).
- **The sparks** (`fx.rs`, off `Event::RailSlug`): a spit of `BLUE_PALE`
  and white sparks out of the bore down the line on a first leg, and a
  cone splashing back off the stop (and a splash where it is wet).
  Particles, so `rand::rng()`, never the round's. A rail `Hit` puts on
  only its flash (`fx::Flash`), never a shot's impact burst - the slug's
  own picture carries them.
- **A charge's end** (`gauss::compose_end`, from `Game::charge_ends`):
  fizzled - four `BLUE_LT` blocks dropping off the bore over 0.15 s;
  vented - five shaded puffs of white steam (`SMOKE`'s lightest steps)
  rising off the module and leaning with the wind (`pyro::smoke_lean`) for
  0.8 s, and three `WHITE` sparks; lapsed - nothing (the wreck or the EMP
  has its own show).
- **The recoil**: dust off the skidding hull (the hammer's `fx.rs`); the
  overcharged spin is the hull's and the turret's eased swing.
- **The light** (`weather::lights_in`), in the blue of the glows: a
  charging tank's bore throws an unshadowed point light, `24 + 40 *
  progress` px, at `gauss_charge_light * progress`, so an enemy's charge
  reads at night - the tell is fair in the dark; a slug's white frame
  throws, fading over three times `gauss_flash_seconds`, an unshadowed
  light every `FRAME_LIGHT_SPACING_PX` (96) along each leg, 72 px, at
  `gauss_frame_light`; the stop a shadowed one fading over 0.3 s. The trail
  throws none.
- **The module** (`tankdesign`, `lines/vanguard.py`, `module_fn('gauss')`):
  a coil gun on the left cheek - the laser's hardpoint, shared, since a tank
  carries one special at a time (`hp.get('gauss', hp['laser'])`): a
  gunmetal capacitor block (2 x 4 design px) at the root with its four
  charge cells in a column on its outer face, two steel rails running five
  pixels forward of it with a dark bore between them, a steel collar at
  their tips; the muzzle at the bore's tip (`meta['muzzle']`, written into
  `tank_art::RAIL_MUZZLE`). Seven cells, `TANK_MODULE_GAUSS_COL` = 33..39
  (`tank_modules.png` grows from the EMP's 33 to 40 columns, 1600 x 480):
  0 idle, the cells dark (`DIM_RAIL`, `BLUE_MD`); 1-4 the charge, that many
  cells lit in the light layer (`'ion'`: `BLUE_PALE` over `BLUE_BRIGHT`); 5
  full, all four cells `'white'` and the rails' inner edges `'ion'`; 6 the
  shot, the rails `'white'` and the cells dark. `module_cols` (now nine
  entries): 6 while `Tank::rail_flash` (`gauss_module_flash_seconds`, set
  by `kick_rail` on every slug - the room's, a replica's `Fired`, a client's
  release); 5 at full; 5 and 6 alternating at 10 Hz overcharged; `1 +
  floor(progress * 4)` (at most 4) while charging; else 0. A disabled tank
  carrying it shows cell 0 (the EMP's rule). `render.SHOWN_TOGETHER` leaves
  it out with the laser it shares a cheek with. Read on the scout, the
  warden and the leviathan: a slim pair of rails with a teal light bar,
  apart from the laser's lens on every one.
- **The crate**: row 15 of `gen_crates.py`'s sheets (`crates_sheet.png`
  280 x 640, `pickup_glyphs.png` 24 x 384). Its symbol, 10 x 10 design
  px: two rails, the slug's trail between them and its white-hot head (`o`,
  the ink's light) leaving their mouth:

  ```
  '..........',
  'XXXXXXX...',
  'XXXXXXX...',
  '..........',
  'X.X.XXX.oo',
  'X.X.XXX.oo',
  '..........',
  'XXXXXXX...',
  'XXXXXXX...',
  '..........',
  ```

  Ink (`punypalette.PICKUP_INK['gauss_rail']`, admitted on the crate sheets
  alone like the others): hot magenta - shade `#B01E92`, base `#FF3DD8`,
  light `#FFB0F0`. Jade (`#36E07A`, in the hue wheel's widest gap) was
  rendered beside all fifteen shipping crates and the grass and read as
  both: a green object on green ground, which the art direction keeps off.
  Magenta was the most distinct of the candidates rendered (rose sat next
  to the health crate's red, an ice blue next to the minigun's and the
  hammer's) and nothing on the field is that colour.
- **The HUD**: `hud::WeaponSlot::of` gives the slugs in `HUD_GAUSS_COLOR`
  (`#FF3DD8`, the ink's base) and the glyph, and while a charge runs
  `WeaponSlot::charge: Option<ChargeGauge>` (`progress`, `stage`, `warn`,
  `blink`), which `render::hud::draw_weapon_readout` draws **in the
  count's place** - `V_WEAPON_COUNT`, `V_COUNT_W` (30) wide, `GAUGE_H` tall,
  centred on the count's line, through `draw_gauge`: charging, filled to
  the progress in `HUD_GAUSS_COLOR`; full, filled white; overcharged,
  white and `HUD_GAUSS_COLOR` in turn at `CHARGE_BLINK_HZ` (8); its outline
  the HUD's red within `VENT_WARN_SECONDS` of the vent, `DIM` otherwise.
  The count comes back the frame the charge ends; an offline special
  (`WPN OFFLINE`) shows no gauge. The ring's ammo pips are the slugs left
  against `full_load` (4). A replica's seat and a client's own read the
  same slot (the client's from its predicted charge, §8).
- **The off-screen warning**: the wind-up's arrow (`ArrowKind::Windup`) in
  `HUD_GAUSS_COLOR` rimmed hostile red, blinking quicker as the charge
  fills, for a charging enemy off the screen - a wind-up reveals its tank
  as firing does. When this seat is in its lane (`TankView::lane`, from
  `PresentWorld::rail_trace` from its gun-line muzzle along its facing,
  `laser_reach` long, reaching this seat's hull before a stopper - through
  cover, whatever the range) the arrow carries the lane warning's pulsing
  ring, dark red to half charged and bright after. A seat hit by a slug
  points its hit arc back down the slug's line (`hit_from`: a `RailSlug`
  whose pierce list holds the hit point).

## 6. Tuning

New group `gauss` (every row live unless marked), plus one row in the
enemies' group:

| Row | Default | Range | Doc |
|---|---|---|---|
| `gauss_slugs_per_pickup: i32` | 4 | 1..=20 | Slugs one gauss rail crate loads. One per full release. |
| `gauss_charge_seconds` | 1.5 | 0.1..=10 | Seconds the trigger is held before the rail is full and a release fires; released sooner, the charge fizzles. |
| `gauss_overcharge_seconds` | 1.0 | 0..=10 | Seconds past full before a release fires overcharged (cuts iron, throws the shooter twice as far and spins it round); at or past `gauss_hold_seconds`, never. |
| `gauss_hold_seconds` | 2.0 | 0..=20 | Seconds a full charge may be held before it vents, firing nothing. |
| `gauss_reload_seconds` | 0.6 | 0..=10 | After a slug, seconds before a charge may start again. |
| `gauss_vent_cooldown_seconds` | 1.2 | 0..=10 | After a vent, seconds before a charge may start again. |
| `gauss_crawl_pace` | 0.2 | 0..=1 | The share of its top speed a charging hull keeps. |
| `gauss_damage` | 120 | 0..=500 | A seat's slug's damage to the first tank or tower it goes through; no roll. |
| `gauss_enemy_damage` | 60 | 0..=500 | An enemy's slug's, the same way. |
| `gauss_pierce_keep` | 0.8 | 0..=1 | The share of its damage a slug keeps past each tank, frog or tower it goes through. |
| `gauss_tile_keep` | 0.95 | 0..=1 | The share it keeps past each wall, prop or tree. |
| `gauss_frog_damage` | 20 | 0..=200 | A frog's hit from a slug, times what the slug has kept. |
| `gauss_half_width` | 4 | 0.5..=16 | The slug's half width (px): what it grows every box it is traced against by. |
| `gauss_recoil_cells` | 1.0 | 0..=6 | How far the recoil slides a chassis of mass factor 1 on dry ground, in cells. |
| `gauss_recoil_mass_exponent` | 0.5 | 0..=4 | How much a heavy chassis resists the recoil: its speed over the mass factor to this power. |
| `gauss_overcharge_recoil_factor` | 2.0 | 1..=6 | An overcharged slug's slide against a full one's. |
| `gauss_flash_seconds` | 0.05 | 0.01..=0.5 | The white frame along the line. |
| `gauss_trail_seconds` | 1.6 | 0.1..=5 | How long the ionised trail takes to thin out. |
| `gauss_module_flash_seconds` | 0.25 | 0..=2 | The module's shot cell. |
| `gauss_shock` | 0.4 | 0..=2 | The screen ripple and shake against a tank dying's. |
| `gauss_charge_light` | 0.6 | 0..=2 | The light a full charge throws at night, against a headlight's. |
| `gauss_frame_light` | 1.0 | 0..=2 | The light the white frame throws along the line at night. |
| `gauss_ai_fire_interval` | 2.0 | 0.1..=20 | Seconds between an enemy's decisions to charge. |
| `gauss_ai_box_margin_px` | 32 | 0..=128 | How far inside a seat's sight box (and its own sight) an enemy must stand to start a charge at it; the release needs only the box. |
| `enemy_special_weapon_gauss_share` (`enemies`, `@ Restart`) | 0 | 0..=1 | The share of special-carrying enemies that spawn with the gauss rail instead, decided by a hash of the spawn point and the slot - never the round's RNG - so at 0 nothing changes. |

Constants (geometry and policy, not feel): `CHARGE_HOLD_SPARE_TICKS` (6)
in `tank.rs` beside the pattern it bounds; in `gauss.rs` `CHARGE_RING_PX`
(34, falling to 22), `CHARGE_MOTES` (10), `VENT_WARN_SECONDS` (0.6),
`FRAME_LIGHT_SPACING_PX` (96), `TRAIL_WOBBLE_PX` (2), `GRASS_REACH_PX`
(12), `PIERCE_SECONDS` (0.5), `STOP_SECONDS` (0.3), `END_SECONDS` (0.15,
0.8); `hud::CHARGE_BLINK_HZ` (8).

## 7. Text

Data names by family (`named("tool", ..)`), so no `text::keys` constant.

| Key | en | sl |
|---|---|---|
| `tool-gauss_rail` | gauss rail | gaussov top |
| `tool-short-gauss_rail` | rail | tir |

`gaussov top` ("Gauss cannon") because the literal `gaussova tirnica`
runs past the tool list's 144 pt at the font's own widths; both pass
`every_language_fits_every_budget`. The HUD shows the glyph, a count and
the gauge; the weapon has no other words.

## 8. Wire

Protocol 17 (from the EMP's 16), once in the PR.

- `WeaponKind::GaussRail`, appended to `ALL`; `drawn_on_press` true (its
  show is drawn on the *release*, which is its press show).
- `TankState::charge: u16` - 0 none, else the whole ticks held, at least 1.
  `TankState::weapon` is the special carried (the EMP's rule), `ammo` its
  slugs.
- `WireEvent::RailSlug { slot: u16, seat: u8, leg: u8, x0: i16, y0: i16,
  x1: i16, y1: i16, portal: bool, overcharged: bool, pierced:
  Vec<RailPierce> }` and `RailPierce { x: i16, y: i16, what: Pierced }`
  (`Pierced`: `Tank`, `Shield`, `Frog`, `Tile(Material)`) - every leg and
  every thing it went through, quarter pixels; `seat` is `NO_SEAT` for an
  enemy's. Mirrors `Event::RailSlug`. A slug through a whole wall run is a
  few dozen bytes, and slugs are rare.
- `WireEvent::ChargeEnded { slot: u16, weapon: WeaponKind, end:
  ChargeEnd }`; mirrors `Event::ChargeEnded`. `Event::ChargeStarted` is on
  `NOT_SENT` (the state is what draws).
- `HitCause::Rail`, appended (`WireEvent::Hit::cause`): a replica puts on
  no impact flash for it, as for a sonic hit.
- `WireEvent::press_show`: a `RailSlug` with `leg: 0` and a seat is
  `(seat, WeaponKind::GaussRail)`. Legs past a portal stay the room's.
- **What a replica draws**: on a `RailSlug`, `rail_show` - the leg on
  `Game::rail_slugs` with its pierce points, the ripple on leg 0, the grass
  laid flat - so the bursts and the trail play from the event, and `fx`
  sparks off the fresh slug; the hits flash from their `Hit`s and the tiles
  come down from their `ObstacleDestroyed`. On a `ChargeEnded`,
  `charge_end_show`. From state: `write_tank` sets `Tank::charge` (and the
  slugs), `tick_presentation` counts it up between snapshots; the module's
  shot cell from `Fired`'s `kick_turret`. Recoil and the spin arrive as the
  hull's pose and facing.
- **What is drawn at once** (decision 3 of BB-36, hammer §3.3), on the
  shooter's client:
  - *the charge, from the press*: every tick the sandbox's seat runs the
    charge machine in `Game::predict_seat_with` (§3.3), handed the press
    edge and the predictor's local gate as `open`
    (`Predictor::charge_gate_open`: its cooldown out, no local offline, a
    charge weapon with slugs left less the owed ones), and `OnlineRound`
    writes the sandbox's `Tank::charge` into the shown seat where it writes
    the drawn pose, every frame (`Game::set_seat_charge`) - so the glow, the
    module's cells and the HUD gauge start on the press frame and end on
    the release frame. An owned hull crawls on its own prediction.
  - *the slug, on the release*: `Predictor::charge_edge` takes the edge
    `predict_seat_with` reported (`SeatCharge`, with the gun-line muzzle,
    the bore and the facing from before the tick's step). On
    `Released(stage)` it sets the local gate to `gauss_reload_seconds`,
    owes the slug, and, while presses are drawn, queues
    `PressShow::Rail(RailPress { start, muzzle, dir, overcharged })` and the
    drawn press `(GaussRail, input tick of the release)`. `fly_own_shots`
    traces it through the drawn world - into the first portal on its way if
    it meets one, else `PresentWorld::rail_trace` to the stop (the first
    stopper, iron only while not overcharged, or the field's edge at
    `laser_reach`), with what it goes through on the way - and puts it on
    through `Game::draw_rail_press`: the same `rail_show` a replica puts on
    for the room's `RailSlug`, plus the module's shot cell (`kick_rail`).
    The release reaches both on the packet after the fire hold's
    (`client::FIRE_HOLD_TICKS`), a tick after the key comes up.
  - *the recoil and the spin*: the charge machine's own, so
    `predict_seat_with` applies the knock (and an overcharged release's
    spin) to the sandbox's seat on the release tick before the solver steps,
    as `resolve_rails` does before `step_world` - live in owned mode, and
    again in a stage-2 replay of a release after the acked tick (the
    history keeps each tick's press edge and gate), as the room did. An
    owned reconciliation keeps the sandbox's own charge over the room's.
  - *a fizzle or a vent*: `PressShow::ChargeEnd(ChargeEndPress { at, end })`
    → `charge_end_show`.
- **What is claimed**: the room's leg-0 `RailSlug` for this seat, by the
  input tick its `Fired` names (`presses_drawn`, one pending claim per
  kind): the last drawn rail release at or before that tick is the room's,
  earlier ones it refused. The room stamps that `Fired` with the read's
  `press_tick` - on a release read, the ack, which is the release's intent
  or a later one merged with it - so "at or before" finds it. A claimed
  event is not drawn and not handed on to `game.events`. Under
  `Show::OwnShotsDrawn` this seat's `ChargeEnded` events are left out too
  (the client drew its own). A drawn release nobody claims within the
  refusal wait is dropped; a room `Fired` the client never drew seeds the
  local gate (`seed_gate`) and its `RailSlug` is drawn.
- **Why the release agrees**: the room's charge counts the client's ticks
  (the hold report, §3.3), and the client's sandbox counts the same ticks -
  owned, one sandbox tick per intent; in stage 2, a replay from the room's
  exact count at the acked tick. So the room fires on exactly the release
  the client drew a slug for, and fizzles exactly where it drew a fizzle.
- **What stays the room's**: damage and kills (`Hit`, `Wreck`), tile deaths
  (`ObstacleDestroyed`), drums (`Blast`), shields (`ShieldBroken`), the
  legs past a portal, the recoil of a hull the client does not own. The
  room's own copy of an owned hull is knocked too (overwritten by the next
  pose) and allowed the knock's speed (`Shoves::allow_knock`, the
  validator's `SeatKnock`).
- **Incoming slugs**: an enemy's slug is drawn where the room fired it,
  when the interpolator hands its tick over, against the replica's world.
  A seat that stepped out of the lane within a round trip of the release is
  hit as the room had it; the charge's second and a half is the warning.
- `delta.rs` needs nothing new (the field is inside `TankState`; events go
  whole); its random snapshots fill the field, and the full snapshot's bound
  grows by a byte a tank to 488 B (486 measured).

## 9. Determinism

- **The rail draws no RNG of its own.** No damage roll, no misfire, no
  spread, no pass-over, deflection or one-shot roll: the pierce walk is
  fixed damage with keep factors. Its order is fixed: legs in order, within
  a leg by entry distance, then rank, then owner slot or cell. What it sets
  off draws as it always does - a drum's blast its damage rolls where it
  goes off, a portal its exit (`draw_shot_exit`) only where a slug goes in.
- **The charge** is ticks; the hold report a clamp; the recoil a knock with
  no roll; the spin a facing.
- **The AI**: `gauss_rule` chooses by fixed priority and integer scores,
  ties to the facing it has, then `Dir::ALL` order, the nearest seat along
  a lane by distance then seat index; `rail_lanes` and `rail_dangers` are
  built in seat then slot order; the edge hold is a function of
  positions.
- **The swap** is the hammer's hash; the new entry runs only with its share
  above 0.
- **A round without the rail replays byte for byte**: no crate kind is
  rolled anywhere, every share defaults to 0, no enemy carries one, so no
  trigger is `Charge`, `step_charge` never runs, `rail_lanes` is empty (no
  danger, no edge hold from a lane), `charge_pace()` is
  exactly 1 and `throttle` what it was; `drive_player`'s trigger match
  answers every shipped weapon as before; a local round sets no hold report,
  and a room's report is read only by a charge. `determinism_tests`' pinned
  streams, the probe fixtures' ceilings and every thumbnail pin but the
  armory's stay as they are.

## 10. Tests

`simulation::gauss_tests` (headless rounds on the default 34 x 17 field,
the seat at cell 3,6 facing east, parked enemies placed by hand; every
number read off the defaults):

- The charge: `a_gauss_crate_arms_the_rail_and_replaces_the_special_carried`,
  `the_rail_charges_while_held_and_fires_on_release_at_full`,
  `a_release_one_tick_short_of_full_fizzles_and_at_full_fires`,
  `a_charge_held_past_its_hold_vents_and_the_rail_cools`,
  `a_charging_hull_crawls`.
- The slug: `the_slug_goes_through_brick_wood_glass_and_every_tank_in_order`,
  `iron_stops_the_slug`, `the_slug_crosses_the_field_to_its_edge`,
  `an_overcharged_slug_cuts_iron_and_spins_the_shooter`,
  `a_door_stops_even_an_overcharged_slug`,
  `the_slug_goes_through_a_portal_leg_by_leg`,
  `the_recoil_slides_about_a_cell_and_a_heavy_chassis_less`,
  `a_drum_in_the_lane_goes_off_and_the_slug_flies_on`,
  `a_tower_in_the_lane_takes_the_slugs_damage_and_the_slug_flies_on`,
  `sandbags_fences_trees_and_lamp_posts_in_the_lane_go_down`,
  `a_rainbow_shield_soaks_one_slug_and_the_slug_flies_on`,
  `a_frog_in_the_lane_takes_frog_damage_and_does_not_hop`,
  `crates_and_wrecks_are_left_alone`.
- Mid-charge: `a_wreck_mid_charge_fires_nothing`, `an_emp_mid_charge_lapses_it`,
  `another_weapons_crate_lapses_a_charge_and_a_rail_crate_keeps_it`,
  `a_teleport_mid_charge_keeps_it`, `a_sonic_shove_mid_charge_keeps_it`,
  `the_round_ending_mid_charge_clears_it_and_nothing_fires_on_the_end_screen`.
- Determinism and the room: `the_rail_draws_no_rng`,
  `the_hold_report_decides_the_release_within_its_spare`,
  `the_hold_reports_spare_is_a_presss_never_a_ticks`,
  `a_round_with_the_rail_replays_bit_for_bit`,
  `the_spawn_swap_hands_out_the_rail_by_its_share`.
- The AI: `the_rail_charges_at_a_seat_in_its_lane_inside_the_sight_box`,
  `the_rail_fires_through_brick_at_a_seat_it_knows_is_behind_it`,
  `the_rail_never_charges_through_iron`,
  `the_rail_does_not_fire_at_a_seat_hidden_in_grass`,
  `the_rail_waits_at_full_and_wastes_the_charge_when_the_seat_steps_aside`,
  `the_rail_fires_when_the_seat_steps_back_in_before_the_overcharge`,
  `the_rail_never_releases_overcharged`,
  `the_rail_never_fires_through_a_friend`,
  `a_friend_beyond_the_seat_does_not_hold_the_rail`,
  `the_rail_never_fires_through_its_own_tower`,
  `the_rail_prefers_a_lane_with_two_seats`,
  `a_training_dummy_never_charges`, `the_generic_tiers_never_fire_the_rail`,
  `a_charge_in_progress_is_never_released_by_another_tier`,
  `an_enemy_never_rails_a_seat_from_outside_its_sight_box`,
  `a_hunter_rails_its_quarry`,
  `enemies_step_out_of_a_charging_seats_lane_and_do_not_step_back`,
  `enemies_wait_at_the_edge_of_a_lane_rather_than_cross_it`,
  `a_charging_tank_is_busy_to_the_commander`,
  `a_charging_rails_lane_through_cover_warns_the_seat`.

Shared path and presentation:

- `gauss::tests`: the composers on the 2 px grid and in their ramps
  (`the_charge_is_in_its_ramp_and_pure`,
  `the_slug_is_in_its_ramp_and_gone_by_its_end`,
  `the_white_frame_lasts_its_frames`,
  `the_trail_dissolves_through_the_bayer_pattern`,
  `a_pierce_burst_throws_out_of_the_far_side`,
  `a_charge_end_is_gone_by_its_end`), `the_recoil_speed_slides_the_cells_asked`,
  `the_module_shows_the_charge`.
- `hud_tests`: `a_charging_rail_shows_its_gauge_in_the_counts_place`.
- `devserver`: `set_tank_arms_the_rail_and_charges_it`; the PICKUP
  category's count (16).
- `thumbnail`: the armory's pin. `maplint`: the armory as it lints.

Wire:

- `mailbox`: `the_hold_report_counts_the_clients_ticks_not_the_reads` - one
  read a tick, a merged read, the release's read, a tap, a starved read.
- `apply.rs`: `a_charge_and_a_slug_reach_the_replica` (the charge's ticks,
  the drawable state equal, counted up between snapshots; the slug's leg
  and pierces through the codec), `a_slug_this_client_drew_is_not_drawn_again`
  (the first leg and this seat's charge end left out under `OwnShotsDrawn`
  with the release's bit, a leg past a portal and an enemy's drawn),
  `a_sonic_or_rail_hit_flashes_no_impact_on_the_replica`.
- `predict.rs`: `a_rail_charges_on_the_press_and_is_drawn_on_the_release`,
  `a_short_hold_draws_a_fizzle_and_no_slug`,
  `an_owned_rail_crawls_and_recoils_as_the_room_does` (the sandbox against
  the authority's `update`, to a hundredth of a pixel every tick through a
  charge, a release and its recoil),
  `a_replay_carries_a_charge_from_the_rooms_count`.
- `rig.rs`: `a_seats_slug_reaches_the_replica_once` and
  `an_enemys_rail_reaches_the_replica` (`Lockstep`),
  `an_own_slug_is_drawn_on_the_release_and_never_twice` (a threaded rig
  over a 40 ms link, through a whole `OnlineRound`: the charge drawn from
  the press frame, the slug on the release, the room's never drawn as
  well).
- `server/tests/round.rs`:
  `a_charge_counts_the_clients_ticks_however_the_room_reads_them` - an
  owned burst of held ticks read in one room tick leaves the room's charge
  at the client's count, and the release at full fires.
- `delta.rs`: the random snapshots and the size bound.

## 11. Probe

- **Defaults first**: `just probe-fixtures` and `just probe-fields`
  unchanged, passing their recorded ceilings untouched.
- **With the crate**: the same two sweeps with `--crate gauss_rail`. AFK:
  enemies on shells collect it, line up on the seat (through cover too) and
  charge.
- **Armed enemies**: the same sweeps with `--tuning armed.json`,
  `{"enemy_special_weapon_chance": 1.0, "enemy_special_weapon_gauss_share":
  1.0}` - every enemy that would carry a special spawns with the rail.
- **The mix and the night**: `{"enemy_special_weapon_chance": 0.5,
  "enemy_special_weapon_gauss_share": 0.5}`, and `armed` at night
  (`"weather_override": 1`) and with the commander (`"c2_enabled": true`).
- The probe's tank line gains ` rail=N` (slugs, only while it carries
  some, so a round without the rail prints as before) and ` chg=true`; its
  fire tuple counts the slugs, so a slug is a trigger pull for
  `FIRED_RECENTLY_FRAMES`; a charging tank (`TankSnapshot::charging`) is a
  deliberate hold in `HOLDS`, beside the tell, the skid and a wait at a
  danger's edge (`kept out`, which the edge hold sets).
- **The bar**: `offbox-fire` 0 in every run; every stall, strand,
  never-arrived, tank-grind or spin read round by round from its `ANOMALY`
  lines, and one the rail's own action causes fixed, not re-baselined.

**Recorded 2026-10-07** (release build - the same answers as a debug one,
checked byte for byte on a round -, `--rounds 10 --seed 1000`; the 9
fixtures at 1800 frames, the 7 field maps at 3600; minutes of round in
brackets):

| Run | Fixtures | Fields |
|---|---|---|
| Defaults | border-stuck 4, jitter 32, spin 3, churn 34, clustering 10, pile-up 6 (11.1) - every map's output byte for byte the EMP's reviewed head (`7a85ed5`) | border-stuck 11, jitter 108, spin 24, churn 83, clustering 12, wall-grind 1, pile-up 8 (18.9) - byte for byte |
| `--crate gauss_rail` | as the defaults (no weapon slot) | spin 28, clustering 29, pile-up 10, wall-grind 1 (19.1) |
| Mix: chance 0.5, rail share 0.5 | spin 4, clustering 8, pile-up 4 (11.6) | spin 12, clustering 3, pile-up 1, wall-grind 2 (16.8) |
| Every enemy armed, by day | spin 4, clustering 14, pile-up 7 (12.5) | spin 23, clustering 23, pile-up 12, never-arrived 3 (17.2) |
| Every enemy armed, night | clustering 9, pile-up 6, never-arrived 1 (12.3) | spin 31, clustering 20, pile-up 9 (19.2) |
| Every enemy armed, commander on | spin 3, clustering 12, pile-up 7 (11.5) | spin 23, clustering 19, pile-up 8, wall-grind 1 (17.2) |

No stall, stale start, low progress or tank-grind in any run; `offbox-fire`
0 in every run, and no shot or hit on a seat from off its box. The default
map's 30-round sweep, `waves-basic`, the advance scenario on maze and
hedge-maze, two seats on the default map and on archipelago, and the perfect
defence on longwater are byte for byte the base's too.

**Against the earlier weapons** (their docs' §11, the same settings): the
EMP armed by day ran stall 2, tank-grind 1, spin 24 on the fixtures (42.4
min) and stall 2, low-progress 5, never-arrived 5, tank-grind 9, spin 79
on the fields (60.6). The rail's rounds are the defaults' length - a rail
pack kills an AFK seat as a shells pack does, where an EMP pack fires
nothing at a bare seat and its rounds run to the frame cap - and per ten
minutes of round its spins (3.2 and 13.4) sit by the defaults' (2.7 and
12.7) and the EMP's (5.7 and 13.0).

**Read round by round** (`classify.py`: every anomaly replayed on the
window at its seed, the AI tiers its tank ran over the 180 frames before):

- **The rail's own bugs, fixed** (decisions 22 to 24, 27): a tank chasing
  a seat in a charging ally's lane swung north and south the whole round as
  the ally crept and recoiled (never-arrived, frontier 0x3e9); two rail
  tanks either side of a seat each held fire for the other with a third
  pushing into them from behind (tank-grind and low-progress, frog-block
  0x3eb); a tank at the sight box's edge charged and vented for 60 s
  (never-arrived, frontier 0x3eb); a tank charging from 350 px, past the
  attack range the probe measures arrival by (never-arrived, maze 0x3f1).
- **What remains**: the never-arriveds are hedge-maze 0x3eb (a seat in the
  north-west corner no enemy is alerted to; three tanks fleeing from ram
  damage or patrolling) and portals 0x3ef at night (a chase round the
  portal) - the same tanks never arrive with every enemy on the EMP
  instead, so they are the armed pack's, not the rail's. Of the spins, 1 to
  4 a run have a lane's dodge in their window (4 of 27 armed, 3 of 31 at
  night, 2 of 26 with the commander, 2 of 16 in the mix, 1 of 31 with the
  crate): a tank stepping out of an ally's charging lane walks round a
  hedge or a tank to its exit and comes back; the rest are the tree's
  tiers - patrol, chase, attack, guard and the seeks.
- **Clustering and pile-up** rise with the crate (29 and 10 against 12
  and 8 on the fields) and with the armed pack (23 and 12): a rail tank
  with a lane stands its ground to charge and to wait out its interval, so
  the pack behind it bunches - 9 of the armed run's 37 clusterings and 3 of
  its 19 pile-ups have the rail's rule in their window, the rest the attack
  and chase tiers converging as they do on shells. Per ten minutes they sit
  under the EMP's armed pack (clustering 13.4 against 20.8, pile-up 7.0
  against 13.7 on the fields).

## 12. Interactions, decisions, what is left out

### Interactions with what ships

| With | What happens |
|---|---|
| Rainbow shield | Soaks one slug whole (and almost always shatters); the slug flies on past it |
| Heat shield, speed boost, ooze coat | No effect on the slug; the crawl scales the boosted top speed and the coat's pace |
| Portals | Leg by leg, as the laser; a charge holds through a teleport |
| Water, ice | Crossed; fish scatter; a recoil slides further on a ford, wet ground and ice; a charging hull in a ford crawls at the ford's pace times the crawl |
| Night, storm, fog | The charge and the white frame throw light; an enemy fires at a seat only within the sky's sight range |
| Rain, snow | Longer recoil skids (grip) |
| Sandstorm gusts | The crawl drives in the gust's frame, as every drive does |
| Towers | Take the slug's damage, no deflection, the slug flies on; an enemy prefers a lane with a player tower, never fires one through its own |
| Frogs | `gauss_frog_damage` times the keep, either side's, no hop; a hunter rails its quarry; an enemy never fires through its own side's frog |
| Crates | Flown over, never broken |
| Drums, oil, fires | A drum goes off where the slug finds it; a fused one is passed; oil and fires untouched, lit by a drum's blast as ever |
| Trees, grass | Trees go down or catch fire; grass along the line lies flat for a moment, hiding whoever it hid |
| Glass, walls, sandbags, fences, lamp posts | Gone; iron, a door and a cone stop it |
| Lava, the volcano | Crossed; the cone stops it |
| Shells, bullets, plasma, missiles, grenades, globs, lava bombs | Untouched |
| Wrecks | See-through |
| Field maps | Crosses the whole field; the sight box binds every use; a coasting far tank's charge never fires; a slug hit raises the director's intensity like any |
| The couch and its split | A teammate in the lane is hit (`friendly_fire_damage_factor`); the slug is drawn in both halves' worlds |
| Training | `drop = ["gauss_rail"]` works by its name; a dummy never charges |
| The C2 commander | Never orders a charging tank |
| Online | §3.3, §8 |

### Interactions with the sonic hammer (built here)

| Hammer | Rail |
|---|---|
| A shove on a charging tank | The charge holds; the skid carries it, facing kept |
| The hammer's trouble (`lands_in_trouble`) | A charging enemy rail's lane is trouble where the shove leaves the seat at rest: a hammer enemy shoves a seat into it, through cover, whatever the range |
| A hammer tell in a rail lane | The tell holds (the special tier is above the dodge); the slug goes through the tank |
| The skid, the knock | The rail's recoil is a knock: one skid model, one validator allowance |
| The wave and the slug | Independent: neither stops nor triggers the other |
| The online claim | One pending claim per kind: a seat's hammer blast and rail slug claim their own events |
| The probe's holds | The tell, the skid and a charge are each a deliberate hold |

### Interactions with the EMP (built here)

| EMP | Rail |
|---|---|
| The ring reaches a charging tank | The charge is cleared (`Tank::disable`); the special is offline, so no charge starts for `emp_disable_seconds` |
| A disabled tank carrying the rail | Its module shows the idle cell; its slugs are kept |
| A seat charging a rail within an EMP enemy's ring | Worth `emp_ai_special_value` as any seat carrying an online special: the pulse kills the charge |
| The dangers and the dodge | One tier, one latch, one held exit walked to on foot: the EMP's disc and the rail's lane; the edge hold is the lane's (the disc keeps its slack band) |
| The EMP brawler's approach | Keeps out of a charging rail's lane (`out_of_danger`) |
| `WPN OFFLINE` and the charge gauge | Never at once: an offline special cannot charge |
| The online claim | The rail's and the EMP's presses claim their own events |

### Decisions taken

1. **A release before full fires nothing.** The rail's identity is the
   commitment: a weak shot would make it a spammable sniper, blur the tell
   (every glow would be a threat at any moment of the charge), and give
   enemies' "wastes its charge" no meaning. It also keeps the online
   threshold to one kind of show. Rejected: a weak slug scaled by the
   charge. *For Oto.*
2. **A slug used on a seat goes through whatever else is in the lane**,
   another seat whose sight box the shooter stands outside included - as a
   shell that misses its target flies on. The decision to fire is held to
   the sight box of the seat it is used on, and recorded as that seat; the
   rest is collateral, warned by the arrow and the lane ring. Rejected:
   holding the release while any seat down the lane is off-box, which would
   let a seat at the far end of a long lane shield the near one. *For Oto.*
3. **Frogs take `gauss_frog_damage` (20), not the slug's damage**: a slug
   would otherwise kill a frog outright, and a hunter with a rail would end
   a Protect round from across the field in one charge. Two slugs kill a
   frog. *For Oto.*
4. **An enemy's slug is lighter than a seat's** (60 against 120): the
   shells' split, so a seat survives one slug at full health and an enemy
   does not - even through three tiles of cover (120 x 0.95^3 = 103), the
   shot the armory's screen is built for; at 110 it fell short (94).
5. **A rainbow shield soaks one slug whole** (and shatters), through the
   ordinary `take_damage` seam, and the slug flies on: the shield is the
   counter to a slug aimed at you, not to one aimed past you. Rejected:
   deflecting it (a slug bouncing back along its lane through its own side
   is chaos with no counterplay) and piercing it (the shield would be
   worthless). *For Oto.*
6. **The "at 11" is built**: the overcharge is a second threshold of the
   same charge, cutting iron is a stop rule, the spin a facing and a lock
   during the recoil's skid. Holding past full is a risk (crawling longer,
   seen longer, a vent soon after) for a reward (iron no longer covers).
   An enemy never overcharges, so iron stays precious against enemies.
7. **A charging seat in grass still makes its lane a danger**: the glow is
   drawn over the tufts and lights the lane; the seat stays hidden from
   targeting. Rejected: the EMP's "a hidden armed seat is a trap", which
   would make the brightest weapon in the game invisible. *For Oto.*
8. **The charge is a sibling of the tell**, sharing its drawing per weapon,
   its arrow and its lapses, rather than a tell: a tell commits and fires by
   itself, a charge is held, moves and fires on the release.
9. **The trigger kinds are named** (`Trigger`) rather than matched weapon by
   weapon in `drive_player` and the predictor, so the rod adds one
   `ChargeRule` and no match arm there.
10. **The room counts the client's ticks** (the hold report), held within
    six of its own: without it, a release on the full tick could be a slug
    on the client and a fizzle in the room once in a while, and the slug is
    the weapon. It reads facts the wire already carries (the intents' ticks)
    and follows `pose_reach_ticks`' trust.
11. **The crawl lowers the top speed**, not the acceleration, at
    `intent.slow`'s seam, and `throttle` says so. The pose validator does not
    enforce it on an owned hull: its floor already allows more than full
    speed, which is the existing trust.
12. **The recoil is the hammer's knock**, sized to a cell by the skid's own
    friction: a plain impulse along the hull's axis is braked in a few
    pixels (the hammer's decision 1).
13. **Walls, props and trees die outright, towers take the damage**: "pierces
    brick, wood, glass" read as gone; a tower is the one tile the slug may
    leave standing, so a tower line is not deleted by one slug.
14. **Lamp posts fall**, as they do to a shell: the hammer leaves them
    because its wave is not a shot; the slug is.
15. **No breach with the rail**: a rail tank does not spend slugs on the
    wall it is stuck on; its stuck escape does that. Revisited if the probe
    strands one.
16. **The AI counts a seat it can see by the sky's range, through cover,
    not hidden** - the alert's own rule made exact - rather than reading
    the alert point, which is the same position whenever it is fresh.
17. **Lanes are dangers and an edge hold** together: the danger empties
    the lane, the edge hold keeps it empty without the dodge's
    back-and-forth.
18. **The pierce bursts are part of the slug's picture**, composed from its
    age, so a replica and the shooter's own client draw them alike from the
    leg; `Hit` adds only the flash.
19. **The crate cooks off**, as the laser's and the plasma's do: slugs and
    charged capacitors.
20. **Crate ink hot magenta** (`#FF3DD8`): jade, the designed ink, was
    rendered beside the other fifteen crates and the grass and read as
    green on green; magenta is the most distinct of the candidates and
    nothing on the field shares it (§5). *For Oto*, with the comparison.
21. **The module on the laser's cheek**, not the roof: a rail reads as a gun
    along the turret, and the roof hardpoint already holds the missiles,
    grenades, hammer and EMP.
22. **No route surcharge on a lane**, though the design had one
    (`gauss_ai_lane_cost`): a lane runs to a permanent tile or the field's
    edge and lives a second or two, so a surcharge only flipped the flow
    field between a detour and the straight route each time a charge
    started and ended. The armed sweeps read the same with it at 16 and at
    0; the edge hold and the danger do the work.
23. **A charging enemy stands its ground**, though the design had it creep
    along its lane (`gauss_ai_creep_min_px`): creeping in and being thrown
    back a cell by each release kept moving the cell it stood in, and on
    the study map a tank chasing a seat behind it swung north and south
    the whole round and never arrived (`never-arrived` on frontier 0x3e9;
    gone without the creep, as the low-progress and tank-grind on
    frog-block 0x3eb were). The seat's crawl is unchanged.
24. **A friend beyond the target does not hold the rail**: only one between
    the shooter and its seat (or quarry) does, as a friend in the line holds
    a shell. Holding fire for any friend down the lane left two rail tanks
    either side of a seat on its row each waiting on the other, the rest
    queued behind them (frog-block 0x3eb: tank-grind and low-progress); the
    slug flies on into the far one, as a shell that misses would. A lane
    that scores on player towers alone is no target either: the rule waits
    for a seat or the quarry to release at.
25. **The rail's light is a cold blue off the palette** (`RAIL_LIGHT`): the
    ramp's teal, added over the grass, read as green in the screenshots;
    the blocks keep the ramp.
26. **A release reaches the room a tick after the key comes up**, on the
    packet after the fire hold's (`client::FIRE_HOLD_TICKS`, which keeps a
    tap from being lost between two samples): the client's sandbox steps
    the same packets, so its slug and the room's leave on the same tick.
27. **A charge starts a cell inside the box; it is released anywhere in
    it** (`gauss_ai_box_margin_px`): a tank at the box's very edge charged,
    drifted a pixel out while it held and vented, over and over (frontier
    0x3eb, the whole round). The margin is the hysteresis, and at a cell a
    charge on a seat's row starts within the attack range (336 of 340 px),
    where every other enemy engages - the probe's `never-arrived` measures
    arrival by that range.

### Not in this PR

- Placing the rail in the shipped levels - level design, a follow-up.
- An aim line for a charging seat, and a warning to a teammate standing in
  its lane - HUD design of their own; the glow and friendly fire are the
  warning for now.
- The danger and the lane warning following a charging slug's legs past a
  portal - the exit is the room's draw at the release.
- The prediction report's crossing counts for slugs (`crossings_hit`,
  `crossings_missed`, docs/online-coop-prd.md decision 9) - the room's
  `Hit`s already say what a slug went through.
- A scorch or a burnt line on the ground along the slug, and a dent in iron
  where it stopped - the trail and the stop's sparks are the mark.
- A minimap mark for a charge or a lane.
- Sound effects - the game has no audio yet.
- Breaking crates, shooting down missiles or shells, or putting fires out
  with the slug - each a mechanic of its own beyond the issue.

### What this PR adds to the shared path

The hammer's and the EMP's hooks as they shipped took the rail with these
additions, kept general for the rod (BB-41) and whatever charges after it:

1. **The charge-and-hold pattern** on `Tank`: `Trigger`/`ActiveWeapon::trigger`,
   `ChargeRule`/`charge_rule`, `Charge`, `ChargeStage`, `ChargeEdge`,
   `ChargeEnd`, `step_charge` with the hold report's clamp, `charge_pace`,
   `Tank::windup` answering a charge; `drive_player` and `enemy_trigger`
   routing a charge weapon's trigger every tick; `coast_enemy` holding a
   charging trigger down.
2. **The AI hook**: `windup_rule`'s charge arm (a charge is its weapon's to
   hold or let go, never `None`), `SpecialUse::{Charge, Release}` in
   `act_special`, `DangerShape::Lane` beside the EMP's disc (its `depth`,
   `exits` the dodge walks to and holds, `posts`, `middle`), the edge hold
   (`enters_lane`, `Ai::kept_out`), `command::Busy::Charging`.
3. **The knock**: `sonic::knock_hull` (the hull's half alone, for the
   prediction sandbox) and `knock_with`'s echo switch; `Shoves::allow_knock`
   for a knock the room puts on no `Shoved`.
4. **The prediction**: `Game::predict_seat_with` (the trigger stepped in the
   room's order, its edge reported), the predictor's history keeping each
   tick's press edge and gate, `PressShow::{Rail, ChargeEnd}`,
   `Game::draw_rail_press`, `Game::{seat_charge, set_seat_charge}`,
   `PresentWorld::rail_trace`.
5. **The hold report**: `Mailbox::hold_ticks`, `authority::take_hold`,
   `Game::set_seat_hold` - in the room server's tick and the rig's.
6. **The wind-up arrow's lane**: `ArrowKind::Windup { lane }`, set from
   `TankView::lane` for a charging rail.
7. **The weapon slot's gauge**: `WeaponSlot::charge` drawn in the count's
   place by `draw_weapon_readout`.
8. **The probe's holds**: a charge (`TankSnapshot::charging`) beside the
   tell, the skid and a wait at a danger's edge.
9. Small: `Dir::opposite`, `ChargeStage::name`, `grass::flatten_along`.
