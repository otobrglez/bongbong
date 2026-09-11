# Fullscreen and resolution: research notes

Status: research, no code changes. Written 2026-09 against v0.0.18
(`sola-raylib` 6.3.0 over raylib 6.0.0).

The constraint this whole document works under: **the entire battlefield is
on screen at all times.** No scrolling, no camera that follows the player, no
zoom that crops. That rules out most of what "resolution handling" means in
other games and turns the problem into pure geometry, so the first half of
this document is about the geometry and the second half about what the
engine and the browser will let us do with it.

Contents

1. Where the game stands today
2. The geometry: why tanks are small on a phone, and the only three levers
3. The concept toolbox (how engines and games frame this)
4. Options evaluated against the constraint
5. Platform notes: native (raylib), web (emscripten), iPhone specifically
6. Recommended direction and what it touches in this codebase
7. Open questions

## 1. Where the game stands today

Facts pulled from `main.rs`, `game.rs`, `lib.rs`, `battlefield.rs`, `map.rs`
and `site/src/pages/index.astro`:

- **Window size is world size.** `Game::init(width, height)` gets the window
  size, `battlefield::spawn_walls` puts the boundary walls' inner faces at
  `0..width` x `0..height`, `ground::build` tiles that same rectangle, and
  every simulation quantity (speeds in px/s, AI ranges, collider boxes,
  `PATHFIND_CELL_SIZE`, `OBSTACLE_GRID_SIZE`) is in those same pixels. There
  is no camera and no view transform anywhere (`game.rs` says so explicitly
  at the camera-shake blit). `--resolution 1920x1080` therefore does not
  zoom the game, it makes a *bigger battlefield* with the default map's
  cells sitting in its top-left 1280x720.
- **Maps are cell grids with no declared size.** `maps/default.toml` uses
  cells 0..40 x 0..22, i.e. 41x23 cells of 32 px = 1312x736, trimmed by the
  1280x720 window to the 40x22.5 cells `map.rs` mentions. The map's extent
  is implied by the window, never stored.
- **The scene render target is window-sized and blitted 1:1** (`game.rs`
  `Game::render`: `load_render_texture(screen_width, screen_height)`, then
  `draw_texture_rec` with a flip and the shake offset). The three ripple
  shaders take the screen size for their UV maths (`screen_to_ripple_uv`).
- **HUD, banners, debug overlays are in absolute screen pixels**
  (`HUD_FONT_SIZE = 26`, `HUD_MARGIN = 20`, `draw_debug_overlays` is drawn
  post-composite in screen space).
- **The window is fixed-size and windowed.** `sola_raylib::init().size(w, h)`
  with no `resizable()`, `fullscreen()` or `highdpi()` builder call, no
  fullscreen toggle key, and `get_screen_width()` is re-read each frame but
  can never change.
- **Web: the 1280x720 canvas is scaled by CSS only.** `index.astro` sets
  `width: min(100vw, 1280px); height: min(100vh, 720px); object-fit: contain;
  image-rendering: pixelated`. The canvas backing store stays 1280x720
  whatever the device pixel ratio (raylib's web platform never consults
  `devicePixelRatio`, and it warns that `FLAG_WINDOW_HIGHDPI` is
  unavailable there).
- **There is no touch input at all.** `main.rs` reads arrow keys, Space, P,
  R, L, I. On a phone the game is unplayable regardless of size; raylib's
  web platform does register touch callbacks
  (`emscripten_set_touchstart_callback` and friends in `rcore_web.c`), so
  `get_touch_point_count`/`get_touch_position` work, nothing reads them yet.

What this produces on an iPhone 15 (393x852 CSS points, about 0.165 mm per
point on the 6.1" panel):

| Orientation | Canvas element (CSS) | Scene after `object-fit: contain` | One tank (64 world px) |
|---|---|---|---|
| Portrait | 393 x 720 (`min(100vh, 720)`) | 393 x 221 | 19.6 pt, **3.2 mm** |
| Landscape | 852 x 393 | 699 x 393 | 34.9 pt, **5.8 mm** |

So "tanks are very small" is 3 mm in portrait and 6 mm in landscape. For
reference, Apple's minimum comfortable touch target is 44 pt (7.3 mm on this
panel), and a unit you have to track and aim at in a fast game wants to be
at least that, ideally 8 to 12 mm.

