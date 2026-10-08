# Crates spec — the pickups' supply crates

Every pickup is a wooden supply crate with the symbol of what is inside
sprayed on its lid: the **Stencil** line of the Quartermaster study (the
four-line design study the line was picked from, with Air drop, Crack open
and Blasts and fire only). This is the integration spec: the two sheets,
the drawing, the shows a crate puts on and the breakable crates behind
`crate_breakable`.

## 1. Files

| File | Size | Grid | Drawn at |
|---|---|---|---|
| `static/crates_sheet.png` | 280 × 640 | 7 cols × 16 rows of 40 px | 1:1, centred on the pickup's 32 px cell (`CRATE_CELL`) |
| `static/pickup_glyphs.png` | 24 × 384 | 1 col × 16 rows of 24 px | 1:1 on the field; 24 pt in the HUD and the builder's bar |

Both are written by `tools/spritegen/gen_crates.py` (raw PNG bytes, no
Pillow, deterministic - every choice is a position hash):

```
python3 tools/spritegen/gen_crates.py
just check-sheets
```

Never edit them by hand. The rows are `PickupKind` in declaration order
(`PickupKind::row`, the generator's `KINDS`): health, ammo, laser, minigun,
plasma, missiles, speedup, shield, flamethrower, frog_health, tower_pack,
heat_shield, grenades, sonic_hammer, emp_burst, gauss_rail, fpv_swarm,
rod_from_god.

### Columns of `crates_sheet.png`

| Col | What | Constant |
|---|---|---|
| 0 | the crate | `CRATE_COL_INTACT` |
| 1–4 | a glint sweeping the lid, a two-design-pixel band one ramp step lighter, top left to bottom right | `CRATE_COL_GLINT`, `CRATE_GLINT_FRAMES` |
| 5 | damaged: a split plank, a cracked batten, chipped paint | `CRATE_COL_DAMAGED` |
| 6 | charred: every board two steps darker, the paint scorched | `CRATE_COL_CHARRED` |

## 2. The art

- **Density**: a 40 px cell is 20 × 20 design pixels, each a 2 × 2 block,
  like the walls and props; the crate is 18 × 18 of them (36 px), three
  planks between two battens, standing 2 px past its 32 px map cell on every
  side - big enough to read across the field beside the tanks, still inside
  what a hull collects from (`PICKUP_SIZE` grown by `pickup_collect_pad_px`).
  Two crates on neighbouring cells stand edge to edge like stacked stores.
- **Perspective**: straight down with a two-row front face as the tilt cue
  (the props' `front_strip`), lit from the top left, the tanks' `#252525`
  outline. The runtime drop shadow is an obstacle's
  (`obstacle_shadow_offset`, `obstacle_shadow_opacity`).
- **Wood**: dark-stained planks (`WOOD_DK`, `WOOD_DEEPER`, `WOOD_DARKEST`
  seams) between two lighter side battens (`WOOD_LT`, `WOOD_MD`,
  `WOOD_AMBER`), nails in `STONE_MID`, a hashed grain speckle a step
  darker. Dark on purpose: honey wood (`#DE9943`) is the wood walls' and
  fences', and a crate must never read as a wall.
- **The symbol**: 10 × 10 design pixels between the battens, a plank's
  width clear of them on either side, flat paint in
  the kind's base ink with a hashed chip or two gone from its edge - never
  a hole inside it, which turns a solid symbol like the cross into a blot.
  The shield's is the rainbow swept corner to corner.
- **Inks** (`punypalette.PICKUP_INK`, `PickupKind::ink` - shade, base,
  light): off the palette on purpose, the old pickup icons' exemption kept
  for the one part that has to be spotted from across the field; the bases
  are the HUD's weapon colours where the HUD has one. `check_sheets.py`
  admits them on these two sheets alone and holds the crate sheet to the
  no-green rule.

| Kind | Symbol | Base ink |
|---|---|---|
| health | a cross | `#E84A3C` |
| ammo | two shells | `#F2C84B` |
| laser | a beam with a flare | `#FF4FA8` |
| minigun | a barrel cluster | `#C9D6DE` |
| plasma | an orb with sparks | `#3FE0CC` |
| missiles | a rocket | `#B6E848` |
| speedup | a lightning bolt | `#FFD93D` |
| shield | a shield, rainbow | `#A77BFF` |
| flamethrower | a flame | `#FF8A2B` |
| frog_health | a frog from above | `#7EDB5A` |
| tower_pack | a spanner | `#8FB0FF` |
| heat_shield | a shield, molten red over black basalt (docs/volcano.md) | `#F0461E` |
| grenades | four grenades, each with its lamp lit in the light ink (`#F4B6FF`; white on the symbol sheet) | `#D656F5` |
| sonic_hammer | a speaker's cone and two arcs of sound (docs/sonic-hammer.md) | `#46C3F2` |
| emp_burst | the power-off sign: a broken ring and its bar, the bar in the light ink (docs/emp-burst.md) | `#4F6BFF` |
| gauss_rail | two rails, the slug's trail between them and its white-hot head leaving their mouth in the light ink (docs/gauss-rail.md) | `#FF3DD8` |
| fpv_swarm | a quadcopter from above, two-tone like the heat shield: an ivory frame with its rotor hubs and body lit crimson in the light ink - its lamps (docs/fpv-swarm.md) | `#FFF0C8` |
| rod_from_god | a tungsten rod falling point first into a reticle's four corner brackets, two-tone like the swarm: the rod a dark steel grey lit white along its top, its tip and the brackets in the light ink - the designator's red (docs/rod-from-god.md) | `#8A9099` |

The symbol sheet is each symbol on its own, lit along its top, shaded along
its bottom and outlined: what rises out of an opened crate, what a broken
one spills, and the HUD's symbols (§3).

## 3. Drawing

`pickup::draw_pickup(c, pickup, time, shadows)` is generic over `Canvas`,
so the game, the builder's canvas and `mapshot` all draw crates the same.
It draws, in order of precedence:

1. **Loose contents** (`Pickup::loose`): the symbol on the ground with its
   shadow, blinking out over its last 1.5 s.
2. **A burning crate** (`Pickup::burn`): the charred column and flames off
   its lid (`pickup::crate_flames`, `pyro::tongues`); their light is drawn
   with the burning tiles' in the lit pass's additive block.
3. **A hurt crate** (`health` under half of `crate_hp`): the damaged column.
4. **The air drop** while `crate_fx::drop` has a frame for its age.
5. **A standing crate**: its shadow, then the crate in the glint's column
   for the round clock (`crate_fx::glint_col`), each crate on its own
   hashed beat.

The builder draws the crate's column 0 for a pickup cell at its own size,
and a pickup tool's icon (`draw_tool_icon`) at a whole multiple of a sheet's
scale centred on the icon, so every block stays: the crate where it fits
(the list and the palette, every touch slot), the symbol alone in the bar's
32 pt category button.

**The HUD** (`render::hud`) labels every readout with the symbol of the crate
that fills it, at the symbol sheet's own 24 pt: the vitals' health (the
cross) and what the trigger fires - the special weapon carried
(`hud::weapon_pickup`), else shells (ammo) -, the speed and shield gauges
(the bolt and the shield) and the frog's gauge in the right cluster (the
frog pack). A symbol whose readout is empty - no shells, no boost, no
shield, no frog - is drawn dim (`SYMBOL_UNLIT`), so the row says at a glance
what is running. The gauges carry no words.

## 4. The shows

All in the effects language (docs/effects.md): whole 2 px blocks, ramp
steps, hashed and never rolled, pure functions of age (`crate_fx.rs`, with
tests).

- **Air drop** (`crate_fx::drop`): a crate that comes down mid-round
  (`Pickup::dropped_at`, the round clock) - a shadow gathers, the crate
  falls into it drawn larger while it is high, lands with a squash and a
  stretch and throws a ring of dust. **Drawn only**: the crate can be taken
  the frame it appears, so nothing waits on the drop and no replay moves.
  The crates `init` places stand there from the start. A replica stamps a
  crate it has not seen with the snapshot's tick (`net::apply`), so a room's
  drop plays on every client and a welcome's crates stand.
- **Crack open** (`crate_fx::open`, `Opening::Taken`): when a tank takes a
  crate - the crate flashes and splits into planks that tumble off, a puff
  of wood dust, the symbol rises, blinks and drops into the tank that took
  it, and a ring goes round that tank. Started by `Event::PickupCollected`
  (which carries the crate's position) on the particle layer's clock
  (`fx::CrateOpen`) and painted over the tanks in the glowing pass, since
  the hull that takes a crate is over it the frame it does.
- **Glint**: the idle, the sheet's four glint frames now and then.

Knobs (the `crates` tuning group): `crate_drop_seconds`,
`crate_drop_height_px`, `crate_open_seconds`, `crate_glint_period_seconds`,
`crate_glint_frame_seconds`; 0 turns the first, third and fourth off.

## 5. Breakable crates (`crate_breakable`, off by default)

`simulation/crates.rs`. Read once by `init` into `Game::crates_breakable`, so
a round keeps the rule it began under and a test turns it on for its own
round. Off, nothing touches a crate and every round replays as before.

- **What reaches a crate** is what reaches the ground: a drum's blast, a
  missile's burst, a dying tank's blast, another crate cooking off
  (`blast_crates`: the blast's mid damage times its falloff, no roll), and
  fire - `crate_ignite_seconds` of the flamethrower's heat on its cell
  (`Game::heat`) or a burning ground cell under it lights it, and it falls
  in `crate_burn_seconds` later. **Shells and bullets still fly over** a
  crate: making it a target would change every firefight, the AI's line of
  sight and the client's prediction.
- **How it breaks** (`PickupKind::cooks_off`): ordnance (ammo, minigun,
  missiles, flamethrower) and energy (laser, plasma) **cook off** - a blast
  of an oil drum's scaled by `crate_cookoff_scale`, on both sides, the
  frogs, the tiles and the other crates, the flamethrower's fuel leaving a
  pool of fire; the crate is gone. Health, shield, speed and the frog and
  tower packs **spill**: the pickup lies loose on its cell for
  `crate_spill_seconds`, taken like any other, then goes and the slot
  refills as usual.
- **Order**: breaks go on the frame's explosion worklist with the kills and
  the drums (`Frame::crate_breaks`, sorted top to bottom, left to right),
  so a cook-off that breaks another crate breaks it the same frame. Only a
  cook-off draws RNG (its damage rolls, a drum's order).
- **Events**: `Event::CrateBroken { kind, x, y, cooked }` - the planks
  (`Opening::Broken`) and wood dust on the particle layer, and on a replica
  the cook-off's show (`Game::crate_cookoff_show`). `PickupCollected`'s
  `spilled` says the pickup was lying loose (`Opening::Spilled`: the symbol
  flies, no crate splits).
- **The wire**: `Snapshot::crates`, a keyed family of `CrateState` (cell,
  whole hit points, `crate_flags` BURNING/LOOSE, the time left in tenths)
  for the crates that are not whole only; `DrawablePickup` holds the same in
  the replica's picture.

Defaults: `crate_hp` 8 - an oil drum beside a crate breaks it, a missile
landing on it does, a crate cooking off beside it does, a dying tank's blast
alone does not. Tests: `simulation/crate_tests.rs` and
`net::apply::tests::broken_crates_are_the_same_on_the_replica`.
