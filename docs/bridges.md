# Bridges: a deck across water and lava (PRD)

Status: proposal, BB-62. Nothing here is built yet; every section says what
the game does today, what a bridge should do, and where the change lands.

A bridge is a straight deck a tank drives across a river, a lake or a
lava flow, **wide enough for two tanks to pass each other on it** - any two
of the twelve chassis, head on, without one backing off. The water keeps
flowing under it; fish swim under it; shots fly over it as over the water
round it. It is the crossing the map's author chose, so it is where the
fight goes: a chokepoint both sides want, and (in the second phase) a
thing to blow up.

## 1. What the game does today

- **Deep water and deep lava** are walls to hulls (seam-closed static
  colliders, `Game::init` from `WaterLayout::deep_grid_cells`) and nothing
  to shots; **fords** slow a hull, loosen its grip and, in a north/south
  stream, carry it south (docs/water.md). Lava fords burn (docs/volcano.md).
- **A road cell painted in a river's line** is the only crossing. A cell
  holds one object, so the road *replaces* the water: `ground::Layout`
  draws an isolated road tile with the river cut on both sides
  (`road_wins_over_water_and_the_two_never_join`), the rules read it as
  `Depth::Dry`, and the lava doc calls it "a stone crossing". It reads as a
  causeway that dams the river, not a bridge, and nothing makes it wide.
- docs/water.md lists "there is no bridge tile" under *Not done, on purpose*.
  This document is that decision.

## 2. Goals and non-goals

Goals:

