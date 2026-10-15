# Bong Sprites — Integration Spec

`static/bongs/<kind>/{idle,hurt,hop,attack,explosion}.png` — a side's
objective (`src/bong.rs`). Seven characters, one folder each, named by
`BongKind::name`:

| Side | Kind | Character | Strike | Knock-out |
|---|---|---|---|---|
| player | `spike` (default) | Bong Spike, a potted cactus | needle volley | hides in its pot and peeks over the rim |
| player | `oggle` | Bong Oggle, a one-eyed feeler blob | feeler zap | deflates and zips off the top of the field |
| player | `gloop` | Bong Gloop, a bouncy slime | goo spit | splits into three mini-slimes that bounce away |
| enemy | `blip` (default) | Bong Blip, a squid-shaped space invader | tentacle slap | a tractor beam lifts it home |
| enemy | `puff` | Bong Puff, a pocket storm cloud | lightning bolt | rains itself out into a puddle |
| enemy | `capi` | Bong Capi, a mushroom kid | spore puff | ducks into the soil and leaves a sprout |
| enemy | `fuzz` | Bong Fuzz, a soot puff | sneezes a soot ball | bursts into soot, its eyes last |

A map names its two (`MapFile::bong`, `MapFile::enemy_bong` - a top-level
`bong = "oggle"`, `enemy_bong = "capi"`; the builder's MAP panel, TANKS tab,
YOUR BONG and FOE BONG). `MapFile::bong_kind` answers for a side: the map's
pick when it is one of that side's kinds, else Bong Spike or Bong Blip. The
kind is fixed for the round and purely cosmetic - every kind has the same
clips, frame counts and timing, and nothing in `simulation/` reads it. A
player reads its name, "Bong Spike", from `bong-name-<kind>`; the training
round's first line says it (`bong-hello`, `{ $name }`).

None of the seven is a real animal, and no knock-out shows one dying: they
leave, hide, regrow or burst into something that is not a body.

## The generator

The art is drawn by code: `tools/spritegen/bongs/sprites.js` draws every
character procedurally, and `export.js` writes the filmstrips.

```
node tools/spritegen/bongs/export.js        # writes static/bongs/<kind>/*.png
```

Node only (it ships in the dev shell, `devenv.nix`); deterministic, no
randomness, so a re-run rewrites the same bytes. `sprites.js` is a plain
module that also runs in a browser, which is how the characters were
reviewed (the concept page linked from BB-136).

How it draws:

- **One shared body language.** Each character's `body()` fills shapes into
  a material mask (`dome`, `ellipse`, `poly`, `rect`), `shade()` gives each
  material a 1 px rim light (shadow on the lower right, highlight where the
  upper left meets the air), details (eyes, mouths, spots) are painted on
  top, and `compose()` outlines the whole silhouette in the character's
  darkest tone. Effects (bolts, puffs, projectiles) are their own layer with
  their own outline.
- **Continuous animation.** A clip is sampled at time `t` in 0..1, the
  middle of each frame, and every pose value (squash and stretch, lift,
  lean, a projectile's flight, a falling part's arc) is a curve through the
  clip's keyframes (`track`, Catmull-Rom, one key per original frame). The
  `keys` frames per clip land exactly on the keys; the shipped `n`, three
  times as many, are real in-betweens: eased squash, hop arcs, half-closed
  blinks, flickering flames, parts tumbling under gravity (`fly`).
- **The clips' roles** are the old frog's: Idle loops; Hop is the evasive
  leap; Attack is the strike, reaching the cell's right edge, x 47; Hurt
  flashes white in its second quarter (`flashAt`); Explosion is the
  knock-out and ends on a still frame the game holds forever.

## Layout

Each file is a plain horizontal filmstrip of 48×48 cells, no padding or
margin between frames — `col * 48` finds any frame directly, same shape as
`shells.png`. RGBA, transparent background.