Two smaller iOS-specific bugs sit in the same CSS: `100vh` on iOS Safari is
the *tallest* viewport (address bar collapsed), so in portrait the canvas
element is taller than the visible area until the bar hides, and there is
no `viewport-fit=cover`/`env(safe-area-inset-*)` handling, so nothing knows
where the notch and the home indicator are.

## 2. The geometry: why tanks are small, and the only three levers

With the whole battlefield always visible and uniform (non-distorting)
scaling, the on-screen size of anything is fixed by one formula:

```
scale     = min(screen_w / world_w, screen_h / world_h)
tank_size = 64 world px * scale
```

Nothing in the renderer, the window mode or the browser can beat this. The
only three levers are:

1. **Screen size** (the device; on web also whether the page chrome and
   letterbox bars are wasting it).
2. **World size in pixels**, which for this game means *how many 32 px cells
   the map spans*.
3. **Tank size in world pixels** (`Tank::scale`, currently 2.0; the
   `// 3.0` next to it in `tank.rs` shows it was tried). This is a gameplay
   change, not a view change: it moves collider sizes, corridor widths and
   the obstacle-grid alignment (`OBSTACLE_GRID_SIZE` assumes 32 px tiles and
   a two-cell tank). Ruled out below.

Lever 1 is the fullscreen/letterbox work. Lever 2 is the one that actually
makes tanks bigger on a phone, and it is a *content* decision: phone maps
need fewer cells. The table below is the formula evaluated for a few
candidate map sizes. "Rotated" means the scene is drawn turned 90 degrees so a
landscape map fills a portrait screen (section 4, option D).

| Device | Map (cells / world px) | scale | tank | letterbox left over |
|---|---|---|---|---|
| iPhone 15 portrait, not rotated | 40x22.5 / 1280x720 (today) | 0.31 | 3.2 mm | 631 pt tall strip |
| iPhone 15 portrait, rotated | 40x22.5 / 1280x720 | 0.55 | 5.8 mm | 153 pt strip |
| iPhone 15 portrait, rotated | 32x18 / 1024x576 | 0.68 | 7.2 mm | 153 pt |
| iPhone 15 portrait, rotated | 24x14 / 768x448 | 0.88 | 9.3 mm | 178 pt |
| iPhone 15 portrait, rotated | 20x12 / 640x384 | 1.02 | 10.8 mm | 197 pt |
| iPhone 15 landscape | 40x22.5 / 1280x720 (today) | 0.55 | 5.8 mm | 153 pt wide strip |
| iPhone 15 landscape | 24x14 / 768x448 | 0.88 | 9.3 mm | 178 pt |
| iPhone 15 landscape | 20x12 / 640x384 | 1.02 | 10.8 mm | 197 pt |
| iPhone 15 either, phone-aspect map | 26x12 / 832x384 | 1.02 | 10.8 mm | none |
| iPad 10.9" landscape (1180x820) | 40x22.5 (today) | 0.92 | 11.2 mm | 156 pt tall |
| iPad 10.9" landscape | 24x14 | 1.54 | 18.7 mm | 132 pt |
| MacBook 14" (1512x982) | 40x22.5 (today) | 1.18 | 15.1 mm | 132 pt tall |
| 24" 1080p monitor | 40x22.5 (today) | 1.50 | 26.5 mm | none (exact 16:9) |
| 24" 1080p monitor | 24x14 | 2.41 | 42.6 mm | 69 px wide |

Readings:

- Fullscreen and correct fitting alone take the phone from 3 mm to 6 mm.
  Real, but not enough on its own.
- A **24x14-cell map** gets a phone to about 9 mm, a 20x12 map past 10 mm.
  Those are the sizes at which the game reads well on a phone; they are
  also 35 to 60 percent of the default map's area, so they are a different
  level design, not the same map shrunk.
- On desktop the current map is comfortable and a 1080p monitor is an exact
  fit; a 16:10 laptop gets a thin bar. Nothing here argues for changing the
  desktop map.
