# Fullscreen, screen sizes and cross-play: research notes

Status: research, revision 2. Written 2026-09 against v0.0.18 (`sola-raylib`
6.3.0 over raylib 6.0.0). Revision 1 treated the phone as a second platform
with its own maps; this revision is rewritten around two requirements that
arrived after it: **matches are cross-platform** (a mobile player and a
desktop player in the same round, on the same battlefield) and **mobile
plays in landscape only**. The verified platform facts from revision 1 are
kept; the conclusions changed.

Standing constraint: **the entire battlefield is on screen at all times**,
on every device. No scrolling, no camera that follows, no zoom that crops.

Contents

1. The requirements and what each one forces
2. Where the game stands today
3. The geometry, redone for a shared world
4. The standard map size: 28 x 14
5. Touch input in landscape
6. Fairness between a thumb and a keyboard
7. Concepts and precedents (unchanged reference)
8. Platform notes: native, web, iPhone and Android in landscape
9. Recommended direction
10. Open questions

## 1. The requirements and what each one forces

**Cross-play on one battlefield.** Both players simulate the same world:
same map, same cell grid, same positions. Nothing about the *world* may
differ per device, only how it is drawn. Two things follow directly:

- The renderer must separate world size from screen size (the "view"),
  because the same 896 x 448 world has to land on an 852 x 393-point phone
  and a 1920 x 1080 monitor. This is the same letterbox work revision 1
  proposed, now mandatory rather than recommended.
- **A match's map must be comfortable on the smallest screen in the
  match.** There is no "phone map" and "desktop map"; there is one map
  class, sized so a tank is still a target on a phone. The desktop player
  sees the same map larger. Two map pools (a cross-play pool and a
  desktop-only pool) are possible but split the players; the
  recommendation below is one standard size for everything.

