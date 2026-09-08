# Phones, iPads and screen scaling — exploration

Status: exploration, nothing implemented. Companion to
docs/hud-and-builder-layout-design.md: that doc puts a HUD panel next to
the battlefield on a desktop window; this one asks what happens when the
screen is an iPhone or an iPad, and lands on one mechanism that serves
desktop, retina, phone and tablet alike.

## What runs where today

- **raylib 6.0 (bundled by sola-raylib-sys 6.3.0) has no iOS backend.**
  Its platforms are desktop (GLFW/RGFW/SDL/Win32), web (emscripten),
  Android and DRM. `sola-raylib-sys`'s `build.rs` has an Android branch
  (`cargo ndk`) but nothing for `aarch64-apple-ios`. A native iOS build
  would mean `PLATFORM=SDL` (SDL runs on iOS) plus a new build-script
  branch, an Xcode wrapper and App Store review - real work, and not the
  first step.
- **The web build is the iOS path.** Safari runs the emscripten build as
  is: WebGL 1 / GLSL ES 100 (the shaders are already ported), wasm with
  memory growth, and raylib's web platform registers `touchstart/end/move/
  cancel` on the canvas, so `get_touch_point_count`/`get_touch_position`
  and the gesture system already work; a single touch also drives the
  mouse position, which is why the editor's click-to-place works on a
  tablet without changes. Up to 8 touch points, so move + fire at once is
  fine.
- **Two web bugs found while reading `rcore_web.c` and the page**, both
  invisible on a 16:9 desktop and both wrong on a phone:
  1. `site/src/pages/index.astro` sizes the canvas with `width: min(100vw,
     1280px); height: min(100vh, 720px); object-fit: contain`. raylib maps
     touch and mouse coordinates by scaling `targetX/Y` over the canvas
     element's **CSS box**, but `object-fit: contain` draws the 16:9 image
     letterboxed *inside* that box. On an 852x393 iPhone the box is
     852x393 while the image is 698x393, so every touch is off by up to
     77 pt horizontally; the same happens in any desktop window narrower
     than 16:9. Fix: size the element itself to the aspect (`aspect-ratio:
     1280 / 720` with `max-width: 100vw; max-height: 100dvh`, or a resize
     handler), so the box *is* the image and `object-fit` is not needed.
  2. `game_loop::run(rl, thread, 120, ...)` passes `fps = 120` to
     `emscripten_set_main_loop_arg`, and a positive fps there means a
     `setTimeout` cadence, not `requestAnimationFrame`. On a 60 Hz phone
     that is jittery and burns battery; pass `0` on emscripten (vsync-
     paced rAF) and keep 120 for native only.
  Also: `-sASYNCIFY=1` is on (marked experimental in `.cargo/config.toml`)
  and costs wasm size and per-call overhead, which shows first on a phone;
  `100vh` is unreliable in iOS Safari (toolbar) - use `100dvh`.

## The mechanism: a virtual resolution and one scale-blit

The battlefield is 1280x720 and stays so (every map, probe baseline and
lint fixture assumes it - see the layout doc). The layout doc adds a 32 px
HUD bar and grows the window to 1280x752. Neither of those is a phone
size, so the whole picture gets rendered once and then **fitted**:

```
  Game renders the field into scene_target (1280x720)   - exists today
  hud.rs / editor.rs draw UI in *screen* space           - new
  main.rs blits scene_target into the field rect        - blit_offset exists;
     of a Layout computed from the real screen size        add a scale