- Rotation matters more than map size for portrait: it turns a 3 mm tank into
  a 6 mm one for free and leaves a 150 to 200 pt strip at the bottom, which
  is exactly where a thumb D-pad and a fire button want to be. In landscape
  the same strip appears on the side, but a 16:9 map on a 19.5:9 phone
  leaves it narrow (153 pt) and the thumbs cover the battlefield.
- A **square map** (22x22 shown) fits both orientations identically and
  needs no rotation at all. It is the Battle City answer (below); the price
  is a lot of wasted screen on desktop monitors.
- A **phone-aspect map** (26x12, the phone's own 19.5:9) fills the screen
  with no bars at all and gets the same 10.8 mm as 20x12. It only makes
  sense once the controls need no strip of their own, which is what the
  invisible touch scheme in section 4 buys.

## 3. The concept toolbox

The vocabulary other engines use, so the options in section 4 have names.

**Virtual resolution.** The game renders into an off-screen target of a
fixed logical size (here: the world, e.g. 1280x720) and that target is then
drawn to whatever the real screen is. raylib's own
`core_window_letterbox` example is exactly this pattern: a `RenderTexture2D`,
`scale = min(GetScreenWidth()/W, GetScreenHeight()/H)`, `DrawTexturePro`
into a centred destination rectangle, and mouse coordinates mapped back
through the same scale and offset. This game already has the render target;
it is missing the scale/offset step and the world-to-screen mapping.

**Stretch modes** (Godot's names, libGDX's `Viewport` classes and Unity's
Canvas Scaler are the same taxonomy):

| Mode | What it does | Fit for this game |
|---|---|---|
| `ignore` / `StretchViewport` | Stretch to fill, aspect distorted | Never. Square cells become rectangles. |
| `keep` / `FitViewport` | Uniform scale, letterbox bars | The baseline. |
| `expand` / `ExtendViewport` | Uniform scale, then show *more world* into the bars | Only if the map can grow into the bars (see option E). |
| `keep_width` / `keep_height` | Fix one axis, let the other show more | Same caveat. |
| `ScreenViewport` (1 world px = 1 screen px) | No scaling, more or less world | Violates the constraint. |
| Integer scaling | `keep`, but `scale` rounded down to a multiple that keeps art pixels whole | Crisp; wastes more screen. |

**World scale vs UI scale.** Stardew Valley, Factorio, Terraria and most
pixel-art games with a PC-to-phone port keep two independent scale factors:
the world zoom and the UI scale. Text and HUD are drawn in screen space at a
device-appropriate size, never through the world scale. Our HUD, drawn into
screen space in absolute pixels, currently gets neither: on the phone the
26 px HUD font becomes 14 pt (too small), on a 4K monitor it would stay 26 px
(too small the other way).

**Pixel-perfect vs fractional scaling.** Art pixels here are 2 world px
(`Tank::scale = 2.0`, the walls sheet is pre-pixelated by the same factor).
An art pixel stays a whole number of device pixels when
`2 * scale * devicePixelRatio` is an integer. Fractional scales make some
art pixels one device pixel wider than their neighbours, which reads as
shimmer on moving sprites. Approaches, in ascending effort:

1. Accept it with nearest filtering (`image-rendering: pixelated` today).
   Tolerable for a top-down game with a static floor and slow tanks; worst
   on scrolling backgrounds, which this game does not have.
2. Snap `scale` down to the nearest value that makes art pixels whole
   (Celeste: 320x180 internal, integer multiples only, letterbox the rest).
   Costs screen area: on the iPhone landscape example 0.55 would snap to
   0.5 (art pixel = 3 device px at DPR 3), tank 5.3 mm instead of 5.8.
3. Render the scene at the nearest *larger* integer scale, then bilinear
   downsample to the screen ("sharp bilinear"). Smooth and near-crisp,
   costs a second pass and a bigger target.

Shovel Knight ships non-integer scaling of a 400x240 frame to 1080p and
nobody minds; for this game option 1 with an optional option 2 toggle is
enough.

**HiDPI.** Two sizes exist: logical (points, what CSS and macOS report) and
physical (device pixels). raylib on desktop exposes both when
`FLAG_WINDOW_HIGHDPI` is set (`GetScreenWidth` logical, `GetRenderWidth`
physical, `GetWindowScaleDPI` the ratio); without the flag a Retina Mac
renders at half resolution and the OS upscales, blurring everything. On web
raylib has no DPR support at all: the canvas backing store is whatever
`SetWindowSize`/init asked for, and the page has to size it in device
pixels itself (section 5).

**Safe area.** Phones have notches, rounded corners and a home-indicator
strip. Games put nothing interactive or informational into those insets;
the battlefield itself may extend under them only if nothing important is
there. CSS exposes them as `env(safe-area-inset-*)` once the viewport meta
carries `viewport-fit=cover`.

**Orientation.** Two schools:

- *Landscape-only* (most action games): show a "rotate your phone" card in
  portrait. The web Screen Orientation API can `lock("landscape")` only
  inside the Fullscreen API, which iPhone Safari does not implement, so on
  iPhone this is only a request, never a lock. The player holds the phone
  the way the game asks.
- *Orientation-adaptive* (board and puzzle games, Mini Metro, chess):
  design the play area so it fits both, and move the chrome (HUD, buttons)
  into whichever margin is left over. For a 4-direction top-down arena this
  is unusually cheap, because the battlefield has no "up": the whole scene
  can be drawn rotated 90 degrees and the game is the same game (option D).

**Precedents worth copying from:**

- *Battle City* (NES, 1985), this game's direct ancestor: a 13x13-tile
  arena, whole board visible, HUD in a strip on the right, on a nearly
  square 256x240 screen. Arena and HUD are two separate regions with their
  own layout; the arena never scales. The modern web/mobile Tank 1990
  clones keep the square arena and put a virtual D-pad and fire button in
  the leftover margin in either orientation.
- *Into the Breach*: an 8x8 board always fully visible, with UI in the
  margins; the iOS port stays landscape and moves UI panels rather than the
  board.
- *Celeste* and *Shovel Knight*: the integer vs fractional scaling
  positions above.
- *Vampire Survivors* mobile: the opposite of our constraint (it crops on
  phones, showing less world with bigger sprites). Useful as the reminder of
  why we cannot do that: crop is the *usual* mobile answer and we have
  ruled it out.
- *Godot's project settings* (`display/window/stretch/mode`, `aspect`,
  `scale_mode = integer`) are the most complete public writeup of the
  taxonomy above and worth reading once for the edge cases.

