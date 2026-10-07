# Tank sprite sheets — integration spec

The tanks are four generated sheets and one generated Rust file, all written
by `tools/spritegen/tankdesign/export.py` from the **Vanguard** design line
(`tools/spritegen/tankdesign/lines/vanguard.py`). Never edit them by hand:
change the design and export again (§9).

| File | Size | What it holds |
|---|---|---|
| `static/scifi_tanks_sheet.png` | 1320 × 2400 | the paint: 33 columns × 60 rows of 40 × 40 cells |
| `static/scifi_tanks_glow.png` | 1320 × 2400 | the light layer, the same layout |
| `static/tank_modules.png` | 1760 × 480 | the weapon modules: 44 columns × 12 rows |
| `static/tank_modules_glow.png` | 1760 × 480 | their light layer |
| `src/tank_art.rs` | — | the anchors the engine reads: lamps, muzzles, tube mouths (§7) |

Exact palette PNGs - every colour the art uses in the palette, alpha in
`tRNS` - which decode to straight-alpha RGBA (`export.py`'s `save_sheet`;
a third of the RGBA files' size). Transparent background, hard one-pixel
outline in the Puny Palette's darkest tone. Nearest-neighbour filtering
only.

---

## 1. Orientation, pivot and scale

- Every sprite faces **up** (−Y); rotation 0 is up, as everywhere in the game.
- **One pivot for everything**: the centre of the 40 px cell, `(20, 20)` -
  the turret ring's centre. Hull, turret and every module are drawn at the
  tank's position about that point, each at its own angle (the hull at
  `Tank::visual_rotation`, the turret and its modules at
  `turret_visual_rotation`), so the turret stays seated at every angle with
  no offset math.
- A design pixel is drawn `Tank::scale` (2) world pixels wide, so a cell is
  80 px on screen (`TANK_SPRITE_SIZE`, `Tank::sprite_size`). The gameplay
  frame is still the middle 32 px (`TANK_FRAME_SIZE`, `Tank::size`): the art
  keeps the hull footprints of `TANK_HULL_BBOX_BY_ROW`/
  `TANK_TURRET_BBOX_BY_ROW`, so hit boxes, colliders and corridor fit are
  unchanged, and the extra 4 px each side is room for a recoiling barrel, a
  thrown turret and a wreck's debris.

## 2. Rows: five team blocks of the roster

Rows `block * 12 + chassis`, the chassis in `TankKind` order:

| Block | Rows | Team | Colours |
|---|---|---|---|
| 0 | 0–11 | enemy | each chassis's own body and accent (§6) |
| 1 | 12–23 | player 1 | sky blue |
| 2 | 24–35 | player 2 | hot pink |
| 3 | 36–47 | player 3 | silver-white |
| 4 | 48–59 | player 4 | vivid orange |

`Tank::sheet_row` picks the block from the owner (`sheet_block`,
`TANK_TEAM_BLOCKS`); a seat past the fourth draws player 1's block and is
told apart by its ring colour (`tank::TEAM_COLORS`) and its `P5`..`P8`
label. In a player block the body and accent ramps are the team's ramp
(`kit.TEAM_RAMPS`, Resurrect 64 steps extended at both ends) and the marker
and sensor lights glow in the team's lamp colour (`kit.TEAM_LIGHT`); steel,
glass, treads, char and embers are the same in every block, and the damage
is seeded per chassis, so a player's wreck has the same holes as the enemy's.

## 3. Columns of the tank sheets

| Cols | Layer | Contents |
|---|---|---|
| 0–3 | hull | tier 0 (pristine), track frames 0–3 |
| 4–7 | hull | tier 1 (damaged), track frames 0–3 |
| 8–11 | hull | tier 2 (heavily damaged), track frames 0–3 |
| 12–15 | hull | tier 3 (critical), track frames 0–3 |
| 16–19 | hull | wrecks: blown, gutted, husk, cook-off |
| 20–22 | turret | tier 0: at rest, first barrel recoiled, second barrel recoiled / return |
| 23–25 | turret | tier 1, the same three poses |
| 26–28 | turret | tier 2 |
| 29–31 | turret | tier 3 |
| 32 | turret | broken (on every wreck but the blown one) |

`Tank::hull_col` is `tier * TANK_TRACK_FRAMES + frame` for a live tank and
`wreck_col` for a wreck; `Tank::turret_col` is `TANK_TURRET_COL + tier *
TANK_TURRET_POSES + pose`, or `TANK_BROKEN_TURRET_COL` on a wreck.

## 4. Damage tiers and wrecks

`TANK_DAMAGE_TIERS` = `[25, 50, 75]`: the tier is how many of those a tank's
damage has reached (`Tank::damage_tier`). The kit plans the damage once per
design and each tier applies more of the plan, so a scratch taken at 25 is
still there at 75 and a tank reads as the same tank getting worse
(`damage.py`):