```

- `Layout::fit(screen_w, screen_h, dpr, touch: bool)` picks the field
  rect: the largest 16:9 rectangle that fits after reserving the panel
  and (on touch) the control zones, giving `field.scale` and
  `field.origin`. Everything the earlier doc says about the field origin
  (`blit_offset`, ripple UVs, mouse minus origin) gains one multiply by
  `scale`; the simulation still sees 1280x720.
- **UI is drawn at screen resolution, sized in points**, not in field
  pixels. A 32 px bar at the iPad's 0.8 field scale would be 25.6 pt, under
  Apple's 44 pt touch minimum; drawing the panel in screen space lets the
  bar be 32 px on a desktop and 48 pt on a tablet while the field texture
  underneath scales freely.
- **Pixel-friendly scales.** Every sprite draws at scale 2 (tanks, walls'
  baked 2x pixelation, ground, shells), so the art's native grid is 2x2
  blocks: a field scale of exactly 0.5 lands each block on one device
  pixel and stays crisp; other scales resample. Snap to 0.5 when the fit
  is within ~5 % of it (an iPhone is), otherwise blit with linear
  filtering and accept the softness - rotated tanks and text already have
  1 px features that no scale keeps perfect.
- **Retina.** raylib's web platform sizes the canvas backing store to the
  logical screen size and reads `devicePixelRatio` only for
  `get_window_scale_dpi`. For a crisp result the backing store should be
  the physical size (`emscripten_set_canvas_element_size` to CSS size x
  DPR, i.e. `set_window_size`) and the fit computed in physical pixels;
  the field texture is then downscaled once, by the blit, instead of
  twice. Same code path gives desktop retina a sharp HUD.
- This is the "later" item the layout doc lists under small screens, and
  it also answers `--resolution` on a 1366x768 laptop: the window is
  whatever the OS gives, the field fits inside.

## Where the panel goes: into the leftover space

Once the field is fitted, the leftover area is where the panel and the
touch controls live. Its shape depends on the device, and the panel
follows it rather than forcing a bar on every screen:

| Screen (points, landscape) | Field fit | Leftover | Panel |
| --- | --- | --- | --- |
| Desktop 1280x752 window | 1.0, 1280x720 | 32 px strip on top | the 32 px bar (variant A) |
| iPad 1024x768 (4:3) | 0.8, 1024x576 | 192 pt band below | 48 pt HUD bar + 124 pt control band (stick zone left, fire zone right), home indicator below |
| iPad 1180x820 (Air/10th) | 0.92, 1180x664 | 156 pt band below | same, tighter |
| iPhone 852x393 (19.5:9) | 0.5 snapped, 640x360 | 106 pt columns each side, 16 pt top/bottom | readouts at the top of each column, stick zone in the left column, fire zone in the right; the notch sits in a column, the safe-area inset keeps readouts off it |
| Phone in portrait | 0.31, 393x221 | unplayable | show "rotate your device"; landscape only for now (a 90-degree rotated world is possible later and would need the stick's axes rotated too) |

On a phone the builder is out of scope: cells are 16 pt at scale 0.5 and
the dropdown lists have no room. On an iPad it works as on desktop with
44 pt targets: tap places, drag paints, the category dropdowns open as
lists under the bar.

```
 iPhone, landscape, 852 x 393 pt                      iPad, 1024 x 768 pt
 ┌────┬──────────────────────────┬────┐   ┌───────────────────────────────┐
 │♥100│                          │W2/5│   │                               │
 │▬ 12│                          │▮▮▮▯│   │      field 1024 x 576         │
 │    │     field 640 x 360      │    │   │                               │
 │    │                          │    │   ├───────────────────────────────┤
 │ (+)│                          │(●) │   │ HUD bar 48 pt                 │
 │move│                          │fire│   │ (+) move          fire (●)    │
 └────┴──────────────────────────┴────┘   └───────────────────────────────┘
