# Larger maps: a follow camera, edge arrows and the whole screen

Status: research, partly built (section 14 says what has landed).
Written 2026-10 against v0.2.5.
It answers one question: how could maps much larger than one screen
play on phones, tablets, desktops and the web, with a camera that follows
the local tank and indicators for whatever is off screen, while cross-play
stays fair and the builder stays usable. It revisits the standing
constraint of docs/fullscreen-resolution-research.md ("the entire
battlefield is on screen at all times") and keeps it for arena-sized maps
only. An interactive companion page (the Camera Lab: device previews of
every rule, HUD layout and indicator below, plus a working builder, on a
96 x 54 study map, `maps/study/frontier.toml`, drawn by the game's own
renderer) was published with this research. The patterns shipped games
follow for the same problems are catalogued in docs/large-maps-patterns.md
and summarised in section 13. The questions it left open are decided;
section 15 lists the decisions.

Contents

1. Summary of the recommendation
2. Where the game stands
3. How much world a screen shows
4. Numbers per device
5. Fair fire: rows, columns and the sight box
6. The camera
7. What is off screen
8. The HUD
9. The builder on a large map
10. Cross-play and platform notes
11. Engine work
12. Large-map content and AI pacing
13. Patterns from shipped games
14. Plan
15. Decisions
16. Sources

## 1. Summary of the recommendation

- **Two map classes, decided by the map, not the screen.** A map no
  bigger than an arena (about 36 x 18 cells: the standard 34 x 17 maps
  and the 36 x 18 ones, seven of the fourteen levels) is shown whole on
  every device, exactly as today. Anything bigger is a *field map* and
  gets a follow camera. That includes the other seven levels (40 x 20 up
  to grand-campaign's 48 x 24), which an iPhone 16 draws today at 5.2 to
  6.2 mm, under the 44 pt floor, so the camera fixes them on phones. The
  class is room-wide, so every seat in a round sees the same kind of
  round. A map key (`view = "whole" | "follow"`) overrides the size
  default for any map.
- **The same world area on every screen in a room.** A field map shows
  about 578 cells (the standard 34 x 17 field's area) on every device in
  a room; the screen's shape decides only the outline, clamped to aspects
  between 4:3 and 2.4:1. Nothing is letterboxed from a 4:3 iPad to a 21:9
  monitor, every player gets the same amount of information and the same
  reaction time, and a phone keeps a tank of 7.5 to 9.3 mm. Played
  locally, a big monitor zooms out to about 40 x 22.5 cells, since no
  other screen shares its round.
- **A sight box every device shows, and enemies only fire from inside
  it.** Enemies shoot only along rows and columns (section 5), and a
  landscape phone cannot show the 340 px fire range above and below the
  tank. The room defines a box of +-11.5 x +-7.5 cells that every device
  must show, the camera never pushes it off screen, and the AI fires at a
  seat only from inside that seat's box.
- **The HUD moves into the top corners and the vitals onto the tank.**
  The world fills the screen edge to edge, under the notch and the home
  indicator; two small clusters sit inside the safe area at the top
  (Apple's guidance puts menus there, and the thumbs are at the bottom).
  Health is already a ring under the tank; ammo joins it. The bar stays
  as the builder's toolbar.
- **Edge arrows for threats and teammates, a lane warning, and a
  minimap for objectives on tablets and desktops.** Arrows sit where the
  line from the tank to the target meets an inset rectangle that avoids
  the HUD and the thumbs, scale and fade with distance, cluster with a
  count, and pulse when an enemy is lined up on your row or column.
- **The builder gets its own camera.** One finger paints, two fingers pan
  and pinch-zoom, a two-finger tap undoes, a minimap navigates, and on a
  touch screen a tap at a zoom where a cell is under about 6 mm zooms in
  instead of painting.
- **The engine draws the window, not the map.** Every field-sized render
  target becomes viewport-sized, drawing is culled, and the camera moves
  in whole 2 px blocks with the remainder applied when the frame is
  presented, which is what raylib's own `core_smooth_pixelperfect`
  example does.
- **Shipped games back this up.** Teeworlds ships the same-area rule,
  Diablo II: Resurrected removed wide screens for the problem the sight
  box solves, and Battlefield 4's scope glint is the lane warning;
  section 13 summarises 72 patterns from shipped games and what they
  changed here.

## 2. Where the game stands

From the code, with the places a camera touches:

- **No camera.** Pass 1 draws the field into a field-sized `scene_target`
  (`render/game.rs`), pass 2 blits it at the field origin (0, 32) with
  camera shake only, and `view::View::fit_capped` letterboxes the whole
  bitmap (field plus the 32 px bar) onto the window, capped at
  `view_max_scale` 1.5 on desktops. Comments in `render/game.rs`,
  `decal.rs` and `render/fx.rs` state that the game has no camera
  transform; the only `Camera2D`s are fixed at (0, 32), zoom 1.
- **The standard size is a phone constraint.** `lib.rs` documents 34 x 17
  as "the largest field a landscape phone still shows at a 7 mm tank".
  The largest shipped level is grand-campaign at 48 x 24; no map
  anywhere is bigger. On an iPhone 16 today a 48 x 24 map is a 5.2 mm
  tank and a 96 x 54 map would be 2.4 mm.
- **Field-sized GPU resources.** `scene_target` and `composite` (field
  plus bar, `app.rs`), three weather targets (`light`, `ground`, `lit`,
  `render/weather.rs`), the floor shade's `BlockImage` (half the field in
  each axis, `ground.rs`), and the three `RippleFx` shaders whose
  `resolution` uniform is the field. `static/plasma_orb.fs` and
  `static/flame_jet.fs` compute field coordinates from `gl_FragCoord` and
  `fieldHeight`; the shockwave works in field UV units, so its ring would
  grow with the map; the weather light pass has a sun gradient and a
  vignette across the field, the sky pass clears around seat positions.
- **Nothing is culled.** `ground::draw` and `draw_current` walk every
  cell every frame; every `paint_*` stage walks every entity; weather
  gathers a light for every tank and fire on the map.
- **Per-tick costs scale with cells.** `Terrain::build` clones the
  `WaterLayout` (two vectors of cols x rows) every update and every
  rendered frame on online clients; `route_grid` builds the nav grid,
  labels its components with a flood fill and runs one Dijkstra flow
  field per live seat plus the frog; every A* query allocates
  cols x rows arrays; `conceals` scans the grass cells linearly. A 96 x 54
  map has nine times the standard's 578 cells. Measured with the probe
  (`--scenario circle --frames 3600 --seed 1000`, release build, one
  2.8 GHz Xeon core): 0.16 ms a tick on grand-campaign (48 x 24) and
  0.61 ms on the 96 x 54 study map, `maps/study/frontier.toml`, roughly
  in proportion to the cells. That is negligible for one client, but a
  room server pays it once per room it holds.
- **The UI is framed by the field.** Banners, dims, dialogs, the lobby,
  the level select and the end screen are drawn in the field camera and
  hit-tested with `Layout::to_field`; the touch halves split the field
  rect; `touch_*` knobs are in bitmap pixels; text budgets are measured
  against a 768 px field. On a field larger than the screen all of this
  has to move to screen space.
- **The builder cannot pan or zoom.** A big map is shrunk with the
  bitmap: a 48 x 24 map on an 852 x 393 pt phone has 16 pt cells, a 16 pt
  bar and 24 pt popup rows. The wheel cycles a category's tools, only the
  first touch is read, and every painted cell calls `rebuild_ground` (a
  full `ground::build`, a shade re-bake and a re-upload).
- **Hard limits a big map meets.** Positions travel as `i16` quarter
  pixels (8191 px, about 255 cells a side); tile, fire and bonus keys are
  `u16` cell indices (cols x rows <= 65 535); the pickup bitmask is a
  `u64` (slots past 64 fall back to bonus entries); `LASER_MAX_RANGE` is
  4000 px, "longer than any battlefield diagonal" (a 96 x 54 diagonal is
  3525 px, a 110-cell-wide map breaks the comment); `wave_max_alive` caps
  live enemies at 31; `NEAREST_FREE_CELL_MAX_RADIUS` is 64 cells. The
  global effect caps (`SHOCK_MAX` 4, `SCORCH_MAX` 128, `DECAL_MAX` 256,
  `fx_max_particles` 900) are map-wide, so effects nobody can see would
  use them up.
- **The network already sends everything.** One delta per tick for the
  whole round, the same bytes to every seat; a snapshot costs about 14 B
  a tank and 10 B a shell, independent of map size. Only the `Welcome`
  grows with the map, because it carries the map's TOML (17 KB for
  `default.toml`, 41 KB for the 96 x 54 study map).

## 3. How much world a screen shows

Every rule below fills the screen; they differ in what decides the zoom.
Fairness here is co-op against the AI, not player against player: the
aim is that no device plays a harder or different game, not that nobody
ever sees an extra cell.

| Rule | Zoom from | Who ships it | For bongbong |
|---|---|---|---|
| **Same area** (recommended) | the screen's shape and a fixed world area, `w * h = A`, `w / h = clamp(aspect, 4/3, 2.4)` | Teeworlds (a constant view area with per-axis caps, read in its source); Wild Rift's fix for 4:3 tablets, "split the difference" (summary) | equal information and reaction time on every device; the outline follows the screen; desktops draw big tanks |
| Same box, cropped ("cover") | `max(W / boxW, H / boxH)` against one reference box | agar.io, diep.io and surviv.io (`max(H/1080, W/1920)`) | nobody sees past the box; a 4:3 tablet loses a third of its width |
| Letterbox one box ("keep") | `min(W / boxW, H / boxH)`, bars beyond | StarCraft II (16:9, bars on wider), Overwatch (21:9 capped at 103 degrees: "it would be unfair to 16:10 and 16:9 players"), Brawl Stars and Vampire Survivors (pillarbox wide screens) | fair, but spends the screen this work is meant to use |
| Same height (Hor+) | `H / rows` | League of Legends and Dota 2 on ultrawide | height is the scarce axis in landscape, so it keeps that equal, but phones and ultrawides see much further sideways |
| Same tank size | millimetres per world pixel | nobody, for a shared match | a 27" monitor sees three times a phone's world |
| Whole map (today) | the map | bongbong | right for arenas; 2.4 mm tanks on a phone for a 96 x 54 map |

**Why same area.** It is the one rule that keeps the amount of world, and
with it the time a shell takes to cross the screen, the same on a phone,
a tablet and a monitor, without bars between 4:3 and 2.4:1. The game's
own history argues for it: the 34 x 17 standard was chosen as what a
phone can show, and same area keeps exactly that much world on every
device. The default of 578 cells is that field's area.

**What it costs.** A 24" 1080p monitor draws a 35 mm tank, the size that
made the desktop cap necessary for the whole-map view
(fullscreen-resolution-research.md, "The desktop cap"). On a static board
that read as too big; in a scrolling view large sprites are the norm for
pixel-art action games (Enter the Gungeon renders 480 x 270 and scales
that up to the monitor), and the angular size is what changes: about 1.5
degrees for a phone tank at 30 cm against over 3 degrees for a monitor
tank at 60 cm. Local play answers it with a wider view on big screens
(below); a room keeps one area for all its seats.

**Local play on a big screen.** A round with no room, alone or two on one
screen, has nobody to be fair to, so a big screen zooms out. From the
shared view the whole-block zoom steps outward while the tank stays at
least 25 mm wide and the view at most 900 cells (40 x 22.5). A 24" or
27" monitor then shows 40 x 22.5 cells with a 26.5 to 30 mm tank, and a
21:9 or 32:9 monitor 43 x 18 cells with a 37 mm tank; laptops, tablets
and phones, whose tanks are under 25 mm already, keep the shared view.
The millimetres come from the monitor's reported size natively
(`GetMonitorPhysicalWidth`) and from the CSS reference pixel (0.26 mm) on
the web, where raylib reports none. Both numbers are knobs for a
playtest. The costs are accepted: a level is a little easier alone on a
monitor than on a phone, and the same monitor looks zoomed in when it
joins a room. A field map no larger than the wide view (Hedge Maze's
40 x 20, Harbor Lights' 40 x 22.5) shows whole there, because the camera
centres any axis the map does not fill.

**The aspect clamp.** Between 4:3 and 2.4:1 every screen fills edge to
edge. A 32:9 monitor is clamped to 2.4:1 and gets side bars, which is
where its HUD can sit.

**Pixel crispness.** The field is drawn in 2 px blocks (walls, props,
trees, tanks at 2x, the effects language), so the art stays crisp when a
block covers a whole number of device pixels: the scale in device pixels
per world pixel is a multiple of 0.5. On a 3x phone the whole-block steps
are about 20% apart, and the snap would either zoom in far enough to push
the sight box off screen (iPhone 16) or drop the tank under 44 pt; there,
a device pixel is 0.055 mm and a block of 4 or 5 pixels is invisible, so
phones above about 360 ppi keep the exact zoom. Below 360 ppi (tablets,
laptops, monitors, the 720p Android class) the zoom snaps to the nearest
whole-block scale, outward when inward would hide the sight box; the area
lands within about 12% of the target.

## 4. Numbers per device

The recommended rule (same area, 578 cells; whole-block snap under
360 ppi; corner HUD; app or full screen) on a field map. "Sight" is cells
from the tank to the screen edge with the camera centred; the box needs
11.5 sideways and 7.5 up and down. "Warning" is the time a shell (522.5
px/s) takes from the nearest screen edge.

| Device (landscape) | pt | World on screen (cells) | Tank | Block (device px) | Sight left-right / up-down | Warning |
|---|---|---|---|---|---|---|
| iPhone SE (3rd gen) | 667 x 375 @2 | 27.8 x 15.6 | 7.5 mm, 48 pt | 3.00 | 13.9 / 7.8 | 0.48 s |
| iPhone 16 | 852 x 393 @3 | 35.4 x 16.3 | 8.0 mm, 48 pt | 4.51 (exact) | 17.7 / 8.2 | 0.50 s |
| iPhone 17 Pro (16 Pro, 17) | 874 x 402 @3 | 35.4 x 16.3 | 8.2 mm, 49 pt | 4.62 (exact) | 17.7 / 8.2 | 0.50 s |
| iPhone 17 Pro Max | 956 x 440 @3 | 35.4 x 16.3 | 8.9 mm, 54 pt | 5.06 (exact) | 17.7 / 8.2 | 0.50 s |
| Pixel 9 | 923 x 411 @2.625 | 36.0 x 16.0 | 8.1 mm, 51 pt | 4.20 (exact) | 18.0 / 8.0 | 0.49 s |
| Galaxy S25 | 780 x 360 @3 | 35.4 x 16.3 | 8.1 mm, 44 pt | 4.13 (exact) | 17.7 / 8.2 | 0.50 s |
| Galaxy A06 (720p class) | 800 x 360 @2 | 33.3 x 15.0 | 9.3 mm, 48 pt | 3.00 | 16.7 / 7.5 | 0.46 s |
| iPad mini (A17 Pro) | 1133 x 744 @2 | 28.3 x 18.6 | 12.5 mm | 5.00 | 14.2 / 9.3 | 0.57 s |
| iPad Air 11" / iPad 11th gen | 1180 x 820 @2 | 29.5 x 20.5 | 15.4 mm | 5.00 | 14.8 / 10.2 | 0.63 s |
| iPad Pro 13" (M4) | 1376 x 1032 @2 | 28.7 x 21.5 | 18.5 mm | 6.00 | 14.3 / 10.8 | 0.66 s |
| Galaxy Tab S9 (11") | 1280 x 800 @2 | 32.0 x 20.0 | 14.8 mm | 5.00 | 16.0 / 10.0 | 0.61 s |
| Chromebook 11.6" | 1366 x 768 @1 | 28.5 x 16.0 | 18.1 mm | 3.00 | 14.2 / 8.0 | 0.49 s |
| MacBook Air 13" | 1470 x 956 @2 | 30.6 x 19.9 | 19.0 mm | 6.00 | 15.3 / 10.0 | 0.61 s |
| MacBook Pro 14" | 1512 x 982 @2 | 31.5 x 20.5 | 19.2 mm | 6.00 | 15.8 / 10.2 | 0.63 s |
| 24" 1080p | 1920 x 1080 @1 | 30.0 x 16.9 | 35.3 mm | 4.00 | 15.0 / 8.4 | 0.52 s |
| 27" 1440p | 2560 x 1440 @1 | 32.0 x 18.0 | 37.3 mm | 5.00 | 16.0 / 9.0 | 0.55 s |
| 27" 4K at 150% | 2560 x 1440 @1.5 | 30.0 x 16.9 | 39.9 mm | 8.00 | 15.0 / 8.4 | 0.52 s |
| 34" ultrawide 21:9 | 3440 x 1440 @1 | 35.8 x 15.0 | 44.3 mm | 6.00 | 17.9 / 7.5 | 0.46 s |
| 49" 32:9 (clamped) | 5120 x 1440 @1 | 36.0 x 15.0 | 44.7 mm | 6.00 | 18.0 / 7.5 | 0.46 s |

Played locally, the five monitors zoom out (section 3): the 24" 1080p,
27" 1440p and 27" 4K at 150% show 40 x 22.5 cells with a 26.5, 29.8 and
29.9 mm tank, the 34" and 49" ultrawides 43 x 18 cells with a 37 mm tank.
Every other device in the table keeps its row.

For comparison, today's whole-map rule on the same devices, tank size for
the standard 34 x 17 map / grand-campaign 48 x 24 / the 96 x 54 study map:

| Device | 34 x 17 | 48 x 24 | 96 x 54 |
|---|---|---|---|
| iPhone SE | 6.1 mm | 4.3 mm | 2.1 mm |
| iPhone 16 | 7.2 mm | 5.2 mm | 2.4 mm |
| iPhone 17 Pro Max | 8.1 mm | 5.8 mm | 2.7 mm |
| Galaxy A06 | 7.8 mm | 5.6 mm | 2.5 mm |
| iPad Air 11" | 13.4 mm | 9.5 mm | 4.7 mm |
| iPad Pro 13" | 15.6 mm | 11.0 mm | 5.5 mm |
| MacBook Air 13" | 17.1 mm | 12.1 mm | 6.0 mm |
| 24" 1080p | 26.5 mm | 22.1 mm | 10.8 mm |
| 27" 1440p | 22.4 mm | 22.4 mm | 12.2 mm |

Device data: Apple and Android developer documentation, manufacturer
specifications and viewport aggregators (section 16). Landscape safe
areas: 59 pt each side and 21 pt at the bottom on the iPhone 16, 62 pt on
the 16 Pro and Pro Max class, 20 pt at the bottom on iPads with the status
bar hidden, nothing on the iPhone SE. Android punch-hole insets and every
browser-chrome height in the lab are estimates; a page should measure
`visualViewport` rather than trust a table.

## 5. Fair fire: rows, columns and the sight box

Enemies only shoot when lined up on a cardinal axis. `ai.rs`'s
`act_attack`: "Hold near the target and shoot when lined up on a cardinal
axis. The tank only fires after staying aligned for ENEMY_AIM_SETTLE, and
stops to aim." The numbers: within `enemy_fire_align_px` (24 px) of the
axis, within `enemy_attack_range` (340 px), with line of sight, after
`enemy_aim_settle` (0.25 s), every `enemy_fire_interval` (1.2 s) or so.
Chasing starts much further out, at `enemy_view_range` (800 px; 480 at
night or in a storm, 360 in fog). So **the only threats are on the
player's own row and column**, and two facts follow:

- **Horizontal is easy, vertical is not.** Every rule above shows more
  than 340 px sideways on every device. Up and down, a landscape phone
  with a tank no smaller than 44 pt can show at most `H/2 * 64/44` world
  pixels: 273 px on an iPhone SE, 286 on an iPhone 16, 320 on a Pro Max,
  262 on a Galaxy S25 or A06. Under any rule that keeps phone tanks
  readable, some enemies can shoot a phone player from above or below the
  screen. Today the whole map is visible, so this never happens; a camera
  would introduce it.
- **The warning window exists.** The 0.25 s settle plus the flight time
  (0.46 s from 240 px) is time a lane warning can use (section 7).

The fix is a room rule, never a per-device one: **the sight box**,
+-11.5 x +-7.5 cells (+-368 x +-240 px) around each seat.

1. Every device shows it. The view rule guarantees it (the whole-block
   snap steps outward rather than hide it, section 3), and the camera's
   look-ahead may only spend the room outside it (section 6).
2. An enemy fires at a seat only from inside that seat's box. Sideways
   this changes nothing (340 < 368); up and down an enemy closes from 340
   to 240 px before it shoots. The engagement ring's vertical slots move
   in to match (272 to 224 px).
3. It is a gameplay constant in the room's tuning patch, like
   `wave_size_scale`, so it is deterministic, the same for every client,
   and never derived from any client's window. Vampire Survivors shows
   why that matters: its spawn edge is derived from the view rectangle,
   and the community's ultrawide fix had to add an "enemies spawn at 16:9
   position" option.

Spawns and wave gates follow the same box: an enemy appears outside every
seat's box, never outside "the screen". The rule changes AI balance, so
it lands with a `just probe-fixtures` re-baseline, consciously, and a
probe metric for "fired from outside the target's box" that should read
zero. The lab counts shots fired at the local player from off its own
screen, with the rule on and off; its AI is a stand-in with the real
ranges and fire rule, so the count shows the mechanism, not a balance
figure.

## 6. The camera

Presentation only: nothing in `simulation/` sees it, a replica and the
local round use the same code, and the dev server reports it.

- **Follow the local seat.** The base point is the tank, held inside a
  dead zone of about 0.4 cell so the 4-way corrections and the collision
  slides do not wobble the view (Keren's camera window).
- **Look ahead in the facing direction**, ramped by speed (35% at rest,
  because you fire where you face), eased over about 0.5 s so a turn does
  not whip the view, and **never further than the room outside the sight
  box**. On a phone that is up to six cells sideways and almost nothing
  up or down; on a tablet it is more even. The facing used is
  `Tank::rotation`, which snaps on a direction change (not the eased
  `visual_rotation`), with a short hold before a reversal flips the
  look-ahead (Keren's dual forward focus).
- **Smooth with a critically damped spring** (`SmoothDamp`, about 0.16 s),
  frame-rate independent. A spring trails a target moving at constant
  speed by about `v * halflife / ln 2` (30 px at 210 px/s and 0.1 s);
  feeding the tank's velocity forward as the spring's goal velocity
  removes most of it.
- **Clamp to the field.** The view never shows past the boundary walls on
  an axis where the map is larger than the view; where it is smaller (a
  short, wide field map on a tall tablet) that axis centres.
- **Whole blocks, smooth motion.** Render the visible window plus one
  block of margin into a viewport-sized target at one world pixel per
  texel, with the camera snapped to whole 2 px blocks, and shift the
  presented image by the remainder times the scale, rounded to device
  pixels. raylib's `core_smooth_pixelperfect` example is this technique
  (the world camera takes `truncf(target)`, a screen-space `Camera2D`
  applies the fraction); Celeste's motion-smoothing mod and
  `bevy_smooth_pixel_camera` do the same. The existing shake already
  snaps to 2 px.
- **Shake after the follow**, still snapped to blocks, scaled by
  `screen_fx_intensity`, attenuated by distance from the camera (surviv.io
  shakes fully within 10 units and not at all past 40) so a blast across
  the map does not shake your screen, and switchable off (Apple's "make
  motion optional", Xbox accessibility guideline 117).
- **Cuts, not pans**, for a teleport through a portal, a seat re-entering
  through a gate (decision 7 of the co-op PRD) and a restart; a pan across
  half the map is disorienting at this pace.
- **Spectate while wrecked.** In a wave round a wrecked seat waits for the
  next wave: the camera follows the nearest live teammate and an arrow
  marks the gate the seat will come through.
- **An establishing shot** at round start: the whole map for about a
  second (the frog, the gates), then a cut or a fast zoom to the tank.
- **Couch play** (desktop only, two seats on one screen): one shared
  view while both tanks' sight boxes fit in it, then a dynamic (Voronoi)
  split along the line between them, each half following its seat at the
  local view's zoom (section 3), with a 2 px divider. No zoom-to-fit:
  zooming breaks the block grid, and a shared screen that holds players
  at its edge pins a tank there under fire (the complaint about
  Gauntlet's and Smash's cameras). The LEGO games and Godot's
  split-screen demo do the split.

## 7. What is off screen

Research on off-screen indicators (Halo, CHI 2003; scaled arrows,
MobileHCI 2006; Wedge, CHI 2008; EdgeRadar, CHI 2007; arrows against
wedges against an overview map, 2011) agrees on a few things: arrows that
encode distance by size beat numeric labels for judging distance,
clutter needs aggregation, moving targets are tracked better at the
screen edge, and an overview map beats arrows for finding clusters.
Peripheral vision picks up brightness and motion, not detail or hue
(Player Research), so urgency is a flash or a pulse, never a colour
change alone.

- **Placement.** Cast a ray from the tank's screen position to the
  target and stop it at an inset rectangle: the world's visible rect,
  inside the safe area, about 10 pt in. Where that point lands in a HUD
  cluster, the minimap or a thumb's rest (a 12 mm pad about 22 mm in from
  each side and 18 mm up), slide it along the edge until it is clear.
- **Distance** by size and opacity: full at one screen away, 60% size and
  55% opacity at four. The frog's arrow carries its distance in cells.
- **Clustering and priority.** Enemies whose arrows land within about
  22 pt merge into one arrow with a count; at most eight arrows, filled by
  priority: lane threats, teammates, the frog, then the nearest enemies,
  the rest folded into one count per edge. Teammates and objectives are
  never merged away.
- **The lane warning.** An enemy that is lined up on your row or column
  inside the fire range pulses red at the edge, on the screen of the seat
  it is lined up on only, from the moment its aim starts to settle; the
  frame it fires, its arrow flashes white. That is 0.25 s of settle plus
  the flight time in which to step out of the lane. Battlefield 4's scope
  glint, visible only near the sniper's line of aim, is the same idea.
- **Where the hit came from.** A red arc on your own tank points at
  whoever just hit you, for most of a second: bongbong has no audio,
  the channel most shooters use for off-screen danger.
- **Teammates** in their ring colour with their `P2`..`P8` label; a
  wrecked teammate waiting at a gate shows there.
- **The frog** (Protect) in green with its distance; the enemy frog
  (Hunt) as the objective.
- **Wave gates** flash amber for a few seconds when a tank rolls in, using
  the `WaveStarted` and `TankEntered` events nothing renders today.
- **Concealment holds.** An enemy in tall grass gets no arrow unless it
  fired in the last 1.5 s or is within about two cells, the same rule its
  sprite follows; when it slips into the grass it leaves a hollow
  last-seen marker that never moves. At night or in fog, arrows follow
  the sky's sight factor for the same reason.
- **The minimap** is for objectives and clusters: the field at a glance,
  the view rectangle, teammates, the frog, enemies that are not
  concealed. On by default on tablets and desktops under the top-right
  cluster, and off on phones, where the edge arrows carry the field. In
  the builder it is the navigator on every device.
- **No interest management.** Arrows need every enemy's position, the
  snapshots already carry them, and at 8 seats and 31 live enemies the
  bytes do not justify it (surviv.io's server reportedly sends only what
  is inside each player's view radius).

## 8. The HUD

The HUD is drawn in screen space at a UI scale from points, never from
the world scale, with text at 11 pt or more and hit areas of 44 pt (Apple)
or 48 dp (Android) on touch screens. Four layouts were compared; the lab
draws all four. Share of the screen the HUD covers (and how much of the
screen the world gets):

| Layout | iPhone 16 | iPad 11" | 24" 1080p | Notes |
|---|---|---|---|---|
| **Corner clusters** (recommended) | 4.5% (world 100%) | 2.1% | 1.0% | over the field, in the top corners, inside the safe area |
| **On the tank** (phone option) | 1.4% (world 100%) | 0.7% | 0.3% | health ring and ammo pips at the tank, one corner chip for the wave and the menu |
| Top bar in screen space | 7.6% (world 92%) | 4.2% | 3.2% | costs height, the scarce axis in landscape, and the sight box with it |
| Side rails | 15.5% (world 85%) | 12.9% | 14.4% | the phone's island insets are unsafe anyway, but tablets and monitors lose width |
| Today, standard map | bar 4.8%, letterbox 13% (world 82%) | | | the bar scales with the bitmap: 21.8 pt tall on an iPhone 16 |

**Why the corners.** In a follow camera the corners are the pixels
furthest from the tank, the least valuable on the screen; in today's
whole-map view they are as valuable as any. Apple's guidance for games
puts menus at the top, keeps controls out of the Dynamic Island and the
home indicator, keeps the centre clear, and anchors each section to a
side at a constant size rather than scaling the whole UI. The thumbs rest
at the bottom corners, where the floating stick already lives.

- **Top-left:** health (number and gauge), shells, the weapon queue with
  the active weapon outlined.
- **Top-right:** the level and wave with the enemy count, then the
  buttons (levels, BUILD, ONLINE or LEAVE), then the seat chips of a room,
  then the minimap (not on phones).
- **On the tank:** the health ring the player tank already draws
  (`RingStyle::Gauge`), plus ammo pips on its lower arc, so the number
  that matters most never needs a glance away.
- **Fade under action:** a cluster drops to about 35% opacity while a
  tank or a shell is under it.
- **The bar remains** as the builder's toolbar, where tools matter more
  than pixels, at 44 pt on touch screens.
- **Banners, dialogs, the lobby, the level select and the end screen**
  move from the field camera to screen space; their geometry tables stop
  depending on the smallest shipped field.

## 9. The builder on a large map

The builder edits the map, not the round, so it gets its own free camera,
independent of play's; BUILD opens it on what play was showing, PLAY
starts the round as today.

| Action | Mouse and keys | Touch |
|---|---|---|
| Paint (toggle-erase rule unchanged) | click or drag | one finger, once it has moved past the slop |
| Pan | middle drag, or Space + drag (Tiled); the arrows. The right button still erases | two-finger drag, or the pan tool for one hand |
| Zoom | wheel at the cursor; `+` / `-` | pinch at the fingers |
| Undo / redo | Ctrl+Z / Ctrl+Y | two-finger tap / three-finger tap (Procreate, Pixaki) |
| Overview | FIT button | FIT, or a quick pinch out (Procreate) |
| Navigate | click or drag the minimap (Tiled's mini-map dock) | tap or drag the minimap |

The zoom steps on whole 2 px blocks (half a device pixel per world pixel)
on a coarse screen and is smooth on a fine one, as play's framing is
(section 3); a pinch eases onto the nearest whole-block scale when it
lifts. The canvas is drawn at a texel per world pixel into its own
target and that is scaled to the window, so a fractional zoom never
samples across a tile's edge; at FIT on an arena it is drawn exactly as
before the camera existed. A field map's builder bitmap is the window's
shape with the bar at the standard arena's size (`editor::camera`).

- **A paint threshold on touch.** Below a cell of about 6 mm a finger
  cannot hit one cell; there a tap zooms in to a 9 mm cell at that point
  instead of painting, and a drag pans. Geometry Dash's editor solves the
  same problem with an explicit paint/pan switch; Super Mario Maker 2 has
  a zoomed-out view mode for the overview.
- **Edge scroll while painting.** A stroke held near the canvas's edge, or
  past it, scrolls the camera and keeps painting, so a long wall does not
  need a pan in the middle.
- **Map size becomes a setting.** A cols x rows stepper in the MAP panel,
  with an anchor for where the old map sits, capped by the wire limits
  (section 2) and by `LASER_MAX_RANGE` until that becomes a range.
- **Raw touches, not raylib's gestures.** raylib's gesture module reads
  only two touch points and has 0.3 s thresholds; the builder needs the
  pointers themselves (`editor::gesture`).
- **Incremental ground.** A painted cell re-bakes the ground and the floor
  shade around that cell, not the whole field (`GroundGrid::repaint`), and
  the GPU copy of the shade takes only the blocks that changed. On the
  96 x 54 study map a cell of a wall stroke went from 53 ms and a 5.3 MB
  upload to 0.3 ms and 16 kB in a debug build.
- **Thumbnails in the Load list** from `mapshot`, since a large map's name
  says less about it than its picture.

## 10. Cross-play and platform notes

- **The view never enters the simulation.** The camera, the HUD and the
  arrows are per client. What must agree across clients is a room rule:
  the map's class, the view area, the sight box. They travel in the
  room's tuning patch next to `wave_size_scale`, so a replica and its
  sandbox resolve them as the room does. A round with no room uses the
  local view (section 3).
- **The AI, the spawns and the gates use the sight box**, never a client's
  window.
- **Online indicators need nothing new on the wire**: the snapshot already
  carries every tank, shot and frog.
- **Android 16 ignores the landscape lock on large screens.** Apps
  targeting API 36 have `screenOrientation`, `resizeableActivity` and
  aspect-ratio limits ignored on displays 600 dp and wider, unless
  `android:appCategory` marks them as a game. bongbong targets 36 with
  `sensorLandscape` and declares `android:appCategory="game"`
  (`tools/android/AndroidManifest.xml`), which keeps the lock; the
  any-aspect camera is the long-term answer for windows.
- **iPadOS 26 deprecates `UIRequiresFullScreen`** (bongbong sets it) and
  scales a windowed app's scene when the user resizes it. A camera that
  fills any aspect handles windowed iPads, Stage Manager and freeform
  Android windows without a special case.
- **Google Play's Level Up guidelines** ask landscape games to fill 4:3,
  16:10 and 21:9 screens without letterboxing and to keep UI clear of
  cutouts and system bars. Field maps already fill; arenas should draw
  their margins as out-of-bounds ground rather than flat bars.
- **Mobile Safari** has no element fullscreen on iPhone and keeps its
  toolbar on a page that never scrolls; `viewport-fit=cover` with
  `env(safe-area-inset-*)` is already in the page. The camera removes the
  need for the canvas box to keep the bitmap's shape, because the view
  fills whatever box it gets; raylib's touch mapping then only needs the
  canvas to fill its box exactly.

## 11. Engine work

| Area | Change |
|---|---|
| `view.rs` | A `Camera` beside `View`: the visible world rect, the scale in device pixels per world pixel, the sub-block remainder; `to_world` for pointers. `View` keeps letterboxing arenas and clamped 32:9 screens. |
| `app.rs`, `render/game.rs` | `scene_target`, `composite` and the weather targets sized to the window, not the field; pass 1 inside a `Camera2D` with the snapped target; pass 2 presents with the remainder offset. |
| Shaders | `plasma_orb.fs`, `flame_jet.fs` (and their `static/web/` twins) take a camera origin uniform instead of assuming the target is the field; the shockwave works in world pixels, not field UV; the weather light pass's sun gradient in world space and its vignette in screen space; the sky pass's seat clearings through the camera. |
| Culling | `ground::draw`, `draw_current`, every `paint_*` stage, weather lights and occluders, and the effect lists skip what is outside the view plus a margin (the largest light or blast radius). |
| Effect caps | Built for the marks: `SCORCH_MAX` and `DECAL_MAX` are world state and capped the whole map, so a field map wore away marks still on screen; `Game::mark_caps` keeps them on an arena and scales them by area on a field map (the study map keeps nine times as many). Particles and shocks are client-side and can prefer what is on screen. |
| Weather fallback | Built: a device whose weather shaders fail drew every sky clear, so its player saw through night and fog. It now draws the sky without them (`weather::plain`, docs/weather.md "Without shaders"): the light map, which needs no shader, multiplied onto the field by a blend mode - the night as dark, with every headlight and shadow - and the snow on the ground and the fog, sand, rain and snow in the air as plain blocks from the shaders' own noise; `status.weather.without_shaders` reports it and the `weather_without_shaders` knob shows it anywhere. The halved `fx_density` on phones thins only cosmetic smoke: no rule hides a tank behind smoke (concealment is the tall grass's cell test). |
| UI | Banners, dialogs, lobby, level select, end screen and the stick move to screen space at a UI scale in points; `touch_*` knobs move from bitmap pixels to points. |
| Simulation | `Terrain::build` borrows the water layout instead of cloning it; flow fields limited to a radius around each seat or updated every few ticks on field maps; A* scratch arrays reused. |
| Wire | Built: a map is at most `map::MAX_SIDE_CELLS` (250) cells a side, refused by name past it, so its far corner, a wave tank a tank length beyond it and its last cell index all fit the wire (positions in quarter pixels as `i16`, cell indices as `u16`); a laser is traced past the field's diagonal (`simulation::laser_reach`, the fixed 4000 px wherever that already spans the field, so those beams are bit for bit the same) by the room and by a client's drawn beam alike. The `Welcome`'s map TOML grows with the map: the 96 x 54 study map's is 42 KB, far inside the sockets' limits. |
| Dev server | `status` reports the camera (rect, scale, block, follow target); `screenshot` keeps capturing what the window shows; a `camera` tool to pin a view for screenshots. |
| Memory | Five field-sized RGBA8 targets for a 96 x 54 map are about 106 MB; window-sized, about 12 MB. Chrome on Android caps WebGL textures at 8192 (4096 below Android 14); iOS Safari allows 16384 on A9 and later. A low-end Mali-G52 has about 14 GB/s of bandwidth, and writing a 64 MiB target sixty times a second is a quarter of it. |

## 12. Large-map content and AI pacing

- **The shared alert is map-wide.** Any enemy within sight of a player
  alerts the whole pack, and patrolling enemies head to the alert from
  anywhere. On a field map that drains every patrol on the map toward one
  fight; the alert needs a radius (a few screens) or a chain (enemies
  alert neighbours within sight), and guards need a home and a leash.
- **Far enemies should sleep.** An enemy with no seat or frog within a
  few screens of path distance can stop thinking and routing until a
  hit, an alert or a seat wakes it, and the rest can think every k-th
  tick, staggered by owner slot. Both are pure functions of positions,
  so replays hold; the probe re-baselines.
- **Band spawns are fractions of the shorter side** (0.272 to 0.4), which
  on a 54-row map is a ring 470 to 690 px in from the edges. Field maps
  need spawns relative to the seats: outside every seat's sight box,
  within a few screens.
- **Wave gates far from everyone** make a wave a long walk;
  `wave_gate_min_player_dist` needs a maximum to go with it, or gates
  chosen by distance to the nearest seat. In the probe run above, one wave
  tank on the study map took 52 s to engage against a 13 s ideal route
  (61 cells); on the standard map the same figures are a few seconds.
- **Landmarks.** A screen-sized view of a large map needs features that
  say where you are: rivers, roads, a town, a fort, each region's
  material palette. The study map used by the lab is laid out that way.
- **Objectives across the map** (the frog, pickups, gates) are what the
  minimap and the objective arrows are for; a Protect field map should
  keep the frog within a couple of screens of the start.

**Built for field maps** (`simulation::field`, `Game::field_map`; an arena
takes none of it and replays as before): an enemy that sees a seat alerts
the enemies within `enemy_alert_chain_px` of itself and they pass it on,
with no line of sight, instead of the map-wide alert; an enemy with no
alert, call or hit keeps within `enemy_leash_px` of where it first stood
and turns back past it; a wave tank is called to the nearest live seat
until it first has a seat in sight range or is hit - without the call a
wave leashed to its gate a walk from the fight would never reach it;
farther than `enemy_far_px` from every live seat and the players' frog an
enemy thinks every `enemy_far_think_ticks` ticks, staggered by owner slot,
and one nothing has woken does not think or route at all; band spawns and
wave gates stay outside every seat's sight box, a wave taking the gates
whose walk from the nearest seat is within `field_walk_slack_seconds` of
`field_walk_seconds` (15 s) where the map has any and a band leaning that
way (a 40-wide level has no walk that long, and only keeps its spawns out
of sight); a fallen seat comes back through the free gate nearest the
living seats. On the study map, 30 AFK rounds, a wave tank's walk to the
fight went from a median 14.8 s, p90 34.5 s and worst 47.5 s to 11.6 s,
20.8 s and 30.2 s, and with every round run to two minutes a tick costs
0.59 ms against 0.92. `just probe-fields` sweeps the study map and the
five 40-wide levels. Not built: the pacing director, re-rolling a
straggler through a nearer gate, and flow fields bounded to the bubble -
the frame's routing grid, about 0.33 ms of the 0.59, is now most of a
tick.

**The first field map** is `longwater` (`maps/longwater.toml`, free play,
80 x 45; its header says how it plays): a fort on the south shore of a
lake that crosses the whole map, a ford at each end and a causeway either
side of the fort, six wave gates north of the water 10 to 14 s down the
roads from the fort, and two on the south road, 8 s out, which only a
fallen seat comes back through (`field::nearest_gates`). Laying it out
found two rules for a field map. A tank turns off its road for any pickup
it wants inside its leash (`Brain::seek`), so whatever a fresh enemy
fetches - laser, plasma, minigun, missiles, speed and shield, and the
health packs a bonus shield turns up beside - belongs beyond every gate's
`enemy_leash_px`: a laser on the street the east waves came down drew them
up it. And a tank stops being called once it has a seat in sight, so a
crossing far from the fight strands it: sent round the lake by a far ford,
it is out of sight longer than `enemy_alert_hold_seconds`, loses its alert
and drives home - the causeways keep the usual way across within sight.

Still open, and the most visible fault on a map that size: a hull riding
the far edge of a nav row, nearer the next row's centre than
`ai_dir_switch_margin_px`, can never beat the margin in `Ai::steer_toward`
for a step into that row, so it drives past every turning its route
takes, to and fro between two walls, for the rest of the round. On
longwater with a perfect defence - every enemy destroyed the moment it
comes within 400 px of the seat or the frog - four rounds in six still had
a tank out after seven minutes: three driving up and down the east gates'
road, the fourth back home on its leash, its alert run out. Letting a
field map's hull take such a step on any gain at all cleared every
stranded walk from 30-round sweeps of longwater and the study map, but
about doubled `probe-fields`' jitter on the 40-wide levels, past their
ceilings; taking it only where the margin cannot be beaten still raised
hedge-maze, archipelago and harbor-lights past them, and so did that
gated to hulls out of every seat's sight, which also left the tanks
stranded within sight of the fight.

## 13. Patterns from shipped games

docs/large-maps-patterns.md catalogues 72 patterns from shipped games,
each with its sources, how far each source was verified, and what
bongbong should take: 40 to adopt, 20 to adapt, 4 bongbong already
follows, 8 to skip. The ones that confirm the recommendation above:

- **Same area has shipped.** Teeworlds keeps its view area constant
  whatever the window's shape, with per-axis caps; Wild Rift answered
  tablet players who saw less than phones by splitting the difference,
  more vertical view and less horizontal. Overwatch caps 21:9 for that
  reason in its director's words; StarCraft II and Brawl Stars cap or
  pillarbox wide screens too, for reasons their players report.
- **The sight box has a precedent, in reverse.** Diablo II: Resurrected
  removed ultrawide support because its monsters react only inside the
  original view, so wide screens hit monsters that never answered; shoot
  'em up design has long held that off-screen enemies should not fire.
- **The lane warning is a scope glint.** Battlefield 4 shows a glint only
  near the sniper's line of aim, and Battlefield's lock warning beeps
  while a lock builds and goes solid when it lands.
- **Hidden tanks that fire are revealed, briefly.** Call of Duty's red
  minimap dots, League of Legends' two-second reveal from brush, Brawl
  Stars' bushes that hide only beyond two tiles.
- **The HUD layout is the genre's.** Honor of Kings, Pokémon Unite and
  PUBG Mobile put information in the top corners and leave the bottom
  ones to the thumbs, as Apple's guidance asks; Brawl Stars draws health
  and ammo on the brawler.
- **The pixel camera is a known technique.** Godot's pixel camera and
  raylib's own example snap the camera and slide the finished picture by
  the remainder; Celeste closes a fixed fraction of the gap per second.
- **Spawning out of sight and within reach** is how Vampire Survivors,
  Terraria and Left 4 Dead keep fights near the players.

What the catalogue changed or added:

1. **Couch play splits rather than zooms** (section 6).
2. **A failed weather shader must not clear the sky** (section 11):
   today it draws night as day, the "low settings see more" problem
   Counter-Strike 2, PUBG and Rust had to patch.
3. **Arrows by priority, warnings by target, a hit arc, last-seen
   markers and a proximity reveal** (section 7).
4. **A probe for the reverse unfairness:** count seat hits on enemies
   outside that seat's sight box, which cannot fire back until they close
   in.
5. **Bounded AI on large maps** (section 12): chained alerts with leashes
   (World of Warcraft, Diablo II, Helldivers 2), a simulation bubble and
   staggered thinking (Minecraft, Cataclysm: DDA, Unreal's significance
   manager), spawns and gates by path distance outside every sight box
   with stragglers re-rolled, re-entry through the gate nearest the
   living seats (Battlefield, Halo: Reach), a pacing director with relax
   phases (Left 4 Dead), and field maps authored as lanes so the walk
   from a gate to the fight stays near 15 s.
6. **HUD sizes in points.** Today the bar's 10 px labels render at about
   6.8 pt on an iPhone 15 and the 44 px lobby buttons at 27 to 32 pt,
   under Apple's 11 pt text and 44 pt target floors. Add a UI scale
   separate from the world zoom (Stardew Valley, Terraria), hints that
   follow the input last used, a fade while play is under a cluster, and
   seat numbers on every chip and arrow (eight colours cannot all stay
   distinct). The stick side stays a build-time choice (section 15).
7. **Arenas draw their margins.** Google Play's Level Up guidelines and
   Apple's WWDC24 advice both ask games not to letterbox; arenas fill
   their margins with out-of-bounds ground beyond the boundary walls
   instead of flat bars.
8. **The builder grows large-map tools:** play from here (Unreal,
   Ultimate Doom Builder, Super Mario Maker 2), a lint panel that jumps to
   each finding with quick fixes (Ultimate Doom Builder), a clear check
   before a custom map can be hosted (Super Mario Maker 2, Trackmania),
   rectangle select and stamps, fills and a scatter brush (Tiled), a
   resize anchor (Tiled, Wesnoth), touch slop before a stroke commits,
   and thumbnails rendered on save (OpenRA).
9. **One motion switch,** seeded by the OS's reduce-motion setting:
   shake at off, 50 % or 100 %, attenuated by distance from the viewer
   (OpenRA); the establishing shot on a level's first play only, and
   skippable.

The Camera Lab demonstrates several of these: the establishing shot,
play from here, a loupe over the painted cell, the hit arc and the
last-seen markers.

## 14. Plan

Each step ships alone. Step 1 changes no picture; step 2 changes only
field maps, which include the seven levels bigger than 36 x 18; step 3
changes every HUD; step 5 changes the AI everywhere and re-baselines the
probe.

1. **The camera seam.** A presentation `Camera` that, for every map
   shipped today, is the identity: window-sized targets, culling, the
   shader uniforms, the UI in screen space. Screenshots of every shipped
   map must not change.
2. **The follow camera for field maps.** The map class and the `view`
   key, the same-area rule with the aspect clamp and the whole-block
   snap, the local wide view on big screens, the sub-block present,
   look-ahead inside the sight box, cuts, spectating, the couch split,
   one motion switch seeded by the OS; `status` reports it.
3. **The HUD in the corners.** Corner clusters, vitals on the tank, a UI
   scale and minimum sizes in points, input-aware hints, seat numbers, a
   fade under play; arenas draw their margins; the bar stays for the
   builder.
4. **Off-screen awareness.** Edge arrows by priority, the targeted lane
   warning, the hit arc, last-seen markers, gate flashes, the minimap on
   tablets and desktops.
5. **Fair fire.** The sight box as a room rule; the AI fires only from
   inside it; spawns and gates follow it; a weather fallback that keeps
   the dark; `just probe-fixtures` re-baselined with probe checks that no
   shot comes from outside the box and counting hits on enemies outside
   it.
6. **The builder.** Its camera, gestures with touch slop, the loupe, the
   minimap navigator, the paint threshold, play from here, the lint panel,
   the clear check, select and stamps, fills and scatter, the map size
   setting with an anchor, incremental ground rebuilds, thumbnails.
7. **Large-map content.** Chained alerts with leashes, sleeping and
   staggered far enemies, spawns and gates by path distance, re-entry
   near the team, a pacing director, effect caps, a first field level,
   the study map in the probe sweeps, and the Android `appCategory` fix.

Landed so far: step 1 whole (`view::Camera`); from step 2 the view rules
(`framing.rs`), the map's `view` key and the follow camera itself
(`follow.rs`: the dead zone, the look-ahead inside the sight box, the
spring, the sub-block present, cuts, spectating a teammate, the couch's
shared view, `status.camera`) - the split screen, the establishing shot
and the motion switch are still to come; step 3's HUD in the corners
(`hud::corners` inside the safe area at a UI scale in points, the play
bar gone, ammo pips on the tank, a cluster fading while play is under
it, banners, dialogs, the end screen, the lobby and the level select in
window space) - arenas drawing their margins and input-aware hints are
still to come; from step 4 the awareness model
and its drawing (`indicators.rs`: edge arrows by priority, the lane
warning, the hit arc, last-seen marks, gate flashes, thumb rests) and
the minimap on tablets and desktops (`minimap.rs`: a texel per cell,
patched where a tile dies, under the right cluster as a slot of
`hud::corners`); from step 5 the sight box, the AI's fire gate,
the probe's `offbox-fire` check and the weather fallback
(`weather::plain`); from step 6 the builder's own camera
(`editor::camera`: zoom at the cursor, pan, FIT, BUILD opening on what
play showed), its gestures from raw touch points with a slop
(`editor::gesture`), the paint threshold, edge scroll, the map size
setting with its anchor, incremental ground (`GroundGrid::repaint`) and
the minimap navigator (`MapEditor::navigator_rect`, repainted per
stroke) - the loupe, play from here, the lint panel, the clear check,
select and stamps, fills and scatter and thumbnails are still to come; from step 7 the Android `appCategory`, maps capped at
what the wire carries, the laser's reach, the mark caps and the bounded
AI (`simulation/field.rs`: chained alerts with leashes, far enemies
thinking less, spawns and gates by walk outside every sight box,
re-entry through the gate nearest the living seats, `just
probe-fields`) and a first field map, free play rather than a level
(`longwater`, section 12) - the pacing director, stragglers re-rolled
through a nearer gate, flow fields bounded to the seats and a turn a hull
on the edge of its row can take are still to come.

## 15. Decisions

The questions this research left open, as decided:

- **The area.** 578 cells, the standard 34 x 17 field's area, on every
  screen in a room: phone tanks of 7.5 to 9.3 mm, 35 mm on a 24" 1080p
  monitor.
- **Local play on a big screen** zooms out on its own (section 3):
  40 x 22.5 cells on a 24" or 27" monitor. There is no setting, and
  online rooms always use the shared area.
- **Arena threshold.** Maps up to 36 x 18 are shown whole; the five
  40-wide levels (Hedge Maze, Archipelago, Black Gold, Harbor Lights,
  Castle Moat) get the follow camera on phones, tablets, laptops and in
  rooms. A phone keeps an 8 mm tank and still shows about nine tenths of
  their width and four fifths of their height; shown whole they would
  draw it at 5.8 to 6.5 mm. A monitor playing locally shows them whole
  anyway (section 3), and a map's `view` key can still choose for
  itself.
- **Vertical engagement.** The sight box stays +-11.5 x +-7.5 cells
  (section 5): an enemy straight above or below closes to 240 px before
  it fires, so nobody is shot from beyond the edge of their screen, and
  phone tanks stay 7.5 to 9.3 mm. Sideways nothing changes.
- **Minimap on phones.** Off. A phone's view relies on the edge arrows,
  which already point at threats, teammates and the frog; tablets and
  desktops keep the minimap, and the builder's navigator stays on every
  device.
- **Stick side.** It stays a build-time choice (the `touch-steer-right`
  feature) for now, although Brawl Stars and Call of Duty: Mobile make
  it a setting. The corner clusters leave both bottom corners to the
  thumbs, so both builds use the same HUD. `TouchScheme::update` already
  takes the side as an argument, so a setting later would change only
  where `app.rs` reads it from.

## 16. Sources

Verification: [read] opened and read; [summary] from a search summary of
the page, not opened; [forum] community source; unverified where stated.
The sources for section 13 are listed per pattern in
docs/large-maps-patterns.md.

Code (this repository): `src/view.rs`, `src/app.rs`, `src/render/game.rs`,
`src/render/weather.rs`, `src/render/shockwave.rs`, `src/ground.rs`,
`src/hud.rs`, `src/render/hud.rs`, `src/lobby.rs`, `src/level_select.rs`,
`src/touch.rs`, `src/editor/mod.rs`, `src/ai.rs` (`act_attack`),
`src/tuning.rs` (`enemy_attack_range`, `enemy_fire_align_px`,
`enemy_aim_settle`, `enemy_view_range`, `shell_speed`, `tank_speed`,
`view_max_scale`), `src/lib.rs`, `src/net/wire.rs`, `src/net/encode.rs`,
`src/simulation/weapons.rs` (`LASER_MAX_RANGE`), `src/pathfind.rs`,
`src/simulation/hits.rs`, `tools/android/AndroidManifest.xml`,
`tools/ios/Info.plist`; docs/fullscreen-resolution-research.md,
docs/hud-and-builder-layout-design.md, docs/mobile-and-scaling-design.md,
docs/game-editor-fusion.md.

Cameras: Itay Keren, "Scroll Back: The Theory and Practice of Cameras in
Side-Scrollers", GDC 2015, gamedeveloper.com and GDC Vault [summary];
Squirrel Eiserloh, "Juicing Your Cameras With Math", GDC 2016,
mathforgameprogrammers.com [summary]; Daniel Holden, "Spring-It-On",
theorangeduck.com [summary]; dr-jam/CameraControlExercise and
prime31/CameraKit2D on GitHub [read]; raysan5/raylib
`examples/core/core_smooth_pixelperfect.c`, `core_2d_camera_platformer.c`,
`core_2d_camera_mouse_zoom.c`, `core_window_letterbox.c` [read];
FancyFurret/celeste-motion-smoothing, voithos/godot-smooth-pixel-camera-demo,
doonv/bevy_smooth_pixel_camera [read]; Unity 2D Pixel Perfect Camera
documentation and Godot "Multiple resolutions" and `ProjectSettings`
[read]; survev/survev `client/src/camera.ts`, `client/src/game.ts`,
`shared/gameConfig.ts` [read].

Fairness: Overwatch 21:9 and FOV (wccftech, dsogaming) [summary];
PKDT-93/sc2-ultrawide README [read]; WSGF, "Ultrawide and multi-monitor
support in competitive games" [summary]; Brawl Stars landscape update and
balance changes (TouchArcade, 2018) [summary], pillarboxing on wide phones
[forum]; Vampire Survivors on PCGamingWiki [summary] and p1xel8ted
UltrawideFixes [read]; agar.io 2015 client gist and ABCxFF/diepindepth
`canvas/scaling.md` [read]; Valorant 21:9 behaviour unverified (sources
conflict).

Indicators and HUD: Baudisch and Rosenholtz, Halo, CHI 2003; Burigat,
Chittaro and Gabrielli, scaled arrows, MobileHCI 2006; Gustafson et al.,
Wedge, CHI 2008; Gustafson and Irani, EdgeRadar, CHI 2007; Burigat and
Chittaro 2011 (arrows, wedge and overview map) [summary]; Player
Research, "Perceiving without looking" [summary]; Apple Human Interface
Guidelines, "Designing for games", "Game controls", "Motion" [read] and
WWDC24 session 10085 [read]; Android accessibility (48 dp), display
cutouts, gesture navigation [read]; Xbox Accessibility Guideline 117
[summary].

Devices and platforms: Apple support specifications and the HIG; useyourloaf
iPhone 16 and 17 screen sizes, 1440px.com safe areas, ios-resolution.com,
yesviz, gsmarena, screensize.net viewport statistics [summary]; Android 16
behaviour changes and `appCategory` [read]; Apple `UIRequiresFullScreen`
documentation and WWDC26 session 278 [read]; MDN browser-compat-data for
the Fullscreen and Screen Orientation APIs [read]; WebKit
`WebGLRenderingContextBase.cpp` and `CanvasBase.cpp`, ANGLE
`DisplayMtl.mm`, Chromium `gpu_driver_bug_list.json`, Apple Metal feature
set tables [read]; web3dsurvey MAX_TEXTURE_SIZE [summary].

Touch editors: Procreate handbook (gestures), Pixaki user guide, Super Mario
Maker 2 handheld controls, Townscaper, Geometry Dash level editor, Clash of
Clans village edit [summary]; Tiled keyboard shortcuts and `minimap.cpp`
[read]; raylib `rgestures.h` [read].
