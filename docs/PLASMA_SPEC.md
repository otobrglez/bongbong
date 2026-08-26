# Plasma Bolt Sprite Sheet — Integration Notes

`plasma.png`, the projectile fired by a tank holding plasma ammo (see
`plasma.rs`, `pickup::PickupKind::Plasma`).

---

## 1. Overview

| | |
|---|---|
| Dimensions | 320 × 64 |
| Grid | 10 cols × 2 rows |
| Cell size | 32 × 32 |
| Rows | 2 — one per `plasma::PlasmaVariant` (Teal, Purple) |

Every plasma bolt draws from one of the two rows depending on
`Plasma::variant`, not the shooter's chassis — `plasma::Plasma` has no
`shooter_row`-indexed colour field the way `shell::Shell` does. The
variant is a property of the *ammo* (rolled on pickup), not the tank
firing it, same "every unit shares one shared piece of art" spirit as
`bullet::Bullet`'s single row, just doubled for the two variants.

---

## 2. Column layout

Same overall Fire/Flying/Hit shape as `shell::ShellState` — a plasma bolt
is fired the exact same way a shell is (`Tank::pending_plasma_shot` mirrors
`Tank::pending_shot`'s twin-barrel delay) — but `Flying` spans 4 columns
instead of 1, a baked breathing-cycle animation instead of a single static
frame:

| Col | `PlasmaState` | Meaning |
|---|---|---|
| 0 | `Fire0` | Charge building at the muzzle |
| 1 | `Fire1` | Bright flash as the bolt clears the barrel, arcs kicking out |
| 2 | `Fire2` | Flash finishing, bolt pulling away |
| 3 | `Flying` (frame 0/4) | Dim — the original single Flying frame's size |
| 4 | `Flying` (frame 1/4) | Rising — slightly bigger, a faint 2-arc shimmer |
| 5 | `Flying` (frame 2/4) | Peak — biggest/brightest, a 4-arc shimmer |
| 6 | `Flying` (frame 3/4) | Falling — back to "rising" size, shimmer at different angles |
| 7 | `Hit0` | Impact burst starting |
| 8 | `Hit1` | Electric burst expanding, arcs radiating outward |
| 9 | `Hit2` | Burst dissipating |

`PlasmaState` itself still has one `Flying` variant, not four — game logic
(hit detection, movement) only ever needs "is this bolt currently flying",
so splitting it into four enum states would've meant updating every
`state == PlasmaState::Flying` check across `simulation.rs`/`game.rs` for no
logic payoff. Instead `plasma::flying_col(timer)` picks which of columns
3-6 to *draw* this frame (cycling forward at `PLASMA_FLYING_CYCLE_FPS`,
wrapping every 4 frames), entirely inside `draw_plasma`/`draw_plasma_shadow`
— `PlasmaState::col()` itself only returns a sane value for the other six
states; its `Flying` arm is unused (kept only so the match stays exhaustive).

Fire/Hit timings are identical to `ShellState::duration` (0.06/0.06/0.05s
fire, 0.08/0.10/0.14s hit) — a plasma bolt fires at the same cadence a shell
does, unlike a minigun burst.

---

## 3. Two animation layers while Flying

`Flying` visibly pulses via two independent, differently-paced layers:

1. **Baked breathing cycle** (`flying_col`, this sheet) — a discrete 4-frame
   dim→rising→peak→falling triangle wave in the sprite's own size, plus a
   couple of shimmer arcs at the brighter frames, cycling at
   `PLASMA_FLYING_CYCLE_FPS`.
2. **Runtime glow halo** (`plasma::glow_pulse`, not baked into the sheet) —
   two concentric translucent discs drawn on top of the sprite, sized/faded
   by a continuous sine wave at `PLASMA_PULSE_HZ`, coloured by
   `PlasmaVariant::glow_colors`.

Deliberately two different rates/shapes (a discrete cycle plus a continuous
sine), not one clock driving both — see `PLASMA_FLYING_CYCLE_FPS`'s doc
comment. Each Flying frame (cols 3-6) is kept fairly small/subtle even at
its brightest so the runtime glow still reads as an addition on top of it,
not a duplicate of the same effect baked twice.

---

## 4. `PlasmaVariant`: Teal and Purple

Rolled once per `PickupKind::Plasma` pickup (`PLASMA_PURPLE_PICKUP_CHANCE` =
0.3, so Teal is the remaining 70%) and carried on `Tank::plasma_variant`
until the next pickup rerolls it — the exact same mechanism
`laser::LaserVariant`/`LASER_BLUE_PICKUP_CHANCE` already uses. Purple hits
`PLASMA_PURPLE_DAMAGE_FACTOR` (1.10×) harder, stacked on top of the base
bolt's own `PLASMA_DAMAGE_FACTOR`.

Purple is a **second baked row**, not a runtime tint over Teal's row. A
tint was tried first (`draw_texture_pro`'s `tint` param, one shared row) and
rejected: that tint is a per-channel multiply, which can only ever
darken/filter a pixel toward the tint colour, never invert a channel that
started at zero — Teal's glow body has no red channel at all, so multiplying
it by a violet tint just produced a darker *blue*, with only the sprite's
white core/arc pixels actually taking on the tint's violet (since
white × tint = tint exactly). The net result read as "blue bolt with purple
sparks," not a purple bolt. A real second colour pass sidesteps the problem
entirely - see `tools/spritegen/gen_plasma.py`'s `TEAL`/`PURPLE` palettes.

`punypalette.py` has no true purple/violet family (its own doc comment: Puny
World's source art doesn't have one either, which is why `gen_tanks.py`'s
"wraith" chassis reassigned that identity to a different hue instead of
inventing one). There was no other hue family free to redirect *this* role
to without it colliding with an existing identity (a wall material, another
tank chassis, etc.), so `PURPLE` introduces one literal off-palette violet
family instead - the same allowance `laser::LaserVariant::colors` already
exercises for a runtime-drawn effect, extended here to a second baked
sprite row since the palette genuinely has no candidate to reassign.

The hot core and electric arcs (`CORE`/`ARC`, both `WHITE`) are shared by
both rows unchanged — real electric arcs read white-hot regardless of the
surrounding plasma's own colour, so this is a deliberate shared constant,
not a missed variant hook.

---

## 5. Sizing, speed and damage

| Constant | Value | Note |
|---|---|---|
| `PLASMA_SCALE` | 2.08 | The original 2.6 tuning, reduced 20% after it read too big on screen; still above `SHELL_SCALE` (2.0). Also sizes the runtime glow (`glow_pulse`'s `base_radius`) — one constant scales the whole effect. |
| `PLASMA_SPEED` | 504 | The original 420 tuning, increased 20%; now a touch *faster* than `SHELL_SPEED` (500) rather than slower. |
| `PLASMA_DAMAGE_FACTOR` | 1.24 | Base bolt vs. a shell, before `PlasmaVariant`. |
| `PLASMA_PURPLE_DAMAGE_FACTOR` | 1.10 | Purple's further multiplier on top of the above. |

Total damage = `PLAYER_DAMAGE_MIN/MAX` or `ENEMY_DAMAGE_MIN/MAX` (whichever
side fired it) × `TANK_CHASSIS_DAMAGE_FACTOR_BY_ROW` × `PLASMA_DAMAGE_FACTOR`
× `PlasmaVariant::damage_factor()` — see the plasma hit-resolution block in
`simulation.rs`'s `update`.

---

## 6. Style

Generated by `tools/spritegen/gen_plasma.py`, reusing
`gen_shells.py`/`gen_bullets.py`'s drawing primitives (`blank`/`put`/`disc`,
copied in rather than imported — no generator script imports another's
primitives, only `punypalette` is shared), plus two new ones this sheet
needed: `ring` (an annulus, for the `Flying` frames' dark rim) and `bolt` (a
short two-segment kinked line radiating from center, the "electronic" read
`Fire1`/`Hit0`/`Hit1`/`Hit2`/the Flying shimmer arcs lean on for a sci-fi
splash rather than a shell's smoke-and-fire blast). Every frame function
takes a `Palette` (`glow`/`glow_md`/`glow_soft`/`dark`) parameter instead of
reading module-level colour constants, so the same drawing code produces
both rows.

No chunky-pixelate post-process (unlike `gen_walls.py`) — `PLASMA_SCALE`
(2.08) still sits at/above `Tank::scale` (2.0), so the sprite doesn't read
blurrier/finer than tanks at the same native resolution without needing one.

Regenerate in place with:

```
nix-shell -p "python3.withPackages (ps: [ps.pillow])" \
  --run "SPRITE_OUT=static python3 tools/spritegen/gen_plasma.py"
```

---

## 7. Rust constants

```rust
pub const PLASMA_AMMO_PER_PICKUP: i32 = 10;
pub const PLASMA_DAMAGE_FACTOR: f32 = 1.24;

pub const PLASMA_TEXTURE_SIZE: f32 = 32.0;
pub const PLASMA_SCALE: f32 = 2.08;             // still above SHELL_SCALE (2.0)
pub const PLASMA_SPEED: f32 = 504.0;            // now a touch faster than SHELL_SPEED (500)
pub const PLASMA_HIT_HALF_EXTENT: f32 = 5.0;    // bigger than SHELL_HIT_HALF_EXTENT (3.0)
pub const PLASMA_RECOIL_SPEED: f32 = 22.0;
pub const PLASMA_RECOIL_MAX_SPEED: f32 = 45.0;
pub const PLASMA_SHADOW_OFFSET_MIN: f32 = 10.0;
pub const PLASMA_SHADOW_OFFSET_MAX: f32 = 22.0;
pub const PLASMA_SHADOW_OPACITY: f32 = 0.30;
pub const PLASMA_IMPACT_KNOCKBACK_SPEED: f32 = 45.0;

pub const PLASMA_PULSE_HZ: f32 = 6.0;
pub const PLASMA_PULSE_MIN_SCALE: f32 = 0.85;
pub const PLASMA_PULSE_MAX_SCALE: f32 = 1.35;
pub const PLASMA_FLYING_CYCLE_FPS: f32 = 10.0;

pub const PLASMA_PURPLE_PICKUP_CHANCE: f32 = 0.3;
pub const PLASMA_PURPLE_DAMAGE_FACTOR: f32 = 1.10;
```

---

## 8. Not changed

- `ShellState`, `Shell`, `shells.png` — entirely untouched by this sheet.
- `BulletState`, `Bullet`, `minigun_bullets.png` — untouched.
- The ground pickup icon is a separate, standalone 32×32 PNG
  (`static/pickups/plasma.png`, `tools/gen_plasma_pickup.py`, raw PNG bytes,
  no Pillow — same convention as `gen_laser_pickup.py`/`gen_minigun_pickup.py`),
  not part of this sheet, and doesn't distinguish `PlasmaVariant` (same as
  `laser.png`'s ground icon not distinguishing Red/Blue - which variant a
  pickup grants is rolled on collection, so the icon on the ground can't
  promise one or the other).
