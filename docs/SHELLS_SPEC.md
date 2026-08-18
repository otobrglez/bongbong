# Shell Sprite Sheet — Integration Notes

Regenerated `shells.png`, matched to the current 12-tank sprite sheet.

---

## 1. What changed

| | Before | After |
|---|---|---|
| Dimensions | 224 × 96 | **224 × 576** |
| Grid | 7 cols × 3 rows | **7 cols × 18 rows** |
| Cell size | 32 × 32 | 32 × 32 *(unchanged)* |
| Variants | 3 (colour only) | 18 (colour × size-class × barrel-count × firing pattern) |

**The column layout is untouched.** All seven `ShellState` columns keep their existing meaning and index, so `ShellState::col()` needs no edit. Only the number of rows grew.

**`SHELL_TEXTURE_SIZE` stays 32.** Shells were resized *within* the cell rather than by changing the cell, so `source_rec()`, `SHELL_SCALE`, and all draw code work unchanged.

**Rows 0–2 keep their original meaning** (orange / red / blue, standard single-barrel), so existing `shell_variant` values 0, 1, and 2 remain valid and now simply render 10% smaller. Everything else is additive.

---

## 2. Row layout

Rows are grouped by class, then by colour, so `variant = class_base + colour`.

| Row | Colour | Class | Intended for |
|---|---|---|---|
| 0 | orange | standard, single | regular 1-gun tanks |
| 1 | red | standard, single | regular 1-gun tanks |
| 2 | blue | standard, single | regular 1-gun tanks |
| 3 | orange | standard, **twin** | regular 2-gun tanks |
| 4 | red | standard, **twin** | regular 2-gun tanks |
| 5 | blue | standard, **twin** | regular 2-gun tanks |
| 6 | orange | **super**, single | `leviathan` |
| 7 | red | **super**, single | `leviathan` |
| 8 | blue | **super**, single | `leviathan` |
| 9 | orange | **super**, twin | `titan` |
| 10 | red | **super**, twin | `titan` |
| 11 | blue | **super**, twin | `titan` |
| 12 | orange | standard, **twin staggered** | regular 2-gun tanks, alternating fire |
| 13 | red | standard, **twin staggered** | regular 2-gun tanks, alternating fire |
| 14 | blue | standard, **twin staggered** | regular 2-gun tanks, alternating fire |
| 15 | orange | **super**, **twin staggered** | `titan`, alternating fire |
| 16 | red | **super**, **twin staggered** | `titan`, alternating fire |
| 17 | blue | **super**, **twin staggered** | `titan`, alternating fire |

```
class_base: standard single    =  0    super single    =  6
            standard twin      =  3    super twin      =  9
            standard staggered = 12    super staggered = 15
colour:     orange = 0, red = 1, blue = 2
```

Column meanings are unchanged: `0 Fire0 · 1 Fire1 · 2 Fire2 · 3 Flying · 4 Hit0 · 5 Hit1 · 6 Hit2`.

---

## 3. Sizing

Standard shells were scaled to **0.9×** the previous art, matching the 10% reduction applied to the ten regular tank chassis. Super shells are **~1.25×** the new standard.

Measured max bbox per frame (original → standard → super):

| Frame | Original | Standard | Super | std/orig | super/std |
|---|---|---|---|---|---|
| Fire0 | 29 | 25 | 31 | 0.86 | 1.24 |
| Fire1 | 23 | 21 | 25 | 0.91 | 1.19 |
| Fire2 | 22 | 19 | 24 | 0.86 | 1.26 |
| Flying | 18 | 16 | 20 | 0.89 | 1.25 |
| Hit0 | 23 | 21 | 27 | 0.91 | 1.29 |
| Hit1 | 30 | 25 | 31 | 0.83 | 1.24 |
| Hit2 | 31 | 28 | 31 | 0.90 | **1.11** |

**Known cap:** the original `Hit2` smoke cloud already filled the 32 px cell (31 × 31), so the super variant's smoke could not grow proportionally — it is capped at ~1.11× instead of 1.25×. Every other frame, including the projectile itself, scales correctly. If a fully proportional super smoke cloud matters, `SHELL_TEXTURE_SIZE` would need to rise to 48 and the whole sheet be re-exported.

### Projectile alignment

Twin projectiles are positioned to match the tanks' actual barrel spacing:

| Class | Shell projectile columns | Tank barrel columns |
|---|---|---|
| standard single | x14–17 | x15–16 |
| standard twin | x11–13, x17–19 | x12–13, x18–19 |
| super single | x13–18 | x14–17 (`leviathan`) |
| super twin | x9–12, x19–22 | x9–12, x19–22 (`titan`) |

---

## 4. Twin variants — important

The twin rows are **art only**. They depict two projectiles and a double muzzle flash so a 2-gun tank reads correctly, but the game still spawns **one** `Shell` entity per shot. No change to `Shell::spawn`, physics, or damage is required — a twin-barrel tank fires one shell whose sprite shows both barrels discharging.

If you later want genuinely independent per-barrel shells, that is a gameplay change (two `Shell`s with lateral spawn offsets), and you would then use the *single* rows for each, not the twin rows.

The `Hit*` frames for the simultaneous twin variants render a single combined burst — slightly wider, with a twin core — on the assumption both rounds land together.

### Staggered twin (rows 12–17)

Same two-barrel idea, but the **left barrel fires first** and its round runs ahead of the right one. The lead offset is 4 px on standard, 5 px on super, measured along the travel axis.

How the delay reads across the frames:

