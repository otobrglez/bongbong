# Sci-Fi Tank Sprite Sheet — Integration Spec

This document describes `scifi_tanks_sheet.png` for engine integration. It is written to be self-contained: an implementer should not need to inspect the image to slice and use it correctly.

---

## 1. File overview

| Property | Value |
|---|---|
| Filename | `scifi_tanks_sheet.png` |
| Dimensions | 416 × 384 px |
| Grid | 13 columns × 12 rows |
| Cell size | 32 × 32 px (uniform, no padding, no margin, no gutters) |
| Format | PNG, RGBA, straight (non-premultiplied) alpha |
| Background | Fully transparent (alpha = 0) |
| Art style | Top-down pixel art, hard 1 px near-black outline `#252525` (Puny Palette's darkest tone, sampled from the Puny World tileset's own shadow pixels — that pack has no true black either) |

**Orientation:** All sprites face **up / north (−Y in screen space)**. Barrels point toward the top of the cell. If your engine treats 0° as "east/right", apply a **−90° offset** when converting an aim angle to sprite rotation.

**Rendering:** Use **nearest-neighbour / point filtering**. Disable mipmaps, anti-aliasing, and texture compression. Bilinear filtering will blur the outline and bleed neighbouring cells.

**Atlas padding:** Cells are directly adjacent with no bleed margin. If repacking, add 1–2 px extrusion, or restrict rendering to integer zoom levels.

---

## 2. Slicing

```
x = col * 32
y = row * 32
w = 32
h = 32
```

- `col` ranges 0–12 (see §3)
- `row` ranges 0–11 (see §4)

---

## 3. Column layout

| Col | Contents | Notes |
|---|---|---|
| 0 | Hull — track frame 0 | Default/idle chassis. **No turret attached.** |
| 1 | Turret | Rotating turret with barrel(s) |
| 2 | Hull — track frame 1 | Movement animation |
| 3 | Hull — track frame 2 | Movement animation |
| 4 | Hull — track frame 3 | Movement animation |
| 5 | Broken turret | Destroyed turret, barrel severed |
| 6 | Hull — **lightly damaged** | Cosmetic damage, still operational |
| 7 | Hull — **disabled** | Immobilized, structurally intact |
| 8 | Hull — **wreck A** | Turret ring blown out |
| 9 | Hull — **wreck B** | One track run torn away |
| 10 | Hull — **wreck C** | Burnt-out husk |
| 11 | Hull — **wreck D** | Catastrophic / ammo cook-off |
| 12 | Track marks | Ground decal (semi-transparent) |

Columns 0, 2, 3, 4 are the **same hull** with only the tread pattern shifted. Column 0 is both the idle pose and animation frame 0.

Columns 8–11 are four **interchangeable** destroyed variants of equal severity. Pick one at random per destroyed unit to avoid visual repetition across a battlefield — they are not a sequence.

---

## 4. Row roster

12 tanks. Rows 0–9 are standard chassis; rows 10–11 are **super-heavy** chassis, roughly 21% larger in linear dimension than the standard baseline (about 34% larger than the current standard tanks).

