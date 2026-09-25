# Health & ammo pickup icons — provenance

`laser.png` is **not** from the same pack (it has no laser variant) - it's
generated from scratch by `tools/gen_laser_pickup.py` (raw PNG bytes, no
Pillow, same convention as `tools/gen_damage.py`), deliberately in the same
loud/high-contrast spirit as the two below rather than recolored onto
punypalette. Regenerate with `python3 tools/gen_laser_pickup.py`.

`minigun.png` is likewise not from the pack - generated from scratch by
`tools/gen_minigun_pickup.py`, same raw-PNG-bytes/no-Pillow convention as
`laser.png` above and the same not-palette-snapped, loud/high-contrast
treatment. Three small muzzle sparks with short staggered trailing streaks
(bold amber-orange), encoding "burst of rounds" the way `laser.png` encodes
"one continuous beam". Regenerate with `python3 tools/gen_minigun_pickup.py`.

`plasma.png` is likewise not from the pack - generated from scratch by
`tools/gen_plasma_pickup.py`, same raw-PNG-bytes/no-Pillow convention as
`laser.png`/`minigun.png` above and the same not-palette-snapped, loud/
high-contrast treatment. A centered glowing cyan/teal orb with four short
electric arcs radiating outward, echoing `static/plasma.png`'s own in-flight/
impact art (`tools/spritegen/gen_plasma.py`) so the ground icon and the
projectile read as the same weapon - distinct from `laser.png`'s beam and
`minigun.png`'s muzzle sparks, since the plasma cannon's identity is the bolt
itself, not a stream. Regenerate with `python3 tools/gen_plasma_pickup.py`.

`speedup.png` is likewise not from the pack - generated from scratch by
`tools/gen_speedup_pickup.py`, same raw-PNG-bytes/no-Pillow convention as
`laser.png`/`minigun.png`/`plasma.png` above and the same not-palette-
snapped, loud/high-contrast treatment. A filled lightning-bolt polygon
(electric yellow, dark outline, white-hot core down the middle) - the
universal "speed boost" symbol, distinct from the other three's beam/
sparks/orb. Regenerate with `python3 tools/gen_speedup_pickup.py`.

`missiles.png` is likewise not from the pack - generated from scratch by
`tools/gen_missiles_pickup.py`, same raw-PNG-bytes/no-Pillow convention and
the same loud/high-contrast treatment. A volley of four small missiles side
by side, staggered as if they left their tubes a beat apart - lime bodies
(the HUD's `HUD_MISSILES_COLOR`), red noses and fins, a flame under each -
the seeker pod's volley, distinct from the minigun's sparks and the plasma's
orb. Regenerate with `python3 tools/gen_missiles_pickup.py`.

`flamethrower.png` is likewise not from the pack - generated from scratch by
`tools/gen_flamethrower_pickup.py`, same raw-PNG-bytes/no-Pillow convention
and the same loud/high-contrast treatment. A dark fuel drum with a short
steel nozzle spitting a tongue of flame to the right - white-hot at the
nozzle through orange (the HUD's `HUD_FLAME_COLOR`) to red at the tip, the
same ramp the in-game stream's particles use. Regenerate with
`python3 tools/gen_flamethrower_pickup.py`.

`shield.png` is derived from `health.png` below: `tools/gen_shield_pickup.py`
reads the health pack and sweeps its red through the rainbow (diagonally,
red top-left to violet bottom-right), leaving the white cross, dark outline
and all shading untouched - so the rainbow shield reads as "a health pack,
but rainbow", the pickup it always appears next to. Same raw-PNG-bytes/no-
Pillow convention as the generators above, plus a minimal PNG decoder for
the source. Being a recolour of the pack's art, it inherits `health.png`'s
provenance and terms. Regenerate with `python3 tools/gen_shield_pickup.py`.

`frog_health.png` is derived from `health.png` the same way:
`tools/gen_frog_health_pickup.py` rotates every saturated pixel of the box
103 degrees around the colour wheel - to hue 106, the hue of the HUD's
`FROG_COLOR` - with saturation and value left alone, so the outline,
highlights and shading survive intact; it then paints the white cross out
and stamps a 12x12 top-down frog in the cross's own off-white, eyes in the
box's darkest tone. The job is that it reads as "a health pack, but for the
frog" at 32px (docs/frog-health-pack-prd.md section 7). Same
raw-PNG-bytes/no-Pillow convention and the same minimal decoder as the
shield, and it inherits `health.png`'s provenance and terms for the same
reason. Regenerate with `python3 tools/gen_frog_health_pickup.py`.

`health.png` and `ammo.png` are `health-red 32px.png` and
`ammo-pistol 32px.png` from the third-party "2D Health & Ammo Pickups v6.2"
pack (`bongbong-assets/2D Health & Ammo Pickups v6.2/32px/`), copied in
unmodified. Not produced by bongbong's own generators.

- Author: https://fightswithbears.itch.io/ (per the pack's own
  `author.txt`) - no separate license file bundled; usage was confirmed
  directly by the project owner before integration, same as the other
  third-party art in `static/` (`punyworld/`, `toxic_frog/`). If this ever
  needs to be redistributed beyond local development, check the terms on
  the itch.io page first.
- The pack ships several other variants per kind (`health-green`,
  `health-armor`, `ammo-pistol-alt`, `ammo-rifle`, `ammo-rifle-alt`,
  `ammo-shotgun`, `ammo-shotgun-alt`) and a combined spritesheet - not used
  here, only these two 32px PNGs were copied in.
- **Deliberately not recolored onto punypalette** (see
  `tools/punypalette.py`/`docs/PALETTE.md`), same reasoning as
  `toxic_frog/`'s: pickups are meant to read as visually loud against the
  muted terrain specifically so they're easy to spot at a glance, which is
  the opposite of what blending them into the palette would do.