## 4. Options evaluated against the constraint

Each option, what it buys, what it does not, and what it costs here.

**A. CSS-only fixes on web (`100dvh`, `viewport-fit=cover`, safe-area
insets, `100vw`).** Fixes the iOS address-bar overflow and the notch; costs
an hour; the canvas backing store stays 1280x720 and is still resampled by
the browser. Buys nothing for tank size beyond the portrait-vs-landscape
difference already visible. Worth doing regardless, but it is not the
answer.

**B. In-engine letterbox (virtual resolution).** The scene target becomes
world-sized and independent of the window; the blit applies
`scale = min(...)` and a centring offset; a small `View { scale, offset,
rotated }` value is the one place world-to-screen and screen-to-world
conversion happens (HUD placement, debug overlays, editor mouse, touch
input). This is what unlocks everything else: a resizable window, native
fullscreen, any monitor aspect, and DPR-correct rendering on web. It buys
no tank size on its own beyond option A's fit, because the formula in
section 2 is unchanged. Cost: a focused change in `main.rs` and `game.rs`
plus every screen-space consumer, listed in section 6.

**C. Smaller maps for small screens.** The only lever that makes a tank on a
phone bigger than about 6 mm. The map format already is a cell grid, so the
mechanism is a declared map size (`size = [cols, rows]`) that becomes the
world size, and a set of phone-sized maps (about 24x14 or 20x12 cells). How
the game picks one is a design question (section 7): a "compact" map set
chosen by screen size, a map picker, or authoring every map at a compromise
size like 32x18. Cost: level design time more than code.

**D. Rotate the scene 90 degrees in portrait.** With option B in place,
`rotated` is one more branch in the blit (`DrawTexturePro` takes a rotation)
and one more transpose in the world-to-screen mapping. On a phone held
upright this turns 3 mm tanks into 6 mm ones with no other change, and the
letterbox strip lands at the bottom, where thumbs already are. It is the
single highest value-for-effort item for the iPhone. Does not affect
desktop.

