# PRD: the map builder inside the game

Status: requirements, 2026-09-10; implemented the same day (the code is the reference where the two differ). Supersedes the build-mode half of
docs/hud-and-builder-layout-design.md (variant A, whose play half is
implemented) and the "Entering the editor" section of
docs/map-editor-design.md. Nothing here is implemented yet; the seams the
layout doc named (`Layout`, `hud.rs`, the field origin) exist and are the
starting point.

Related: docs/map-editor-design.md (the editor's data model, palette and
file format), docs/hud-and-builder-layout-design.md (the bar, the field
origin, the `Driver` sketch), docs/mobile-and-scaling-design.md (why the
bar is small on a phone and what would fix it), docs/maps-to-levels.md
(the `mission`/`spawn` tables the settings menu edits).

## 1. Summary

The map editor stops being a separate `--editor` program and becomes a
**mode of the game**. In play mode the HUD bar ends in a `BUILD` button.
Pressing it freezes the round and asks whether to leave it; on yes, the bar
becomes the builder's toolbar and the field becomes the map canvas, showing
the *authored* map the round was built from. `PLAY` at the same right-end
slot starts a fresh round on the edited map. The builder ships to every
player, on desktop and on the web, mouse and touch alike. Edits live in
memory for this pass; saving comes later. The dev server and the `bbmcp`
MCP adapter learn the builder too, so an agent can enter it, edit, play
and screenshot the result without a hand on the mouse (section 11).

## 2. Decisions

These were settled in the interview that produced this document and are
not re-argued below.

| Question | Decision |
| --- | --- |
| Audience | Players too: release and web builds, not dev-only. |
| "Only the current map can be modified" | Edit the *authored* map (`Game::map`, the TOML the round was built from). No file switching: no New, no Load. |
| Return to play | PLAY always starts a fresh round on the edited map; confirming BUILD abandons the round for good. |
| Toolbar | Category dropdowns per the layout doc (WALL / PROP / GROUND / ACTOR / PICKUP), each button showing its current tool. |
| Persistence | **Native**: Load and Save to `maps/*.toml` for everyone. **Web**: Load from the maps shipped inside the binary, edits kept in memory and lost on reload (localStorage is a later step, section 12). |
| Touch | A first-class target. |
| Touch sizing | Big targets over the field (dropdown rows, dialog buttons 48 px and up); the bar itself stays 32 px. Screen-space UI scaling remains the mobile doc's follow-up. |
| Map settings | In scope: steppers for tanks, player chassis, mission, spawn plan and the wave parameters. |
| Feature gate | Remove `map-editor`; the builder compiles into every build. `--editor` stays and means "start in Build". |
| Erase | Tap the same object again to erase, plus right-click with a mouse; a small ERASE tool stays. |
| Undo | An undo/redo stack in the bar, one step per edit, plus Ctrl+Z / Ctrl+Y. |
| Validation before PLAY | None; `Game::init`'s fallbacks handle an incomplete map. |

Assumptions made while writing, listed again in section 15 so they can be
reversed cheaply: the leave dialog is skipped on the end screen; PLAY keeps
`--seed`'s meaning. The app starts in play mode; `--editor` initialises the
round as usual and then opens the builder on it.

## 3. Goals

- One binary, one window, two modes: **Play** and **Build**, switched from
  the bar (and `Tab`), with the loaded map as the thing both modes share.
- A player can reshape the map they are playing and play it again within
  seconds, without a file, a flag or a restart of the program.
- The builder's toolbar lives **in the 32 px bar**, so the whole field is
  editable and no palette floats over the bottom rows any more.
- Everything a map file can say is authorable in-game: cells, and the
  `tanks`/`tank`/`mission`/`spawn` keys that today only a text editor sets.
- Works with a finger: every choice a player makes while building has a
  target of at least 48 px on the canvas, and erasing needs no right button.
- Mistakes are cheap: undo and redo.
- Testable without a hand: every transition and edit is reachable from the
  dev server (`just run-dev`, the `mcp__bongbong__*` tools) and the same
  code path a click takes, so what a tool did is what a player would get.