**Landscape only on mobile.** This removes revision 1's biggest phone trick
(drawing the scene rotated for a portrait phone) and its biggest problem
(the portrait phone's 3 mm tank). A landscape phone is a 2.1:1 to 2.2:1
screen, wider than a 16:9 monitor. That aspect becomes the design target
for the standard map: a map at about 2:1 fills a landscape phone with
almost no bars and leaves a thin bar above and below on a monitor, which
is where the desktop HUD goes.

**Whole battlefield visible.** Unchanged, and now also a fairness rule:
both players see everything, so nobody has hidden information.

## 2. Where the game stands today

Facts from `main.rs`, `game.rs`, `lib.rs`, `battlefield.rs`, `map.rs` and
`site/src/pages/index.astro`:

- **Window size is world size.** `Game::init(width, height)` gets the window
  size, `battlefield::spawn_walls` puts the boundary walls at `0..width` x
  `0..height`, `ground::build` tiles that rectangle, and every simulation
  quantity (speeds in px/s, AI ranges, collider boxes, `OBSTACLE_GRID_SIZE`,
  `PATHFIND_CELL_SIZE`) is in those pixels. There is no camera or view
  transform. `--resolution 1920x1080` makes a *bigger battlefield*, not a
  zoomed one.
- **Maps have no declared size.** `maps/default.toml` uses cells 0..40 x
  0..22; the window trims that to 40 x 22.5 cells. The extent is implied.
- **The scene render target is window-sized and blitted 1:1.** HUD,
  banners and debug overlays are in absolute screen pixels.
- **The window is fixed-size and windowed.** No `resizable()`,
  `fullscreen()` or `highdpi()` builder call, no toggle key.
- **Web: CSS scales the 1280 x 720 canvas** (`width: min(100vw, 1280px);
  height: min(100vh, 720px); object-fit: contain`). The backing store
  ignores device pixel ratio; raylib's web platform has no HiDPI support.
- **There is no touch input.** Arrow keys, Space, P, R, L, I only. raylib's
  web platform registers touch callbacks; nothing reads them.
- **The simulation is deterministic and input-driven** (`Game::update`
  takes a plain `Input`, one seeded RNG stream, `determinism_tests`). This
  is the property cross-play will lean on: both clients can run the same
  world from the same inputs, and the view is a purely local concern.

On an iPhone 15 in landscape today (852 x 393 points, about 0.165 mm per
point) the scene is 699 x 393 points and a tank is **5.8 mm**. Apple's
minimum comfortable touch target is 44 points, 7.3 mm on this panel.

## 3. The geometry, redone for a shared world

With uniform scaling and the whole battlefield visible:

```
scale     = min(screen_w / world_w, screen_h / world_h)
tank_size = 64 world px * scale
```

The screen is given per device and the tank is fixed at two cells (making
tanks bigger in world pixels changes colliders, corridor clearance and every
map, see revision 1's rejected option F). **The only shared lever is the
map's cell count**, and it must be one number for the whole match. The
table evaluates candidate map sizes on every device likely to be in a
match. Bars are per side; "|n" is a side bar, "-n" a top/bottom bar.

| Map (cells) | Aspect | Lanes | iPhone 15 | iPhone 15, safe area | Pixel 8 | iPhone SE 16:9 | iPad 10.9" | MacBook 14" | 24" 1080p | 27" 1440p | 34" ultrawide |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 40 x 22.5 (today) | 1.78 | 11 | 5.8 mm, \|77 | 5.8, \|18 | 5.7, \|91 | 5.2 | 11.2, -78 | 15.1, -66 | 26.5 | 29.8 | 29.7, \|440 |
| 36 x 18 | 2.00 | 9 | 7.2, \|33 | 6.7, -13 | 7.2, \|46 | 5.8, -21 | 12.5, -115 | 16.8, -113 | 29.4, -60 | 33.1, -80 | 37.1, \|280 |
| 34 x 16 | 2.12 | 8 | 8.1, \|8 | 7.1, -24 | 8.1, \|20 | 6.1, -31 | 13.2, -132 | 17.8, -135 | 31.2, -88 | 35.1, -118 | 41.8, \|190 |
| 32 x 16 | 2.00 | 8 | 8.1, \|33 | 7.6, -13 | 8.1, \|46 | 6.5, -21 | 14.0, -115 | 18.9, -113 | 33.1, -60 | 37.3, -80 | 41.8, \|280 |
| 30 x 14 | 2.14 | 7 | 9.3, \|5 | 8.1, -25 | 9.2, \|16 | 6.9, -32 | 14.9, -135 | 20.2, -138 | 35.3, -92 | 39.8, -123 | 47.7, \|177 |
| **28 x 14** | **2.00** | **7** | **9.3, \|33** | **8.7, -13** | **9.2, \|46** | **7.4, -21** | **16.0, -115** | **21.6, -113** | **37.9, -60** | **42.6, -80** | **47.7, \|280** |
| 26 x 12 | 2.17 | 6 | 10.8 | 9.3, -27 | 10.8, \|11 | 8.0, -34 | 17.2, -138 | 23.3, -142 | 40.8, -97 | 45.9, -129 | 55.7, \|160 |
| 24 x 14 | 1.71 | 7 | 9.3, \|89 | 9.3, \|30 | 9.2, \|104 | 8.4, \|12 | 18.7, -66 | 25.2, -50 | 42.6, \|34 | 47.9, \|46 | 47.7, \|486 |

Panel pitches used: iPhone 15 0.165 mm/pt, Pixel 8 0.157 mm/dp, iPhone SE
0.156, iPad 0.19, MacBook 0.20, 24" 1080p 0.276, 27" 1440p 0.233, 34"
ultrawide 0.232. "Lanes" is rows divided by two, the number of tank-widths
the map is tall. "Safe area" is the iPhone's landscape safe rectangle
(59 pt insets each side for the dynamic island and its mirror).

Readings:

- **Today's map cannot be the cross-play map.** 5.8 mm on the phone, and
  its 16:9 shape wastes a fifth of a landscape phone in side bars.
- **Rows set the tank size, columns are almost free.** On a landscape
  phone the height is the binding dimension, so 28 x 14 and 30 x 14 give
  the same 9.3 mm tank; the extra columns only fill the phone's width.
- **The 2:1 shape is the compromise between the screens.** It is between
  a phone (2.17) and a monitor (1.78): 33 pt side bars on the phone, a
  60 px bar above and below on a 1080p monitor, 113 on a 16:10 laptop. The
  bars are useful, not waste: the phone's side bars absorb the dynamic
  island, the monitor's bars hold the desktop HUD.
- **14 rows is the sweet spot, 16 the acceptable maximum.** 14 rows is
  9.3 mm on the phone with 7 lanes; 16 rows is 8.1 mm with 8 lanes, still
  above the 7.3 mm touch minimum. 18 rows drops to 7.2 mm and 22.5 to
  5.8 mm. The iPhone SE class (16:9, small) is the only device under
  7.5 mm at 14 rows; treat it as a floor, not a target.
- **The 27" 1440p and ultrawide desktops are the other extreme**: a 28 x 14
  map gives a 43 to 48 mm tank there. Chunky, but a 32 px sprite at 2x is
  crisp at any integer multiple, and the desktop player already plays
  Vampire Survivors and Brotato at that pixel size. The desktop loses map
  *density* (7 lanes instead of 11), not quality.

## 4. The standard map size: 28 x 14

Recommendation: **one standard cross-play size, 28 x 14 cells (896 x 448
world px, 2:1), with 32 x 16 (1024 x 512, also 2:1) as the large variant**
for matches that want more room and accept an 8.1 mm phone tank. Both share
one aspect, so the letterbox layout, HUD placement and safe-area handling
are designed once.

Why not the alternatives on the table:

- *26 x 12* fills the phone edge to edge at 10.8 mm but has 6 lanes; at 2
  cells per tank the map is a corridor and the AI ring has nowhere to go.
- *24 x 14* was revision 1's phone size. It is 16:9-ish and leaves 89 pt
  side bars on a landscape phone, a fifth of the screen, for the same tank
  as 28 x 14.
- *30 x 14 / 34 x 16* match the phone's own aspect and lose the side bars,
  which puts the dynamic island over the arena's edge and leaves nothing to
  put the HUD in on the phone.
- *36 x 18 and up* are under 7.5 mm on the phone.

What 28 x 14 means for the existing content and knobs (measured against
`tuning.rs`):

| Knob | Value | On 896 x 448 |
|---|---|---|
| `enemy_attack_range` | 340 px | 38% of the width, 76% of the height |
| engage ring radius (range x 0.8) | 272 px | ring diameter 544 px exceeds the map height |
| retreat range (range x 1.3) | 442 px | equals the map height |
| `guard_leash_px` / `guard_keep_off_px` | 260 / 130 px | leash covers most of the map |
| spawn band, 0.272 to 0.4 of the short side | 122 to 179 px from the edge | a 57 px band, narrower than one 48 px nav cell |
| `wave_gate_min_player_dist` | 300 px | rules out most of the edge |
| `tank_speed` 210, `shell_speed` 500 px/s | | a tank crosses in 4.3 s, a shell in 1.8 s |

Because there is one standard size, these are **one retune, not a per-map
scale**: the distance knobs come down by about 0.62 (448 / 720) once, the
spawn band gets a wider fraction range or a cell-based margin, and the
speeds stay where they are (a quicker round is fine, and it is the same
round for both players). The probe sweeps, fixture budgets and
`determinism_tests` are pinned to 1280 x 720 today and get re-baselined
once, on the new default world.

The desktop map at 40 x 22.5 stays as content, playable in desktop-only
matches or single player, but it stops being the default.

Four maps in `maps/crossplay/` exist as size studies, all hand-authored and
playable today with `--resolution WxH` since the window is still the world:
`arena` (28 x 14, the reference layout), `crossroads` and `bunker` (24 x 14)
and `strip` (26 x 12). They were rendered but **not lint-checked at their
own size** in this environment (the crate could not be built here: raylib's
X11 headers were not installable through the session proxy); lint them
locally before relying on them.

## 5. Touch input in landscape

Landscape changes the touch layout compared with revision 1. With a 2:1
map on a 2.17:1 phone there is no strip to put a visible D-pad in (33 pt
side bars are too narrow for a thumb), so the controls live over the arena.
Two schemes, both producing the same `Input` the keyboard does:

**I. Visible overlay pad.** A semi-transparent 4-way pad in the lower-left
corner and a fire button in the lower-right, drawn over the arena at about
40% alpha (Brawl Stars, Mini Militia). Discoverable; costs a permanent
smudge over two corners.

**J. Invisible: floating joystick under the right thumb, tap-to-fire under
the left** (the user's proposal; handedness is a setting). Nothing drawn at
rest. The first touch on the steering half sets the joystick origin where
the thumb landed; the drag direction is snapped to 4-way by dominant axis
with a dead zone of about 12 pt and a hysteresis band of about 15 degrees
around the diagonals so the direction does not flicker; lifting stops the
tank, which is the game's instant-stop model already. A tap on the other
half fires, a held touch holds fire (edge for shells, held for laser and
minigun, exactly what `Input::player_intent.fire` carries). A faint stick
and knob fade in only while a thumb is down, and a one-time hint on the
first round shows the two halves. Brawl Stars, Vampire Survivors and
Archero ship this "dynamic joystick".

J is the default: it covers nothing at rest, and the floating origin lets
the player put the thumb on an empty patch. I is kept as a setting.

**Occlusion is the real cost**, and it is a mobile-only cost. A thumb pad
is about 18 to 20 mm across, so each thumb hides a 2-tank patch of a lower
corner while pressed. Mitigations, in order:

- HUD in the top corners on the phone, never under the thumbs.
- Map design keeps the two bottom corners low-value: nothing that must be
  watched constantly (the frog, a gate) goes in the bottom 3 rows x 6
  columns of either side. The `arena` layout puts pickups there instead.
- The joystick only exists while pressed, so a player who lifts the thumb
  sees everything.

## 6. Fairness between a thumb and a keyboard

Cross-play is only acceptable if the phone player is not structurally
worse off. What the design above gives and what it does not:

| Aspect | Status |
|---|---|
| Same world, same visibility | Yes: one map, whole map visible on both, no hidden information. |
| Same simulation | Yes: `Game::update` is deterministic on the same `Input` stream; the view is local. This is also the natural netcode model (lockstep input exchange) but that is a separate design. |
| Movement precision | Near parity: 4-way snap with instant stop is the one movement model a joystick handles as well as keys. The dead zone adds roughly 30 to 50 ms before a direction registers; a keyboard registers on the frame. |
| Fire timing | Parity: a tap is an edge, a hold is a hold. |
| Target size | Phone 9.3 mm vs desktop 27 to 48 mm. The phone player aims at a smaller target, but aiming is 4-way and range-gated, not pointer-based, so this affects reading the field more than hitting things. |
| Occlusion | Phone only. Mitigated by section 5, not removed. |
| Latency | Touch sampling on iOS is 60 to 120 Hz vs a keyboard's per-frame poll; comparable to the network jitter any cross-play match already has. |
| HUD legibility | Solved by a separate UI scale (section 8): phone text at 12 pt minimum in the corners, desktop text in the letterbox bars. |

Net: the phone player gives up a little input latency and two thumb-sized
patches of view. Nothing else. That is in line with what cross-play shooters
with simple inputs accept (Brawl Stars has no input-based matchmaking
split; Fortnite and Rocket League do, because their inputs are analog).

## 7. Concepts and precedents (unchanged reference)

The vocabulary, kept from revision 1 for reference.

- **Virtual resolution**: render to a world-sized target, blit with
  `scale = min(...)` and an offset; raylib's `core_window_letterbox`
  example is this pattern. The game has the target, not the scale step.
- **Stretch modes** (Godot / libGDX / Unity names): `ignore` (stretch,
  never), `keep` (letterbox, the baseline), `expand` (show more world into
  the bars, rejected: per-device arenas break seeded replays and
  cross-play), `keep_width`/`keep_height` (same caveat), integer scaling
  (crisp, more bars).
- **World scale vs UI scale**: Stardew, Factorio, Terraria keep them
  independent. HUD and text are sized from the screen, never from the world
  scale.
- **Pixel-perfect vs fractional**: art pixels are 2 world px; they are
  whole device pixels when `2 x scale x devicePixelRatio` is an integer.
  Fractional scaling shimmers slightly on moving sprites; acceptable for a
  static-floor top-down game (Shovel Knight ships 400 x 240 at 4.5x). An
  integer-snap option costs a little more bar.
- **Safe area**: nothing interactive or informational under the notch, the
  dynamic island, a punch-hole camera or the home indicator; CSS
  `env(safe-area-inset-*)` with `viewport-fit=cover`.
- **Precedents**: Battle City (whole arena visible, HUD in a side strip,
  arena never scales); Into the Breach (board fixed, UI panels move);
  Celeste / Shovel Knight (integer vs fractional); Vampire Survivors mobile
  (crops, the opposite of our constraint); Brawl Stars (landscape-only
  cross-play with a floating joystick, the closest product analogue).

## 8. Platform notes

### Native (raylib 6.0 through sola-raylib 6.3)

Verified against the crate sources in the cargo registry.

- Builder: `init().size(w, h).resizable().highdpi()`; runtime
  `set_window_state`/`clear_window_state` with `FLAG_WINDOW_RESIZABLE`,
  `FLAG_FULLSCREEN_MODE`, `FLAG_WINDOW_HIGHDPI`; `toggle_fullscreen`,
  `toggle_borderless_windowed`, `is_window_fullscreen`,
  `is_window_resized`, `set_window_min_size`, `set_window_size`,
  `get_current_monitor`/`get_monitor_width`/`_height`, `get_screen_width`
  (logical) vs `get_render_width` (physical), `get_window_scale_dpi`.
- `FLAG_WINDOW_HIGHDPI` must be set before the window exists. With it a
  Retina Mac's framebuffer is 2x and drawing stays in logical units.
- Borderless windowed (`toggle_borderless_windowed`) keeps the desktop
  resolution, is instant and alt-tabs cleanly; exclusive `toggle_fullscreen`
  switches video mode and misbehaves on macOS and multi-monitor setups. Use
  borderless on F11, start windowed.
- A resize only rebuilds the view. The ripple shaders work on the scene
  texture's UVs, so with a world-sized target they need no change.
- `probe.rs` and the dev server keep taking the world size only.

### Web (emscripten via raylib's `rcore_web.c`)

- **Resizing.** With `FLAG_WINDOW_RESIZABLE`, raylib's resize callback sets
  the canvas backing store to `window.innerWidth x innerHeight` on every
  browser resize, in CSS pixels; it never multiplies by `devicePixelRatio`.
- **Device pixel ratio.** For crisp art on a DPR-3 phone the page (or a DPR
  read on the Rust side) sizes the backing store in device pixels and calls
  `SetWindowSize`. Without it the pixel art is soft on every phone.
- **Fullscreen.** `ToggleFullscreen` on web calls the Fullscreen API on the
  canvas after a 100 ms delay and needs a user gesture. iPhone Safari does
  not implement the Fullscreen API for elements (iPad does). On Android
  Chrome it works, and there `screen.orientation.lock("landscape")` works
  *inside* fullscreen.
- **Landscape on iPhone in a Safari tab.** Cannot be locked. The tools are:
  a full-screen "rotate your phone" card whenever `orientation: portrait`
  matches (the match keeps running behind it, since a multiplayer round
  cannot pause for one player), `100dvh`, and `viewport-fit=cover` with
  `env(safe-area-inset-left/right)` for the island. A web app manifest
  with `display: standalone` and `orientation: landscape` gives Android a
  locked, chrome-less landscape game from the home screen; iOS honours
  `display` but not `orientation`, so the card stays.
- **Touch.** raylib registers touchstart/move/end/cancel on the canvas;
  `get_touch_point_count`/`get_touch_position` work. Coordinates arrive in
  canvas CSS pixels and go through the same view mapping as the editor's
  mouse. `touch-action: none` on the canvas stops scroll and pinch.
- **Dynamic island in landscape.** It sits 59 pt into one side. With
  28 x 14 the arena starts 33 pt in, so the island overlaps the arena's
  first half-cell, which is the boundary wall. Draw the arena under it and
  keep the HUD out of both insets; if a playtest shows the island hiding a
  gate, fit the arena to the safe rectangle instead (8.7 mm).
- **Android.** Punch-hole cameras are smaller than the island and sit in
  the same inset; 20:9 phones (2.22:1) get 46 pt side bars with a 2:1 map.
  DPR 2.6 to 3.5; the same backing-store handling applies.

### iPhone checklist, in the order it pays off

1. `100dvh`, `viewport-fit=cover`, safe-area insets, `touch-action: none`,
   the portrait "rotate" card.
2. The engine letterbox with a 28 x 14 world (the phone shows it at 9.3 mm).
3. Invisible touch controls with the visible pad as a setting.
4. DPR-correct canvas size.
5. A manifest so Add to Home Screen gives a standalone window.

## 9. Recommended direction

Layered so each step ships alone. Steps 1 to 4 are presentation and input;
the simulation changes only in step 5, once.

**Step 1: split world size from window size.** `MapFile` gains `size =
[cols, rows]` (default 40 x 22.5 so nothing existing changes). `Game::init`
takes the world size from the map; `--resolution` becomes the initial
window size; probe and dev server use the world size.

**Step 2: the `View` and the letterbox blit.** One struct (`scale`,
`offset`, `world_to_screen`, `screen_to_world`, optional integer snap)
rebuilt per frame from the screen size. World-sized render target,
`draw_texture_pro` blit, shake offset scaled. HUD, banners and debug
overlays move to screen space at a UI scale from the screen height; on a
wide screen the HUD goes in the top/bottom bars, on a phone in the top
corners. Editor mouse through `screen_to_world`.

**Step 3: window modes and web sizing.** Native: `resizable().highdpi()`,
minimum window half the world, F11 borderless, `--fullscreen`. Web:
`FLAG_WINDOW_RESIZABLE`, `100vw x 100dvh`, safe-area padding,
`touch-action: none`, DPR-sized backing store, the portrait card, a
manifest.

**Step 4: touch input in `main.rs`.** A `TouchScheme` setting: floating
joystick + tap-to-fire (default, with handedness swap, low-alpha feedback
while touched, first-round hint) and the overlay pad. Both produce the
keyboard's `Input`; a mouse fallback makes them testable on desktop and
through the dev server's `input` tool.

**Step 5: the standard world.** `DEFAULT_WORLD_*` becomes 896 x 448
(28 x 14); the distance knobs are retuned once by about 0.62 and the spawn
band gets a cell-based margin; `maps/default.toml` is re-authored at 28 x 14
(the 40 x 22.5 layout is kept as a named desktop map); probe sweeps,
fixture budgets and determinism tests are re-baselined on the new default.
Lint `maps/crossplay/` at their own size and promote `arena` or a
successor to the new default.

**Step 6: cross-play itself.** Out of scope here, but the shape is set by
the above: both clients run the deterministic simulation from a shared
`Input` stream; the view never enters the protocol.

## 10. Open questions

- **14 or 16 rows as the standard?** 9.3 mm and 7 lanes, or 8.1 mm and 8
  lanes. Decide with a phone in hand on the `arena` layout; both are 2:1
  so nothing else changes.
- **One pool or two?** One standard size for every match is simplest and
  keeps the player base together; a desktop-only pool at 40 x 22.5 is
  possible if desktop-only matches want the denser map.
- **Island policy.** Arena under the island (9.3 mm) or inside the safe
  rectangle (8.7 mm). A playtest question.
- **Which thumb steers by default?** Right-thumb steer is the proposal;
  most dynamic-joystick games ship the mirror. One setting either way.
- **Integer snap by default?** Measure the shimmer on a real phone at DPR
  3 after the DPR fix.

## Revision 3: the HUD bar is part of the bitmap

Master now renders the field plus a 32 px HUD bar above it as one bitmap
(`Layout::for_field`, the web canvas locked to that shape with
`aspect-ratio: 1280 / 752` because raylib maps touches against the canvas
box). What gets letterboxed onto a screen is therefore `cols x 32` wide and
`rows x 32 + 32` tall, and the bar scales with the field. Redone with that
shape, for a phone in landscape steering with an invisible joystick on the
right and tapping to fire on the left (so nothing is reserved for
controls):

| Field | Bitmap | Shape | iPhone 15 | Pixel 8 | iPhone SE | iPad 10.9" | iPad Pro | MacBook 14" | 24" 1080p | 27" 1440p | min used |
|---|---|---|---|---|---|---|---|---|---|---|---|
| 40 x 22.5 (today) | 1280 x 752 | 1.70 | 5.5 mm, 79% | 5.5, 77% | 5.0, 96% | 11.2, 85% | 13.3, 78% | 15.1, 90% | 25.4, 96% | 28.6, 96% | 77% |
| 32 x 16 | 1024 x 544 | 1.88 | 7.6, 87% | 7.6, 85% | 6.5, 94% | 14.0, 76% | 16.6, 71% | 18.9, 82% | 33.1, 94% | 37.3, 94% | 71% |
| 30 x 15 | 960 x 512 | 1.88 | 8.1, 86% | 8.1, 84% | 6.9, 95% | 14.9, 77% | 17.7, 71% | 20.2, 82% | 35.3, 95% | 39.8, 95% | 71% |
| 30 x 14 | 960 x 480 | 2.00 | 8.6, 92% | 8.6, 90% | 6.9, 89% | 14.9, 72% | 17.7, 67% | 20.2, 77% | 35.3, 89% | 39.8, 89% | 67% |
| **28 x 14** | **896 x 480** | **1.87** | **8.6, 86%** | **8.6, 84%** | **7.4, 95%** | **16.0, 77%** | **18.9, 71%** | **21.6, 82%** | **37.9, 95%** | **42.6, 95%** | **71%** |
| 26 x 13 | 832 x 448 | 1.86 | 9.3, 86% | 9.2, 84% | 8.0, 96% | 17.2, 77% | 20.4, 72% | 23.3, 83% | 40.8, 96% | 45.9, 96% | 72% |
| 26 x 12 | 832 x 416 | 2.00 | 10.0, 92% | 10.0, 90% | 8.0, 89% | 17.2, 72% | 20.4, 67% | 23.3, 77% | 40.8, 89% | 45.9, 89% | 67% |

Tank in mm at the device's fit scale; "used" is bitmap area over screen
area. Two families: 2:1 fields fill a phone to 92% but drop tablets to 72%
and monitors to 89%; fields of about 1.87:1 give 86 / 77 / 95, the best
minimum. Rows set the tank size, columns only spend the phone's side bars.
Under 14 rows the AI's ring and retreat range have nowhere to go.

**Recommendation stands at 28 x 14** (896 x 448 field, 896 x 480 bitmap).
On an iPhone 15 the fit scale is 0.82: an 8.6 mm tank, 59 pt side bars
that match the safe-area inset so the dynamic island never touches the
field, a 26 pt bar with 14.7 pt text, no separate UI scale needed. 30 x 15
is the same family at 8.1 mm if more room is wanted. Section 9's steps are
unchanged except that the view letterboxes the whole bitmap (bar included)
and the web canvas's `aspect-ratio` follows the new size (896 / 480). The
companion plan page renders the arena on each screen class with the bar
and the touch layout.
