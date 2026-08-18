# Wall / Obstacle Sprite Sheet — Integration Spec

All destructible obstacles in one sheet. Unlike the tank and shell sheets, these tiles are **outline-free and full-bleed** so they can be assembled into continuous walls and buildings.

---

## 1. File

| Property | Value |
|---|---|
| Filename | `walls_sheet.png` |
| Dimensions | 256 × 448 px |
| Grid | 8 columns × 14 rows |
| Cell size | 32 × 32 px (uniform, no padding) |
| Format | PNG, RGBA, straight (non-premultiplied) alpha |

Slice with `x = col * 32`, `y = row * 32`. Use **nearest-neighbour / point filtering**, no mipmaps, no compression.

### Tiling contract

- **No outline.** Nothing is ringed in black; tiles butt directly against each other with no seam.
- **Full bleed, square corners.** Every intact tile fills all 32 × 32 pixels including the corners. Nothing is rounded or inset.
- **All patterns have periods that divide 32**, so courses, ribs, boards, and rivets line up across tile joins in every direction. Brick bond alternation also survives vertical tiling (each tile contains an even number of courses).
- Tiles are **position-independent** — any tile can sit next to any other tile of the same variant and the pattern continues.

Damage holes are fully transparent and are given a **darkened inner edge** (a shaded tone of the material itself, not a black outline) so they read as depth without breaking the seamless join.

---

## 2. Sheet map

Column count varies by material. **Cells outside a material's range are empty** — never sample them.

| Rows | Material | Valid cols | States |
|---|---|---|---|
| 0–3 | **Brick** | 0–5 | intact + 5 decay |
| 4–7 | **Iron** | 0–3 | intact + 3 damage |
| 8–11 | **Wood** | 0–7 | intact + 2 damage + destroyed + 3 burning + charred |
| 12–13 | **Glass** | 0–3 | intact + 3 shatter |

```
row_base:  brick = 0,  iron = 4,  wood = 8,  glass = 12
row = row_base + variant_index
col = state
```

---

## 3. Brick — rows 0–3, cols 0–5

All four share one muted clay family (`#846152`–`#8C6758`) with a common mortar tone `#655A55`, differing only in **bond pattern and brick size**, so mixed-variant walls still read as one material. Mortar coverage is held near 30% across variants so none look greyer than the others.

These values sit in the same warm-brown range as the tank chassis (`#806E56`, `#705652`) — mean luminance ~105, saturation ~0.19, matching the tanks' own browns.

| Row | Type | Brick | Period | Pattern |
|---|---|---|---|---|
| 0 | `running` | 7 × 3 | 8 × 4 | Running bond |
| 1 | `block` | 15 × 7 | 16 × 8 | Running bond, large blocks |
| 2 | `long` | 15 × 3 | 16 × 4 | Long stretcher course |
| 3 | `stacked` | 7 × 7 | 8 × 8 | Stack bond (aligned grid) |

| Col | State |
|---|---|
| 0 | Intact |
| 1 | Chipped |
| 2 | Damaged — holes, cracks, craters |
| 3 | Breached — several bricks gone |
| 4 | Collapsing — edges eroding |
| 5 | Rubble — fragments only (destroyed / passable) |

Bricks are blown out with a ragged edge rather than a clean rectangle.

---

## 4. Iron — rows 4–7, cols 0–3

**Never destroyed** — the plate stays whole at every level, so the collider never changes.

All four share a steel tone (`#666D79`–`#6D7480`) taken from the tanks' slate and blue-grey chassis (`#60656D`, `#6D7584`), differing by surface treatment. Every treatment tiles. Measured saturation 0.085 — the closest match to the tank sheet of any material here.

| Row | Type | Surface | Period |
|---|---|---|---|
| 4 | `riveted` | Flat plate, rivets on a 16 px lattice | 16 × 16 |
| 5 | `corrugated` | Vertical ribbing | 4 × — |
| 6 | `banded` | Horizontal reinforcing bands with rivets | 8 × 16 |
| 7 | `tread` | Diamond tread plate | 8 × 8 |

| Col | State | Rust patches |
|---|---|---|
| 0 | Intact — **clean steel**, a couple of faint flecks only | 0 |
| 1 | Dented — first rust blooms, pocks, light scorch | 3 |
| 2 | Battered — deeper dents, spreading rust | 6 |
| 3 | Heavily beaten — dense rust and scorch, **no perforation** | 10 |

Damage is deformation only — no pixel is ever cleared, so an iron tile is always fully opaque and always tiles cleanly regardless of state.

---

## 5. Wood — rows 8–11, cols 0–7

All four share a muted oak tone (`#7F6B4F`–`#867256`) on an 8 px board period, in the same range as the tanks' khaki and brown chassis (`#8B8A60`, `#806E56`).

| Row | Type | Layout |
|---|---|---|
| 8 | `planks_h` | Horizontal boards |
| 9 | `planks_v` | Vertical boards |
| 10 | `stagger` | Horizontal boards with staggered butt joints |
| 11 | `palisade` | Vertical logs, rounded shading |

| Col | State |
|---|---|
| 0 | Intact |
| 1 | Damaged |
| 2 | Heavily damaged |
| 3 | Destroyed — splintered stubs, broken along the grain |
| 4 | **Burning frame 1** |
| 5 | **Burning frame 2** |
| 6 | **Burning frame 3** |
| 7 | Charred remains — burnt out, embers cooling |

**All four rows carry all eight states**, so any wood type can be wired to either behaviour — assign "breaks easily" and "catches fire" as gameplay data (hit points, flammability), not art.

### Fire