- **Tier 1, scuffed** — scratches to bare metal, a scorch, a dent, one lamp
  cracked, loose stowage gone.
- **Tier 2, damaged** — an armour plate blown off to the frame and its
  cables, a penetration hole, antennas snapped, half the lamps out, a spark
  at the wound.
- **Tier 3, critical** — more plates gone and fire (or ion glow) inside, a
  broken track run, every light dead but one blinking warning lamp, the
  silhouette chipped.

The tracks keep turning at every live tier (a hurt tank still drives), and
`lay_tracks` keeps pressing marks until the hull is a wreck. On the light
layer a tier-2 or tier-3 hull cycles its frames on the clock rather than on
distance, so its sparks and warning lamp blink on a tank standing still.

The four wrecks are peers, rolled once when the tank dies (`roll_wreck_col`):

| Col | Wreck | Turret |
|---|---|---|
| 16 | **blown** — the turret ring blown out, embers | the broken turret thrown clear, lying beside the hull (`Tank::turret_thrown`, `turret_placement`) |
| 17 | **gutted** — armour stripped, burning inside | broken, on the ring |
| 18 | **husk** — cold and burnt out, no glow | broken, on the ring |
| 19 | **cook-off** — the deck torn open by an ammunition fire, one track run gone | broken, on the ring |

The effects language adds only what moves (`damage_stage.rs`,
docs/effects.md), drawn on top of whichever tier the tank is at: smoke off
the engine deck from the damaged tier (`damage_stage::SMOKES_AT`), fire on
the deck from the critical tier (`damage_stage::BURNS_AT`), and a wreck
burning down, with its smoke.

## 5. Recoil

Firing the main gun kicks the turret through its poses: `Tank::kick` shows
pose 1 (the first barrel back) for `tank_recoil_seconds`, then pose 2 - the
second barrel of a twin, fired `tank_twin_shot_delay_seconds` later, or the
barrel on its way back on a single gun - then rest (`Tank::tick_recoil`). A
plasma shot kicks the same way and flashes the plasma module; a laser shot
only flashes the laser module (`kick_laser`). Presentation only: a replica
kicks on the `Fired` event (`net::apply`).

## 6. The chassis

| Row | Chassis | Codename | Role | Signature |
|---|---|---|---|---|
| 0 | `scout` | Lynx | fast recon | arrowhead nose, round turret with a big optic, radar dish |
| 1 | `assault` | Bulwark | general purpose | twin guns in a slab-sided box turret, teal glacis strip |
| 2 | `breaker` | Maul | heavy brawler | ram prow with two steel ram blocks, hex turret, one heavy gun |
| 3 | `longbow` | Yew | artillery / sniper | long hull, rangefinder ears, bore evacuator, stern spades |
| 4 | `flak` | Hornet | anti-air / close range | twin autocannons with flash hiders, drum feeds, radar panel |
| 5 | `wraith` | Shade | stealth | faceted plates, full-length skirts, a low kite turret with one visor slit |
| 6 | `warden` | Aegis | support / defence | shield emitters glowing on the corners and turret flanks |
| 7 | `ravager` | Warhog | heavy assault | spiked ram, sponsons with MGs, twin stacks, twin guns |
| 8 | `glacier` | Floe | balanced | the plainest of the line: compact skirted hull, box turret, one long gun |
| 9 | `obelisk` | Trebuchet | siege | twin long guns from a wedge turret, revolver autoloader, stabiliser feet |
| 10 | `titan` | Colossus | super-heavy assault | four track runs, massive hex turret, twin 4 px guns, a searchlight |
| 11 | `leviathan` | Behemoth | super-heavy siege | one massive braked gun, road wheels, twin engine decks |

Every chassis keeps its colour identity (`kit.CHASSIS`: body and accent
ramps on the Puny Palette), carries two or four headlamps, tail lights, a
turret sensor and a marker light in its accent (the team colour on players).

## 7. Weapon modules

`tank_modules.png` is one row per chassis (the enemy's colours; every team
shares them, since the modules are gunmetal and glass) - each module drawn
at that chassis's own hardpoint on the turret, in the turret's frame about
the shared pivot:

