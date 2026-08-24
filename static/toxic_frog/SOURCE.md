# ToxicFrog sprite — provenance

`static/toxic_frog/<variant>/{idle,hurt,hop,attack,explosion}.png` are all
six colour variants of the third-party "ToxicFrog" character pack (from the
user's own `bongbong-assets/ToxicFrog/`), copied in unmodified. Not produced
by bongbong's own generators.

| `<variant>` directory | Pack source folder |
|---|---|
| `purple_white/` | `PurpleWhite/` |
| `blue_blue/` | `BlueBlue/` |
| `blue_brown/` | `BlueBrown/` |
| `green_blue/` | `GreenBlue/` |
| `green_brown/` | `GreenBrown/` |
| `purple_blue/` | `PurpleBlue/` |

Only the five per-animation PNGs (`_Idle`/`_Hurt`/`_Hop`/`_Attack`/
`_Explosion`) were copied from each pack folder - the combined `_Sheet.png`
and the `.aseprite` source file are not used here.

`Frog::variant` (`src/frog.rs`) picks one of these six at random per round
(see `crate::frog::FROG_VARIANT_DIRS`, the single source of truth for both
the directory names above and how many variants exist) and keeps it for the
frog's whole life - purely cosmetic, no gameplay difference between colours.

- No license/readme/attribution file is bundled in the source zip (checked
  the archive listing — nothing). Usage was confirmed directly by the
  project owner before integration; if this ever needs to be redistributed
  beyond local development, check the terms on whatever page it was
  originally downloaded from first.
- **`purple_white/` is deliberately not recolored onto punypalette** (see
  `tools/punypalette.py`/`docs/PALETTE.md`) — that palette has no purple/
  violet family by design (Puny World's own source art has none), so
  running this sprite's purple/cream colours through it would snap them to
  a dull red/brown and erase the exact "PurpleWhite" identity that was
  picked from the pack's six colour variants. Kept as an intentional
  accent, the same way HUD warning colors (red/orange) also sit outside
  the palette rather than a clash to fix. The other five variants (added
  later, once every colour got wired in rather than just one) weren't
  recolored either, for the same reason and for consistency across the set.
- Each frame is a 48×48 cell; the actual sprite content is a much smaller
  ~22×16px glyph bottom-anchored at the same baseline (~y=33) across every
  frame in every animation, including Hurt/Explosion — see `docs/FROG_SPEC.md`.
