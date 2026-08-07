# Drop shadows for tanks and shells — design doc

Status: implemented, including two additions beyond the original proposal
below (kept in the doc for the reasoning, not because the plan changed):
a runtime/startup on-off toggle (see "Runtime toggle", not in the original
scope) and a per-shell *randomized* shadow offset instead of one flat
distance (see "Why shells get a bigger offset", updated with the actual
shipped range).

## Goal

Give tanks and shells a small drop shadow so they read as sitting *on* the
battlefield rather than flat-pasted onto it, and give shells specifically a
bigger offset than tanks so they read as airborne (a shell visibly
separated from its own shadow implies height; a tank's shadow should stay
tight, since it's a grounded vehicle).

- Tanks: 3px offset - just enough to imply a low, grounded silhouette.
- Shells: a noticeably bigger, *randomized* offset (shipped as 9-20px, see
  "Tuning") while `Flying` - the same shell/shadow separation trick used in
  every top-down arcade game with a jump/lob (Zelda's "Link jumping", Micro
  Machines, classic top-down GTA) to imply height with no actual z-axis.
  Randomizing it per shell (rolled once at fire time, fixed for that
  shell's flight) additionally implies different shots are lobbed at
  different heights, rather than every shot looking identical.

## Non-goals

- No real height/z-axis or projectile arc. Shells already fly in a flat 2D
  plane (`Shell::update` is `position += velocity * dt`, no altitude field) -
  see docs/physics-engine-design.md's non-goals for the same reasoning
  applied there. A *dynamic* shadow (offset growing/shrinking with a fake
  arc height over the shell's flight) is called out below as a natural
  follow-up, not part of this pass.
- No new textures, shaders, or asset-pipeline changes. This reuses the
  existing `tanks.png`/`shells.png` atlases and the existing tint mechanism
  already used for the dead-tank fade (`Tank::draw_tank`'s
  `DEAD_TINT_FACTOR` tint) - no new `tools/gen_*.py` script needed.
- No shadows on muzzle flash / impact burst frames, or on the damage overlay
  (`draw_damage`'s smoke/fire). Those are light/particle effects, not solid
  objects - a shadow under a muzzle flash doesn't make physical sense, and
  the wreck/burning-damage overlay already reads fine without one.

## Rendering approach: tinted silhouette, not a new asset

Raylib's `draw_texture_pro` tint is multiplicative. Drawing the *exact same*
sprite - same source rect, same dest size, same rotation - a second time
first, with `Color::new(0, 0, 0, alpha)` as the tint instead of
`Color::WHITE`, multiplies every opaque pixel's RGB down to black while
preserving the sprite's own alpha shape. That's a perfect silhouette with
zero new art: every existing frame (all 8 hulls x however many rows, every
shell variant/state) gets a correct shadow for free, including rotation.

This is the same technique already in the codebase for the dead-tank fade
(`tank.rs`'s `draw_tank`, `let tint = if tank.is_dead() { Color::new(v,v,v,255) } ...`)
- just pushed to full black + partial alpha instead of a gray fade.

Concretely, per entity, immediately before its normal draw call:

```rust
// same src/dest/origin/rotation as the real sprite, translated by the
// shadow offset, tinted flat black at partial opacity instead of WHITE
d.draw_texture_pro(texture, src, shadow_dest, origin, tank.rotation,
    Color::new(0, 0, 0, (255.0 * TANK_SHADOW_OPACITY) as u8));
```

where `shadow_dest` is the same `dest` rect used for the real sprite, just
with `x`/`y` shifted by the offset. The shadow inherits the sprite's own
rotation - it's a rotated silhouette of the actual hull shape, not a static
blob - which is what makes it read as "this specific tank's shadow" instead
of a generic ellipse.

### Why not a plain ellipse blob (the other common approach)?

A soft dark ellipse under each sprite is cheaper to reason about and is
common in top-down games, but this project's tanks aren't circular or
symmetric (twin-barrel hulls, angled turrets - see `TANK_SPRITE_ORDER`'s
single/twin-barrel mix), so a generic ellipse would clip barrels or leave
awkward gaps depending on rotation. The tinted-silhouette approach costs one
extra draw call per entity and already handles rotation/hull-shape
correctly, so it's worth the (trivial) extra draw.

## Draw-order integration

Shadows belong in `Game::render`'s existing Pass 1 (the offscreen
`scene_target` draw, `game.rs` ~line 701) - not a separate pass - so they
automatically get swept into the shockwave/muzzle/impact ripple distortion
like everything else already in that texture; no new plumbing needed there.

Within Pass 1, shadow draws interleave with the existing per-entity loop
rather than becoming one batched "draw all shadows first" pass:

```
for track in &self.tracks { draw_track(...) }        // unchanged

for enemy in &self.enemies {
    draw_tank_shadow(&mut d, textures.tanks, enemy);   // new
    draw_tank(&mut d, textures.tanks, enemy);
    draw_damage(&mut d, textures.damage, enemy, self.time);
}

draw_tank_shadow(&mut d, textures.tanks, &self.tank);  // new
draw_tank(&mut d, textures.tanks, &self.tank);
draw_damage(&mut d, textures.damage, &self.tank, self.time);

for shell in &self.shells {
    if shell.state == ShellState::Flying {
        draw_shell_shadow(&mut d, textures.shells, shell); // new
    }
    draw_shell(&mut d, textures.shells, shell);
}
```

Interleaving (shadow immediately before that same entity's sprite) rather
than a separate loop matters once tanks overlap on screen: it keeps a
tank's own solid body drawn on top of *its own* shadow and, for whichever
tank draws later in the existing enemies-then-player order, on top of an
earlier tank's shadow too if they're close together - the same simple
draw-order-as-layering the game already relies on elsewhere (e.g. tracks
under tanks, damage overlay over tanks). No real y-sorting needed at this
scale.

## New small draw functions, not a shared generic helper

`draw_tank_shadow` (in `tank.rs`, next to `draw_tank`) and `draw_shell_shadow`
(in `shell.rs`, next to `draw_shell`) - two small, near-duplicate functions
rather than one generic `draw_shadow(src, dest, origin, rotation, offset,
opacity)` utility shared across modules. This matches the existing style:
`draw_track`/`draw_tank`/`draw_shell`/`draw_damage` are already four
separate small draw functions with similar shapes, not unified behind a
generic abstraction, and each entity's shadow needs a different offset/
opacity constant and (for shells) a state check anyway, so a shared helper
would mostly just be passing those differences through parameters.

## Tuning constants (new, in `lib.rs`)

Proposed, next to the existing per-sprite tuning (`TANK_TEXTURE_SIZE`/
`SHELL_TEXTURE_SIZE` etc.) rather than the "track marks" or "shockwave"
sections, since these are static per-sprite-draw constants like scale, not
gameplay tuning or timed effects:

```rust
// Drop shadows: a second copy of the sprite drawn first, tinted flat black
// (see Game::render / draw_tank_shadow / draw_shell_shadow), offset toward
// a fixed screen-space light direction so it reads as "this shape, sitting
// slightly behind/below the real sprite" rather than a generic blob.
// Shared offset direction (down-right, a common top-down-arcade convention)
// - only the *distance* differs per entity type below.
pub const SHADOW_DIR_X: f32 = 0.6;
pub const SHADOW_DIR_Y: f32 = 0.8;

pub const TANK_SHADOW_OFFSET: f32 = 3.0;   // px - grounded, stays tight to the hull
pub const TANK_SHADOW_OPACITY: f32 = 0.35;

// Bigger than the tank offset on purpose: the separation between a shell
// and its own shadow is what reads as "airborne" with no real z-axis (see
// design doc). Only applied while ShellState::Flying. Rolled once per shell
// at fire time within this range (Shell::shadow_offset, set in
// Game::update right after Shell::spawn) rather than a flat distance, so
// different shells read as flying at different heights.
pub const SHELL_SHADOW_OFFSET_MIN: f32 = 9.0;  // px
pub const SHELL_SHADOW_OFFSET_MAX: f32 = 20.0; // px
pub const SHELL_SHADOW_OPACITY: f32 = 0.30;
```

`SHADOW_DIR_X`/`SHADOW_DIR_Y` are plain `f32` consts rather than one
`Vector2` const, as flagged as the likely fallback in an earlier draft of
this doc - the draw functions build the `Vector2` at call time instead
(`shell.position.x + SHADOW_DIR_X * shell.shadow_offset`, etc).

### Why shells get a bigger, randomized offset

A flat few px extra on a 64px-on-screen shell (`SHELL_TEXTURE_SIZE *
SHELL_SCALE`) barely reads at a glance during fast flight, and every shot
looking identically "high" reads as a rendering quirk rather than height.
Shipped as a 9-20px *range*, rolled once per shell at fire time
(`Shell::shadow_offset`, fixed for that shell's whole flight - see
`Shell::spawn`'s doc comment) rather than either a single flat distance or
a re-rolled-every-frame jitter: fixed-per-shell is what makes a given shot
read as "this shell is flying higher than that one," instead of a shimmer
on every individual shell. Both bounds and the opacity are starting points
to eyeball and adjust in-game, not values derived from anything physical
(there's no real height to derive them from).

## Runtime toggle

Not in the original scope above, added on request: shadows can be turned
off entirely, at runtime via the `L` key (`Game::shadows_enabled`, flipped
in `Game::update` the same way `P` flips `paused`) and at startup via
`--no-shadows` (parsed in `main.rs`'s `Args`, shadows on by default). The
toggle is not reset by `Game::init`, so - like `paused` - it survives round
restarts instead of resetting to on every round.

## Edge cases

- **Wrecked/burning tanks**: still solid objects sitting on the ground -
  keep the shadow. No special-case needed; `draw_tank_shadow` doesn't need
  to know about `is_wreck()`/`is_dead()` at all, unlike `draw_tank`'s own
  dead-tint logic.
- **Shell fire/hit frames** (`Fire0-2`, `Hit0-2`): explicitly skipped (see
  the `if shell.state == ShellState::Flying` guard above) - these are
  stationary blast/impact sprites at the muzzle or impact point, not
  airborne objects.
- **Screen edges**: a shadow can be nudged a few px past the battlefield
  bound (`spawn_walls`' 0..width/0..height) for a tank/shell hugging the
  wall. Cosmetic only (shadows aren't collidable), and `draw_texture_pro`
  clips naturally like any other slightly-off-screen draw - no clamping
  needed.
- **Performance**: one extra `draw_texture_pro` call per tank (always) and
  per shell (only while `Flying`) - at this game's scale (3-10 enemies,
  `MAX_SHELLS` per shooter) this is a trivial addition, no batching or
  texture-atlas changes needed.

## Possible follow-up (explicitly out of scope for this pass)

A *dynamic* shell shadow - offset growing then shrinking over the shell's
`Flying` lifetime to fake a lob arc (small offset just after firing, peak
offset mid-flight, small again just before impact) - would sell "airborne"
even harder than a constant offset. Deliberately left out here: it needs a
progress/lifetime signal `Shell` doesn't currently track for `Flying` (its
`timer` field is reset to 0 on entering `Flying` and never used again while
in that state - see `Shell::update`), so it's a small but real scope
increase (either give `Flying` a duration to interpolate against, or derive
progress from distance travelled vs. screen bounds). Worth a fast-follow
once the constant-offset version is in and the look is validated in-game.

## Implementation plan (completed)

1. Constants added to `lib.rs` per "Tuning constants" above.
2. `draw_tank_shadow` added to `tank.rs`, next to `draw_tank`.
3. `draw_shell_shadow` added to `shell.rs`, next to `draw_shell`, reading
   `shell.shadow_offset` rather than a flat constant.
4. Both wired into `Game::render`'s Pass 1 in the interleaved order above,
   gated on `self.shadows_enabled`.
5. `Game::shadows_enabled` (`L` key, `--no-shadows`) added - see "Runtime
   toggle".
6. `Shell` gained a `shadow_offset: f32` field, initialized to `0.0` inside
   `Shell::spawn` (which has no `rng` to roll from, same placeholder
   pattern as `body: None` there) and rolled for real in `Game::update`
   immediately after each `Shell::spawn` call site (player fire and enemy
   fire), from `SHELL_SHADOW_OFFSET_MIN..MAX`.
7. Verified with `cargo check` (clean) and `cargo run -- --help` (flag
   present, correctly optional). Full in-game visual confirmation (shadow
   rotation while turning, shell/shadow separation in flight, toggle via
   `L`) is on the user to eyeball per CLAUDE.md's "no automated test
   suite" note - this doc records what was built and why, not a screenshot
   record.
8. Not yet re-tuned against the candy-recolored palette (`tools/
   gen_tanks_candy.py`) vs. the original - still an open eyeball-tuning
   step, not done as part of this pass.