| Cols | Module | States |
|---|---|---|
| 0–3 | minigun | idle, then the three hot-barrel cells a burst cycles (`minigun_cycle_frame`) |
| 4–8 | missile launcher | 0–4 tubes empty (`missile_tubes_empty`) |
| 9–11 | plasma | idle, armed (the live weapon), firing |
| 12–14 | laser | idle, armed, firing (the lens flash) |
| 15–18 | flamethrower | pilot flame (two frames), firing (two frames) |
| 19–23 | grenade launcher | 0–4 rounds fired from its drum, front pair first; it sits on the missiles' roof hardpoint, since a tank carries one special at a time |
| 24–27 | sonic hammer | idle, two wind-up cells (the tell), the blast; an acoustic dish on the roof hardpoint (docs/sonic-hammer.md) |
| 28–32 | EMP burst | armed (the core's lamp dim), two crackle cells (the tell: the left, then the right half of the windings lit), the pulse (the whole ring lit), offline (the windings scorched, the core dark); a toroid coil of brass windings on a 7 x 6 plinth on the roof hardpoint (docs/emp-burst.md); its centre is `tank_art::EMP_COIL` |
| 33–39 | gauss rail | idle (the charge cells dark), one to four cells lit (the charge filling), full (the cells white, the rails' inner edges lit), the shot (the rails white, the cells dark); a capacitor block with four cells and two rails with a bore between them on the laser's cheek, which it shares since a tank carries one special at a time (docs/gauss-rail.md); its bore is `tank_art::RAIL_MUZZLE` |
| 40–43 | FPV swarm | armed (the screen dark, the link lamp dim), linked (the screen lit, the lamp white - alternating with armed at 4 Hz while one of its drones is up), the launch (the screen and the antenna white), offline (the screen dark, the antenna scorched, no lamp); a ground-control relay on the roof hardpoint - a squat gunmetal box with the FPV feed's screen on top and a panel antenna across its front (docs/fpv-swarm.md); the drones leave the halo, so it has no anchor |

A module is hardware, not a firing-mode indicator: `module_cols` draws one
for every special weapon the tank carries, in the order above, over the
turret; a wreck carries none. The shadow pass draws every layer (hull,
turret, modules) offset in flat black (`draw_tank_shadow`).

`src/tank_art.rs` holds where on the art things happen, in design pixels
from the pivot (x right, y toward the tail):

- `HEADLIGHTS` (hull frame) and `SPOTLIGHTS` (turret frame) - where the
  weather's light pass throws its cones from (`weather::lights`): every lamp
  at tiers 0–1, half of them at tier 2, none at tier 3; a spotlight follows
  the aim.
- `GUN_MUZZLES` - the main gun's tips at rest; held within a pixel of
  `tank_muzzle_forward_offset` by a test, since shells and plasma leave from
  the tunables.
- `MINIGUN_MUZZLE` - where bullets leave, boresighted onto the gun line
  `minigun_boresight_px` ahead (`Bullet::spawn`).
- `MISSILE_TUBES` - the four tube mouths in firing order, front pair first
  (`MISSILE_TUBE_OFFSETS` is each tube's place in the fan and the landing).
- `GRENADE_MUZZLE` - the launcher's barrel mouth, where a grenade leaves.
- `SONIC_MUZZLE` - the dish's face, where a tell's arcs gather. The wave
  itself is cast from the hull's pivot.
- `LASER_MUZZLE`, `FLAME_MUZZLE` - where the beam and the jet are **drawn**
  from. Both are judged along the gun line from `Tank::gun_line_muzzle`, so
  a side-mounted module never moves what they hit; the beam is drawn from
  the lens to where the gun line stopped, the jet from the nozzle to the
  end of its reach (`FlameJet::drawn`).
- `RAIL_MUZZLE` - the gauss rail's bore, where a slug's first leg is drawn
  from and a charge's motes gather. The slug is judged along the gun line
  too (docs/gauss-rail.md).

## 8. The light layer

`scifi_tanks_glow.png` and `tank_modules_glow.png` carry only the pixels
that shine: lamps, markers, sensors, engine and reactor glows, a module's
lens or hot barrel, sparks and embers - in the same cells as the paint,
which already has them at their daylight colours. `draw_tank_glow` adds them
over the field after the sky has multiplied it down, at `1 - day pools`
(`render/game.rs`), so they are invisible under a clear sky and every lamp
shines at night. A burning wreck's embers breathe.

## 9. Regenerating

```sh
cd tools/spritegen/tankdesign
nix-shell -p "python3.withPackages (ps: [ps.pillow])" --run "python3 export.py"
cd ../../.. && just check-sheets
```

`export.py` renders all five team blocks of every chassis (`BLOCKS`), the
modules and their anchors, and writes the four sheets and
`src/tank_art.rs`. The kit is the rest of the directory: `kit.py` (the
palette, materials, shading, parts and the builder), `damage.py` (the damage
tiers and wrecks from each part's tags), `render.py` (previews, hero sheets
and the review page's build), `lines/` (the four design lines of the study;
Vanguard ships) - see its `README.md`. After an export, re-pin the thumbnail
hashes (`thumbnail::tests`) consciously.

**Palette** (`tools/check_sheets.py`): the enemy block and the modules use
`PUNY_PALETTE_ALL` plus the kit's two deep water/teal steps
(`kit.TANK_EXTRA`); a player block may add its own team's ramp and lamp
colour and nothing of another team's.

**Track marks** are not in these sheets: tread marks come from
`static/tracks.png` (`track.rs`).