**E. Grow the battlefield into the bars ("expand").** Instead of black
bars, extend the ground layer (and the boundary walls) so the playable area
fills the screen aspect. Rejected for gameplay: the arena would differ per
device, which breaks the seeded-replay and probe baselines
(`DEFAULT_SCREEN_WIDTH/HEIGHT` exists precisely so probe and game agree
byte-for-byte) and gives phone players a different map than desktop
players. A cosmetic half-version is fine: draw *decorative* ground, or the
HUD panel, in the bars instead of black, Battle City style, while the
playable rectangle stays the map's.

**F. Bigger sprites in the same world (`Tank::scale = 3.0`).** Rejected.
It changes collider sizes, corridor clearance (`max_tank_avoidance_radius`
and the k>=6 corridor rule in the fixtures), spawn legality and the grid
alignment that the whole obstacle/pathfinding layer assumes; it also makes
every existing map and fixture baseline wrong. If bigger-tanks-per-cell is
ever wanted it is a world-unit change (16 px cells, 3x art) across the
board, not a view setting.

**G. Camera zoom or follow.** Rejected by the constraint. Listed only so
the reasoning is recorded: a dynamic camera that frames all tanks (Smash
Bros., TowerFall) would keep every *tank* visible but not every *wall*, and
a walled-off pickup or a wave gate out of frame is exactly the information
the player needs.

**H. Native fullscreen modes.** Not an option on its own but a choice inside
B. Borderless windowed (`ToggleBorderlessWindowed`, raylib 5.0+) keeps the
desktop resolution, is instant, and alt-tabs cleanly; exclusive
`ToggleFullscreen` switches the video mode and is the one that misbehaves
on macOS and with multi-monitor setups. Use borderless, bind it to F11 (and
F on macOS-style keyboards if wanted), start windowed.

Summary:

| Option | Tank on iPhone | Desktop | Verdict |
|---|---|---|---|
| A. CSS fixes | 3 to 6 mm (unchanged) | n/a | do, cheap, insufficient |
| B. Letterbox in engine | 6 mm | fullscreen, any size | do, prerequisite |
| C. Phone-sized maps | 9 to 11 mm | unchanged | do, the real lever |
| D. Rotate in portrait | portrait becomes 6 mm, room for controls | n/a | do, cheapest big win |
| E. Expand into bars | no gain | no gain | reject (cosmetic variant ok) |
| F. Bigger sprites | gameplay change | gameplay change | reject |
| G. Camera | violates constraint | | reject |
| H. Borderless fullscreen | | yes | do, inside B |
| I. Touch: visible D-pad + fire in the strip | needs a strip | n/a | do as an option |
| J. Touch: invisible floating joystick + tap-to-fire | no strip, phone-aspect maps reach 10.8 mm with no bars | n/a | do, the default scheme |

### Touch input: two schemes

The phone needs touch input before any of the above matters. The movement
model is 4-direction with instant stop and `Input::player_intent.fire` is a
plain held flag (edge for shells, held for laser and minigun), so both
schemes below reduce to producing one `Input` per frame in `main.rs`; the
simulation never learns a touch screen exists.

**I. Visible controls in the strip.** A four-key D-pad under the left thumb
and a fire button under the right, drawn into the letterbox strip that
options B and D free up. Discoverable, never covers the arena, and the
4-way pad matches the movement model exactly. It needs the strip, which
ties the map's aspect to "not the phone's": the strip has to stay.

**J. Invisible controls: floating joystick under the right thumb,
tap-to-fire under the left.** No drawn controls at rest. The first touch on
the right half sets the joystick's origin where the thumb landed; dragging
from there picks the direction (dominant axis, snapped to 4-way, a dead
zone of about 12 pt so a resting thumb does not creep); lifting stops the
tank, which is the instant-stop model already. A tap anywhere on the left
half fires, a held touch there holds fire. A faint stick and knob fade in
only while the thumb is down (Brawl Stars, Vampire Survivors, Archero use
this "dynamic joystick"; the mapping of which thumb does what is a
handedness setting, with the user's proposal of right-thumb steering as one
of the two presets).

What J changes in the analysis:

