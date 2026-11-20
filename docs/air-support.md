# Air support

BB-119, BB-177. A supply crate shows a helicopter or a jet. A seat that picks
it up calls air support: a count of 15 s runs in a small box under the HUD's
ammo slot, then the aircraft flies in - an Apache-like gunship or an
A-10-like attack jet, drawn ten per cent bigger than a tank. The helicopter
is a companion: it comes to the tank that called it, stays with it as it
drives and fires on the enemies round it until its rockets are gone. The jet
flies like a plane: it runs in on a target, follows it if it moves, comes
back for another pass, and with nothing to attack circles the tank once and
leaves. The minimap shows the aircraft and an arrow points to it when it is
off the screen. The design questions were settled with Oto on 2026-10-12 and
2026-11-20 (§9).

It is built from what the special weapons laid down. The rod from god
(docs/rod-from-god.md) gave the pattern of a call on the round clock carried
whole on the wire, the FPV swarm (docs/fpv-swarm.md) flying hecs entities
with a ground point and a height, a shadow and the air-target list, the
missiles `Game::blast` - the side-aware area hit - and their seeker volleys,
and the tower pack a crate that is not a tank weapon. New here: the call
that belongs to a seat, the two aircraft and their flights, the HUD's box
and the sprites.

## 1. How it plays

- **The crate.** Two kinds, `PickupKind::AirHeli` and `PickupKind::AirJet`
  (`pickup = "air_heli"` / `"air_jet"` in a map, the builder's PICKUP
  tools of the same names). Each shows its aircraft from above, so a player
  sees which one is coming. Only seats take them (`Tank::wants_pickup` is
  `false` for an enemy, as for the flamethrower, the heat shield and the
  grenades). Neither is a weapon (`PickupKind::weapon` is `None`): the
  special the seat carries stays. They come only from the map's slots - no
  random bonus roll places one.
- **The call.** Taking the crate calls the sortie (`Event::AirSupportCalled`).
  The seat's HUD shows the crate's symbol and the whole seconds before it
  arrives, from `air_support_delay_seconds` (15). One sortie of a seat is
  counting or in the air at a time: a crate taken meanwhile queues another,
  shown as `+1` beside the count, whose count starts when the first
  aircraft is gone - off the field or shot down. A helicopter with rockets
  left stays, so a call queued behind it waits as long as it does.
- **The arrival.** At zero the aircraft comes in from outside the field,
  `air_edge_margin_px` (96) past an edge. Which edge is a hash of the call's
  id (`air_support::entry_edge`): not random, and no RNG. The helicopter
  comes in in line with its seat. The jet's hash picks only the axis; it
  comes in from the side farther from its first target - from its seat with
  none - (`jet_edge`), so it has at least half the field to line up in.
- **What it may attack.** Read every step, not once: inside the seat's sight
  box (`ai::in_sight_box`, the screen the follow camera shows round the
  seat), the enemy tanks live on the field and the enemy towers still
  standing - tesla, gun tower, bio slush (`Game::air_marks`, an
  `AirView::marks`). Nearest the seat first; at the same distance a tank
  before a tower, tanks by slot, towers by cell. Each carries a key
  (`MarkKey`) that is the same from step to step, its position and its
  velocity. The players' own towers are never targets. A wrecked seat's
  call still comes, round its wreck; a seat gone from the round spends it
  (`Event::AirSupportLeft`). `Event::AirSupportArrived::armed` says whether
  anything was in the box as it came.
