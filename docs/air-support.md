# Air support

BB-119. A supply crate shows a helicopter or a jet. A seat that picks it up
calls air support: a count of 15 s runs in a small box under the HUD's ammo
slot, then the aircraft flies in - an Apache-like gunship or an A-10-like
attack jet, drawn ten per cent bigger than a tank - attacks the enemies
round the seat and flies away. The minimap shows it and an arrow points to
it when it is off the screen. The design questions were settled with Oto on
2026-10-12 (§9).

It is built from what the special weapons laid down. The rod from god
(docs/rod-from-god.md) gave the pattern of a call on the round clock carried
whole on the wire, the FPV swarm (docs/fpv-swarm.md) flying hecs entities
with a ground point and a height, a shadow and the air-target list, the
missiles `side_blast` - the side-aware area hit - and their seeker volleys,
and the tower pack a crate that is not a tank weapon. New here: the call
that belongs to a seat, the two aircraft, the HUD's box and the sprites.

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
  aircraft is gone - off the field or shot down.
- **The arrival.** At zero the aircraft comes in from outside the field,
  `air_edge_margin_px` (96) past an edge, in line with what it is coming for.
  Which edge is a hash of the call's id (`air_support::entry_edge`): not
  random, and no RNG. The jet's hash picks only the axis; it comes in from
  the side farther from its aim point (`jet_edge`), so its run over the
  field is at least half the field.
- **The target picture.** Taken when it arrives, not when the crate was
  taken: the enemies on the field inside the seat's sight box
  (`ai::in_sight_box`, the screen the follow camera shows round the seat),
  live, nearest first, ties to the lower slot. A wrecked seat's call still
  comes, round its wreck; a seat gone from the round spends it
  (`Event::AirSupportLeft`). No enemy in the box: the aircraft comes, passes
  over the seat - the heli hovers there `air_heli_linger_seconds` (1) - and
  leaves without firing; the crate is spent.
- **The helicopter.** At `air_heli_speed` (220 px/s) it flies to a hover
  point `air_heli_standoff_px` (160) short of the nearest target, back
  towards where it came in, held a cell inside the field. From there it
  fires `air_heli_salvos` (4) salvos, one every `air_heli_salvo_seconds`
  (0.9), each at the nearest enemy in the box at that moment, turning to
  face it: `air_heli_salvo_rockets` (4) seeker missiles off its pods
  (`missile.rs` - their climb, seek, lock and blast are a missile's, every
  rule of theirs kept; the blast is `side_blast`). Then it leaves the way it
  came. It is on the field about ten seconds.
- **The jet.** At `air_jet_speed` (520 px/s), one straight pass through the
  middle of the nearest `air_jet_targets` (3) targets in the box. Its
  cannon puts `air_jet_gun_bursts` (6) bursts in a stripe
  `air_jet_gun_len_px` (192) long ending on that point, each landing
  `air_jet_gun_ahead_px` (160) ahead of the jet (radius 28, 9 damage, fixed);
  `air_jet_bomb_lead_px` (96) short of the point it lets two bombs go, landing
  `air_jet_bomb_spacing_px` (48) apart astride it (radius 64, 75 to 85
  damage). A full-health tank on the aim point is wrecked. It fires only
  over the field: on a run too short for the stripe and its lead, both
  shrink to fit (`gun_burst`, `bombs`). It flies on off the far edge; about
  three seconds on the field.
- **Friendly fire.** None: every hit is `side_blast`/`explosion_hit`, which
  spare `Owner::same_side` - seats, the players' frog and towers. Tiles take
  damage (`damage_obstacle` with falloff) and crates break, as for a
  missile.
- **Shot down.** The helicopter is an air target (`air.rs`,
  `AirKey::Aircraft`): enemy bullets crossing its strike box hit it, and
  `air_heli_hits` (16) bring it down; the gun tower turns on it and the tesla
  arcs it; anything else that strikes the air - a rod, a well's core, the
  hammer's wave, a rail's slug, an EMP's ring - downs it at once. An EMP's
  ring is blind (`air.rs` rule 2), so a seat's own pulse downs its own
  helicopter. Down, it is gone with a fireball where it falls (cosmetic: it
  hurts nothing, `Event::AircraftDowned`) and the seat's next call starts its
  count. The jet is no air target: nothing can hit it.
