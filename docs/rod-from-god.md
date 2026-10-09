# Rod from god

BB-41, the fifth of the six weapons of BB-36. A special weapon from its own
crate (`pickup = "rod_from_god"`), one at a time like every other
(`Tank::take_weapon`): a crate loads `rod_per_pickup` (2) calls, a re-pick
stacks two more up to `rod_max` (6), another weapon's crate replaces it. Seats and enemies
alike: an enemy takes the crate while it carries no special, or to stack
the calls it carries (`Tank::wants_pickup`) and uses it by its own rule (§4).

Hold the trigger and a reticle appears a few cells ahead of the tank; steer
it with the stick while the hull stands still; let go and the strike is
called. A thin red beam drops onto the circle from the top of the screen and
a countdown runs from four. At zero a tungsten rod arrives: a one-frame white
column, a flash, a shock ripple and a ring of dust. Every hull with any part
inside the circle - a cell and a half - is wrecked, whoever's it is; hulls
out to five cells are knocked off their tracks; every breakable tile within
two and a half cells comes down and iron stands. On a lake it throws the
fish onto the bank; on a volcano it sets the eruption off; where it lands it
leaves a crater for the rest of the round, a pit that slows tanks and fills
with water when it rains. Counter: the telegraph - leave the circle.

The sonic hammer's doc (docs/sonic-hammer.md §3), the EMP's
(docs/emp-burst.md §3.3), the gauss rail's (docs/gauss-rail.md §3) and the
FPV swarm's (docs/fpv-swarm.md §3) lay down the shared path this plugs
into: the AI hook, the wind-up and its arrow, the online press show claimed
by input tick, the knock and its skid, the carried/fired split, the
disabled state, the dangers and the edge hold, the **charge-and-hold
trigger**, air targets, the probe's `--crate`, the spawn swap,
`spawn_pickup` and the armory. This PR adds **zones** (§3.3): a world-placed
area with an owner, a centre, a radius and an end on the round clock, which
the AI keeps out of, the router prices, the indicators point at and the wire
carries as a family - the rod's call is the first, and the gravity well
(BB-42) is the second. It also adds the per-seat **motion record** (how long
a seat has stood still, how fast it moves), which the well's AI reads too.

## 1. How it plays

### The reticle: press, steer, let go

- **Press to start.** On the trigger's press edge, with a call left and the
  cooldown out (`Tank::fire_cooldown`), a charge starts - the rod is a
  `Trigger::Charge` weapon (docs/gauss-rail.md §3.3) with its own
  `ChargeRule` (§3.2) - and a reticle appears on the map cell
  `rod_reticle_start_cells` (4) cells ahead of the hull's cell along its
  facing (`Dir::from_rotation`), held inside the reticle's range (below).
  Online the shooter's client draws it on the press frame (§8).
- **The stick steers the reticle, not the hull.** From the tick after the
  press (the drive runs before the trigger in `drive_player`, as for the
  rail's crawl) the seat's stick (`Intent::move_dir`) moves the reticle and
  the hull is driven with no stick: it brakes to a stop and stands, its
  facing kept (`ChargeRule::crawl` 0, `ChargeRule::stick = Stick::Aim`). A
  direction pressed moves the reticle one cell at once; held, it moves on
  after `rod_reticle_delay_seconds` (0.18) and then a cell every
  `rod_reticle_repeat_seconds` (0.06) - a tap is one cell, a hold crosses
  the box's half width in under a second. Released, it stays. Turning the
  hull before the press decides where the reticle starts; while it is held
  the hull does not turn at all. On a touch screen the floating stick
  steers it the same way while a thumb holds the fire half; the couch's
  second seat steers it with WASD while it holds Left Shift.
- **Its range is the caller's sight box**, the screen the follow camera
  always shows round a seat: the reticle's cell centre is held within
  `sight_box_half_cols - 0.5` cells sideways and `sight_box_half_rows -
  0.5` cells up and down of the hull's cell (11 and 7 at the defaults),
  and inside the field. A step that would leave the range is not taken; a
  hull moved under a held reticle (a shove's skid, a teleport) pulls the
  reticle back into range, which counts as a step (`rod::Reticle`, §2).
- **Let go to call.** The release fires the charge
  (`ChargeEdge::Released`): the strike is called on the reticle's cell
  (below), one call spent, `rod_reload_seconds` (1.0) before the next
  reticle. The caller is free at once - it drives, turns and fights while
  the countdown runs.
- **A tap calls nothing.** Released before `rod_settle_seconds` (0.25)
  held, the charge fizzles (`ChargeEnd::Fizzled`): nothing called, nothing
  spent, no cooldown.
- **Let go on your own tank to cancel.** Released with the reticle on the
  caller's own cell (`map::world_to_cell` of the hull's centre), the call
  is cancelled - a fizzle: nothing called, nothing spent. The reticle is
  drawn crossed out there and the prompt says so (§5, §7), so steering it
  home is the way out of a call the seat no longer wants.
- **Held too long, the uplink times out.** Held `rod_hold_seconds` (10)
  past settled, the charge vents (`ChargeEnd::Vented`): nothing called,
  nothing spent, `rod_vent_cooldown_seconds` (0.5) before a reticle may
  start again. A new reticle needs a new press.
- **It lapses** (`ChargeEnd::Lapsed`) - nothing called, nothing spent - as
  every charge does: the caller wrecked, disabled by an EMP (the uplink is
  electric), another weapon's crate taken, the special taken offline, the
  round ended. A rod crate taken mid-reticle stacks its calls and keeps it. It holds
  through a teleport, a hammer's shove, a hit and a shield breaking (the
  rail's rules), the reticle pulled back into range where the hull now
  stands.
- **One trigger.** While a tank carries calls its trigger is the rod's: a
  seat fires no shells until its calls are spent, as with every special
  (`Tank::active_weapon`). A seat that wants its cannon back calls both -
  press, let go, a quarter second each.

### The call

- **On a cell.** The strike lands on the centre of the reticle's map cell
  (`map::cell_to_world`), the struck cell. Anywhere in range: on open
  ground, on a wall, on water, on a cone.
- **A zone.** The call is a `Zone` (§3.3) of kind `ZoneKind::Rod`: its id
  from the round's projectile counter (`Game::take_shot_id`), its owner the
  caller (`Owner`, and its seat if it is one - the kill credit), its centre
  the struck cell's, its radius `rod_kill_radius_px` (48), its end
  `rod_countdown_seconds` (4.0) later on the round clock. It belongs to the
  world, not to the caller: **a call once made always lands** - the caller
  wrecked, disabled, teleported, gone through a gate, or the round paused
  (the clock stops with it) changes nothing.
- **Seen by everyone, the whole countdown**: the red beam onto the circle,
  the circle, the countdown's number (§5), and off the screen an arrow that
  is never dropped (§5, "Off the screen"). Every enemy keeps out of the
  circle from the call's first tick (§4).
- **Events**, on the release tick: `Event::Fired { weapon: "rod_from_god" }`
  then `Event::RodCalled { id, slot, seat, cell, land }` (the struck cell,
  the end on the round clock), in that order, in one tick - which
  the online claim reads (§8). The reticle's start and end log the rail's
  `ChargeStarted` and `ChargeEnded`.

### The impact

At the call's end (`Game::resolve_zones`, after the hammer's, the EMP's and
the rail's resolvers, before `step_world`, so the shoves land in this tick's
solver step), in this order, all measured from the struck cell's centre `c`:

