# PRD: the mushroom hunt

Status: **draft, 2026-10-06.** Nothing is implemented. This is BB-6, the
design document for BB-5. Section 2 lists the decisions the feature needs,
each with the answer this document assumes until the author changes it. The
art is being chosen in the "Mushroom picks" artifact
(https://claude.ai/artifact/85JnrZJxxPaNeKkf5ZuGPR). It holds six mushrooms,
five ways of changing colour, three sizes, three pick-up shows and three HUD
counters, drawn on the game's 2 px block grid beside a real player tank, an
enemy and the trees from the sheets. Section 6 names the proposals by their
codes (M1-M6, C1-C5, S1-S3, P1-P3, R1-R3) and is filled in once the picks
are made. The artifact's **Copy picks** button gives the line to paste back.

The request, as written (BB-5):

> As a player I want to be able to play a game of collecting mushrooms
> across the map/level.
>
> There needs to be a new mission type mushrooms hunt where the objective is
> to collect all mushrooms placed on the map.
>
> Mushrooms are placable items that are placed on map with map builder.
>
> Mushroom design follows the design language of the whole game. However
> mushrooms should be bigger than tanks and colorful and they should change
> colors so that they are visible to the players.
>
> Once player picks up mushroom there should be some kind of indicator
> telling the player how many are left.

And BB-6: "Prepare the design document for mushrooms where I can pick
various designs."

Contents

1. Why
2. Decisions: proposed answers
3. What a mushroom is
4. The mission
5. Finding them: the count, the arrows, the minimap
6. Presentation
7. Map format, builder and linter
8. Online co-op
9. Determinism
10. Knobs
11. Tests
12. Non-goals and open questions
13. Phases

## 1. Why

Every mission today ends in a fight. Protect, Hunt and Destroy are all won
by wrecking tanks or a frog. A collecting mission rewards a different skill:
covering ground, reading the map and routing through danger instead of
clearing it. It also gives field maps (docs/large-maps-follow-camera.md)
something they are well suited to. A 60-cell map with mushrooms in its far
corners is a tour of the whole level, and the follow camera, the off-screen
arrows and the minimap were built for exactly that.

## 2. Decisions: proposed answers

| # | Question | Proposed answer | Section |
|---|---|---|---|
| 1 | What is the mission called? | `Mission::Forage`, spelt `forage` in data (`mission.kind = "forage"`, `--mission forage`). Players read **MUSHROOM HUNT** (`mission-forage`). | 4 |
| 2 | Is there combat? | Yes. Enemies spawn by the map's band or waves as in every mission. The mushrooms are the objective and the enemies are the obstacle. | 4 |
| 3 | How is the round lost? | When every seat is a wreck at once, the same rule as every mission. A map that places a frog also loses on the frog's death, as in Protect, so a level can ask for both. | 4 |
| 4 | Must the enemies all be wrecked to win? | No. The last mushroom wins the round on the frame it is picked up. Enemies still alive are left standing. | 4 |
| 5 | Who can pick one up? | Any seat. Enemies drive over mushrooms and leave them. | 3 |
| 6 | Does a mushroom do anything else? | No heal, ammo or buff in the first version. It only counts. A small heal is a candidate knob (`mushroom_heal`, default 0). | 3, 10 |
| 7 | Does a mushroom block anything? | No. It is not solid and not an `Obstacle`: no collider, no nav-grid cell and no line-of-sight blocker. Tall grass is the precedent. Shots fly over it. | 3 |
| 8 | Can it be destroyed? | No. A blast or fire leaves it alone, since a mission that a stray drum can make unwinnable is no fun. | 3 |
| 9 | Do they respawn? | No. Each one is picked up once per round. | 3 |
| 10 | Does it count in other missions? | A map's mushrooms are drawn and collectable in every mission but only counted in Forage. A builder author sees them, and they do nothing elsewhere. Alternative: drawn only in Forage. | 3, 7 |
| 11 | Bigger than a tank: how big? | The art is S1/S2/S3 (pick). The **collect box** is one cell grown by `mushroom_collect_pad_px`, whatever the art's size, so a big mushroom never collects from two cells away. | 3, 6 |
| 12 | What tells the player how many are left? | The R pick in the HUD, plus off-screen arrows and minimap marks for the ones still out there, which come at no extra cost. | 5 |

## 3. What a mushroom is

A mushroom is **map data turned into a round entity at `init`**, built like
the pickups (`pickup.rs`). It is not an `Obstacle`, because the nav grid and
the linter treat every obstacle as impassable. A new `mushroom.rs` at the
top level (no clash: the mushroom *cloud* lives in `mushroom.rs` today and
is renamed `mushroom_cloud.rs` in phase 1) holds `Mushroom { cell, taken:
Option<TakenBy>, seed }` and its drawing, generic over `canvas::Canvas`.

- **Placement.** One per `CellObject::Mushroom` cell. Multi-instance like
  portals, at most `MUSHROOM_MAX` (64) a map.
- **Collection.** On the frame a seat's hull box touches the cell's square
  grown by `mushroom_collect_pad_px`, the same box test as
  `Pickup::in_reach`. Seats only, walked in seat order, so when two seats
  touch the same mushroom on the same frame the lower seat takes it. No RNG.
  A seat in a gate lane (`seats_on_field()`'s `None`) takes nothing.
- **Events.** `Event::MushroomTaken { seat, cell, left }`. The HUD's toast,
  `fx.rs`'s show and the online mirror all read it.
- **Not solid, not cover.** No collider and no nav-grid change. The AI does
  not know mushrooms exist, but it does know about the players heading for
  them (section 4).

## 4. The mission

`Mission::Forage` joins `Protect`/`Hunt`/`Destroy` in `level.rs` with
`text::keys` `mission-forage`, the banner's line ("PICK UP EVERY
MUSHROOM!") and the end screen's words.

- **Win**: `check_round_end` sees the last mushroom taken. The outcome is a
  win on that frame, and the enemies are frozen as on any win.
- **Lose**: every seat wrecked at once, or the players' frog dead on a map
  that has one (decision 3).
- **Waves** run as the map says. A wave round does not end when the waves do,
  only when the mushrooms run out. Once the last wave is called the field
  stays as it is, so the lone straggler behind a far mushroom is the finale
  rather than a stall.
- **The AI** plays as it plays Destroy: hunt the seats. One optional tactic is
  for the C2 commander (docs/enemy-command-and-control-prd.md) to send a
  guard to the mushroom nearest a seat. It is off by default and measured
  with the probe before it is turned on.
- **`RoundStats`** gains `mushrooms: (taken, total)` and `taken_by_seat`,
  shown on the end screen beside the wrecks.

## 5. Finding them: the count, the arrows, the minimap

- **The count** is the R pick (section 6). In Forage it takes the right
  cluster's enemy slot (`Corners`), and the enemy count moves down to the
  status line. It is laid out like every slot, so `corner_tests` pin it, and
  `every_language_fits_every_budget` measures its words.
- **Off-screen arrows** (`indicators.rs`): `ArrowKind::Mushroom`, drawn in
  the mushroom's current cap colour, for the nearest
  `indicator_mushroom_arrows` (3) mushrooms still out. They merge like enemy
  arrows and are never dropped for the last one.
- **Minimap** (`minimap.rs`): a 2x2-texel mark per mushroom still out,
  blinking in its cap colour, so the map at a glance shows what is left. A
  taken mushroom's mark is gone.
- **The frog's voice**: no change. The frog only speaks in training.

## 6. Presentation

**Art is chosen in the artifact.** Fill in this table from the copied line:

| Pick | Options | Chosen |
|---|---|---|
| Mushroom | M1 fly agaric, M2 king bolete, M3 troop, M4 glowcap, M5 parasol, M6 morel | _pending_ |
| Colour change | C1 rainbow sweep, C2 beacon blink, C3 spot chase, C4 glow breath, C5 confetti drift | _pending_ |
| Size | S1 48 px (a tree), S2 64 px (two cells), S3 96 px (three cells) | _pending_ |
| Pick-up show | P1 pop, P2 gulp, P3 spore puff | _pending_ |
| Count | R1 count readout, R2 pip row, R3 readout + toast | _pending_ |

All the candidates follow the rules the other sheets do, and so does the
chosen one:

- **The 2 px block grid**, a one-design-pixel `#252525` outline like the
  tanks, lit from the top left, three-quarter view like the trees (cap seen
  from a little above, stem below it). The art stands on its cell's centre
  and rises over the cells north of it. Like a tree, it is drawn in the
  y-sorted standing walk, so a tank north of it passes behind and one south
  of it in front.
- **A drop shadow** on the ground like an obstacle's, and with M4 or C4 a
  pool of light under it (`Game::draw_ground_light`). At night
  (`weather.rs`) every mushroom also throws a faint light in its cap colour
  (`Light::still`), so it can still be found in the dark.
- **Colour is the one exemption.** The caps' ramps are off the Puny palette,
  as the crates' symbol inks are (`PICKUP_INK`), because a mushroom has to be
  spotted from across the field. They are admitted by `check_sheets.py` on
  the mushroom sheet alone. The stem, spots and outline stay on the palette.
- **Colour steps, never blends.** Every mode moves between whole ramp steps
  on a clock (`Game::time` and a hash of the cell), and C5's change is a
  Bayer wipe, as every effect in the game is (docs/effects.md).

**The sheet.** `static/mushrooms_sheet.png` from
`tools/spritegen/gen_mushrooms.py` (raw PNG bytes, no Pillow, the crates'
convention), with one row per cap ramp (red, gold, teal, blue, magenta,
violet), so a colour change is a change of row rather than a tint. The
columns are: standing, two idle sway frames, squashed (P1), and the shrink
steps (P2) where picked. C3's spots and C4's brightened ramp are extra
columns. C5's dithered wipe draws two rows through a Bayer mask. The
artifact's renderer is the generator's first draft: the shapes are written
as functions of the cell, and the generator ports them.

**The shows** are composed at draw time in the effects language (`pyro.rs`),
started by `fx.rs` off `Event::MushroomTaken`: P1's ring and spores are
blocks in the cap's ramp, and P3's cloud is `pyro::Puff`s leaning with
`smoke_lean`, leaving a spore-ring decal. P2 flashes the hull through the
same tint path as a hit flash. None of them is simulation state.

**The HUD glyph** goes into `static/pickup_glyphs.png` as a new row, so the
count, the arrows and the minimap draw the same mushroom.

## 7. Map format, builder and linter

- **Map**: `[[cells]] kind = "mushroom"` on any dry, non-solid cell
  (`CellObject::Mushroom`). A cell that already holds a wall, prop, tree,
  tower, deep water or lava refuses it, and the builder tool won't paint
  there.
- **Builder**: `Tool::Mushroom` in the pickups category, multi-instance,
  following the toggle-erase rule. Its icon is the HUD glyph. FILL and
  SCATTER work as for any multi-instance tool, so a mushroom field is one
  stroke.
- **Linter** (`maplint.rs`): `forage-no-mushrooms` (error: a Forage map with
  none), `mushroom-unreachable` (error: outside every seat's nav component,
  checked with portals as the planner walks them, and the fix is
  `LintFix::Remove`), `mushroom-gated` (warning: reachable only through
  destructible walls, the `gated-pickup` rule) and `mushroom-on-start`
  (warning: within the collect box of a start).
- **Levels**: one new level in `levels.toml` once the art lands, on a field
  map, so the arrows and the minimap have something to do.

## 8. Online co-op

- **Wire**: a `u64` bitmask over the map's mushroom cells in their sorted
  order, `Snapshot::mushrooms`, the way pickups travel. `WireEvent::
  MushroomTaken` mirrors the event. Both are layout changes, so bump
  `PROTOCOL_VERSION`.
- **The replica** applies the bitmask and plays the show off the event. The
  colour clock runs on `Game::time`, which a replica already has, so every
  client cycles in step with no extra bytes.
- **Collection is the room's.** A client-owned hull (stage 3) that touches a
  mushroom does not take it locally; the room does, on its tick. The show
  waits for the event, about a round trip. If that reads late in `netlab`,
  the fix is a provisional show like provisional shots, keyed by cell and
  taken back if no event arrives within the refusal wait.
- **`hostable_maps`**: a Forage map is hostable like any shipped map.

## 9. Determinism

No RNG anywhere in the feature. Placement is the map's. Collection walks
seats in index order. The colour clock and every cosmetic choice are hashed
from the cell and `Game::time`. A map without mushrooms runs none of it, so
every existing replay, the probe fixtures and the pinned seat streams are
untouched. `determinism_tests` gains a Forage round.

## 10. Knobs

A `mushrooms` tuning group:

| Row | Default | Range | When |
|---|---|---|---|
| `mushroom_collect_pad_px` | 6 | 0..=32 | Live |
| `mushroom_cycle_seconds` | per the C pick | 0.1..=10 | Live |
| `mushroom_show_seconds` | 1.5 | 0.2..=4 | Live |
| `mushroom_light_reach_px` | 64 | 0..=256 | Live |
| `mushroom_heal` | 0 | 0..=100 | Live |
| `indicator_mushroom_arrows` | 3 | 0..=8 | Live (`indicators` group) |

## 11. Tests

- `mechanics_tests`: a seat touching the collect box takes the mushroom and
  one a cell away does not; two seats on the same frame give it to the lower
  seat; an enemy never takes one; a blast leaves it; the last one wins the
  round; every seat wrecked loses it; a frog's death loses a Forage map with
  a frog.
- `maplint`: each new kind, with a fixture in `maps/test/forage.toml`.
- `hud_tests`/`corner_tests`: the count slot at every window size, and the
  R3 toast centred.
- `net::apply` round-trip: the bitmask and the event, and a late join
  mid-round.
- `thumbnail`: the shipped Forage level's CPU render hash.
- `check_sheets.py`: the sheet's exemption covers the caps only.

## 12. Non-goals and open questions

- **Not in the first version**: mushrooms with effects (speed, a shield, a
  trip), poisonous decoys, enemies that collect, a timer mode. Each is a
  small follow-up once the base is played.
- **Open**: should a mushroom sway when a tank drives past, like tall grass?
  Cheap, and it makes them feel alive. Proposed yes, as idle frames.
- **Open**: does S3 hide too much? A 96 px mushroom covers a tank parked
  north of it. If S3 is picked, draw a tank behind it as an outline, as the
  trees would, or fade the mushroom while a hull is under its art.

## 13. Phases

1. **Rules**: rename the cloud module, `Mushroom`, `CellObject::Mushroom`,
   `Mission::Forage`, the event, win and loss, mechanics tests. Draw the
   mushrooms as placeholder blocks.
2. **Art**: `gen_mushrooms.py` from the picks, the sheet, the colour clock,
   the shadow and light, the show in `fx.rs`, the HUD glyph.
3. **Finding**: the count slot, the arrows, the minimap marks, the end
   screen stats.
4. **Builder and linter**: the tool, its icon, the lint kinds, the fixture.
5. **Online**: the bitmask, the event, the protocol bump, a `netlab` drive
   that collects.
6. **A level**: one Forage level in `levels.toml`, play-tested.
