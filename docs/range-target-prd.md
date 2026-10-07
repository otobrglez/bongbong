# The range board

A target to shoot at: a bullseye on a stand, placed on a map so a player
can see what each weapon does. Linear: BB-44. Research: the range-target
proposal (four designs - a steel pop-up plate, this board, a plywood tank
decoy, a scrap-hull dummy - compared on art, mechanics and cost;
<https://claude.ai/artifact/WPDaCAa8qPcXbMJf7yyz1R>). This is design B.

## 1. Why

Nothing on the field exists only to be shot. Showing a weapon - in Boot
Camp, a play-test, a PR preview, a conference demo - means waiting for an
enemy to roll in and then watching it fight back. A board placed on a map
shows how hard a weapon hits (how many shots it takes), how wide it spreads
(how many boards a burst or a blast touches) and what it sets alight.

## 2. What a player sees

One cell: a round board on an easel - two splayed legs, a back leg and a
crossbar - a gold centre in a red and a white ring, a dark wooden rim lit
from the upper left and the board's thickness under it. On the Puny
palette, in the props' 2 px blocks (`gen_props.py`), with the obstacle
drop shadow. It is drawn 30 % larger than a prop so it reads across the
field: the face is a whole cell (32 px) wide and the sprite 36 px tall,
from its own sheet's 44 px cells (`target_sheet.png`), standing a block
above its cell with its feet a block below it the way a tree's canopy
overhangs - while the board still occupies exactly one cell (its
collider, the nav grid, the map).

- **Shot** - by a shell, a bullet, a plasma bolt, the laser, a grenade's
  or a missile's blast, or a ram - it takes the hit like any tile (the hit
  flash, a burst of sawdust) and wears through four stages: intact, holed,
  cracked, splintered (a quarter of the board gone). On its last point it
  breaks into wood rubble. `target_max_health` 100 is three or four
  player shells (a shell lands 25 to 42 on a tile), so the stages show on
  the way.
- **Caught by fire** it burns, whatever its health, and leaves charred
  rubble (section 3).

A shot never sets it alight: no board is rolled flammable, so a shot that
finishes it splinters it, and only fire burns it.

## 3. Fire

Fire is the one thing a board does besides breaking, so it gets its own
animation. Every part is drawn in the effects language (docs/effects.md):
whole 2 px blocks, ramp steps, no RNG.

1. **Scorching.** Under the flamethrower's stream, before it catches, soot
   creeps in from the rim as the board's heat rises toward
   `flame_ignite_seconds`, dithered through the field's Bayer pattern
   (`target::fire_shapes`). The gold centre still shows as it catches.
2. **Catching.** It lights when its heat reaches `flame_ignite_seconds`
   (`Event::Ignited { what: "target" }`), when a burning ground cell - an
   oil pool, a lit trail, the flamethrower's ground fire, a lava bomb's
   pool - is in or beside its cell (`Game::tick_fires`), or when a
   volcano's surge sets the banks alight. These are the paths that light
   flammable timber, opened to the board by `Material::catches_fire`.
3. **Burning**, for `target_burn_seconds` (3 s; a plank's
   `wood_burn_seconds` is 1 s, too short for the char to read):
   - the board **chars in order** through the sheet's three burn columns -
     the rings scorching under a lit rim, the board blackened with the
     rings glowing through, the charred frame - by how far the fire has
     got (`Obstacle::col`, `burn_progress`), never flickering back;
   - **embers** pulse over the char, each on a spot and a cadence hashed
     from the board, more of them and hotter as the fire spreads, fewer and
     duller as it chars out;
   - the **flames** standing on it are every burning tile's
     (`game::tile_flames`), catching over the first moments and dying down
     over the end of the board's own burn time;
   - the smoke, the embers in the air and its light at night come from
     the particle layer and the weather's lights, as for any burning tile.

   A burning board is already lost: what hits it changes nothing.
4. **Burning out.** In the last fifth the stand gives way and the board
   sinks one block (`Obstacle::burn_sag`); then it is charred wood rubble
   and the ash cloud a burnt-out tile leaves.
5. **Online.** A replica is told only that the board burns. The char, the
   embers and the flames run on `Obstacle::burn_shown`, which
   `tick_burn_frame` advances on every client between snapshots, so a
   co-op round shows the same burn. The scorch reads the board's heat,
   which only the simulating round has, so a replica shows the board
   catching without the soot first.

## 4. Rules

- `Material::Target`, appended to the enum (the wall block's indices are
  untouched). A prop drawn from its own `target_sheet.png` (44 px cells;
  docs/PROPS_SPEC.md), one variant, four visible stages, seam-closed like a prop, blocks sight
  like a wall, not permanent, no pass-over or deflection chance, no ram
  collapse - a tank stops against it and its ram damage wears it down.
- `CellObject::Target`, `kind = "target"` in a map.
- Tuning, group `props`: `target_max_health` (spawn) and
  `target_burn_seconds`.
- The builder: the `target` tool (`range target`, `tarča`) in the ACTOR
  list beside the frogs - the thing on the field there to be shot at - and
  a prop's grey on the minimap.
- The wire: `Material` and `IgnitedWhat` gained a variant, so
  `PROTOCOL_VERSION` moved on. A board's state travels as any tile's does.
- The AI treats a board as any destructible tile: something in the way,
  breached when it blocks a route.
- `maps/range.toml` is a weapons range: lanes of boards at 4, 8 and 12
  cells, an oil drum feeding a trail into a board, three boards round a
  fuel drum and a crate of every weapon. Not shipped.

## 5. Not in this change

The steel pop-up plate that springs back up (design A), floating damage
numbers, and the scrap-hull dummy that missiles lock onto (design D).

## 6. Tests

- `props_tests`: shells wear a board through its stages and splinter it,
  never alight; the flamethrower scorches it, lights it at
  `flame_ignite_seconds` and it burns out after `target_burn_seconds` as
  charred rubble; a fire running down an oil trail lights a board beside
  it.
- `target::tests`: the scorch creeps in from the rim, on the block grid in
  char steps; the embers stay on the face in fire steps and pulse; the
  burn chars in order and sags in its last fifth on `tick_burn_frame`
  alone; the same board draws the same picture.
