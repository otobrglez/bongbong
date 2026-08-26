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