- **Nothing on the ground meets it.** It has no physics body, the nav grid
  never sees it, and walls, tiles and tanks pass under it.
- **Enemies do not react.** They do not see it coming and do not scatter;
  their only answer is shooting the helicopter down.
- **On the end screen** no call arrives, a helicopter breaks off its attack
  and leaves, a jet flies on; nothing fires.

## 2. Where it lives

- `src/air_support.rs`, headless: `Sortie` (Heli, Jet: `of` a crate,
  `pickup`, `code`), `AirCall` (id, seat, sortie, `arrive_at` - `None`
  while it waits; `left`), `Aircraft` (id, seat, sortie, `AirStage` Inbound,
  Attack, Egress, ground point, height, heading, entry, mark, `stage_time`,
  `flown`, `fired`, `armed`, `hits`; `advance` one fixed step - the heli in
  to its hover and out past its entry, the jet straight along its run -,
  `gone`, `drawn`, `as_target`), the geometry (`entry_edge`, `entry_point`,
  `hover_point`, `run_mark`, `gun_burst`, `bombs`), and the drawing on any
  canvas (`paint_shadows`, `paint_aircraft`; docs/AIRCRAFT_SPEC.md).
- `src/simulation/air_support.rs`, the world half: `call_air_support` (from
  `pickup_phase`), `air_support_phase` (before `spawn_pending`, so a salvo
  spawns the frame it is fired; with `live` false on the end screen),
  `air_arrivals`, `air_acts` (what an aircraft fires this step),
  `air_salvo`, `air_strike` and `air_strike_show`, `aircraft_crash_show`,
  `strike_aircraft` (reached through `Game::strike_air`), the readers
  `Game::aircraft` and `Game::air_calls`.
- State: `Game::air_calls` (sorted by id, cleared by `init`) and the
  `Aircraft` entities. Ids come from `Game::take_shot_id`; an aircraft keeps
  its call's.
- Tests: `src/simulation/air_support_tests.rs`, and the headless half's own.

## 3. What it reuses

| Need | Piece |
|---|---|
| A crate that is not a weapon | the tower pack's arm in `pickup_phase`, after the tank's borrow |
| A count on the round clock, carried whole | the zones' pattern (`Zone::until`); a tank timer would restart at full on a client (`net::apply::set_timer`) |
| A flying entity | `fpv::Drone`: a hecs component alone, ground + height |
| Shot down | `air.rs`: `AirTarget`, a key, `Game::strike_air` the one mutator |
| The heli's rockets | seeker missiles (`Missile::spawn`, `Frame::pending_missiles`) |
| The jet's cannon and bombs | `side_blast` with `BlastParams::air_gun`/`air_bomb` |
| Cosmetics a replica plays too | `*_show` into a `Spectacle` |
| The shadow | a tinted silhouette (docs/sprite-shadows-design.md) |

## 4. HUD, minimap, arrows

- **The box** (`hud::AirHud`, `hud::air_box`): in the block's lamp row, under
  the ammo slot - `AIR_BOX_X` (108) in, `WEAPON_SLOT_W` (64) wide, scaled
  with the row. The lamp row (`CornerShape::lamp_row`) opens for it as for
  the lanterns and the heat shield (`air_support_up`, `PlayChrome` and
  `CornerShape::air_support`); the box sits clear of the lantern and the heat
  shield's gauge. It shows the crate's symbol and the whole seconds left
  (`ceil`, never under 1) in amber, red over the last three; the symbol
  alone while the aircraft is up; `+N` for the sorties queued. Read on the
  client's present (`Game::zone_lead`). A wrecked seat's block keeps its box.
  In a room (`HudLayout::Compact`) only the local seat's block is shown.
- **The minimap** (`Marks::aircraft`): every aircraft, a frame in its
  seat's team colour round a dot, blinking while it attacks. Nothing marks
  a call still counting - its targets are picked only when it arrives - and
  nothing is drawn on the field where it will strike.
- **The arrow** (`Scene::aircraft`, `ArrowKind::Aircraft`): every aircraft
  off the screen, in its seat's team colour, steady, never merged, never
  left out. On a phone, which draws no minimap, the arrow is the marker.

## 5. Drawing