1. A bridge tile in the map format, the builder and every shipped renderer
   (GPU, `CpuCanvas` thumbnails, the minimap, the builder's canvas).
2. **Two tanks pass** on any bridge the linter accepts (section 3).
3. Water and lava keep their shape, picture and animation under a deck.
4. The AI routes over bridges, prefers them to fords, and does not jam on
   them (section 6, checked by a probe fixture).
5. No RNG, no wire change in phase 1; a map without a bridge replays and
   renders byte for byte (determinism tests, pinned thumbnail hashes).

Non-goals (phase 1): destruction (phase 2, section 9), bridges over dry
ground (viaducts), diagonal or curved decks, a lower level a tank drives
under the deck on, amphibious chassis, repair.

## 3. How wide is "two tanks"

The numbers that decide it are in `lib.rs`: a tank's **movement collider**
is `TANK_HULL_BBOX_BY_ROW` x `scale` (2.0) x `TANK_MOVE_BBOX_FRACTION`
(0.9), a rounded box (`TANK_MOVE_CORNER_RADIUS` 4). Driving along a deck a
hull presents its width across it:

| Chassis | across (px) | along (px) |
|---|---|---|
| scout, wraith | 25.2 | 34.2 |
| assault, longbow, flak, warden, glacier, obelisk | 28.8 | 30.6 - 46.8 |
| breaker, ravager | 32.4 | 43.2 |
| leviathan | 39.6 | 50.4 |
| titan | **43.2** | 46.8 |

Two hulls side by side on a deck of `n` cells (32 px each):

| Deck | Two titans (86.4) | Titan turning (50.4 long) + titan | Two assaults (57.6) |
|---|---|---|---|
| 2 cells, 64 px | no | no | 6.4 px spare - a sloshing turn touches |
| **3 cells, 96 px** | 9.6 px spare | 2.4 px spare | 38.4 px spare |
| **4 cells, 128 px** | 41.6 px spare | 34.4 px spare | 70.4 px spare |

So **three cells is the least deck on which any two hulls pass**, and two
cells is a one-tank bridge for the heavy chassis.

The AI sees the deck through the nav grid. A map cell's centre sits on a
grid vertex (`col * 32`) and a nav cell's half a cell off it, and a nav
cell is blocked when its centre lies within `half_extent + margin` (16 +
25.2: `max_tank_clearance_half_extent`, the leviathan's long half, every
hull in every heading) of a blocking box - the deep water, or the rail
(section 4.3), at the deck's edge. As with the corridors of
`maps/test/corridors/`, **a deck `n` cells wide leaves `n - 1` nav rows**,
32 px apart:

| Deck | Nav rows | What the AI does |
|---|---|---|
| 2 cells | 1 | single file; it never plans to pass |
| 3 cells | 2, 32 px apart | two rows, but only the light hulls (25.2 - 28.8 across) fit side by side at 32 px; two heavies meeting head on pass by sliding off each other (the brake, the avoidance, the stuck escape) |
| 4 cells | 3, the outer two 64 px apart | two-way traffic in the outer rows for every chassis (two titans need 43.2 px between centres) |

**Decision proposed:** the builder's BRIDGE tool lays **4-cell** decks by
default, the linter refuses a deck under **3** (`bridge-narrow`, an error -
it breaks the promise this feature makes), and a 3-cell deck is a warning
(`bridge-tight`: any two hulls *fit*, but the planner has no two rows a
pair of heavies can hold). Open question 1 asks whether 3 is worth
allowing at all.

## 4. Rules

### 4.1 The map

```toml
cells."20,9"  = { kind = "bridge" }                      # over water, stone
cells."20,10" = { kind = "bridge", over = "lava" }        # over lava
cells."21,9"  = { kind = "bridge", material = "wood" }   # phase 2
```

- `kind = "bridge"` is a `CellObject::Bridge { over, material }`; `over`
  is `water` (absent) or `lava`, `material` is `stone` (absent) or `wood`.
  Both are written back only when not the default, so a file reads the way
  every other optional key does. The BRIDGE tool writes `over` from the
  liquid it was laid across, so a hand-written file is the only place the
  default is relied on. Lava takes `stone` only (`bridge-wood-on-lava`, an
  error; read as stone).
- **A deck is a connected component of bridge cells, a rectangle.** Its
  **axis** is the one along which both ends touch a non-liquid cell (the
  banks); its **width** is its extent across. A component that is not a
  rectangle, or whose axis is ambiguous (both or neither pair of ends on a
  bank), is a lint error (`bridge-shape`, `bridge-unanchored`) and is read
  as its bounding box's longer extent. `map::bridges(&MapFile) ->
  Vec<Span>` (cells, axis, width, over, material) is the one reading every
  consumer takes, sorted by first cell (determinism).
- Considered and not taken: a top-level `[[bridge]] from to width` table of
  spans over cells that stay painted water. It reads better and needs no
  inference, but every builder tool - the brush, FILL, the select tool's
  clips and stamps, undo, the kept index, the lint fixes, the minimap - is
  cell based, and a second kind of object would need its own path through
  all of them.

### 4.2 Ground and water under a deck

- **Shape:** `ground::Layout` counts a bridge cell as its `over` liquid when
  it reads the water's (or the lava's) shape. A river runs on under the
  deck, a lake keeps its open middle and grows no shore round the piers, and
  the deep cells beside the deck keep their colliders. (Counting it as dry
  ground would ring the deck with fords, making a 4-cell bridge a 6-cell
  causeway and moving the deep water's colliders.)
- **Depth:** `WaterLayout`/`LavaLayout` give a deck cell `Depth::Deck`, a new
  variant: dry for every rule (no slowing, no current, no spray, no wet
  tracks, fire and heat as on dry ground, nothing spawns into a lake), with
  the liquid underneath kept for the picture (`under(cell)`: the current's
  marks and the fish pass under, the lava's flow bands carry on). `is_wet`
  stays false; `freeze` leaves a deck a deck; `fill` (a rod's crater
  filling) never touches one.
- **Lava's heat:** a deck over lava is `lava_heat` x `bridge_lava_heat_factor`
  (0.5) hot, so `heat_hurt_from` still bites a hull that parks on it -
  a lava bridge is crossed, not held. A heat shield keeps it off.

### 4.3 The rails

Each long edge of a deck carries a rail: a static collider
`bridge_rail_px` (4) thick along the outer cell edge, ending a cell short
of each bank so the bridgehead stays open. It is what makes the width a
property of the deck alone - over a ford the cells beside the deck are
drivable, and without a rail a hull would slide off into the stream
sideways (a sonic hammer's knock, a well's pull, a skid on ice). Over deep
water the rail stands just inside the deep cell's box and changes nothing a
hull feels.

- **Shots** fly over a rail (it is low, like the water) - `hits::Terrain`
  never sees it, so a tank on the deck fights across the river as a tank on
  the bank does. Light passes too (`blocks_light` false).
- **Nav:** `Grid` gains `block_rect` (a box, not a square half-extent) and
  the rails are blocked with it at `init` and in `NavCache`'s layer, so the
  deck's lanes are the section 3 table.
- **Physics:** one cuboid per rail run, the deep-water pattern
  (`tile_hull_half_extent`), so a hull slides along it without catching.
- **Online:** the pose validator refuses a pose on the far side of a rail
  from the room's copy, as it refuses one inside a solid tile.

### 4.4 Placement

- `enemy_spawn_legal` refuses a deck cell (a wave tank does not appear in the
  chokepoint); band spawns, wave gates and `relocate_unusable_spawns` follow.
  A `start` cannot be a deck (a cell holds one object); PLAY HERE
  (`drop_cell`) skips decks; a pickup cannot stand on one (one object).
- A gate cannot be a deck cell; a deck that runs to the map's edge is
  `bridge-off-map` (warning) since its far end has no bank.

## 5. Picture

- **Sheet:** `tools/spritegen/gen_bridges.py` -> `static/bridges_sheet.png`
  (no Pillow, raw PNG bytes like `gen_towers.py`), 32 px cells authored on
  the walls' 2 px block grid (16 x 16 design pixels), on the shared palette
  (`tools/punypalette.py`, `just check-sheets`, no green). Rows: stone, wood
  (phase 2), each axis drawn on its own (the light comes from one side, so
  the art is not rotated at draw time); columns by place in the deck: the
  two abutment ends x left rail / middle / right rail, the span's middle
  x the same three, and phase 2's damaged and broken columns. A spec,
  docs/BRIDGES_SPEC.md, in the other sheets' shape.
- **Drawing** (`bridge.rs`, generic over `canvas::Canvas`): in `paint_floor`
  after the ground (water and lava animated under it) and before
  `paint_floor_marks`, so tread marks, scorches and rubble land *on* the
  deck. Fish are drawn in `paint_field_lit` straight after the floor, so
  they would swim over a deck: `fish.rs` skips a fish whose cell is a deck
  (it is under it). The current's marks are drawn by the ground and
  disappear under the deck by the same order.
- **Shade:** the deck casts a stepped shadow on the water along
  `shadow_dir` (the floor shade's `ShadeRecipe`, a bridge counted like a
  wall's foot), and a band of foam stands on the upstream side of each pier
  where a north/south stream's current meets it - hashed per cell, no RNG.
- **Weather:** the cell mask reads a deck as road, not water, so snow
  settles on it and never ices it, and rain puddles it like a road; the
  plain (shaderless) path follows the same mask.
- **Minimap:** a `Class::Bridge` palette step (the deck's colour) per cell;
  the builder's navigator inherits it.
- **Thumbnails:** the pinned hashes of the shipped maps stay as they are
  until a shipped map gets a bridge.

## 6. The AI

- A deck is open, unweighted ground to `Grid::weigh` - cheaper than any
  ford (`water_ford_path_cost` 3, `lava_ford_path_cost` 14) - so the flow
  fields send enemies over a bridge whenever one is near their way.
- **Jams.** A bridge is the classic place a pathfinder's units pile up: every
  route through one gap at once. What the game already has for it: the
  crowd surcharge (`route_crowd_cost`) prices a cell another enemy stands
  in, the lane surcharge (`route_lane_cost`) prices a deck lying down a
  seat's barrel - so a covered bridge sends some enemies to the ford, which
  is the right tactic -, the personal-space brake and the engage ring. What
  is new: a probe fixture, `maps/test/bridges.toml` (a river with a 3-cell
  and a 4-cell bridge and a ford between, enemies on both banks), run in
  `just probe-fixtures` with ceilings for `pile-up`, `clustering`,
  `tank-grind`, `stall` and `never-arrived`. If a 3-cell deck jams beyond a
  ceiling worth keeping, that is the answer to open question 1.
- Engage slots, waypoints and pickups searched by A* cross decks as any
  open cell. The sight box, the alerts and the snipe do not change.
- Later (not phase 1): an AI that holds a bridgehead (a guard's post at the
  deck's end), and in phase 2 one that blows a bridge behind it or in front
  of a seat.

## 7. Builder

- A **BRIDGE** tool in the Ground category beside road, water and lava,
  with RECT's gesture: drag from bank to bank and the deck is laid on the
  drag's long axis, `builder_bridge_width` (4, a MAP-panel-free brush
  setting: 3 or 4, BRUSH's row) cells wide, centred on the drag, `over`
  from the liquid it crosses. It replaces the cells it covers, one
  `EditStep`. The eraser takes a deck back to its `over` liquid (not to
  grass), so erasing a bridge leaves the river whole.
- PEN, FILL and SCATTER paint single bridge cells for touch-ups; the linter
  says when the result is no longer a deck.
- **Lint** (`maplint.rs`, CHECK's rows): `bridge-narrow` (error, with a FIX
  that widens it to 3 where the cells beside are liquid), `bridge-shape`,
  `bridge-unanchored`, `bridge-wood-on-lava` (errors), `bridge-tight`,
  `bridge-off-map` (warnings). Every message is a `lint-` key in
  `lang/en.ftl` and `lang/sl.ftl` within its budget, as are `tool-bridge`
  and `tool-short-bridge`.

## 8. Online

Phase 1 changes nothing on the wire: the map's TOML rides the `Welcome`, a
deck is static, and the replica and the sandbox build the same rails and
depths in `init`. A build that cannot parse `kind = "bridge"` cannot be on
the same protocol as one that can, since the client and the room ship
together on the version tag; `PROTOCOL_VERSION` is bumped only by phase 2.
`hostable()` and the lobby's stepper are untouched.

## 9. Phase 2: wooden bridges that go down

The wargame's oldest objective. Proposed, for its own issue once phase 1
has been played:

- A wooden deck is cut across into **sections**, one row of cells across
  the span each, every section with `bridge_wood_health`. A deck is not an
  `Obstacle` (the nav grid and the linter take every obstacle as solid), so
  sections live in `Game::bridges`, keyed by cell.
- **What breaks one:** blasts (drums, missiles, grenades, cook-offs, a
  wreck's blast, the rod) by falloff, and fire - a section burns like
  timber (`flammable`, the flamethrower's heat, a burning oil trail across
  it), charring through `tick_burns`'s rule. Shells and bullets fly over a
  deck as over water and do not hurt it. Stone takes the rod alone, or
  nothing (open question 3).
- **The collapse:** a broken section's cells become their `over` liquid
  (`WaterLayout::open_deck`, the `fill` precedent): deep cells get their
  colliders, the nav layer is rebuilt, the rails of that section go. A hull
  whose centre stands on a cell that turns deep is **lost**: wrecked where
  it fell, sunk (no wreck left, `Event::Fell`), credited to whoever's blast
  broke the section. A seat lost so returns with the next wave like any
  wreck. The sections either side stand, so a bridge goes down in pieces
  and a broken one can still be crossed to the gap.
- **Wire:** `BridgeState` (a section's health in whole points, burning)
  for the sections that are not whole, `WireEvent::SectionBroken`, a
  `PROTOCOL_VERSION` bump; the replica opens the deck from the event as it
  removes a dead tile. The pose validator refuses a pose on an opened
  section.
- **The AI:** the nav rebuild makes it route round a broken bridge at once;
  a hammer or a well that throws an enemy into the gap is a kill.

## 10. Knobs

A `bridge` tuning group: `bridge_rail_px` (4, Restart),
`bridge_lava_heat_factor` (0.5), `builder_bridge_width` (4, in `builder`);
phase 2 adds `bridge_wood_health`, `bridge_burn_seconds`,
`bridge_blast_damage_factor`.

## 11. Tests

- `mechanics_tests`: two titans driven head on in opposite lanes of a
  3-cell deck both reach the far bank; a hull stopped on a deck over a
  north/south stream does not drift; a knock across a deck over a ford
  leaves the hull on the deck (the rail); a hull parked on a lava deck
  burns at the factor and not under a heat shield; nothing spawns on a deck.
- `pathfind`/`nav`: a deck `n` cells wide leaves `n - 1` nav rows, with the
  rails as with deep water; `NavCache` equals `Game::scratch_nav` with rails.
- `ground`: a deck keeps a river's tiles and a lake's deep middle as they
  are with water there (the picture of every non-deck cell unchanged); a
  repaint of a deck lands where `build` would.
- `map`: round trip of `over` and `material`, defaults not written; the
  `bridges()` reading of the lint shapes.
- `maplint`: each new kind on a fixture under `maps/test/`, and its FIX.
- `determinism_tests` and the thumbnail hashes unchanged; the probe
  fixture with recorded ceilings.

## 12. Work, as issues

1. Map format, `Depth::Deck`, rails, nav, spawns, lava heat, the tests above
   (headless; no picture yet beyond a flat placeholder).
2. The sheet, its spec, the drawing, shade, foam, fish, weather mask,
   minimap.
3. The BRIDGE tool, the lint checks and their fixes, the strings.
4. The probe fixture and its ceilings; the AI tuning that falls out of it.
5. A shipped map that uses bridges (longwater's lake and serpent-river are
   the natural first ones), as its own level change.
6. Phase 2: wooden bridges and the collapse.

## 13. Open questions

1. **Allow 3-cell decks, or only 4?** 3 lets any two hulls pass, but the
   planner's two rows hold only light hulls side by side; 4 is two lanes
   for every chassis at the cost of a wider gap in the river. The probe fixture is meant to answer it.
2. **Rails or none over deep water?** Proposed: always, for one rule.
   Without them over deep water nothing changes for a hull, but the deck's
   width would then depend on what is under it.
3. **Is stone ever destructible** - to the rod from god only, or never?
4. **Should a tank fall off a ruined deck into a ford** rather than only
   into deep water (a slow, burning wade out under lava)? Phase 2.
5. **Bridges over dry ground** (a ravine, a dry riverbed) would need a
   ground under the deck that is impassable but not liquid - a new floor
   kind. Out of scope unless a map asks for it.