- **The strip is no longer needed.** A phone map can be authored at the
  phone's own aspect (26x12 cells for 19.5:9) and fill the screen with no
  bars, 10.8 mm tanks, either orientation via rotation. That is the best
  number on the table, and it is only reachable with J.
- **Thumbs cover the arena.** This is the cost the constraint makes
  explicit: a thumb pad is about 18 to 20 mm across, so each thumb hides
  roughly a 2 to 3 tank-wide patch of the bottom third while steering. The
  usual mitigations apply: the joystick origin can float anywhere so the
  player picks an empty patch; the arena's bottom rows are the player's
  own side on a Protect map; the HUD goes in whatever bar remains (the
  153 pt strip in the rotated 1280x720 case) or in the top corners, never
  under the thumbs.
- **Discoverability.** Nothing on screen says "touch here". A one-time
  hint on the first round (two ghost circles with "steer" and "fire") and
  the fade-in feedback on touch cover it; every dynamic-joystick game does
  the same.
- **Precision.** For a 4-way snap game a joystick is slightly worse than a
  D-pad at diagonal-ish drags (the dominant axis flips near 45 degrees).
  A hysteresis band of about 15 degrees around the diagonals keeps the
  current direction until the drag clearly commits to the other axis;
  the same idea as `ai.rs`'s direction-commitment gate, applied to a thumb.

Both schemes are the same `main.rs` work behind one `TouchScheme` setting;
J is the default because it unlocks the full-screen phone map, I stays as
the accessible option and for tablets, where thumbs do not reach the
middle anyway.

## 5. Platform notes

### Native (raylib 6.0 through sola-raylib 6.3)

Verified against the crate sources in the cargo registry (the `../raylib`
and `../sola-raylib` checkouts were not present in this session):

- Builder: `init().size(w, h).resizable().highdpi()`; runtime:
  `set_window_state`/`clear_window_state(WindowState)` with the
  `FLAG_WINDOW_RESIZABLE`, `FLAG_FULLSCREEN_MODE`, `FLAG_WINDOW_HIGHDPI`
  bits, `toggle_fullscreen()`, `toggle_borderless_windowed()`,
  `is_window_fullscreen()`, `is_window_resized()`, `set_window_min_size`,
  `set_window_size`, `get_current_monitor`/`get_monitor_width`/`_height`,
  `get_screen_width` (logical) vs `get_render_width` (physical),
  `get_window_scale_dpi`.
- `FLAG_WINDOW_HIGHDPI` must be set *before* the window is created
  (builder), it is not a runtime toggle on GLFW. With it, on a Retina Mac
  the framebuffer is 2x the logical size and raylib applies the scale to
  its projection so drawing code keeps using logical coordinates; the
  letterbox maths stays in logical units and the render target should be
  world-sized in world units either way.
- A resized or fullscreened window is detected per frame with
  `is_window_resized()`; nothing else needs to change because the scene
  target is world-sized, only the blit scale and the screen-space layout are
  recomputed. The ripple shaders operate on the scene texture's UVs, so they
  keep working unchanged at world size; only their `resolution`-style
  uniforms and `screen_to_ripple_uv` must be fed the *world* size, which is
  what they already receive as long as the target is world-sized.
- `set_window_min_size` should be the world size times some floor (say 0.5)
  so a tiny window does not produce a 1-px-per-cell battlefield.
- `probe.rs` and the dev server keep taking the world size, never a window
  size; `DEFAULT_SCREEN_WIDTH/HEIGHT` are really `DEFAULT_WORLD_*` and want
  renaming when the split lands.

### Web (emscripten via raylib's `rcore_web.c`)

- **Resizing.** With `FLAG_WINDOW_RESIZABLE` set, raylib's
  `EmscriptenResizeCallback` sets the canvas backing store to
  `window.innerWidth x innerHeight` (clamped to min/max window size) on
  every browser resize, and `SetWindowState(FLAG_WINDOW_RESIZABLE)` does the
  same once immediately. That is the whole-tab canvas most web games want
  and it removes the need for a `bb_resize` export. It is in CSS pixels, not
  device pixels: raylib never multiplies by `devicePixelRatio`.