- The simulation is untouched. `simulation/`, `battlefield.rs`, `map.rs`,
  `maplint.rs`, the probe and every seeded baseline keep seeing a 1280x720
  field and a `MapFile`; the builder is presentation, like `game.rs`.

## 4. Non-goals (this pass)

- **Web persistence.** On the web an edited map survives PLAY/BUILD round
  trips within the session and is lost on reload; localStorage is the
  named follow-up in section 12. Native saves to disk.
- **New / blank map** from inside the game: a map is edited from one that
  exists (loaded, or the one the program started with).
- **Screen-space UI scaling** (a 48 pt bar on a phone). The bar stays 32 px
  of the 1280x752 canvas; the builder gets finger-sized targets by opening
  them over the field instead. The mobile doc's `Layout::fit` is the fix
  for the bar itself and is not part of this work.
- **Validation before PLAY.** A map with no start cell, no frog, or an
  unreachable frog plays through `Game::init`'s existing fallbacks, as it
  does today from `-m`. The linter stays a dev tool.
- Multi-select, copy/paste, rectangle fill, map size changes, a second
  field size, panning or zooming.
- Editing the live, shot-up battlefield. Build mode edits the map as
  authored; a wall the round destroyed is back in the builder.

## 5. The two modes

```
             BUILD (bar / Tab) -> "Leave this round?" -> Leave
   Play  ----------------------------------------------------->  Build
   round  <-----------------------------------------------------  canvas
             PLAY (bar / Tab): game.map = builder.map; game.init()
```

- A `Session` (`mode.rs`, section 13) owns a `Driver { Play, Build }` next
  to the `Game` and the `MapEditor`; both exist for the whole session and
  share the loaded textures. `main.rs`'s loop body branches on the driver:
  only Play gathers a `simulation::Input` and calls `Game::update`. The
  simulation never learns the builder exists.
- **Play -> Build** shows the builder's canvas. It is seeded once, at
  startup, from `Game::map` (a `MapFile`, the map the round was built
  from) and from then on it *is* the session's map: switching modes never
  reseeds it, so edits survive a PLAY/BUILD round trip. The one thing that
  replaces it is the dev server's `restart` with a `map`/`map_toml`, which
  sets both the game's and the builder's map (and the baseline). No
  conversion from live entities, ever.
- **Build -> Play** clones the builder's map into `game.map` and calls
  `Game::init` for the field size. It is the dev server's `restart {map_toml}`
  path without the TOML. Every PLAY is a new round: new spawn rolls unless
  `--seed` pinned them, mission banner as on any restart. There is no
  "resume": confirming BUILD gave the round up.
- `--editor` keeps working and means "start in Build": the game is
  initialised as usual (so PLAY has everything it needs) and the driver
  starts on the builder. The leave dialog never shows on that path because
  no round is in progress.
- The builder compiles into every build. The `map-editor` cargo feature is
  removed: `Cargo.toml`, the `#[cfg]`s in `main.rs`/`lib.rs`/`editor.rs`,
  the justfile/CI flags. `static/ui/eraser.png` loads unconditionally (it is
  already bundled with `static/`).

## 6. Play mode: the BUILD button and the leave dialog

- The play bar gains `BUILD` at its right end, the slot the layout doc
  reserved (x 1200..1272 of 1280, full bar height). Drawn by `hud.rs` in the
  bar's style: 18 px text, an outline, the builder's amber accent.