```

## Touch controls

(The stick-and-fire-zone scheme below is one of two; docs/tap-orders-
design.md explores tap-to-move and tap-to-attack, which needs no reserved
zones and lets the touch layout be the desktop bar. The two coexist: a
tap is an order, a drag is the stick.)

The simulation's input model is already touch-shaped: movement is four
directions, constant speed, no momentum, snap-on-press and stop-on-release
(`Intent::move_dir`), and fire is a raw held state (`Intent::fire`, edge-
triggered for shells and full-auto for the laser inside `Game::update`).
`main.rs` is the only file that reads raylib input, so touch is one more
way to fill the same `Input`; nothing under `simulation/` changes.

- **Move: a floating stick in the left zone.** The first touch that lands
  in the move zone becomes the stick; its direction is the dominant axis
  of (current - anchor) once past a 16 pt dead zone, and the anchor
  drifts along behind the finger (re-anchor when the offset exceeds ~48
  pt) so a long push never needs the thumb to travel back. Release stops.
  No drawn d-pad to aim at; a faint ring at the anchor while held.
- **Fire: hold anywhere in the fire zone.** Held state maps straight to
  `Intent::fire`, so shells fire on the press edge and the laser runs
  while held, exactly like Space.
- **Pause, restart, BUILD/PLAY** are 44 pt buttons in the HUD bar; on a
  phone pause and restart go to the top of the columns.
- **Gamepads** work in Safari through the Gamepad API (MFi, PlayStation,
  Xbox controllers pair with iOS), and raylib's web platform maps them, so
  the same `Input` gathering can read a pad as a third source with no
  design work.
- The stick and fire zones are the leftover columns/band on a phone or
  tablet; when there is no leftover space (a 16:9 tablet, say) they
  overlay the field's bottom corners as translucent rings, which is the
  one case where something touches the field.

## iOS web specifics

- **Fullscreen.** iPhone Safari has no element fullscreen API; the way to
  lose the browser chrome is a **PWA**: `manifest.webmanifest` (`display:
  fullscreen`, `orientation: landscape`, icons) plus the
  `apple-mobile-web-app-capable` meta and touch icons, then "Add to Home
  Screen". iPad Safari does support element fullscreen, so a fullscreen
  button in the bar works there and on desktop.
- **Viewport.** `viewport-fit=cover` on the meta tag and `env(safe-area-
  inset-*)` paddings so nothing important sits under the notch or the home
  indicator; the game can read the insets from JS (a `Module` value the
  page sets) if the panel wants to place readouts precisely.
- **Gestures Safari owns.** `touch-action: none` and `-webkit-touch-
  callout: none` on the canvas (the page already disables selection and
  the tap highlight); raylib's touch callbacks prevent default on the
  canvas, but the page around it still needs pull-to-refresh and double-
  tap zoom off during play, and `overscroll-behavior: none` on the body.
- **Audio, when it arrives**, needs a user gesture to unlock on iOS; the
  page already has the `AudioContext.resume()` click listener, it will
  need a `touchend` twin.
- **Assets** are 584 KB under `static/` bundled into `bongbong.data`, fine
  on cellular. The tuning panel stays desktop-only (it is below the canvas
  and mouse-sized).

## App Store, if wanted

| Path | Effort | What you get |
| --- | --- | --- |
| PWA (web build + manifest) | small; site changes only | home-screen icon, fullscreen, no review, updates on deploy |
| WKWebView wrapper (Capacitor or a bare Xcode project shipping `site/dist/`) | medium | an App Store listing; Apple allows bundled HTML5 games, remote-loaded ones less so; a controller and haptics through the bridge |
| Native via raylib's SDL platform | large: build-script branch, SDL for iOS, Xcode, signing | native performance and Metal via ANGLE; only worth it if the web build cannot hold 60 fps |
| Android native | small: `sola-raylib-sys` already builds `PLATFORM=Android` with `cargo ndk` | a Play Store build for comparatively little, if Android ever matters |

Start with the PWA: it is the same build the site already deploys, and
every scaling and input change above is needed by it and by any wrapper.

## What changes, in order

1. **Web fixes now**, independent of everything else: canvas element
   sized to the 16:9 aspect (touch mapping bug), `fps = 0` on emscripten,
   `100dvh`, `touch-action: none`.
2. **`Layout::fit` and the scale-blit** in `main.rs`/`game.rs`: field rect
   with origin and scale, UI in screen space, physical-size backing store.
   This is the small-screens item of the layout doc and lands before or
   with the HUD bar.
3. **Touch input in `main.rs`**: the floating stick and the fire zone,
   gated on `get_touch_point_count() > 0` so a desktop never sees it.
4. **PWA metadata** in `site/`: manifest, icons, meta tags, safe-area CSS.
5. **Playtest on an actual iPhone and iPad** through a PR preview
   (`pr-<N>.preview.bongbong.io` already exists for exactly this), then
   decide about a wrapper.