- **Device pixel ratio.** To render at native density on a DPR-3 phone the
  page has to size the backing store itself (`canvas.width = clientWidth *
  dpr`) and tell raylib through `SetWindowSize`, or the game has to read the
  DPR (`emscripten_get_device_pixel_ratio`, or a tiny JS shim) and do it
  from the Rust side. Either way it is a small custom step raylib does not
  provide; without it the browser upsamples a CSS-px canvas and the pixel
  art is soft on every phone. The scene target stays world-sized; only the
  final blit gets 3x the pixels.
- **Fullscreen.** `ToggleFullscreen` on web calls
  `Module.requestFullscreen(false, false)` (the Fullscreen API on the
  canvas) after a 100 ms delay, and it must be triggered from a user
  gesture. `ToggleBorderlessWindowed` instead resizes the canvas to the tab
  and back. **iPhone Safari does not implement the Fullscreen API for
  arbitrary elements**, only iPad does, so on an iPhone the fullscreen
  toggle is a no-op. The iPhone route to a chrome-less full screen is a
  web app manifest with `display: standalone` plus the
  `viewport-fit=cover` meta, i.e. "Add to Home Screen". In a normal Safari
  tab the best achievable is the tab's visible viewport (`100dvh`), which
  loses the address bar height in portrait and, in landscape, gets the
  whole width once the bar auto-hides.
- **Orientation lock.** `screen.orientation.lock` requires the Fullscreen
  API, so not on iPhone. Rotating the scene in-game (option D) is the
  workaround and the better product anyway.
- **Touch.** raylib registers touchstart/move/end/cancel on the canvas and
  fills the touch point list; `get_touch_point_count`/`get_touch_position`
  are available in the binding. Touch coordinates arrive in canvas CSS
  pixels, so the same `View` conversion that maps the mouse for the editor
  maps them. `-webkit-tap-highlight-color` and `user-select: none` are
  already set on the canvas; `touch-action: none` should be added so the
  browser does not interpret drags as scroll or pinch.
- **Viewport meta.** Add `viewport-fit=cover` and keep `user-scalable=no`
  out of the meta (iOS ignores it, and it hurts accessibility); pinch-zoom
  is stopped by `touch-action: none` on the canvas instead.
- Astro is unaffected: the canvas element and its CSS live in
  `index.astro`, the wasm and its glue are copied byte-for-byte, so every
  change here is either CSS/JS in the page or Rust in `main.rs`.

### iPhone checklist

The things that together make the game acceptable on an iPhone, in the order
they pay off:

1. `100dvh`, `viewport-fit=cover`, safe-area padding, `touch-action: none`.
2. Portrait rotation of the scene (needs the engine letterbox).
3. Touch input: the invisible floating joystick and tap-to-fire (scheme J),
   with the visible strip pad (scheme I) as a setting.
4. DPR-correct canvas size so the pixel art is crisp.
5. A phone-sized map set so tanks reach 9 to 11 mm; with scheme J the
   maps can be phone-aspect and fill the screen.
6. A web app manifest so Add to Home Screen gives a full-screen standalone
   window with no Safari chrome.

## 6. Recommended direction and what it touches

Layered so each step ships on its own and the probe/test baselines stay
byte-identical throughout (the world stays 1280x720 for the default map).

**Step 1: split world size from window size.**
- `MapFile` gains an optional `size = [cols, rows]` (default 40x22.5 cells,
  i.e. 1280x720, so every existing map and fixture is unchanged and the
  determinism tests keep passing). `Game::init` takes the world size from the
  map, not the window.
- `DEFAULT_SCREEN_WIDTH/HEIGHT` become the default *world* size; `main.rs`'s
  `--resolution` becomes the initial *window* size; `probe.rs` and the dev
  server use the world size only.
- Nothing visible changes yet.

**Step 2: the `View` and the letterbox blit.**
- One small struct in `game.rs` (or its own `view.rs`): `scale`, `offset`,
  `rotated`, `world_size`, `screen_size`, with `world_to_screen`/
  `screen_to_world`, built once per frame from `get_screen_width/height` and
  the world size. Optional `integer_snap` flag.