docs/AIRCRAFT_SPEC.md has the sheet. The shadow is drawn after the floor,
under the tanks (`render/game.rs` beside the drones' shadows); the aircraft
over everything that stands, after the drones - it flies highest. The heli's
rotor is a blur over its mast. The strikes are in the effects language
(docs/effects.md): a cannon burst a small fireball leaning down the run, the
impact flash and a small scorch; a bomb a big one, a ripple and a wide
scorch, the grass flattened round it; a downed heli the wreck's fireball. All
of it hashed from position, no RNG.

## 6. Tuning

The `air_support` group in `tuning.rs`: `air_support_delay_seconds`,
`air_edge_margin_px`, `air_heli_height_px`, `air_heli_speed`,
`air_heli_standoff_px`, `air_heli_salvos`, `air_heli_salvo_rockets`,
`air_heli_salvo_seconds`, `air_heli_hits`, `air_heli_hit_half_px`,
`air_heli_linger_seconds`, `air_jet_height_px`, `air_jet_speed`,
`air_jet_targets`, `air_jet_gun_bursts`, `air_jet_gun_len_px`,
`air_jet_gun_ahead_px`, `air_jet_gun_radius_px`, `air_jet_gun_damage`,
`air_jet_gun_knockback`, `air_jet_bomb_spacing_px`, `air_jet_bomb_lead_px`,
`air_jet_bomb_radius_px`, `air_jet_bomb_damage_min`/`_max`,
`air_jet_bomb_knockback`. The heli's rockets are the missiles' own knobs.

## 7. Text

No sentence on the field: the box is a symbol and digits. The builder names
the two tools (`tool-air_heli`, `tool-air_jet` and their short forms in
`lang/en.ftl` and `lang/sl.ftl`).

## 8. Online

- `PROTOCOL_VERSION` 30: two `PickupKind` variants and a snapshot family.
- **One family, `Snapshot::air`** (`wire::AirState`, by id): a call - its
  seat, sortie, stage (`AIR_WAITING`, `AIR_COUNTING`) and arrival tick - or
  an aircraft - its stage (`AirStage::code`), ground point, height and
  heading. Positioned in the delta (`air`, `air_moved`, `air_gone`),
  interpolated like a drone, applied whole by `apply::apply_air_states`: a
  replica's aircraft are placed fresh each snapshot and never flown.
- **Events**: `AirSupportCalled`, `AirSupportArrived`, `AirStrike`,
  `AircraftDowned`, `AirSupportLeft` travel; `AirSalvo` does not (the
  missiles are the state). A replica plays `air_strike_show` off
  `AirStrike` and `aircraft_crash_show` off `AircraftDowned`.
- A room of one plays the offline round; difficulty by seat count is
  untouched.

## 9. Decisions (Oto, 2026-10-12)

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

## 10. Determinism

The call, the arrival, the edge, the flight and the target pick draw no RNG;
the only draws are `explosion_hit`'s damage rolls and the missiles' own, in a
round where a seat took an air crate. A round without one reads nothing
(`air_support_phase` returns at once) and keeps its stream, so
`the_one_and_two_seat_streams_are_pinned` is unchanged. Walks go by id;
targets sort by distance then slot; trig through `math::`.

## 11. Tests

`air_support_tests.rs`: an enemy never takes the crate; the crate keeps the
special and nothing arrives before the count; the jet wrecks the tank on its
aim point and spares the one outside the box and the seat; the heli flies in,
hovers, fires its salvos and leaves, sparing the seat; with no enemy in the
box neither fires; the jet fires only over the field, wherever the seat
stands; a second crate queues a second sortie whose count starts
when the first leaves; the heli comes down to `air_heli_hits` bullets and the
next call counts; the jet is no air target; a sortie replays bit for bit.
`air_support.rs`'s own: the edges, the hover point, the run mark, the gun
stripe and the bombs, the flights. `hud.rs`: the lamp row and the box.
`indicators.rs`: the arrow. `net::events`: the round trip of every event.

## 12. Not in this change

- No shipped map places an air crate yet; a map gets one with
  `pickup = "air_heli"` / `"air_jet"` or the builder's tools.
- No sound (the game has none).
- The probe has no air support scenario of its own; `--crate air_heli`
  turns a map's weapon crates into air crates, as for any kind.