| File | Frames | FPS | Length | Loop? |
|---|---|---|---|---|
| `idle.png` | 24 (`BONG_IDLE_FRAMES`) | 18 (`BONG_IDLE_FPS`) | 1.33 s | yes, the default/fallback state |
| `hurt.png` | 12 (`BONG_HURT_FRAMES`) | 30 (`BONG_HURT_FPS`) | 0.4 s (`BONG_HURT_SECONDS`) | no — plays once per hit (`Bong::hurt_timer`), then falls back |
| `hop.png` | 21 (`BONG_HOP_FRAMES`) | 30 (`BONG_HOP_FPS`) | 0.7 s (`BONG_HOP_SECONDS`) | no — plays once for the full duration of an evasive hop (`Bong::hop_timer`), in step with the actual movement (see below) |
| `attack.png` | 18 (`BONG_ATTACK_FRAMES`) | 30 (`BONG_ATTACK_FPS`) | 0.6 s (`BONG_ATTACK_SECONDS`) | no — plays once whenever the bong strikes a tank in range (`Bong::attack_timer`) |
| `explosion.png` | 27 (`BONG_EXPLOSION_FRAMES`) | 30 (`BONG_EXPLOSION_FPS`) | 0.9 s | no — plays once on the knock-out, holds on the last frame forever after |

The generator's `CLIPS` table and `lib.rs`'s constants must agree; change
both. The HUD draws the idle loop beside PROTECT and HUNT cut to
`hud::MISSION_BONG_SRC` (x 11..37, y 12..36 of the cell, where every
kind's idle frames stand, feet on y 35); a redrawn idle that leaves that
box is clipped there - measure it and widen the cut (and its
`text_tests` budget) with it. `BONG_TEXTURE_SIZE = 48.0` is the cell size for all five;
`Bong::anim` (`src/bong.rs`) picks which file and frame to draw each frame,
in priority order: Explosion > Hop > Attack > Hurt > Idle.

**The sprite is much smaller than its cell**: roughly 20×16 to 22×20 px,
standing on the y 33 baseline in every frame of every clip, so there is no
pivot drift between clips. The strike and some knock-outs use the rest of
the cell (Blip's beam and Oggle's flight go to the top edge). The collider
(`BONG_COLLIDER_HALF_EXTENT`) is the same for every kind.

**The hop is a real animated leap, not a teleport**: once
`simulation.rs`'s `bong_hop_target` finds a valid landing spot,
`Bong::start_hop` only records where the hop is headed (`hop_start`/
`hop_end`) - `Bong::tick` then linearly interpolates `position` from one to
the other over BONG_HOP_SECONDS, exactly as long as `hop.png` plays, and
`Game::update` copies that same position into the physics body every frame
so the bong stays collidable throughout the leap instead of just at the
start/end. The clip's own lift and shadow carry the arc; the position moves
along the ground. The same clip carries every hop the bong makes: the evasive
leap, the training bong's walk, and a hurt bong's hops to a bong health pack
(`Game::bong_pack_hop`, docs/bong-health-pack-prd.md section 11).

## Facing

**Every character is drawn facing right**: its strike reaches out to **+x**.
There is one filmstrip per clip and no left-facing art.

`bong::Facing` (`Right`/`Left`) is therefore a *draw-time mirror*, and
`bong::mirror` is the only place it is applied: it returns the sign to
multiply the source rectangle's width by - raylib's mirror idiom, shared
with `blast::draw_scorch`, `decal::draw_decal` and `grass::draw_tuft` - plus
a destination offset.

The generator centres every body on the cell's axis, x 23.5 between pixel
columns 23 and 24 (`BONG_SPRITE_BODY_CENTER_X`), so a flip leaves the body
on the same ground and the offset is zero. `mirror` keeps the offset for
art that is not centred.

The facing **persists**: `Bong::facing` is set by `start_hop` from where the
leap lands (not from the away direction - `combat::bong_hop_target` can angle
a hop up to a quarter turn off it, and the sprite has to match what the bong
visibly does) and by `start_attack` from where the victim is, then kept until
one of those changes it. A hop with less than `BONG_FACING_DEADBAND_PX` of
horizontal travel leaves it alone: the hop fan reaches a quarter turn either
way, so a dead-vertical leap is a real case whose sign would be float noise.
On a frame that both strikes and hops the hop wins, matching `Bong::anim`'s
Hop-over-Attack priority. Nothing in `simulation/` reads `facing`.

## Colour

Off the Puny Palette on purpose (`tools/check_sheets.py` does not list the
bongs): Blip's lilac, Oggle's magenta and Gloop's aqua have no family in a
palette sampled from Puny World, and the characters need to read against
grass, sand and every team colour. Each character's ramp is in its `setup`
in `sprites.js`, and its darkest tone is its outline.

## Older maps

A map or training script saved with the frog's keys still loads: `frog`,
`enemy_frog` and `frog_health` cells, a beat's `frog`, `shoot_frog` and
`frog_full`, are serde aliases of the bong's, and a line named `frog-...`
reads as `bong-...` (`bubble::line_key`). A re-save writes the bong's.