Columns 4–6 are a **3-frame loop** with flames baked in:

```
burning: 4, 5, 6, 4, 5, 6, ...   (~6–9 FPS reads as a good flicker)
```

Flame patches move and resize between frames over scorched boards. Fire uses the shell sheet's own palette — `#DC461E` / `#FF9628` / `#FFEC78` with `#C4602A` embers — so explosions and burning wood match.

Suggested lifecycle: `intact → damaged → heavily damaged → burning (loop) → charred`.

---

## 6. Glass — rows 12–13, cols 0–3

| Row | Type |
|---|---|
| 12 | `pane` — plain sheet |
| 13 | `reinforced` — wire-mesh safety glass (8 px mesh grid) |

| Col | State |
|---|---|
| 0 | Intact |
| 1 | Cracked — one impact star |
| 2 | Heavily cracked — multiple impacts, stress rings |
| 3 | Shattered — wedge shards fallen away, lit edges (passable) |

The only **semi-transparent** art in the sheet (body alpha 138), so terrain and units show through while still reading as a solid obstacle. Uses a desaturated blue-grey — `#547694` / `#7C9CB0` / `#B0CAD8` — pulled toward the tanks' blue-grey chassis (`#6D7584`) rather than the shells' vivid cyan, since glass is terrain and should not compete with effects.

Both the subtle glass ripple (4 px checker) and the diagonal sheen (16 px lattice) tile seamlessly — there is no corner-to-corner gradient, since a gradient can never tile.

The `reinforced` row uses a **wire mesh rather than a window frame**, specifically so it tiles into continuous glazing instead of showing a frame around every 32 px cell. The mesh survives shattering, leaving a wire grid with shards clinging to it — useful if you want it to remain a collider after the glass is gone.

---

## 7. Integration

```rust
pub const WALL_TILE: f32 = 32.0;

pub enum Material { Brick, Iron, Wood, Glass }

impl Material {
    pub fn row_base(self) -> i32 {
        match self {
            Material::Brick => 0,
            Material::Iron  => 4,
            Material::Wood  => 8,
            Material::Glass => 12,
        }
    }

    pub fn variants(self) -> i32 {
        match self { Material::Glass => 2, _ => 4 }
    }

    pub fn max_state(self) -> i32 {
        match self {
            Material::Brick => 5,
            Material::Iron  => 3,
            Material::Wood  => 7,
            Material::Glass => 3,
        }
    }

    pub fn is_destroyed(self, state: i32) -> bool {
        match self {
            Material::Brick => state >= 5,
            Material::Iron  => false,
            Material::Wood  => state == 3 || state == 7,
            Material::Glass => state >= 3,
        }
    }
}

fn wall_src(m: Material, variant: i32, state: i32) -> Rectangle {
    Rectangle::new(state as f32 * WALL_TILE,
                   (m.row_base() + variant) as f32 * WALL_TILE,
                   WALL_TILE, WALL_TILE)
}
```

Burning wood needs a frame timer:

```rust
burn_timer += dt;
if burn_timer >= 0.13 { burn_timer = 0.0; burn_frame = (burn_frame + 1) % 3; }
let state = 4 + burn_frame;
```

Notes:

- **Keep one variant per structure.** A building should use a single row for its walls so the pattern runs unbroken; switch variants between structures, not within one.
- Snap walls to a 32 px grid. Because tiles are full-bleed, off-grid placement will visibly break the pattern.
- Iron colliders never change. Other materials should drop or shrink their collider once `is_destroyed` returns true.
- Damage states are static; only wood's burn loop animates.
- Holes are transparent — draw walls **above** terrain and **below** units.

---

## 8. Palette

The whole sheet is tuned to sit with the tank sprites rather than beside them:

| Set | Mean luminance | Mean saturation |
|---|---|---|
| Tank sheet (reference) | 78 | 0.110 |
| Brick intact | 105 | 0.193 |
| Iron intact | 114 | 0.085 |
| Wood intact | 102 | 0.227 |
| **Walls overall** | **103** | **0.212** |

Walls land slightly above the tanks in value, deliberately: they are the backdrop, so keeping them a step lighter lets the darker tank silhouettes read on top. Brick and wood saturation sits where the tanks' own brown and khaki chassis sit (0.18–0.20), not higher.

**The one exception is fire.** Wood burn frames keep the shell sheet's vivid `#DC461E` / `#FF9628` / `#FFEC78`, well above the terrain range. That is intentional — fire is an effect, matches shell explosions, and should be the brightest thing on screen.

---

## 9. Known limitations

- **Damage patterns are baked per cell**, so two adjacent tiles at the same state show identical damage. Intact tiles tile perfectly; heavy-damage tiles placed in a long run will repeat visibly. Mitigate by mixing states along a wall, or ask and I can generate 2–3 alternate damage rolls per state.
- **No dedicated corner, end-cap, or T-junction pieces.** None are needed for flat runs — the patterns simply continue — but there is no special art for an outer corner of a building if you want the bond to wrap.
- No smoke plume, dust puff, or debris-particle art; burning frames carry only small in-tile smoke specks.
- Wood rows all carry burn frames; if only two are ever set alight, four columns per unused row are dead weight in the atlas.
- Glass relies on alpha blending; without blending it reads as flat pale blue.
- 22 of the 112 cells are intentionally empty (iron and glass are narrower than wood). Do not sample outside each material's valid column range.
- These tiles have no outline, unlike the tanks and shells. That is intentional for wall continuity, but it means an isolated single wall tile sits flatter against terrain than a unit does. If you want lone blocks to pop, add a drop shadow at the entity level rather than baking outlines back in.
