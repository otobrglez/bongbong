# Codebase reference

Module by module: what each file owns, the rules it keeps and where its design doc is. CLAUDE.md
carries one line per module and the invariants every session needs; this is the detail behind
it, read on demand. When a change alters what an entry says, update the entry here (and the
module's design doc); touch CLAUDE.md only when an invariant changes.

`render/<m>.rs` is the raylib half of `<m>.rs` under the same item names (CLAUDE.md,
Conventions). Tooling, tests and the asset pipeline are in docs/tooling.md; the room server and
the platform builds in docs/platforms.md.

Contents: [Layout](#layout), [App, session and data](#app-session-and-data),
[Simulation](#simulation), [AI](#ai), [World entities](#world-entities),
[Weapons and projectiles](#weapons-and-projectiles),
[Drawing and effects](#drawing-and-effects), [HUD and chrome](#hud-and-chrome),
[View and cameras](#view-and-cameras), [Builder](#builder), [Online](#online),
[Dev tooling surfaces](#dev-tooling-surfaces), [Seats](#seats).

## Layout

- `src/main.rs` is a shim: parse `Args`, call `bongbong::app::run` (iOS: `app::ios::main`).
  `src/app.rs` is the game: the `Args` struct, window/texture/shader setup, the main loop.
  `src/app/ios.rs` and `src/app/android.rs` are the platform entries, and `src/app/macos.rs` moves a
  run from inside `BongBong.app` into its `Resources` and its saved maps to the user's data
  directory (`map::set_maps_dir`, `levels::data_dir`), and gives a run outside it the app's icon in
  the Dock (`dock_icon`: the embedded `AppIcon.png` through the Objective-C runtime once GLFW has
  made `NSApp`); `android/` is the cdylib crate. The `bongbong`, `bbmcp` and `mapshot` bins carry
  `required-features = ["render"]`; `probe` builds without it.
- `static/` assets and shaders, `tools/` scripts, `maps/` TOML maps (`maps/stamps/` the builder's
  shipped stamps), `levels.toml` the level order (docs/levels.md), `docs/` design docs and sheet
  specs (read the doc a module names before changing it), `site/` the Astro web page, `k8s/` the
  room server's manifests (see its README and docs/platforms.md, Room server), `tests/` integration
  tests, `netlab/` the network measurement harness (docs/tooling.md), `ntk/` the conference demo
  crate (`just ntk <bin>`: numbered single-file raylib examples; `06_explosions` runs the real
  engine through the `bongbong` lib and is the only one that depends on it).

## App, session and data

- `app.rs` — owns the raylib handle, the loop and the `mode::Session`. Play: gathers a
  `simulation::Input` (`Input::two` from the keyboard and touch) and runs `Game::update` in whole
  steps of `PHYSICS_FIXED_DT` - `StepClock` pays real frame time out as zero or more steps per
  rendered frame, at most `SIM_MAX_STEPS_PER_FRAME` (4, the rest dropped, so a stall never spirals;
  one step every other frame on a 120 Hz display), every step on the frame's input with the one-shot
  presses spent by the first (`Input::held_only`); a frame that ran no step carries its presses into
  the next (`Input::or_presses`) so a tap is never lost; a dialog, the builder or the dev server's
  freeze resets the clock, so a resume starts on a fresh step. `Fx::observe_events` runs after every
  step so a two-step frame drops no burst, and so does the `indicators::ScreenAwareness` for
  `local_seats` (seat 0, seat 1 on a couch, the room's seat once there is a replica; once a frame on
  a replica); its picture is composed once a frame only while `!camera.shows_whole_field()`, with
  the field's rect and `window_units_per_point / view.scale` (the screen density over 160 on
  Android, else 1), and passed as `Effects::indicators`; the round's minimap
  (`minimap::RoundMinimap`, synced and uploaded only while the corners hold its slot) goes as
  `Effects::minimap`, the lava's kept pictures are brought up over the frame's views
  (`Game::refresh_pictures`) and every `BlockImage` the round draws from is synced through one
  `BlockTextures` (`Game::with_block_images`, `Textures::blocks`), and `Session::minimap_on` is set
  from `Presentation::minimap_on` before the hit tests; touch screens (no keyboard,
  `--touch-from-mouse`, or `TouchScheme::seen`) add the thumb rests as keep-outs. Build: gathers an
  `editor::BuilderInput` in window coordinates and calls `Session::update_builder` with the frame's
  `editor::BuilderFrame`. Online (docs/online-coop-prd.md §4.5): gathers the same `Input` and hands
  seat 0's `Intent` to `Session::update_online`, which is the whole frame - no `StepClock`, no
  `Game::update`, and `Session::shown()` rather than `session.game` is what the render, the HUD and
  `fx` read; Esc gives the seat up. Lobby: gathers a `lobby::LobbyInput` (the pointer through
  `Layout::to_field`, the typed characters, Backspace/Enter/Escape) and calls
  `Session::update_lobby`, which is that frame whole - the local round stands still behind the
  screen and the touch stick is not drawn over it. `open_online` refuses `--host -m` with a map that
  is not `hostable()` before dialling (a `transport::Failed` with `note-not-cleared`, so the lobby
  shows its closed face with the reason; client-side only - the room server checks no stamp).
  `open_online` builds a round from the command line: `--rig` (`--delay`/`--jitter`/`--loss`) starts
  `net::rig` and plays its loopback end, `--host`/`--join CODE` build a `net::client::Target` and
  dial it through `net::client::connect`; either lands in the lobby (`Session::open_lobby_with`),
  which hands over to `Driver::Online` when the round starts. `--rooms` and `--nick` are handed to
  the session, so the screen's own `HOST`/`JOIN` reach the same server under the same name; a
  `--nick` picks the reconnect token `bongbong-<nick>`, and without one (every phone) the window is
  `net::client::Identity::anonymous` - a token minted for the run and the `Player #ABC102` read off
  it (`anonymous_nick`) - so two windows are two seats, never one seat each reclaims from the other
  (`cli_identity`); the web reads the name off its tab's token (`Identity::anonymous_with`). **On
  the web there is no command line**: `page_string` reads `window.bbInvite` and `window.bbToken`
  once at startup through `emscripten_run_script_string` (the `bbShift` pattern; the answer is a
  pointer into emscripten's own buffer, copied on the spot, and every script is wrapped so a throw
  cannot take the runtime down), `net::rooms::Invite::parse` turns the page's URL into the room and
  the rooms override, and a link naming a room goes straight to `Session::join_room`. Nothing past
  this file touches `RaylibHandle` for input. The HUD's corner buttons and the dialogs are
  hit-tested before the frame's input is gathered, and `Session::playing()` is false while a dialog
  is up, so the round is frozen by construction (no simulation flag). `Tab` toggles modes,
  `--editor` starts in Build (the round is initialised as usual, only the driver differs). **A
  session opens on a level** (docs/levels.md): the furthest reached (`load_progress` - the page's
  `localStorage` on the web, a file elsewhere) or `--level`'s; `-m` is free play. After every
  frame's steps `Session::note_outcome` and `take_progress` hand a won level to `save_progress` and
  `Session::follow_countdown` takes the way a level's end screen counted down to, and in Play mode a
  press on a level's end screen (`press_result`) and Enter (`enter_result`) come after the dialogs
  and before the corners' buttons. The level select comes before everything while it is up (a
  `level_select::SelectInput` from the pointer, the arrows, Enter and Esc/Tab, every press claimed
  from the touch scheme); Esc opens it in play mode, as the bar's level button does. **The field is
  the map's, the window is anyone's** (docs/fullscreen-resolution-research.md,
  docs/hud-and-builder-layout-design.md): an arena's bitmap (and a pinned view's) is the field alone
  (`Layout::bare`) in every mode, rendered into one `composite` render texture and fitted by
  `view::View` (uniform scale, centred); in Build the bar stands on the window in UI points and the
  canvas under it (`editor::BuilderFrame`: an arena fitted at its round's capped scale and
  letterboxed, a field map's canvas made to the shape of the window under the bar,
  `editor::chrome::canvas_frame`); where an arena's window has room past its field - in play, online
  and behind the lobby - the margins show the world past the boundary (`margin.rs`,
  `render::margin::MarginFx`, owned here and passed as `Effects::margins`), and the builder
  letterboxes in the bar colour. **A field map is followed** (docs/large-maps-follow-camera.md):
  `Presentation::of` picks the bitmap (the whole field for an arena, the builder and a pin; a
  `FollowFrame` for a `MapClass::Field` map, `Seating::Room` online and `Local` otherwise) and the
  targets (the view plus one block), re-made when a press or a countdown changes the mode or the map
  mid-frame; `screen()` gives the window in points and `GetWindowScaleDPI` - natively the monitor's
  physical width where it reports one (GLFW's mode is points on macOS, pixels elsewhere), else 96
  ppi; the web the canvas's box in CSS pixels at 96 ppi and 0.2646 mm per point (its window is the
  canvas buffer in device pixels, `web::units_per_point` of them to the CSS pixel), except a touch
  screen's page (`window.bbTouch`, `web::touch_screen`), which is framed as the app on that device;
  iOS and Android fine, size unknown, Android's pixels turned into points. After the steps
  `Follow::update` places the view (on the local round's steps, a replica's frame time online,
  nothing while frozen) for `local_seats`, `Camera::following` snaps it at
  `device_scale(framebuffer_ratio)`, and dev builds `publish_camera`. A couch pair apart on a field
  map gets a second camera, its targets made once (`split_targets`); the indicators, the HUD fade
  and the minimap take each half on its own side of the divider. While a round opens behind its
  mission banner on a field map, `Presentation::of(.., establishing)` draws the establishing shot
  (`establish.rs`) through whole-field targets with no margins, the follow camera running
  underneath. The field size comes from the map (`MapFile::field_size`, re-read every frame so a
  dev-server restart on another size re-creates the targets and ripple shaders); `--resolution` is
  the window's initial size; `DEFAULT_SCREEN_WIDTH/HEIGHT` (1088 x 544, 34 x 17 cells) is the field
  a map without `size` gets. The scale cap (`View::fit_capped`, `view_max_scale` 1.5, `--zoom`
  stages the knob) keeps an arena from being drawn huge on a monitor; iOS and Android pass no cap,
  and the web caps in points. Native windows are resizable + highdpi, F11 / `--fullscreen`
  borderless. A pointer is read on the window for the builder (its `BuilderFrame` puts it on the
  canvas or the chrome) and through `UiFrame::to_ui` for the chrome and the touch scheme (play's
  touch area is the whole window in points, `UiFrame::screen`; the builder's none). **Hints follow
  the input last used** (`hud::Hints` on `UiFrame::hints`): a touch landing turns the words to taps
  and a key press back to the keys (`key_pressed` drains raylib's key queue, Android's Back, Menu
  and volume keys left out), while `UiFrame::touch` stays sticky, so a key press changes the words
  and never a button's size; the dev server's `key`, `builder_touch` and `click {touch}` stand in
  (`DevServer::take_hints`). **A camera drawn straight onto the window carries the HighDPI scale**
  (`render::view::window_camera_units`, `onto_window`): raylib's `BeginMode2D` leaves out the screen
  scale plain drawing gets, so on a desktop HighDPI window the bitmap-to-window camera, play's UI
  camera and the builder's chrome camera are given the framebuffer's pixels per window unit (1 on
  iOS, the web and Android, and into any render texture). **The chrome is in UI points**
  (`hud::UiFrame` from `ui_frame`: window units per point times `ui_scale`, shrunk where 720 x 352
  pt would not fit the safe area - iOS's from `app::ios::safe_area_insets`, SDL3's
  `SDL_GetWindowSafeArea` - and `touch` with no keyboard, under `--touch-from-mouse`, once a touch
  is seen, or on the frame one lands - `TouchScheme::touch_chrome`, decided before the builder's
  update, so a finger lifts from the frame it landed on), drawn on the window at the UI scale, never
  the world's; `hud::Fade::step` fades a corner cluster to `ui_fade_opacity` while a drawn tank,
  shot or blast is under it and back over `ui_fade_seconds` (the `ui` tuning group), and
  `Corners::keep_out` is a keep-out for the touch scheme and the off-screen arrows. The desktop
  window opens at play's bitmap (the builder's with `--editor`) - the standard field's for a map the
  camera follows - at the scale cap, shrunk to the monitor; its smallest is half the standard
  field's bitmap. Touch goes to `touch.rs` (`--touch-from-mouse` for dev); a press on the field is
  never a play input. `game_loop::run`'s fps is 0 on web (rAF) and 120 native, see docs/platforms.md, Web.
- `mode.rs` (docs/game-editor-fusion.md §5, docs/online-coop-prd.md §4.5, §4.10) — `Session {
  driver, game, builder, dialog, players_dialog, online, lobby, rooms, nick, token, campaign,
  level_select }` owns the round, the builder and the seat in a room for the whole session;
  `playing()` (play mode, no dialog, no level select) is the one predicate deciding whether `update`
  runs - the leave dialog is not a simulation pause, and neither is the lobby or an online round.
  `press_build` (asks while a round is in progress), `answer_dialog`, `play` (`MapEditor::leave`,
  then a fresh `init` on the builder's map, seed pinned - there is no resume), `toggle`,
  `replace_map`, `update_builder`, `play_chrome`, `press_players`/`answer_players` (a new count
  restarts). **`Driver::Lobby`** is the room screen (`lobby.rs`): `press_online` opens it over the
  parked local round, `open_lobby_with` opens it on a round the command line already dialled,
  `join_room` opens it on the room a page's invite link names (the web build's only way in,
  docs/online-coop-prd.md §4.10), and `update_lobby` is one frame - poll the room, run the screen's
  hit tests, then do what it asked (`HOST`/`JOIN` dial through `net::client::connect` with `rooms`,
  `nick` and `token` - the reconnect key, the nickname's on a desktop and this tab's own in a
  browser, which is what makes two clients two seats; `READY`/`START`/`KICK` go to the round;
  `LEAVE` is `leave_online`). The frame the room reports the round has begun it hands over to
  **`Driver::Online`**, the fourth driver: `update_online` is one frame of the
  `net::round::OnlineRound`, and `shown()`/`shown_mut()` - the replica while there is one, the local
  round before the `Welcome` - is what every drawing path and every one of the dev server's readers
  takes. The frame the room reports the round over (`OnlineRound::ended`) it hands **back**: a fresh
  `Lobby` over the same seat and room, so the code, the QR and the roster are still there and the
  host's action reads `REMATCH`. `leave_online` is the only thing that gives the seat up, and comes
  back to the local round, which has stood still the whole time. `Deref` to `Game` still reaches the
  *local* round, so the dev server keeps driving that one. No raylib here. **Levels**
  (docs/levels.md): `campaign` (`set_campaign`) and `level()`, the index of the local round's map
  name; every map change sets `Game::hold_end_screen` from it (`sync_level`), so a level's end
  screen counts down into a hold while free play restarts on its own, and **Play from here**:
  `play_here` is `play` with seat 1 on `play_here_cell` (`Game::drop_cell`/`battlefield::drop_cell`:
  spawn-legal, dry, off portals, in the map's start's nav component, nearest the builder view's
  centre) as `Game::start_override`, kept by restarts and cleared by PLAY, a level or a new map;
  such a round plays for no level (`level_round`: free play's end screen, which restarts it on its
  spot, no way on) and clears nothing. **The clear check** (docs/large-maps-patterns.md, "Clear
  check before sharing"): `play` sets `clear_attempt` (the canvas's `MapFile::revision`) and
  `note_outcome` puts a win's first frame to `note_clear` - cleared when the round is that revision
  played as authored (`played_as_authored`: one seat, no start, enemy, mission, spawn, wave or
  chassis override, the map's own sky and the `player_tank` knob at its default), the par its round
  clock, the best kept (`MapEditor::note_clear`). `follow_countdown` (after every frame's steps)
  takes the way it counted down to: `next_level` after a win, `play_again` after a loss, nothing
  after the last level's win, which waits for its button. `start_level` opens a level's map (the
  session's edit first) on the field and in the builder as a new document (`MapEditor::open`);
  `play_again`, `next_level` (won levels only; the last leads to the first); `play` remembers a
  level's canvas as its edit; `note_outcome` after every frame's steps turns a win into progress and
  `take_progress` hands it to the store once; `press_result`/`enter_result` are the end screen's one
  entry for a click, a tap, the dev server and Enter, hit-tested on `hud::result_layout`. **The
  level select** (`level_select.rs`) is `level_select: Option<LevelSelect>`: `press_levels` opens it
  (play mode with a campaign; it closes a dialog, and `playing()` is false while it is up, so the
  round is frozen the dialogs' way) or closes it, `update_level_select` is one frame of it (a
  `Start` is `start_level`), and `level_button` is the HUD's level button's number - a local round
  on a level - which the painter and every hit test read. `minimap_on` (set by `app.rs` every frame
  before the hit tests from `Presentation::minimap_on`: play or online, a view that shows part of
  the field - a followed map larger than its view, or a pinned zoom - and `MinimapRules::shown_on`
  the window's `Screen`) is what `play_chrome` lays the minimap's slot out from.
- `level.rs` — `Mission` (Protect/Hunt/Destroy), `SpawnKind` (Band/Waves), `Tier`
  (`TANK_TIER_BY_ROW`), `LevelOverrides` (one field per CLI flag, shared by `app.rs`, the probe and
  the dev server), resolved in `Game::init` as CLI > map > the `waves` tuning group, then sized to
  the team by `SpawnPlan::scaled(wave_size_scale, wave_tier_step)` - the identity on every local
  round, since only a room sets those two rows (docs/platforms.md, Room server).
- `levels.rs` (docs/levels.md) — the levels: `levels.toml` (repo root, compiled in as `LEVELS_TOML`)
  parsed into `Levels` (`[[level]]` with `map`, a `SHIPPED_MAPS` name, and the English `title`;
  `find` is `--level`'s number-or-name), and `Campaign`, the session's place in them - `reached`
  (the furthest level, moved only forward and only by `won`), the builder's edits by map name
  (`remember_edit`, `map` returns the edit else `map::open_map`) and `take_unsaved` for the store.
  **A level is a map by name** (`Session::level`), so anything else is free play. The progress
  store's pure half is here (`progress_text`/`parse_progress`, `read_progress`/`write_progress`,
  `progress_path` - `BONGBONG_PROGRESS`, else the platform's data dir); `app.rs` picks the
  platform's store (the page's `localStorage['bongbong.level']` on the web, `app::android::data_dir`
  on Android). `Level::title` is the catalogue's `level-<map>` in a language that has one, else the
  English. A level may be `skippable` (Boot Camp, level 0: a skippable first level is numbered from
  0, `Levels::number`, so the first real level stays level 1, and the last level leads round to that
  one): while it is the furthest reached, `Campaign::open_to` is the one after it, which the level
  select opens. Tests: `level_tests`.
- `level_select.rs` (docs/levels.md) — the level select, headless like `lobby.rs`: every level as a
  tile over the dimmed field, won levels and the furthest reached open (and the one after a
  `skippable` level while that is the furthest reached - `Campaign::open_to`,
  docs/training-stage.md), the rest locked - the way back to a level already won (a replay moves
  nothing; only a win on the furthest level reached opens another). A fixed 704 x 336 panel in UI
  points centred in the chrome's area (the lobby's; `hud::UI_MIN_W` x `UI_MIN_H` is this panel and
  its edges, so the UI frame shrinks the point on a window too small for it), sixteen tiles in two
  rows of eight; **`tile_rect`/`back_rect` are the one geometry** the painter
  (`render/level_select.rs`) and every hit test read, and `LevelSelectView`/`TileView` (`TileState`
  Won/Next/Locked, `current`, `focus`) the painter's plain data. `LevelSelect` holds only the
  keyboard's focus (the arrows walk the open tiles; the pointer takes it only by moving onto one);
  `update` turns a `SelectInput` into a `SelectAction` (`Stay`/`Close`/`Start(i)`) - a locked tile
  is no button, `BACK`, Esc and a press outside close it. `wrap` breaks a title onto two balanced
  lines. `SELECT_TILES` is one page, and a test holds `levels.toml` to it.
- `training.rs` (docs/training-stage.md) — a training map's script as data (`Training`:
  `start_health`, `start_shells` and the `beat` list; `Beat`: `id`, the frog's cell, its
  `say`/`nudge` message keys, `BeatStart`, `BeatDone`), read from and written back to the map's
  `[training]`/`[[training.beat]]` tables (`MapFile::training`), and the two training cells' drawing
  generic over `canvas::Canvas`: `draw_door` (banded iron with a hazard band; `map::CellObject::Door
  { beat }` spawns a permanent `Material::Door` whose `variant` is the beat) and `draw_flag`
  (`CellObject::Flag`, not solid). `maps/boot-camp.toml` is the course (five pens; the fourth a drum
  range any one drum sets off). The rules are `simulation/training.rs`.
- `bubble.rs` (docs/training-stage.md) — the frog's voice in a training round, headless: `FrogVoice`
  (owned by `app.rs` beside `fx::Fx`, `observe` after every step, `update` once a frame on the time
  the round ran) queues each beat's `say` keys when the beat begins (clearing what the last one
  still had), the situational lines from the frame's events (hit in its beat, healed, a bite, the
  minigun, the first enemy hit, a wrecked seat) and the beat's `nudge` after
  `training_nudge_seconds` of quiet, and holds a line `training_line_seconds` plus
  `training_line_seconds_per_char` of its length; `key_for` picks a line's `-touch` twin on a touch
  screen (`hud::Hints`); `layout` is the bubble's one geometry in the bitmap's pixels at the UI's
  points (sized for the whole line, above the frog, under it near the top or over a corner cluster's
  keep-out, at the view's edge with no tail while the frog is off it), `revealed` the words up so
  far, and `<ARROWS>`/`<SPACE>`/`<STICK>`/`<TAP>` tokens become key caps. `render/bubble.rs` paints
  it (`Effects::bubble`) after the indicators, under the HUD. The simulation never reads a word;
  every line is a `frog-*` key constant. Tests: `bubble::tests`.
- `map.rs` — the TOML map format (docs/map-editor-design.md): `size = [cols, rows]` in 32 px cells
  (rows may be fractional; absent = 34 x 17; at most `MAX_SIDE_CELLS`, 250, a side - what the wire's
  `i16` positions and `u16` cell indices carry - and at least `MIN_SIDE_CELLS`, 8, refused by name
  past either - a builder stamp's `size` is its own extent, read by `from_toml_str_any_size`;
  `maps/crossplay/` are size studies), `theme = "grass"|"desert"` (`Theme`, presentation only - the
  ground tileset and tall-grass sheet `app.rs` picks per frame, `Theme::drifts` for the sand
  patches; docs/desert-theme.md; absent = grass, not written back), `weather = "night"` or `["rain",
  "snow"]` (`Skies`, a set over `Weather::SKIES`: the skies `weather.rs` draws and whose rules it
  sets - enemy sight, grip, ice, gusts -, each round's seed picking one; `random` is every sky; one
  sky is written as its name, so older files re-save byte for byte; the MAP panel's SKY tiles;
  docs/weather.md; absent = clear, not written back), interior terrain only - walls, props (`kind =
  "sandbag"|"barrel"|"fence"`, `drum = "oil"|"fuel"` pins a drum), trees (`tree`/`pine`),
  `tall_grass` (not solid), `oil` trail cells, `portal` anchors (multi-instance, not solid,
  `portal_cells`; docs/teleporting.md), road, `water` (painted like road; `ground::build` draws a
  one-cell-wide line as a river and a wider block as a lake, both animated and flowing down the map -
  docs/GROUND_SPEC.md §9 - and the rules follow the shape, docs/water.md), frog, `enemy_frog`,
  `gate` (explicit gates replace the edge scan), pickup slots, `start`/`start2`, `lava` (painted
  like water: a stream is a burning ford, a lake's middle deep), `volcano` (the crater cell; its
  cone covers the 21 cells within `sqrt(5)`, `volcano_footprint`), `lamp` (a lamp post), `target` (a
  range board, docs/range-target-prd.md; `maps/range.toml` is a weapons range of them, not shipped) -
  plus top-level `nightfall` (seconds of the round when night falls; docs/volcano.md), `tanks`,
  `tank`, `tank2` and the `mission.*`/`spawn.*` tables, **written as dotted keys** in hand-authored
  files because a `[spawn]` header swallows every `cells.` line after it. Border walls and enemy
  spawns stay procedural. CLI flags outrank map defaults; the chassis chain is
  `simulation::resolve_player_row` (CLI → `player_tank` knob → map → roll).
  `CellObject::material()`/`is_solid()` fold props and trees in with walls.
  `to_toml_string`/`from_toml_str` back `map_get`/`restart {map_toml}`; `MapFile::name` is
  display-only. `[cleared]` (`MapFile::cleared`: a revision as 16 hex digits and a par) counts only
  for the map's own `revision()` (FNV-1a of the canonical TOML, the stamp and the name left out;
  `cleared_par`), and `hostable()` is a shipped map as it ships or one carrying a valid stamp.
  `SHIPPED_MAPS` (default, default-desert, hunt-basic, waves-basic, portals, towers, `longwater` -
  the 112 x 63 free-play field the follow camera is for, a lake across the map with a fort on its
  south shore, waves through six gates north of the water -, `armory` - the 36 x 18 arena the BB-36
  weapons are tried on, the one shipped arena (docs/sonic-hammer.md) -, then the fifteen
  hand-authored levels from `lotus-lagoon` (and `vulkan`, the volcano level) to `grand-campaign`,
  each file's header saying how it plays; every one a field map, 48 x 24 to 67 x 34 and longwater,
  since they were scaled up by 40 % a side with their layouts - `tools/scale_map.py`: lines kept one
  cell thick, single things single, two-wide fords two wide; then checked with the linter, since its
  reaches are in pixels) are embedded - all the web build can list, what the lobby's map stepper
  walks in that order and what the room server can open; a slug is drawn as-is in the stepper, so
  keep one within its 204 px at `HUD_TEXT_SIZE`. `map_source` is the text `open_map` parses (the
  file under `maps_dir()` natively, else the shipped one), and `fnv1a` the hash a revision is made
  from. `iter_cells` sorts (determinism). An editor re-save drops a file's header comments: edit
  fixtures by hand. `view = "whole"|"follow"` (`MapView`, `MapFile::view: Option<MapView>`; absent
  is by size and not written back) overrides the arena rule - `whole` only within `WHOLE_MAX_CELLS`
  (128, a 4096-texel target) a side; `MapFile::class` is the answer (`framing::MapClass`).
- `mapstore.rs` (BB-33) — the player's maps kept between sessions on every platform, on unless
  `--map-modding false`: a shipped map the builder changed is its modified copy, key `<name>-modd`,
  first line `# modified from <fnv1a of the shipped text>`, which `map::map_source`, the Load list
  (`MapEntry::modified`), the levels and the builder open in the original's place; a map of the
  player's own is kept under its name, one under a shipped name is ignored. A `Backend` per
  platform: `Dir` (`levels::data_dir()/maps`, Android's activity directory, `BONGBONG_MAPS` names
  one) and the web's `localStorage` (`app.rs`'s `PageMaps`, `bongbong.map.<key>`); `enable` once at
  startup, `store()` the run's, tests make their own `Store` over `Memory` and hand it to
  `MapEditor::use_store`. `Store::save` keeps no copy of a map that is its original again; `stale`
  is a copy of another original, which `Session::watch_originals` asks about once per map
  (`Question::OriginalChanged`: SWITCH reverts and restarts the round on the original, KEEP MINE
  re-stamps the copy); `Question::Revert` is FILE's. The question is `Session::question`, drawn by
  `render::hud::draw_question` over play and the builder, hit-tested on `hud::question_rects`
  (`screen_buttons` `yes`/`no`, `status.question`, `status.map_modding`); while it is up the round
  is frozen. Online rooms still play the shipped map by name (BB-34).
- `text.rs` (docs/localization-prd.md) — every string a player reads. `lang/<tag>.ftl` are Fluent
  catalogues embedded like `SHIPPED_MAPS` (`SHIPPED_LANGS`: `en`, the source and the per-message
  fallback, and `sl`); `text()` is the catalogue in force, an `Arc` snapshot with `tuning()`'s
  discipline, written only by `set_language` at startup (`app::run`: `--lang`/the page's `?lang=`,
  else the platform's list - `sys-locale`, `window.bbLang`, SDL's locales on iOS,
  `persist.sys.locale` on Android -, else English; `text::choose`/`negotiate`) and by the dev
  server's `lang` tool. Every message is a `text::keys` constant (`text_tests` holds `keys::ALL` and
  `lang/en.ftl` together both ways); data names are looked up by family (`named("tank",
  kind.name())`, `mission-`, `theme-`, `spawn-`, `tier-`, `tool-`/`tool-short-`, `stamp-` (a shipped
  stamp's file name), `lint-` by `LintKind::tag`; a level's title is `levels.toml`'s English and
  `level-<map>` in the other languages only, which `text_tests` allows), so `Mission::name`,
  `TankKind::name`, `Tool::name` stay the data and protocol spellings. **No custom font**: raylib's
  default draws U+0020..U+007E and U+00A0..U+00FF, and `fold` maps every other letter onto the
  Latin-1 letter it is built on (Č → C, Ő → Ö) from one table, applied as a message is resolved and
  to user text (a nickname) where it enters a model; `every_message_is_drawable` fails on a letter
  the table lacks. `width(text, size)` is `MeasureText`'s answer headless (the default font's
  224-entry `charsWidth` table and `DrawText`'s spacing), replacing every `len() * CHAR_W` guess;
  `fit` truncates with `~`. **Budgets**: `every_language_fits_every_budget` measures every
  language's message against the points its box has (the corners' slots, the banners through
  `fit_size`, the dialogs, the lobby's buttons and seat columns, the build bar), so a translation
  that does not fit does not merge. The room server's refusals are `net::wire::Refusal` codes the
  client puts into words (`text::refusal`); `net/round.rs` is the one `net/` file that reads the
  catalogue, and nothing under `simulation/` does (a test greps). The wave banner (`WaveBanner`) and
  the mission banner are data the renderer words. Tests build local `Catalogue`s; never call
  `set_language` in a test.
- `tuning.rs` — the runtime knob table (docs/runtime-tuning-design.md): `tunables! { group x { ///
  doc  name: ty = default in min ..= max [labels NAMES] [@ Live|Spawn|Restart]; } }` expands to
  `Tuning`, `Tuning::DEFAULT`, `Tuning::SCHEMA` (the doc comment is captured and shown by the panel)
  and range-checked `get`/`set` by name (`name.label` / `name.<i>` for array rows). Read via
  `tuning().field`: a shared `Arc` snapshot cloned under a lock that is released at once, so holding
  it across other `tuning()` calls and across a write on another thread is safe - the table used to
  hand out the read guard itself, and a write arriving between two nested reads (a client applying
  its room's patch, the room server's `init_under` in another room) deadlocked every reader; bind
  once in hot loops. **Writes only at the frame boundary**: transports stage a patch
  (`submit_json`/`submit_reset`/`submit_file`) and `app.rs` calls `apply_pending()` right before
  `Game::update`; nothing in `simulation/` mutates tuning. `request_restart` becomes
  `Input::restart_pressed`. Derived values are methods, not rows. Tests work on local `Tuning`
  values only - never mutate the global in a test (parallel threads).
- `math.rs` — the plain value types the simulation and the renderer share: `Vec2` (`Position` is its
  alias; positions, velocities and directions, the arithmetic written out component by component in
  the order raylib's `Vector2` performs it so seeded replays and the probe fixtures keep their
  bits), `Color` (`alpha`/`color_from_hsv` ported byte for byte from raylib, the named consts the
  game uses) and `Rectangle` (`contains` is `CheckCollisionPointRec`'s half-open test). Nothing
  under `simulation/` and no entity's state names a raylib type; the `From` impls to raylib's types
  at the bottom (`render` only) are the render boundary (draw calls take `impl Into`, so all three
  pass straight through). Add an operation the same way, never by calling out to another vector
  library.
- `lib.rs` — *layout* constants and types only: `Position` (= `math::Vec2`), atlas columns/sizes,
  `*_VARIANTS`, collider boxes, grid/physics sizes, `HUD_BAR_HEIGHT` (the builder's bar with a
  mouse, in UI points)/`Rect`/`Layout` (`bare`: the field alone at `(0, 0)`, play's bitmap and the
  builder's canvas; `for_field`: the field under a bar, origin `(0, 40)` - the size a desktop window
  opens at with `--editor`), the builder's popup geometry (`EDITOR_*`),
  `EMBEDDED`/`KEYBOARD_AVAILABLE`/`TWO_PLAYERS_AVAILABLE`/`ONLINE_AVAILABLE`/`TOUCH_STEER_RIGHT`
  (the `touch-steer-right` cargo feature: the stick on the right half, a recompile rather than a
  knob). Heavily commented inline - those comments are the source of truth. Gameplay/feel numbers do
  not belong here: add a `tunables!` row. `tank_art.rs` is generated beside it
  (`tools/spritegen/tankdesign/export.py`, never edited by hand): each chassis's lamps, gun muzzles,
  module muzzles and missile tubes in design pixels about the pivot.

## Simulation

- `simulation/` — the simulation layer. `mod.rs` owns `Game`: `init`, and `update` as a sequence of
  named phase methods (intro, timers, frogs, pickups, players, roll-in, enemies, waves, lasers,
  flames, `step_world`, ram, projectile hits, explosions, cleanup, round end) sharing a per-frame
  `Frame` (round RNG, terrain snapshot, deferred spawns/kills/blasts/events). No raylib dependency:
  a round needs only this module and an `Input` - `seats: [Intent; MAX_SEATS]`
  (`Input::single`/`two`/`seat`; the round reads the first `players.count()` seats, and
  `Game::seats` holds their tanks - see Seats below) plus the one-shot toggles; every driver
  passes `dt = PHYSICS_FIXED_DT`. **Event log**: phases push `Event`s (fired, hit, wreck, ram,
  shield, frog, pickups, round start/end, `Ricochet` - a shot bouncing off iron or a barrel, with
  the heading it flies on along, emitted where `resolve_projectiles` bounces it; `LaserBeam` - the
  beam's muzzle and stop point, since an instant hit leaves nothing in the world for a snapshot to
  carry and a replica draws the beam from it - one a leg, `leg`/`portal`, where portals bend it;
  `ShotTeleported` - a shot or beam through a portal; AI decisions only while `Game::trace_ai` is
  set) onto `Frame::events`, merged into `Game::events` and cleared at the top of `update` - no RNG,
  so tests assert on `game.events()`. A kill lays down fireball, screen flash (`flash_screen`,
  gapped by `blast_screen_flash_min_gap_seconds`), scorch, thrown decals, flattened grass and hashed
  cook-offs - no RNG. Each cause's cosmetic half is its own `pub(crate)` show - `Game::wreck_show`,
  `cookoff_show`, `blast_show` (props.rs), `missile_show` (missiles.rs), `props::tile_rubble` -
  returning a `Spectacle` (blast fx, scorches, decals, muzzle and impact flashes, shocks) that the
  phase stages into its `Frame` (`Frame::stage`) and `finish_frame` puts on screen through
  `Game::show`, the caps included (`Game::mark_caps`: `SCORCH_MAX`/`DECAL_MAX` on an arena, as many
  per standard field of area on a field map; `SHOCK_MAX`); a replica's spectacle comes through the
  same shows (`net::apply`), so the two cannot drift. `Shoved { seat, vx, vy }` is the velocity
  change the room put on a **client-owned** hull - a hit's knockback, a blast, a missile burst, a
  ram - for the owner to apply to the body it drives, since the next pose would otherwise erase it
  (`Frame::shoves` keeps an entry only for a seat owned this update, so a local round logs none);
  recoil of the shots the client launches itself is not echoed. Projectiles integrate inside the
  fixed-step loop and keep flying, damage-free, on the end screen. **The end screen** restarts the
  round after `restart_delay` unless `hold_end_screen` is set (a level's, docs/levels.md): then
  `restart_timer` counts down to zero and holds there, and the caller starts the next round -
  `Session::follow_countdown`, `init`, or the R key's `restart_pressed`; `round_stats()`
  (`RoundStats`: the round clock when `end_round` ran, enemy wrecks counted in `explosions` through
  `credit_wreck`, the band or every wave's total, and wrecks by seat through `Tank::last_hit_by`,
  which `Tank::credit` sets from a seat's shot, beam, flame, missile burst (`BlastParams::by`) or
  ram) is bookkeeping with no RNG. **`Game::accept_seat_pose(seat, SeatPose, reach_ticks)`** is
  stage 3's validator (docs/online-coop-prd.md §4.14): a client that owns its hull reports where it
  is every tick, and if the step is within the chassis's top speed over `reach_ticks` - what the
  mailbox read vouches for (`Mailbox::pose_reach_ticks`), never fewer than `POSE_REACH_TICKS` - plus
  `POSE_REACH_SLACK_PX`, inside the field, off a solid tile and out of deep water, the room places
  the seat's body **one tick back along the reported velocity** (so the coming update's solver step
  lands it on the pose rather than a tick past it) and records the update it owns (`seat_owned`
  holds that frame number - ownership lasts one update and lapses by itself, so a client that drops
  or stops sending poses is the room's to drive on the next tick); `drive_player` leaves an owned
  seat alone (the stick still fires - from the placed pose); a wreck or a seat in a gate lane owns
  nothing; `seat_pose` reads it. `Event::Placed` is the mirror of the room's answer.
  **`net::authority`** is the one rule every room applies it by - `take_pose` before the tick
  (applied / refused with a `Placed` / released), `moved_since` after it (`Placed` past `PLACED_PX`) -
  so the room server and the rig cannot drift. **`Game::tick_presentation(dt)`** is a frame's
  cosmetic half on its own, for a `Game` nothing ever `update`s - a client replica
  (docs/online-coop-prd.md §4.5) runs it on every rendered frame between snapshots: `tick_effects`,
  the round clock, the mission and `WAVE N` banner timers, the hull eases with the tread marks their
  displacement presses (`Tank::track_from`), grass, the tiles' burn flicker, the fires' fade, the
  wrecks' fade, the drums in the air and the flame jets rebuilt from every `flame_held` hull
  (`derive_flame_jets`, reach capped at the first solid tile as `resolve_flames` caps it - the wire
  carries the flag, not the jet). Every step is a function the phase that owns it calls too, so
  `update` keeps its own order and **a local round never calls it**. `Game::debug_overlays`
  (`Overlays`: `hitboxes`, `stats`, `nav_grid`, `ai`, `projectiles`, `engage`, `pickups`; the I-key
  preset cycle off -> `INSPECT` = the two tank layers -> `ALL`).
  - `weapons.rs` — `dispatch_fire`/`tick_queued_shots` shared verbatim by player and AI; the
    `Projectile` trait lets one hit loop serve shells, bullets and plasma. A laser is traced
    `laser_reach(field)` px: the fixed `LASER_MAX_RANGE` wherever that spans the field's diagonal,
    past the diagonal on a larger field (`net::round` traces a drawn beam the same), bent leg by leg
    through portals (`portals.rs`).
  - `portals.rs` (docs/teleporting.md, "Shots") — shells, bullets, plasma and laser beams through
    portals while `portal_shots` is on (missiles are left alone): a path passing within
    `portal_shot_radius` of an active anchor goes in at its point nearest the anchor (`shot_entry`;
    `resolve_projectiles` and `resolve_lasers` judge it only that far) and comes out at the same
    offset from another portal on the same heading (`exit_point`, `Game::shot_through`), the exit
    one round-RNG draw (`draw_shot_exit`) made only where a shot goes in, so a round with no shot
    through a portal replays byte for byte. `ShotPortals` (a component, attached only to a shot
    fired inside a swirl or on its first pass) counts passes against `portal_shot_max_passes` and
    names the portal it is `leaving`, which it cannot enter until it has left the swirl.
    `Event::ShotTeleported` (no id for a beam) is a small blue flare in `fx.rs`, no ripple. The AI
    does not know: an enemy lined up through a portal fires as through open ground.
  - `missiles.rs` — the seeker missiles' world half (`missile.rs` is the flight): `guide_missiles`
    (before `step_world`) locks a missile at the top of its climb onto the nearest opposing live
    tank within `missile_seek_range` (ties on slot; none = the launcher's aim point) and keeps its
    aim on that tank while it chases (an enemy's missile locks a seat only while its launcher stands
    inside that seat's sight box); `resolve_missiles` bursts the ones that landed (`missile_blast`:
    `explosion_hit` for the side opposing the shooter, a shove for everyone, opposing frogs,
    `damage_obstacle` for tiles). Missiles have **no hit test** - they fly over everything and only
    the blast touches anything. No RNG beyond the blast's damage rolls.
  - `grenades.rs` (docs/grenade-launcher.md) — the grenades' world half (`grenade.rs` is the ball):
    `roll_grenades` (after `step_world` and the ram, on the end screen too) rolls each among
    `Terrain::tile_boxes`/`edge_boxes` and every hull with a body (a live one at its body's
    velocity, a wreck still); `resolve_grenades` sets off the spent ones in id order through
    `side_blast` (missiles.rs: the side opposing the launcher hurt, everyone shoved, opposing frogs,
    tiles, crates) and `grenade_show` (a replica's off `Event::GrenadeBlast`: fireball, a
    `grenade_shock` ripple, the screen flash, scorch). A grenade moves no tank and a round without
    one draws exactly what it did; no RNG beyond the blast's damage rolls.
  - `sonic.rs` (docs/sonic-hammer.md) — the sonic hammer's world half (`sonic.rs` at the top is the
    cone, the wave and the drawing): `fire_sonic` from the dispatch arm, `resolve_sonic` casts the
    frame's presses into `Game::sonic_waves`, `tick_sonic_waves` (after `resolve_flames`, before
    `step_world`; on the end screen with `live` false) strikes what each wave's front reaches once -
    `knock` (an impulse and `Tank::skid`, the slide every later shove may use; `Event::Shoved::skid`
    for an owned hull, `Game::seat_knock` the pose validator's allowance - a `SeatKnock`, each pose
    the knock's speed past the reach and all of them the knock's slide, no more), damage to the
    opposing side only (`HitCause::Sonic`), glass shattered, grass flattened (`Game::grass_flat`,
    `grass::pin`, `Game::cover_cells` what conceals), drums thrown (`throw_drum`, `drum_landing`),
    frogs stunned, lanterns broken, grenades pushed - no RNG. `sonic_show` is the cosmetic half a
    replica's `SonicBlast` and a client's press take too; `hammer_senses` is what the AI is handed
    (`ai::HammerSense`; `closer_spots` the two closers' spots, square on the seat and out of each
    other's cones; `lands_in_trouble` hazards on the way, a lane or a tower's reach where the slide
    rests); `swap_spawn_special` the hashed spawn swap (`enemy_special_weapon_sonic_share`);
    `debug_spawn_pickup` the `spawn_pickup` tool. Tests: `sonic_tests.rs`.
  - `emp.rs` (docs/emp-burst.md) — the EMP burst's world half (`emp.rs` at the top is the pulse, the
    scoring and the drawing): `fire_emp` from the dispatch arm (the shooter's own special offline,
    `Tank::special_offline`), `resolve_emp` puts the frame's presses on `Game::emp_pulses` and, at
    night (`is_night`: night or storm), every lamp post out (`Game::lamps_out`, `lit_lamp_posts`),
    `tick_emp_pulses` runs each ring out (on the end screen with `live` false) and `strike_emp` is
    its one walk over the band the front swept - tanks of either side but the shooter's
    (`Tank::disable`: brain, lights, shield, wind-up and the special out), towers
    (`Tower::disable`), missiles (`Missile::kill`, landing as duds) - later weapons add their blocks
    after the missiles. No RNG. `emp_show` is the cosmetic half a replica's `EmpPulse` and a
    client's press take; `emp_senses` (`ai::EmpSense`) and `emp_dangers` (an armed seat, an ally's
    crackle, or its pulse held for allies - for `emp_ai_clear_patience_seconds` at most,
    `Ai::clear_waited`) what the AI is handed; `danger_route_cells` the route surcharge. A disabled
    enemy coasts on its last intent with its brain off (`enemy_phase`'s first branch, `Ai::down`,
    `reboot`), spots, relays and takes orders from nobody (`command::Busy`). Tests: `emp_tests.rs`.
  - `gauss.rs` (docs/gauss-rail.md) — the gauss rail's world half (`gauss.rs` at the top is the slug
    as drawn, the measures and the composers): **the charge-and-hold pattern** - `charge_trigger`
    steps a held trigger every tick (`Tank::step_charge`: started on a press with the gate open,
    held, full, overcharged, released at a stage, fizzled, vented; a room's count held to the
    client's by the hold report, `set_seat_hold`/`seat_hold_report` from `Mailbox::hold_ticks`
    through `authority::take_hold`), `fire_charge` queues a `PendingRail`, `resolve_rails` (after
    `resolve_lasers`) traces each leg through `Terrain::pierce_rewound` (every box in order to the
    first permanent tile or wall, iron cut overcharged, portals leg by leg), `pierce_hit` (fixed
    damage with keep factors, `DamageCause::Pierce`, `HitCause::Rail`), the recoil (`recoil_hull`
    through `sonic::knock_hull`, `Shoves::allow_knock`, an overcharge's spin);
    `rail_show`/`charge_end_show` the cosmetic halves a replica and a client's own release take;
    `gauss_senses` (`ai::GaussSense`; only for the tanks that think this tick - `field::thinks`,
    `field::mind`'s rule asked ahead - and only the lanes a counting seat or the quarry could be on,
    `hits::SlugBand`) and `rail_lanes`/`rail_dangers` (`ai::DangerShape::Lane`, with the edge hold)
    what the AI is handed; `predict_seat_with` runs the same charge in a client's sandbox. No RNG of
    its own. Tests: `gauss_tests.rs`.
  - `fpv.rs` (docs/fpv-swarm.md) — the FPV swarm's world half (`fpv.rs` at the top is the drone, its
    flight and the composers; `air.rs` the air targets): `fire_fpv` queues a launch from the halo's
    top slot (the halo is the stock, drawn, no body), `launch_drones` locks it (`pick_lock`: a
    seat's press the nearest enemy in its sight box not under a tree's crown, an enemy's what its
    rule asked, `Tank::fpv_want`), `guide_drones` keeps or loses the lock (wreck, crown, portal),
    `advance_drones` flies them in the fixed step (gusts carry them), `resolve_drones` bursts a
    dive's end as a small side blast (`side_blast_sparing`: hulls and frogs under a crown spared; in
    a crown the tree alone) or crashes a downed one a dud; `drone_show` the cosmetic half a replica
    takes. **Air targets** (`air.rs`, `Game::air_targets`/`strike_air`, the one setter): only
    bullets strike the air (`Terrain::sweep_air`, lag-compensated through `HitBoxFrame::air`), sides
    spare their own but the EMP's ring and the rail's lane, towers and enemies engage a seat's drone
    only from inside that seat's sight box, the tesla arcs them on its own clock, the gun tower
    turns on them first. `fpv_senses` (`ai::FpvSense`: launch from cover, by walk, with a patience)
    and `air_threats` (`ai::AirThreat`: flak, a tree's crown, a break) what the AI is handed. No RNG
    of its own. Tests: `fpv_tests.rs`.
  - `rod.rs` (docs/rod-from-god.md) — the rod from god's world half (`rod.rs` at the top is the
    reticle, the measures and the composers; `zone.rs` the zones): a charge whose stick aims
    (`Stick::Aim`: the hull stands, `Tank::step_trigger` walks the reticle a cell at a time, a
    room's report - `Mailbox::reticle` through `authority::take_reticle` - putting it where the
    client has it), `fire_rod` on a full release off the hull's own cell (on it, the cancel),
    `place_calls` puts the call on the field as a `Zone` (`Event::RodCalled`), `resolve_zones` lands
    it `rod_countdown_seconds` on (`rod_impact`: every hull with any part in the circle crushed
    whatever its shield, frogs killed, drones downed - `AirStrike::Rod` -, breakable tiles crushed
    (`DamageCause::Crush`), hulls out to the shove radius knocked (`knock_from`, the hammer's
    knock), grenades, lanterns, oil, crates, grass; a crater on dry ground - `Craters`, a pit in
    `Footing`, filled to a ford under rain, `WaterLayout::fill` -; a volcano struck set off by a
    shift of its cycle in ticks, `Volcano::set_off_shift`, on the wire). Every zone's `Danger`
    (owner `None`) is every enemy's to keep out of, the router surcharges it, and a seat in one is
    herded (`EngageCtx::herd`). `rod_senses` (`ai::RodSense`: a seat standing still from
    `SeatStill`, a slow one led, a player tower, the frog; never on an ally; the seat it keeps its
    distance from) is what the AI is handed - a rod tank fires no shells and stands off a band out
    of its own circle. No RNG of its own. Tests: `rod_tests.rs`.
  - `well.rs` (docs/gravity-well.md) — the gravity well's world half (`well.rs` at the top is the
    orb, the field, what a well holds and the composers): `fire_well` launches a slow orb off the
    gun line (`place_orbs`, its id from the projectile counter), a press while it flies anchors it
    whatever the reload (`take_anchor_press`, in `drive_player` and `enemy_trigger`), as does what
    its sweep meets or its range (`resolve_orbs`); the well is a `Zone` (`ZoneKind::Well`, forming
    then pulling, `until` the stage's end) that `well_phase` turns on the clock - lifting the drums
    in reach into `held_drums` at the pull's start, pulling frogs and crates (`Pickup::drift`),
    burning held fuses, swirling tread marks and bowing grass (`drain_marks`, `lean_grass`, a
    replica's too in `tick_presentation`) - and collapses (`collapse_well`: held drums off together
    where they circle, every hull flung by `knock_from`, the side opposing its owner hurt, frogs
    hopped out, grenades thrown out in a ring, what flies turned out, crates slid out). The pull
    reaches hulls through the drive (`Footing::pulled`: the current along the tracks, the side pull
    across them against the grip), shots through `Projectile::bend` and a swallow at the core in
    `resolve_projectiles`, missiles and drones through `pull_air`, grenades through
    `Grenade::roll_pulled` and a capture onto the ring; a blast over a held drum sets it off
    (`chain_held_drums`); an EMP's ring fizzles an orb and collapses a well early.
    `well_senses`/`pull_senses` are the AI's (`ai::WellSense`: clump, trouble, guard, shield, never
    dragging more allies than seats; `ai::PullSense`: brace or drive across). No RNG of its own.
    Tests: `well_tests.rs`.
  - `hits.rs` — **the only projectile hit test**: `Terrain` (per-frame snapshot of seam-closed
    boxes, frogs, walls) and `Terrain::sweep_rewound`, a nearest-entry segment-vs-box sweep over the
    whole frame's movement (wrecks skipped). A player's shot is tested against an enemy's hull and
    turret grown by `player_shot_hit_pad_px` (read into the snapshot at `build`), while enemy shots,
    the seats, tiles and walls keep the exact boxes. A tower's shot flies over the tiles its aim
    sees past (sandbags and fences, `Material::blocks_sight`). Projectiles have no physics body;
    rapier is never consulted for hits. A shot is swept only up to the portal it goes into
    (`portals.rs`). `Terrain::line_of_sight` is the AI's fire gate. **Lag compensation**
    (docs/online-coop-prd.md §4.16, favour the shooter): `HitBoxHistory` is a ring of the last
    `REWIND_MAX_TICKS` (15, 250 ms) ticks of every live enemy's hull and turret boxes and every
    frog, recorded at the end of each simulated tick and cleared by `init`; a seat's shot carries
    the ticks its client was drawing behind the room (`weapons::Rewind`, a component attached only
    when non-zero, from `Game::seat_rewind` = the room's frame less
    `IntentMsg::view_tick`/`view_frac`, set through `Game::set_seat_view`), and `sweep_rewound`
    moves only the enemy and frog boxes back to that tick - who is a candidate is decided in the
    present, a tank with no entry is tested where it stands, and seats, tiles and walls stay
    current. The laser takes the firing seat's rewind; a shield deflection zeroes a shot's. A round
    with no seat views - every local round - rewinds nothing, and recording draws no RNG.
  - `combat.rs` — `apply_hit` (damage, knockback, frog hop, `Ai::notify_hit`), `ram` (impact speed
    from the bodies' *real* velocities; enemy pairs walked sorted i<j with a distance cull so the
    RNG draw order is fixed), explosions.
  - `engage.rs` — the engagement-slot ring (`EngageRing`; a seat ring's firing slots stand half a
    cell inside the seat's sight box, `EngageCtx::sight_box` - north and south at 224 px): world
    access as closures, reachability from one `Grid::components` flood fill per frame;
    `EngageReport` feeds the overlay, the snapshot and the AI event diff (docs/dev-server-design.md
    "Debugging enemy clustering").
  - `debug.rs` — the tooling surface the dev server calls between frames: `debug_snapshot`,
    teleport, `debug_set_tank`, `debug_kill` (queued into `Frame::kills` so it explodes like a real
    kill), `debug_spawn_enemy` (the one mutator that draws round RNG), `nav_grid_ascii`,
    `field_dump` (the flow field toward a player or the frog as arrows and costs),
    `debug_track_rows`, `clusters`/`CLUSTER_RADIUS_PX` (shared with the probe).
  - `props.rs` (docs/sandbags-barrels-fences.md, docs/barrel-explosion-variety.md) —
    `damage_obstacle` is the one place a tile loses health (fence one-shot odds, barrels on a `Fuse`
    scaled by the drum kind, direct hit/ram pops at once); `apply_blast` (a `PendingBlast`: falloff
    damage and knockback to both sides, frogs and tiles, sooted wall faces, oil pools, thrown and
    re-thrown decals, flattened grass, burnt-in tracks, cook-offs); `tick_fires` (`GroundFire`:
    pools and oil-trail cells burn, spread, damage hulls, fuse drums; burning cells are blocked in
    the nav grid; `fade_fires` is the burn-down alone and `fire_burnt_out` what a spent cell leaves -
    a burn scar (`Scorch::burn`) and the oil gone - once per cell); `tick_fuses` (a fuel drum lit by
    *another* blast launches as a `FlyingDrum` and blasts where it lands;
    `drum_in_flight`/`age_flying_drums` are the arc, staged from a launch or from the event);
    `tick_burns` (wood that finishes charring dies through `obstacle_died`, the funnel every tile
    death passes); `ram_props`; `barrels`/`debug_detonate` (a tool's click: queued like
    `debug_kill`, applied as a fatal direct hit at the top of the next frame). Pass-over and
    deflection rolls live in `resolve_projectiles`. Every cosmetic choice is hashed; only damage
    rolls and chance knobs strictly between 0 and 1 draw RNG, so prop-free maps replay unchanged.
    Tests: `props_tests.rs` (statistical over seeds).
  - `flame.rs` (docs/flamethrower-prd.md) — a `Cone` per held frame from the gun line's muzzle
    (`Tank::gun_line_muzzle`; the jet is *drawn* from the flamethrower module's nozzle,
    `FlameJet::drawn`), reach capped at the first solid tile on the centre line, rate damage with
    **no RNG in the whole phase**, afterburn dealt only on frames the stream is off the hull.
    Exposure: ground cells (`Game::heat`, a `BTreeMap`) and tiles gain heat under the stream and
    ignite at `flame_ignite_seconds` (`Event::Ignited`); the shooter's own cells never heat. The
    fuel pickup is player-only. Tests: `flame_tests.rs`.
  - `towers.rs` (docs/defence-towers-prd.md) — the defence towers' world half (`tower.rs` is the
    data and the drawing): `build_towers` makes one `Tower` per tower tile at `init` (a turning one
    aimed at the field's middle), `tower_phase` (after `wave_phase`) runs each in cell order -
    `tower_upkeep` (it burns below `tower_burn_below` or under the flamethrower's heat, at
    `tower_burning_fire_factor`), then the tesla (charges on an opposing hull in reach, concealed or
    not, strikes at full charge through `apply_hit` and chains `tesla_chain_jumps`), the gun tower
    (turns at its rate, leads, bursts of minigun bullets owned by `Owner::Tower` inside its fire
    cone, holds while a friend is in the line; the hit test skips its own side) or the bio slush
    (lobs a `Glob` between its minimum and maximum range, scatter hashed). `resolve_globs` splashes
    the landed ones (a coat - `Tank::slime_timer`, `slime_pace` - and ooze on dry cells),
    `tick_ooze` dries the puddles in `Game::ooze`, coats any hull over one and washes a coat off in
    water; `tower_died` (from `obstacle_died`) leaves a `TowerRuin` and discharges, cooks off or
    spills. Enemies route round the player's towers' reach (`player_tower_reach`,
    `route_tower_cost`) and the ooze (`ooze_route_cells`) and fire back at a tower that hurt them
    (`Ai::notify_tower_hit`, the `grudge` tier). The tower pack (`PickupKind::TowerPack`) mends the
    collector's side. An enemy tower picks a seat only from inside that seat's sight box
    (`box_allows`; at the defaults only the gun tower reaches past it). **The bio slush draws no
    RNG**; the gun tower draws its spread and its bullets' damage, the tesla its strike's damage
    roll, and a map without towers runs none of this. Tests: `tower_tests.rs`.
  - `volcano.rs` (docs/volcano.md) — the volcano's and the lava's world half (`volcano.rs`/`lava.rs`
    at the top are the data and the drawing): `build_volcanoes` at `init` (one `volcano::Volcano`
    per crater cell, its gullies cut toward the lava leaving it), `volcano_phase` after
    `tower_phase` (an erupting volcano throws the bombs whose launch falls this frame -
    `volcano_bomb_aimed_share` at a seat in range, landing inside its sight box, the rest round the
    crater, every target hashed from the eruption and the bomb's number - as
    `Event::LavaBombLaunched`, and its surge sets alight the flammable tiles on the banks, hashed),
    `tick_lava_bombs` in both branches beside `tick_launches` (a landed bomb bursts through the
    drums' blast path as `Drum::Lava`, splashing a pool of burning lava - a `GroundFire` with `lava`
    set - and breaking a lantern near its middle), `lava_phase` beside `tick_fires` (heat over
    `heat_hurt_from` burns a hull by how far it stands over it, the lava itself at
    `lava_damage_per_second` with afterburn; never through a heat shield), the lanterns
    (`Intent::lamp`'s edge sets one down, `lamps_per_seat` a seat in a dark round,
    `Event::LanternSet`), `tick_nightfall` (the map's `nightfall`: the sky in force turns to night,
    the map's key kept; `nightfall_mix` eases the light before it) and `eruption_show` (the rumble's
    tremor, the eruption's shock, flash and screen flash, once per eruption, from the clock - a
    replica's too). A ford is `Footing` (`lava_speed_factor`, `lava_grip_factor`), deep lava a
    seam-closed static collider and a nav-grid wall that shots cross, a ford weighed
    `lava_ford_path_cost` by the router; nothing spawns on lava and the pose validator refuses a
    hull in deep lava. The cycle is a pure function of the round clock, so a replica rumbles on the
    room's tick; no RNG here beyond the blasts' damage rolls, and a map with neither a volcano nor
    lava runs none of it. Knobs: the `volcano` tuning group. Tests: the lava cases in
    `mechanics_tests`, `simulation::volcano::tests`.
  - `crates.rs` (docs/CRATES_SPEC.md) — breakable crates, behind `crate_breakable` (off by default,
    read once by `init` into `Game::crates_breakable`, so a test turns it on for its own round):
    what reaches the ground breaks a crate - `blast_crates` from every drum, missile, wreck and
    crate blast (mid damage times falloff, no roll), and fire (`tick_crates`: `crate_ignite_seconds`
    of `Game::heat` or a burning cell lights it, it falls in `crate_burn_seconds` later) - while
    shells and bullets still fly over it. Breaks join the explosions worklist
    (`Frame::crate_breaks`); ordnance and energy cook off (`crate_cookoff`, an oil drum's blast
    scaled by `crate_cookoff_scale`, the only RNG), the rest spill (`Pickup::loose`, takeable for
    `crate_spill_seconds`). `Event::CrateBroken`; `crate_cookoff_show` is the replica's show. Tests:
    `crate_tests.rs`.
  - `waves.rs` (docs/maps-to-levels.md) — wave pacing, the `WAVE N` banner (`tick_wave_banner`
    counts the breather down; it travels in `RoundState` so a replica holds the banner up for as
    long), wreck despawn (`fade_wrecks` arms each new wreck's `despawn_timer` and runs it down - the
    fade is all a replica needs, since the removal here is the server's), the monotonic owner-slot
    counter (`wave_max_alive` caps *live* enemies). `pick_gate` is the one lane chooser both
    roll-ins ask: a free gate, `Busy` (every lane taken, retry next frame - answered before the
    lanes' preferences and walks are worked out, since each keeps some of the lanes it is given) or
    `None` (the map has no gate, so the wave's tank falls back to the band sampler). A rolling-in
    tank spawns outside the field at a `battlefield::Gate` with no body and **no `Ai` until it
    arrives**, so every `.with::<&Ai>()` query excludes it for free. **A wrecked seat comes back
    with the next wave** (docs/online-coop-prd.md §4.11, decision 7): `call_wave` queues every seat
    that is a wreck, and the queue is drained at the head of the wave's own roll-ins, the seat's
    tank reset to a fresh one of its chassis at a gate and driven in kinematically - no `Ai` on
    arrival, the stick again. A round that seats one queues nothing (its one wreck is every seat
    wrecked, and `check_round_end` has already lost the round) and a band round calls no wave, so
    both replay byte for byte. While a seat is in the lane it is `seats_on_field()`'s `None`:
    outside the field, not shot at, blasted, rammed, targeted or routed to, and not counted by
    `waves_finished`. On a field map `pick_gate` keeps the lanes `field::prefer_gates` leaves, a
    seat coming back takes `pick_return_gate`, every tank that rolls in arrives `called`, the
    breather is the director's (`director.rs`) and `reroll_stragglers` rolls a lost wave tank in
    again through a nearer gate (`field.rs`).
  - `field.rs` (docs/large-maps-follow-camera.md §5, §12) — the enemies' rules on a **field map**
    (`Game::field_map`: `MapFile::class()` is `Field`, read once at `init`); an arena runs none of
    it and replays as before. **Chained alerts** (`Game::field_alerts`, run by `enemy_phase` in
    place of the shared alert): each enemy carries its own (`ai::FieldMind`), a spotter within
    `Game::enemy_sight` of an unconcealed seat takes the nearest, and every group of live enemies
    within `enemy_alert_chain_px` of one another (a union-find in slot order) shares the best any
    member holds - freshest, then nearest sighting, then lower slot; no line of sight, like the
    shared alert. **The leash**: home is where an enemy first stood on the field; with no alert,
    call or hit it wanders and seeks pickups within `enemy_leash_px` and turns back past it
    (`ai::Brain::home_leash`/`seek`, `act_patrol`). **A wave is called** (`FieldMind::called`, set
    on arrival): it routes at the nearest live seat (`call_target`) until one is within sight range
    or it is hit - the call is never an alert, so it does not travel the chain. **Far enemies think
    less** (`mind`): farther than `enemy_far_px` from every live seat and the players' frog, a tank
    `Think`s when `frame + slot` falls on `enemy_far_think_ticks`, its timers covering the skipped
    ticks (`think_debt`), and `Coast`s on its last intent, trigger released, in between; one nothing
    has woken `Sleep`s - no think, no route. **Spawns and gates by walk, out of sight**
    (`spawn_cells`/`pick_spawn`, `prefer_gates`): never inside a seat's sight box
    (`ai::in_sight_box`), measured by path from the nearest seat (`Grid::walk_costs`) at
    `enemy_speed`; a band draws `field_spawn_spread_candidates` cells and keeps one whose walk is
    within `field_walk_slack_seconds` of `field_walk_seconds`, then the one farthest from the tanks
    already down (`SpawnPool`: the cells filtered once, each enemy's neighbourhood dropped as it
    goes down - the same draws as filtering them all again); a wave takes the gates out of sight
    and, where the map has any, about the walk; a wrecked seat comes back through the free gate
    nearest the living seats (`waves.rs`'s `pick_return_gate`, under `pick_gate`'s `Busy`/`None`
    contract). No RNG beyond the spawn draws; ties on slot or cell order.
    `TankSnapshot::asleep`/`leashed` tell the probe which holds are deliberate. **Stragglers**
    (`Game::reroll_stragglers`, once a second, `field_reroll`): a wave tank (`FieldMind::wave`) that
    has gone `field_reroll_after_seconds` with no live seat or the frog in its sight (`lost`), is
    farther by walk than any gate is paced for (`far_by_walk` of `WalkCosts::from_hull`) and stands
    past every outline a `view_local_max_cells` view can take round every seat's sight box
    (`beyond_every_screen`) - never a guard keeping its frog or a burning hull - is taken off and
    rolled in again through a free lane `pick_gate` would give a wave, outside every sight box and
    shorter by walk (`reroll_gates`), keeping its slot, role, damage and weapons (`Rejoin` through
    its `RollIn`) and arriving called; `Event::Rerolled` is not sent, and the lane is drawn from the
    round's RNG like a wave's, so a round with no straggler replays byte for byte. Knobs: the
    `field` tuning group. Tests: the field cases at the end of `mechanics_tests`.
  - `director.rs` (docs/large-maps-patterns.md, "Pacing director") — a field map's wave breather:
    each seat's intensity (0-1) jumps with what its tank loses (health and shield,
    `director_hurt_full` of a pool the whole way; the players' frog's losses raise every seat), is
    held up by live enemies in its sight box (`director_crowd_full`) and falls back over
    `director_fall_seconds`; the team is its most pressed seat on the field. `wave_phase` takes the
    breather from it (`breather_length`, then `pace_breather` each tick): held at `director_peak`,
    never ended before the `director_relax_seconds` a peak owes, `director_calm_rate` times faster
    at or under `director_calm` with nothing owed, within
    `director_breather_min_seconds`/`director_breather_max_seconds`. No RNG; arenas, band rounds and
    `director_enabled` off keep `wave_gap_seconds`. The time left travels in
    `RoundState::next_wave`, so a replica's banner holds as long as the room's (a rig test);
    `Game::pacing()` is `status.pacing`.
  - `training.rs` (docs/training-stage.md) — a training map's rules: the map's `[[training.beat]]`
    script (`crate::training`) run one beat at a time by `training_phase` (after the frame's blasts
    and wreck removals, before the cleanup that sweeps the doors it opens). A beat starts what it
    names (`shoot_frog`: one heavy enemy shell from inside an edge at the frog; `drop`: air-dropped
    crates; `roll_in`: tanks through `pick_gate`'s lane, `Game::training_roll_in`, arriving with the
    script's role) and is done once every trigger it sets holds (`flags`, `collect`, `destroyed` -
    any of its cells -, `destroyed_all` - every one -, `past_col`, `frog_full`, `wrecks`, and no
    roll-in still to come); then every `Material::Door` whose `variant` is that beat (1-based) is
    marked destroyed - no rubble, no `ObstacleDestroyed`, `Event::DoorOpened` - and the next beat
    begins. Never lost: `check_round_end` defers to the run, a wrecked seat comes back after
    `training_respawn_seconds` in the last door opened (`TankEntered`), a fallen frog gets up after
    `training_frog_revive_seconds` (`FrogRevived`) and its beat starts again. Nothing a player does
    stalls a beat: an enemy wreck fades off after `training_wreck_seconds` (`fade_training_wrecks`;
    a band round keeps its wrecks, and one in a gate's lane would keep the next beat's tank out), a
    roll-in with no free lane for `training_lane_wait_seconds` drops in out of sight
    (`spawn_in_band`), `shoot_frog` fires once the frog stands on its beat's cell and again every
    `training_frog_shot_retry_seconds` until it has hurt the frog, and a beat started again takes
    back its tanks and crates. No RNG of its own; a map without a script runs none of it.
    `BeatDone`/`DoorOpened`/`FlagTaken`/`FrogRevived` are `NOT_SENT` (a training map is never
    `hostable`, and `map::hostable_maps` - what the lobby's stepper walks - leaves it out).
    `open_every_door` is what the linter checks a course on. The frog walks to each beat's `frog`
    cell a hop at a time (`walk_frog`, routed on `NavCache::layer` - the tiles with no frog in them -
    so it waits behind a shut door) and does not shy from tanks while it walks (`frog_walking`); a
    script that sets `start_shells` holds the seats' shell refill until one opens an ammo crate
    (`Run::shells_held`). `status.training` is `training_status`. A beat's `ai = "dummy"` tank is a
    hunter with `Ai::frog_only`, which `Brain::may_fire_at_seat` reads: it never fires at a seat. A
    round with a script says `TRAINING`, `FOLLOW THE FROG!` and `TRAINING DONE!` whatever its
    mission. It seats one: `init` sets a couch's count aside (`Game::couch_players`) on a map with a
    script and puts it back on the next without one, and the players button and dialog are not
    offered there. Tests: `simulation::training::tests`.
  - `nav.rs` — the nav grid: `Game::nav_grid` and `route_grid` build it from scratch (the linter,
    mid-frame readers - wave gates, stragglers, band spawns -, the dev server's tools, tests);
    `NavCache` is the grid the round keeps (`refresh_nav` at the top of `update`). Its layer (the
    boundary, seam-closed tiles, deep water) is rebuilt when the world's tiles in order, the field
    size or the portal and ford knobs change; its base (the layer plus frogs and fires,
    `with_portals`, `weigh`, `label`) when the occupancy differs (`Grid::same_occupancy`), and
    `update` prices a `without_fields` copy. Exact because `Grid::open` plus `block` equals `build`,
    held by `nav::tests` (`Grid::same_as`, `Game::scratch_nav`); `init` clears it. A kept `Grid`
    holds its fields' searches in `RefCell`s, so `Game` is `Send` but not `Sync`.
  - `comms.rs`/`command.rs` (docs/enemy-command-and-control-prd.md) — the one layer that looks at
    several enemies at once. `comms.rs` is vocabulary (`Signal`, `Order`, `Blackboard`, `BTreeMap`s
    by owner slot). `Commander` is shaped like `engage.rs`: no
    `World`/`Physics`/`Frame`/`Tank`/`Ai`, closures and pre-sorted input, **draws no RNG ever**
    (ties break on slot) and **speaks `Intent`, never `Ai`** (writing headings/timers would bypass
    the hysteresis). `enemy_phase` is collect (perceive, `think`, aim, `dispatch_fire` - firing
    stays here because it draws RNG) → command → apply (`drive_tank_with`, replaying the captured
    order because a later query has no guaranteed order). `c2_enabled` defaults to false; off, the
    game is byte-identical to pre-C2. Producers: `clear_rings` (an EMP tank's ring cleared by
    `Order::Nudge` of the allies not already backing out of it, `UnitView::dodging`), then
    `deconflict`; a `UnitView::busy` unit (`Busy::Disabled`, an EMP's outage) is deaf - never
    ordered, never giving way.
- `battlefield.rs` — static terrain *placement*: boundary walls, wave gates (`gate_candidates`,
  `gates_from_cells`), `spawn_from_map`, enemy placement (`sample_clear_position` returns `None` on
  the attempt cap - snap to `Grid::nearest_open`, never use a rejected sample), `enemy_spawn_legal`
  (the one legality predicate, shared with `maplint`, and the clearance every seat keeps from the
  enemies), `relocate_unusable_spawns`, `max_tank_clearance_half_extent` (a *box* half-extent shared
  by placement, spawn legality and the nav grid - not `Tank::avoidance_radius`'s circle) and
  `boundary_lane_inset`. **No procedural fallback battlefield**: a map (`-m`, default
  `maps/default.toml`) is the only terrain source.
- `physics.rs` (docs/physics-engine-design.md) — the rapier wrapper: solid bodies only, no sensors
  or collision groups. `settle_wreck` is the one place damping is set, on the frame a tank becomes a
  wreck. Rapier's lengths are pixels (`length_unit` 1, so its own look-ahead is 0.02 px and it
  pushes an overlap out at 3 px/s); the two limits that matter in a pixel world are set here: the
  speed cap (`PHYSICS_MAX_SPEED`, one cell a step, above every knock and pull) and every hull's
  one-step look-ahead (`HULL_LOOK_AHEAD`, rapier's soft CCD, set at spawn for the body's whole
  life), so a hull driven, rammed, blasted, knocked or pulled stops flush against what it meets
  (docs/physics-engine-design.md "Scale"). `quarantined` reports rapier's NaN quarantine as
  `Event::PhysicsQuarantine`.

## AI

- `ai.rs`, `bt.rs`, `pathfind.rs` — AI reasons over snapshots, never the world. `Ai::steer` routes
  through `Grid`; on top: an obstacle-ahead override of the heading commitment, a stuck escape on
  *displacement projected onto the commanded heading* (never a physics velocity - the solver reports
  motion for a jammed tank), breach (shoot a destructible tile after `enemy_breach_after_seconds`,
  never iron), and the personal-space brake (`crowded_ahead`/`yield_timer`: a brake, not a sidestep;
  `enemy_yield_seconds` is load-bearing because the stuck escape only counts commanded movement).
  **The sight box** (docs/large-maps-follow-camera.md §5): an enemy fires at a seat only while its
  centre stands inside that seat's box, `sight_box_half_cols` x `sight_box_half_rows` cells (11.5 x
  7.5, +-368 x +-240 px, `Tuning::sight_box_half_px`), with `ai::in_sight_box` the one test
  (`in_sight_box_of` with the half extents read once, for a loop) - `Brain::may_fire_at_seat` gates
  `act_attack`'s alignment (a tank lined up outside the box repositions and closes in) and the
  hunter's snipe; shots at a frog, a tower or a tile are not bound. `Ai::shot_at_seat` (inspection
  only) names the seat a trigger pull was aimed at. Roles Player/Hunter/Guard are rolled last
  (`roll_role`). **Dangers** (`Danger`: a `DangerShape` and an optional owner; docs/emp-burst.md
  §3.3): the places an enemy keeps out of, built by `enemy_phase` only while a weapon makes one -
  the `dodge` tier walks out through steering (`Brain::steer_out`: on foot, avoidance and the stuck
  escape, the turn from a heading leading away put on first) to the exit it chose (`Ai::dodge_exit`,
  a point kept while out of every danger and walkable), every steering target and every pickup
  headed for is kept outside (`out_of_danger`, `seek`, `nearest_pickup`; a seat at its own danger's
  middle is waited on from a `post`, `Ai::kept_out` a probe hold) and the seats' dangers are priced
  on the route grid (`enemy_danger_route_cost`). `AiSnapshot` is the memory as plain values. `bt.rs`
  is a generic tree whose `tick_traced` names the last action. `pathfind.rs` is one `Grid` with two
  routers behind `next_step`: A* per query, and a **flow field** per shared target (`add_field`: a
  Dijkstra outward from the goal, every enemy then reads its cheapest neighbour - kept open in a
  `RefCell` and worked out only as far as the frame reads it: `descend` until the frontier's lowest
  cost reaches the cheapest step known, `to_goal` until the cell is settled, so every answer is the
  whole table's while a frame pays only for the cells between each goal and its readers;
  `weigh`/`surcharge`/`with_portals` come before the first field (debug-asserted), a read the goal
  cannot reach runs the search to its end, and the tests compare against
  `Grid::settle_fields`/`Game::whole_fields`). The frame's grid is the round's kept nav grid
  (`simulation/nav.rs`: occupancy and `label` - O(1) `connected`, what `wander` and the engage ring
  ask - rebuilt only where the terrain changed), priced by `Game::route_grid_on` with the tactical
  `surcharge`s - the cells down each player's barrel (`route_lane_cost`/`route_lane_cells`) and the
  cell each enemy stands in (`route_crowd_cost`; 0 turns either off) - then a field per live player
  and for the player's frog; `route_grid` builds the same from scratch. Engage slots, waypoints and
  pickups still go through A*, on per-node tables the thread keeps between searches (`SearchTables`,
  stamped per search), so a query costs the cells it reaches rather than the whole grid. **Lanes**
  (field maps only, `ai_lane_turns`; docs/large-maps-follow-camera.md §12): a hull slides `v^2/2a`
  past a turn (`tank_turn_grip_force` over its mass), so the switch margin alone
  (`ai_dir_switch_margin_px`, measured to the next cell's centre) leaves it on the far edge of the
  row it turned into, where no later step that way beats the margin. `Ai::lane_turn` reads
  `ROUTE_AHEAD_CELLS` of the route (`Grid::route_ahead`: a flow field's read straightened along the
  heading where that costs nothing, a search's first step only) and turns where the rest point
  (position plus slide, from `Ai::motion`) reaches the centre line of the cell the route turns in -
  within half a cell (or the first think in that cell), after the full hold, never across a blocked
  corner - on a flow field's route always, on a searched one only where the margin never could
  (`margin_never_turns`: the hull rides the edge of its lane on the turn's side, or is so near it
  that the margin's window - twice what its error across beats `ai_dir_switch_margin_px` by - is no
  wider than its stride, `Ai::motion` along the heading over the think's seconds, so no think need
  land in it); `Ai::walks_into_wall` judges the obstacle-ahead override from where the slide across
  the heading leaves the hull. Arenas replay byte for byte and `ai_lane_turns` off is the old
  steering everywhere; `just probe-defend` (the probe's `--scenario defend`) is the stranded-walk
  check. Round 0x3fd of the study map's defence was that stride case: a hunter 20.2 px across from
  its turnings, a 0.4 px window against a 2.4 px stride. `Grid::walk_costs` is the flow field's
  Dijkstra from several goals at once (`WalkCosts::at`: nav steps to the nearest, through the portal
  hub; `from_hull` reads a hull in a blocked cell - pressed against a wall - from its open
  neighbours) - what a field map's spawns and gates are measured by. Ties break on cell index, no
  RNG. Both routers walk the optional portal **hub** (`Grid::with_portals`: footprint cell -> hub at
  `portal_hop_cost`, hub -> any other portal's footprint free; exact for two portals, optimistic for
  more, components unioned; the field's Dijkstra relaxes the entrances through it and `descend`
  offers the exits beside the neighbours; a step onto a footprint is handed out as the portal's
  centre; `next_step_walking` leaves the hub out, which is how a tank on its portal cooldown routes -
  docs/teleporting.md). **`PATHFIND_CELL_SIZE` must equal `OBSTACLE_GRID_SIZE`** (occupancy at cell
  centres - another pitch phases against the map grid), the margin is a box half-extent, boundary
  cells within the margin are blocked, so routability is corridor width alone (2 cells route, 1 does
  not; pinned by `maplint`'s corridor test and `maps/test/corridors/`).

## World entities

- `tank.rs` — the tank entity. `TankKind` is the 12 chassis in sheet-row order, the single list
  `--tank`, a map's `tank` and the `player_tank` knob spell from. `Tank::owner: Owner`
  (`Player(u8)`/`Enemy(slot)`); `sheet_row` picks the team block (enemy, then players 1-4 -
  `TANK_ROWS_PER_TEAM`, `TANK_TEAM_BLOCKS`; a seat past the fourth borrows P1's),
  `TEAM_COLORS`/`team_color` (one per seat, deliberately off the palette - docs/PALETTE.md,
  docs/player-indicator-improvements.md). **The sprite is layers about one pivot**
  (docs/SPRITESHEET_SPEC.md): 40 px cells drawn at `scale` (`sprite_size`) while the gameplay frame
  stays 32 (`size`), the hull (`hull_col`: the damage tier - `TANK_DAMAGE_TIERS` 25/50/75,
  `damage_tier` - times the track frame, or the rolled wreck), the turret (`turret_col`: tier times
  the recoil pose `kick`/`tick_recoil` walk; the broken one on a wreck, thrown clear on a blown one)
  and a module per special weapon carried (`module_cols`), all drawn by
  `draw_tank`/`draw_tank_shadow` and lit at night by `draw_tank_glow`;
  `turret_point`/`gun_line_muzzle` put `tank_art`'s anchors in the world. Ground rings: the health
  gauge (`RingStyle::Gauge`, `HealthRamp::Team` per seat; enemies show theirs after a hit or under a
  threshold) with, in play and online, what this window's seats' triggers have left as ammo pips on
  its lower arc - the special carried against its carry limit, else shells against the magazine
  (`ActiveWeapon::full_load`), the vitals' own readout (`hud::WeaponSlot`) -
  (`ammo_pips`/`draw_ammo_pips`, `AMMO_PIPS` 10, drawn in `paint_field_glowing`), the shield ring,
  the locate ripple and `P1`..`P8` labels, all centred on the hull's `position` (no lag behind it). `Tank::hit_by_seat` is the per-frame twin of
  `last_hit_by` (the probe's fire tally). **One special weapon at a time** (`Tank::take_weapon`,
  `SPECIAL_WEAPONS`): a weapon crate replaces the special carried (its ammo lost) and stacks a
  crate's worth (`*_per_pickup`) on the same one, up to its carry limit (`*_max`, never under a
  crate - `ActiveWeapon::crate_load`); `active_weapon` is that special while it has ammo, else
  shells, whose magazine is its own (recharge, the Ammo crate). Seats and enemies stack alike; the
  pickup phase re-checks `wants_pickup` crate by crate, so a hull over two crates takes the second
  only while it still wants it. Tests: `health_ring_tests`, `weapon_inventory_tests`,
  `chassis_tests`.
- `frog.rs` (docs/FROG_SPEC.md) — `Frog`/`Side`; one struct serves the player's frog and the Hunt
  mission's enemy frog. Art faces right and is mirrored. **A frog only bites the other side**; the
  evasive hop shies from any tank (`combat::frog_hop_target`, a box test via `Terrain::frog_fits`).
  `damage`/`heal` are the only health paths and no-op once dead; healing comes only from the frog
  health pack.
- `pickup.rs` — kinds Health, Ammo, Laser, Minigun, Plasma, Missiles, SpeedUp, Shield, Flamethrower,
  FrogHealth, TowerPack, HeatShield (player-only; `heat_shield_timer` keeps every kind of heat off,
  docs/volcano.md), Grenades (player-only, `grenade.rs`), SonicHammer (`sonic.rs`), Emp
  (`emp_burst`, `emp.rs`), GaussRail (`gauss_rail`, `gauss.rs`), FpvSwarm (`fpv_swarm`, `fpv.rs`),
  RodFromGod (`rod_from_god`, `rod.rs`), GravityWell (`gravity_well`, `well.rs`) (`PickupKind::ALL`,
  `name`/`parse` - the map's spelling -, `weapon` - the special a weapon crate arms -, `row` - the
  sheets' row -, `ink` - the symbol's shade/base/light, `punypalette.PICKUP_INK`'s values -,
  `cooks_off`); **each is drawn as a supply crate** (docs/CRATES_SPEC.md: `draw_pickup` from
  `static/crates_sheet.png` with an obstacle's drop shadow, the air drop while `Pickup::dropped_at`
  is young, the idle glint, the damaged and charred columns, a spilled pickup as its bare symbol
  from `static/pickup_glyphs.png` - `draw_glyph`, which the HUD's readouts draw too); no body,
  collected the moment a hull's box grown by `pickup_collect_pad_px` touches the pickup's square
  (`Pickup::in_reach`, a box test so the front and the corners collect like the side), fixed map
  slots. **Enemies only collect what they would use** (a weapon crate while on shells, or of the
  weapon carried while short of its carry limit, so one never trades away a stocked weapon;
  `Tank::wants_pickup`, the same predicate `ai::build`'s seek tiers call - collection must never
  be stricter than seeking or a tank parks on a pickup forever);
  players always collect. SpeedUp refreshes its timer, never stacks. **Shield is a pool on a
  clock**: full heal plus `shield_hp` and `shield_timer` (`Tank::raise_shield`; it shatters when
  `shield_seconds` run out, 6 by default, or when spent - about four enemy shells), spent at two
  seams - `take_damage` (absorb path) and `resolve_projectiles` (shots deflect and cost
  `shield_deflect_cost_factor` x damage); a live shield recharges after a delay, a shattered one
  never returns; `drain_shield_breaks` emits `ShieldBroken`. Shield and FrogHealth also ride the
  Health-slot bonus roll (`maybe_spawn_health_slot_bonuses`; the frog gate is checked before any RNG
  draw). FrogHealth heals the collector's own side's frog and is taken unless that frog is alive and
  full (docs/frog-health-pack-prd.md).
- `obstacle.rs` — `Obstacle` is a wall (`Material` Brick/Iron/Wood/Glass), a prop
  (Sandbag/Barrel/Fence, and Target - the range board, docs/range-target-prd.md: never rolled
  flammable, so a shot splinters it, yet `catches_fire` from the stream, a burning cell beside it or
  a lava bank, and burns for its own `burn_seconds`), a tree (Tree/Pine) or a tower
  (Tesla/GunTower/BioSlush, `is_tower`; `tower.rs`), or one drawn by its own module rather than a
  sheet (`is_drawn`: `Volcano` - a cone cell, permanent like Iron - and `Lamp`, a lamp post;
  docs/volcano.md). Ask the predicates (`is_permanent` - Iron only -, `blocks_sight`, `blocks_light`
  (the weather's shadows: walls but glass, and towers), `is_wall`/`is_prop`/`is_tree`,
  `pass_over_chance`, `deflect_chance`, `ram_seconds`, `flammable_chance`, `is_explosive`), never
  match variants. **Trees are 48 px art on a 32 px cell**: excluded from seam-closing in physics and
  hits (`battlefield::tile_half_extent`), no edge cap, drawn in the vegetation pass after the tanks;
  their idle is a colour shimmer, not motion, and a ram lean shifts whole 2 px bands. The edge-cap
  neighbour mask is cached and refreshed by `Game::refresh_edge_masks` on destruction. A barrel's
  `variant` is its `Drum`. Specs: docs/WALLS_SPEC.md, PROPS_SPEC.md, TREES_SPEC.md.
- `grass.rs` — tall grass, **deliberately not an `Obstacle`** (the nav grid and the linter treat
  every obstacle as impassable). Simulation-owned, seeded by `seed_at`, rebuilt in `init`, drawn
  interleaved with the y-sorted units (trees still after everything). **No tuft is drawn over a
  tile** (`keep_off`/`place`, from the non-tree obstacles at `init`): its root moves off a tile
  beside or below its cell by its art's reach (`TUFT_EXTENTS`, held against both sheets by a test)
  and the steady lean, its lean toward that tile is capped (`GrassTuft::lean`), and under a wall to
  the north it moves down until the art overlaps the wall's foot by at most `grass_wall_overlap_px`
  (8, about one brick course); a tuft that cannot fit takes the next of the sheet's tufts that does,
  and a cell with room for none grows none. Every tuft sorts with the tanks. Wind (`wind_at`: a
  steady lean plus gust fronts rolling across the field along `grass_gust_heading_deg` as a
  travelling wave on a slower envelope, pure in position and clock), each tuft's own flutter, crush
  and push are cosmetic ticks. Concealment is a *cell* query (`Terrain::conceals`) and gates four
  things - the group alert, the attack/chase tiers and ram damage - with `Ai::is_hit_alerted` as the
  exemption; gating only the shot measured worse. The shared alert
  (`Game::alert_position`/`alert_timer`: any enemy within `Game::enemy_sight` - `enemy_view_range`
  under the sky - of an unconcealed player refreshes the group's last-known position, held for
  `enemy_alert_hold_seconds`) is a pure distance test with no line-of-sight check on purpose -
  adding one re-opens the pile-ups the engage ring solved; re-baseline `just probe-fixtures` if you
  touch it. On a field map each enemy's alert is its own and travels a chain of neighbours instead
  (`simulation/field.rs`), under the same no-line-of-sight rule; re-baseline `just probe-fields` if
  you touch that.
- `tower.rs` (docs/defence-towers-prd.md, docs/TOWERS_SPEC.md) — `TowerKind` (Tesla/Gun/Bio, the
  `Material::{Tesla, GunTower, BioSlush}` tiles), `Tower` (side, heading, charge, bursts, burning),
  `Glob`, `OozePuddle`, `TeslaBolt`, `TowerRuin`, and the drawing generic over `canvas::Canvas`:
  `static/towers_sheet.png` holds each tower as a base that never turns and a **separate top layer
  rotated to the tower's heading** (the tank hull/turret split), four damage stages, a ruin and a
  glow overlay; `draw_ooze` (every puddle cell a pull in one metaball field, so the cells of a
  splash run together into one puddle) and `draw_glob` in the ooze's acid lime (`OOZE_*`, off the
  palette on purpose). A tower tile's `variant` is its side (`side_variant`). `render/tower.rs`
  draws the rest the way every shot is drawn (`render/shot_fx.rs`'s smooth additive primitives): the
  tesla's violet bolt, the towers' own light, a glob's trail, and the towers' pools of light on the
  ground (`draw_towers_ground_light`, from `draw_ground_light`); a strike and a splash are bursts
  composed in `burst.rs` (`ImpactKind::Tesla`, `ImpactKind::Ooze`), started by `fx.rs` off the
  events, and neither pushes the fire-coloured muzzle or impact flash. A tower a shot hits flashes
  like a tile (`tower::draw_tower_tinted`).
- `portal.rs` (docs/teleporting.md) — how a portal looks: the `PORTAL_FRAMES` baked frames of
  `static/portal_sheet.png` (row-major in rows of `PORTAL_SHEET_COLS`) cycled from `Game::time` and
  the position hash (never rotated at draw time), painted at the tail of `paint_floor` so thumbnails
  inherit them, an additive `pixel_disc` glow in the round, `Event::Teleported` bursts in `fx.rs`.
  Shots go through as `simulation/portals.rs` says. The tank mechanic is `Game::portal_phase`
  (before `step_world`: trigger radius, random exit among the *other* portals with room,
  `Grid::nearest_open_reachable` arrival outside the exit's radius, `place_tank`, per-tank
  `portal_cooldown`, `Ai::on_teleported` + `EngageRing::release`); fewer than two portals = inert,
  undrawn, grid untouched.
- `volcano.rs`, `lava.rs`, `lamp.rs` (docs/volcano.md) — the volcano, its lava and the lamps,
  headless. `volcano.rs`: the footprint (`in_footprint`, `FOOTPRINT_RADIUS2`), the cycle (`phase` -
  Asleep, Rumble, Erupt, Cool - a pure function of the round clock and a hashed offset per crater),
  a bomb in flight (`LavaBomb`, its arc derived from age), the cone's picture (`compose_cone`, baked
  once per set of outlets and cached: block runs in `ASH`/`SCORIA` with molten gullies, through
  `trig.rs` so a pinned thumbnail agrees across platforms; `ConeImages` its body, outline, shadow
  and skirt as `BlockImage`s, `draw_cone_with`/`draw_cone_glow_with` drawing from them where the
  canvas holds them) and the composers in the effects language (`plume`, `eruption`, `bomb`,
  `bomb_ring`). `lava.rs`: `LavaLayout`, built from the map alone at the top of `init` - water's
  shape through `ground::WaterLayout` (a stream a ford, a lake's open middle deep), the flow away
  from the cones (a walk from the cells beside a cone; a cut-off run flows from its end nearest one,
  else down the map), the heat (1 in the lava, `lava_heat_falloff` a cell out, gone past four;
  `Game::heat_at`) - and the drawing generic over `canvas::Canvas` (`draw_cell` block by block from
  the neighbour mask with flow bands and crust plates on a lake, `draw_floes`, the toasted banks
  baked once into a `BlockImage` beside the floor shade, a bomb's pool as metaballs). **The picture
  is kept between frames** (docs/volcano.md "Drawing it cheaply"): `LavaPictures` (in a `RefCell` on
  the layout; `Game::refresh_pictures(views)` brings it up before `app.rs` syncs the textures) -
  each cell's blocks split into what is fixed (`BlockGeo`) and the colour the clock gives it, baked
  a quarter of the tiles on screen a frame (`STAGGER`, all of them after a clock jump past
  `JUMP_SECONDS`) into a `TileAtlas` of the lava and one of its bright blocks, the bombs' pools
  reshaped only when one comes or goes, the banks' daylight glow baked once at `GROUND_LIGHT_BAKED`
  (`Game::draw_lava_ground_light` adds it to the colour alone, a separate blend), each crater's
  molten pixels; a painter takes it only where `fresh(time)` and the canvas holds the texture, else
  draws block by block, and the tests hold the two equal texel for texel. `lamp.rs`: a lamp post's
  and a lantern's drawing and `Lantern`. The light they throw is `weather::lights_in`'s, and
  `Game::sight_on` is the rule it sets: an enemy sees whoever stands within `lamp_reveal_px` of a
  lamp or a lantern, or on ground `lava_reveal_heat` hot, at the full `enemy_view_range` whatever
  the sky. The HUD's vitals carry the lantern count and the heat shield's gauge in a row under them
  that is there only while it holds one (`CornerShape::lamp_row`; the touch screen's
  `CornerButton::Lamp` where there are lanterns), the indicators an arrow to a waking volcano off
  the screen (`ArrowKind::Volcano`, never dropped by the cap) and the minimap a frame pulsing round
  its crater.
- `target.rs` (docs/range-target-prd.md) — the range board's fire, headless: `fire_shapes` is the
  soot creeping in from the rim under the flamethrower's stream (`Obstacle::heat`) and the embers
  pulsing over its char once it burns, in the effects language and drawn by `paint_tiles` under the
  tile flames; the char itself is `target_sheet.png`'s three burn columns (the board drawn on 44 px
  cells, 30 % larger than a prop, on its one-cell footprint), which `Obstacle::col` steps through in
  order by `burn_progress`, and `burn_sag` drops the board a block in the last fifth. Everything
  runs on `Obstacle::burn_shown`, which `tick_burn_frame` advances on a replica too. Tests:
  `target::tests`, the board cases at the end of `props_tests`.
- `ground.rs` (docs/GROUND_SPEC.md) — the decorative floor from the Puny World tileset, built once
  per round: the pack's grass fill (dust under the desert retint), soft drift patches of its sand
  tiles through the pack's corner autotile (`SAND_CORNER`, hashed at the grid vertices, never beside
  a road or water cell, only when `Theme::drifts`; `ground_drift_cover`/`ground_drift_scale`), road
  under walls, water (`ground::Layout`, one reading for the picture and the rules: a water cell
  inside a 2x2 block is a lake cell on the `WATER_SHORE` corner autotile at the vertices - wet only
  where all four cells are water, plus a saddle-smoothing pass that lets a diagonal run flow, the
  grass cell in each step drawn from the lake tiles -, a lake cell a stream enters takes a
  `WATER_MOUTH` tile, any other water cell is a stream cell on the `WATER_CHANNEL` edge autotile,
  and water painted to the map's edge runs off it; the diagonal shores and corner mouths the pack
  lacks are composed from its own tiles by `tools/water_tiles.py` inside `retint_ground.py` (ids
  602-615, `WATER_EXTRA`), and `no_tile_edge_puts_water_against_land` checks every tile edge pixel
  by pixel; the pack's four-frame shimmer baked per cell as `[i32; 4]`, and `draw_current` drifts 2
  px marks down open water and north/south streams - `water_frame_seconds`, `water_flow_speed`,
  `water_flow_lanes`; `draw` takes the clock: `Game::time` in a round, the wall clock in the
  builder). `WaterLayout` (`Game::water`, built from the map's cells at the top of `init`) is the
  rules' view - `Depth::{Dry, Shallow, Deep}`, `pushes_south` -, docs/water.md: deep cells (a lake's
  open middle) get seam-closed static colliders and block the nav grid but never a shot
  (`hits::Terrain` never sees them); a ford scales a hull's pace and grip (`Footing` in
  `drive_tank_with`, which runs in the water's frame so a north/south stream's `water_current_speed`
  carries a stopped hull south; the sky adds its own - wet grip, a gust's flow - and under snow
  `WaterLayout::freeze` makes every water cell `Depth::Ice`, whose footing keeps the pace and loses
  grip, traction and brake) and costs `water_ford_path_cost` per step to the router (`Grid::weigh`,
  occupancy untouched); water takes no heat, no fire, no scorch, and puts afterburn out; the frog's
  hop prefers a wet landing; tread marks stop in water and come out wet; `fx.rs` throws spray.
  `dry_cell_near` moves a centre-fallback start out of a lake. `GroundGrid` keeps its `Layout`, seed
  and `Drifts`, so `repaint(&[(col, row, CellFloor)])` re-resolves only the tiles an edit can
  change, matching the walls it adds and removes in one pass, and lands where `build` would
  (`a_repaint_is_the_build_of_the_edited_lists`), and answers the cells whose tile - and so whose
  water depth - can have changed (`Layout::depth` is the one depth reading, `WaterLayout::build`'s
  and `GroundGrid::depth`'s); wet vertices are re-run whole only with water within a cell of the
  change, because the saddle smoothing is an order-dependent fixpoint, and the shade re-bakes only
  the blocks a changed wall reaches (`ShadeRecipe`) under a new stamp with a `canvas::BlockPatch`.
  The floor shade (`bake_shade`, `GroundGrid::shade`, drawn by `draw_shade` at the head of
  `paint_floor_marks`, docs/GROUND_SPEC.md §4): the walls' contact shade leaning along `shadow_dir`
  and, on a round's field (`Look::edge_shade`; the builder's canvas has none), a rounded edge shade
  deepening into the corners toward the theme's own dark, both stepped and Bayer-dithered on the 2
  px block grid into one `BlockImage` - baked on first draw, so a room server never pays for it.
  **Past an arena's field** (docs/GROUND_SPEC.md §4, `margin.rs`): `GroundGrid::beyond(cells)` is
  the round's floor in a layout grown `cells` cells on every side (`origin`), hashed by world cell
  so the cells the two grids share draw alike, water past the map the nearest map cell's (water
  painted to the edge runs out of the picture), a road ending on its cap; `bake_margin_shade`
  carries the edge shade on past the edge in its own steps and dither, deepening over
  `ground_margin_ramp_px` to `ground_margin_shade` (`MarginShade`: image, origin in blocks, plateau
  colour). Materials are named after the pack's wangset colours, not the theme.
- `weather.rs` (docs/weather.md) — the sky, headless. **The rules** (the one part the simulation
  reads): `Game::init` settles the round's sky once (`Game::weather`, a field), and `sight_factor`
  (night and storm `night_sight_factor`, fog `fog_sight_factor`, times `enemy_view_range` =
  `Game::enemy_sight`, which the shared alert, the chase and attack tiers - `Ai::think`'s `sight`,
  the attack range capped by it -, the engagement ring and a hunter's snipe read), `grip_factor`
  (rain and storm, `rain_grip_factor` on every hull's grip), `freezes` (snow:
  `ground::WaterLayout::freeze` turns every water cell into `Depth::Ice` - no deep colliders, open
  to the nav grid and spawns, no current, tracks but no wet marks, spray, fire-dousing or frog
  refuge; `Footing` gives it `ice_grip_factor`/`ice_traction_factor`/`ice_brake_factor`) and
  `gusts`/`gust_at` (a sandstorm's gust front crossing the field every `sand_gust_gap_seconds` or
  so, a pure function of the round clock like `lightning`, added to `Footing::flow` so a hull drives
  relative to the wind; the pose validator allows the drift). All of it is gated by `weather_rules`,
  and every factor is exactly 1 under a clear sky, so a clear round replays bit for bit. The look:
  `map::Weather` resolved to a `Look` (the ambient light the field is multiplied by, how strongly
  the round's lights show, rain/lightning/fog/sand/snow/haze amounts; `Look::of` applies the
  `weather` tuning group, `weather_strength` 0 is clear) and its `Plan` (which stages run; `None`
  for clear, which draws exactly as before). `in_force` is the map's skies under the
  `weather_override` knob (`--weather`, the page's `?weather=` - `weather_from_url`; a `Restart`
  row), never `Random`: `pick_sky(skies, round_seed)` takes one of them (`random_sky` over every
  sky) - a hash of the seed, never a draw from the round's RNG, so the stream is untouched and a
  pinned seed pins the sky. **A room's round is its map's sky alone** (`Game::weather_from_map`, set
  by `net::apply::welcome` on the replica and the sandbox and by the rig on its authority, which
  bakes the window's override into the room's map key instead), so every client draws, predicts and
  is validated under the room's sky with nothing new on the wire. `lights(game, impacts, look, t)`
  gathers every light the round throws - headlight cones from each chassis's lamps
  (`tank_art::HEADLIGHTS`, half of them out at the second damage tier, all at the third) and
  team-coloured hull glows, fires, fuses, portals, blasts, every shot and flash, the particle
  layer's hits, faint pickup and frog glows - each cast against `Occluders` (the cells whose
  `Material::blocks_light`, walked Amanatides-Woo, the light's own cell never stopping it) into a
  per-ray reach. `lightning(time, t)` is a pure function of the round clock. Reads `Game`, never
  writes it, draws no RNG; nothing in `simulation/` reads it. `render/weather.rs` is `WeatherFx`:
  the light map (the scene target's view a texel per 2 px block, `LIGHT_MAP_SCALE`, cleared to the
  ambient; `lights_in` drops lights that cannot reach the camera's view before casting; an unblocked
  point light (`Light::unblocked`) is one quad of a soft disc baked to the fan's own ring falloff, a
  still one (`Light::still`: a portal, a lamp post, a lantern) a shadowed pool kept in
  `LightCache`'s atlas until its reach changes and tinted each frame, anything else a fan of
  vertex-coloured triangles out to its reach; stored halved so a pixel lights to twice daylight, a
  brighter colour drawn in shares), the cell mask (water depth and road per map cell, made once a
  round - `MaskFor`, `minimap::RoundKey` and the grid's first cell; a `CellMask` names its first
  cell - the shader's `cellOrigin` - so the margins' wider grid reads the same: `margin_mask_bytes`,
  `MaskTexture::sync_for`), and three passes around pass 1 - ground (`static/weather_ground.fs`:
  snow, ice, wet earth, puddles, rings, over the bare ground and under the marks), light
  (`weather_light.fs`: the field times the light map, banded on the 2 px grid with dithered step
  edges) and sky (`weather_sky.fs`: haze, fog, sand, rain, snow, lightning's white) - ping-ponging
  with `scene_target` and always ending in it. Each pass draws a `PassView` (its target's world
  rect, its light map and the parts it draws, none for the whole target), which is how
  `render::margin` runs them over the margins (`WeatherFx::shader_passes`). Shaders compiled from
  sources in the binary (ES 100 twins in `static/web/`); a driver that will not compile them draws
  the sky without them (docs/weather.md "Without shaders"): `weather/plain.rs` composes the snow on
  the ground and the fog, sand, rain and snow in the air as plain blocks from the shaders' own noise
  (headless, tested), and `render/weather.rs` multiplies the light map onto the field with a blend
  mode (`multiply_light`) and draws the blocks with the target's alpha kept (`draw_blocks`); the
  `weather_without_shaders` knob shows it anywhere and `status.weather.without_shaders` reports it.

## Weapons and projectiles

- `shell.rs`, `bullet.rs`, `plasma.rs`, `laser.rs` — projectiles (docs/SHELLS_SPEC.md,
  BULLETS_SPEC.md, PLASMA_SPEC.md). `Owner` lives in `shell.rs` (`same_side` is the friendly-fire
  test). Shell rows are matched to chassis via `TANK_SHELL_VARIANT_BY_ROW` and the staggered table
  (`Tank::alternate_shot` for twin barrels); the flying frame is identical across rows. Every
  projectile carries a per-round `id` from one counter (`Game::spawn_pending`), and each state enum
  maps to its sheet column (`col`/`from_col`), the wire's spelling. Bullets have a 3-state machine
  and leave the minigun module, boresighted onto the gun line `minigun_boresight_px` ahead
  (`Bullet::spawn`); plasma rows are keyed by variant not chassis, never ricochet, and fly as a
  shader orb (`static/plasma_orb.fs`: a lit sphere with 3D-noise energy spun about a per-bolt axis,
  teal electric with lightning, purple arcane with spiral arms, a comet tail, worked out per 2 px
  block of the field in the bolt's own ramp steps; `plasma_orb_*`, per-bolt variety in `OrbLook`)
  with the baked sprite wherever the shaders did not compile; the laser is an instant-hit drawn
  line, judged along the gun line and drawn from the laser module's lens to where it stopped. Their
  drawing - sprite columns, source rects, shadows, the glow - is
  `render/{shell,bullet,plasma,laser}.rs`; everything else is the effects language
  (docs/effects.md): the light every shot throws - tracers, halos, the laser's bloom and end flares,
  the missile's exhaust - is additive, built from 2 px blocks (stepped glows and streaks and rays of
  blocks, `render/shot_fx.rs`'s `glow`/`streak`/`taper`) and stateless (a `draw_*_light` per
  projectile, additive blocks in `Game::render`), with stepped pools of light on the ground under
  every shot, burn and flash (`Game::draw_ground_light`); the flamethrower's jet of burning fuel
  (`static/flame_jet.fs`: a tight rope that whips and carries slugs of fuel, blooming into rolling
  fire near the reach, worked out per 2 px block in the fire ramp's flat steps) and the orb are
  `render/shot_shaders.rs` (`ShotShaders`, compiled from sources embedded in the binary,
  `Effects::shots`, `None` falls back to the sprites); every hit is a burst `burst.rs` composes and
  every muzzle flash `burst::muzzle`, with the baked muzzle and impact frames left out
  (`*_hit_seconds`, `hit_fx_scale`, `muzzle_glow_radius`), all scaled by the `shot_fx` tuning group
  (`shot_glow_strength` 0 draws the plain sprites); `fx.rs` adds the particles (muzzle sparks down
  the shot's line, hit sparks off hulls and the border, a laser's burn, glints off plasma and
  shells).
- `missile.rs` — the seeker missile's flight (`Missile`, `MissileStage` Climb → Seek → Chase →
  Dive): a ground point plus a `height`, drawn lifted and scaled up with its shadow on the ground
  (the `FlyingDrum` convention) and pointing along the path it is *drawn* on (`Missile::facing`:
  ground motion plus climb, eased by `missile_facing_smoothing`; the shadow keeps the ground
  heading); the chase turns at a limited, growing rate, and inside `missile_commit_distance` it
  stops tracking and dives on the spot, so a moving tank can slip it. Fired as a volley of
  `missile_salvos` salvos of four, one per tube of the launcher module (`Tank::missile_volley`,
  `missile_salvo_gap_seconds` between salvos), each from its tube's mouth
  (`tank_art::MISSILE_TUBES`) and fanned and landed by its place in `MISSILE_TUBE_OFFSETS`, so a
  missile comes down on the side it fanned out to; the module empties through each salvo
  (`missile_tubes_empty`). `fx.rs` lays each missile's smoke trail by distance flown
  (`ParticleKind::Trail`, `missile_trail_*`). Tuning group `missiles`.
- `grenade.rs` (docs/grenade-launcher.md) — a grenade (`Grenade`; `pickup::PickupKind::Grenades`,
  `Tank::grenade_ammo`, six a crate, `ActiveWeapon::Grenades`, one per press, player-only): `launch`
  from just clear of the hull along the gun line plus `grenade_launch_carry` of the hull's velocity,
  lobbed up (`height`/`climb`, `grenade_gravity`); `roll` is its whole motion with no physics body -
  in the air (`airborne`) it flies over every tile and hull, swept against `Surroundings::edges`
  alone, and lands hopping; on the ground drag (times
  `grenade_water_drag_factor`/`grenade_ice_drag_factor`), hulls pushing it out and handing it their
  motion (`grenade_tank_restitution`), a sweep against the boxes reflecting off the face struck
  (`grenade_wall_restitution`); its band flashes white-hot, faster across `grenade_fuse_seconds`
  (`lamp_lit_at`); `draw_grenade` is a steel canister (black cap, red band, brass lever turning with
  the roll) lying across its roll and tumbling in the air, lifted by its height over its shadow, in
  the y-sorted standing walk so grass covers it only from in front (over everything while airborne);
  `fx.rs` trails a thick, shaded white `ParticleKind::Plume` (`Game::grenade_trails`,
  `grenade_trail_*`), opaque blocks thinned by the dither (blended white lands grey on the scene
  target). The launcher module (`TANK_MODULE_GRENADE_COL`, its drum showing the rounds fired) and
  `tank_art::GRENADE_MUZZLE` come from `tankdesign`. Tuning group `grenades`.
- `sonic.rs` (docs/sonic-hammer.md) — the sonic hammer, headless: `SonicCone` (a fan of rays
  `SONIC_RAY_ARC_PX` apart walked over map cells, stopped by `Material::blocks_sound`, glass a
  `Block::Glass` pane it strikes; `reaches`, `entered`, `nearest_reached`), `SonicWave` (its front
  at `sonic_wave_speed`, `spent`, `done`), `falloff`, `shove_speed`,
  `skid_friction`/`skid_seconds`/`slide` (the AI predicts with `slide`), `drum_landing`,
  `tree_push`, `module_cell`, and the composers `wave_arcs`, `wave_dust`, `tell_arcs` in the `STONE`
  ramp (`pyro::Shape::Arc`). The `sonic` tuning group. **The shared path the BB-36 weapons plug
  into** (docs/sonic-hammer.md §3): `Tank::tell`/`Windup` (an enemy's wind-up, `enemy_trigger`, held
  over the commander's orders by `commanded_intent`, carried in `TankState::tell`, an
  `ArrowKind::Windup` arrow), the AI hook (`ai::SpecialSense`, `special_rule`, `generic_fire`,
  `SEEK_SPECIALS`), the press drawn at once online (`WeaponKind::drawn_on_press`,
  `WireEvent::press_show`, `Show::OwnShotsDrawn { presses }`, `PressShow`), `HitCause`,
  `Tank::special` (what a tank carries, against `active_weapon`, what it fires).
- `emp.rs` (docs/emp-burst.md) — the EMP burst, headless: `EmpPulse` (a ring from a hull at
  `emp_ring_speed` to `emp_radius_px`, nothing stopping it), `box_reach` (the ring reaches a box by
  its nearest point), `seat_value` (the AI's scoring), `droop_side`, `module_cell`, `lamp_lit`,
  `sparking` and the composers `ring`, `sparks`, `tell`, `lamp_sparks` in `pyro::EMP`. The `emp`
  tuning group. **The disabled state** the later weapons share: `Tank::disable`/`disabled` (an
  enemy's brain off, every seat's special offline - `Tank::special_down`, and `active_weapon` the
  shell meanwhile -, its lights out, `draw_tank_dark`, an enemy's turret sagging, `droop`),
  `Tower::disable`, `WeaponSlot::offline` (`WPN OFFLINE`), `TankState::{disabled, offline}`,
  `Predictor::offline_left`.
- `gauss.rs` (docs/gauss-rail.md) — the gauss rail, headless: `RailSlug` (a leg as drawn: start,
  end, portal, overcharged, `Pierce`s, age), `ChargeEndFx`, `muzzle`, `damage`, `recoil_speed`,
  `module_cell`, and the composers `compose_charge`, `compose_slug` (glowing), `compose_slug_lit`,
  `compose_end` in `pyro::RAIL` with glows in `RAIL_LIGHT`. The `gauss` tuning group. **The charge**
  later weapons share (the rod's reticle): `Trigger`, `ChargeRule`, `Charge`,
  `ChargeStage`/`ChargeEdge`/`ChargeEnd` on `Tank`, `Tank::windup` answering a charge,
  `TankState::charge`, `WeaponSlot::charge` (the HUD's gauge), `SpecialUse::{Charge, Release}`,
  `PressShow::{Rail, ChargeEnd}`.
- `fpv.rs`, `air.rs` (docs/fpv-swarm.md) — the FPV swarm, headless: `Drone` (Launch along
  `launch_path`, the same whatever it is after - what lets a client draw its own launch -, Cruise,
  Dive committed at `fpv_commit_px`, Falling when struck), `DroneLock` (`code`, the wire's),
  `AirWant`, `halo_slot`, the composers `compose_drone` (twice the block up at cruise height,
  `drone_block`), `compose_shadow`, `compose_halo`, `look_of`, `module_cell`; `air.rs` is
  `AirTarget` (its `strike_box` the column from shadow to body), `AirKey`, `AirStrike` and the rules
  every reader keeps. The `fpv` tuning group.
- `rod.rs`, `zone.rs` (docs/rod-from-god.md) — the rod from god, headless:
  `Reticle`/`Range`/`Steer`/`step_reticle` (a stick a cell at a time with a repeat, an enemy's aim
  walking to its cell, a report, all held to the caller's sight box), `rule()` (its `ChargeRule`),
  `SeatStill`, `cell_reach`, `shove_speed`, `crater_cells`, `Craters`, `lens`
  (`tank_art::ROD_LENS`), `module_cell`, and the composers `compose_reticles`, `compose_calls` (the
  circle, the beam to the view's top, the count in `pyro::digits`, on the zones' clock -
  `Game::zone_lead`, a client's present online), `compose_column`, `compose_impact`,
  `compose_crater_smoke` in `pyro::LASER_RED` and the dust ramps; `draw_crater` on any canvas.
  `zone.rs` is `Zone` (id, kind, owner, centre, `until`; `danger`, `route`, `wire_kind`,
  `provisional`) - the rod's call the first kind, the gravity well's next. The `rod` tuning group.
- `well.rs` (docs/gravity-well.md) — the gravity well, headless: `Orb` (a shot on the wire,
  `ShotKind::Orb`), `WellZone`/`WellStage`/`AnchorBy`, `WellField` (the wells pulling at a tick,
  read off the zones' clock - `at`, `pull`, `hull_pull`, `shot_accel`, `core_hit`, `strongest`),
  `strength`, `bend` (a heading turned at its own speed), `HeldDrum`/`held_at` and
  `GrenadeOrbit`/`orbit_at` (pure functions of the round clock), `holds_broadside`, `escape_dir`,
  `brace_dir`, `frog_fling_target`, `module_cell`, `Swallow`, `WellFx`, and the composers
  `compose_orb`, `compose_snap`, `compose_well` (the core drawn after every light by
  `compose_core`), `compose_swirl`, `compose_collapse`, `compose_collapse_dust`, `compose_swallow`
  in `pyro::VOID`; `curved_streak` lays a shell's or a bullet's tracer along the path the pull bent.
  Online a client flies its own orb from the press (`net::round`'s `OwnOrb`, `OwnAnchor` until the
  room's `WellAnchored` claims it), own orbs and provisional shots bend on the room's tick grid, its
  sandbox pulls the hull on the round tick its input lands on, incoming fire is carried to the
  present through the wells a tick at a time (`round::carry`: the seat's hull and the walls before a
  core), and the pose validator allows the pull toward the core and no faster than the solver's cap
  (`WELL_SIDE_REACH_SECONDS`); a pulled hull, like every hull, looks a step's travel ahead
  (`physics::HULL_LOOK_AHEAD`), so the pull presses it flush against a wall rather than into it. The probe holds a pulled tank out of its hands, a brace and a hold for the orb as
  deliberate, leaves a braced tank's clustering and pile-up uncounted, and tags an anomaly in a pull
  or within three seconds of one `/pull`. The `well` tuning group.

## Drawing and effects

- `game.rs` — the field painted through `canvas::Canvas` in three stages any canvas can run,
  headless: `paint_floor` (ground, floor shade, tracks, scorches, landed rubble, oil, portals -
  `paint_ground` then `paint_floor_marks`, the seam the weather's ground pass draws in),
  `paint_tiles` (walls/props with shadows and caps, then the flames on burning timber,
  `tile_flames`), `paint_standing(PaintOptions)` (pickups, the y-sorted tanks/frogs/grass walk - a
  tank's layers (`tank::draw_tank`) and then its flames, `damage_stage.rs` - trees, a burning one
  with its flames), `paint_field` = all three; reads state, never mutates it.
  `PaintOptions::locate_cue` is the one thing a thumbnail leaves out. `Game::plain_canvas` (flat
  white instead of the ground tileset; the builder has the same flag) and `Game::hide_players` (no
  player sprite/ring/label) exist for the demos. `render/game.rs` is `Game::render` with
  `Textures`/`Effects`: pass 1 runs the three stages through a `GpuCanvas` into `scene_target` (the
  camera's view at a texel per world pixel, drawn inside `Camera::in_target`; `Canvas::cull` lets
  the ground loops, tiles, grass, trees and floor marks skip what lies outside it, and the effect
  lists are culled the same way) with the live-round layers between them (fire, glows, the `P1`
  label, hit flashes, projectiles, hits, muzzle flashes, blasts, airborne debris, particles) drawn
  with raylib directly, as two halves - `paint_field_lit` (the floor, tiles and everything standing)
  and `paint_field_glowing` (what shines by itself, the tanks' light layer included -
  `draw_tank_glow` at `1 - day pools`) - so a weathered frame can multiply the first by the light
  map before the second (`render/weather.rs`); pass 2 is `draw_world_layer` (the scene through the
  shockwave, the ripples' quads, the kill flash, the debug overlays) then `draw_chrome` on the
  window in UI points (the indicators, then the end screen, banners and PAUSED under the corner
  clusters - still pressable -, then the dialogs, lobby and level select over them, their dims over
  the whole window, and the stick; Build's bar through `draw_bar_layer`): an arena runs both into
  the bitmap, while a followed view draws its world into `composite` at one pixel per texel,
  `render::view::present_world` puts it on the window shifted by `Camera::offset`, and `draw_chrome`
  draws on the window through the bitmap-to-window camera, so the HUD stands still while the world
  slides under it. After pass 2 an arena's margins are made (`Effects::margins`) for `present_into`;
  the composite is always cleared to black, so the field looks the same with margins or bars round
  it. Takes `hud::PlayChrome` from `Session::play_chrome`. `draw_debug_overlays` and the per-tank
  `draw_tank_boxes`/`draw_tank_stats` (`cfg(feature = "dev-tools")`) draw through `Camera::on_field`
  after pass 2, and then `render::indicators::draw_indicators` paints `Effects::indicators`, under
  the banners and dialogs. `Effects::split` (`SplitLayer`): `Follow` draws the second half of a
  couch split through its own camera and targets (`Game::draw_world` is one view's passes 1 and 2),
  `render::view::present_half` puts it past the divider as a textured convex polygon (raw rlgl) and
  `draw_divider` draws one 2 px block in near-black over both, fading in over the first 8 px the
  halves part; `Zoom` presents the establishing shot's whole-field composite a second time through
  the second half's zoom (`present_zoom_half`, `draw_zoom_divider`).
- `canvas.rs` (docs/mapshot-prd.md) — the drawing surface the field's leaf draw fns (`ground`,
  `grass`, `obstacle`, `frog`, `pickup`, `tank` rings/hull/turret/modules, `damage_stage`, `track`,
  `blast::draw_scorch`, `decal::draw_decal`) are generic over: `Sheet` names a sprite sheet
  (`Sheet::path` is the one asset-path table every loader reads; `Ground`/`Grass` carry the map's
  `Theme`, so `Sheet::all` holds both themes' files and `ground::draw`/`grass::draw_tuft` name the
  live one; `obstacle::Sheet` is a re-export), `Canvas` is
  `blit`/`fill_rect`/`gradient_v|h`/`disc`/`ring` with `DrawTexturePro`/`DrawRing` semantics, plus
  `blocks` (a baked `BlockImage` - the floor shade - laid over the field at a whole-block scale, one
  draw on the GPU; `BlockImage::patches`/`changed_since` keep its last `BLOCK_PATCHES_KEPT` (64)
  patched rectangles, so `BlockTexture::sync` uploads only those texels and a copy further behind
  takes the whole image), `blocks_part` (one rectangle of one) and `has_blocks` (whether this canvas
  holds that stamp's copy - a thumbnail or the builder does not, and draws the long way),
  `TileAtlas` (a `BlockImage` of 16 x 16-texel tiles, one per map cell, put, removed and drawn by
  cell - the lava's kept picture), and **no text** (the `P1` label stays raylib-only), `CpuCanvas`
  is an own nearest-neighbour rasteriser over `Vec<math::Color>` with alpha-over blending, headless
  (rotation and arcs through `trig.rs`, portable `sin`/`cos`/`atan2` in plain f64 arithmetic,
  because libm differs by an ulp between macOS and Linux and the pinned thumbnail hashes must agree
  on CI; the grass sway uses it too) (raylib's `ImageDraw` resizes linearly, cannot mirror and does
  not blend, so raylib only decodes sheets and encodes PNGs). `render/canvas.rs` is the raylib side:
  `Sheets`, the `Sheet` → `Texture2D` lookup (`render::game::Textures`, `editor::EditorTextures`,
  `render::thumbnail::GpuSheets`), `GpuCanvas`, which forwards to raylib and borrows the draw handle
  for one statement - make one per stage -, `BlockTexture` (the GPU copy of a `BlockImage`, uploaded
  once per stamp by its owner - `app.rs` for the round and the builder, `render_gpu` - and handed
  out through `Sheets::blocks_texture`, so a draw never uploads; `BlockTextures` a list of them, one
  per place - the round's floor shade, the lava's banks, atlases and craters, the cones, from
  `Game::with_block_images`, held in `Textures::blocks`), `Pixels::load` (PNG decode) and
  `CpuCanvas::{load, to_image, png_bytes, write_png}`.
- `pyro.rs` (docs/effects.md) — **the effects language**, the one set of rules every explosion, hit,
  fire, smoke plume and damage mark is drawn by: whole 2 px blocks on the field's grid
  (`block_disc`, `dither_disc`, `block_line`, `mark`), colours as ramp steps (`FIRE`, `SMOKE`,
  `DUST`, `CHAR`, and the energy weapons' own; `dust_of` per material;
  `step`/`step_dithered`/`between` never blend), fades through the field's Bayer pattern (`bayer`)
  down to half then in eighths, light as stepped glows with dithered band edges (`glow`,
  `glow_bands`), shaded puffs (`Puff`, `shade`, `fire_puff`, `dust_puff`), flames (`tongues`) and
  the wind smoke leans on (`smoke_lean`, off `grass::wind_at`). Composers return `Shape`s; `draw`
  paints them and `draw_glows` their light inside an additive blend, on any `Blocks` - every
  `Canvas`, and the raylib handle through `render::pyro::Rl`. Headless and pure.
- `fireball.rs`, `burst.rs` (docs/effects.md) — composed at draw time, pure functions of what they
  draw and its age, no RNG: `fireball::compose` is every blast but a mushroom cloud (a drum, a plain
  kill, a missile's burst, a cook-off): a flash with rays, shaded puffs burning white to red and
  cooling into smoke that climbs and leans with the wind, fragments, dust racing out, one light over
  the hottest fire, its form (round, column, flat, double) the `BlastFx::row` the cause hashed,
  sized by `blast_fireball_px` and paced by `blast_fireball_seconds`; `fireball::done` is when a
  `BlastFx` goes. `burst::compose` is every hit by `fx::ImpactKind` - a shell's flash and small
  fireball thrown back toward the gun, a bullet's or ricochet's spark star, plasma and tesla rings,
  a laser's molten splash, ooze, a tile's dust and collapse cloud in its material's colours - and
  `burst::muzzle` every muzzle flash, a tongue of fire down the shot's line (a plasma cannon's in
  its bolt's colour).
- `mushroom.rs` (docs/mushroom-cloud.md) — the mushroom cloud a dying tank goes up in
  (`wreck_mushroom_chance` of kills, hashed in `BlastFx::wreck`): composed at draw time in the
  effects language, never from a sheet: shaded block puffs (no outlines) for the stem, the rolling
  cap and the dust, a warm flash with rays, the shock ring, condensation, debris streaks and a
  stepped bloom, leaning with the wind; `Cloud::compose` is pure and tested, `draw` paints through
  `pyro::Blocks`, called from `render/game.rs`'s blast loop where a `BlastFx` carries a cloud. Shape
  and pace hashed per kill.
- `damage_stage.rs` (docs/effects.md) — what a damaged tank gives off, in the effects language. The
  tank sheet's damage tiers and wrecks carry the wear (`Tank::damage_tier`); this adds what moves, a
  step per tier, from one hashed spot on the engine deck behind the turret (`engine_deck`, one of
  `DAMAGE_VARIANTS` per tank): from the damaged tier (`SMOKES_AT`) smoke (`smoke`, which `fx.rs`
  puts up), from the critical tier (`BURNS_AT`) or with afterburn on it a burning deck, and a wreck
  burning hard until the last fifth of `wreck_burn_seconds` (`fire`, `flames` - `pyro::tongues` plus
  their one light, which `render/game.rs` draws with the ground's fire glows).
- `blast.rs`, `decal.rs`, `track.rs` — blast presentation state (`BlastFx::shaped` hashes the form,
  mirror, turn, pace and size, the cause overrides; `fireball.rs` composes it), scorches (a blast's,
  and `Scorch::burn`, the scar a burnt-out ground fire leaves - drawn from `barrel_explosion.png`'s
  scorch row, the one row of it drawn), and in `render/blast.rs` the fuse, burning-cell and nozzle
  glows; decals are rubble on the walls sheet (rows 14-21, the props' rubble too, row 21 the tank's)
  seeded by `blast::seed_at` (a salted, avalanche-mixed position hash - positions are multiples of
  32), thrown arcs derived from age so nothing integrates them, staged by `obstacle_died`; tread
  marks (a wreck's marks are scorched and never fade).
- `crate_fx.rs` (docs/CRATES_SPEC.md) — the crates' shows, pure functions of age in the effects
  language: `drop` (the air drop a crate comes down in; drawn only - a crate can be taken the frame
  it appears), `glint_col` (the idle, hashed per crate) and `open` (`Opening::Taken` - planks fly,
  the symbol rises and drops into the tank -, `Spilled`, `Broken`). Knobs: the `crates` tuning
  group.
- `fx.rs` — particles, owned by `app.rs` and passed through `Effects::fx`; `render/fx.rs` draws them
  in the effects language (docs/effects.md). **Deliberately not a field of `Game`**, which is why it
  may use `rand::rng()` while `decal.rs` may not: anything whose position matters later belongs in
  `decal.rs`. `Fx::observe` diffs bursts out of `game.events()` (guarded on `Game::frame()`, since
  the dev server renders without advancing) and samples continuous emitters from
  burning/ramming/contact states - among them the smoke off a damaged hull's engine deck
  (`damage_stage::smoke`, `hull_smoke_rate`, black once the deck burns: `SOOT_T`, which oil fires
  and wrecks smoke too). Every particle is whole 2 px blocks in ramp steps: light (sparks, embers) a
  hot block cooling down the fire ramp, a fast spark dragging a tail; air (smoke, dust, trail)
  shaded puffs drifting down-wind (`pyro::smoke_lean`); matter (chips, spray) small squares;
  non-additive kinds then one additive block (a blend switch breaks the batch). `fx_max_particles`,
  `fx_density` (halved on embedded builds). It keeps the **crate openings** (`CrateOpen`, from
  `Event::PickupCollected` and `CrateBroken`, composed by `crate_fx::open` and drawn over the tanks)
  and the **hits** (`Impact`, `ImpactKind`): a shell, bullet or plasma bolt seen in its impact
  frames (by id, `watch_impacts`, so a replica starts the same bursts), a `LaserBeam`'s end, a
  ricochet, a deflection, a tesla strike, a glob's splash, a tile's dust when a shot hits it
  (`Dust`) and its cloud when it comes down (`Collapse`, or `Ash` for a tile that burnt out) start
  one, which plays on this layer's clock for its kind's seconds - longer than the projectile's own
  impact frames - and is composed by `burst.rs`; and the **flashes** (`Flash`, `Flashed`): a hull,
  tile or tower a shot landed on, drawn again in light by `render/game.rs` for `hit_flash_seconds`.
  In a dev-server lockstep `app.rs` ages the layer by the simulated time a `step` ran, not wall
  time, so a frozen round's particles freeze with it.
- `fish.rs` (docs/water.md "Presentation") — fish in the water, cosmetic and reactive: `Shoal` (kept
  in `fx::Fx`, so `app.rs` owns it; `Fx::shoal`) holds a school per `fish_school_cells` of each
  lake's deep cells, rebuilt on a new round, and is stepped on the round's clock (`Game::time`,
  split into ticks) from `Fx::observe_events`, so a lockstep and a replica swim alike. Fish go
  waypoint to waypoint between neighbouring deep cells (a diagonal only through a 2x2 of deep
  water), toward their school's hashed spot of the lake, and dart away (`Scare`, `scares`) from live
  hulls, shots in the air, hits, ricochets, laser beams, blasts and wrecks. Streams carry the odd
  fish down their current (`stream_fish`, pure in the clock). Every choice hashed (`pyro::unit`), no
  RNG, nothing on the wire; none under ice. `compose` rasterises one fish onto 2 px blocks in
  sixteen headings (`FISH`: the palette's `BLUE_DEEP` back, a fainter swinging tail fin, a
  `BLUE_PALE` glint) at `fish_opacity`; `render/fish.rs`'s `draw_fish` paints them in
  `paint_field_lit` straight after the floor. Knobs: the `fish` tuning group. Tests: `fish::tests`
  (`preview`, ignored, writes `target/fish-preview.png`).
- `shockwave.rs` — `Game::shocks` is a list capped at `SHOCK_MAX` (the shader's uniform-array
  length), resolved in one blit that accumulates every ripple; eviction by `remaining()`.
  `render/shockwave.rs` is `RippleFx`, the compiled shader and its uniforms, compiled once; every
  ripple is measured in `RIPPLE_FRAME`, the standard field (the speed, width and strength rows are
  in its heights, so a ring is the same size in world pixels on every map and a flash stays inside
  its quad), and `RippleFx::set_view` puts the scene target in that frame (`viewUv`/`viewUvSize`,
  `Camera::ripple_view`) with every centre measured from the target's corner (`ripple_uv`,
  `RippleFx::uv_of`), so the numbers stay small on any map. Camera shake sums the ripples and snaps
  to whole 2 px blocks; `camera_shake(shocks, view, t)` scales each ripple by `shake_reach` from the
  view a followed or pinned screen shows - full within `camera_shake_margin_px`, gone
  `camera_shake_fade_screens` screens past it (the `fx` group); `None` (an arena) is the plain sum,
  each split half shakes by its own view, and nothing shakes or bends under reduced motion;
  `screen_fx_intensity` scales flash, ripple and shake together. Shaders: `static/*.fs` with
  hand-ported GLSL ES 100 twins in `static/web/`.
- `frame_stages.rs` — where a frame's time goes, in a dev-tools build (every call is empty without
  it): `stage(name)` guards in `app.rs` and `Game::render` (`sim`, `fx`, `pictures`, `lights`,
  `world`, `post`, `chrome`, `swap`) and `frame_done` at the top of the loop, kept as averages over
  about half a second; read by `status.frame_stages`, the line under the left cluster with
  `ui_frame_stages` on (the `ui` group, so a PR preview's tuning panel shows it on a phone) and a
  phone's `FrameStats` log line. Processor time issuing each stage; the GPU's lands in the swap.
  `render/batch.rs` is `RenderBatch`: rlgl's batch raised to the desktop's 8192 quads on the GLES
  builds (the web, iOS, Android; ES 2's own is 2048), loaded after the window opens and held by the
  frame closure.

## HUD and chrome

- `hud.rs` (docs/hud-and-builder-layout-design.md, docs/large-maps-follow-camera.md §8) — the HUD,
  its headless half: `HudModel::gather(game, local_seat)` once per frame, the shared colours and
  sizes, and every button and dialog rect the hit tests read. **Play and online draw no bar**: the
  HUD is two corner clusters inside the safe area, laid out in UI points (`UiFrame`: `scale` window
  units per point, `screen`, `area` - the safe area less `UI_EDGE_PT` -, `touch`;
  `to_ui`/`to_window`), and the builder keeps its bar (`editor::chrome::Bar`, in UI points too);
  `UiFrame::hints` (`Hints::Keys`/`Touch`, `Hints::follow`) is the input the chrome's words name.
  **`corners(&UiFrame, &CornerShape) -> Corners` is the one geometry** the painter
  (`render::hud::draw_corners`, fixed slot tables pinned by `corner_tests`), `app.rs`'s hit tests
  and the dev server read (`Corners::hit`/`buttons`/`keep_out`/`chip`): the left cluster is the
  vitals, one row as tall as the right cluster's (`Corners::row_h`, a button's height, so the two
  corners share one line; every plate along the top - each block's and the right cluster's - starts
  at one top and is one row tall, a block growing downward by a lamp row - `Corners::lamp_row` -
  only while it holds something, the lanterns of a dark round (`PlayChrome::lanterns`) or a heat
  shield's gauge on a seat the blocks show (`PlayChrome::heat_shield`, `hud::heat_shield_up`), the
  right cluster never; laid out at full size and, on a window too narrow for the one row, all of it
  drawn smaller by one factor, `Corners::scale` - the painter draws `Corners::unscaled` under an
  rlgl transform, everything else reads the scaled rects) (health number and gauge, what the trigger
  fires with what it has left - the one special weapon carried in its accent, else shells,
  `WeaponSlot::of` -, the speed and shield gauges - each readout beside the symbol of the crate that
  fills it, `pickup_glyphs.png`, dim while it is empty; a couch's player 2 block beside or under
  it), the right cluster the level button (`LEVEL N`, `LEVEL_BUTTON_W`, opens the level select) or
  the free-play mission word with the wave count, the enemies and the frog gauge under the frog
  pack's symbol, then the `CornerButton`s (`BUILD` or, online, `LEAVE` - `Session::leave_online`,
  the way out with no keyboard -, left of the mode button a local round's pause icon -
  `CornerButton::Pause`, the P key's `Input::pause_pressed`, two bars or a play triangle by
  `PlayChrome::paused` -, players or `RESTART` on a keyboard-less build, `ONLINE` where
  `ONLINE_AVAILABLE`) and a room's seat chips between the numbers and the buttons - all one row on
  every window. The minimap's slot (`Corners::minimap`, from `CornerShape::minimap` =
  `PlayChrome::minimap`, its size in points): its own plate under the right cluster, flush right,
  shrunk to the room left and left out under `MINIMAP_MIN_PT` or where it would reach the left
  cluster; it is no button, `keep_out()` (a `Vec`) holds its plate, so no arrow lands and no touch
  steers or fires there, and `covered` fades it with the right cluster. **`HudLayout`** picks the
  block's shape: `One` and `Two` for a couch, `Compact` for three or more on a couch or any room of
  two or more - the block is the *local* seat's (`PlayChrome::seat`) and every other seat a
  `SeatHud` chip, numbered, in that seat's `tank::team_color`, a wreck greyed in place, built from
  the seat count, never from who is alive. The status, build-stamp and dev lines sit under the left
  cluster. **Fade** (`Fade::step`, `action_marks`, `WorldOnScreen`, `covered`): a cluster drops to
  `ui_fade_opacity` while a tank, shot or blast drawn in the camera's view lies under it.
  `PlayChrome` is what `Game::render` draws around the field (`hud` says whether the corners are
  drawn): the buttons, the dialogs, an online round's `status` line, its `countdown_label` (the end
  screen counts down to the lobby rather than to a restart) and the `lobby::LobbyView`. The dialogs
  (`leave_dialog_rects`/`players_dialog_rects`), the end screen and a level's banner are centred in
  the chrome's area in points (`centred_in`), their dims over the whole window; banners are
  `banner_size` (never under `BANNER_MIN_SIZE`, 48). The end screen (docs/levels.md):
  `PlayChrome::level` (`LevelBanner`: number, count, title) and `PlayChrome::result` (`ResultView`:
  `RoundStats`, the seat count, a level's `ResultButtons`/`NextLevel`), laid out by
  **`result_layout(area, view)`, the one geometry** the painter (`render::hud::draw_result`) and the
  hit tests read - a level's buttons in one centred row, `LEVELS` (`RESULT_LEVELS_W`), `PLAY AGAIN`,
  the way on, the one the screen counts down to carrying the count in its label
  (`ResultButtons::countdown`: `NEXT LEVEL IN 3` / `PLAY AGAIN IN 3`, none after the last level's
  win) - pinned by `hud_tests` inside the smallest area (704 x 336 pt), an iPhone's safe area and a
  1080p monitor's; `clock_text` is `m:ss`. `PlayChrome::level_button`/`levels` carry the level
  button and the `level_select::LevelSelectView`.
- `indicators.rs` (docs/large-maps-follow-camera.md §7) — what is off screen, headless like
  `hud.rs`: one seat's edge arrows, lane heads-up, hit arc, last-seen markers and gate flashes as
  plain values (`Indicators`), and `picture`, the blocks and labels a screen paints of them
  (`Picture`, pure, no RNG, pulsing on the round clock; painted by `render/indicators.rs`'s
  `draw_indicators`). `Awareness` (one per seat on a screen) remembers across frames on the round
  clock, reads a local round or a replica with the same code, never writes it, draws no RNG and
  walks in owner-slot order: `observe_events(game, seat)` after every step (`Fx::observe_events`'s
  discipline), `gather(game, seat, &ViewFrame) -> Indicators` once a frame. `ViewFrame` is the view
  as plain values (the world rect shown, points per world pixel, the inset rect, keep-out rects);
  `Awareness::frame(&Scene, &ViewFrame, &Tuning)` is every rule, which the tests drive without a
  `Game`. Arrows are cast from the seat's tank to the inset rect and slid clear of the keep-outs
  (`cast`, `slide`), sized and faded by screens away, merged within `indicator_cluster_pt`, at most
  `indicator_max_arrows` (lane threats, then teammates and frogs - never merged, never dropped -,
  gates, the nearest enemies; the rest folded into a count per edge). `lined_up` is the AI's aim
  (alignment and range) without the sight box, on purpose: the box is always on screen, so an
  arrow's warning is the heads-up that an off-screen enemy is lined up and closing in. `concealed`
  is the one tall-grass rule for enemies (no drawing hides a tank in grass). No event names a shot's
  shooter, so the hit arc reads back the line of the shot at its `Hit` point. `ScreenAwareness` is
  what `app.rs` keeps: an `Awareness` per local seat on the screen, fed every step's events, and a
  picture composed once a frame. Arrows are notched arrowheads of whole 2 px blocks at the eight
  compass headings (`arrow_cells`; free rotation read as a smudge that crawled as a target drifted),
  rimmed near-black, tips on their edge points: hostile red, a teammate's `team_color` with
  `P2`..`P8`, the FROG gauge's green with the distance in cells (the enemy frog rimmed red), gates
  amber blinking at `indicator_gate_blink_hz`. A lane warning is a ring pulsing at
  `indicator_pulse_hz`, dark red while the aim settles and bright once it has, the arrow swelling by
  `indicator_pulse_swell`; on the shot the arrow turns white on a white disc. Counts and `+N` sit on
  dark plates in the default font at whole multiples of its size (`label_font`, at least 8 pt). In
  the world (through `on_field`): corner marks where an enemy was last seen (a hollow arrow at the
  edge when that spot is off screen), a blinking frame round an on-screen gate, and the hit arc
  round the seat's tank (`indicator_hit_arc_px`/`_degrees`). A couch screen shows the first seat's
  arrows plus each seat's arcs, teammates and warnings (`shared_arrows`). Point sizes become bitmap
  pixels at the window's scale (`in_points`, `ViewFrame::of_camera`), and touch screens keep arrows
  off two `thumb_rests` (`indicator_thumb_*`, `POINTS_PER_MM`). A split half is
  `ViewFrame::split_at(at, normal)` (`Beyond`): past the divider is off screen, and its arrows sit
  `indicator_inset_pt` inside the divider; `ScreenAwareness::pictures` gives one picture per half,
  and the world marks are drawn in each half's world. Knobs: the `indicators` tuning group. Tests:
  `indicator_tests`, `picture_tests`.
- `minimap.rs` (docs/large-maps-follow-camera.md §7, §9, §15) — the map at a glance, headless like
  `indicators.rs`; `render/minimap.rs` paints it. `Minimap` is a `BlockImage` of one texel per cell
  (texel `(col, row)` is the cell `map::world_to_cell` names, so `source` starts half a texel in), a
  palette step per `Class` (ground, grass and road by `Theme`, water by `ground::Depth`, gate,
  brick/iron/wood/glass, props, trees, towers by side); `of_map` (the builder, depth from
  `GroundGrid::depth`) and `of_round`; `repaint` patches only the texels that changed under a new
  stamp, so `BlockTexture` uploads only them - the builder's per stroke, and `RoundMinimap::sync`
  (`app.rs` keeps one) bakes once per round (seed, map, sky, or the frame counter going back) and
  patches when the live tile count drops. `Marks::gather`/`picture`: the view rect, the live seats
  in `team_color` (the screen's own rimmed white, the others numbered in a round of two or more),
  the frogs (the enemy frog rimmed red), gates flashing while a wave rolls in, and only the enemies
  in `indicators::ScreenAwareness::shown` (`Indicators::in_sight`, the arrows' concealment rule) -
  never the world; terrain, not light. `MinimapRules` (the `minimap` tuning group): `minimap_show` 0
  never / 1 not on phones / 2 always, `is_phone` (a short side under `minimap_phone_short_pt`, or
  under `minimap_phone_short_mm` where `Screen::mm_per_point` is known - on a desktop that is the
  window's size), `size_pt` (the map's shape in `minimap_width_pt` x `minimap_height_pt`, snapped to
  half points where that keeps three quarters of the fit). `Marks::second_view` outlines a split's
  second half, or the establishing zoom's. Tests: `minimap_tests`.
- `lobby.rs` (docs/online-coop-prd.md §4.10) — the room screen, headless like `hud.rs`: hosting and
  joining without a command line. Five faces (`Stage`), derived from whether a room has been opened
  and how far along it is rather than stored, so the screen cannot disagree with the client -
  `Start` (map and mission steppers over `map::SHIPPED_MAPS` and the three missions, then `HOST A
  ROOM`/`JOIN A ROOM`), `Code` (the entry), `Waiting`, `Room` (the code, its QR, the seats, `I'M
  READY`/`START`/`LEAVE`/`KICK`) and `Closed`. The `Room` face is where a round comes back to: with
  `RoomView::ended` set it carries the outcome as its line and the host's action reads `REMATCH`,
  dead until every other seat has readied again. A seat row's name is cut to `LOBBY_SEAT_NICK_W`,
  which holds any anonymous name whole. The seat column draws `LOBBY_SEAT_ROWS` (4) rows and counts
  the rest as a `+N MORE` line, so a full room of `MAX_SEATS` reads as four rows and `+4 MORE`, and
  a kick button exists only for a row that is drawn. `RoomView` is the room as a plain snapshot
  (`RoomView::of` off an `OnlineRound`), so the tests need no socket; `LobbyView` is the painter's
  plain data, the `HudModel` pattern; `LobbyAction` is what the screen asks `mode::Session` to do.
  **`button_rect` is the one geometry table** the drawing and every hit test read, and
  `Lobby::buttons` the one list, so a button that is not drawn cannot be pressed and a disabled one
  is not hit (`START` before the room can start, `JOIN` before five characters). The panel is a
  fixed 704 x 336 in UI points centred in the chrome's area (the rect functions take the area) - no
  button moves under a finger, and it is laid out in points rather than in a field's pixels. The
  **code entry** takes taps on twenty keys (`net::rooms::CODE_ALPHABET`, two rows of ten) and, where
  `KEYBOARD_AVAILABLE`, typing - which accepts any letter or digit, since the room server is what
  decides which letters name a room and a mistyped code deserves its refusal. Every button is at
  least `LOBBY_TOUCH_MIN` (`hud::UI_TOUCH_PT`, 44 pt) on both sides, pinned by `lobby_tests`, whose
  taps round-trip through UI frames from a desktop to an iPhone's safe area. `render/lobby.rs` is
  the painting.
- `qr.rs` — the QR of the room's join link (`net::rooms::join_url`), encoded here rather than by a
  crate: the one thing the game encodes is a URL of at most a hundred characters, which is byte mode
  at level L in versions 1 to 5 - all single-block, all one alignment pattern - so the whole encoder
  is the bit stream, one polynomial division over GF(256), the module placement, the eight masks and
  their penalty scores. `Qr::encode` picks the smallest version that holds the payload; `qr::draw`
  paints it through `canvas::Canvas::fill_rect` as whole blocks at an integer scale (`scale_for`),
  so it is crisp at the game's resolution and a test renders it on a `CpuCanvas` with no window.
  Tests: the structure a scanner looks for, a read-back of the grid (format bits, mask, data walk,
  Reed-Solomon syndromes) and a pinned module hash.

## View and cameras

- `view.rs`, `touch.rs` — bitmap-to-screen mapping (`fit`, `fit_capped`, `to_bitmap`; the blit
  itself is `render::view::present`), the presentation **`Camera`**
  (docs/large-maps-follow-camera.md §6, §11: the world rect the field area shows - `whole` an
  arena's (`is_whole`, the view whose window margins show the world past the field), `zoomed` a
  pin's (a magnified view on the 2 px grid), `following(field, corner, size, scale, device_scale)` a
  followed view's: the corner floored to the block grid, the rest an `offset` in whole device pixels
  applied at presentation, the scene target one block bigger (`margin`, `follows`) -, `scale`,
  `target_size`/`target_rect`, `rect`/`to_world`/`to_view`/`dest` through the offset, `ripple_view`,
  the ripples' frame, and `cull`, the view grown by `CULL_MARGIN_PX`, `None` for the whole field;
  `render/view.rs` builds its `in_target` (pass 1) and `on_field` (the world onto the bitmap's field
  area, through the offset) `Camera2D`s, and `present_world` puts a followed view's world on the
  window shifted by its offset). `View::fill` is `fit` with the centring offset rounded.
  `FollowFrame::new(Screen, window, Seating, SightBox, &ViewRules)` is `framing::frame` (play draws
  no bar over the world), `Layout::bare` of the view and `View::fill`, so nothing is letterboxed
  from 4:3 to 2.4:1; `device_scale(framebuffer)` is what the offset rounds to and the touch scheme
  (a floating 4-way stick on the steering half of the field - the left, or the right under the
  `touch-steer-right` cargo feature (`TOUCH_STEER_RIGHT`, a build-time choice with no runtime
  switch) -, its origin trailing the thumb on a `touch_follow_radius_pt` leash so a change of
  direction is always a short slide, the axis switching `touch_axis_switch_deg` off the held one,
  the three numbers carried as a `StickRule` so a test never touches the table; tap/hold to fire on
  the other half, a HUD cluster claimed (`set_keep_out`), and a touch that pressed a button over the
  field `claim`ed - it neither steers nor fires until it lifts, so the tap on `NEXT LEVEL` does not
  also skip the next banner; the builder and the lobby claim every touch they see and the dialogs
  their press, so a finger on PLAY or START is nobody's shot in the round it starts; everything in
  UI points (`hud::UiFrame`: the touches through `to_ui`, the area, the keep-outs from
  `Corners::keep_out`), so the stick is the same size on a phone and a desktop: `stick_drawing`
  (`StickDrawing`) is the base at the origin the rule measures from, at least `BASE_MIN_RADIUS_PT`,
  and the knob, `KNOB_RADIUS_PT` (both at least 44 pt across), which `TouchScheme::draw`, the one
  `render`-gated item, paints under the UI camera with the hint at `HINT_TEXT_PT` at the area's
  quarters). Both unit-tested headless.
- `framing.rs` (docs/large-maps-follow-camera.md §3, §4, §15) — how much world a screen shows,
  headless and pure, the reference for the rules a follow camera frames a field map by. `MapClass`
  (`Arena` up to `ARENA_MAX_CELLS`, 36 x 18, shown whole by `view::View`; `Field` past it;
  `by_size`, overridden by the map's `view` key through `MapFile::class`) is the map's, never the
  screen's. `frame(Screen, Seating, SightBox, &ViewRules) -> Framing`: the same area on every screen
  (`view_area_cells`, 578) in the screen's shape clamped to `view_aspect_min`..`view_aspect_max`
  (4:3..2.4:1, bars past them), snapped under `view_fine_ppi` (360) to the nearer whole-block scale
  and outward wherever that would hide the sight box, and for `Seating::Local` on a screen of known
  size stepped out a block at a time while a tank stays at least `view_local_min_tank_mm` (25) wide
  and the view at most `view_local_max_cells` (900: 40 x 22.5 on 16:9); a room never steps.
  `Framing::shows`/`room_outside` are the sight box on screen and how far a look-ahead may lead.
  `ViewRules::current()` reads the `view` rows once a frame; tests use
  `ViewRules::of(&Tuning::DEFAULT)` and reproduce the doc's device table. `frame_under_bar(Screen,
  Bar, ...)` frames the world under a bar whose height depends on the scale, keeping the outward
  step of a two-step snap cycle (with no bar it is `frame`, which is what play, with no bar over the
  world, frames through). Under `view_fine_ppi` the zoom snaps to whole-block scales, so a small
  low-density desktop window can show more than the area (a 40-wide map whole at 852 x 393 and 96
  ppi); a phone or tablet is fine and keeps the exact zoom.
- `follow.rs` (docs/large-maps-follow-camera.md §6) — the follow camera for field maps, headless and
  presentation only (reads a local round or a replica, never writes it, no RNG). `Seats::of` reads
  seats into `SeatTank`s (position, facing as an axis via `cardinal`, velocity, `base_speed`,
  `live`). `Follow::update(&Seats, dt, &Stage, &FollowRules) -> Shot` moves the view by the time the
  round advanced and answers the exact corner, the `ShotKind` (Seat; Shared for a couch pair while
  both sight boxes fit the view as far as the field reaches, else Split; Spectating after
  `camera_spectate_delay_seconds`; Nobody), the lead and the cut: a dead zone
  (`camera_dead_zone_px`), a look-ahead along the facing (`camera_lead_at_rest` of
  `Framing::room_outside` less the dead zone, all of it at top speed, swinging at the pace of
  `camera_lead_ease_seconds`, a reversal held `camera_lead_reverse_hold_seconds`), an exact
  critically damped `spring` (`camera_spring_seconds`) with velocity feed-forward toward a goal
  eased into the field over `camera_edge_ease_px`, then hard rules: the sight box stays on screen
  (`Shot::boxes_in`), then the field, a short axis centred. It cuts on a followed seat's
  `Teleported`/`TankEntered`, on `RoundStarted` or a frame counter going back (`observe_events`,
  after every step), on a jump, on a far hand-over and on `Follow::cut`.
  `CameraReport`/`FollowReport` are what `app.rs` hands the dev server. **The couch split**:
  `Shot::split: Option<Split>`, one `Tracker` per half - a Voronoi split along the bisector of the
  tanks as the halves show them (`Split::at`/`normal` in view pixels, `apart`, `in_second`); each
  half keeps its own seat's box at the local zoom and opens from the point that holds that box
  (`split_aims`/`entry`/`holds`), so neither half jumps; the split closes once the pair fits and the
  halves meet (`MERGE_PX`/`MERGE_SPEED`), chasing the shared centre with no dead zone or edge ease;
  a seat going down hands its half on without a cut; `FollowReport::second` is the second half's
  camera. Tests: 23 in `follow::tests` (dead zone, lead cap, the box under a slow spring, the spring
  at 30/60/144 Hz, reversal hold, clamps, edge ease, cuts, spectating, the couch).
- `establish.rs` (docs/large-maps-follow-camera.md §6) — the establishing shot: a local round on a
  field map whose view shows part of it, opening behind its mission banner, on a field no more than
  `MAX_TEXELS` (4096) a side (and on a phone or a tablet no more than `EMBEDDED_MAX_AREA`, 2048 x
  2048 texels, `establish::fits`), shows the whole map for `camera_establish_hold_seconds`, then an
  eased zoom over `camera_establish_zoom_seconds` (a cut under reduced motion) inside the banner; a
  steer or a shot ends both. `Establish::update` runs on `Game::intro_timer`, so a lockstep replays
  it; `Mapping`/`between` is the zoom, a homothety about the one point both ends share, and
  `zoom_split` zooms a couch pair that opens apart into its split. No shot without an intro, in a
  room's round, or on an arena.
- `motion.rs` (docs/large-maps-follow-camera.md §6) — the one motion switch: the `reduce_motion` row
  (camera group: 0 follows the platform, 1 full motion, 2 reduced) resolved by `reduced()` against
  the platform's answer, which `app::run` reads once at startup (`set_platform`): iOS's
  `UIAccessibilityIsReduceMotionEnabled` (`app::ios::reduce_motion`) and the page's
  `window.bbMotion` (`page_string`, `from_page`); Android, macOS, Linux and Windows give none, which
  is full motion. Reduced motion means no camera shake, no kill ripple bending the whole screen and
  an establishing shot that cuts instead of zooming; the follow camera, the split, the arrows and
  every effect of the round stay. Tests use `reduced_by`, never the global; `status.camera.motion`
  reports it.
- `margin.rs` (docs/large-maps-follow-camera.md §13 item 7) — an arena's margins, headless:
  `MarginFrame::of(view, layout)` is the world the window shows round the field (rounded out to the
  block grid; `None` where the bitmap fills the window), `parts(into)` the window past the field as
  at most four disjoint rects reaching `into` px under the field's edge, `Margin` the ground and
  shade made for a round's floor and reach (`fits`; reach rounded up to 4 cells, at most `MAX_CELLS`
  96, the plateau past it). Nothing stands or plays there; the simulation, the field's rect, `View`
  and every pointer are untouched. `render/margin.rs` is `MarginFx`: into targets of the window's
  world, the ground once, the shade part by part, then the round's sky - `render::weather`'s passes
  over the parts (`PassView::parts`) under a 1x1 light map of the ambient with the margin grid's own
  cell mask (`CellMask::origin`, the shader's `cellOrigin`), or the plain path's blocks and
  `multiply_ambient` - copied onto black like the field's bitmap; `Game::render` makes it only while
  `Camera::is_whole()` (never a followed or pinned view, never the builder), and
  `render::view::present_into` fills the window with the backdrop, then draws its parts round the
  bitmap, so the two meet colour for colour. The margins do not shake with the field's camera shake.

## Builder

- `editor/` (docs/game-editor-fusion.md, docs/map-editor-design.md, docs/large-maps-follow-camera.md
  §9) — Build mode, compiled into every build. Edits the authored `MapFile`, never live entities.
  Input is a plain `BuilderInput` that `app.rs` and the dev server fill the same way, so a tool
  lands on the same hit test a finger does and tests run headless; `editor/render.rs` (feature
  `render`, a child module because it draws the builder's private state) is the chrome:
  `MapEditor::render`, the bar and popup painters, the tool and brush icons, `EditorTextures` and
  `ThumbnailTextures`, the drawing's own slot offsets and labels, and `bar_tests`. `Tool`s in bar
  order (`TOOLS`, 52 with the eraser and the select tool; `name`/`parse` are the dev-server
  spelling) in five categories that remember their current tool; singletons (start, start2, frogs)
  move. A **stroke** paints once per cell crossed (a drag that skips cells between frames fills the
  gap edge to edge, `drag_to`; the tools' `stroke` takes its cells as given) under the
  **toggle-erase rule** decided on its first cell: right button, the eraser, or a first cell already
  holding exactly the brush's object makes an erase stroke, anything else a paint stroke that never
  erases. **The brush's shape** (`Shape`, BRUSH's list; the cells are `editor/brush.rs`): PEN is
  that stroke; RECT fills the rectangle a drag draws on its release (`rect_stroke` while held; the
  eraser or the right button clears it, Escape takes it back); FILL floods the pressed cell's region -
  the cells joined edge to edge holding exactly what it holds - under the toggle-erase rule and
  refuses one past `builder_fill_max_cells` with a status line; SCATTER lays the brush's object on
  `builder_scatter_density` of the empty cells within `builder_scatter_radius_cells` of the drag,
  picked by `brush::scattered` (a hash of the cell and a per-session stroke counter, never `rand`),
  and erasing thins the brush's object (anything under the eraser). RECT and SCATTER never toggle; a
  singleton's brush paints with the pen whatever the shape. **The select tool** (`Tool::Select`; the
  data is `editor/select.rs`): a drag draws a `CellRect`, a drag from inside it lifts its cells and
  carries them, and the strip under the bar (`chrome::Strip`: COPY, CUT, PASTE, the flips, DELETE, +
  STAMP, STAMPS, or PLACE, the flips and CANCEL while a paste ghost stands) and Ctrl+C/X/V and
  Delete act on it; a paste or a stamp is a `Ghost` that follows a hovering mouse until a press puts
  it down (the right button takes it away), or that a finger drags by the cells it moves, taps
  elsewhere to move and taps to put down. A `Clip` is transparent; a start or a frog moves with a
  moved selection and a paste places one only where the map holds none, so it never leaves two;
  gates and portals are carried like any cell; cells past the field are dropped. Stamps are
  `select::SHIPPED_STAMPS` (`maps/stamps/`: fort, bunker, river bend, worded `stamp-<name>`) then
  the ones kept this session, in memory on every build. Every stroke of any shape, move, paste,
  flip, cut, delete, settings field, load and reset is one `EditStep` on `history.rs`'s `UndoStack`
  (`UNDO_DEPTH` 200, a new edit drops the redo branch). `dirty` - the cells, settings or size differ
  from the baseline (as loaded, opened or last saved) - is compared once per edit on kept indexes.
  **The kept index** (`editor/index.rs`, `MapEditor::cell_index`): the cells in `iter_cells`'s order
  with the portals, the singletons' cells, the tower kinds on the map and the cells' `digest` (a sum
  of a hash a cell, kept up through every change); an edit of cells takes its cells in
  (`cells_changed` -> `CellIndex::update`, a binary search a cell, worked out whole past 64), any
  other edit drops it until the next read; the canvas walks only the rows its cull spans
  (`CellIndex::near`) and the singleton badges read it, so no frame walks `MapFile::cells`
  (`a_dense_maps_frame_timing`: 0.03 ms a frame at FIT on a filled 250 x 250 map in release).
  `UndoStack::undo`/`redo` hand the moved step back by reference (`last_undone`/`last_done`), never
  a clone. One popup at a time; a press outside it - the Save prompt's too - closes it and is
  consumed. **The chrome is on the window in UI points** (`editor/chrome.rs`, the one geometry table
  the painter, every hit test, `status.builder.buttons`/`click` and `bb_ui_json` read):
  `BuilderFrame` (the canvas's bitmap with no bar in it, the view that puts it under the bar - the
  canvas between the safe area's sides, down to the window's bottom -, the `UiFrame`); `Bar::of`
  lays the bar along the safe area's top from its width - a mouse's slots at a point a pixel on a
  desktop in a 40 pt bar at the HUD's scale (`HUD_BAR_HEIGHT`, a play corner plate's height, its
  boxes a HUD button's 32), every button 44 pt both ways on a touch screen, hit rects reaching
  `EDITOR_BAR_HIT_SLACK` above and below - and short of room drops the BUILD label, then the map's
  name (the status line names it), then folds the five categories and BRUSH into one TOOLS button
  and its palette (BRUSH's rows its last row; a category longer than the room is wide runs on into a
  second row, the cells shrinking to the room's height but never under 44 pt), never shrinking a
  button; `PopupLayout` hangs the popups under the bar, a long list running into a second column and
  the CHECK panel, the Load list and the STAMPS list (at most `LOAD_VISIBLE_ROWS`) turning pages by
  a pager row and the wheel where their rows do not fit; the MAP panel (`MapPanel`, its four groups
  `MapTab` - ROUND, TANKS, FIELD, SKY - of `MapField`s as choices, steppers and sky tiles,
  `PanelButton` its presses) shows every group at once as sections where the room holds 884 x 432 pt
  and one at a time behind a rail of tabs elsewhere, RESET MAP the rail's last slot, never paged,
  ROUND keeping its wave rows' room so a spawn change moves no other group; the select tool's strip
  stands at the top-left of the room under the bar, never beside the selection, so it never moves
  under a finger or covers the cells being worked on; every button of the bar, the strip and the
  Save prompt is drawn alike - one box (`chrome::button_box`: its slot less `BOX_GAP`, `box_height`
  tall, a 2 pt square outline), every word at `chrome::BAR_SMALL_TEXT` (14 pt with a mouse and on
  touch, held to `label_room` in every language) and one colour rule (`render.rs`'s `Face`: idle,
  dim, washed while its popup is up, an amber outline for the brush in force, amber outline and word
  for PLAY, PLAY HERE, PLACE and SAVE). The Load list's rows show each map's thumbnail and its size
  in cells (`load_picture`, `load_text`, `fit_picture`): `editor/thumbs.rs` makes the map's minimap
  (`minimap_of`, a texel a cell) only for the page shown, `MADE_PER_FRAME` a frame, and keeps it by
  name and an FNV-1a hash (`map::fnv1a`) of the text `map::map_source` reads, checked again each
  time the list opens; `ThumbnailTextures` uploads each once and frees them the frame the list
  closes. `BuilderInput` is in window coordinates. Tests: `chrome_tests`. `save` keeps the canvas
  (`map::saving_available`): in the player's store with map modding on (`mapstore.rs`), else
  `maps/<name>.toml` on native; the Save prompt saves on Enter or its SAVE button
  (`chrome::save_button`). With modding on the canvas is kept on its own too (`autosave`:
  `builder_autosave_seconds` after the last edit, and on `leave`), and FILE's REVERT MAP
  (`revertible`) asks the session's question. **Its own camera** (`editor/camera.rs`,
  docs/large-maps-follow-camera.md §9): `BuilderCamera` (FIT or `Zoom{scale, center}`) over a
  `Viewport` (the field, the canvas area, a `CanvasScreen`); `view` is the `view::Camera` every draw
  and hit test goes through (`MapEditor::cell_at`/`world_at`). At FIT on an arena it is
  `Camera::whole`: the canvas is drawn into the field's own bitmap and presented through the frame's
  view at its round's capped scale, letterboxed under the bar, the very picture play shows; anything
  else draws into `render::BuilderScene` at a texel per world pixel (`scene_plan`, halved to stay
  within `SCENE_MAX_TEXELS`, 4096 a side), the world past the field in the canvas fill, scaled onto
  the canvas (`window_mapping`). A field map's canvas is the window's shape under the bar
  (`chrome::canvas_frame`). Zoom steps in whole blocks (0.5 device px multiples) on a coarse screen
  and smoothly on a fine one, from FIT up to `builder_zoom_max_cell_pt`; the wheel zooms at the
  cursor, middle drag or Space + drag pans (the right button still erases), `+`/`-`, the arrows and
  the bar's FIT button work too, and BUILD opens on what play showed (`Session::play_view` ->
  `look_at`). **Touch is raw** (`BuilderInput::touches`, `editor/gesture.rs`): one finger paints
  once past `builder_touch_slop_pt`, two fingers pan and pinch about their midpoint (a second finger
  takes a stroke back, a pinch settles on whole blocks), a two-finger tap undoes and a three-finger
  tap redoes; where a cell is under `builder_paint_min_cell_mm` a tap zooms to
  `builder_tap_zoom_cell_mm` and a drag pans. A finger is the canvas's only where it lands on the
  canvas and no press of the chrome takes it (`lands_on_canvas`: a bar button's hit slack reaches
  over the canvas's top edge). `MapEditor::leave` (from `Session::play` - PLAY HERE and Tab too -
  and a level's start) closes the popup, takes back a half-drawn RECT and a select drag as Escape
  does, lands a freehand stroke and forgets every finger; the first `update` after it ignores the
  fingers already down (`entering`, `Gestures::ignoring`). A stroke - or a select tool's drag - held
  within `builder_edge_scroll_pt` of the edge, or past it, scrolls the view and goes on under the
  nearest canvas point. **The map's size** is the MAP panel's WIDTH/HEIGHT rows about an ANCHOR
  (`resize`, `Anchor::shift`, up to `map::MAX_SIDE_CELLS`) as one `EditStep::Resize` - presses fold
  while the panel is open, undo restores the dropped cells, the view shifts with the map. A cell
  edit repaints the ground around its cells (`repaint_ground`, given the edit's `CellChange`s); one
  that moves the floor - road, water or a wall - under more than half the map (`REBUILD_SHARE`)
  makes it whole again, as a load, reset, resize or theme does. **The navigator**
  (docs/large-maps-follow-camera.md §9): `navigator_rect` (bitmap px, the canvas area's bottom-right
  corner `NAVIGATOR_MARGIN_PT` in, sized like play's minimap in points and at most half the area,
  `None` at FIT on an arena and while an open popup's panel reaches its plate - a phone's MAP panel
  does -, where it steps aside) and `navigate`, one `BuilderInput` hit test before the wheel, the
  pan and the stroke - a press puts the view's middle there (`BuilderCamera::navigate`, from FIT at
  a tap's zoom), a drag carries it to the lift; `point_on_ui`/`on_canvas` leave its plate out, so
  nothing paints under it. `MapEditor::minimap()` is rebuilt with the ground and repainted with it
  (`repaint_ground` passes the cells `GroundGrid::repaint` answers), and `draw_navigator` draws it
  on the zoomed path only (`EditorTextures::minimap`). **Large-map tools**
  (docs/large-maps-follow-camera.md §9): after FIT the bar carries `CHECK` (`Popup::Lint`: a
  clear-check row, then `maplint::lint_map`'s findings errors first, seven a page; a row pans and
  zooms the camera onto its cells and outlines them, at least `builder_lint_jump_cols` x
  `builder_lint_jump_rows` cells, and FIX is one `EditStep` whose hit rect is the row's height
  (`LintLayout::fix`, the outline `fix_box`); re-linted the frame after each edit while open, about
  29 ms on the study map in a debug build), the clear flag (`SLOT_CLEAR`: green with the par once
  the canvas's revision is cleared, a tap opens CHECK) and `PLAY HERE` (`EditorAction::PlayHere`).
  `clears`/`revision()` (cached per edit)/`par()` - which works the revision out only where the
  canvas's `ClearKey` (its size, cell count and the index's digest) is a cleared revision's, so an
  edit away from one costs nothing on any map (a revision is about 0.3 s on a filled 250 x 250 map
  in release); SAVE writes the `[cleared]` stamp (`map_to_save`), `new`/`load`/`open` take a valid
  stamp in, and the canvas never carries one. **The loupe** (`loupe`, `loupe_rect`,
  `builder_loupe_cell_mm`): while one finger paints, draws a selection or carries one where a cell
  is drawn under that size, a magnified view of the cells under it stands above the finger (left of
  it near the right edge, beside it where there is no room above) with the finger's cell outlined
  (`finger_cell`: the stroke's last cell, a rectangle's corner, a carry's grabbed cell), at
  `builder_loupe_zoom` on the nearest whole-block scale, `builder_loupe_lift_pt` clear of the
  finger, drawn into `BuilderScene`'s second target; never for a mouse. Knobs: the `builder` tuning
  group.

## Online

- `net/` (docs/online-coop-prd.md §4.3, §4.4, §4.6) — the online co-op wire protocol, plain
  synchronous code with no sockets, threads or raylib (shared by the room server and every client,
  wasm included); it never reads or writes a `Game`. `wire.rs`: `IntentMsg` (the human-settable
  `Intent` fields only - `fire_aim_offset`/`slow` are AI-only and never travel - plus, with `owned`,
  the client's own hull pose in `x`/`y`/`dir`/`vx`/`vy`: stage 3, docs/online-coop-prd.md §4.14,
  `IntentMsg::with_pose`/`pose`), `Snapshot` (`acked` and `mailbox` per seat - the input tick
  applied and `net::mailbox::Mailbox::wire_state` -, keyed families sorted by key - `TankState`,
  `ShotState`, `MissileState`, `GrenadeState` (position, height and the fuse in hundredths, so the
  lamp's quickening blink never jerks), `DroneState` (an FPV drone in the air: ground point, height,
  heading, stage, halo slot, owner and lock), `FrogState`, `TileState`, `FireState` (with `lava`, a
  bomb's pool), `BonusPickup`, `CrateState` (a crate that is not whole: hurt, burning or spilled),
  `LampState` (the lanterns set down), `ZoneState` and `CraterState` (a rod's calls and craters,
  docs/rod-from-god.md), the volcanoes' shifts -, `RoundState` (the wave, the live and pending
  counts, the mission banner's, the `WAVE N` breather's and the end screen's restart countdown's
  time left, the outcome), `events`), `Welcome` (`WireOverrides` mirrors `LevelOverrides`), the JSON
  `Lobby` enum, and the quantisation helpers (positions quarter pixels as `i16`, velocities `i8` at
  4 px/s, headings `u8` of a turn, health whole points, timers tenths; only NaN collapses to 0).
  `events.rs`: `WireEvent`, the owned mirror of `simulation::Event` - `from_event` is an exhaustive
  match, so a new `Event` variant fails to compile until mirrored or put on `NOT_SENT` (the AI
  trace, `PhysicsQuarantine` and `Rerolled`); `LaserBeam` is how a laser reaches a replica at all,
  `net::apply` pushing the beam the event carries (one a leg, the lens flashing on leg 0 alone);
  `ShotTeleported` carries a shot's `ShotState` key so the interpolator draws its jump. The lobby's
  `Ended { outcome }` is how a round's end reaches a client, since the snapshots stop with the tick.
  `codec.rs`: `encode`/`decode` of a `Msg`, one `kind` byte then `postcard` (lobby: JSON).
  `delta.rs`: `delta`/`apply_delta` - per family the changed entries in full, short moves as `Moved
  {key, dx, dy}`, removals as keys; scalars as `Option`; events whole. `PROTOCOL_VERSION` (bump on
  any postcard layout, enum order, kind tag or quantisation change), `MAX_SEATS`. Measured sizes in
  the module docs. The two files that do touch a `Game` (docs/online-coop-prd.md §4.2, §4.5):
  **`encode.rs`** — `snapshot(game, acked)` fills every family from the round through
  `simulation/replica.rs` (tanks by owner slot with the chassis `row`, shots by the per-round
  `Shell::id`/`Bullet::id`/`Plasma::id` counter with their sprite `variant`, frogs, the pickup
  bitmask over `pickup_slots` plus bonus cells, only the tiles that differ from fresh in whole
  points, fires, `RoundState`, the frame's events; a tile death is an `ObstacleDestroyed` event and
  a `DESTROYED` entry cut from it, since the world drops a dead tile the same frame), `welcome(game,
  seat, roster, tuning_json, acked)` adds the map TOML, seed, `WireOverrides`, `enemy_count`, the
  unburnt `oil_cells` and the `dead_cells` a state snapshot cannot express; `wire_events` for a
  server accumulating the ticks between snapshots; `server_ms` is the caller's. **`apply.rs`** —
  `welcome(&Welcome) -> Result<Game>` builds the replica by `Game::init` on the same map, seed,
  overrides and roster-pinned chassis (so `init`'s rolls match), `strip_ai`, removes `dead_cells`,
  then `snapshot(&mut Game, &Snapshot)`: events first (`RoundStarted` re-inits on its seed unless
  the replica already stands in that round at frame 0, which is every welcome; `ObstacleDestroyed`
  removes the tile; `DrumLaunched` puts the drum in the air), then tiles (absent = fresh), tanks
  (spawned without `Ai`, body placed with `Physics::set_position`/`set_velocity`; the hull's facing
  snaps and the *drawn* angles stay put, for `tick_presentation` to swing across), shots by id,
  frogs, pickups, fires (a cell the snapshot stops listing burnt out, the one way a fire ever leaves
  the list, so the replica darkens the ground and spends the oil there), round scalars, `frame`, and
  the round clock as `tick * PHYSICS_FIXED_DT` - exact because a room server's round runs without
  the mission banner. Never `update`d, no RNG; what the wire omits is hashed (wreck art, shadow
  heights), a full timer (a flag) or the replica's own to run (`Game::tick_presentation` between
  snapshots). `simulation/replica.rs` is the surface both read/write through and `DrawableState`
  (`Game::drawable_state`, positions on the quarter-pixel grid, health in whole points, timers in
  tenths) is the definition of "the same picture" the round-trip tests in `apply.rs` assert on
  (authoritative round vs replica after every apply, with `tick_presentation` on the two frames
  between, late join at frame 600, re-encoding the replica gives the server's bytes).
  - **The client's socket** (feature `online`, on by default and off under `--no-default-features`,
    so the server and the probe carry no second WebSocket stack): `transport.rs` is the whole
    contract - `send(&[u8])`, `drain(&mut Vec<Msg>)` (decoding through `codec`, appending, never
    blocking) and a `ConnState` of connecting/open/`Closed{reason, requested}`; a message that does
    not decode is skipped, since a new `Lobby` variant needs no protocol bump. `Failed` is the one a
    browser hands back when it will not dial the URL at all - closed from the first frame, so the
    refusal reaches the lobby along the path a hang-up takes. Three implementations: `native.rs`,
    blocking `tungstenite` + rustls/webpki-roots on one std thread that flushes the outgoing queue
    then reads with a 5 ms timeout and talks to the frame over two mpsc channels (desktop, iOS,
    Android; `rustls` is a direct dependency only to pick the `ring` provider). **Every socket sets
    `TCP_NODELAY`** - the native client's here, the server's through `ListenerExt::tap_io` - because
    a sixty-a-second stream of small frames is exactly what Nagle's algorithm turns into
    round-trip-paced bursts, and no loopback test can see it (docs/online-coop-prd.md decision 16);
    a new transport inherits the rule. `web.rs`, emscripten's WebSocket API through FFI
    (`-lwebsocket.js` in `.cargo/config.toml`), the four callbacks pushing into an unbounded queue
    the frame drains - a hidden tab's backlog is caught up, never trimmed, or the delta chain
    breaks; `loopback.rs`, the rig's in-process pair with dialled delay, jitter and loss off a
    seeded `SmallRng` per direction, head-of-line like the socket it stands in for, drivable at a
    given `Instant` so its tests need no sleeping. `rooms.rs` is the one URL rule: host from
    `--rooms`, else `BONGBONG_ROOMS`, else `wss://rooms.bongbong.io`; **one server holds every
    room**, so hosting and joining both dial that host's own `/ws` and a code picks no path
    (`socket_url` takes no code; spreading rooms over several servers is deferred work,
    docs/online-coop-prd.md §4.8). It also holds `CODE_ALPHABET` and `CODE_LETTERS` (the twenty
    symbols and the five of them a code is, which the room server re-exports and mints from and the
    lobby's key grid offers - a test reads `server/src/code.rs` so the two cannot drift) and both
    ends of the invite: `join_url` writes `bongbong.io/j/CODE` for the QR, carrying a `--rooms`
    override as a query parameter so a scan reaches the same server, and `Invite::parse` reads one
    back off a page's URL for the web build, which has no command line - `/j/CODE` or `?join=CODE`,
    the `?rooms=` override completed to `ws`/`wss` by the page's own scheme (`Invite::rooms_host`).
    Nothing in it refuses a URL: a mangled link leaves a field unset and the lobby opens where a
    code is typed by hand. `client.rs` is `RoomClient` over any transport - `host`/`join`, one
    `poll` per frame draining `ClientEvent`s (`Created`, `Welcomed`, `Roster`, `Started`, `Ended`,
    `Snapshot`, `Said`, `Refused`, `Closed`; `Ended` puts the phase back to `Lobby` so a rematch is
    askable), `send_intent` holding fire `FIRE_HOLD_TICKS`, the seat, the code, the roster, and
    deltas applied onto the baseline so a caller only sees whole snapshots; **a ping nothing answers
    for `ROOM_SILENT_AFTER` (5 s) closes the connection** as "the room stopped answering" - a path
    can die without either end hanging up, and a socket left open on it shows a frozen round forever -
    timed from the ping rather than the last message, so a hidden tab that sent none is never closed
    for the silence it caused, and started again after a gap of more than `CLIENT_AWAY_AFTER`
    between the client's own polls (a suspended app, a debugger), so a live connection is not closed
    on the first frame back; a socket still unopened after `ROOM_REACH_WITHIN` (15 s) closes as "the
    room could not be reached", and the native dial times its connect and both handshakes
    (`DIAL_TIMEOUT`, 10 s) so a path that dies mid-handshake ends the socket thread instead of
    blocking it (a `Game` is none of its business - that is `apply.rs` and the caller's).
    `connect(host, identity, Target)` is **the one place a client socket is opened** -
    `--host`/`--join`, the lobby's `HOST`/`JOIN`, a page's own invite link and the dev server's
    `click` all come through it, so a button, a flag and a link reach a room the same way; it is two
    functions, the native thread's and the browser's, behind one signature. `server/tests/round.rs`
    plays a round through `NativeTransport` and `RoomClient` against the real server.
  - **The window's side** (docs/online-coop-prd.md §4.5): `round.rs` is `OnlineRound`, one
    `frame(intent, dt)` per rendered frame - poll, apply a `Welcome` (`net::apply::welcome`, the
    room's `tuning_json` staged first because `init` reads the knobs) or a snapshot, send this
    seat's intent, draw - plus the lobby's view of the seat (`roster`, `is_host`, `host_seat`,
    `can_start` - the room server's own three conditions, so the button is dead exactly when a press
    would come back refused -, `ready`, `kick`, `note`, `ended` - how the round the room just
    finished went, which is `mode::Session`'s cue to hand the window back to the lobby) and
    `status()`, the one line `PlayChrome::status` puts over the field *while the round runs*;
    everything before it is `lobby.rs`'s. The intent is paced at one packet per `PHYSICS_FIXED_DT`
    of real time, `app::StepClock`'s discipline: a frame that covered several ticks sends one packet
    per tick up to `SEND_CATCH_UP_TICKS` (a 30 fps phone still sends sixty a second, or the room
    starves every other tick and the sandbox falls behind the hull it moved), and a tap on a frame
    that sends nothing is carried into the next packet. Nothing is sent unless the client's phase is
    `Playing`: the lobby before a round and after one steers nothing, so a rematch never opens on
    the last round's poses. `AnyRound` boxes the transport, so `mode::Session` is one type whatever
    the round is played over. `interp.rs` is what makes the room's snapshot stream look like a game
    on a screen of any rate. **The picture is placed in ticks, the link is measured on the wall**:
    `ServerClock` takes each snapshot's `server_ms` (the room's wall clock at send) less its arrival -
    the largest over `CLOCK_WINDOW_MS`, followed both ways, its fall limited to `CLOCK_FALL_PER_MS`
    only while it settles after a stall that outlasted the window (a backlog that comes back in
    chunks extends a settling already in effect, up to `CLOCK_SETTLE_MAX_MS` from its first gap; a
    link that is merely gappy settles nothing), a reading past `CLOCK_SNAP_MS` above it a new clock -
    and the round's **anchor** (`server_ms - tick time`, the lowest of the last `ANCHOR_SNAPSHOTS`,
    or at once on `ANCHOR_RUN` equally late ones) turns it into tick time. The anchor is what moves
    when the room's schedule does - a waiting room's welcome at tick 0, a round start, dropped
    ticks, a pause - so a `Welcome` restarts it, and the link's clock only when its stamp is more
    than `CLOCK_SNAP_MS` off or the clock's newest reading is older than `CLOCK_WINDOW_MS` (a lobby,
    a rematch's wait); `server_ms` is unwrapped across the hub clock's `u32` wrap, and an anchor
    that jumps forward past `ANCHOR_HOLD_MS` holds the picture for as long as the room stood still;
    read as one clock, a lobby welcome made every round open a tenth of a second off, on seconds of
    extrapolation and `Interpolator` (the last `BUFFERED_SNAPSHOTS` and a playout clock: render time
    never goes back, steered toward the clock less the delay at a bounded rate - while it runs past
    the newest snapshot with the target behind it, bounded by the full `RATE_FAR` (10 %) rather than
    the error-ramped bound, without the deadband - jumped forward past `RENDER_SNAP_MS`, held where
    it stands while a new clock's target is behind it; only a round starting over or a `Welcome` -
    stamped with its arrival like every snapshot - restarts it). **The delay is sized from
    lateness** (decision 8): one measured interval plus one 60 Hz frame plus the 95th percentile of
    lateness - behind the envelope (re-based down when the envelope falls) plus how late the room
    sent the tick behind the anchor, isolated head-of-line stalls left out and ridden out on
    extrapolation -, floored at `online_interpolation_delay_ms` and capped at
    `online_interpolation_delay_max_ms`; `online_interpolation_adaptive` off pins it at the floor.
    `report()` (`InterpReport`, every field in `stats_json`) is what `status.round.interpolation`
    reads. Positions blend linearly on the quarter-pixel grid; a hull's facing and a shot's heading
    do not - they snap, being facts about a tick - and everything discrete is the bracket's near
    end's, whose events are handed over exactly once, on the frame render time reaches it
    (`Game::frame` is that tick, which is what makes `Fx::observe` fire once per snapshot); a
    snapshot handed over past `EVENT_STALE_MS` late (a hidden tab's backlog, a full buffer) keeps
    only the events `carries_state`. A hull that jumped (`Teleported`, `Placed`, or further than it
    could drive) is drawn whole at its tick and named in `Frame::snapped`, which `draw` lifts the
    tread-mark trail of before `tick_presentation`; a shot whose far end carries its
    `ShotTeleported` holds and is drawn at the exit the same way, its correction offset dropped.
    With nothing newer, hulls run on their last velocity for `HULL_EXTRAPOLATE_MS` then dead-blend
    to a stop, and corrections ease off as per-entity offsets; a tick behind the newest is the round
    having started over and the buffer starts again. `predict.rs` is stage 2
    (docs/online-coop-prd.md §4.12, `online_predict_own_tank`, live): the local seat's own hull run
    ahead of the room and pulled back by each snapshot. The sandbox is a whole `Game` built the way
    `apply::welcome` builds the replica, so the statics a replay steps against are the server's *by
    construction*; `Game::predict_seat` is the only thing ever called on it - `drive_tank` plus one
    solver step, no RNG, nothing ages, nothing fires. **A reconciliation applies the whole snapshot
    to the sandbox** through `net::apply::snapshot`, the same call the replica takes, so the sandbox
    is a *projection* of the server's world rather than one of its own - every other hull, a
    destroyed wall, a speed boost, all of it, by one already-tested path. Syncing only the own hull
    is what made the predicted tank drive through tanks that had moved and stop against tanks that
    were gone. The own hull is then placed at `acked` and every later input replayed, the difference
    carried as an offset that decays over `NUDGE_SECONDS` or is taken whole past `SNAP_PX` (which is
    what a portal is). `IGNORE_PX` is set by the wire, not by taste: positions travel as quarter
    pixels, so under half a pixel is rounding rather than disagreement and nudging for it would be
    jitter on every snapshot. The **drawn** pose is written into the replica between the snapshot
    and `tick_presentation`, never after - the presentation pass eases `visual_rotation` and presses
    the tread marks, so it has to run on the pose that will be drawn. **Provisional shots**
    (`online_predict_shots`, **on by default**; stage 4, docs/online-coop-prd.md §4.16): the
    provisional is the only drawn copy of this seat's shot for its whole life - it runs the real
    projectile's state machine (`ProvisionalShot::advance` in `simulation/present.rs` rebuilds the
    shell/bullet/bolt and steps its `update`, muzzle frames first), is swept each frame against the
    drawn world (`Game::present_world` / `PresentWorld::shot_contact`: tiles, the field edge, every
    live tank but the shooter's, frogs - `Terrain::sweep` cannot serve, it finds enemies by their
    `Ai`, which a replica strips) and stops there with its impact drawn at once; the room's copies
    of this seat's shots (`ShotState::owner`, `Game::seat_shots`) are paired with the provisionals
    by the `Fired` that names the press's input tick
    (`Predictor::confirm_fired`/`observe_server_shots`) and taken off the picture
    (`hidden_server_shots`, `Game::remove_shots`); a room hit the client did not draw snaps the
    provisional to it, and a room copy that flies on past a drawn hit or bursts away from it is
    shown instead (`crossings_missed`, decision 9's reading). Presses, cooldowns, the twin barrel
    and the minigun run on the sandbox's tick grid; a press waits as long as the link's round trip
    plus delay for its `Fired` (`set_refusal_after`); the laser is drawn on the press from the
    predicted muzzle (`Game::seat_beam`, `take_beams`), and so is each shot's muzzle ripple
    (`take_muzzles`, `Game::draw_muzzle`), while the replica takes the room's snapshot as
    `apply::Show::OwnShotsDrawn { seat, beams }`, which skips this seat's `Fired` ripple and the
    beams it drew: before the apply each of the seat's laser `Fired` claims the last beam the client
    drew at or before its input tick - earlier ones still waiting were refused -
    (`OnlineRound::confirm_beams`, `Predictor::confirm_beam`; drawn beams are dropped past the
    refusal wait), and only a claimed `Fired`'s `LaserBeam` is left out - one the client never drew
    (its gate refused the press) is the room's to draw, and seeds the local gate (the sandbox takes
    `Show::Quiet`: nothing ages it, so no spectacle); firing recoils the owned hull on the press
    (`Game::seat_recoil`), once per shot whether it is drawn or not - the room echoes only a missile
    launch's recoil. **Into a portal** (`ShotStop::Portal`, `PresentWorld::portal_entry`, the room's
    rule and `leaving` portal) a provisional leaves the picture with no impact, and its room copy
    stays hidden until that copy's `ShotTeleported` is handed over (`Predictor::shot_teleported`),
    then is the shot from the exit on - for about a round trip plus the delay the shot is in neither
    picture; a beam drawn on the press stops at its first portal and the legs past it are the
    room's; incoming fire carried into a portal is off the picture until its `ShotTeleported`
    restarts its lead from the exit. **A shot outlives its own drawing** (`Live::finished`): kept
    off the picture until its paired room copy has left the snapshots, or the refusal wait passes
    with none, so a copy arriving a round trip after its shot burst is still that shot's and stays
    hidden - never taken for the next shot's, which then leapt to this one's impact, nor shown on
    its own; every shot keeps one provisional id (`PROVISIONAL_ID_BASE` + `Live::id` within
    `PROVISIONAL_ID_MASK`) for its life. A press's first shot's copy is in the picture on the frame
    its `Fired` confirms it, a later shot's its own delay after (`Live::after`,
    `LATER_SHOT_SLACK_TICKS`), so a shot still unpaired on the frame after its copy came due is
    **orphaned** - its copy never reached the picture - and left out of the pairing, so the next
    shot's copy is never taken for it, and goes the moment it is off the picture. **Incoming fire in
    the present** (`draw_incoming_in_present`): every other shot in flight is carried from render
    time to where it will be when this client's latest pose lands on the room's clock
    (`incoming_lead_ticks`: newest tick + inputs since its `acked` - render tick, capped at
    `MAX_LEAD_TICKS`), easing up over `CATCH_UP_MS` from the muzzle it left, stopping at walls
    (`PresentWorld::static_contact`); one reaching this seat's drawn hull shows its impact at once
    and is kept off the picture until the room's copy goes - or until the room's copy, where the
    room has it, is seen flying on past that point, when it is drawn again. The incoming pass runs
    first and hands back every opposing shell's stretch of the frame (`IncomingShell`), and each own
    shell is swept against them by the room's `shell_vs_shell` rule (`present::shells_meet`,
    `first_contact`: whichever of a world contact and a meeting comes sooner), both bursting at the
    midpoint - both are drawn on the room's clock of this client's present, so the crossing in the
    picture is the room's. A press opens a `Press` gated as the server gates it - the sandbox's seat
    carries the server's weapon and ammo (`Game::seat_arms`), the cooldown is the weapon's own, the
    ammo of presses not yet accounted for is subtracted, shells and plasma fire on the press edge
    and the minigun while held (`drive_player`'s rule) - and draws its shots from the sandbox's
    muzzle as each falls due (`Game::seat_shot`: the shell or bolt at once, a twin barrel's second
    `tank_twin_shot_delay_seconds` on, a burst's bullets `minigun_bullet_delay_seconds` apart with a
    hashed spread), handed to the replica every frame (`Game::add_provisional_shot`). A press is
    `note_fired` when its `Fired` *arrives* (its ammo is in the sandbox now) and `confirm_fired`
    when the interpolator *hands that `Fired` over*, by the input tick it names; a `Fired` with no
    press waiting seeds the local gate from its input tick; a press nobody claims within the refusal
    wait expires, counted. The pod and the flamethrower's cone stay the room's (the cone is the
    `FLAME` flag: `tick_presentation` rebuilds the jet from it, and `Game::hold_flame` raises it on
    the local seat while the key is down). The room judges this seat's shots against the hit boxes
    of the tick this client was drawing (lag compensation, `hits.rs`), which is what the drawn hits
    are measured against; still the server's: damage, pickups, ram, everyone else. **The lead**
    (`round.rs`'s `Lead`, §4.12): the client's lead is a *depth* in its mailbox, not a stamp - on a
    reported starvation one extra packet goes out (`Adjust::Extra`, the sandbox steps twice), on a
    depth over `LEAD_DEPTH_MAX` for two `LEAD_WINDOW`s without one a packet is skipped
    (`Adjust::Skip`), at most one adjustment a window. **Stage 3, `online_client_hull` (on by
    default, Restart; docs/online-coop-prd.md §4.14): the client owns its hull.**
    `OnlineRound::client_hull` puts `Predictor` in owned mode (and turns the `Lead` off - one packet
    and one sandbox tick per tick, never two or none, the room's play point keeping its own margin;
    the hull is drawn at the sandbox's newest tick, `Predictor::drawn_pose`, as a local round draws
    its newest step): the sandbox drives the seat as before, `reconcile` writes the room's world
    around it and puts the hull back where it was (no replay, no nudge, `in_flight` 0) - with
    `online_predict_own_tank` off too, which then decides only what is drawn: an owned sandbox
    always takes the room's world, `Fired` and shoves (`sandbox_follows_room`), since its poses are
    the seat's - and every packet carries the pose that tick produced (`send_one`:
    `RoomClient::prepare_intent` → `step_at` → `IntentMsg::with_pose` → `send_prepared`). The room
    puts the seat there (`Game::accept_seat_pose`, below) and answers a pose it refused or a hull it
    moved itself with `WireEvent::Placed`, which - like this seat's `Teleported` and `TankEntered` -
    `place_from` snaps the sandbox to (`Predictor::place_own`). Off, stage 2 predicts and reconciles
    as before. `room_has_seat_at`/`own_hull_at` are what the server's
    `a_client_that_owns_its_hull_is_followed_by_the_room` compares. Knockback on an owned hull
    arrives as `WireEvent::Shoved`, which `note_fired` applies to the sandbox's body
    (`Predictor::shove`). `Predictor::report` (corrections by `ERROR_BUCKETS_PX`, nudges, snaps,
    shots drawn/refused, inputs in flight) plus the lead's counts is `status.round.prediction`, with
    **decision 9's instrument**: a provisional that `advance_shots` stops against a drawn tank or
    frog is a `crossing`, its paired room copy bursting within `HIT_MATCH_PX` of that stop makes it
    `crossings_hit`, and its copy flying on past `MISS_MARGIN_PX` or bursting anywhere else
    `crossings_missed` (the room's shot then shown) - the rate that says whether lag compensation's
    rewind is right. `rig.rs` is the offline rig - see docs/tooling.md.

## Dev tooling surfaces

- `devserver.rs` *(feature `dev-tools`, native only)* — newline-delimited JSON on `127.0.0.1:4747`
  (`--dev-port`/`BONGBONG_DEV_PORT`, `--no-dev-server`), docs/dev-server-design.md. Socket threads
  only queue `Request`s; `before_frame`/`dispatch` drain them at the frame boundary with the whole
  `Session`, so nothing touches `Game` mid-update. **Lockstep**: `step {frames}` freezes real time
  and runs N updates at `PHYSICS_FIXED_DT` in one rendered frame (bit-for-bit replayable from
  `restart {seed}`, which leaves the round frozen); `pause`/`resume`. Screenshots are captured one
  rendered frame after the request (raylib's read-back lags the swap). `TOOLS` is the single tool
  table `bbmcp` advertises: a new tool is one `ToolSpec` row (name, description, schema,
  `read_only`/`destructive` - the MCP annotations) plus a `dispatch` arm. **The round the tools read
  is `Session::shown()`**, so in an online round every reader - `status`, `snapshot`, `terrain`,
  `events`, `history`, `nav_grid`, `field`, `map_get`, `lint`, and `screenshot`/`overlays`, which
  write only a drawing flag through `Session::shown_mut` - describes the room's replica;
  `status.round` names whose round that is (`local`|`online`, the room code, the seat, the phase,
  `buffer_ms`, the server's tick, and the stage-2 readings: `interpolation` - the delay in force,
  its target, the jitter, the cadence, frames extrapolated, lateness p50/p95, stalls, the playout
  rate, corrections, stale events - and `prediction` - `Predictor::report` plus the lead's
  `lead_up`/`lead_down`/`lead_depth` and the `crossings`/`crossings_hit`/`crossings_missed` of
  decision 9), `ONLINE_REFUSED_TOOLS` - everything that would drive the local round or the builder -
  refuse by name because only the server simulates that round, and `key {escape}` gives the seat up,
  as does a `click` on the bar's `LEAVE` button. `input` is allowed online: it stands in for the
  keyboard, so it drives this window's seat in the room (`shape_input` runs before the online
  branch), its player-2 fields and `cycle_overlays` refused there (`ONLINE_INPUT_REFUSED`). A
  replica is never `advance`d, so `before_frame` banks its events and track rows on the frames a
  snapshot moved it on. `status` carries `level` (number, count, map, title, reached; `null` in free
  play), `stats` (`round_stats`) and `pacing` (the director's intensities and breather,
  `Game::pacing`), and `click`/`key {enter}` reach a level's end screen through
  `Session::press_result`/`enter_result`; `click` reaches the bar's level button and the level
  select's tiles, and `key` takes `escape` (open or close it) and `left`/`right`/`up`/`down`/`enter`
  in it (`status`/`mode` carry `levels_open` and `levels_focus`). `step` refuses by name while a
  dialog or the level select freezes the round, rather than waiting for frames that never run.
  `GAME_ONLY_TOOLS` are refused in build mode with an error naming `play`; `status`, `screenshot`,
  `overlays`, `map_get`, `lint` (the map linter over the builder's canvas or the round's map, on a
  temporary headless round), `tuning_*`, `lang` (the language on screen: report, or switch to a
  shipped tag at the frame boundary; `status.language` carries it) and `restart` work in both modes;
  `weather` (the sky on screen: report `in_force`/`map`/`override` and the `rules` in force -
  `enemy_sight_px`, `grip`, `frozen`, the gust on player 1 and the front crossing the field -, or
  set the round's map key - a name or a list - and change the sky mid-round (`Game::change_weather`:
  the look and rules at once, snow icing the water over - deep cells' bodies off, the nav grid
  rebuilt - but no thaw until the next round, since a hull may stand on the ice), or with `restart:
  true` start the round over under it on its own seed - a game-only tool, since the builder's canvas
  is drawn clear, and report-only online; `status.weather` carries it), and `restart` with
  `map`/`map_toml` replaces the builder's canvas too. In build mode `builder_camera` (`{x, y, zoom}` |
  `{fit: true}`; `status.builder.camera`: `fit`, `rect`, `scale`, `zoom`, `fit_scale`,
  `device_scale`, `cell_mm`, `area`), `builder_touch` (frames of `{id, x, y}` in window coordinates,
  the fingers lifted after the last; refused online), `builder_select` (`rect`, `clear`, `move_by`,
  `action`: copy|cut|delete|flip_h|flip_v|stamp|paste|place|cancel, `at`) and `builder_stamp`
  (`name`, `at`, `place`) drive the select tool and the STAMPS list (both in
  `ONLINE_REFUSED_TOOLS`), `builder_tool`/`builder_paint` take `shape` (pen|rect|fill|scatter) and
  `builder_paint` replies with the status line's `message`, `builder_settings {size, anchor}`, `key
  {zoom_in|zoom_out|arrows|copy|cut|paste|delete}`, and `click` replies with the world point and the
  cell. `status.builder` also carries `buttons` (the bar's by name - `play`, `play_here`, `check`,
  `clear`, `fit`, `map`, `file`, and with CHECK open its `finding_N`, `fix_N`,
  `page_back`/`page_next` - in window coordinates with each one's `ui` rect in points; with a popup
  open its rows too - `tool_<name>`, `load`/`save`/`save_as`/`clear_map`, the Save prompt's
  `save_confirm`, `map_<name>`, the MAP panel's `tab_<group>`, `<field>_<option>`,
  `<field>_dec`/`_inc`, `weather_<sky>` and `reset`, `page_back`/`page_next` -, the bar's own
  `erase`, `undo`, `redo`, `category_<c>`/`list_<c>`, `brush` or the folded `tools`, BRUSH's
  `shape_<name>`/`tool_select`/`brush_stamps`, the select strip's `sel_*` and the STAMPS list's
  `stamp_<key>`), `check`, `clear` (`revision`, `cleared`, `par`, `attempt`), `loupe`, `shape`,
  `rect` (a RECT drag's rectangle), `message`, `selection`, `ghost`, `clipboard`, `stamps` and
  `thumbnails` (the open Load list's: `name`, `cells`, `picture`); `status.play_here` is a play-here
  round's cell; `builder_touch {hold: true}` leaves the last frame's fingers down (`held_touches`)
  until the next call or the builder is left; `lint` replies carry `cells`, `nav_cells` and `fix`.
  `status.ui.minimap` is the play minimap's picture in window coordinates (`null` where none is
  drawn) and `status.builder.navigator` the navigator's (window coordinates with its `ui` rect, for
  `click` and `builder_touch` alike; `null` at FIT on an arena and under a popup over its corner);
  `click {touch: true}` is a tap, and `status.ui.hints` names the input the words follow.
  `status.camera.focus` can be `split`, with `status.camera.split` carrying `line`, `window_line`,
  `apart` and each half's `seat`/`rect`/`offset`/`cut`/`in_view` (a `click` past the divider lands
  in the second half's world); `status.camera.view` is `establishing` during the establishing shot,
  with `establishing.phase`/`progress`; `status.camera.motion` reports the motion switch. `play`
  does what `restart` does afterwards, so the step/screenshot loop carries over. `terrain` is the
  tiles-and-fire counterpart of `snapshot`. The capture (`after_render`) is `render`-only; a
  headless server arms a `screenshot` and never answers it. A history ring keeps 3600 frames of
  `debug::TrackRow` for the `history` tool, and per-slot `TurnStats` for the round (turns, u-turns,
  A->B->A reversals, spins by the probe's rule, turret sweep - `history.tanks[].round`, summed in
  `status.turns`; the rule itself is `simulation::debug`'s `signed_quarter_turn`/`SPIN_*`, shared
  with the probe); the event ring is 4096 with `kinds`/`exclude` filters; `before_frame` sets
  `Game::trace_ai`. Tested headlessly via `DevServer::headless()`.
- `capi.rs` *(feature `dev-tools`)* — the `extern "C"` surface the web page calls via
  `Module.ccall`: tuning for the panel, plus `bb_net_stats` (an online round's
  `OnlineRound::stats_json` - link, interpolator, prediction, drawn tanks - published by `app.rs`
  every frame of the round and taken down to `""` the first frame the window is in any other mode,
  `NetStatsFeed`) `bb_input(move_dir, fire, frames)` (drives the local seat in place of the
  keyboard) and `bb_ui_json` (the window as laid out last frame: `frame`, the window's size and
  `units_per_point`, the mode, map and level, the dialogs, the chrome through `capi::ui_status` -
  which is also the dev server's `status.ui`, built from `Session::screen_buttons` and
  `lobby::Button::name` -, the `view` and `field`, the builder bar's buttons and the last `press`,
  which a script taps the canvas by), so a browser tab against a deployed room is measured and
  driven by a script; strings in a thread-local scratch buffer valid until the next call. `app.rs`
  calls `capi::keep_alive()` so the symbols survive; `build.rs` emits `-sEXPORTED_FUNCTIONS` and
  `-sSTACK_OVERFLOW_CHECK=2` for the wasm dev build (`capi::EXPORTS` and the build.rs list are
  checked against each other by a test).
- `bin/bbmcp.rs` *(`required-features = ["dev-tools", "render"]`, excluded from `[dist.binaries]`)*
  — the stdio MCP adapter `.mcp.json` launches, **once per target**: bare it drives the game
  window's dev server (`TOOLS`, port 4747), `bbmcp rooms` the room server's (`ROOM_TOOLS`, port 4849) -
  one binary, one protocol, two tool tables, picked by a `Target` so a name from one can never be
  called against the other. One request line to the target per call (`handle_line` is the
  per-message handler, unit-tested with a stubbed game), screenshots as `image` content,
  `tools/list` carries each tool's annotations, an unreachable game is an `isError` naming `just
  run-dev`. `bbmcp call <tool> [json]` for shells (`just mcp-call`). stdout is protocol only - log
  with `eprintln!`. Stub `main` for emscripten (a wasm build without `--bin` compiles every bin; the
  web builds name `bongbong` alone).

## Seats

docs/two-players.md, docs/online-coop-prd.md §4.11.

`Game::players: PlayerCount` is how many seats the round holds, `1..=MAX_SEATS` (8) - a pre-init
setting kept across restarts (`--players`, `--tank2`, the players button/dialog, `restart {players,
tank2_row}`, `builder_settings {tank2}`). `Game::seats: [Option<Entity>; MAX_SEATS]` is one tank per
owner slot (`players()`/`player()`/`seat(i)`/`player_index`/`is_player`/`first_enemy_slot`),
`Input::seats` the matching intents, one `EngageRing` per seat. **Every walk goes seats in index
order then enemies**, and `init`'s per-seat block runs once per seat after player 1, so a one- or
two-player round's RNG stream is byte-for-byte what it was
(`determinism_tests::the_one_and_two_seat_streams_are_pinned` is the gate - re-baseline consciously,
never to go green). Spawns: `start`, `start2`, then the nearest open nav cell to player 1 a tank can
*drive* to, a tank's width clear of every seat already down and moved ashore by
`ground::dry_cell_near`; seat 1's fallback keeps the plain `nearest_open` walk, because a seeded
two-player replay is that walk's answer. `Ai::target_player` retargets to the nearest live, visible
seat with hysteresis (`Event::Retarget`, from two seats up); `friendly_fire_damage_factor` scales
player-on-player damage. **Death**: a wrecked seat waits for the round to end, except in a **wave**
round of two or more, where it re-enters with the next wave through a gate, no penalty, as a fresh
tank of its own chassis (`waves.rs`, docs/online-coop-prd.md §4.11 decision 7). The round is still
lost the frame every seat is a wreck at once, so a solo round is untouched; a seat wrecked after the
last wave is called, or while the round ends, stays where it fell. `Game::seats_on_field()` is
`players()` with the seats in a gate lane masked out - the walk every phase that must not touch an
off-field tank takes. Map: `start2`, `tank2`; editor `Tool::Start2`, `SettingsRow::Tank2` (and
`SettingsRow::Theme` for the map's look); dev server/probe take `p2_*` intents / `--players N
--tank2 --p2-scenario` (one script for every seat past the first); maplint adds
`player2-unreachable` (any seat past player 1), `players-too-close`, `start-penned`. Slot rule:
seats first, enemies from `first_enemy_slot`.

**Two is still the couch's number**: player 1 is arrows + Space, player 2 WASD + Left Shift and
there is no third pair of keys (a seat past two is a room's and stands idle in a local round); the
players dialog offers one or two. The HUD has a block shape per kind of round (`hud::HudLayout`):
`One`, `Two` (player 2's block beside or under player 1's in the left corner) and `Compact` from
three couch seats or any room of two - the local seat's block plus a ring-coloured, numbered chip
per other seat. The sheet has four player blocks (sky blue, hot pink, silver-white, orange), so a
seat past the fourth draws player 1's and is told apart by `tank::TEAM_COLORS`/`team_color` (the
four blocks' base steps, then four more Resurrect 64 steps; `HealthRamp::Team`) and its `P1`..`P8`
locate label. Not available on iOS/Android (`TWO_PLAYERS_AVAILABLE`); on the web Left Shift arrives
through the `window.bbShift` shim (docs/platforms.md, Web). On a field map the couch's screen splits
once both sight boxes no longer fit one view (`follow.rs`), and the HUD keeps `HudLayout::Two`.