- **The helicopter** (`air_support/heli.rs`) is a companion.
  - *It comes to its tank.* From its edge it flies to where the seat's tank
    is now, and from then on holds station `air_heli_escort_px` (120) from
    it, following it as it drives: its velocity is the tank's own plus what
    closes the gap to its station, held to `air_heli_speed` (320 px/s, over
    `tank_speed`'s 210) and changing no faster than `air_heli_accel`
    (700 px/s^2), so it speeds up, swerves and brakes into its station
    rather than snapping. Its nose turns at `air_heli_turn_rate_deg` (240)
    at most. The station is held a cell inside the field.
  - *With nothing in view* (`AirStage::Escort`) the station is on the side
    of the tank it is already on, so it trails, leads or flanks as the tank
    turns; its nose follows its flight above `air_heli_nose_speed` (60).
  - *With a target in view* (`AirStage::Attack`) the station is toward the
    nearest one, nose on it, no nearer it than `air_heli_standoff_px` (160,
    outside a tesla's reach): straight toward a target far enough off, round
    the tank's flank for a nearer one, behind the tank for one on top of it.
    The station is also moved out of the standoff of every other target in
    view. Every `air_heli_salvo_seconds` (1.6) it fires a salvo at the
    nearest target: `air_heli_salvo_rockets` (2) seeker missiles off its
    pods (`missile.rs` - their climb, seek, lock and blast are a missile's,
    every rule of theirs kept; the blast is `Game::blast`). It fires only
    with its tank - within the escort plus `air_heli_station_slack_px` (64)
    of it - and its nose within `air_heli_aim_deg` (20) of the target. A
    missile's seek locks tanks only, so a salvo at a tower comes down on the
    tower unless an enemy tank is nearer the missile when it turns.
  - *It leaves when its rockets are gone*: `air_heli_salvos` (8) salvos,
    sixteen rockets, enough to bring a tesla (120) down. It stays
    `air_heli_linger_seconds` (1) after the last, the rockets on their way,
    and flies out towards where it came in (`AirStage::Egress`). No clock
    sends it home: with rockets left and nothing to shoot at it stays with
    its tank. It also leaves on the end screen, when its seat has no tank,
    and when that tank is a wreck with nothing in view.
- **The jet** (`air_support/jet.rs`) flies a plane's plan.
  - *The flight.* Always at `air_jet_speed` (264 px/s); its heading changes
    only through a turn of `air_jet_turn_deg_per_s` (100) at most, so its
    path is straights and arcs of a turn circle 151 px in radius. It banks
    into a turn (`Aircraft::bank`, rolled in and out over
    `air_jet_bank_seconds`, 0.25), which is how it is drawn.
  - *A target in view.* It locks one - the nearest its seat - and looks it
    up again every step, so a target that moves is followed; its nose is
    led by the target's velocity (`air_jet_aim_lead`) over the time left to
    the bombs' release. Gone - wrecked or out of the box - it takes the
    nearest that is left.
  - *The run* (`AirStage::Attack`). Lined up within `air_jet_gun_cone_deg`
    (8) and inside the cannon's range, it puts up to `air_jet_gun_bursts`
    (6) bursts in a stripe `air_jet_gun_len_px` (192) long, each landing
    `air_jet_gun_ahead_px` (160) ahead of it, the last on the target (radius
    28, 9 damage, fixed); on a shorter run the stripe is as long as the run.
    `air_jet_bomb_lead_px` (96) short of the target, with it within
    `air_jet_bomb_cone_deg` (20) of the nose, two bombs go
    `air_jet_bomb_spacing_px` (48) apart along its heading astride where the
    target is at that moment (radius 64, 75 to 85 damage). A full-health
    tank is wrecked; a gun tower's 150 survives one pass with about 40.
  - *Again.* That is one of its `air_jet_passes` (2). With one left it flies
    straight on `air_jet_extend_px` (360) past the target
    (`AirStage::Loiter`), turns back - the way that keeps it nearer the
    field - and runs in again on the same target while it lives, else the
    nearest. Another target already under its nose it runs in on at once.
    Off the field it turns back as soon as the way back leaves the cannon a
    run.
  - *A run it cannot make* - the target inside its turn circle - is a
    go-around: straight on, then back. After `air_jet_go_arounds` (2) the
    next sends it home.
  - *Nothing in view.* It flies to its seat's tank, circles it once on a
    ring `air_jet_orbit_px` (176) round where the tank is now
    (`air_jet_orbit_turns`, 1), rolls out and leaves. A target that shows
    meanwhile, with a pass left, is attacked. The same when its target is
    wrecked with a pass left and nothing else in view: one circle, then out.
    It flies one circle a sortie.
  - *It leaves* (`AirStage::Egress`, straight on off the field) out of
    passes, after its circle, on the end screen, with no seat, out of
    go-arounds, or `air_jet_sortie_seconds` (25) after it came - the cap
    that means it always leaves. An armed sortie is seven to twelve seconds.
  - It fires only with its ground point over the field, and never on the
    step it arrives.
- **Friendly fire is full, so the aircraft hold.** Every hit is
  `Game::blast` with `BlastParams::ordnance`, which hurts every hull and
  every bong it reaches, the caller's own included
  (docs/november-26-rebuild-upgrade.md decision 2). So an aircraft holds
  what would reach a friend, as a tower holds fire with a friend in line:
  the world shows it `AirView::friends` - every seat's live hull on the
  field and the players' bong, each with its half extent - and
  `AirView::clear` says whether a blast landing at a point reaches none of
  them with `air_hold_margin_px` (8) to spare. The jet lets go no cannon
  burst whose `air_jet_gun_radius_px` reaches one (the stripe walks on
  past it) and no bomb whose `air_jet_bomb_radius_px` does; a release
  with both bombs held spends no pass, and the jet comes round again. The
  helicopter holds a salvo while a friend stands within its reach of the
  target - `missile_blast_radius` past the tubes' widest spread
  (1.5 x `missile_impact_spread_px`, `heli::salvo_reach`) - and spends
  nothing until the target is clear. Tiles take damage whoever's they are
  (`damage_obstacle` with falloff) and crates break, as for a missile, so a
  player's own tower next to what is struck takes the splash - it is never
  aimed at.
- **Shot down.** The helicopter is an air target (`air.rs`,
  `AirKey::Aircraft`, its velocity the one it flies at): enemy bullets
  crossing its strike box hit it, and `air_heli_hits` (16) bring it down;
  the gun tower turns on it and the tesla arcs it; anything else that
  strikes the air - a rod, a well's core, the hammer's wave, a rail's slug,
  an EMP's ring - downs it at once. An EMP's ring is blind (`air.rs` rule
  2), so a seat's own pulse downs its own helicopter. Down, it is gone with
  a fireball where it falls (cosmetic: it hurts nothing,
  `Event::AircraftDowned`) and the seat's next call starts its count. The
  jet is no air target: nothing can hit it.
- **Nothing on the ground meets it.** It has no physics body, the nav grid
  never sees it, and walls, tiles and tanks pass under it.
- **Enemies do not react.** They do not see it coming and do not scatter;
  their only answer is shooting the helicopter down.
- **On the end screen** no call arrives and both aircraft go home; nothing
  fires.

## 2. Where it lives

- `src/air_support.rs`, headless: `Sortie` (Heli, Jet: `of` a crate,
  `pickup`, `code`, `ammo`), `AirCall` (id, seat, sortie, `arrive_at` -
  `None` while it waits; `left`), `AirStage` (Inbound, Attack, Egress,
  Escort, Loiter), `Aircraft` (id, seat, sortie, stage, ground point,
  height, heading, velocity, bank, entry, `stage_time`, `ammo`, `hits`, and
  each flight's own `HeliState` / `JetState`; `arrive`, `enter`, `step`,
  `gone`, `drawn`, `as_target`), the door between an aircraft and the world
  - `AirView` (the seat's position, velocity and whether it is a wreck, the
  `AirMark`s it may attack, the field, `live`) in and `AirAct` (`Salvo`,
  `Strike`) out -, the geometry (`entry_edge`, `entry_point`, `jet_edge`,
  `unit`, `turn_toward`), and the drawing on any canvas (`paint_shadows`,
  `paint_aircraft`, `banked_span`; docs/AIRCRAFT_SPEC.md).
- `src/air_support/heli.rs` and `src/air_support/jet.rs`: one flight each,
  `step` a pure function of the aircraft, the view and the fixed step. An
  aircraft reads the world only through its view and changes it only
  through its acts.
- `src/simulation/air_support.rs`, the world half: `call_air_support` (from
  `pickup_phase`), `air_support_phase` (before `spawn_pending`, so a salvo
  spawns the frame it is fired; with `live` false on the end screen) -
  `air_arrivals`, then for each aircraft by id its view (`air_anchor`,
  `air_marks`), its step and its acts (`air_salvo`, `air_strike`) -,
  `air_strike_show`, `aircraft_crash_show`, `strike_aircraft` (reached
  through `Game::strike_air`), the readers `Game::aircraft` and
  `Game::air_calls`.
- State: `Game::air_calls` (sorted by id, cleared by `init`) and the
  `Aircraft` entities. Ids come from `Game::take_shot_id`; an aircraft keeps
  its call's.
- Tests: `src/simulation/air_support_tests.rs` (the call, the queue, the
  shoot-down, the round the others share), `air_heli_tests.rs`,
  `air_jet_tests.rs`, and each headless file's own.

## 3. What it reuses

| Need | Piece |
|---|---|
| A crate that is not a weapon | the tower pack's arm in `pickup_phase`, after the tank's borrow |
| A count on the round clock, carried whole | the zones' pattern (`Zone::until`); a tank timer would restart at full on a client (`net::apply::set_timer`) |
| A flying entity | `fpv::Drone`: a hecs component alone, ground + height |
| Shot down | `air.rs`: `AirTarget`, a key, `Game::strike_air` the one mutator |
| The heli's rockets | seeker missiles (`Missile::spawn`, `Frame::pending_missiles`) |
| The jet's cannon and bombs | `Game::blast` with `BlastParams::air_gun`/`air_bomb` |
| Cosmetics a replica plays too | `*_show` into a `Spectacle` |
| The shadow | a tinted silhouette (docs/sprite-shadows-design.md) |

## 4. HUD, minimap, arrows

- **The box** (`hud::AirHud`, `hud::air_box`, `hud::air_box_layout` - the
  one geometry its painter draws): in the block's lamp row, under the ammo
  slot - `AIR_BOX_X` (108) in, `WEAPON_SLOT_W` (64) wide, scaled with the
  row. The lamp row (`CornerShape::lamp_row`) opens for it as for the
  lanterns and the heat shield (`air_support_up`, `PlayChrome` and
  `CornerShape::air_support`); the box sits clear of the lantern and the
  heat shield's gauge. It shows the crate's symbol and
  - while the call counts, the whole seconds left (`ceil`, never under 1)
    in amber, red over the last three;
  - while the aircraft is up, its ammunition (`Aircraft::ammo`: the
    helicopter's salvos, the jet's passes) as pips, one a round of its load
    - amber while left, dark once spent; one row up to six, two even rows
    past that (the helicopter's eight are four over four), twelve at most.
    Pips and not a number: a digit right after the count would read as
    seconds;
  - `+N` for the sorties queued, after either.
  Read on the client's present (`Game::zone_lead`). A wrecked seat's block
  keeps its box. In a room (`HudLayout::Compact`) only the local seat's
  block is shown.
- **The minimap** (`Marks::aircraft`): every aircraft, a frame in its
  seat's team colour round a dot, blinking while it attacks
  (`AirStage::Attack`), steady escorting, loitering, coming and going.
  Nothing marks a call still counting, and nothing is drawn on the field
  where it will strike.
- **The arrow** (`Scene::aircraft`, `ArrowKind::Aircraft`): every aircraft
  off the screen, in its seat's team colour, steady, never merged, never
  left out. On a phone, which draws no minimap, the arrow is the marker.

## 5. Drawing

docs/AIRCRAFT_SPEC.md has the sheet. `Game::paint_air_shadows` (the drones'
shadows and the aircraft's) runs after the floor, under the tanks;
`Game::paint_air` (the drones, then the aircraft over them - it flies
highest) over everything that stands. The window and `Game::paint_field`
run the same two stages, so a headless round's `screenshot` shows what
flies. The heli's rotor is a blur over its mast.

The jet is drawn banked by `Aircraft::bank`: seen from above, a bank is the
wingspan foreshortened, so its cell is drawn narrower across the wings
about its centre (`banked_span`) - each wing tip in by up to
`JET_BANK_TIP_PX` (12), in whole 2 px steps, 88 px level to 64 at a full
bank - and its shadow slides up to `JET_BANK_SHADOW_PX` (6) toward the
inside of the turn. No bank draws the cell exactly as it is. The helicopter
is drawn level.

The strikes are in the effects language (docs/effects.md): a cannon burst a
small fireball leaning down the run, the impact flash and a small scorch; a
bomb a big one, a ripple and a wide scorch, the grass flattened round it; a
downed heli the wreck's fireball. All of it hashed from position, no RNG.

## 6. Tuning

The `air_support` group in `tuning.rs`.

- Both: `air_support_delay_seconds`, `air_edge_margin_px`.
- The helicopter: `air_heli_height_px`, `air_heli_speed`, `air_heli_accel`,
  `air_heli_turn_rate_deg`, `air_heli_escort_px`, `air_heli_standoff_px`,
  `air_heli_station_slack_px`, `air_heli_nose_speed`, `air_heli_aim_deg`,
  `air_heli_salvos`, `air_heli_salvo_rockets`, `air_heli_salvo_seconds`,
  `air_heli_linger_seconds`, `air_heli_hits`, `air_heli_hit_half_px`. Its
  rockets are the missiles' own knobs.
- The jet's flight: `air_jet_height_px`, `air_jet_speed`,
  `air_jet_turn_deg_per_s`, `air_jet_bank_seconds`, `air_jet_passes`,
  `air_jet_aim_lead`, `air_jet_extend_px`, `air_jet_astern_deg` (how far
  astern a target must be before the jet picks the way round by the field
  rather than the short way), `air_jet_go_arounds`, `air_jet_orbit_px`,
  `air_jet_orbit_turns`, `air_jet_sortie_seconds`.
- The jet's weapons: `air_jet_gun_cone_deg`, `air_jet_gun_bursts`,
  `air_jet_gun_len_px`, `air_jet_gun_ahead_px`, `air_jet_gun_radius_px`,
  `air_jet_gun_damage`, `air_jet_gun_knockback`, `air_jet_bomb_cone_deg`,
  `air_jet_bomb_spacing_px`, `air_jet_bomb_lead_px`,
  `air_jet_bomb_radius_px`, `air_jet_bomb_damage_min`/`_max`,
  `air_jet_bomb_knockback`.

The shape of the jet's circle - how far along the tangent it steers and
from how far out its turn counts - is two constants in `jet.rs`
(`ORBIT_LEAD`, `ORBIT_REACH`): the control law, not a feel.

## 7. Text

No sentence on the field: the box is a symbol, digits and pips. The builder
names the two tools (`tool-air_heli`, `tool-air_jet` and their short forms
in `lang/en.ftl` and `lang/sl.ftl`).

## 8. Online

- `PROTOCOL_VERSION` 35: `AirState` carries the bank and the ammunition,
  and `AirStage` two more stages.
- **One family, `Snapshot::air`** (`wire::AirState`, by id): a call - its
  seat, sortie, stage (`AIR_WAITING`, `AIR_COUNTING`) and arrival tick - or
  an aircraft - its stage (`AirStage::code`), ground point, height, heading,
  bank (an `i8`, 127 a full one) and ammunition. Positioned in the delta
  (`air`, `air_moved`, `air_gone`), interpolated like a drone - the bank
  with the heading -, applied whole by `apply::apply_air_states`: a
  replica's aircraft are placed fresh each snapshot and never flown. Their
  velocity is not sent; a replica's is its heading at full speed.
- **Events**: `AirSupportCalled`, `AirSupportArrived`, `AirStrike`,
  `AircraftDowned`, `AirSupportLeft` travel; `AirSalvo` does not (the
  missiles are the state). A replica plays `air_strike_show` off
  `AirStrike` and `aircraft_crash_show` off `AircraftDowned`.
- A room of one plays the offline round; difficulty by seat count is
  untouched.
- The dev tools' `snapshot` (and the room server's `room_snapshot`) lists
  `aircraft` - id, seat, sortie, stage, position, height, heading, speed,
  bank, ammo - and `air_calls`; online it describes the replica.

## 9. Decisions

Oto, 2026-10-12:

1. **Crates.** Two kinds, `air_heli` and `air_jet`. The symbol on the box
   says which aircraft comes.
2. **Payload.** Strong: a sortie can wreck a full-health tank caught in the
   middle.
3. **Shoot-down.** The heli is an air target, the jet cannot be hit.
4. **A second crate while counting.** It queues a second sortie, which starts
   its count when the first one is gone. The HUD box shows +1.
5. **Spawning.** Map slots only, no random bonus roll. Seats only: enemies
   never take the crate or get aircraft of their own.
6. **Marker.** A minimap mark plus an off-screen arrow. No strike marker on
   the field.
7. **Enemy reaction.** None in v1 beyond shooting at the heli.
8. **Towers.** Enemy towers in the box are targets like enemy tanks
   (2026-10-12, after the first review). The players' own are not, but
   they still take splash like any tile.

Oto, 2026-11-20 (BB-177):

9. **The helicopter is a companion.** It comes to the tank of whoever took
   the crate, accompanies it and eliminates nearby targets until it runs
   out of ammunition; then it goes away. Its ammunition ends its sortie, not
   a clock.
10. **The jet flies a real plane's plan.** Toward the player's tank, onto a
    nearby target; a target that moves is followed. With no enemy on the
    screen it flies to the player, does a circle and flies away.

## 10. Determinism

The call, the arrival, the edge, both flights and the target pick draw no
RNG; the only draws are the blasts' damage rolls and the missiles' own, in a
round where a seat took an air crate. A round without one reads nothing
(`air_support_phase` returns at once) and keeps its stream, so
`the_one_and_two_seat_streams_are_pinned` is unchanged. Walks go by id;
targets sort by distance then key; trig through `math::`. A flight is a
function of the aircraft, its view and the fixed step, so the same sortie
is the same bits.

## 11. Tests

- `air_support_tests.rs`: an enemy never takes the crate; the crate keeps
  the special and nothing arrives before the count; a second crate queues a
  second sortie whose count starts when the first leaves; the heli comes
  down to `air_heli_hits` bullets and the next call counts; the jet is no
  air target; the players' own tower is no target; a sortie replays bit for
  bit; the field painter draws what flies.
- `air_heli_tests.rs`: it comes to its tank and goes where it drives; it
  fires every salvo, spaced, and leaves, sparing the seat and the enemy
  outside the box; with nothing in view it stays, and attacks an enemy that
  comes; it brings a tesla down; a wrecked seat with nothing in view and
  the end screen send it home.
- `air_jet_tests.rs`: it holds every burst and bomb that would reach the
  players' bong, which comes through whole, and bombs the same target in
  the open (`the_jet_holds_what_would_reach_the_players_bong`); it wrecks what is in the box and spares the rest; it
  follows a target moved after the run began and bombs a driving tank where
  it is; it comes back for a second pass; with no enemy in view it circles
  its seat once and leaves without firing; an enemy that shows while it
  circles is attacked; it always leaves; it fires only over the field; it
  runs in on an enemy tower; it replays bit for bit.
- `air_support/heli.rs`, `air_support/jet.rs`: each flight on hand-built
  views - the limits it keeps every step (speed, acceleration or turn rate,
  a unit heading, bank in range, ammunition only going down), its geometry,
  when it fires and every way it goes home.
- `air_support.rs`: the edges, the turn, the stages' codes, the banked
  cell. `hud.rs`: the lamp row, the box and its pips. `minimap.rs`: the
  mark blinks only attacking. `indicators.rs`: the arrow. `net`: the round
  trip of every event, of `AirState`, and a replica through every stage.

By hand: `maps/air-range.toml` (not shipped, `-m maps/air-range.toml`) is a
field for trying it out - a pad of helicopter and jet crates by the start,
waves of two tanks from the east, two blocks to drive round, and two yards
of enemy towers, one with two teslas close together. `--spawn band -e 0`
leaves only the towers, the west quiet for the jet's circle and the
helicopter's escort. Headless, `snapshot` lists `aircraft` and `air_calls`
and `screenshot` shows what flies.

## 12. Not in this change

- No sound (the game has none).
- The probe has no air support scenario of its own; `--crate air_heli`
  turns a map's weapon crates into air crates, as for any kind.
- The helicopter's `air_heli_hits` (16) dates from a ten-second sortie; it
  now stays as long as it has rockets, which wants a play-test.
- A replica's aircraft carries no velocity (§8): nothing a client draws or
  predicts reads it.
