# ToxicFrog Sprite — Integration Notes

`static/toxic_frog/<variant>/{idle,hurt,hop,attack,explosion}.png` — the
player's protect objective (`src/frog.rs`). Six colour `<variant>` folders
(`purple_white`, `blue_blue`, `blue_brown`, `green_blue`, `green_brown`,
`purple_blue` - see `crate::frog::FROG_VARIANT_DIRS`), one rolled at random
per round (`Frog::variant`) and fixed for that frog's whole life - purely
cosmetic, identical layout/frame counts/timing across all six. Third-party
art, provenance in `static/toxic_frog/SOURCE.md`.

## Layout

Each file is a plain horizontal filmstrip of 48×48 cells, no padding/margin
between frames — `col * 48` finds any frame directly, same shape as
`shells.png`/`damage.png`.

| File | Frames | FPS | Loop? |
|---|---|---|---|
| `idle.png` | 8 (`FROG_IDLE_FRAMES`) | 6 (`FROG_IDLE_FPS`) | yes, the default/fallback state |
| `hurt.png` | 4 (`FROG_HURT_FRAMES`) | 10 (`FROG_HURT_FPS`) | no — plays once per hit (`Frog::hurt_timer`), then falls back |
| `hop.png` | 7 (`FROG_HOP_FRAMES`) | 10 (`FROG_HOP_FPS`) | no — plays once for the full duration of an evasive hop (`Frog::hop_timer`), in step with the actual movement (see below) |
| `attack.png` | 6 (`FROG_ATTACK_FRAMES`) | 10 (`FROG_ATTACK_FPS`) | no — plays once whenever the frog bites a tank in range (`Frog::attack_timer`) |
| `explosion.png` | 9 (`FROG_EXPLOSION_FRAMES`) | 10 (`FROG_EXPLOSION_FPS`) | no — plays once on death, holds on the last frame forever after |

`FROG_TEXTURE_SIZE = 48.0` is the cell size for all five; `Frog::anim`
(`src/frog.rs`) picks which file + frame to draw each frame, in priority
order: Explosion > Hop > Attack > Hurt > Idle.

**Actual sprite content is much smaller than the 48×48 cell**: roughly a
22×16px glyph, bottom-anchored at the same y≈33 baseline in *every* frame of
*every* animation (checked via per-frame alpha bbox) — there's no pivot
drift to correct for between animations, unlike some sprite packs where
different clips use different padding.

**The hop is a real animated leap, not a teleport**: once
`simulation.rs`'s `frog_hop_target` finds a valid landing spot,
`Frog::start_hop` only records where the hop is headed (`hop_start`/
`hop_end`) - `Frog::tick` then linearly interpolates `position` from one to
the other over FROG_HOP_SECONDS, exactly as long as `hop.png` plays, and
`Game::update` copies that same position into the physics body every frame
so the frog stays collidable throughout the leap instead of just at the
start/end. Ground-level straight-line motion only (no fake screen-space
arc/height) - this is a top-down game with no Z axis to arc through, and
`hop.png`'s own squash/stretch frames already carry the "hop" feel.

## Not used

None — all five animations in the PurpleWhite variant are wired in.

## Colour

Kept as shipped (not run through `tools/punypalette.py`) — see
`static/toxic_frog/SOURCE.md` for why (no purple family in that palette).