Body/accent are curated picks from the Puny Palette (see `tools/punypalette.py`, `docs/PALETTE.md`) — colours sampled directly from the third-party Puny World ground-layer tileset, not an abstract pixel-art palette. Every pixel in the sheet, including every shading step `gen_tanks.py` derives from these two colours, snaps onto that same set. This is the second recolor pass for this roster: an earlier one used [Resurrect 64](https://lospec.com/palette-list/resurrect-64) (see `tools/spritegen/_backup/pre-punypalette-*/gen_tanks.py`) and looked great in isolation, but once the ground layer shipped (`docs/GROUND_SPEC.md`) those candy-vivid R64 colours read as neon plastic next to Puny World's much softer terrain — see `docs/PALETTE.md`'s "why the palette changed a second time" for the full reasoning. Puny World's own art has no purple/violet anywhere in it; `wraith` was reassigned from a purple accent to a hue family the source art actually has.

**Every body colour is picked from the palette's bright/mid steps, never its darkest ones** (`*_DARKEST`/`*_DEEPER` in `tools/punypalette.py`) — a first cut of this pass gave the back half of the roster, especially the two super-heavy chassis, the darkest available step of their family on the reasoning "heavier/stealthier = darker." That read as muddy and drab, the same mistake the R64 muted-pass paragraph above already covers, just rediscovered on a different palette: the body is most of a tank's on-screen area, so a dark body reads as a dark *tank* no matter how bright the rest of the scene is.

| Row | Name | Chassis | Guns | Turret | Accent | Body | Role hint |
|---|---|---|---|---|---|---|---|
| 0 | `scout` | narrow | 1 thin | round | Gold `#EEA343` | Grass green `#85A643` | Fast recon |
| 1 | `assault` | standard | 2 | box | Teal `#00A67F` | Honey wood `#DE9943` | General purpose |
| 2 | `breaker` | wide | 1 heavy | hex | Warm gold `#DC9C4A` | Roof-tile red `#9C3527` | Heavy brawler |
| 3 | `longbow` | long | 1 long | round | Bright red `#FF421A` | Forest green `#5F914B` | Artillery / sniper |
| 4 | `flak` | compact | 2 short | hex | Pale gold `#CAC594` | Water blue `#04A0B4` | Anti-air / close range |
| 5 | `wraith` | narrow | 1 | wedge | Pale stone `#DAE5CE` | Bright teal `#00BB8F` | Stealth |
| 6 | `warden` | standard | 1 heavy | hex | Warm gold `#DC9C4A` | Olive green `#7C983C` | Support / defense |
| 7 | `ravager` | wide | 2 | round | Roof-tile red-orange `#E44219` | Amber wood `#CA8A3B` | Heavy assault |
| 8 | `glacier` | compact | 1 | box | Pale stone `#DAE5CE` | Bright water-teal `#1EB3AE` | Balanced |
| 9 | `obelisk` | long | 2 long | wedge | Gold `#EEA343` | Dark roof-tile red `#812F27` | Siege |
| 10 | `titan` | **super-heavy** | 2 heavy (4 px) | hex | White `#FFFFFF` | Vivid red-orange `#FF421A` | Super-heavy assault |
| 11 | `leviathan` | **super-long** | 1 massive (4 px) | round | Bright water-teal `#27D8C5` | Bright teal `#00D097` | Super-heavy siege |

Names are reference labels only; no text is baked into the art.

---

## 5. Pivot / anchor — the critical part

**Every sprite in this sheet uses the same pivot: the exact center of the 32×32 cell, at pixel `(16, 16)` — normalized `(0.5, 0.5)`.**

| Engine | Setting |
|---|---|
| Unity | Sprite Editor → Pivot = **Center** (or Custom `0.5, 0.5`) |
| Godot | `Sprite2D` default centered `offset` (`centered = true`) |
| GameMaker | Sprite origin = **Middle Centre** (16, 16) |
| Phaser | `setOrigin(0.5, 0.5)` (default) |
| Raw / LibGDX | Rotate about `(16, 16)` in local sprite space |

### Why this works

- Every **hull** is centered on the grid and carries a recessed **turret mount ring** at that exact point.
- Every **turret** is drawn around its **turret ring center** — the physical mounting point — not around its bounding box or barrel.

Draw the turret at the **same world position** as the hull, both pivots centered, and rotate the turret freely. It stays seated at every angle with no offset math.

**Do not derive the pivot from the bounding box.** Bounding boxes differ between cells (a broken turret is shorter than an intact one; a wrecked hull has chunks missing). The pivot is always the cell center regardless.

### Composition order

```
1. ground decals (col 12)   — below everything
2. hull (col 0/2/3/4, or a damage column)
3. turret (col 1, or col 5 if destroyed)
4. muzzle flash / FX        — not included
```

---

## 6. Track animation

Columns 0 → 2 → 3 → 4 form a **seamless 4-frame loop**; the tread pattern scrolls 1 px per frame on a 4 px period.

```
Forward:   0, 2, 3, 4, 0, 2, 3, 4, ...
Reverse:   0, 4, 3, 2, 0, 4, 3, 2, ...
Stationary: hold column 0
```

- Suggested rate: 8–12 FPS at normal speed.
- **Preferred:** advance by distance travelled rather than a fixed timer, so tracks appear to grip the ground.
- All four frames are verified pixel-distinct for every row.
- **The turret is unaffected.** Only the hull cell changes.
- Damage columns (6–11) have **no** animation frames — hold a single frame.

---

## 7. Damage states

Six hull states plus one turret state, forming a severity ladder.

| Col | State | Appearance | Suggested meaning |
|---|---|---|---|
| 6 | **Light** | Scuffed paint, small impact marks, one thruster dark. Accents still lit, tracks intact. | ~60–99% HP — still fully mobile |
| 7 | **Disabled** | Scorched plating, one track gouged, thrusters dead with one ember. | ~1–35% HP, or immobilized |
| 8 | **Wreck A** | Turret ring blown out, both track runs shredded, embers in the breach. | Destroyed |
| 9 | **Wreck B** | Entire left track run torn away, long gash down that flank. Distinctly lopsided. | Destroyed |
| 10 | **Wreck C** | Cold burnt-out husk. Heaviest char, eroded silhouette, **no embers**. | Destroyed (older wreck) |
| 11 | **Wreck D** | Catastrophic cook-off: large penetrations, blown-wide ring, hot ember cluster. | Destroyed (fresh) |
| 5 | **Broken turret** | Barrel severed to a torn stump, dead optic, charred. Same pivot as the intact turret. | Pairs with any wreck |

Notes:

- Columns 8–11 are **peers, not a sequence.** Choose randomly per destroyed unit so a field of wrecks doesn't look copy-pasted.
- Wreck C has no embers by design — useful for wrecks that have been on the field a while, or as the end state of a burn-down.
- The broken turret (col 5) is a drop-in replacement for col 1: same position, same pivot.
- A common pairing is a wreck hull with either the broken turret or no turret at all (turret "blown off" — optionally spawn it as separate debris).
- Damaged and wrecked hulls should stop emitting track-mark decals.

---

## 8. Track marks (column 12)

A ground decal of the tread impressions the tank leaves behind. **Each tank has its own**, matched to that chassis's track width, spacing, and cleat pattern.

### Properties

- **Semi-transparent.** The only partial-alpha cells in the sheet (roughly 25–150 of 255) in dark earth `#1E1916`. Composite over terrain; do not treat as an opaque sprite.
- **No outline** — these are impressions, not objects.
- **Seamlessly tileable.** The pattern spans the full 32 px height on a 4 px period, so stacked decals form an unbroken trail with no seam and no frame matching.
- **Track-aligned.** Mark strips sit at the same local X as that tank's treads.

### Intensity by chassis

| Chassis | Rows | Relative intensity |
|---|---|---|
| narrow | 0, 5 | lightest |
| compact | 4, 8 | light |
| standard | 1, 6 | medium |
| long | 3, 9 | heavy |
| wide | 2, 7 | heavier |
| **super-long** | 11 | very heavy |
| **super-heavy** | 10 | heaviest |

### Usage

Spawn one decal per **32 world-pixels travelled** (one cell length) so consecutive decals abut exactly. Use the **hull's** rotation.

```
distance_since_last_mark += distance_moved
while distance_since_last_mark >= 32:
    distance_since_last_mark -= 32
    spawn_decal(cell_rect(12, row), position, hull_angle, pivot = (0.5, 0.5))
```

- Render **below** all units.
- Same `(16, 16)` / `(0.5, 0.5)` pivot as everything else.
- Fade by modulating sprite alpha; no fade is baked into the art.
- Cap live decals (a ring buffer of a few hundred is typical).
- For tight corners, spawn every 16 px instead and accept slight overlap.
- Stop spawning for disabled/wrecked hulls.

---

## 9. Collision & gameplay sizing

Measured opaque bounding boxes (inclusive coordinates within the 32×32 cell):

| Row | Name | Hull bbox (x0,y0,x1,y1) | Hull size | Turret bbox | Turret size |
|---|---|---|---|---|---|
| 0 | scout | 9, 7, 22, 25 | 14 × 19 | 11, 2, 20, 20 | 10 × 19 |
| 1 | assault | 8, 5, 23, 26 | 16 × 22 | 10, 3, 21, 21 | 12 × 19 |
| 2 | breaker | 7, 4, 24, 27 | 18 × 24 | 10, 4, 21, 21 | 12 × 18 |
| 3 | longbow | 8, 3, 23, 28 | 16 × 26 | 11, 0, 20, 20 | 10 × 21 |
| 4 | flak | 8, 7, 23, 24 | 16 × 18 | 10, 6, 21, 21 | 12 × 16 |
| 5 | wraith | 9, 7, 22, 25 | 14 × 19 | 10, 3, 21, 20 | 12 × 18 |
| 6 | warden | 8, 5, 23, 26 | 16 × 22 | 10, 2, 21, 21 | 12 × 20 |
| 7 | ravager | 7, 4, 24, 27 | 18 × 24 | 10, 2, 21, 21 | 12 × 20 |
| 8 | glacier | 8, 8, 23, 24 | 16 × 17 | 11, 2, 20, 21 | 10 × 20 |
| 9 | obelisk | 8, 3, 23, 28 | 16 × 26 | 10, 0, 21, 21 | 12 × 22 |
| 10 | **titan** | 4, 3, 27, 28 | **24 × 26** | 7, 0, 24, 23 | 18 × 24 |
| 11 | **leviathan** | 5, 2, 26, 29 | **22 × 28** | 8, 0, 23, 23 | 16 × 24 |

Notes:

- Turret bboxes **include the barrel**, hence their height. The rotating turret body is roughly a circle of radius 4–5 px (standard) or 6.5–7 px (super) centered at (16, 16); the rest is barrel.
- Use the **hull** bbox for movement and hit colliders. Exclude the barrel from collision.
- Super-heavy hulls are substantially larger — size their colliders, health, and pathing footprint accordingly. `titan` at 24 × 26 nearly fills the cell.
- Damage-state bboxes vary slightly (wrecks lose edge chunks). If collider size must stay constant across states, derive it from the intact hull (column 0).

---

## 10. Individual files (alternative to slicing)

```
scifi_<name>_hull.png                  (= column 0)
scifi_<name>_turret.png                (= column 1)
scifi_<name>_turret_broken.png         (= column 5)
scifi_<name>_hull_damaged_light.png    (= column 6)
scifi_<name>_hull_damaged_disabled.png (= column 7)
scifi_<name>_hull_wreck_a.png          (= column 8)
scifi_<name>_hull_wreck_b.png          (= column 9)
scifi_<name>_hull_wreck_c.png          (= column 10)
scifi_<name>_hull_wreck_d.png          (= column 11)
scifi_<name>_trackmarks.png            (= column 12)
```

`<name>` ∈ `scout`, `assault`, `breaker`, `longbow`, `flak`, `wraith`, `warden`, `ravager`, `glacier`, `obelisk`, `titan`, `leviathan`.

**Track animation frames (columns 2–4) exist only in the sheet.** Slice them from the sheet if using individual files elsewhere.

---

## 11. Integration pseudocode

```
CELL = 32

function cell_rect(col, row):
    return Rect(col * CELL, row * CELL, CELL, CELL)

HULL_FRAMES  = [0, 2, 3, 4]      # track animation loop
WRECK_COLS   = [8, 9, 10, 11]    # interchangeable destroyed variants

class Tank:
    row               # 0..11
    position
    hull_angle        # movement direction
    turret_angle      # aim direction, independent
    track_frame = 0
    state = INTACT    # INTACT | LIGHT | DISABLED | DESTROYED
    wreck_col = null  # chosen once when destroyed

    function on_destroyed():
        wreck_col = random_choice(WRECK_COLS)   # pick once, then keep it

    function update(dt, distance_moved):
        if state in (INTACT, LIGHT) and distance_moved > 0:
            track_accumulator += distance_moved
            while track_accumulator >= PIXELS_PER_FRAME:
                track_accumulator -= PIXELS_PER_FRAME
                track_frame = (track_frame + 1) % 4

            mark_accumulator += distance_moved
            while mark_accumulator >= CELL:
                mark_accumulator -= CELL
                spawn_ground_decal(cell_rect(12, row),
                                   position, hull_angle, pivot = (0.5, 0.5))

    function hull_column():
        if state == DESTROYED: return wreck_col
        if state == DISABLED:  return 7
        if state == LIGHT:     return HULL_FRAMES[track_frame]  # light dmg still animates
        return HULL_FRAMES[track_frame]

    function draw():
        # ground decals drawn by the decal layer, BELOW all units

        draw_sprite(cell_rect(hull_column(), row),
                    position, hull_angle, pivot = (0.5, 0.5))

        if state == DESTROYED:
            draw_sprite(cell_rect(5, row), position, turret_angle, pivot = (0.5, 0.5))
        else:
            draw_sprite(cell_rect(1, row), position, turret_angle, pivot = (0.5, 0.5))
```

Note: column 6 (light damage) is a **static** sprite with no animation frames. If you want a lightly-damaged tank to still show moving tracks, either accept static tracks while damaged, or overlay damage as a separate decal on top of the animated hull. The pseudocode above keeps animating and reserves column 6 for stationary/showcase use — pick whichever fits your game.

### Turret aiming

```
desired = atan2(target.y - position.y, target.x - position.x) + 90°   # +90: art faces up
turret_angle = rotate_toward(turret_angle, desired, TURN_SPEED * dt)
```

Clamping the traverse rate (rather than snapping) gives the heavy turret feel. Super-heavy tanks should use a slower `TURN_SPEED`.

---

## 12. Constraints and known limitations

- **No directional pre-renders.** Sprites are single-frame, intended for runtime rotation.
- **Rotation artifacts.** At 32 px, rotating non-circular turrets (hex, box, wedge — rows 1, 2, 4, 5, 6, 8, 9, 10) shows mild pixel jitter at intermediate angles. Pre-render 8 or 16 fixed angles if objectionable.
- **No firing, recoil, or muzzle-flash art.**
- **No explosion or transition animation** between states.
- **No track animation for damage states** (columns 6–11 are static).
- **Super-heavy hulls nearly fill their cell.** `titan` is 24 × 26 of the available 32 × 32 — there is little margin, so avoid scaling them up relative to other rows without re-exporting at a larger cell size.
- Sprites are procedurally generated: geometrically consistent, but more regular than hand-drawn art.
