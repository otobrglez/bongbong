# PRD: Health and ammo indicator idicator v2

Purpose of this work is to replace the health indicator that is currenlty a "heart PNG" that follows tanks. This visual asset is to be completly removed from the game.

Instead current "white circle" that is surrounding player tank becomes and health idicator. 
100% health means full white circle. Lower the health less of circle is visible and it becomes more red.
Kind-a like donut chart. The dimensions of circle should remain. 

For player tank this circle should be white and this health indicator should be kind-a overlayed because I need to still see the white circle as it indicates where player tanks is.

For enemy tanks. When health gets low this should appear/be visible. Same as where hearh idicator appears now. This funcionality replaces that.

## Mechanics as built (2026-09)

Decisions from the 2026-09-09 interview, then what the code does.

- **The heart sheet is gone.** `static/health_bar.png` is deleted along with its
  `HEALTH_BAR_*` constants, the overhead bars over tanks and frogs, and the HUD
  corner's icon. The HUD keeps the `HP: n` number (white, orange, red by
  `hud_warn_threshold`/`hud_critical_threshold`).
- **The ground ring is the gauge** (`tank.rs`, `RingStyle::Gauge`, drawn by the
  shared `draw_ground_ring_at`, so radius, thickness and the inner disc are exactly
  the shield ring's - the dimensions did not change). The filled arc starts at 12
  o'clock and runs clockwise for remaining health over max of the circle
  (`health_ring_sweep`), in screen space; the rest of the circle is still drawn so
  the ring stays whole. Smooth raylib arcs, as the ring always was, not 2px blocks.
  The ring sits under the hull, so only its flanks show on most chassis and
  nothing on titan or leviathan; accepted rather than growing the ring.
- **Colour steps, never blends** (`HealthRamp`, one colour per quarter of health,
  `health_ring_step`): tanks and the player's frog use white, GOLD_BRIGHT `#EEA343`,
  RED_BRIGHT `#FF421A`, RED_DEEP `#9C3527`; the enemy frog uses RED_BRIGHT, RED_DEEP,
  RED_DK `#812F27`, RED_DARKEST `#4A2221`, so it is red at any health. All Puny
  Palette entries (`tools/punypalette.py`).
- **Player tank** (`draw_player_ring`): always on. The arc draws at
  `player_ring_opacity` (0.8); the missing part stays white at `player_ring_opacity`
  times `health_ring_base_opacity` (0.35), so the position marker never disappears.
  Cross-fades with the shield ring as before; gone for a wreck.
- **Enemy tanks** (`draw_enemy_ring`, `enemy_health_ring_visibility`): the missing
  part is a dark band, BLACK `#252525` at `health_ring_gap_opacity` (0.45), so a
  nearly full enemy ring never passes for the player's marker. Visible for
  `health_ring_hit_seconds` (3.0) after any hit (`Tank::mark_hit`), fading over the
  last `health_ring_hit_fade_seconds` (0.6), and for good once remaining health is
  at or below `enemy_health_ring_below` (0.5). Hidden under a full shield, for a
  wreck, and for a wave tank still rolling in.
- **Shield ring** (`tank::draw_tank_shield`): the rainbow ring is a gauge of the
  shield time left (`Tank::shield_charge`, the timer over `shield_duration_seconds`):
  same radius as the marker ring, the rainbow bands cover that fraction of the
  circle from 12 o'clock clockwise, and the rest is drawn as the tank's health
  ring draws its missing part (dimmed white for the player, the dark band for an
  enemy). It still cross-fades into the health ring over its last
  `shield_glow_fade_seconds`.
- **Frogs** (`frog::draw_frog_ring`): always on while alive; the player's frog uses
  the white ramp on dimmed white, the enemy frog the red ramp on dimmed RED_MD
  `#E44219`. `Frog::hit_flash_timer` is gone; nothing was left for it to time.
- **Knobs** (`tuning.rs`, group `cosmetics`, all live): `health_ring_hit_seconds`,
  `health_ring_hit_fade_seconds`, `enemy_health_ring_below`,
  `health_ring_base_opacity`, `health_ring_gap_opacity`.
- **Tooling**: the dev server's `snapshot` carries `tanks[].ring`, the ring's
  opacity factor 0..1 (0 while rolling in, for a wreck, under a full shield).
  `cargo test --lib health_ring` covers the step, sweep, segment and visibility
  rules.
- **Deferred: ammo.** The title promises an ammo indicator, but at these dimensions
  everything inside the health band sits under the hull, so an ammo gauge needs its
  own design once the health ring has been seen in play. Ammo stays HUD text.
