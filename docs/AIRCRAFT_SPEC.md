# Aircraft spec — air support's helicopter and jet

The two aircraft that answer an air support crate (docs/air-support.md),
straight down and nose up like every sprite (rotation 0 faces up), lit from
the top left, the tanks' `#252525` outline, on the 2 px block grid. No green:
they fly over the grass (`tools/check_sheets.py`'s no-green rule), so the
helicopter is desert tan and the jet a pale grey.

## 1. File

| File | Size | Grid | Drawn at |
|---|---|---|---|
| `static/aircraft_sheet.png` | 352 × 88 | 4 cols × 1 row of 88 px | 1:1, centred on the aircraft, turned to its heading |

An 88 px cell is 44 design pixels: ten per cent over a tank's 80 px sprite
cell (`TANK_SPRITE_SIZE` 40 at `Tank::scale` 2). Written by
`tools/spritegen/gen_aircraft.py` (raw PNG bytes, no Pillow, nothing random):

```
python3 tools/spritegen/gen_aircraft.py
just check-sheets
```

Never edit it by hand. On the palette's extended set (the stone steps), no
green.

| Col | What |
|---|---|
| 0 | the helicopter's airframe: cabin with a tandem canopy, stub wings with a rocket pod at each tip, tail boom, stabiliser, tail rotor housing; no main rotor |
| 1 | its main rotor: four blades on the axes round the hub, centred in the cell |
| 2 | the rotor a sixteenth of a turn on |
| 3 | the jet: nose gun, canopy, straight wings, two engines high on the tail, twin fins on the tail plane |

The lights are single design pixels: the helicopter's tail light red and its
pod lights amber; the jet's wingtips red (left) and green-blue (right), its
engines' glow amber.

## 2. Drawing (`air_support::paint_shadows`, `paint_aircraft`)

On any canvas, from `Sheet::Aircraft`:

- **The shadow**, after the floor and under the tanks: the airframe's cell
  tinted black at `tank_shadow_opacity` × 0.8, on its ground point nudged
  6 px along the light (`shadow_dir_x`/`_y`). The helicopter's rotor casts
  its disc too, at the rotor's opacity.
- **The aircraft**, over everything that stands, trees and drones included:
  the cell at its ground point lifted by its height (`air_heli_height_px`,
  `air_jet_height_px`), turned to its heading. The helicopter's rotor is
  drawn over the airframe with its centre on the mast, `HELI_MAST_PX` (12)
  ahead of the airframe's centre, at `ROTOR_ALPHA` (0.55) - a blur, not a
  blade - its two frames alternating at `ROTOR_HZ` (30) on the round clock.