- **The kill circle** (`rod_kill_radius_px`, 48, a cell and a half):
  - *every live tank on the field* - either side, a teammate, the caller
    itself - whose hull box (`Tank::hull_bbox_world`) has its nearest point
    within the radius is **crushed** (`Tank::crush`): a live rainbow shield
    pops (`shield_hp`, `shield_timer` and `shield_recharge_delay` to 0,
    `shield_broke` set, so `drain_shield_breaks` logs the ordinary
    `ShieldBroken` later in the frame) and the hull takes `MAX_DAMAGE` whole,
    past the shield, the hull armour and `friendly_fire_damage_factor`;
    `mark_hit`, `credit` from the caller (the kill and `last_hit_by` are the
    caller's, a dead caller's too), `Event::Hit { cause: HitCause::Rod,
    killed: true }` at the hull's centre, the kill onto `f.kills` (the
    wreck's own fireball, mushroom and parts come from `explosions` as
    every kill's). A wreck is left alone; a tank rolling in through a gate
    is off the field.
  - *every frog* - either side's - whose collider box
    (`FROG_COLLIDER_HALF_EXTENT`) reaches inside the radius is killed
    (`Frog::damage` with its whole health; the Protect round is lost the
    frame the players' frog dies, as ever).
  - *every air target* (docs/fpv-swarm.md §3.3) - either side's drones -
    whose ground point is within the radius is downed
    (`Game::strike_air(.., AirStrike::Rod, ..)`): the rod passed through
    the column of air over the circle (§12, decision 10).
- **The break radius** (`rod_break_radius_px`, 80, two and a half cells),
  measured to each cell's box, so the 5 x 5 block round the struck cell and
  the four cells three out along its axes:
  - *every breakable tile* - brick, wood, glass, sandbags, fences, range
    boards, trees, pines, lamp posts and the three towers, either side's -
    is **crushed**
    through `damage_obstacle` with `DamageCause::Crush { from: c }`: it dies
    outright, no fence roll, no deflection, not left burning (a crushed
    plank or tree is rubble, not a fire; one already burning is crushed
    too, its rubble charred), its death the ordinary one
    (`obstacle_died`: `ObstacleDestroyed`, its rubble, the edge masks, a
    tower's ruin and discharge, cook-off or spill). Walked in cell order.
    **Iron, a volcano's cone and a training door stand**
    (`Material::is_permanent`), sooted on the face toward the impact as a
    blast soots a wall.
  - *a drum* dies the same way, so it goes off at once where it stands - a
    direct hit's pop (`obstacle_died` queues its blast) - and a fuel drum's
    neighbours chain as ever. A drum with a fuse already burning is passed
    over (`damage_obstacle` leaves it to its fuse).
  - *a grenade*, on the ground or hopping, goes off at once
    (`Grenade::fuse` to 0, burst by `resolve_grenades` this frame), in id
    order.
  - *a seat's lantern* is broken (`Event::LanternBroken`), in id order.
  - *oil trail cells* not yet burning are lit (`light_cell`, sorted), as a
    drum's blast lights the trail in its reach.
  - *breakable crates* (`crate_breakable`, off by default) break
    (`blast_crates` with the rod's params); with crates unbreakable, as by
    default, a crate in the circle is left lying in the crater.
  - *tall grass* hides nobody for `rod_grass_flat_seconds` (8)
    (`Game::grass_flat`, the hammer's - docs/sonic-hammer.md §1), and its
    tufts out to the shove radius are laid flat (`grass::flatten`,
    cosmetic).
- **The shove ring** (`rod_shove_radius_px`, 160, five cells):
  - *every live tank on the field outside the kill circle* whose hull box's
    nearest point `d` is within the radius is **knocked** (`Game::knock`,
    the hammer's shove and skid, docs/sonic-hammer.md §3.6) along the line
    from `c` to its centre at `v = min(rod_shove_max_speed,
    rod_shove_speed * falloff / m^rod_mass_exponent)`, `falloff = (R_shove
    - d) / (R_shove - R_kill)` - 1 at the circle's edge, 0 at five cells -
    and `m` its chassis's mass factor. Either side, the caller too. At the
    defaults a standard chassis at the circle's edge slides about four
    cells, one halfway out about a cell. It is a skid: the hull drives
    again when it stops, a wall or the field's edge stops it flush against
    its face (the knock's look-ahead, docs/sonic-hammer.md §3.6; §12,
    decision 34), ice and a ford slide it further.
    A seat's shove on `Frame::shoves` (`Event::Shoved` with its skid, for a
    client that owns its hull). **No damage** outside the circle: the
    circle is the danger, the ring is a push (§12, decision 7) - though a
    push can throw a hull into lava, a tower's reach or another call.
  - *every frog* in the ring (its centre) is stunned for
    `rod_frog_stun_seconds` (1.0) (`Frog::stun`, the hammer's): neither hop
    nor bite.
- **The ground**: a scorch, a rubble ring and a crater (§1, "The crater")
  on dry ground; a splash and the fish on water (below).
- **Events**: `Event::RodImpact { id, cell, crater, erupted }` first, then
  the ordinary events of what it did, in the walk order above (`Hit`,
  `ObstacleDestroyed`, `Blast`, `LanternBroken`, `DroneDowned`, `Shoved`,
  `ShieldBroken`, `Wreck`).
- **The show** (§5): the white column, the screen flash, the ripple and
  shake, the impact flash, the fireball, the dust ring, the debris, the
  scorch and the rubble ring.

### On a lake, on a volcano, at the edge

- **A lake** (the struck cell, or water within the shove radius): the
  impact kills, shoves and breaks as on land - a hull on a ford in the
  circle is crushed, deep water holds none. The fish within
  `rod_fish_reach_px` (160) of `c`, nearest first, at most
  `rod_fish_throw_max` (8), are **thrown onto the bank**: each to the
  nearest dry cell past it along the line from `c` (else the nearest dry
  cell to it), where it flops for `sonic_fish_flop_seconds` and hops back
  into the nearest deep cell - the hammer's flop (`fish::Flop`),
  presentation only, hashed, from the `RodImpact` event, so a replica
  throws the same fish. On water the show is a splash (§5), no scorch and
  no crater; water cells are never made pits.
- **Ice** (a snowy round's frozen water): the impact does not break the
  ice; the show is a burst of white powder; no crater on ice.
- **A volcano**: an impact whose kill circle reaches a cell of a volcano's
  footprint (`volcano::in_footprint`, the cell's box within the radius)
  **sets that volcano off** - the eruption begins on the next tick, its
  bombs, shock ring, flash and surge exactly an eruption's (the cone
  itself stands: permanent). The volcano's cycle stays a pure function of
  the round clock: the strike moves the cycle's offset, by a whole number
  of ticks, so that the eruption it leads up to starts now
  (`volcano::set_off_shift`, §3.2 and §9) - from asleep or cooling the next
  eruption comes now, from a rumble that rumble's eruption comes now, and
  an erupting volcano is left as it is. The next one comes a full
  `volcano_period_seconds` later. The shift travels on the wire in the
  round state (§8), so every replica's volcano erupts on the room's tick.
  The rumble a rod skips is marked shown, so no tremor plays for it.
  `RodImpact::erupted` says so.
- **Lava** (a ford or a lake's middle): no crater (the lava stays), smoke
  instead of dust (§5). A hull pushed into it burns as ever.
- **The field's edge**: the reticle never leaves the field; a circle by the
  edge is cut by it; crater cells past it are dropped.
- **Portals**: the rod is not a shot - nothing about a portal changes
  where it lands, and an anchor inside the circle is untouched. A hull
  shoved into a portal's trigger goes through it as any tank does
  (`portal_phase`), the teleport ending its skid.
- **Gates**: a tank rolling in through a gate is off the field until it
  arrives and is neither crushed nor shoved; once it has an `Ai`, it keeps
  out of the circle like the rest.

### The crater (the "at 11")

The issue's at-11 is built: it fits the round's existing terrain systems
(§12, decision 12).

- **Made at the impact**, on dry ground: the struck cell and its
  neighbours within `rod_crater_reach` (1: the cell and its four
  neighbours, a plus) that are inside the field, dry (`WaterLayout` and
  `LavaLayout` both `Depth::Dry`) and outside every volcano's footprint
  (`Game::make_crater`, `rod::crater_cells`). A cell already a crater stays
  one; two craters that touch are one pit. Kept for the round in
  `Game::craters` (`rod::Craters`: the cells, sorted, and the list of
  craters for the drawing - struck cell and round time), cleared by
  `init`. A permanent tile standing on a crater cell (an iron block next to
  the struck cell) keeps its cell; nothing drives there anyway.
- **A pit slows tanks**: a hull whose centre is on a dry crater cell gets
  `rod_crater_pace` (0.6) of its top speed and push (`Footing::at`'s crater
  term, beside the ford's and the lava's; grip, traction and brake as on
  dry ground). The router prices a dry crater cell `rod_crater_path_cost`
  (2) a step (`Grid::weigh` in `nav_finish`, the fords' rule), so a route
  goes round a crater when that is no longer.
- **It fills with water when it rains**: a crater made while the sky in
  force is rain or a storm (`weather::fills_craters`, beside
  `weather::freezes`) is a ford from the impact on - its cells become
  `Depth::Shallow` in `Game::water` (`WaterLayout::fill`, current none) -
  and every water rule follows from that one place: the ford's pace and
  grip in place of the pit's, the router's ford price, wet tracks and
  spray, a burning hull put out, no fire, heat or scorch on it, a frog's
  hop drawn to it. No fish (a ford is not deep). A sky turned to rain
  mid-round (the dev server's `weather`) fills every crater then
  (`Game::change_weather`); a sky turned to snow freezes them with every
  other water cell (`WaterLayout::freeze`, already there); nothing dries a
  filled crater before the round ends. A crater made under snow stays a
  dry pit.
- **The nav cache** is cleared when a crater is made or filled
  (`self.nav.clear()`, as `change_weather` does): a pit and a puddle change
  prices, not occupancy, so the kept base would otherwise go stale.
- **Drawn** as part of the floor, under the tracks and the scorches (§5);
  on the minimap as terrain (§5). On the end screen no crater is made.

### A wreck, an EMP, a shove, a teleport

| What happens | During the reticle | After the call |
|---|---|---|
| The caller is wrecked | The charge lapses: nothing called | The call lands; its kills are the caller's |
| The EMP's ring reaches the caller | `Tank::disable` lapses the charge; the special is offline, so no reticle for `emp_disable_seconds` | The call lands |
| A hammer's shove on the caller | The charge holds; the reticle is pulled back into range where the skid leaves the hull | Nothing |
| The caller teleports | The charge holds; the reticle is pulled into the new range | Nothing |
| Another weapon's crate | The charge lapses next tick | Nothing: the call is the world's |
| A rod crate | Stacks two calls, up to six; the charge holds | Stacks |
| The round ends | `end_round` lapses it | The call lands on the end screen, harmlessly |
| The round restarts | `init` clears it | `init` clears every zone |

### Several rods at once

Calls are independent: each its own zone, circle, beam and countdown. A seat
with two calls can make both a second apart. Calls whose ends fall on the
same tick land in id order, each impact whole before the next; circles may
overlap (a hull crushed by the first is a wreck to the second). Craters
union.

### On the end screen

`player_phase` and `enemy_phase` do not run and `end_round` has lapsed every
reticle, so nothing is called. Calls already made run out their countdown
and land with their whole show - `resolve_zones(f, false)` - and touch
nothing: no crush, no shove, no tile, no frog, no drone, no crater, no
eruption, as a blast on the end screen hurts nobody.

## 2. Where it lives

| File | What |
|---|---|
| `src/zone.rs` (new) | Zones (§3.3): `Zone` (id, kind, owner, centre, `until` on the round clock - the radius is its kind's knob), `ZoneKind` (`Rod(RodCall)` here; the well adds its own), `Zone::{left, radius, danger_radius, danger, holds, route, wire_kind, rod, provisional}`, `ZONE_ROD`, `PROVISIONAL_ZONE_BASE`; the rules every reader keeps, in its doc |
| `src/rod.rs` (new) | The weapon's headless half. `RodCall` (struck cell, the caller's seat), `SeatStill` (`step`, `speed`), `Steer`, `Range` (`of`, `hold`, `holds`), `Reticle`, `reticle_start`, `step_reticle` (pure: a stick or an aim cell, the range, the step clock), `rule()` (the rod's `ChargeRule` from the `rod` knobs), `cell_reach`, `falloff`, `shove_speed`, `crater_cells`, `Crater`/`Craters`, `lens`, the composers `compose_reticles`, `compose_calls`, `compose_reticle`, `compose_designator`, `compose_call`, `compose_call_ring`, `compose_column`, `compose_impact`, `compose_crater_smoke` (pure, `pyro::Shape`s), `RodImpactFx`, `Ground`, `CraterWater`, `draw_crater` (generic over `canvas::Canvas`), `module_cell` |
| `src/simulation/rod.rs` (new) | The world half. `fire_rod` (a release: the call, or the cancel), `place_calls`, `resolve_zones`, `rod_impact` (the walk of §1: `rod_hulls`, `hulls_in_order`, `knock_from`, `rod_frogs`, `rod_tiles`, `rod_ground`), `crater_cells_at`, `make_crater`, `fill_craters`, `struck_volcanoes`, `set_off_volcanoes`, `shift_volcano`, `ground_struck`, `rod_show` (the cosmetic half, which a replica's event takes too), `Game::{zones, craters, seat_still}`, `tick_seat_still`, `set_seat_reticle`/`seat_reticle_report` (a room's report, §8), `any_rod`, `zone_dangers`, `zone_route_cells`, `rod_senses` (§4), `standing_towers_of`, `debug_call_rod`, `Tank::crush` |
| `src/simulation/rod_tests.rs` (new) | The scenario tests (§10) |
| `src/simulation/gauss.rs` | The charge trigger generalised by weapon: `SeatCharge::{weapon, reticle, hull_cell}`, `charge_trigger`'s aim arm, `fire_charge` handing a rod's release to `fire_rod` |
| `src/simulation/weapons.rs` | The rod in the special-weapon dispatch's exclusions |
| `src/simulation/mod.rs` | `Game::{zones, zone_lead, craters, rod_impacts, seat_still, seat_reticle, screen_flash_strength}`; `Frame::pending_calls`; `resolve_zones` in both branches; `tick_seat_still` before `enemy_phase`; `rod_senses` handed to the special tier; the AI's `drop_charge`; `Footing::at` with the craters; the frog's shy hop from a zone; `change_weather` filling craters; `flash_screen_with`; `tick_presentation` (the impacts' ages); `Event::{RodCalled, RodImpact}`; `TankSnapshot::{rods, rod_hold}` |
| `src/simulation/nav.rs` | Crater cells weighed `rod_crater_path_cost`; every zone's cells surcharged in `route_grid_on` |
| `src/simulation/engage.rs` | `EngageCtx::herd` (§4) |
| `src/simulation/props.rs` | `DamageCause::Crush` |
| `src/volcano.rs` | `Volcano::{base, shift}`, `set_off_shift`, `set_shift` |
| `src/simulation/sonic.rs` | `SPAWN_SWAPS`' rod row; the hammer's trouble inside a live call's circle (§12) |
| `src/air.rs` | `AirStrike::Rod` |
| `src/simulation/present.rs` | `Game::{show_seat_reticle, seat_reticle, flash_seat_rod, set_provisional_zones, set_zone_lead}` |
| `src/simulation/replica.rs` | `DrawableState::{zones, craters, volcano_shifts}`, `DrawableTank::reticle` |
| `src/ground.rs` | `WaterLayout::fill` |
| `src/weather.rs` | `fills_craters`; the calls' and the impact's light |
| `src/tank.rs` | `Stick`, `Tank::{rods, reticle, stick, rod_flash}`, `step_trigger`, `kick_rod`; `ActiveWeapon::RodFromGod` (`name`, `full_load`, the charge trigger), `SPECIAL_WEAPONS`; `weapon_ammo`/`take_weapon`/`empty_stock`/`wants_pickup`; the module's cells in `module_cols` |
| `src/pickup.rs` | `PickupKind::RodFromGod` (`rod_from_god`, row 17, its ink) |
| `src/fish.rs` | `throw_from` (the rod's throw onto the bank) |
| `src/ai.rs` | `SpecialSense::Rod(RodSense)`, `RodPick`, `rod_rule`, `rod_aim_rule`, the stand-off (`rod_stand_off_px`, `rod_stand_off`, `rod_free_spot`, `rod_spot_open`, `way_open`, `rod_crowded`, `Ai::{rod_spot, rod_spot_best, rod_wait, rod_held}`), `SpecialUse::{Charge::aim, Drop}`, `Intent::{aim_cell, drop_charge}`, `Brain::in_call`, `generic_fire(RodFromGod)`; the special and air tiers yielding to a call; `SEEK_SPECIALS` gains the rod |
| `src/indicators.rs` | `ArrowKind::Zone`, `Scene::zones` |
| `src/minimap.rs` | `Class::Crater`, `Marks::zones`, `RoundMinimap`'s crater count in its key |
| `src/hud.rs`, `src/mode.rs`, `src/render/game.rs` | `HUD_ROD_COLOR`, the readout's arms, the call-in prompt (`hud::rod_prompt`, `PlayChrome::prompt`) |
| `src/game.rs`, `src/render/game.rs` | Craters in `paint_floor_marks`; reticles, designator lines, calls and the column in the glowing pass; the impact's dust and crater smoke in the lit pass; the screen flash's strength |
| `src/fx.rs` | A rod `Hit` flashes the hull and bursts nothing |
| `src/pyro.rs` | `digits` (block digits, §5) |
| `src/lib.rs`, `src/tank_art.rs` | `TANK_MODULE_ROD_COL`; `ROD_LENS` |
| `src/net/wire.rs` | `WeaponKind::RodFromGod`, `TankState::reticle`, `IntentMsg::reticle` (`with_reticle`), `ZoneState`, `CraterState`, `Snapshot::{zones, craters, volcano_shifts}`, `HitCause::Rod`, `AirStrike::Rod`; `PROTOCOL_VERSION` 20 (`net/mod.rs`) |
| `src/net/delta.rs` | `zones`, `zones_gone`, `craters`, `craters_gone`, `volcano_shifts` |
| `src/net/events.rs` | `WireEvent::{RodCalled, RodImpact}` |
| `src/net/encode.rs`, `src/net/apply.rs` | The families and the shifts (`zones`, `volcano_shifts`, `reticle_code`/`reticle_from_code`; `apply_zones`, `apply_craters`, `apply_volcano_shifts`), the impact's show |
| `src/net/mailbox.rs`, `src/net/authority.rs`, `src/net/rig.rs`, `server/src/room.rs` | The reticle report (§8): `Mailbox::reticle`, `authority::take_reticle` |
| `src/net/predict.rs`, `src/net/round.rs` | `PressShow::Rod(RodPress)`, `Predictor::{reticle, reticle_report}`; `OwnCall`, `claim_own_call`, `place_own_calls` - the client's own call drawn on the release and handed to the room's zone, every countdown on this client's present |
| `src/simulation/debug.rs`, `src/devserver.rs` | `set_tank`'s `rods` (and its `charge` on a rod tank, which puts the reticle up); the snapshot's `rods`, `reticle`, `zones` (`ZoneDebug`), `craters`; `spawn_pickup {kind: "rod_from_god"}`; the `rod_call` tool (§3.4) |
| `src/bin/probe.rs` | The tank line's `rod=`, the fire tuple, `rod-calls-on-seats`/`rod-calls-offbox`, the stand-off hold in `HOLDS` |
| `src/editor/mod.rs` | `Tool::Pickup(PickupKind::RodFromGod)` (`rod_from_god`) |
| `maps/armory.toml` | Its crates (§3.4) |
| `src/tuning.rs` | The `rod` group (§6), one row in `enemies` |
| `lang/en.ftl`, `lang/sl.ftl`, `src/text.rs` | §7 |
| `tools/punypalette.py`, `tools/spritegen/gen_crates.py`, `tools/spritegen/tankdesign/{kit,export,render,lines/vanguard}.py` | The art (§5); writes `static/crates_sheet.png`, `pickup_glyphs.png`, `tank_modules.png`, `tank_modules_glow.png`, `src/tank_art.rs` |
| `src/thumbnail.rs` | The armory's pin re-baselined (its two crates) |
| `docs/` | This, `CRATES_SPEC.md`, `SPRITESHEET_SPEC.md`, `effects.md` (who draws what), `volcano.md` (set off by a rod), `water.md` (a filled crater); `CLAUDE.md` |

## 3. The shared path

### 3.1 What this uses as weapons 1-4 laid it down

Each item of the checklist (docs/sonic-hammer.md §3.0) gets its rod arm:

1. `PickupKind::RodFromGod` (`#[serde(rename = "rod_from_god")]`, appended
   to `ALL`, row 17, `ink`, `cooks_off` false - a radio uplink and a
   kinetic rod, no explosive), `PickupKind::weapon`
   (`Some(ActiveWeapon::RodFromGod)`), `name`.
2. `ActiveWeapon::RodFromGod` (`name` "rod_from_god", `full_load` =
   `rod_max`, `tell_seconds` none - the reticle and the call are its
   tell -, `trigger` `Trigger::Charge`, `charge_rule` §3.2), appended to
   `SPECIAL_WEAPONS`; `Tank::rods` with its arms in `weapon_ammo`,
   `take_weapon`, `empty_stock`, `module_cols`. `wants_pickup` reads
   `special()`: an enemy takes the crate while it carries no special, or to
   stack its calls.
3. The `fire_charge` arm (`fire_rod`); the trigger through
   `drive_player`'s `match weapon.trigger()` (`Charge`).
4. `pickup_phase` needs nothing.
5. `ai::SEEK_SPECIALS = [SonicHammer, Emp, GaussRail, FpvSwarm,
   RodFromGod]`; `SpecialSense::Rod`, the `special_rule` arm,
   `generic_fire(RodFromGod) == false` (§4).
6. `hud::weapon_color` (`HUD_ROD_COLOR`), `hud::weapon_pickup`.
7. `gen_crates.py` (`KINDS`, `GLYPHS`), `PICKUP_INK['rod_from_god']`; the
   tankdesign module `rod` and its anchor `tank_art::ROD_LENS` (§5).
8. `editor::TOOLS` and `Tool::name`; `tool-rod_from_god`,
   `tool-short-rod_from_god` in both catalogues.
9. `WeaponKind::RodFromGod` (appended to `ALL`, both `From`s,
   `drawn_on_press` true - its show is the call, drawn on the release),
   `Predictor::seed_gate`'s arm (`rod_reload_seconds`),
   `apply::write_tank`'s ammo arm, `kick_turret` (`Tank::kick_rod`, the
   module's uplink cell) and `drawn_muzzle` (`None`).
10. `debug::{TankDebug, TankPatch}`, `set_tank`'s schema, `TankSnapshot`,
    the probe's tank line and fire tuple, `render::game::draw_tank_stats`.
11. The armory's crates (§3.4); `SPAWN_SWAPS` gains
    `(ActiveWeapon::RodFromGod, |t| t.enemy_special_weapon_rod_share)` after
    the swarm's; the tuning group; `PROTOCOL_VERSION`.

And, as they are: the **charge-and-hold pattern** - `Trigger::Charge`,
`Charge`, `ChargeRule`, `step_charge`, `charge_trigger`, the hold report,
`TankState::charge`, `ChargeStarted`/`ChargeEnded`, `charge_end_show`, the
HUD's charge gauge in the count's place (`WeaponSlot::charge`), the
wind-up's arrow (`Tank::windup`, `ArrowKind::Windup`), the AI's
`SpecialUse::{Charge, Release}` and the far coast keeping a charging
trigger down; the **dangers** (`Danger`, `DangerShape::Disc`, the `dodge`
tier, `Brain::out_of_danger`, the edge hold); the **knock** and its skid
(`Game::knock`, `Event::Shoved::skid`, `seat_knock`); the hammer's
**frog stun**, **flattened grass** and **fish on the bank**; the swarm's
**air targets** (`Game::strike_air`) and its long-lived press show paired
by payload; `Tank::special`/`active_weapon`/`special_down`/`disable`; the
press show's claim by input tick; `HitCause` on `Event::Hit`;
`spawn_pickup {kind: "rod_from_god"}`; the probe's `--crate rod_from_god`;
`pyro::Shape::Arc`.

### 3.2 What this extends

- **`ChargeRule::stick: Stick`** - `Stick::Drive` (the rail: the stick
  drives the hull, at the crawl) or `Stick::Aim` (the rod: the stick is the
  weapon's). For an `Aim` charge in progress, `drive_player`, the enemy
  collect pass and `Game::predict_seat` hand the tick's `move_dir` (and an
  enemy's `Intent::aim_cell`, below) to the weapon's per-tick step
  (`rod::step_reticle`) and drive the hull with `move_dir: None, face:
  None`. The rod's rule:

  ```rust
  ChargeRule {
      full: t.rod_settle_seconds,
      overcharge: None,
      vent: t.rod_settle_seconds + t.rod_hold_seconds,
      crawl: 0.0,
      vent_cooldown: t.rod_vent_cooldown_seconds,
      stick: Stick::Aim,
  }
  ```

- **`weapons::fire_charge` answers whether it fired**: a release the weapon
  turns down - the rod's reticle on the caller's own cell - is a fizzle
  (`ChargeEnded { Fizzled }`, `charge_end_show`), nothing spent, no
  cooldown.
- **`Tank::reticle: Option<Reticle>`** beside `Tank::charge`: set when a
  rod charge starts, stepped while it runs, cleared with it. The state a
  reticle is (`rod.rs`):

  ```rust
  /// A rod's reticle: the cell it is on, the stick that moves it and when
  /// it next steps, and how long it has stood on its cell.
  pub struct Reticle {
      pub cell: (i32, i32),
      pub held: Option<Dir>,
      /// Seconds to the held stick's next step.
      pub repeat: f32,
      /// Seconds on this cell: what an enemy's rule waits on before it
      /// calls, and the drawing's settle.
      pub rest: f32,
  }
  ```

  `rod::step_reticle(reticle, stick, aim, hull_cell, range, field, dt)` is
  pure: a new stick direction steps at once and sets `repeat` to
  `rod_reticle_delay_seconds`; a held one steps each time `repeat` runs
  out, then every `rod_reticle_repeat_seconds`; with no stick and an `aim`
  cell (an enemy's) it steps toward it at the repeat's pace, along the axis
  with the larger offset first (ties: across); every step is held to the
  range and the field, and resets `rest`.
- **`SpecialUse::Charge { face, aim: Option<(i32, i32)>, why }`** - the
  aim an enemy's reticle steps toward (`Intent::aim_cell`, AI-only, never
  on the wire, like `fire_aim_offset`); `None` for the rail.
- **`SpecialUse::Drop`** - let go of a charge without firing it:
  `act_special` releases the trigger and asks the collect pass to lapse it
  (`Tank::lapse_charge`, `ChargeEnded { Lapsed }`), which a release would
  not do once full. What the rod's rule answers when its target no longer
  holds (§4).
- **A call outranks a special's use**: while a tank's centre stands inside
  a zone's danger, `special_rule`'s first arm (`windup_rule`) answers a
  **charge** in progress `Drop`, every other arm of every weapon answers
  `None`, and the swarm's `air` tier yields the same way - so the `dodge`
  tier takes the tank out (§4, "Reacting"). A tell, half a second long,
  still commits.
- **`Danger::owner: Option<usize>`** (the EMP's is a plain `usize`): `None`
  for a zone's danger, which every enemy keeps out of - the caller too,
  since the rod kills it as surely as anyone.
- **`EngageCtx::herd: Option<f32>`** (§4, "Herding"): the ring built round
  a zone's centre with its firing slots at that distance.
- **`DamageCause::Crush { from }`** - dies outright through `Obstacle`'s
  health, sandbag and fence with no roll (`Fire`'s arm, extended), never
  left burning, rubble thrown away from `from`.
- **`Game::flash_screen_with(strength)`** - the screen flash at
  `strength` times `blast_screen_flash_alpha`; a stronger flash replaces a
  weaker one still on screen and is not held back by
  `blast_screen_flash_min_gap_seconds`. `flash_screen()` is
  `flash_screen_with(1.0)`, as it was.
- **`Game::knock_from(f, center, inner, outer, speed)`** - every live
  hull on the field (`hulls_in_order`) with its box's nearest point more
  than `inner` and at most `outer` from `center`, knocked along the line
  out from it at `speed(mass factor, distance)` px/s (the rod's is
  `rod::shove_speed`, falling off linearly); the hammer's `knock` per
  hull, the footing it stands on taken in. The well's collapse throws with
  it too.
- **`frog_reflexes`** shies from a zone: a frog that can hop and whose
  centre is inside a live zone's danger hops away from the zone's centre
  (straight away from it, or along its facing when it stands on the centre)
  through `combat::frog_hop_target`, before the tank check - a frog under a
  call gets out if it is not penned or stunned.
- **`fish::throw_from(point, reach, max)`** - the hammer's throw onto the
  bank from a point rather than a wave.
- **`AirStrike::Rod`**, **`HitCause::Rod`**.
- **`ArrowKind::Zone`** in the never-dropped set (§5).
- **`Footing::at` takes the craters** (`&Craters`), the pit's pace beside
  the ford's and the lava's.
- **`IntentMsg::reticle`**, `Mailbox::reticle`, `authority::take_reticle`
  (§8), beside the rail's hold report.
- **`PressShow::Rod(RodPress { cell, land_in })`** - a press show that
  lives past its frame and is paired with the room's copy by the claimed
  event's id (the swarm's need 4 and 5).

### 3.3 What this adds: zones

A **zone** is a world-placed area with a lifetime: something a weapon put on
the field that the enemies must keep out of, the router must price, the
seats must be warned of when it is off their screen and the wire must carry
whole. The rod's call is the first; the gravity well (BB-42) is the second
(its orb in flight, then anchored, then collapsing). In `src/zone.rs`:

```rust
/// A world-placed area with a lifetime (`Game::zones`, by id): what the
/// dangers, the router, the off-screen arrows, the minimap and the wire
/// read for anything a weapon leaves standing on the field.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zone {
    /// From the round's projectile counter (`Game::take_shot_id`): its wire
    /// key and its walk order.
    pub id: u32,
    pub kind: ZoneKind,
    /// Whose it is: the kill credit, and who a seat's screen does not warn
    /// of its own.
    pub owner: Owner,
    pub centre: Position,
    /// When it ends, on the round clock (`Game::time`).
    pub until: f32,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ZoneKind {
    /// A rod's call (`rod::RodCall`): lands at `until`.
    Rod(RodCall),
    // Well(WellState) - BB-42.
}

impl Zone {
    /// Seconds left at round time `now`.
    pub fn left(&self, now: f32) -> f32;
    /// The area it acts on (px): its kind's knob - the rod's circle,
    /// `rod_kill_radius_px` - so the wire carries no radius.
    pub fn radius(&self, t: &Tuning) -> f32;
    /// What an enemy keeps out of while it stands: the rod's circle grown
    /// by a hull's half extent and `rod_ai_berth_px` (`danger_radius`),
    /// owned by nobody. `None` for a kind the AI meets some other way.
    pub fn danger(&self, t: &Tuning) -> Option<Danger>;
    /// The radius of the disc whose nav cells the router surcharges while
    /// it stands, and the surcharge: the rod's danger at
    /// `rod_ai_circle_cost`.
    pub fn route(&self, t: &Tuning) -> Option<(f32, u32)>;
}
```

- **The list**: `Game::zones` (a `Vec<Zone>` sorted by id), `zones()` to
  read. Added by the weapon (`fire_rod`), removed when it ends
  (`resolve_zones`). Empty in a round with none, so every reader below
  costs nothing there.
- **The AI**: `zone_dangers` puts every zone's `danger()` on the frame's
  dangers, beside the EMP's discs and the rail's lanes, in id order -
  the `dodge` tier, `out_of_danger` and the edge hold do the rest.
- **The router**: `route_grid_on` surcharges every zone's `route()`
  (`zone_route_cells`: the nav cells whose centre lies inside its disc),
  saturating, before the first field.
- **Off the screen**: `indicators::Scene::zones`; a zone off this screen and
  not owned by a seat on this screen gets `ArrowKind::Zone { kind, left }`,
  never merged and never left out past the cap (§5).
- **The minimap**: a zone is a mark (`Marks::gather`), drawn by its kind.
- **The wire**: the family `Snapshot::zones` (`ZoneState`, §8), keyed by id;
  a replica's zones are the family's (`apply_zones`), so a joiner sees every
  zone standing and a skipped delta loses none.
- **The press show**: a seat's own zone is drawn at once as a provisional
  one (`Game::set_provisional_zones`, an id in the provisional band), and
  the room's copy is kept off the picture until the claim pairs them by
  the `Fired`'s input tick (`claim_own_call`, §8).
- **For the gravity well (BB-42)**: `ZoneKind::Well` with its stage; its
  centre moves while the orb flies (the family's `Moved`); its `danger()`
  is `None` (the AI drives across the pull, not out of it - its own
  reaction) and its `route` its own; the arrow and the minimap mark its
  look. That arm is its PR's.

**The seat motion record** (`Game::seat_still: [rod::SeatStill; MAX_SEATS]`),
which the rod's AI reads (§4) and the well's may:

```rust
/// How a seat has been moving, measured each live tick at the top of
/// `enemy_phase` from its hull's centre. No RNG.
pub struct SeatStill {
    /// Where it last moved off from: reset to its centre whenever the
    /// centre is more than `rod_ai_still_px` from it.
    pub anchor: Position,
    /// Seconds its centre has stayed within `rod_ai_still_px` of `anchor`.
    pub still: f32,
    /// Its velocity (px/s) averaged over about `SEAT_MOTION_SECONDS` (1):
    /// each tick's step over `dt`, eased in by `dt / SEAT_MOTION_SECONDS`.
    pub velocity: Vec2,
    /// Its centre last tick; `None` for a seat not on the field.
    pub last: Option<Position>,
}
```

A wreck, a seat off the field or in a gate lane reads `still` 0 and
`velocity` zero, its record starting again where it comes back. Stepped
every live tick before `enemy_phase` (`Game::tick_seat_still`, a few
additions a seat), read only by the senses that want it, so a round without
the rod decides exactly what it did.

### 3.4 The armory's rod

Into `maps/armory.toml` (docs/sonic-hammer.md §3.5, docs/emp-burst.md §3.4,
docs/gauss-rail.md §3.4, docs/fpv-swarm.md §3.4); nothing placed by weapons
1-4 moves:

| Mark | Cell | Why |
|---|---|---|
| `X` rod crate | 9,14 | The reserved column's fourth cell, four cells south-east of the start |
| `X` rod crate | 34,13 | On the enemy side, so an enemy on shells collects it and calls on a seat that camps |

The rest it needs is there: the start to camp on (the probe's AFK seat camps
there, the issue's standing target), the players' frog (4,14) and towers
(the tesla at 5,6, the gun tower at 7,17) the enemies call on, the enemy
towers (36,5 and 36,16) a seat calls on, brick, wood and glass to break and
iron to stand (22..23 x 5..6, 35..36 x 10..11), the drums and their oil
trail to set off, the grass to flatten, the trees to crush, the lake and its
fish (21..28 x 15..19) to throw onto the bank, the lava ford (34,2..9) a
shove throws a hull into. The crater's filling is seen with `--weather rain` (the
map's sky stays clear, which every earlier weapon was tried under).

No volcano: its 21-cell footprint has no room that keeps weapons 1-4's
cells, and it would erupt on its own every `volcano_period_seconds`,
bombing the start - a different arena from the one the other weapons are
tuned in. The volcano's set-off is tried on `vulkan` (the volcano level)
through the dev server's `spawn_pickup` or `rod_call`, or the builder's
PICKUP tool there and PLAY (§12, decision 18).

```
     0         1         2         3
     0123456789012345678901234567890123456789
 0   ........................................
 1   ........................................
 2   ..................................L.....
 3   ..............ggggg...............L.....
 4   ..............g...g............pp.L.....
 5   ..............g.s.g...II....b.....L.E...
 6   ...+.P...e...*g...g...II....b.....L.....
 7   ..............ggggg.........b.zzz.L.....
 8   .....tp..R.............o..........L.....
 9   ...............................e..L.....
10   ......S..H.........*...f%%%..b.....II...
11   .............................b.....II...
12   .........D.............o...*.d..H.......
13   .............wwwwwT..........g....X.....
14   ....F....X...wwwww..............R.......
15   ............Twwwww...WWWWWWWW.b.........
16   .........r...wwwww...WWWWWWWW.b.....G...
17   .......Q.....wwwww...WWWWWWWW...D.......
18   ...a.................WWWWWWWW....m......
19   .....................WWWWWWWW...........
20   ........................................
21   ........................................
```

(`r` the cell still reserved for weapon 6.) Checked again in Phase 2
against the linter, and the armory's CPU thumbnail pin is re-baselined for
the crates.

**The dev server**: `spawn_pickup {kind: "rod_from_god", x, y}` (the
hammer's tool), `set_tank`'s `rods` (above 0 arms it in place of the special
carried; its `charge` on a tank carrying the rod puts the reticle up where
a press would), and one tool of the rod's own, `rod_call {x, y, enemy}` - a call
at the map cell nearest (`x`, `y`) at once, a seat's (player 1's, its
kills credited to it) or with `enemy: true` an enemy's, its countdown from
now
(`Game::debug_call_rod`, no RNG; a `GAME_ONLY_TOOLS` and
`ONLINE_REFUSED_TOOLS` member, refused off the field; replies `{id, cell,
land}`), so an impact, a crater, a set-off volcano or the pack's herd is
tried in lockstep without steering a reticle. `status`/`snapshot` carry
`zones` (id, kind, centre, seconds left, owner) and `craters` (cells), and
each tank its `rods` and `reticle`. The seats' motion record and an
enemy's pick are not in the snapshot (§12): the probe's trace reads them.

## 4. AI

An enemy carrying calls uses them by `rod_rule`, never through the generic
tiers (`generic_fire(RodFromGod)` is false): attack still lines up and
settles but never presses, `Brain::wants_breach` never latches with it, and
it fires no shells while it carries calls - as with every BB-36 weapon,
until its two are spent. And every enemy, whatever it carries, keeps out of
every call's circle (below, "Reacting to a call").

### What it is handed

`Game::rod_senses` runs once per frame in `enemy_phase`, before the collect
pass, only when some live enemy on the field carries an online rod or holds
a rod charge. It gathers once: every seat on the field (position, hull box,
`SeatStill`, concealed, hit-alerted for each enemy, `Game::sight_on` at it),
the standing player towers (cell), the players' frog (position, alive), the
live enemies (slot, hull box, velocity, whether it hunts the frog), the standing
enemy towers (cell), the enemies' frog (position), the live zones, and the
field. Then, in owner-slot order, each rod tank that thinks this tick gets a
`RodSense`:

```rust
pub struct RodSense {
    /// What this tank would call now, `None` when nothing qualifies.
    pub pick: Option<RodPick>,
    /// Its centre stands inside a live zone's danger: a reticle it holds
    /// is dropped (`windup_rule`) and the dodge takes it out.
    pub under_call: bool,
    /// The nearest seat it knows of from inside that seat's sight box:
    /// what it stands off from (the rule's arm 4).
    pub keep_from: Option<Position>,
    /// No pick, but a seat it would call on were its own hull not in the
    /// circle: what backs it off even from where it holds.
    pub self_blocks: bool,
}

pub struct RodPick {
    /// The cell its reticle goes to and the call lands on.
    pub cell: (i32, i32),
    /// The seat the call is used on - what `Ai::shot_at_seat` records at
    /// the release - for a seat's pick.
    pub at_seat: Option<u8>,
    /// "camper", "lead", "tower" or "frog": the trace's word.
    pub why: &'static str,
}
```

**A target qualifies** - each check against the target as it stands this
tick - in this order of preference:

1. **A camper**: a seat whose `SeatStill::still` is at least
   `rod_ai_still_seconds` (2.0) - stood within `rod_ai_still_px` (12) of one
   spot that long: camping, sniping, holding a rail at full, holding a
   reticle of its own. Its cell is `world_to_cell` of its centre.
2. **A slow seat, led**: a seat that is not still, moving no faster than
   `rod_ai_slow_speed` (48 px/s) and at least `SEAT_LEAD_MIN_SPEED` (4) by
   its averaged velocity - a charging rail's crawl, a tank creeping in a
   ford. Its cell is the one under where that velocity carries it in
   `rod_countdown_seconds` (`centre + velocity * countdown`), held inside
   the field: the call is led, never at where it stands. A seat moving
   faster is never a target.
3. **A player tower**, standing: its own cell (the call crushes it).
4. **The players' frog**, alive: its cell (the frog hops out if it can,
   §3.2; a stunned or penned frog dies).

A hunter (`Role::Hunter` while its quarry lives - `Brain::hunting_frog`'s
rule) prefers its quarry to towers (4 before 3).
For every candidate:

- **The sight box binds a seat's pick**: this tank stands inside that
  seat's sight box (`ai::in_sight_box_of`, against the seat's real
  centre), so the caller is on that seat's screen when it calls and the
  circle - the seat's own spot, or a led spot a few cells off - is too.
  A tower's or the frog's pick needs no seat's box of its own: its
  circle's arrow and minimap mark warn the seats wherever they are (§5).
  But **a call whose circle holds a seat is a call on that seat**: no pick
  - a tower, the frog, a camper with its couch partner beside it - whose
  circle plus the friend margin reaches a live seat's hull while this tank
  stands outside that seat's box, so the box binds every call that can
  crush a seat, not only the ones aimed at one.
- **It knows of it**: a seat not hidden from this tank (`concealed` and not
  hit-alerted - the attack tier's rule) and within its sight under the sky
  (`Game::sight_on` at it); a tower or the frog within `Game::enemy_sight`
  of it.
- **In reach**: the cell inside this tank's reticle range (§1).
- **Never on its own**: no live enemy - this tank included - whose hull box
  reaches within `rod_kill_radius_px + rod_ai_friend_margin_px` (24) of the
  cell's centre, now or where its velocity carries it in a second; no
  standing enemy tower whose cell box reaches inside `rod_break_radius_px`;
  not the enemies' own frog within the circle plus the margin.
- **Not called twice**: no live zone's centre within two kill radii of the
  cell.
- **One caller per target**: a candidate a lower-slot rod tank already took
  this frame is skipped. A tank takes its pick only while it can call on
  it - its reticle up, or its reload (`fire_cooldown`) and fire timer out
  -, so one still waiting never keeps the tanks after it from calling (the
  first armed sweeps' hedge-maze pile-ups: the lowest slot held the frog
  for eight seconds at a time while the pack waited round its pond).

A training dummy (`Ai::frog_only`) gets no pick. No RNG anywhere: fixed
preference, then distance from this tank, then the lower seat, then cell
order.

### The rule (`rod_rule`), in priority order

1. **A charge in progress** (`special_rule`'s first arm, `windup_rule`) is
   answered here and nowhere else:
   1. `under_call` - it stands inside a call's danger: `Drop` (§3.2); the
      dodge tier takes it out.
   2. no pick (the target moved off, an ally walked in, a call already
      covers it): `Drop`.
   3. the reticle not yet on the pick's cell: `Charge { face: the facing
      it has, aim: Some(cell), why }` - the reticle steps toward
      it at a seat's pace, seen by everyone (§5).
   4. on the cell, resting there under `rod_ai_aim_hold_seconds` (0.4):
      `Charge` (it holds).
   5. on the cell and rested: **`Release { face, at_seat, why }`** - the
      call. A led pick's cell moves with its seat; the reticle follows it
      and rests again before the release.
2. **A training dummy**: `None`.
3. **A pick**, not cooling (`fire_cooldown` and `Ai::fire_timer` out):
   `Charge { face, aim: Some(pick.cell), why }` - the reticle starts and
   the fire timer is set to `rod_ai_fire_interval` (8). A release waits for
   the charge to be full (`rod_settle_seconds`) as well as the rest.
4. **The stand-off** (found in Phase 2, below), while it is healthy enough
   not to flee, from the nearest seat it knows of from inside that seat's
   sight box (`RodSense::keep_from`):
   - nearer than `rod_stand_off_px` (the circle, the friend margin, the
     widest hull's half and a cell, 124 px) it backs off to a spot a cell
     past that - on the line out through itself, else on the seat's row or
     column (`Approach`, `why` "stand-off"); with none open it stands where
     it is (`Hold`) - the attack tier would only turn it about. A tank that
     held its spot last tick backs off only from a cell nearer
     (`Ai::rod_held`), so one
     sliding on after it stopped on wet ground is not sent out again -
     unless its own hull is all that keeps it from calling
     (`RodSense::self_blocks`);
   - within `rod_ai_band_px` (64) past it, it holds where it stands,
     keeping the facing it has (`Hold`) - the reticle aims, not the hull -
     unless an ally crowds it (`enemy_separation_px`, hull to hull), when
     it moves to the nearest free spot of the eight round the seat at
     that distance;
   - a spot is one it can drive to straight - along a row then a column,
     or a column then a row, every cell open and out of every danger
     (`way_open`) -, uncrowded, with no tank in its way as it sets off and
     never nearer the seat on the way (`rod_spot_open`). A spot it chose is
     latched (`Ai::rod_spot`) until it gets there, a tank stands in its way
     or that way is shut; a move stopped against a tank or a wall for
     `rod_ai_give_up_seconds` (0.3), or one the router steers away from
     its spot - more than half a cell back, or a cell further than it has
     come (`Ai::rod_spot_best`): the router's way round something, a seat's
     line of fire or a crowd -, is given up, and it stands
     `rod_ai_wait_seconds` (2) where it is before it moves again
     (`Ai::rod_wait`), rather than grind or circle;
   - further out: `None`.
5. Otherwise `None`: the tree goes on (chase, attack's repositioning, patrol
   and the seeks; never a shot).

**Why the stand-off**: a tank carrying calls fires no shells
(`generic_fire` is false), and the attack tier brings it to a firing slot of
the seat's ring - a few cells off, inside the circle a call on the seat
would crush, where the "never on its own" check keeps it from calling. So in
the first sweeps a rod tank parked beside a camping seat and did nothing at
all, and in an armed round a whole pack of them crowded the seat against a
wall (choke: border-stuck 2 against 1, pile-up 5 against 2; archipelago:
a seat that lost in 10 s at the defaults lasting 46). Artillery keeps its
distance: from four to six cells out it is out of its own circle and out of
the way of the tanks that shoot. The band's hold is a deliberate hold for
the probe (`TankSnapshot::rod_hold`, `HOLDS`'s "stand-off"); the crowding
move took pockets' mixed-round pile-ups from 5 to 1, and the latch the
flip-flops it brought from a moving ally.

**The tell**: the reticle stepping from the tank to its target and resting
there (a fraction of a second to about a second, then
`rod_ai_aim_hold_seconds`), the designator line from the module to it, the
module's tracking cells, and off the screen the wind-up's arrow
(`ArrowKind::Windup`, from the charge) - then the call itself: the beam,
the circle and four seconds. A camping seat has about five seconds from the
first sight of the reticle to leave a circle of a cell and a half. The
decision to start and the release are both moments the sight box is
checked; `shot_at_seat` is recorded at the release, the tick the call is
made, which is what the probe's `offbox-fire` reads.

**The hold-still clocks**: a tank holding a reticle commands no movement
(the stuck clock resets, as any deliberate hold), and the C2 commander never
orders it (`UnitView::charging`, the rail's). A charge it lets go of without
calling (`SpecialUse::Drop`: under a call, or no pick left) goes through
`Intent::drop_charge`, AI-only, which the collect pass lapses
(`ChargeEnded { Lapsed }`).

### Herding

After a call, the pack closes on the circle's edge so the seat has to come
out through them. While a seat's centre stands inside a rod zone's danger -
whoever called it - its engagement ring (when it has two or more engaged,
as ever) is built round **the zone's centre** rather than the seat, with `EngageCtx::herd = Some(r)`: the eight firing
slots (rank 0) stand at `r` from the centre on their axes, the lateral
offset as ever, with no `engage_min_radius` clamp (the herd closes in on
purpose) and no sight-box clamp (`r` is inside it); the reserve slots keep
their distance; a slot that cannot stand inside the field is off. `r` is
`rod_ai_herd_px` (128, four cells), and never less than the danger's radius
plus `enemy_danger_clear_px` plus a block, so a herder's slot stands outside
the danger and its latch: no herder jitters on the edge. The slots' line of
sight is checked to the centre.

So the pack takes the four axes round the circle, a few cells out, facing
in: a seat leaving the circle along any axis drives into a firing slot's
line - it has to leave, and it leaves into fire. The impact shoves the
herders a little (they stand inside the shove ring, under half the full
shove at four cells, a slide of under a cell) - the price of herding. The
moment the seat's centre is out of the danger its ring is built round the
seat again and the chase goes on. Eight slots four cells out stand 36 px
apart on an axis and more than 150 px across axes, so a herd is never three
tanks within `CLUSTER_RADIUS_PX`, nor a pile-up. Hunters' frog rings are not herded (the
frog leaves a circle on its own). The herders are the pack's attack tier -
the tanks that shoot; a rod tank with a call left stands off (arm 4) and
takes no slot.

### Reacting to a call (every enemy)

`zone_dangers` makes every live zone's danger a `Disc` round its centre,
radius `rod_kill_radius_px + battlefield::max_tank_clearance_half_extent()
+ rod_ai_berth_px` (24) - measured to a tank's centre, so a hull at the
disc's edge stands clear of the circle -, owned by nobody (`Danger::owner
None`): every enemy keeps out of every call, its own caller's and the
seats', from the call's first tick to its impact:

- **The dodge**: an enemy inside one backs out to `enemy_danger_clear_px`
  beyond it and is latched until it does (the EMP's tier, docs/emp-burst.md
  §3.3).
- **Never paths in**: its chase, attack reposition, alert, seeks, cover
  spot and tree spot aim outside (`out_of_danger`), and the edge hold keeps
  a tank whose next step would enter the disc waiting at its edge until the
  impact (docs/gauss-rail.md §3.2). Out of a call, `Brain::way_out` takes
  the circle's edge nearest the tank itself (the exits from where it
  stands), not the one on the side of the call's middle its target leans:
  a camper stands within a few pixels of the middle of the cell a call on
  it is centred on, and a chaser steered to that side drove round the
  circle and back (`a_chaser_waits_for_a_call_on_its_own_side`: 540
  degrees of turning against at most 270). The EMP's and the rail's
  dangers keep their own exits.
- **The router**: every nav cell whose centre lies inside the disc costs
  `rod_ai_circle_cost` (48) more, saturating, for the countdown
  (`route_grid_on`), so the shared flow fields and the searches go round.
- **A call outranks a special**: a tank holding a charge (a rail or a
  reticle) inside the disc drops it (`windup_rule`, §3.2) and dodges; a
  hammer, EMP or swarm tank inside it does not stand there to shout, pulse,
  launch or break for a drone - its `special` and `air` tiers yield - but
  leaves; a tell, half a second long, commits.

What cannot dodge dies: a disabled enemy (it does not think), one still
skidding from a shove, a far tank coasting on its last intent, a tank whose
roll-in ends in the circle too late to leave it.

### With the commander (`c2_enabled`)

Nothing new: a tank holding a reticle is `charging` (never ordered, keeps
right of way); every other tank's orders come after its own dodge and edge
hold, which `commanded_intent` keeps last for a tank in a tell and the
dodge's steering for the rest. C2 off is untouched.

### What changes for enemies that carry something else

- The dodge, the edge hold and the surcharge for every call, above.
- **The hammer's rule**: a seat whose predicted slide ends inside a live
  call's circle is in trouble (`lands_in_trouble`): a hammer enemy shoves a
  seat into a circle. A hammer brawler's close-in spot is kept out of the
  dangers (`out_of_danger`).
- **The EMP's rule**: a seat carrying online calls is worth
  `emp_ai_special_value`, like any seat with an online special.
- **The rail's and the swarm's rules**: their charge or their watch hold
  yields to a call's danger as every wind-up's does; their cover and tree
  spots are kept out of it.

### Off the field and asleep

A field map's far tank coasting on its last intent has no sense; a charging
trigger stays down on a coast (the rail's rule), so it never calls on a
coast - its reticle vents at worst. A disabled tank's reticle lapsed when it
was struck. A sleeping tank starts nothing.

## 5. Drawing

All of it in the effects language (docs/effects.md): whole 2 px blocks,
ramp steps, Bayer fades; composed at draw time as pure functions of the
zone, the crater, the tank and their age, hashed from ids and cells, never
rolled. The telegraph - reticle, designator, circle, beam, number - is drawn
in the glowing pass, unlit, so it reads at night and in fog (rule 7, and
fair); what lingers on the ground is shaded in the lit pass.

- **Colours.** The call is the designator's red: `pyro::LASER_RED`, the
  laser's off-palette energy ramp (`#6A1410`, `#C8281E`, `#FF3228`,
  `#FFAA96`, white). A seat's reticle is drawn in its team colour
  (`tank::team_color`, so a couch tells its two apart), an enemy's in
  `LASER_RED[2]`. Dust, smoke, char and debris in `DUST`, `SMOKE`, `CHAR`.
  A filled crater in the palette's blues (`BLUE_DEEP`, `BLUE_DK`,
  `BLUE_PALE`).
- **New primitive, `pyro::digits(b, n, at, scale, color, shadow)`**: a
  whole number in a 3 x 5 block font, each font pixel `scale` blocks (2:
  12 x 20 px a digit), a shadow block down and right in `shadow` - the one
  piece of text the field draws in the effects language, so the countdown
  needs no text key and draws on any canvas.
- **The reticle** (`rod::compose_reticle(reticle, owner_colour, t)`),
  glowing pass, for every tank holding one: eight arcs of a circle round the
  cell's centre (`pyro::Shape::Arc`, one block wide, each an eighth of a
  turn less a gap), turning at 1.5 rad/s; their radius
  `rod_kill_radius_px` plus 8 px while the reticle moves, closing to the
  kill radius over its first 0.3 s at rest (the Armory scene's settle); a
  cross of four two-block arms at the centre. On the caller's own cell the
  arcs are drawn at `cover` 0.5 and the cross is an X: the cancel.
- **The designator** (`rod::compose_designator`), glowing pass: a dotted
  line of single blocks, one every 6 px, from the module's lens
  (`tank_art::ROD_LENS`, in the turret's frame) to the reticle's centre, in
  the reticle's colour at `cover` 0.5 - what a seat sees pointing at it
  while an enemy aims.
- **The call** (`rod::compose_call(zone, left, view_top, t)`), glowing pass,
  for every rod zone:
  - *the circle*: the same eight arcs at `rod_kill_radius_px`, in
    `LASER_RED[2]`, turning; over the last second the gaps close and the
    circle beats at 4 Hz (`cover` 1 and 0.7);
  - *the beam*: one block wide, from the circle's centre straight up to the
    top of the view (`view_top`, the camera's world rect - every screen
    draws its own beam to its own top), `LASER_RED[2]` with a
    `LASER_RED[3]` block every 8 px running down it; for the first three
    seconds it flickers - drawn on two of every three frames of
    `ROD_BEAM_HZ` (20), phased by the zone's id -, then steady; over the
    last second two blocks wide with a white core;
  - *the foot*: a stepped glow (`pyro::glow`) of `LASER_RED[2]`, 16 px,
    pulsing with the circle;
  - *the countdown*: the whole seconds left (`ceil`, never under 1), in
    `pyro::digits` at scale 2, `LASER_RED[3]` with a `SMOKE[0]` shadow,
    beside the circle's upper right (38 px right, 32 px up of the centre,
    the scene's place), white in the last second. Online every zone's
    seconds are this client's present's (§8).
- **The call's first frames** (`rod::compose_call_ring`, from the zone's
  age): a ring of `LASER_RED[3]` blocks closing from 32 px onto the circle
  over 0.15 s, and the module's uplink cell (`Tank::kick_rod`) - the
  room's for its tank, a replica's off the `Fired`, a client's own on its
  release (`flash_seat_rod`).
- **The impact** (`rod_show`, staged through `Game::show` like every
  blast's):
  - *the column* (`rod::compose_column`), glowing pass, for
    `rod_column_seconds` (0.05, three frames): six blocks wide of white from
    the top of the view down to the centre, a `LASER_RED[3]` block either
    side - the one bright instant;
  - *the screen flash*: `flash_screen_with(rod_screen_flash)` (2.5 times a
    drum's), never held back by the drum's gap (§3.2);
  - *the ripple and shake*: `Shockwave::scaled(c, rod_shock)` (1.5 of a tank
    dying) through `shockwave.rs`; none under reduced motion, and all of it
    scaled by `screen_fx_intensity`;
  - *the impact flash quad* (`impact_flashes`) and the fireball:
    `BlastFx::shaped(c, BlastKind::Fuel, BlastShape::Fire)` (the column
    form) at `rod_fireball_scale` (1.3) - the heat of the strike; a rod
    carries no explosive, so it is short beside the dust. Not out of water,
    whose spray is its show (below): a fireball there hid the spray and
    its dust ring read as earth on the lake;
  - *the dust ring* (`rod::compose_impact`), lit pass: two rings racing out
    over 0.9 s - a two-block one in `DUST[4]` at 230 px/s and a one-block
    one in white at 180 px/s, squashed to 0.7 vertically (the ground's
    tilt), dissolving through the Bayer pattern - and ten shaded dust puffs
    (`pyro::dust_puff`) running out from 30 px to 150 px and growing,
    leaning with the wind (`pyro::smoke_lean`), gone in `rod_dust_seconds`
    (1.6). The dust is the ground's: `DUST` on dry ground, `SMOKE` steps
    over lava, `BLUE_PALE` spray and white chop over water, white powder
    (`SMOKE[6]`, white) over snow and ice;
  - *the debris*: twelve `SMOKE[1]` and `DUST[0]` blocks thrown out on
    hashed bearings to between half and one and a half times 140 px, each
    on an arc (`60 k - 120 k^2` px of height at age `k`), gone where they
    land;
  - *the rubble ring*: `rod_rubble_pieces` (14) pieces of rubble thrown as
    decals (`Decal::thrown`, the walls sheet's brick and sandbag rows,
    alternating) from the centre to hashed spots between the kill and break
    radii, landing in `debris_flight_seconds` and staying (under
    `mark_caps`); none on water;
  - *the scorch*: `Scorch::with(c, rod_scorch_scale, None)` (2.5) on dry
    ground;
  - *the splash* on water: a column of spray blocks, white with one in three
    `BLUE_PALE`, rising `SPLASH_PX` (40) and falling back over
    `SPLASH_SECONDS` (0.6), over the dust ring's two rings of chop in
    `BLUE_PALE` and white, in place of the scorch and the rubble.
- **The crater** (`rod::draw_crater(c, crater, look, t)`, generic over
  `canvas::Canvas`, so the CPU thumbnail draws it too), at the head of
  `paint_floor_marks` after the floor shade and before the tracks and
  scorches - it is the ground's shape, and tracks pressed into it show:
  - *the pit*: discs of blocks round the struck cell's centre - `SMOKE[1]`
    to 34 px, `SMOKE[0]` to 26 px, and a `CHAR[0]` core 18 px offset 4 px
    down and right (its shadow side) - each block's edge pushed in or out a
    block by a hash of its cell, so the rim is ragged; the rim a ring of
    `DUST[0]` and `DUST[1]` blocks at 36 px (thrown earth, whatever the
    theme: no green);
  - *smoking*: for `rod_crater_smoke_seconds` (6) after the impact, thin
    `SMOKE[3]`/`SMOKE[4]` puffs rise from the pit, one every 0.3 s, leaning
    with the wind and thinning out over the last third
    (`rod::compose_crater_smoke`, lit pass, from the crater's round time,
    so a replica and a late joiner draw the same age);
  - *filled*: inside 30 px, water - `BLUE_DEEP` blocks, `BLUE_DK` on the
    lit upper-left rim, a few `BLUE_PALE` glints hashed and stepped on
    `water_frame_seconds` -, rising from the centre over `ROD_FILL_SECONDS`
    (1) after the impact (drawn only; the rules made it a ford at once);
    in rain, every 0.4 s a hashed one-block ring of `BLUE_PALE` opening from
    2 to 6 px; under ice (filled, then frozen) `SMOKE[6]` and white with
    `BLUE_PALE` cracks.
- **The light** (`weather::lights_in`): a call throws an unshadowed red
  point light over its circle (1.5 kill radii, `rod_beam_light`, 0.7 of it
  until its last second), so the circle reads at night; a reticle a faint
  one on its cell; an impact floods the ground white out to 1.5 shove radii
  for `rod_flash_seconds`, fading; the fireball throws its blast's light.
- **The module** (`tankdesign`, `lines/vanguard.py`, `module_fn('rod')`): an
  uplink on the roof - the missiles' hardpoint, shared, since a tank carries
  one special at a time (`hp.get('rod', hp['missiles'])`): a squat gunmetal
  base (5 x 4, chamfered) carrying a steel phased-array panel (3 x 3, a
  vertical grille) and, forward of it on a one-pixel steel housing, the
  designator's lens (dark, lit by state). Five cells, `TANK_MODULE_ROD_COL`
  = 44..48 (`tank_modules.png` grows from 44 to 49 columns, 1960 x 480;
  the 44 before it unchanged, byte for byte): 0 armed, the lens dim
  (`DIM_LENS`, `RED_DK`); 1 and 2 tracking a reticle, the lens lit red
  (`'warn'`) and the array's left and then its right column lit red; 3 a
  call, the array and the lens white; 4 offline, the array scorched (`RUST`)
  and the lens dark. `module_cols` (eleven entries): 3 while
  `Tank::rod_flash` (`rod_flash_seconds`, set by `kick_rod` on every call -
  the room's, a replica's `Fired`, a client's release), 4 while
  `special_down()`, 1 and 2 alternating at 6 Hz while a reticle is held,
  else 0. Anchor: `tank_art::ROD_LENS` (the lens, turret frame), written by
  `export.py` from the module's `meta['lens']`, where the designator's
  line starts (`rod::lens`). `render.SHOWN_TOGETHER` leaves it out with the
  other roof modules. `just check-sheets` passes.
- **The crate**: row 17 of `gen_crates.py`'s sheets (`crates_sheet.png` 280
  x 720, `pickup_glyphs.png` 24 x 432; the seventeen rows before it
  unchanged, byte for byte). Its symbol, 10 x 10 design px: a tungsten rod
  falling point first into a reticle's four corner brackets - the brackets
  a player steers - its hot tip and the brackets (`o`) in the ink's light:

  ```
  '....XX....',
  '...XXXX...',
  '...XXXX...',
  '...XXXX...',
  '...XXXX...',
  'oo.XXXX.oo',
  'o..XXXX..o',
  '....oo....',
  'o...oo...o',
  'oo......oo',
  ```

  Ink (`punypalette.PICKUP_INK['rod_from_god']`, admitted on the crate
  sheets alone like the others): **two-tone, tungsten and red** - shade
  `#4E545C`, base `#8A9099` (a dark steel grey), light `#FF3228`
  (`pyro::LASER_RED[2]`, the designator's red); `LAMP_TONE` like the
  swarm's, so a glint whitens the rod and leaves the brackets, and the
  glyph's rod is lit along its top in white (`BODY_LIGHT`) rather than in
  the brackets' red. Ultramarine was turned down (it sits beside the EMP's
  cobalt). Candidates were rendered beside all seventeen crates on grass
  and measured (CIELAB, base and light against every crate's base and
  light, and against the grass): coral `#FF9A8A` is 11.5 from the health
  cross's light and reads as a pink health crate; salmon 8.3; amber 21.8
  from the heat shield; teal 34 but beside the plasma and the sonic hammer
  and only 39 from the grass; steel blue 31.5 from the hammer; bronze 23.4
  from the flamethrower. The dark tungsten is 25 from the minigun's pale
  grey-blue, the nearest single tone, and the pair - a grey body with red
  corners - is a pattern no other crate has: the swarm is ivory with
  crimson dots, the heat shield red over black, the health cross red
  alone. No green anywhere.
- **The HUD**: `hud::WeaponSlot::of` gives the calls in `HUD_ROD_COLOR`
  (`#FF3228`, the designator's red - the reticle's colour on the field, the
  crate's light) and the glyph; while a reticle is held the rail's charge
  gauge stands in the count's place (filling over the settle, white once a
  release calls); offline, the EMP's `WPN OFFLINE`. The ring's ammo pips are
  the calls left against `full_load`, the carry limit (6).
- **The call-in prompt** (`PlayChrome::prompt`, `hud::rod_prompt`): while
  this window's seat - a couch's first that holds one - holds a reticle,
  the first line under the left cluster, at `HUD_STATUS_TEXT_SIZE`, in
  `HUD_ROD_COLOR`: `hud-rod-aim` while the reticle is anywhere else,
  `hud-rod-cancel` while it is on the seat's own cell (§7).
  `CornerShape::lines` counts it, so the lines' box and the keep-outs read
  one rect; `every_language_fits_every_budget` holds both lines inside the
  vitals block's plate less 8.
- **Off the screen** (`indicators.rs`): `ArrowKind::Zone { left }` for
  every rod's call whose circle is off this screen and not this seat's own
  (`Scene::zones`, from the zones, so a replica draws them, the countdown
  read on the zones' clock): a notched arrowhead in `LASER_RED[2]`, rimmed
  near-black, blinking at `indicator_gate_blink_hz` and twice that in the
  last second; never merged, never left out past `indicator_max_arrows`
  (the tells', teammates', frogs', volcanoes' and drones' rule). An enemy
  holding a reticle off the screen gets the wind-up's arrow
  (`ArrowKind::Windup`, from the charge) in `HUD_ROD_COLOR`.
- **The minimap** (`minimap.rs`): a crater cell is `Class::Crater` (RUST_DK)
  while dry and the ford's colour once filled; `RoundMinimap::sync` bakes
  the image again when the count of craters or of filled cells changes. A
  call is a mark: a frame of `LASER_RED[2]` round its cell, blinking at
  `indicator_pulse_hz` and twice that in its last second.
- **No dev overlay** of its own: the zones are in `snapshot` (below), and
  the dangers in the engage overlay's report already.

## 6. Tuning

New group `rod` (every row live unless marked), plus one row in the enemies'
group:

| Row | Default | Range | Doc |
|---|---|---|---|
| `rod_per_pickup: i32` | 2 | 1..=10 | Calls one rod crate loads. One per call. |
| `rod_max: i32` | 6 | 1..=20 | The most calls a tank carries, crates stacked (BB-66). |
| `rod_settle_seconds` | 0.25 | 0..=2 | How long the trigger is held before a release calls; a shorter tap calls nothing. |
| `rod_hold_seconds` | 10 | 1..=60 | How long past that a reticle may be held before the uplink times out, calling nothing. |
| `rod_reload_seconds` | 1.0 | 0..=10 | After a call, seconds before the next reticle. |
| `rod_vent_cooldown_seconds` | 0.5 | 0..=10 | After the uplink times out, seconds before a reticle may start again. |
| `rod_reticle_start_cells: i32` | 4 | 0..=20 | Where a reticle appears: this many cells ahead of the hull along its facing, held in range. |
| `rod_reticle_delay_seconds` | 0.18 | 0..=1 | A held stick steps the reticle at once, then again after this. |
| `rod_reticle_repeat_seconds` | 0.06 | 0.01..=1 | Then one cell every this long. An enemy's reticle steps at this pace too. |
| `rod_countdown_seconds` | 4.0 | 0.5..=15 | From the call to the impact. |
| `rod_kill_radius_px` | 48 | 8..=160 | The circle: a hull with any part inside it is crushed, a frog killed, a drone downed. |
| `rod_break_radius_px` | 80 | 0..=256 | Every breakable tile whose cell reaches inside this goes down; iron, a cone and a door stand. |
| `rod_shove_radius_px` | 160 | 0..=480 | How far out the impact shoves hulls, falling from the circle's edge to nothing here. |
| `rod_shove_speed` | 420 | 0..=508 | The shove at the circle's edge (px/s) against the chassis-free mass. |
| `rod_shove_max_speed` | 480 | 0..=508 | The most any shove gives; the wire's shove reaches 508. |
| `rod_mass_exponent` | 1.0 | 0..=4 | How hard a heavy chassis resists the shove. |
| `rod_frog_stun_seconds` | 1.0 | 0..=10 | A frog in the shove ring neither hops nor bites this long. |
| `rod_grass_flat_seconds` | 8 | 0..=60 | Tall grass within the break radius hides nobody this long. |
| `rod_fish_reach_px` | 160 | 0..=480 | Fish within this of an impact are thrown onto the bank... |
| `rod_fish_throw_max: i32` | 8 | 0..=32 | ...at most this many, nearest first. |
| `rod_crater_reach: i32` | 1 | 0..=3 | A crater's cells: those within this many steps of the struck cell (1, a plus of five). |
| `rod_crater_pace` | 0.6 | 0.1..=1 | A dry crater's share of a hull's top speed and push. |
| `rod_crater_path_cost: i32` | 2 | 1..=20 | What a dry crater cell costs the router a step (a ford's `water_ford_path_cost`). |
| `rod_column_seconds` | 0.05 | 0.01..=0.5 | The white column. |
| `rod_screen_flash` | 2.5 | 0..=8 | The screen flash against a drum's. |
| `rod_shock` | 1.5 | 0..=3 | The ripple and shake against a tank dying's. |
| `rod_fireball_scale` | 1.3 | 0.1..=3 | The impact's fireball against a drum's. |
| `rod_scorch_scale` | 2.5 | 0.5..=5 | Its scorch. |
| `rod_rubble_pieces: i32` | 14 | 0..=40 | Rubble thrown into a ring round the crater. |
| `rod_dust_seconds` | 1.6 | 0.1..=5 | The dust ring's puffs. |
| `rod_crater_smoke_seconds` | 6 | 0..=30 | How long a fresh crater smokes. |
| `rod_beam_light` | 0.5 | 0..=2 | The light a call's foot throws at night, against a headlight's. |
| `rod_flash_seconds` | 0.3 | 0..=2 | The module's uplink cell. |
| `rod_ai_still_seconds` | 2.0 | 0..=20 | A seat that has stood still this long is an enemy's target. |
| `rod_ai_still_px` | 12 | 0..=64 | How far a seat's centre may wander and still be standing still. |
| `rod_ai_slow_speed` | 48 | 0..=400 | A seat moving no faster than this (px/s, over the last second) is called on where it will be. |
| `rod_ai_fire_interval` | 8 | 0.1..=60 | Seconds between an enemy's decisions to call. |
| `rod_ai_aim_hold_seconds` | 0.4 | 0..=3 | How long an enemy's reticle rests on its cell before it calls. |
| `rod_ai_friend_margin_px` | 24 | 0..=128 | An ally whose hull is within the circle plus this holds the call. |
| `rod_ai_berth_px` | 24 | 0..=128 | How far past the circle (and a hull's half) an enemy keeps from a call. |
| `rod_ai_circle_cost: usize` | 48 | 0..=255 | Extra route cost on every cell of a call's danger; 0 switches it off. |
| `rod_ai_herd_px` | 128 | 0..=400 | How far from a call's centre the pack's firing slots stand while the seat they fight is inside it. |
| `rod_ai_band_px` | 64 | 0..=256 | How far past its stand-off distance an enemy carrying calls holds where it stands. |
| `rod_ai_give_up_seconds` | 0.3 | 0.05..=5 | A stand-off move stopped against a tank or a wall this long is given up... |
| `rod_ai_wait_seconds` | 2.0 | 0..=10 | ...and the tank stands this long before it moves again. |
| `enemy_special_weapon_rod_share` (`enemies`, `@ Restart`) | 0 | 0..=1 | The share of special-carrying enemies that spawn with the rod from god instead, decided by a hash of the spawn point and the slot - never the round's RNG - so at 0 nothing changes. |

Constants (geometry and policy, not feel): in `rod.rs` `ROD_BEAM_HZ` (20),
`ROD_FILL_SECONDS` (1), the reticle's eight arcs and 1.5 rad/s, the dust
rings' 230 and 180 px/s, the splash's 40 px and 0.6 s (`SPLASH_PX`,
`SPLASH_SECONDS`), the countdown's place (38, -32); in `zone.rs` the
provisional id band; `SEAT_MOTION_SECONDS` (1) and `SEAT_LEAD_MIN_SPEED` (4)
beside `SeatStill`.

## 7. Text

| Key | en | sl |
|---|---|---|
| `hud-rod-aim` (`keys::HUD_ROD_AIM`) | AIM, THEN LET GO TO CALL | NAMERI, NATO SPUSTI ZA UDAR |
| `hud-rod-cancel` (`keys::HUD_ROD_CANCEL`) | LET GO HERE TO CANCEL | SPUSTI TUKAJ ZA PREKLIC |
| `tool-rod_from_god` | rod from god | božja palica |
| `tool-short-rod_from_god` | rod | palica |

`LET GO` serves both a key and a thumb, so the prompt needs no `-touch`
twin; `STEER`/`AIM` names no key either. `božja palica` ("God's rod") folds
to `bozja palica`. **The budget**: `every_language_fits_every_budget` gains
both prompt lines at `UI_SMALL_TEXT` within the block's width less 8
(`Corners::prompt`, the one-seat block on the smallest area); the tool names
are measured by the existing tool budgets (144 and 48). Measured in Phase 2;
`NAMERI IN SPUSTI` ("aim and let go") stands by for Slovene if the aim line
runs over, and `udar` ("strike") for the short name. The countdown is
digits, drawn as blocks: no key.

## 8. Wire

Protocol 20 (from the swarm's 19).

- `WeaponKind::RodFromGod`, appended to `ALL`; `drawn_on_press` true (its
  show is the call, drawn on the release).
- **`TankState::reticle: u16`** - 0 for none, else the reticle's cell as
  `encode::cell_index` plus one (`encode::reticle_code`,
  `reticle_from_code`; a 250 x 250 map fits). A replica draws an enemy's
  reticle and a teammate's from it, keeping the rest it counted while the
  cell stays.
- **`IntentMsg::reticle: u16`** - the client's own reticle in the same
  encoding (`IntentMsg::with_reticle`), on every packet while its rod's
  charge runs, the release's included: the cell the sandbox stepped the
  release on (`Predictor::reticle_report`). The reticle report (below).
- **`Snapshot::zones`**, keyed by id:

  ```rust
  pub struct ZoneState {
      pub id: u16,          // the zone's id, low sixteen bits
      pub kind: u8,         // `zone::ZONE_ROD` for a call
      pub x: i16,           // its centre, quarter pixels
      pub y: i16,
      pub until: u32,       // when it ends, round-clock ticks
      pub owner: u16,       // its owner's slot
      pub cell: u16,        // the struck cell, `cell_index`
  }
  ```

  The radius is the kind's knob, not the wire's; a call never changes, so
  `delta.rs` sends it whole once and its key once more when it goes
  (`zones`, `zones_gone`).
- **`Snapshot::craters`**: `CraterState { cell: u16, tick: u32 }` (the
  struck cell and the impact's tick, for its smoke's age). A replica works
  the crater's cells out from its own map (`Game::crater_cells_at`) and
  fills them by the sky it shares with the room (`Game::make_crater`).
  `delta.rs`: `craters`, `craters_gone`.
- **`Snapshot::volcano_shifts: Vec<i32>`** - each volcano's shift in ticks
  (`Game::volcanoes` order), empty while every one is 0, sent whole when it
  changes (the delta's `Option`). It is beside `RoundState`, not in it,
  which keeps `RoundState` `Copy`.
- **Events**: `WireEvent::RodCalled { id: u16, slot: u16, seat: u8, col:
  u8, row: u8, land: u32 }` (`seat` `NO_SEAT` for an enemy's, `land` in
  round-clock ticks) and `WireEvent::RodImpact { id: u16, col: u8, row:
  u8, crater: bool, erupted: bool }` - a map's side is at most 250 cells, so
  a column and a row are a byte each. `HitCause::Rod` and `AirStrike::Rod`,
  appended. The rail's `ChargeEnded` carries a rod's cancels and time-outs.
- **What a replica draws**: zones from the family (`apply_zones`: the
  room's replaced whole, a client's own provisional ones kept), craters from
  theirs (`apply_craters`), the volcanoes' shifts (`apply_volcano_shifts`:
  `Game::shift_volcano`, which marks a set-off's rumble shown), the
  reticles from `TankState::reticle`; on `RodImpact`, `rod_show` (what it
  struck read before the crater is made, as the room reads it) and, in
  `fx`, the fish thrown (`Shoal::throw_from`). The ring closing on a call is
  drawn from the zone's own age (`compose_call_ring`), so it needs no event
  of its own. The kills, shoves, tile deaths, blasts and downed drones come
  from their own events, as from every blast. The picture the round-trip
  tests hold equal (`DrawableState`) carries the room's zones, the crater
  cells, the shifts and every tank's reticle.
- **What is drawn at once** (decision 3 of BB-36):
  - *the reticle, from the press*: the sandbox's seat runs the charge and
    the reticle in `Game::predict_seat_with` (the stick handed to the
    reticle, `Stick::Aim`), and `OnlineRound::write_predicted` writes the
    sandbox's charge and reticle into the shown seat every frame
    (`Game::show_seat_reticle`) - so the reticle, the designator, the
    module's tracking cells, the HUD gauge and the prompt start on the press
    frame and move with the stick at once.
  - *the call, on the release*: `Predictor::charge_edge`, now by weapon
    (`SeatCharge::weapon`, and `SeatCharge::hull_cell`, the cell the hull
    stood on as the trigger was stepped): on a rod's `Released` with the
    reticle off the hull's own cell it sets the gate to
    `rod_reload_seconds`, owes the call and, while presses are drawn, queues
    `PressShow::Rod(RodPress { cell, tick })` and the drawn press
    `(RodFromGod, the release's input tick)`. `round.rs` keeps it as an
    `OwnCall` and puts it on the picture as a provisional zone
    (`Game::set_provisional_zones`, id past `PROVISIONAL_ZONE_BASE`) with
    the module's uplink cell (`Game::flash_seat_rod`) - the beam, the circle
    and the count on the release frame. A release on its own cell is the
    cancel: no show, nothing owed.
  - *the countdown, on this client's present*: `Game::zone_lead` (set each
    frame from `incoming_lead_ticks`, 0 in a local round) is added to the
    picture's clock wherever a zone's seconds are read - the count, the
    beam's last second, the arrow, the minimap, the light - so every call
    counts down to the moment the room will judge this client's hull, and
    the shooter's own counts exactly four seconds from its release. The
    count never reads under 1: for the lead's fraction of a second at the
    end the circle holds, beating, until the room's impact is handed over.
- **What is claimed**: the room's `RodCalled` for this seat, through
  `Show::OwnShotsDrawn` (`presses_drawn`), claims the call drawn by the
  release whose input tick the seat's `Fired` before it names
  (`claim_own_call`, the swarm's `claim_own_drone` rule: the last waiting at
  or before that tick; any drawn before it still waiting was refused and
  goes). A claimed call's provisional zone goes and the room's zone - same
  cell, same landing - is the call from then on. A drawn call nobody claims
  within the refusal wait goes, and the owed call is given back.
- **The reticle report**: the room cannot step a client-owned seat's
  reticle the way the client did (its mailbox merges intents), so it takes
  the client's cell: `Mailbox::reticle()` is the newest `reticle` any intent
  the last read applied carried - a merged read keeps a release's cell
  though the packet after it carries none, and while the round still has
  the trigger down a read with none keeps the last (a press and its release
  merged into one read deliver the release on the next) -, a starved read
  repeating it; and
  `net::authority::take_reticle(game, seat, code)` hands it to the round
  before the tick (`Game::set_seat_reticle`, for that update alone) - in the
  room server's tick beside `take_hold`, and in the rig's. A report puts
  the reticle on its cell from the press on, held to the reticle's range of
  the room's hull (`rod::step_reticle`), and a reported cell is never
  stepped by the stick. A local round has no report. What the report cannot
  stretch: the range is the room's hull's sight box, so a modified client
  calls nowhere a seat could not; the settle is the room's count of held
  ticks (the rail's hold report, within its slack); only the walk's pace is
  the client's word - a reported cell is taken at once, not walked to.
- **What stays the room's**: the call itself (where and when it lands), the
  crush and its kills (`Hit`, `Wreck`), the shoves (`Shoved` to an owned
  hull), tile deaths (`ObstacleDestroyed`), drums (`Blast`), shields
  (`ShieldBroken`), drones (`DroneDowned`), frogs (`FrogState`), craters
  and the volcanoes' shifts.
- Sizes (`delta::tests::sizes_of_the_prd_snapshot`): the full snapshot 498
  B (from 489), the idle delta 57 B (from 54); a moving and a busy delta
  unchanged in their bounds.

## 9. Determinism

- **The rod draws no RNG of its own.** The crush is whole damage, the shove
  a computed knock, a tile's crush a death with no fence, sandbag or
  pass-over roll, a frog's death and stun timers, a drone's downing a
  strike, the crater and the volcano's shift arithmetic, the fish a hash.
  Its walk is fixed: zones in id order; within an impact the seats in
  index order then the enemies by slot, then the frogs (the players', the
  enemies'), air targets by key, grenades by id, lanterns by id, tiles by
  cell, crates by cell, oil cells sorted, crater cells sorted. What it sets
  off draws as it always does: a drum's blast its damage rolls and an oil
  pool's chance where it goes off, a grenade's burst its rolls, a frog's
  hop out of a circle its jitter - only where a call stands over a frog.
- **The reticle** steps on ticks from the intents (or the reported cell);
  the charge is ticks; the hold report a clamp.
- **The volcano's cycle stays a pure function of the round clock**: the
  shift is a whole number of ticks, worked out once at the impact
  (`set_off_shift`: the eruption it leads up to begins on the next tick)
  and kept in the round state, so the room, a replica and a late joiner
  run the same cycle from then on.
- **The AI**: `rod_rule` chooses by fixed preference; `rod_senses` walks
  rod tanks in slot order and candidates by preference, distance, seat and
  cell; `zone_dangers` is built in id order; the herd is ring geometry; the
  seat motion record is arithmetic on positions.
- **The swap** is the hammer's hash; the new entry runs only with its share
  above 0.
- **A round without the rod replays byte for byte**: no crate kind is
  rolled anywhere, every share defaults to 0, no enemy carries one, so no
  reticle is stepped, no zone stands (no danger, no surcharge, no herd, no
  arrow, no shy hop), no crater is made (`Footing::at` finds none, the nav
  weighs none), no volcano is shifted, `flash_screen` is
  `flash_screen_with(1.0)`. The seat motion record is measured in every
  round but read by nothing else. `determinism_tests`' pinned streams, the
  probe fixtures' ceilings and every thumbnail pin but the armory's stay as
  they are.

## 10. Tests

As built. `simulation/rod_tests.rs` (headless, the default field with the
seat at cell (3, 6), enemies placed by hand):

- `a_rod_crate_arms_two_calls_and_replaces_the_special` - two calls,
  another special emptied, a charge trigger whose stick aims.
- `the_reticle_starts_ahead_and_the_stick_steps_it_while_the_hull_stands` -
  the cell four ahead, a tap a cell, a hold one at once and one after the
  delay, the hull standing, the reticle gone with the charge.
- `the_reticle_stays_inside_the_sight_box_and_the_field`.
- `a_release_calls_the_rod_and_it_lands_after_the_countdown` - `Fired` and
  `RodCalled`, a call spent, nothing before the countdown is out; then a
  shielded hull in the circle crushed, one in the ring knocked, one past it
  untouched, the zone gone.
- `a_release_on_its_own_cell_or_unsettled_spends_nothing` - the cancel and
  a tap: `Fizzled`, nothing spent.
- `the_circle_crushes_every_hull_with_any_part_inside_it` - a hull with a
  corner in, the caller in its own circle; one a few pixels out only
  shoved.
- `a_shove_stops_a_hull_at_a_wall_and_the_fields_edge_never_inside` - a
  seat thrown at the field's top edge and at an iron wall, from every
  phase of a step's travel, never more than a quarter pixel into either,
  every tick of the skid and after.
- `the_impact_breaks_the_tiles_in_reach_and_iron_stands` - brick, a
  sandbag, a drum going off; iron and a cell past the break radius
  standing.
- `a_crater_slows_a_hull_and_fills_under_rain` - the plus of cells, the
  pit's pace, a sky turned to rain making it a ford.
- `no_crater_in_water`.
- `a_frog_under_a_call_hops_out_and_a_stunned_one_dies`.
- `a_rod_on_a_volcano_sets_it_off` - `RodImpact { erupted }`, the eruption
  on the next tick, no crater on the cone.
- `a_rod_on_vulkans_crater_sets_it_off` - the same on the shipped
  `vulkan` level, from asleep, the eruption its usual length.
- `a_drone_over_the_circle_is_downed` - `DroneDowned { by: "rod" }`.
- `several_calls_land_in_id_order`.
- `a_call_lands_harmlessly_on_the_end_screen` - the show, no crush, no
  tile, no crater.
- `the_rod_draws_no_rng` - a call, its shove and a crushed tile leave the
  round's stream where the round without them leaves it.
- `a_call_replays_byte_for_byte`.
- `the_router_prices_a_crater_and_the_kept_grid_follows` - a route
  through the crater costs more, and the kept nav grid is the one built
  from scratch (`Grid::same_as`).
- `a_call_is_a_danger_for_everyone_and_its_flash_is_the_strongest`.
- `a_rooms_report_puts_the_reticle_where_the_client_has_it` - from the
  press, held to the range, the release calling there.
- `the_spawn_swap_hands_out_the_rod_by_its_share`,
  `an_enemy_takes_the_crate_only_with_no_special`.
- AI on whole rounds: `an_enemy_calls_a_rod_on_a_seat_standing_still`
  (once, on the seat's cell, from inside its sight box, the seat crushed),
  `an_enemy_too_near_its_target_backs_off_and_calls`,
  `an_enemy_never_calls_on_a_circle_holding_an_ally`,
  `an_enemy_leaves_a_call_before_it_lands`.
- `the_seat_still_record_counts_still_and_averages_speed`.
- Added in review: `a_burning_plank_in_reach_is_crushed`,
  `a_range_board_in_reach_is_crushed_never_lit`,
  `the_impact_sets_off_grenades_breaks_lanterns_lights_oil_and_flattens_grass`,
  `a_frog_in_the_ring_is_stunned`,
  `a_charge_held_inside_a_call_is_dropped_and_its_tank_leaves` (a wind-up
  yields to a call), `a_hammer_enemy_inside_a_call_leaves_rather_than_shout`
  (a special yields to the dodge), `a_hammer_enemy_shoves_a_seat_into_a_call`
  (`lands_in_trouble`),
  `an_enemy_never_calls_on_a_tower_whose_circle_holds_a_seat_off_its_box`,
  `a_reloading_rod_tank_leaves_its_target_to_the_next`,
  `a_rod_hunter_calls_on_its_quarry_from_outside_the_circle`,
  `a_chaser_waits_for_a_call_on_its_own_side`,
  `set_tank_charge_puts_a_rods_reticle_up`; `engage::tests`
  (`a_herd_puts_the_firing_slots_at_its_distance_round_the_call`).

Headless halves: `rod::tests` (`the_range_is_the_sight_box_less_half_a_cell_inside_the_field`,
`reticle_start_is_ahead_and_in_range`, `the_stick_steps_once_then_repeats`,
`an_aim_walks_the_reticle_larger_offset_first`,
`a_report_puts_it_on_the_reported_cell_held_in_range`,
`crater_cells_are_a_plus_of_dry_cells_in_the_field`,
`the_shove_falls_off_to_nothing_at_its_radius`,
`cell_reach_is_the_distance_to_the_cells_box`,
`the_reticle_is_on_the_grid_and_pure`,
`the_call_draws_its_countdown_and_its_beam_holds_in_the_last_second`,
`the_column_lasts_its_frames_and_the_impact_is_gone_by_its_end`,
`a_crater_draws_the_same_on_any_canvas_and_a_filled_one_draws_water`,
`a_strike_in_water_throws_up_spray_that_falls_back`),
`zone::tests` (`a_rod_zones_danger_covers_its_circle_and_a_hull_beside_it`,
`left_counts_down_on_the_round_clock`), `volcano::tests`
(`set_off_shift_starts_the_eruption_on_the_next_tick_and_leaves_an_eruption_alone`),
`ground::tests` (`fill_makes_dry_cells_fords_and_freeze_ices_them`),
`fish::tests` (`a_rods_impact_throws_the_fish_by_the_shore_onto_the_bank`),
`indicators` (`a_call_off_the_screen_has_an_arrow_whatever_the_cap`), the
minimap's picture test with a call's mark, and the text budgets.

Online: `net::apply` (`a_rods_call_reticle_impact_and_crater_reach_the_replica`,
`a_volcanos_shift_reaches_the_replica`,
`a_rods_calls_and_craters_reach_the_replica` - the round trip frame by
frame), `net::predict` (`a_rods_reticle_and_call_are_drawn_on_the_clients_ticks`
- the report, the call on the release, the claim, the cancel),
`net::round` (`a_call_is_drawn_on_the_release_and_handed_to_the_rooms_zone`),
`net::rig` (`an_enemys_call_reaches_the_replica`), `net::mailbox`
(`the_reticle_report_is_the_newest_intents` - a merged read, a release
merged with the packet after it, a press and its release in one read), the
codec's intent bytes and
the delta's sizes.

## 11. Probe

**How it is read.** The probe's tank line gains `rod=` (calls left; a held
reticle shows as the rail's `chg=`); its fire tuple counts the calls, so a
call is a trigger pull for `FIRED_RECENTLY_FRAMES`. A tank holding a reticle
(`TankSnapshot::charging`), one held at a danger's edge
(`TankSnapshot::edge_hold`) and one holding its stand-off
(`TankSnapshot::rod_hold`, `HOLDS`' "stand-off") are deliberate holds, not a
stall, a stale start, low progress or jitter. **`offbox-fire`** reads
`shot_at_seat` at the release, and the fire line gains
`rod-calls-on-seats`/`rod-calls-offbox`: every enemy `RodCalled` with a
seat pick whose caller stood outside that seat's box that tick is an
`offbox-fire` anomaly. A call on a tower or the frog is not a shot at a
seat. **A call is engagement** for `never-arrived`, as a drone launched at
a seat or the frog is: a rod tank calls from its stand-off, a hunter from
round its quarry, not necessarily from inside `enemy_attack_range` of the
seat.

**The runs**, all at seed 1000, ten rounds a map: the fixtures (`maps/test/*.toml`,
1800 frames) and the seven field maps of `just probe-fields` (3600), with the
recipes' budgets. `armed.json` is `{"enemy_special_weapon_chance": 1.0,
"enemy_special_weapon_rod_share": 1.0}`; the mix is chance 0.5 and share
0.5; night, rain and the commander add `weather_override` 1 or 3 or
`c2_enabled` to the armed patch.

Defaults: every map's output byte for byte the swarm's reviewed head's
(`2c67491`) - the fixtures, the fields, the 30-round default sweep,
waves-basic, the advance scenario on the maze and hedge-maze, two seats on
the default map and archipelago, and two seven-minute defend rounds on
longwater - but for the two new zero counters on the fire line. Totals with
the rod (minutes of round in brackets), as built and after the review's
fixes (§12, decisions 30 and 31; the calls are calls on seats):

| Run | Fixtures, as built | Fixtures, reviewed | Fields, as built | Fields, reviewed |
|---|---|---|---|---|
| Defaults | border-stuck 4, jitter 32, spin 3, churn 34, clustering 10, pile-up 6 (11.1) | the same | border-stuck 11, jitter 108, spin 24, churn 83, clustering 12, wall-grind 1, pile-up 8 (18.9) | the same |
| `--crate rod_from_god` | as the defaults (no weapon slot) | the same | border-stuck 12, jitter 117, spin 21, churn 90, clustering 16, pile-up 7 (19.6); 20 calls | border-stuck 12, jitter 116, spin 20, churn 86, clustering 16, pile-up 7 (19.0); 24 calls |
| Mix: chance 0.5, share 0.5 | border-stuck 2, jitter 27, spin 6, churn 40, clustering 11, pile-up 2 (11.9) | border-stuck 2, jitter 26, spin 4, churn 40, clustering 7, pile-up 1 (11.5) | border-stuck 10, jitter 94, spin 14, churn 67, clustering 4, wall-grind 3 (16.9) | border-stuck 10, jitter 99, spin 14, churn 61, clustering 3, wall-grind 3 (16.6) |
| Every enemy armed, by day | jitter 18, spin 1, churn 31, clustering 11, pile-up 4 (12.9); 83 calls | jitter 17, spin 1, churn 30, clustering 3, pile-up 1 (11.2); 83 calls | stall 1, border-stuck 11, jitter 126, spin 21, churn 97, clustering 30, pile-up 22 (20.1); 62 calls | stall 2, border-stuck 11, jitter 107, spin 18, churn 79, clustering 19, never-arrived 1, pile-up 11 (18.2); 67 calls |
| Every enemy armed, night | jitter 12, spin 2, churn 36, clustering 9, pile-up 3 (13.4) | jitter 13, spin 2, churn 37, clustering 1, pile-up 3 (11.8) | stall 1, border-stuck 9, jitter 112, spin 32, churn 109, clustering 30, tank-grind 2, pile-up 22 (24.3) | stall 2, border-stuck 11, jitter 108, spin 32, churn 102, clustering 29, wall-grind 1, low-progress 1, never-arrived 3, tank-grind 2, pile-up 24 (21.7) |
| Every enemy armed, rain | jitter 21, spin 3, churn 36, clustering 11, pile-up 3 (12.5) | jitter 21, spin 2, churn 36, clustering 6, pile-up 6 (11.5) | border-stuck 3, jitter 138, spin 28, churn 91, clustering 14, pile-up 8 (19.7) | border-stuck 4, jitter 117, spin 27, churn 89, clustering 10, pile-up 10 (18.3) |
| Every enemy armed, commander on | jitter 19, spin 2, churn 30, clustering 7, pile-up 3 (12.3) | jitter 17, spin 1, churn 29, clustering 4, pile-up 3 (11.0) | border-stuck 11, jitter 109, spin 16, churn 87, clustering 24, pile-up 14 (19.2) | border-stuck 11, jitter 108, spin 23, churn 73, clustering 28, pile-up 17 (18.3) |
| Night alone (no rod) | border-stuck 2, jitter 34, spin 4, churn 38, clustering 17, tank-grind 1, pile-up 7 (11.6) | - | border-stuck 14, jitter 101, spin 19, churn 93, clustering 25, low-progress 1, tank-grind 2, pile-up 17 (22.3) | - |
| Rain alone | border-stuck 2, jitter 47, spin 4, churn 60, clustering 10, pile-up 7 (11.1) | - | border-stuck 9, jitter 121, spin 15, churn 76, clustering 7, low-progress 1, tank-grind 2, pile-up 6 (19.0) | - |
| Commander alone | border-stuck 3, jitter 33, spin 3, churn 35, clustering 12, pile-up 4 (11.1) | - | border-stuck 12, jitter 117, spin 21, churn 89, clustering 25, pile-up 12 (18.7) | - |
| Yardstick: the shells pack, `--mission destroy`, `player_armor_factor` 0.1 | spin 22, clustering 95, pile-up 65, tank-grind 12, never-arrived 7, low-progress 8 (44.1) | - | spin 44, clustering 109, pile-up 74, tank-grind 18, low-progress 8, never-arrived 1, stall 1 (66.6) | - |

`offbox-fire` 0 in every run: no call on a seat from off its box (150
calls on seats armed by day alone, reviewed; 145 as built), and no shot, missile, drone
lock or hit on a seat from off its box. The armour does not lengthen a rod
pack's rounds - the circle crushes whatever the armour - so the long-round
yardstick and the rod pack's own run are of about the same length as the
defaults'.

**What the review moved.** The armed sweeps' worst - hedge-maze by day,
clustering 18 and pile-up 15 against ceilings of 11 and 8 - was the pack
waiting round the frog's pond: one caller per target let the lowest slot
take the frog while its eight-second fire timer ran, so nobody else
called (`a_reloading_rod_tank_leaves_its_target_to_the_next`); with a
target taken only by a tank that can call, hedge-maze armed reads
clustering 5 and pile-up 5, the fixtures' armed clustering 11 to 3 and
pile-ups 4 to 1, the fields' clustering 30 to 19 and pile-ups 22 to 11.
The calls come sooner, so rounds go another way from the first call on:
the commander's hedge-maze went from clustering 8 and pile-up 5 to 20 and
11 (its harbor-lights from 14 and 8 to 7 and 5), and the night's hedge-maze
and harbor-lights each run a round to the frame cap - an AFK seat in a
corner the pack does not see at night, the towers grinding it down - in
which a tank at 18 points fleeing (low-progress, wall-grind, a stall) and
three that never reached the seat are flagged. Every remaining
`never-arrived` is in a round that hit the 60 s cap with the seat still
unfound; before the probe counted a call as engagement there were four
times as many (hunters calling on the frog from round its pond).

**Over a ceiling, reviewed** (as built in brackets): armed by day,
hedge-maze jitter 45 and churn 46 against 30 and 43 (52, 53; clustering 18
and pile-up 15 now under) and one never-arrived; at night, hedge-maze
jitter 60, spin 14, churn 68, clustering 15, pile-up 14 (61, 16, 69, 15,
15) over 6.7 minutes - 40 s rounds against the night alone's 14 s, the
spins 2.1 a minute as the night alone's - and harbor-lights tank-grind 2
(2; the night alone 2) and never-arrived 3 (0); in rain, hedge-maze jitter
44, spin 18, churn 51 (51, 18, 47), and two fixtures by one each, props'
pile-up 3 of 2 and towers' jitter 7 of 6 (none); with the commander,
hedge-maze jitter 48, churn 47, clustering 20, pile-up 11 (47, 44, under,
under; harbor-lights' clustering 14 now 7, under); the mix, harbor-lights'
wall-grind 2 of 1 (2; pockets' jitter 7 and spin 2 now under); the crate,
castle-moat's border-stuck 9 of 8 (9, shell tanks chasing past the crates
by the border) and hedge-maze's jitter 31 of 30 (under).

**Read round by round** (as built). Every spin, stall, tank-grind,
wall-grind, never-arrived and low-progress anomaly of the rod runs (156)
was replayed with a scratch trace that lists the AI tiers its tank ran
over the two seconds before it (not in the tree). 138 have no rod state in
their window (patrol, chase, attack, the seeks, flee); 7 are the crate
run's shell tanks seeking the rod's crate (`seek_special`); 11 hold a
reticle's aim and spin in the chase that follows. The review traced that
chase: `out_of_danger` took a disc's exit along the line from its middle
through the target, and a camper stands a few pixels off the middle of the
cell a call on it is centred on, so a chaser was sent round the circle to
the side the seat happened to lean and back (540 degrees of turning in
`a_chaser_waits_for_a_call_on_its_own_side`, against at most 270 now: it
waits at the edge on its own side). The stand-off itself was built against
these readings (decision 26): parked inside its own circle (archipelago, a
seat that lost in 10 s at the defaults lasting 46), a pack crowding a
camper against a wall (choke: border-stuck 2, pile-up 5), flip-flopping
between two spots (the latch), grinding in a corridor (the give-up and
wait), sliding on in the wet and sent out again (the hold's hysteresis),
circling round a seat to its far side (the straight drive on its own
side), routed the long way round a block or a seat's line of fire (giving
up a move steered away from its spot), and turned about by the attack tier
with nowhere to back off to (the hold).

**Not run**: `just probe-defend` (a release build; no rod crate stands on
the maps it plays, and the defend scenario's two seven-minute longwater
rounds in the defaults comparison above match byte for byte).

## 12. Interactions, decisions, what is left out

### Interactions with what ships

| With | What happens |
|---|---|
| Rainbow shield | Popped and the hull crushed inside the circle; outside, the shove lands as ever |
| Heat shield, speed boost, ooze coat | Nothing: a crushed hull is crushed; a boosted or coated hull leaves the circle at its own pace |
| Portals | The rod lands where it was called; an anchor in the circle stands; a shoved hull goes through a portal and stops skidding |
| Water | A ford in the circle is no refuge; a lake takes no crater and no scorch; the fish are thrown onto the bank; a crater beside a lake is a pit (or a puddle in rain) |
| Ice (snow) | Not broken; white powder; no crater on it; shoves slide further on it |
| Rain, storm | A crater is a ford from the impact on; shoves slide further on wet ground |
| Sandstorm gusts | The dust leans with the wind; a skid runs in the gust's frame |
| Night, storm, fog | The reticle, the circle, the beam and the number are drawn unlit and the foot throws red light: the telegraph reads in the dark; the AI calls only on a seat within its sight under the sky |
| Lamp posts, lanterns | A post in the break radius falls; a lantern is broken |
| Towers | Either side's in the break radius are crushed (ruin, discharge, cook-off or spill); enemies call on player towers; a seat calls on enemy towers |
| Frogs | Either side's killed in the circle, stunned in the ring; a frog hops out of a circle if it can; the players' frog is an enemy's target |
| Crates | Left lying in the crater unless `crate_breakable` |
| Drums, oil, fires | A drum in the break radius goes off at once and chains; a fused one is left to its fuse; a trail in reach is lit |
| Trees, grass | Trees crushed, not set alight; grass flat and hiding nobody for a while |
| Glass, walls, sandbags, fences | Crushed; iron, a door and a cone stand |
| Range boards (docs/range-target-prd.md) | Crushed in the break radius like any breakable tile: down at once as charred rubble if it was burning, as splinters if not, never set alight by the strike |
| Lava, the volcano | No crater in lava; a cone struck sets its volcano off; a shove can throw a hull into a ford |
| Shells, bullets, plasma, the laser, missiles, flying drums, lava bombs, globs | Untouched |
| Grenades | Set off in the break radius |
| Wrecks | Untouched |
| Gates, waves | A tank rolling in is off the field; arriving, it keeps out of the circle |
| Field maps | The sight box binds every call on a seat; a far coasting tank never calls; a call's arrow and minimap mark reach a seat wherever it is; a wrecked seat's return lane is not special |
| The couch and its split | Each seat's reticle in its colour; the prompt is the first seat's that holds one (one line under the left cluster); a teammate in the circle is crushed; the beam is drawn in both halves' worlds, each to its own top |
| Training | `drop = ["rod_from_god"]` works by its name; a dummy never calls; a door stands |
| The C2 commander | A tank holding a reticle is never ordered |
| Online | §8 |

### Interactions with the sonic hammer (built here)

| Hammer | Rod |
|---|---|
| The knock and its skid | The rod's shove is the hammer's knock, from the impact out (`knock_from`): one skid model, one validator allowance |
| A hammer's shove into a call's circle | A hammer enemy shoves a seat into a live call (`lands_in_trouble`); a seat can shove enemies in |
| A frog stunned by a hammer under a call | Cannot hop out: it dies - the frog-pin combo |
| A shove on a tank holding a reticle | The charge holds; the reticle is pulled back into range |
| A hammer tell inside a call's circle | The tell commits (half a second); then the dodge |
| Flattened grass | The impact flattens grass by the hammer's `grass_flat` |
| The fish on the bank | The impact throws fish by the hammer's flop |
| The online claim | One pending claim per kind: a seat's blast and call claim their own events |

### Interactions with the EMP (built here)

| EMP | Rod |
|---|---|
| The ring reaches a tank holding a reticle | The charge lapses (`disable`); no reticle while the special is offline |
| The ring after a call | The call lands: the satellite is not in reach |
| A disabled enemy under a call | Coasts on, does not dodge, dies: the EMP-then-rod combo |
| An EMP enemy's value of a seat with calls | `emp_ai_special_value` |
| The dangers and the dodge | One tier, one latch: the EMP's disc, the rail's lane and the call's disc |
| The predictor's `offline_left` | A press during it is a shell, not a reticle |
| `WPN OFFLINE` | The rod's slot reads it while offline |

### Interactions with the gauss rail (built here)

| Rail | Rod |
|---|---|
| The charge-and-hold machine | The rod's reticle is a charge with its own rule (`Stick::Aim`, crawl 0) |
| A charging rail seat | Holding still at full: a camper; crawling: led |
| A rail tank charging inside a call | Drops its charge and leaves (a wind-up yields to a call) |
| A slug through a tank holding a reticle | Damage; the charge holds |
| The edge hold | Keeps tanks out of a call as out of a lane |
| The gauge | The rod's settle in the count's place |

### Interactions with the FPV swarm (built here)

| Swarm | Rod |
|---|---|
| A drone over the circle | Downed (`AirStrike::Rod`), any side |
| A halo on a crushed tank | Falls with the wreck (the swarm's wreck rule) |
| A drone's burst on a tank holding a reticle | Damage; the charge holds |
| Cover and tree spots | Kept out of a call's danger |
| A swarm tank watching its drone inside a call | The dodge comes before the watch |
| The online claim | The swarm's payload claim pairs the rod's provisional zone too |

### Decisions taken

1. **The stick steers the reticle, cell by cell, while the trigger is held;
   the hull stands.** "A reticle that follows the aim" read as the stick
   being the aim while the trigger is down - any cell on screen is
   reachable in about a second, and standing still to aim is the cost.
   Rejected: running the reticle out along the gun line (the hull's four
   facings would reach four lines only), a free analog cursor (the game's
   sticks are four-way), an automatic lock on the nearest enemy (the seat
   would not choose). *For Oto.*
2. **The call is on a cell.** The issue's "cell" taken literally: the
   circle, the crater, the AI's picks and the wire all agree on the grid.
3. **A tap calls nothing, letting go on your own tank cancels, and the
   uplink times out after ten seconds.** A press has to be meant; the way
   out of a call is to steer it home. Rejected: a tap calling at the start
   cell (an accidental strike four cells ahead), a cancel by timeout alone
   (ten seconds standing still). *For Oto.*
4. **The range is the caller's sight box**: the circle is always on the
   caller's screen when it calls, the same box the AI's fairness rule uses.
5. **A call once made always lands** - the caller wrecked, disabled,
   teleported: it is the satellite's.
6. **The circle is blind and goes through a rainbow shield**: anything
   with a part inside it is crushed, a teammate and the caller too - the
   counter is the telegraph, four seconds and a red beam. Rejected: sparing
   one's own side (the hammer's and the slug's side rules), a shield
   soaking it (a shield would make the rod pointless against the seat that
   most needs it). *For Oto.*
7. **No damage outside the circle; the ring is a shove.** "Leave the
   circle" then means what it says: the circle drawn is the whole of the
   danger, and the shove can still throw a hull into lava, a tower's reach or
   another call. Rejected: falloff damage out to five cells (a seat that
   left the circle would still be punished, and the drawing would lie).
   *For Oto.*
8. **Every breakable tile within two and a half cells is crushed outright -
   towers too - and nothing is left burning**; iron, cones and doors stand.
   "Breaks every breakable tile nearby" read as gone, not cracked.
9. **Frogs die in the circle and are stunned in the ring, and a frog hops
   out of a circle on its own** (its shy hop, the evasive hop's sibling).
   Without it an enemy's call on the frog would be a lost Protect round with
   nothing a seat could do; with it, a call on the frog drives it out of
   its spot, and a penned or stunned frog dies - the hammer's frog pin and
   a call is the combo. *For Oto.*
10. **Drones over the circle are downed** - the one ground strike that
    reaches the air, because the rod comes down through it. The swarm's
    rule that blasts below a drone do not touch it stands for every other
    blast. *For Oto.*
11. **A rod on a volcano sets it off now** by shifting its cycle a whole
    number of ticks, kept in the round state: the cycle stays a pure
    function of the round clock and of state the wire carries, and the
    next eruption comes a period later. An erupting volcano is left alone;
    a rumble is brought forward. Rejected: an eruption outside the cycle
    (a second clock to keep, and a regular eruption could follow straight
    after), a set-off decided again on each replica from the impact's tick
    (the room's clock and a replica's differ by float accumulation, so the
    shift could differ by a tick).
12. **The crater is built** - a pit that slows and is priced, a ford while
    it rains - through the systems that already carry such ground: the
    footing, the router's weights, the water layout's depth (a filled
    crater is a ford by every water rule at once), the minimap's classes and
    a family on the wire. It fills at the impact under a wet sky rather
    than over time, so the rules never wait on a timer the predictor would
    have to run; the water's rise is drawn. It never dries during a round,
    freezes with the rest under snow, and is never made in water, lava or a
    cone. Rejected: making crater cells ground-tile water (the autotiles
    would draw a cross-shaped stream), and evaporation (a timer for a rare
    dev-tool case).
13. **Zones** are the shared piece: the call is a world-placed area with an
    end, and so is the well; the dangers, the router, the arrows, the
    minimap and the wire read zones, not rods.
14. **Every countdown is drawn on this client's present**: the number is
    the time the seat has to get its hull out by the room's own judgement.
    The impact itself stays the room's.
15. **The AI calls only on standing things** - a camper, a slow seat led,
    a player tower, the frog -, one caller per target, never on a circle
    holding an ally, with its reticle walking visibly to the target and
    resting before the call; and it **herds** by moving its ring round the
    circle. Rejected: calling on any seat in sight (a moving seat would
    always escape and the call would be wasted noise).
16. **A wind-up yields to a call**: a tank holding a rail or a reticle
    inside a call's danger drops it and leaves; a tell commits.
17. **One trigger**: a seat carrying calls fires no shells until it has
    called them, as with every special; calling both takes half a second
    if the cannon is wanted back.
18. **No volcano in the armory**: no room for its footprint without moving
    weapons 1-4's cells, and it would erupt on its own; the set-off is
    tried on `vulkan`. *For Oto.*
19. **Crate ink two-tone, tungsten and red** (`#4E545C`/`#8A9099` with
    `#FF3228`): a grey rod with red brackets, a pattern no other crate has.
    Ultramarine was turned down beside the EMP's cobalt, coral read as a
    pink health crate (11.5 from its cross's light), teal sat by the plasma
    and the hammer and near the grass - the distances in §5. *For Oto*,
    with the crate beside the other seventeen.
20. **The crate spills** rather than cooks off: an uplink and a rod, no
    explosive.
21. **The prompt is a HUD line under the seat's block**, in UI points, so
    it is measured by the budgets in both languages; the countdown is
    block digits in the world.
22. **The kill credit is the caller's**, a dead caller's too.
23. **Calls and craters travel as families**, with events for their shows:
    a joiner and a client that skipped a delta still see every circle and
    crater (the swarm's drones set the pattern); the issue's "the call as a
    `WireEvent` (cell, land tick)" is `RodCalled`, which carries both.
24. **The room calls on the client's cell** (the reticle report): an owned
    seat's mailbox merges sticks, so only the client knows where its
    reticle stood when it let go; the report is held to the range.
25. **The screen flash takes a strength**, and the rod's is not held back
    by a drum's that just went off: an orbital strike always flashes.
26. **An enemy carrying calls stands off** (§4, arm 4) rather than taking
    a firing slot: with no shells to fire, a slot only parked it inside the
    circle a call on its target would crush, so it never called (the first
    sweeps: a pack crowding a camper against a wall, pile-ups and grinds).
    It holds four to six cells out keeping its facing, moves round that
    band only by a straight drive on its own side of the seat, gives up a blocked move after a third of a second (and one
    the router takes the long way round) and waits two, and backs off
    from a cell nearer once it holds. Each part answered an anomaly
    read round by round (§11). Rejected: the attack tier's ring with a
    larger radius (the ring's slots are firing lines, not stand-off spots,
    and its clamp to the sight box pulls them back in), and closing in to
    the band from further out on its own rather than through the attack
    tier (tried: the fields' clustering went from 27 to 38 and pile-ups
    from 19 to 26 armed, and three fixtures went over their ceilings - a
    rod tank driving straight for its band met the shell tanks on their
    way to theirs).
27. **The client's call is claimed by its `Fired`'s input tick** (the
    swarm's rule), not by the `RodCalled`'s id: the provisional zone's id
    is the client's own, and two releases in flight are told apart only by
    the ticks they were made on.
28. **A reticle the AI no longer wants is dropped** (`Intent::drop_charge`,
    `ChargeEnded { Lapsed }`), never released on its own cell: a release
    there would fizzle by the cancel rule, which is the seat's gesture and
    reads in the trace as a call that failed.
29. **No dev overlay of its own**: the circle, the countdown, the arrow and
    the minimap mark already show every zone; the dev server's snapshot
    carries the zones and craters. A seat's motion record and an enemy's
    pick are read through the probe's trace rather than the snapshot - no
    `AiSnapshot` field for a rule that changes as it is tuned.
30. **The sight box binds every call that can crush a seat** (review): a
    call on a tower or the frog whose circle holds a seat is a call on that
    seat, so it needs that seat's box, like a call aimed at the seat.
    Rejected: leaving tower and frog calls unbound (an enemy off a seat's
    screen could crush the seat defending its frog, warned only by the
    arrow). *For Oto.*
31. **Out of a call, a tank waits on its own side** (review): `way_out`
    takes the circle's edge nearest the tank for a danger nobody owns. The
    EMP's disc round a seat and the rail's lane keep their exits, so the
    earlier weapons play as they did.
32. **The crater's rules are its cells, its picture a disc** (as built):
    the pit slows, and fills, by the plus of five cells, while it is drawn
    as a ragged disc of about 36 px - so a hull a cell off on an axis is
    slowed on the drawn rim's dust, and one on a diagonal inside the drawn
    pit is not. Left for Oto: draw the plus, or measure the rules by the
    disc. *For Oto.*
33. **A range board is crushed like a plank** (docs/range-target-prd.md):
    it is a breakable wooden prop, so the break radius takes it whole - a
    cold board splinters, a burning one goes down charred - and the strike
    never lights one (a crush is not fire). *Alternative*: letting the
    strike set boards alight, which would make the rod the one shot that
    lights a board; only fire does.

34. **A shove stops at a wall's face** (QA): the shove ring threw a hull
    at the field's top edge 5.5 px into the boundary in one step, and it
    sat part-way off the field for two seconds. Rapier is set up in metres
    while the world is in pixels, so its contacts look 0.02 px ahead and it
    pushes a body out of an overlap at 3 px/s: a hull crossing more than
    that in a step lands inside what it meets, the solver stops it there,
    the skid ends on the stop, and nothing pushes it out faster. A
    skidding hull now looks a step's travel ahead (rapier's soft
    continuous collision detection, `sonic::skid_look_ahead`), at the
    shared knock so the hammer, the rail's recoil and every later knock
    stop the same way, in the room and in a client's sandbox alike
    (`a_shove_stops_a_hull_at_a_wall_and_the_fields_edge_never_inside`).
    Rejected: rapier's own CCD (it engages only for a body crossing half
    its thinnest half extent in a step, about 6 px for a standard hull -
    a knock's whole travel at rapier's 400 px/s cap - so it misses most
    knocks); clamping the
    knock's travel against the static boxes by hand (a second collision
    model beside rapier's, for tiles, deep water, lava and the frogs);
    a faster push out (`IntegrationParameters` are the whole world's, so
    every round would step differently); a lower speed cap (the shove's
    reach is a tuning decision). Ram and blast knockback, which can throw
    a hull as fast, are left as they are so a round with no knock replays
    byte for byte (a follow-up). BB-59 took the follow-up: every hull now
    looks a step ahead from spawn (`physics::HULL_LOOK_AHEAD`), and rapier's
    speed cap is one cell a step, so the shove reaches its tuned 420/480
    px/s instead of 400 (docs/physics-engine-design.md "Scale").

### Not in this PR

- Placing the rod in the shipped levels - level design, a follow-up.
- A volcano in the armory (decision 18).
- Breaking or melting ice; a rod into lava splashing a pool of burning
  lava; a lake hit throwing water onto the banks beyond the fish.
- The AI calling on a volcano to set it off on purpose (its bombs aim at
  seats) - a tactic of its own.
- Puddles drying when the rain stops - only the dev server's `weather`
  changes a sky mid-round.
- A crater as a map cell the builder can paint, and craters in thumbnails
  of a map as authored.
- An impact drawn provisionally in a client's present - the room's impact
  is drawn when its snapshot is handed over.
- A rod knocking missiles, flying drums, lava bombs or shells out of the
  air.
- The gravity well's zone arm (BB-42).
- Sound effects - the game has no audio yet.
- A probe scenario in which the seat fires its special.

### Needs from the shared path

What this design takes from weapons 1-4's implementations beyond what their
docs give:

1. **`ChargeRule::stick`** (`Drive` for the rail, `Aim` for the rod), with
   `drive_player`, the enemy collect pass and `Game::predict_seat` handing an
   `Aim` charge the tick's `move_dir` (and an enemy's `Intent::aim_cell`)
   and driving the hull with no stick.
2. **`weapons::fire_charge` answering whether it fired**, so a release a
   weapon turns down is a fizzle.
3. **`SpecialUse::Charge::aim` and `SpecialUse::Drop`**, `act_special`
   lapsing a charge on `Drop`; **`windup_rule`** able to answer `Drop` for a
   charge inside a zone's danger.
4. **`Danger::owner` able to say nobody** (`Option<usize>`).
5. **`EngageCtx` open to a herd** (a centre other than the target and a
   firing distance with no min-radius clamp).
6. **The rail's hold report beside a reticle report**: `IntentMsg` open to
   one more field, `Mailbox` exposing the newest applied intent's, an
   `authority::take_*` sibling, and the rig and room calling it.
7. **`PressShow` living past its frame** and **a claim by the `Fired`'s
   input tick** (the swarm's `claim_own_drone`): the room's `RodCalled` for
   this seat claims the drawn call whose release is the last waiting at or
   before the input tick the `Fired` before it names (`claim_own_call`).
8. **`Game::knock` callable for a shove out from a point** (`knock_from`),
   the echo to an owned hull with its skid and `seat_knock`'s allowance.
9. **The hammer's `Frog::stun`, `Game::grass_flat` (per-cell seconds) and
   `fish::Flop`** reachable from a point, not only from a wave.
10. **`frog_reflexes` open to a second threat** (a zone's centre) beside
    the nearest tank.
11. **`Game::strike_air` and `AirStrike`** open to `Rod`.
12. **`ArrowKind`'s never-dropped set** (the wind-up's, the drones') open to
    `Zone`.
13. **`HitCause`** open to `Rod` and its `fx` arm (flash only).
14. **`DamageCause` open** to `Crush`, with the fence's and sandbag's no-roll
    arm and no burning.
15. **`flash_screen` with a strength.**
16. **`WeaponSlot::charge`** drawn for any `Charge` weapon, not only the
    rail.
17. **The probe's holds and off-box readings** (the charge, the edge hold,
    the swarm's lock count) open to the rod's call count.
18. **`SPAWN_SWAPS` and `SEEK_SPECIALS`** as tables.
19. **The armory**: the reserved cell 9,14 free, and 34,13 left free by
    weapons 1-4 (cells of the 40 x 22 armory).
