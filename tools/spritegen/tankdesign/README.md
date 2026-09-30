# tankdesign — the tank art generator

Everything the tanks look like comes out of this directory: the paint and
light sheets, the weapon modules and the anchors the engine reads
(docs/SPRITESHEET_SPEC.md has the layout). No numpy - Pillow only, under
nix:

```sh
nix-shell -p "python3.withPackages (ps: [ps.pillow])" --run "python3 export.py"
```

`export.py` writes `static/scifi_tanks_sheet.png`, `static/scifi_tanks_glow.png`,
`static/tank_modules.png`, `static/tank_modules_glow.png` and
`src/tank_art.rs` from the **Vanguard** line; run `just check-sheets`
afterwards and re-pin the thumbnail hashes (`thumbnail::tests`) consciously.
Every render is seeded from the design's name, so an unchanged design
exports byte-identical sheets.

| File | What it is |
|---|---|
| `kit.py` | the palette (Puny + `TANK_EXTRA`, the team ramps and lamp colours), materials, shading modes, `Part`, `Builder`, `Design`, `Line`, the chassis table (`CHASSIS`: footprints, guns, colour identities) |
| `damage.py` | the four live damage tiers and four wrecks, planned once per design from each part's tags |
| `render.py` | building a design into its cells (`render_design`), previews on the real ground, hero sheets, the review page's build |
| `export.py` | the shipped sheets and `src/tank_art.rs` |
| `lines/` | the four design lines of the September 2026 study - `vanguard` (shipped), `skimmer`, `foundry`, `prototype` |
| `page.py`, `page/` | the "Motor Pool" review page the lines were chosen on |
| `zoom.py`, `density.py` | one design at a pixel-grid zoom; the one-pixel density study |
| `BRIEF.md` | the design brief: the API, the chassis table, the rules the designs follow |

Previews and review builds land in `target/tankdesign/` (or
`TANKDESIGN_OUT`):

```sh
nix-shell -p "python3.withPackages (ps: [ps.pillow])" --run "python3 render.py hero vanguard assault --zoom 6"
nix-shell -p "python3.withPackages (ps: [ps.pillow])" --run "python3 render.py preview vanguard"
```