- `Game::render`: scene target is world-sized (`load_render_texture(world_w,
  world_h)`), the blit becomes `draw_texture_pro` with the destination
  rectangle from `View` (rotation for portrait), the shake offset scaled
  by `View::scale`. Muzzle/impact ripple quads and `screen_to_ripple_uv`
  keep world coordinates and are drawn through the same destination
  transform.
- HUD and banners are drawn in screen space, positioned via `View` and
  sized by a UI scale derived from the screen height (e.g. font =
  `clamp(screen_h / 28, 14, 40)`), never by `View::scale`. Same for
  `draw_debug_overlays` (positions through `View`, text at UI size) and the
  editor's mouse (`screen_to_world`).
- Bars: black first; a follow-up can draw the ground tiles or a HUD panel
  into them.

**Step 3: window modes on native.**
- `init().size(...).resizable().highdpi()`, `set_window_min_size(world/2)`.
- F11 toggles `toggle_borderless_windowed`; `--fullscreen` flag starts
  there. `is_window_resized()` just rebuilds `View`; nothing else cares.

**Step 4: web sizing.**
- `FLAG_WINDOW_RESIZABLE` on web so raylib tracks the tab; CSS becomes
  `width: 100vw; height: 100dvh` with safe-area padding and
  `touch-action: none`; DPR-sized backing store via a small JS-side resize
  handler calling an exported `bb_resize(w, h)` (or `SetWindowSize` from a
  DPR read on the Rust side). A `manifest.webmanifest` with
  `display: standalone`, `orientation: any`.

**Step 5: touch input in `main.rs`.**
- A `TouchScheme` setting with two implementations behind one function
  that turns this frame's touch points (`get_touch_point_count`/
  `get_touch_position`, mapped through `View`) into the same `Input` the
  keyboard produces. Default: the floating joystick (origin at the first
  touch on the steering half, dominant-axis 4-way snap with a dead zone and
  a diagonal hysteresis band, release = stop) plus tap/hold-to-fire on the
  other half, with a swap for handedness; a stick and knob drawn at low
  alpha only while touched, and a first-round hint. Option: the visible
  D-pad and fire button drawn in the letterbox strip after the blit. A
  mouse fallback makes both testable on desktop and through the dev
  server's `input` tool.

**Step 6: phone-sized maps.**
- Author two or three maps at about 24x14 cells, and at least one at the
  phone's own aspect (26x12) for the invisible-controls layout, with the
  editor (which needs the `size` field from step 1 to show the right
  canvas), lint them, add fixtures. Decide selection policy (section 7).

Everything in steps 2 to 5 is presentation and input; `simulation/` does not
change and the determinism tests, probe baselines and fixture budgets stay
valid, which is the main reason to do it in this order.

## 7. Open questions

- **How does the game choose a phone map?** By screen size at start
  (automatic, invisible, but a desktop player with a small window gets the
  phone map), by a map picker on the start screen (explicit, needs UI), or
  by making every map a compromise size around 32x18 (simplest, costs
  desktop density). The automatic rule with a picker override is the usual
  answer.
- **Square or landscape maps for phones?** Square avoids rotation entirely
  and fits both orientations identically; landscape-with-rotation uses the
  screen better on tablets and desktops. The map format supports both once
  `size` exists, so this can be decided per map.
- **Integer-snap by default or as an option?** Measure the shimmer on a real
  phone after step 4; it may be invisible at DPR 3 with a static floor.
- **Where does the HUD go on a phone?** Into the freed strip next to the
  controls (Battle City's side panel), or over the battlefield's corners as
  now. The strip is likely right, since 26 px text over a 0.55-scaled
  battlefield is unreadable anyway.
- **Fullscreen on iPhone without Add to Home Screen** is not achievable in
  Safari; is the manifest route acceptable as "the" iPhone experience, or
  is the in-tab layout the target?
- **Which thumb steers by default?** The user's proposal is right-thumb
  joystick, left-thumb fire; most dynamic-joystick games ship the mirror
  image. Both are one setting; the question is only the default, and it
  should be settled with a real phone in hand.
- **How much thumb occlusion is acceptable?** With the invisible scheme
  the thumbs cover part of the arena while steering. If playtests show the
  bottom rows matter too much, the fallback is scheme I's strip, or a map
  design that keeps the bottom band as the player's own side.