- A click or tap on it, or `Tab`, opens the **leave dialog**. The round is
  frozen by construction: while the dialog is open the driver does not call
  `Game::update` (and the dev server's `advance` is skipped), so no
  simulation pause flag is touched. The field is drawn as it was, dimmed
  like the PAUSED overlay (`Color(0,0,0,120)`), the bar unchanged.
- Dialog: a centred panel over the field, `Leave this round?` and one line
  `Your progress is lost. The map is kept.`, two buttons `LEAVE ROUND` and
  `KEEP PLAYING`, each at least 48 px tall and 160 px wide, 16 px apart.
  `Esc`, `Tab` again or a tap outside the panel mean keep playing; `Enter`
  means leave. Taps on the field while the dialog is open are consumed
  (never an order).
- The dialog is skipped on the end screen: there is no round to lose, so
  `BUILD` switches at once. In every other state, paused and intro
  included, it asks.
- Release builds, web included, have the button. The dev overlay label and
  the version line stay where they are.

## 7. Build mode: the bar as toolbar

One row, fixed slots, left to right. Widths are for the 1280 px bar; a
`hud_tests`-style unit test pins that the slots do not overlap and end
inside the bar.

```
 x:   8      72          240                              748  796  840  888     960     1032         1200
     [BUILD] [default *] [WALL ▾][PROP ▾][GROUND ▾][ACTOR ▾][PICKUP ▾] [⌫] [↶] [↷] [FILE ▾] [MAP ▾] [12,7 brick] [PLAY]
```

| Slot | x | Width | Contents |
| --- | --- | --- | --- |
| Mode | 8 | 56 | `BUILD` in the amber accent, so the mode is never in doubt. |
| Name | 72 | 160 | The map's display name (`MapFile::name`: `default` for the embedded map, `untitled` for none) plus ` *` while edited since load. |
| Categories | 240 | 5 x 100 | `WALL`, `PROP`, `GROUND`, `ACTOR`, `PICKUP`. Each button shows its category's *current* tool as its 32 px icon, full-bleed, with the tool's name in 10 px to the right and a caret. Click or tap the icon: select that tool. Click or tap the caret or name: open the list. Mouse wheel over the button: cycle inside the category. The active category is outlined in the accent, like the live weapon slot in play. |
| Erase | 748 | 40 | The eraser tool: a plain brush that clears cells. Outlined while active. |
| Undo / Redo | 796 / 840 | 40 each | Dimmed when the stack is empty. |
| File | 888 | 64 | `FILE ▾`: `LOAD...` on every build; `SAVE` and `SAVE AS...` on native, where a map can be written. |
| Map | 960 | 64 | `MAP ▾`: opens the settings panel (section 9). |
| Cursor | 1032 | 160 | `col,row` and what is in the cell: the hovered cell with a mouse, the last tapped cell on touch. Dim text. |
| Play | 1200 | 72 | `PLAY`, in the same slot `BUILD` occupies in play mode. |

Categories and their tools, in list order:

| Category | Tools (23 with the eraser) |
| --- | --- |
| WALL | brick, iron, wood, glass |
| PROP | sandbag, barrel, fence, tree, pine |
| GROUND | road, tall grass, gate |
| ACTOR | start, frog, enemy frog |
| PICKUP | health, ammo, laser, minigun, plasma, speed-up, shield |

Each category remembers its current tool for the session; the initial tool
is the first in each list, and the active brush at entry is `WALL / brick`.

### Dropdowns

- Open below the button, over the field, one panel in the editor's existing
  rounded/bordered/shadowed style (`draw_panel`). Rows are **48 px tall and
  200 px wide**: the 32 px icon with 8 px inset, the name in 18 px, the
  current tool highlighted. A list never exceeds 7 rows (PICKUP), 336 px,
  well inside the field.
- One popup at a time: opening any dropdown, the settings panel or the
  leave dialog closes the others.
- Closes on pick, `Esc`, or a press anywhere else. **That press is
  consumed**: it neither places a cell nor selects another tool, so a tap
  to dismiss a menu on a phone cannot paint through it.
- The singleton badge (a dot on the icon when a frog/start/start2/enemy
  frog is already placed) moves from the old palette to the dropdown row and to the
  category button when that tool is current.

### Files

- `FILE ▾` opens a menu of the same row style. `LOAD...` opens the **Load
  list**: a centred panel over the field, 48 px rows, twelve visible with
  the wheel scrolling the rest, every map `map::available_maps` offers -
  the `maps/*.toml` files on native plus the maps shipped inside the
  binary (`default`, `hunt-basic`, `waves-basic`, marked `shipped`), which
  is all the web build can list since nothing outside `static/` ships in
  the wasm. Picking a row loads the map as one undo step and the new
  baseline; no confirmation, undo covers a mistake.
- `SAVE` writes the map back to `maps/<name>.toml` when it has a name,
  otherwise it behaves as `SAVE AS...`, which prompts for one (letters,
  digits, `-`, `_`). Saved is the new baseline, so the ` *` clears. Both
  rows exist only where `map::saving_available` is true: native. The web
  menu is `LOAD...` alone; its edits live in memory for the session.

## 8. Placing, erasing, undo

- A press on the field places the active tool at the cell under it; a drag
  paints every new cell it crosses, once per cell (today's `drag_cell`
  rule). Singletons (start, start2, frog, enemy frog) move rather than
  duplicate, each independently.
- **Toggle erase.** With a brush selected, a press on a cell that already
  holds *exactly that object* (same kind, same material or pickup) clears
  it instead. The decision is made **on the first cell of a press** and
  held for the whole drag: a drag that started by clearing keeps clearing,
  one that started by painting keeps painting and skips nothing, so a
  stroke across a row of bricks with the brick brush is either all paint or
  all erase, never a checkerboard. Road and tall grass follow the same rule.
- Right mouse button erases regardless of the brush. The `ERASE` tool
  clears whatever is there; it is the touch user's "clear a mixed area"
  brush.
- The hover highlight follows the mouse; on touch it stays on the last
  tapped cell so the cursor readout has something to show.
- **Undo/redo**: every completed press (a click, or a whole drag stroke)
  and every settings change is one step. A step stores the cells it changed
  with their before and after values, so undoing a 30-cell stroke is one
  action. Depth 200; a new edit clears the redo branch. `Ctrl+Z`/`Ctrl+Y`
  (`Cmd` on macOS) and the two bar buttons. The stack is cleared on PLAY and
  on RESET, not on switching modes and back.
- `dirty` means "differs from the map as loaded at startup" (kept as a
  baseline copy), since there is nothing to save to. It drives the ` *` and
  the RESET row.

## 9. The MAP ▾ settings panel

A panel below the button, 340 px wide, rows 48 px tall, each a label with
a stepper (`<`/`>` or `-`/`+` buttons of 48x48 px and the value between).
Values are the `MapFile` fields, so nothing new is stored.

| Row | Field | Range / values |
| --- | --- | --- |
| TANKS | `tanks` | 0..=31 (`wave_max_alive`); stepping below 0 shows `auto` = the knob roll (`None`) |
| TANK | `tank` | `auto` (`None`) then the 12 `TankKind` names in row order |
| MISSION | `mission.kind` | protect, hunt, destroy |
| SPAWN | `spawn.kind` | band, waves |
| WAVES | `spawn.waves` | 1..=20, `auto` = the `waves` tuning group |
| SIZE | `spawn.size` | 1..=31, `auto` |
| GROWTH | `spawn.growth` | 0..=10, `auto` |
| TIER START / TIER END | `spawn.tier_start`/`tier_end` | auto, light, medium, heavy, super |
| RESET MAP | | Reverts cells and settings to the baseline; asks nothing, it is undoable. |

The five wave rows are drawn dimmed while `SPAWN` is `band` (they still
edit, so a map keeps its wave numbers when switched back). CLI overrides
(`--enemies`, `--mission`, `--spawn`, ...) still outrank the map's values
at PLAY exactly as they outrank a file's; the panel shows the map's value
and marks a row `(cli)` when a flag is overriding it, so a player who
started the program with `-e 8` understands why TANKS is not honoured.

## 10. Touch

- raylib maps a touch to the mouse position and a tap to a press, so
  placement, drags and the bar's hit-testing need no second input path;
  `Layout::to_field` maps a window press into the field for both modes.
- Targets: everything a finger chooses **on the field** is 48 px or more
  (dropdown rows, stepper buttons, dialog buttons). The bar buttons are
  32 px tall in canvas pixels and about 21 CSS px on a phone; that is
  accepted for this pass, and each bar button's hit rect extends 8 px above
  and below its drawn box into the field/panel gutter so a slightly low tap
  still lands.
- Only the first touch point is read. A second finger does nothing (no
  pinch, no two-finger erase).
- No hover on touch: the cursor readout shows the last tapped cell, the
  highlight stays there, and category buttons never rely on a hover state.
- The web page needs no change: `touch-action: none` and the 1280x752
  aspect box are already in place (`site/src/pages/index.astro`).
- Verified by playing a PR preview on a phone, not by desktop emulation
  (section 14).

## 11. Driving the builder from Claude Code (dev server / MCP)

The dev server (docs/dev-server-design.md) drives `Game` between frames;
this pass extends it to the mode switch and the builder, with the same
rules: socket threads only queue requests, `main.rs` services them at the
frame boundary, nothing touches the builder or the game mid-update, and
every tool is one `ToolSpec` row so `bbmcp` advertises it to Claude Code
and `just mcp-call <tool> '<json>'` reaches it from a shell.

The principle that makes this cheap: **the builder takes a plain input
struct, not a `RaylibHandle`.** `MapEditor::update` today reads the mouse
and keyboard itself. It changes to take a `BuilderInput` (a press/release
with a window position and button, a drag position, a wheel delta, the
keys `Tab`/`Esc`/`Enter`/undo/redo, typed text for the dev Save prompt)
that `main.rs` gathers once per frame, exactly as it gathers a
`simulation::Input` for `Game::update`. The tools below then feed the same
struct, so a `click` lands on the same hit-test a finger would, and the
mode transitions can be unit-tested headlessly through
`DevServer::headless()` like the game tools are.

### New tools

| Tool | Parameters | Does |
| --- | --- | --- |
| `mode` | none | Reports `mode` (`play`/`build`), whether the leave dialog is open, the builder's `dirty` flag, map name, active tool and category, which popup is open. Cheap; `status` carries the same `mode` field. |
| `build` | `answer?: "leave" \| "stay"` | Presses BUILD: opens the leave dialog when a round is in progress, switches at once on the end screen. With `answer`, answers an open dialog instead. Replies like `mode`. |
| `play` | `intro?: bool` | Presses PLAY from Build: the edited map becomes the round's map and a fresh round starts, frozen in lockstep like `restart` (so `step` counts play frames; `resume` for real time). Replies with `status`. Fails in Play mode. |
| `builder_tool` | `tool?: name` | Selects a brush by name - the 22 tool names (`brick`, `iron`, `wood`, `glass`, `sandbag`, `barrel`, `fence`, `tree`, `pine`, `road`, `tall_grass`, `gate`, `start`, `frog`, `enemy_frog`, `health`, `ammo`, `laser`, `minigun`, `plasma`, `speedup`, `shield`) or `eraser` - through the category's own selection path, so the category button updates. Without `tool`, lists the categories with their current tool and the active one. |
| `builder_paint` | `cells: [[col,row], ...]`, `tool?: name`, `button?: "left" \| "right"` | One **stroke**: presses on the first cell and drags through the rest, so the toggle-erase rule, singleton moves and the one-undo-step-per-stroke rule all apply exactly as for a mouse. `button: right` erases. Replies with each cell's object before and after and the undo depth. |
| `builder_undo` / `builder_redo` | `steps?: n` (default 1) | Undo or redo that many steps. Replies with the depth left on each side and the cells the last step changed. |
| `builder_settings` | `tanks?`, `tank?`, `mission?`, `spawn?`, `waves?`, `wave_size?`, `wave_growth?`, `tier_start?`, `tier_end?` (each `null` = auto), `reset?: bool` | Sets the MAP ▾ values (each changed field is one undo step, in field order) or, with `reset`, reverts cells and settings to the baseline. Without parameters, reports the current values and which ones a CLI flag is overriding. |
| `builder_map` | `name?: string`, `map_toml?: string`, `map?: path` | Without parameters: the **builder's** map as TOML, its name, `dirty`, and a diff against the baseline (cells added, removed, changed; settings changed). With one: loads that map (by Load-list name, as inline text, or from a path) into the canvas as a single undo step and makes it the new baseline - the "load a map to test" path, and the way an agent hands back a map it edited as text. `map_get` keeps returning the map the *current round* was built from, which differs from this once the builder is dirty. |
| `builder_files` | none | What `FILE > LOAD` offers: every loadable map with `on_disk`, and `can_save`. |
| `builder_save` | `name?: string` | `FILE > SAVE` / `SAVE AS`: writes `maps/<name>.toml` (native), makes it the baseline. Replies like `builder_map`. |
| `click` | `x`, `y` (window pixels, bar included), `button?: "left" \| "right"`, `drag_to?: [x, y]` | A raw press at a window position, in either mode: the bar's buttons, a dropdown row, a dialog button, a stepper, a field cell. With `drag_to`, a press, a straight drag to the second point and a release, crossing every cell on the way. In Play mode a left `click` on the field is a `tap`. This is how the *UI* is tested, as opposed to the model the tools above address directly. |
| `key` | `key: "tab" \| "escape" \| "enter" \| "undo" \| "redo"`, `text?: string` | Presses one key (or types `text` into an open prompt) for one frame through the same `BuilderInput`. `undo`/`redo` are Ctrl+Z / Ctrl+Y. |

Existing tools in Build mode: `screenshot` and `overlays` work (the
presented frame is the builder); `status` and `mode` always answer;
`map_get` answers with the round's map; `tuning_*` work; `step`, `input`,
`teleport`, `set_tank`, `kill`, `spawn_enemy`, `pause`, `resume`,
`snapshot`, `events`, `history` and `nav_grid` return an `isError` result
naming `play` rather than touching a frozen game. `restart` switches to
Play first, and with `map`/`map_toml` replaces the builder's map and
baseline as well (section 5).

### What an agent session looks like

```
mode                                   -> play
build                                  -> dialog open
screenshot                             -> the dimmed field and the dialog
build {answer: "leave"}                -> build, tool wall/brick
click {x: 340, y: 16}                  -> WALL dropdown open (the caret)
screenshot
click {x: 340, y: 120}                 -> picked iron from the list
builder_paint {cells: [[10,5],[11,5],[12,5]]}
builder_paint {cells: [[11,5]]}        -> toggle-erased the middle iron
builder_undo
builder_settings {tanks: 3, mission: "hunt"}
builder_map                            -> TOML + diff: 3 cells added, 2 settings
play                                   -> frozen round on the edited map
step {frames: 300}
screenshot
```

The `restart`/`step`/`screenshot` loop that already verifies gameplay
carries over unchanged once `play` has run; a seed given to an earlier
`restart` stays pinned across `play` (section 5).

## 12. What stays out and where it plugs in later

- **Web storage.** `map::available_maps`/`open_map`/`MapEditor::save` are
  the seam: on the web they list only the shipped maps and refuse to
  save. localStorage via `emscripten_run_script`/`web_sys`, keyed
  `custom:<name>` so a shipped map is never shadowed, plugs in behind
  those three functions and the Load list gains a `saved` mark.
- **Screen-space UI** (docs/mobile-and-scaling-design.md): when the bar is
  drawn at device points the 32 px limitation in section 10 goes away with
  no change to this design; the dropdowns and dialogs simply get sized in
  points too.
- **Validation**: a `maplint`-light pass at PLAY that warns in the bar
  (`no start cell`, `frog unreachable`) is a natural next step once the
  linter's checks are callable on a `MapFile` without a full `Game::init`.
- **New/Load for players**, a map picker, sharing a map as text.

## 13. Code seams

| Where | Change |
| --- | --- |
| `mode.rs` (new) | `Session { driver: Driver, game: Game, builder: MapEditor, dialog: Option<LeaveDialog> }` with the transitions as methods: `press_build`, `answer_dialog`, `play`, `replace_map` (what `restart {map_toml}` calls). Both the window and the dev server go through these, so a tool and a click are the same path. No `RaylibHandle` in this file. |
| `main.rs` | Gathers a `BuilderInput` (mouse/touch press, drag, wheel, `Tab`/`Esc`/`Enter`/Ctrl+Z/Ctrl+Y, typed text) in Build mode the way it gathers `simulation::Input` in Play, hands it to `Session`, and renders whichever mode is live. The `--editor` branch collapses into "start in Build". Feature `#[cfg]`s go. |
| `Cargo.toml`, `justfile`, `.github/` | Remove `map-editor`; the builder is unconditional. `dev-tools` gates only the dev `SAVE`. |
| `hud.rs` | `draw_bar` draws `BUILD` at the right-end slot and reports its rect (`hud::build_button_rect(panel)`) for hit-testing; the leave dialog's drawing (`draw_leave_dialog`) and its button rects live here so play-mode chrome stays in one file. |
| `game.rs::render` | Takes an optional overlay to draw last, after the bar (the leave dialog), since `render` owns `begin_drawing`. The dim reuses the PAUSED rectangle. |
| `editor.rs` | The palette, the hamburger-era `Close`, `New`, `Load` and the yellow debug line go. New: `Category` with a current tool each, dropdown state, the toggle-erase rule in `place`, an `UndoStack` of `Vec<(cell, before, after)>` steps plus settings steps, the settings panel, `EditorAction::Play`. **`update` takes a `BuilderInput` instead of reading raylib**, so the builder is drivable headlessly; hit-testing of the bar moves to slot constants in `hud.rs`'s style. `MapEditor::new` takes the baseline map and keeps a copy for RESET/dirty. Direct entry points for the tools: `select_tool(name)`, `stroke(cells, button)`, `undo/redo(n)`, `set_settings(patch)`, `load(map)`, `map_diff()`. |
| `map.rs` | Nothing: `MapFile` already derives `Clone`, and the settings panel edits existing fields. |
| `lib.rs` | The `EDITOR_PALETTE_*` constants become dropdown/panel constants; `Layout` unchanged. |
| `devserver.rs`, `bin/bbmcp.rs` | `before_frame`/`advance` take the `Session` rather than a bare `Game`. `status` gains `mode`; the ten tools of section 11 are ten `ToolSpec` rows plus `dispatch` arms (`bbmcp` needs nothing, it reads `TOOLS`); the game-only tools refuse in Build with an `isError` naming `play`. Screenshots work in both modes (the presented frame). |
| `site/` | No change. |
| CLAUDE.md | Module map entries for `editor.rs`, `hud.rs`, `main.rs` once it lands. |

## 14. Testing

Headless (`cargo test --lib`, extend `editor_tests`/`hud_tests`):

- Bar slots in build mode do not overlap and end inside 1280 px, both with
  and without the dev `SAVE` button.
- Every dropdown fits inside the field for every category; rows are at
  least 48 px.
- Toggle erase: pressing a cell holding the brush's object clears it;
  a drag that starts by clearing only clears; a drag that starts by
  painting never clears; a different material is overwritten, not toggled.
- Undo/redo: a 30-cell stroke undoes in one step; redo restores it; a new
  edit drops the redo branch; the depth cap holds; settings changes are
  steps.
- Settings steppers stay in range and `None` round-trips as `auto`.
- Round trip: `Game::init` on a map, clone into the builder, edit, clone
  back, `Game::init` again, and the new round's obstacles match the edit
  (extend `mechanics_tests`' inline-map style).
- Dev server, through `DevServer::headless()` on a `Session`: `build` opens
  the dialog mid-round and skips it on the end screen; `build {answer}`
  both ways; `play` starts a round whose obstacles match the builder's map
  and leaves it frozen; `builder_paint` obeys the stroke rules and
  `builder_undo` reverses it; `builder_settings` round-trips `null` as
  auto and reports CLI overrides; `builder_map {map_toml}` loads and
  resets the baseline; `click` on the BUILD slot equals `build`; `key
  {tab}` equals it too; `restart {map_toml}` replaces the builder's map;
  `step` in Build is refused; `status.mode` reports the mode.
- `bbmcp`'s existing test that the advertised tool list matches `TOOLS`
  covers the new rows for free.

Windowed (`just run-dev` and the `mcp__bongbong__*` tools): the agent
session of section 11, screenshotting the play bar with `BUILD`, the
dialog, the build bar with a dropdown open, the settings panel and the
round after `play`; `click` on the field in Build and confirm no order is
made; `click` outside an open dropdown and confirm the cell under it was
not painted.

On a phone: a PR preview (`pr-<N>.preview.bongbong.io`), BUILD -> leave ->
pick a tool from a dropdown -> paint and toggle-erase a few cells -> undo
-> MAP ▾ change tanks -> PLAY, with no mis-taps at phone width.

## 15. Open questions

1. Skip the leave dialog on the end screen (assumed yes)?
2. `Tab` as the mode key; is a second control wanted on the web page's
   overlay, next to Full screen?
3. Should PLAY from the builder also clear `--seed` so a player always gets
   a fresh layout, or keep the flag's meaning (assumed keep)?
