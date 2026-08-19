# Puny World overworld tileset — provenance

`punyworld-overworld-tileset.png` (432×1040, 16×16 tiles, 27 columns) and
`punyworld-overworld-tiles.tsx` (the matching Tiled tileset definition,
kept here for reference — not loaded by the game) are **third-party art**,
downloaded from the user's own Downloads folder
(`punyworld-overworld-tileset.png` and `PUNY_WORLD_v1.zip`), not produced
by bongbong's own generators.

- No license/readme/attribution file was found bundled in either the zip
  or the standalone PNG (checked the archive listing and scanned both
  files' bytes for embedded copyright/license/author text — nothing).
  Usage was confirmed directly by the project owner before integration;
  if this ever needs to be redistributed beyond local development, check
  the terms on whatever page it was originally downloaded from first.
- Deliberately **not** on the Resurrect 64 palette that every other sheet
  in `static/` uses (see `docs/PALETTE.md`) — a documented exception, not
  an oversight. See `docs/GROUND_SPEC.md` for why (mechanically snapping
  it onto R64 was tried and visibly degraded it — flattened shading and
  made the road hard to tell apart from grass).
- The pack ships full Tiled "wangset" autotile metadata (which tile goes
  where, by corner/edge terrain matching) in the `.tsx` — `docs/GROUND_SPEC.md`
  documents the exact lookup tables `src/ground.rs` extracted from it.
- **The live `punyworld-overworld-tileset.png` here is a retinted copy**,
  produced by `tools/retint_ground.py` from the pristine original kept at
  `_original/punyworld-overworld-tileset.png` (de-green pass, 2026-08 —
  deeper grass, muted dirt; see `docs/GROUND_SPEC.md` §1). `_original/` is
  the untouched third-party art; keep it pristine and rerun the script to
  regenerate the live copy.