| Frame | Staggered behaviour |
|---|---|
| `Fire0` | Lead barrel already blooming into a full blast; right barrel only just igniting (small flash, offset back) |
| `Fire1` | Lead round is clear and ahead with a fading flash; trailing barrel produces the big blast |
| `Fire2` | Both rounds away, lead further ahead; only a spark left at the lead muzzle |
| `Flying` | The clearest read — two projectiles, one visibly ahead of the other |
| `Hit0` | Lead round detonates first (large burst); trailing round is still a small burst behind it |
| `Hit1` | Both bursting, cores still offset, smoke elongated along the travel axis |
| `Hit2` | Merged elongated cloud with a hot lead core and a cooler trailing one |

Because the offset runs along the travel axis, it stays correct at every rotation — the lead round always reads as further downrange.

This is still **one `Shell` entity**; the stagger is depicted, not simulated. Use these rows in place of the simultaneous twin rows (3–5 / 9–11) for a punchier, less mechanical firing feel, or alternate between the two sets on successive shots.

---

## 5. Style preserved

Regenerated procedurally against the original's measured geometry, not resampled. Palette is curated from `tools/punypalette.py` (see `docs/PALETTE.md`) — colours sampled directly from the third-party Puny World ground-layer tileset. Every pixel in `shells.png` is one of that fixed set, no exceptions. This is the second recolor pass for this sheet — an earlier one used [Resurrect 64](https://lospec.com/palette-list/resurrect-64) (`tools/spritegen/_backup/pre-punypalette-*/gen_shells.py`); shells needed the least adjustment of any sheet in that switch, since fire reads as fire in either palette — mostly a matter of picking the nearest equivalent step:

- Palette per colour family (dark / mid / core / smoke): orange = smoke `#5D654F`, dark `#E44219`, mid `#EEA343`, core `#CAC594`; red = smoke `#4C523C`, dark `#9C3527`, mid `#FF421A`, core `#DC9C4A`; blue = smoke `#ACB7A1`, dark `#038AAB`, mid `#27D8C5`, core `#FFFFFF` (a white spark core — Puny World has nothing brighter than actual white either).
- Shared casing colours across all rows: dark `#4C523C`, highlight `#ACB7A1`, bronze-gold nose `#DE9943`.
- Same construction: layered concentric discs, four cardinal 1 px rays drawn *on top* of the disc, white sparkle diamond in the `Fire0` core, four mini-rings inside `Hit1`, diamond ember in `Hit2`, and alpha-blended smoke (dense on `Hit1`, light on `Hit2`) with a few scattered specks at the edge.

---

## 6. Rust constants

```rust
/// Rows in shells.png.
pub const SHELL_VARIANTS: i32 = 18;

/// Distance from a tank's centre to its muzzle tip, in 32px sprite units.
/// Taken from the turret bounding boxes published in SPRITESHEET_SPEC.md
/// (offset = 16 - turret_bbox_y0). Multiply by `tank.scale` at use site,
/// exactly as `Shell::spawn` already does.
pub const TANK_MUZZLE_FORWARD_OFFSET_BY_ROW: [f32; 12] = [
    14.0, // 0  scout
    13.0, // 1  assault
    12.0, // 2  breaker
    16.0, // 3  longbow
    10.0, // 4  flak
    13.0, // 5  wraith
    14.0, // 6  warden
    14.0, // 7  ravager
    14.0, // 8  glacier
    16.0, // 9  obelisk
    16.0, // 10 titan
    16.0, // 11 leviathan
];

/// Suggested shell row per tank row: picks the size class from the chassis,
/// the barrel count from the turret, and the colour from the tank's accent.
pub const TANK_SHELL_VARIANT_BY_ROW: [i32; 12] = [
    2,  // 0  scout      std  single, cyan   -> blue
    3,  // 1  assault    std  twin,   amber  -> orange
    1,  // 2  breaker    std  single, red    -> red
    2,  // 3  longbow    std  single, green  -> blue
    5,  // 4  flak       std  twin,   cyan   -> blue
    2,  // 5  wraith     std  single, purple -> blue
    2,  // 6  warden     std  single, cyan   -> blue
    3,  // 7  ravager    std  twin,   amber  -> orange
    2,  // 8  glacier    std  single, blue   -> blue
    4,  // 9  obelisk    std  twin,   red    -> red
    10, // 10 titan      SUPER twin,  red    -> red
    8,  // 11 leviathan  SUPER single, cyan  -> blue
];

/// Optional: staggered counterparts for the 2-gun tanks, for alternating fire.
/// Rows not listed here have no twin variant and are unchanged.
pub const TANK_SHELL_VARIANT_STAGGERED_BY_ROW: [i32; 12] = [
    2, 12, 1, 2, 14, 2, 2, 12, 2, 13, 16, 8,
];
```

To alternate, flip between the two tables on each shot from a 2-gun tank:

```rust
let variant = if self.alternate_shot {
    TANK_SHELL_VARIANT_STAGGERED_BY_ROW[row as usize]
} else {
    TANK_SHELL_VARIANT_BY_ROW[row as usize]
};
self.alternate_shot = !self.alternate_shot;
```

Assign at tank construction:

```rust
shell_variant: TANK_SHELL_VARIANT_BY_ROW[row as usize],
```

The colour column is a suggestion only — the class (size + barrel count) is what must match the chassis; the colour is free. Note that the green and purple accents (`longbow`, `wraith`) have no exact shell family, so they fall back to blue. Say the word if you want green and purple families added — that would take the sheet to 20 rows using the same `class_base + colour` scheme.

---

## 7. Not changed

- `ShellState`, its column indices, and all durations.
- `source_rec()`, `draw_shell()`, `draw_shell_shadow()`.
- `SHELL_TEXTURE_SIZE`, `SHELL_SCALE`, shadow constants.
- Shell physics, ownership, and collision.

The only required code edit is `SHELL_VARIANTS: 3 -> 18`, plus updating the two lookup tables above if the tank roster changed rows.
