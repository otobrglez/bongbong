# ToxicFrog sprite — provenance

`idle.png`, `hurt.png`, `hop.png`, `attack.png`, `explosion.png` are the
"PurpleWhite" colour variant of the third-party "ToxicFrog" character pack
(from the user's own `bongbong-assets/ToxicFrog/PurpleWhite/` —
`ToxicFrogPurpleWhite_Idle.png`/`_Hurt.png`/`_Hop.png`/`_Attack.png`/
`_Explosion.png` respectively), copied in unmodified. Not produced by
bongbong's own generators.

- No license/readme/attribution file is bundled in the source zip (checked
  the archive listing — nothing). Usage was confirmed directly by the
  project owner before integration; if this ever needs to be redistributed
  beyond local development, check the terms on whatever page it was
  originally downloaded from first.
- The pack also ships five other colour variants
  (BlueBlue/BlueBrown/GreenBlue/GreenBrown/PurpleBlue) and an `.aseprite`
  source file — not used here, only the PurpleWhite PNGs were copied in.
- **Deliberately not recolored onto punypalette** (see
  `tools/punypalette.py`/`docs/PALETTE.md`) — that palette has no purple/
  violet family by design (Puny World's own source art has none), so
  running this sprite's purple/cream colours through it would snap them to
  a dull red/brown and erase the exact "PurpleWhite" identity that was
  picked from the pack's six colour variants. Kept as an intentional
  accent, the same way HUD warning colors (red/orange) also sit outside
  the palette rather than a clash to fix.
- Each frame is a 48×48 cell; the actual sprite content is a much smaller
  ~22×16px glyph bottom-anchored at the same baseline (~y=33) across every
  frame in every animation, including Hurt/Explosion — see `docs/FROG_SPEC.md`.
