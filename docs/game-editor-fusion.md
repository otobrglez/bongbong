# PRD: the map builder inside the game

Status: requirements, 2026-09-10; implemented the same day (the code is the reference where the two differ). BB-140 (2026-10-13) changed the way between the modes: BUILD pauses the round instead of asking to leave it, and the builder's BACK comes back to it (sections 1, 2, 5, 6, 7 and 11). Supersedes the build-mode half of
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
**mode of the game**. In play mode the HUD's top-right corner ends in a
`BUILD` button. Pressing it pauses the round - nothing is asked, nothing is
lost - and play's corners give way to the builder's plates over the same
world, which becomes the map canvas: the *authored* map the round was built
from, drawn as that round showed it. `BACK` at the top-left comes back to
the paused round where it stood; `PLAY`, in the very box `BUILD` stood in,
starts a fresh round on the edited map. The builder ships to every
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
| Return to play | BACK comes back to the round BUILD paused, where it stood, the edits made since put away as one undo step; PLAY starts a fresh round on the edited map. (Until BB-140 a "Leave this round?" dialog asked first and BUILD gave the round up for good.) |
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
reversed cheaply: PLAY keeps `--seed`'s meaning. The app starts in play mode; `--editor` initialises the
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
  for the bar itself and is not part of this work. *Status: the chrome is
  in UI points now - the builder's controls stand on plates in play's
  corner style, 44 pt on a touch screen (sections 7 and 10).*
- **Validation before PLAY.** A map with no start cell, no bong, or an
  unreachable bong plays through `Game::init`'s existing fallbacks, as it
  does today from `-m`. The linter stays a dev tool.
- Multi-select, copy/paste, rectangle fill, map size changes, a second
  field size, panning or zooming. *Status: map size changes, field sizes,
  the builder's own camera (pan, zoom, FIT), a rectangle select tool
  with move, copy, paste, flips and stamps, and rectangle, flood and
  scatter fills have since landed - sections 9 and 10,
  docs/large-maps-follow-camera.md section 9.*
- Editing the live, shot-up battlefield. Build mode edits the map as
  authored; a wall the round destroyed is back in the builder.

## 5. The two modes

```
             BUILD (corner / Tab): the round pauses behind the builder
   Play  ----------------------------------------------------->  Build
   round  <-----------------------------------------------------  canvas
             BACK (plate / Esc): the same round, edits put away
             PLAY (plate / Tab): game.map = builder.map; game.init()
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
- **The canvas is the round's world.** BUILD dresses the canvas in the
  looks of the round it paused (`MapEditor::dress`, an
  `editor::RoundLook`): each wall material's variant, each prop's and
  tree's variant by cell and the ground's seed (`obstacle::Looks`, which
  `battlefield::spawn_from_map` fills; `Game::looks`), each couch seat's
  chassis and the round's seed - kept after BACK and PLAY until the next
  BUILD brings a newer round's. The canvas is the map as a round stands it
  (`editor::still::Still`: worked out whole on a whole-map edit, patched
  cell by cell through a stroke, its woods settling when the stroke is
  done), drawn by the round's
  own painters: the tiles with their shadows and caps, the trees, the
  grass and the woods' floor, each start as its seat's tank at round size
  with `P1` or `P2` over it, the bongs with their rings, the towers facing
  the way a round opens them, the lamps, crates, doors, flags and oil, on
  the round's ground with its darkened field edge - the grass swaying, the
  trees shimmering and the bongs idling on the wall clock. A start draws
  the chassis its map names for its seat, else the paused round's, else
  the first; a tile placed since BUILD draws a variant hashed from its
  cell. What a round adds to its map - damage, wear, fires, the enemies -
  stays out. `still_tests::the_still_picture_is_the_rounds` holds the
  picture to a round's, and
  `a_stroke_patches_the_picture_to_the_one_worked_out_whole` a patched
  picture to one worked out whole.
- **The same frame and the same sky.** The canvas fills the window as
  play's field does (section 7), and it previews its map's sky through
  the round's weather passes: the one the paused round's seed picks from
  the map's skies (`MapEditor::map_sky`), so an unchanged map shows the
  very sky the round had, lit by the canvas's own starts, bongs, crates,
  lamps, towers, portals, lava and craters (docs/weather.md, "The
  builder's sky"). DAY draws it by daylight for editing.
- **Build -> Play by PLAY** clones the builder's map into `game.map` and
  calls `Game::init` for the field size. It is the dev server's `restart
  {map_toml}` path without the TOML. Every PLAY is a new round: new spawn
  rolls unless `--seed` pinned them, mission banner as on any restart. It is
  the one thing that gives the paused round up. It comes in by the swap
  below, never by the dissolve a round taking its way from an end screen
  gets.
- **Build -> Play by BACK** (`Session::back`) comes back to the round BUILD
  paused, exactly where it stood: the same frame, the end screen if it was
  one, nothing re-rolled. The round runs on its own map, so the canvas goes
  back to that map too: the edits made since BUILD are put away as one
  undo step (`MapEditor::put_away`, the baseline as it was then coming
  back with it) that the next BUILD's UNDO takes back, and a line under the
  left cluster says so for `ui_put_away_seconds`. Nothing is lost, so
  nothing asks. BACK dims where there is no round to go back to: after
  FILE's "switch to the original?" was answered yes for the map the round
  is on, which ends that round as it would in play.
- **The swap.** BUILD, BACK and PLAY leave the world where it stands and
  change the chrome over it (`mode::Swap`, over `ui_swap_seconds`, 0.18 s,
  eased out): the world the window left - a window-sized picture `app.rs`
  takes the frame the swap starts - fades out over the one it opens on,
  the builder's plates slide in from the window's top and bottom edges,
  and play's corners fade up. At 0, or under reduced motion, it cuts.
- `--editor` keeps working and means "start in Build": the game is
  initialised as usual (so PLAY has everything it needs) and paused behind
  the builder, so BACK plays it from its first frame.
- The builder compiles into every build. The `map-editor` cargo feature is
  removed: `Cargo.toml`, the `#[cfg]`s in `main.rs`/`lib.rs`/`editor.rs`,
  the justfile/CI flags. The eraser's icon is drawn in code
  (`editor::render::draw_eraser_icon`), so nothing extra loads.

## 6. Play mode: the BUILD button

- `BUILD` stands at the right end of the HUD's top-right corner cluster
  (`hud::corners`' mode button, `MODE_BUTTON_W` wide), drawn by
  `render::hud` as the corners' buttons are: its word and its outline in
  the builder's amber accent. The builder's PLAY stands in exactly that box
  wherever the corners are drawn at their full size (`chrome_tests`), so
  the way into the builder and the way out are one place under a finger.
- A click or tap on it, or `Tab`, enters the builder at once, mid-round, on
  the end screen, paused and in the intro alike. The round is paused by
  construction: outside play mode the driver does not call `Game::update`
  (and the dev server's `advance` is skipped), so no simulation pause flag
  is touched, and a round paused for an hour comes back bit for bit. With
  the players dialog or the level select up, the press closes that instead.
- The corners swap for the builder's plates (section 5, "The swap"), the
  world staying where it stood.
- Coming back is the builder's `BACK` (section 5): the round resumes at
  once, with no "get ready" beat, as after the pause button.
- Release builds, web included, have the button. The dev overlay label and
  the version line stay where they are.

## 7. Build mode: the plates and the tray

The canvas fills the whole window, as play's field does
(`editor::chrome::BuilderFrame`): an arena through the very view a round of
it uses (`View::fit_capped` under the round's scale cap) - at FIT with
play's margin world round it (`render::margin::MarginFx`) and the window's
backdrop past that -, a field map's canvas made to the window's shape
(`chrome::canvas_frame`). The safe area keeps only the chrome. The
builder's controls stand on three plates in play's corner style
(`editor::chrome::Bar`, drawn by `render::hud::draw_cluster_plate`: play's
plate fill, `PLATE_PAD` round one row of buttons), floating over the canvas
as play's corners float over the field. `Bar::of` lays them out from the
safe area in UI points; the painter, every hit test and the dev server's
`status.builder.buttons` read it, and `chrome_tests` pins that no two
buttons overlap and every one stands inside its plate and the safe area on
every window from a phone to a 1440p monitor.

```
 ┌───────────────────────┐          ┌────────────────────────────────────────────┐
 │ [< BACK] [DAY] NAME * │          │ UNDO REDO FILE▾ MAP▾ FIT CHECK ⚑ HERE PLAY │
 └───────────────────────┘          └────────────────────────────────────────────┘
                                                                   ┌─────────────┐
                    the canvas, the whole window                   │  navigator  │
                                                                   └─────────────┘
 ┌────────────────────┐ ┌────────────────────────────────────────────┐
 │ WALL: BRICK 12,7 … │ │ [▣▾] [▣▾] [▣▾] [▣▾] [▣▾] [▣▾] [BRUSH▾] [⌫] │
 └────────────────────┘ └────────────────────────────────────────────┘
   the status plate       the tray, centred along the bottom
```

| Plate | Where | What it holds, in order |
| --- | --- | --- |
| Top-left | the area's top-left corner, where play's vitals stand | `BACK`: a left-pointing arrow and the word; dim with no round to go back to, an amber dot while the canvas differs from the paused round's map. `DAY` (`BarButton::Day`), only while the map's sky is not clear: a press draws the canvas by daylight, another puts it under its sky again, the box washed while it is down (section 5). The map's name in capitals (`MapFile::name`: `default` for the embedded map, `untitled` for none) with ` *` while edited since load. |
| Top-right | the area's top-right corner, where play's right cluster stands | `UNDO` and `REDO` (dim with nothing to take), `FILE ▾` (`LOAD...`, `SAVE` and `SAVE AS...` where a map can be written, `REVERT MAP` for a changed shipped map, `CLEAR MAP`), `MAP ▾` (section 9), `FIT`, `CHECK` and the clear flag, then `PLAY HERE` and `PLAY` at the right end - `PLAY` in the box play's corner draws `BUILD` in (section 6). Back at one end, go at the other. A button on either top plate is a play corner button's height, `hud::button_height`: 32 pt with a mouse, 44 on a touch screen. |
| Tray | centred along the area's bottom | The six category buttons - `WALL`, `PROP`, `GROUND`, `PLANT` (the trees and bushes, docs/BUSHES_SPEC.md §4), `ACTOR`, `PICKUP` -, then `BRUSH` and the eraser. Its row is 36 pt with a mouse, so a tool's 32 px icon stands in its box a texel a point, and 44 on a touch screen (`chrome::tray_button_height`). A category button shows its category's *current* tool as its icon, with a caret: its icon half selects that tool, its list half opens the list, the wheel over it cycles inside the category; it is outlined in the accent while its category holds the brush. The tool's name is the status line's. The eraser is a plain brush that clears cells, outlined while it is the brush. |

**Short of room nothing shrinks; it folds.** Along the top the name
narrows and then goes (the status line names the map instead), then BACK's
word (its arrow stays), then the clear flag folds into CHECK, which turns
the flag's green once the map is cleared. Along the bottom, where the six
category buttons, BRUSH and the eraser would take more than `TRAY_MOST`
(three quarters) of the area's width - a phone's - the six and BRUSH fold
into one `TOOLS` button whose palette holds every category - PICKUP's
groups a row each where they fit - and the brush's row, which leaves a
phone's canvas its bottom edge. Every button keeps its
full size, 44 pt both ways on a touch screen (`hud::UI_TOUCH_PT`).

**What stands round the plates.** The status line is a plate of its own
(`chrome::status_plate`), its words in capitals: the map's name where the
top has no room for it whole, the active category and tool, the cell under
the pointer (the last tapped one on touch) and what is in it, a
rectangle's size, and any message. It stands beside the tray in the area's
bottom-left corner where that leaves it `STATUS_MIN_W` (200 pt), else at
the area's left just above the tray. The navigator stands where play's
minimap does, under the top-right plate and flush with its right edge
(docs/large-maps-follow-camera.md section 9); the select tool's strip
hangs from the top-left of the room under the top plates.

Categories and their tools, in list order:

| Category | Tools (30 with the eraser) |
| --- | --- |
| WALL | brick, iron, wood, glass, concrete |
| PROP | sandbag, barrel, oil drum, fuel drum, fence, tree, pine |
| GROUND | road, water, tall grass, oil trail, gate |
| ACTOR | p1 start, p2 start, bong, enemy bong |
| PICKUP | in four groups (`editor::PickupGroup`): SUPPLIES - ammo, health, shield, heat shield, speed-up, bong pack, tower pack; WEAPONS - laser, minigun, plasma, missiles, flamethrower, grenades; SPECIALS - sonic hammer, EMP burst, gauss rail, FPV swarm, rod from god, gravity well; AIR SUPPORT - chopper, jet |

Each category remembers its current tool for the session; the initial tool
is the first in each list - PICKUP's is AMMO, so its button shows a crate of
supplies rather than a heal - and the active brush at entry is `WALL / brick`.

Every button on the plates - and on the select tool's strip, and the Save
prompt's SAVE - is framed as play's corner buttons are
(`editor::chrome::button_box`, `Face` in `editor/render.rs`): its slot less
4 pt at its right end and a 2 pt block above and below - 28 pt tall on a
top plate with a mouse, 32 on the tray, 40 on a touch screen - inside a
square 2 pt outline; every word in it at 14 pt (`BAR_SMALL_TEXT` - the top
plates hold their row at `hud::UI_MIN_W`, so the words cannot take the
corners' 18). Its colours say what it is: BACK in the text colour, outline
and word, as play's pause button is; the edit actions a quiet grey outline
round a word in the text colour, fainter round a dim word while they cannot
act (UNDO with nothing to undo, FIT at FIT); CHECK in the clear flag's
green, outline and word, once the map is cleared; PLAY, PLAY HERE, PLACE
and SAVE in the amber, outline and word, as play's corner draws BUILD. A
box is washed while its popup is up and outlined in the amber while it
holds the brush in force.

`FIT`, after `MAP ▾`, puts the builder's camera back on the whole canvas;
it is dim while the canvas is already at FIT. The camera itself - zoom,
pan, the gestures, edge scroll - is docs/large-maps-follow-camera.md
section 9 and section 10 below.

`CHECK`, after FIT, opens the CHECK panel: the clear check's row, then the
map linter's findings, a jump to each and its quick fix. After it a
chequered flag is the clear check at a glance: green with the par beside
it once this revision of the map has been won from plain PLAY, dim while
it has not; a press opens the CHECK panel too. `PLAY HERE`, just before
PLAY, starts the round from the middle of the view instead of the map's
start. All three are docs/large-maps-follow-camera.md section 9.

`BRUSH`, after the six category buttons, pictures how a press on the
canvas works and opens a list of the ways: the brush's shapes - PEN, RECT
(a rectangle filled on the release), FILL (a flood of the pressed cell's
region) and SCATTER (a hashed share of the empty cells round the drag) -,
the select tool and the STAMPS list; a tray folded into TOOLS carries it
as the palette's last row. With the select tool a strip of its actions -
COPY, CUT, PASTE, the flips, DELETE, + STAMP, STAMPS, or PLACE, the flips
and CANCEL while a paste ghost stands - hangs from the top-left of the
room under the top plates (`editor::chrome::Strip`;
docs/large-maps-follow-camera.md section 9).

### Dropdowns

- A category's list, BRUSH's list and the TOOLS palette rise from their
  button on the tray into the room between the top plates and the tray
  (`BuilderFrame::over_tray`, `chrome::tray_list`/`tray_palette`); on a
  window too short for their columns there they hang under the top plates
  down the whole room, over the tray. FILE's menu and the MAP and CHECK
  panels hang from their button under the top-right plate, and the Load
  list, the STAMPS list and the Save prompt are centred in that room
  (`BuilderFrame::room`). Rows are **48 pt tall and 200 pt wide**: the
  32 px icon with 8 pt inset, the name in 18 pt, the current tool
  highlighted; a list too long for its room runs on in a second column,
  its rows shortening where the room is short, never under 44 pt.
- PICKUP's list stands its 21 crates in their groups' columns
  (`chrome::tray_tools`, `headed_list`): SUPPLIES; WEAPONS over AIR
  SUPPORT; SPECIALS - each group under its heading, in the accent over a
  rule as the MAP panel heads its groups (`LIST_HEADING_H` 28 pt, 8 pt
  between two groups of a column). Three columns of 200 pt, nine rows at
  most; where the room is too short for them even at 44 pt rows the list
  falls back to plain rows, still group by group. The folded palette gives
  each group a row of its own under its short name (`pickup-group-*-short`:
  AIR), and keeps PICKUP one row where those rows would not fit the room
  (`chrome::Shelf`).
- Every list, panel and prompt stands on the chrome's one dark panel
  (`render::hud::draw_panel`: a near-opaque fill, corners 8 pt round, a
  4 pt drop shadow and a thin dark edge) - the one play's questions, the
  lobby and the level select stand on too.
- One popup at a time: opening any dropdown or the settings panel closes
  the others.
- Closes on pick, `Esc`, or a press anywhere else. **That press is
  consumed**: it neither places a cell nor selects another tool, so a tap
  to dismiss a menu on a phone cannot paint through it.
- The singleton badge - a rimmed dot of 2 pt blocks in the clear flag's
  green at the icon's top-right, while a bong, start, start2 or enemy bong
  is already placed - stands on the dropdown row and the palette's cell,
  and on the category button (or TOOLS) when that tool is current.

### Marks on the canvas

The canvas is the map as a round stands it (section 5); what only the
builder shows is drawn over it after the sky, so the dark never hides it.
The gates and the reaches are whole 2 px blocks of the palette's mark
colours, as play's indicators mark the field (`indicators.rs`): each
gate's chevron pointing in from its edge in `indicators::GATE_AMBER`
rimmed in `indicators::RIM`, a square frame of the same blocks round a
gate off the edge or a portal off its network, each tower's reach a faint
ring of blocks in its side's colour. Over them stand each start's `P1` or
`P2` over its tank, the select tool's marching outline, what the brush's
shape is about to do, the CHECK panel's picked finding and the hover
highlight. A cell the select tool carries or a paste ghost holds is drawn
on its own as a tool's icon (`draw_cell`), a start or the enemy bong
ringed in blocks of its colour - its seat's, or `indicators::HOSTILE`.

### Files

- `FILE ▾` opens a menu of the same row style. `LOAD...` opens the **Load
  list**: a panel centred in the room under the top plates, 48 pt rows,
  at most `LOAD_VISIBLE_ROWS` (eight) a page, turned by a pager row and the
  wheel, every map `map::available_maps` offers -
  the `maps/*.toml` files on native plus the maps shipped inside the
  binary (`default`, `default-desert`, `hunt-basic`, `waves-basic`, `portals`, marked `shipped`), which
  is all the web build can list since nothing outside `static/` ships in
  the wasm. Picking a row loads the map as one undo step and the new
  baseline; no confirmation, undo covers a mistake. Each row shows the
  map's thumbnail - its minimap, made as the list's page shows it - and
  its size in cells (`editor::thumbs`, docs/large-maps-follow-camera.md
  section 9).
- `SAVE` writes the map back to `maps/<name>.toml` when it has a name,
  otherwise it behaves as `SAVE AS...`, which prompts for one (letters,
  digits, `-`, `_`). Saved is the new baseline, so the ` *` clears. Both
  rows exist only where `map::saving_available` is true: native. The web
  menu is `LOAD...` and `CLEAR MAP`; its edits live in memory for the
  session.
- `CLEAR MAP` empties the canvas of every placed object - walls, props,
  trees, ground, actors, gates, pickups - and keeps the map's settings,
  size, theme and name: a map started from scratch without an empty file
  on disk first. One undo step, not a new baseline, so the ` *` shows until
  the empty canvas is saved; no confirmation, undo covers a mistake. An
  already empty canvas records nothing.

## 8. Placing, erasing, undo

- A press on the field places the active tool at the cell under it; a drag
  paints every new cell it crosses, once per cell (today's `drag_cell`
  rule). Singletons (start, start2, bong, enemy bong) move rather than
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
  (`Cmd` on macOS) and UNDO and REDO on the top-right plate. The stack is
  cleared on PLAY and on RESET, not on switching modes and back.
- `dirty` means "differs from the map as loaded at startup" (kept as a
  baseline copy), since there is nothing to save to. It drives the ` *` and
  the RESET row.

## 9. The MAP ▾ settings panel

A panel hanging from MAP under the top-right plate, on the chrome's one
dark panel (`render::hud::draw_panel`, section 7). Its values are the
`MapFile` fields, so nothing new is stored. `editor::chrome::MapPanel` is the one
geometry table the painter, every hit test and the dev server's
`status.builder.buttons` read.

**Four groups, two shapes.** The fields are in four groups (`MapTab`):
ROUND (the mission and the enemies), TANKS (the players' chassis), FIELD
(the look and the size) and SKY (the skies a round may take), plus RESET
MAP.
- **Sections:** where the room under the top plates holds the whole panel
  (884 x 432 pt; a desktop, a tablet), every group stands at once under
  its heading. ROUND and TANKS fill the left column; FIELD, SKY and RESET
  MAP fill the right.
- **Tabs:** where it does not (a phone in landscape, a small window), a
  rail of tabs down the left shows one group at a time, and RESET MAP is
  the rail's last slot. The wheel walks the tabs.

Nothing pages, and every button is at least 44 pt both ways.

**A row** is one control with its label beside it, or several side by side
with their labels over them. A choice is a row of options with the one in
force filled; a press picks it. A stepper has `-`/`+` (numbers) or `<`/`>`
(choices) at its ends and the value between. A sky is a tile that a tap
puts in or takes out.

| Group | Field | Control | Values |
| --- | --- | --- | --- |
| ROUND | `mission.kind` | choice | protect, hunt, destroy |
| ROUND | `spawn.kind` | choice | band, waves |
| ROUND | `tanks` (band) | stepper | 0..=31 (`wave_max_alive`); below 0 is `auto` (`None`, the knob's roll) |
| ROUND | `spawn.waves` / `size` / `growth` (waves) | three steppers | 1..=20 / 1..=31 / 0..=10, each `auto` = the `waves` tuning group |
| ROUND | `spawn.tier_start` / `tier_end` (waves) | two steppers | auto, light, medium, heavy, super |
| TANKS | `tank` / `tank2` | stepper | `auto` (`None`) then the 12 `TankKind` names in row order, the picked chassis drawn in that seat's team colours |
| TANKS | `bong` / `enemy_bong` | two steppers, one row | YOUR BONG: Spike, Oggle, Gloop; FOE BONG: Blip, Puff, Capi, Fuzz. Shows the bong the map plays (its side's default where it names none, docs/BONG_SPEC.md); the canvas draws the bong cells as the picks |
| FIELD | `theme` | choice | the `map::Theme` names |
| FIELD | `size`, and the anchor | three steppers | WIDTH and HEIGHT in cells, a press each, 16 x 9 up to `map::MAX_SIDE_CELLS`; ANCHOR, where the old map sits when the size changes (a 3 x 3 glyph, the middle at first; the panel's, not the map's) |
| SKY | `weather` | nine tiles | each of `map::Weather::SKIES`; none in is clear, and CLEAR shows in then |
| | RESET MAP | button | Reverts cells and settings to the baseline. It asks nothing, since it is undoable. |

**Under SPAWN.** The band's TANKS count shows under `band`, and the wave
rows show under `waves`. Either way, ROUND keeps the room its wave rows
take, so a spawn change moves no other group under a finger. A hidden row
keeps its value, so a map keeps its wave numbers when switched back.

**Resizing.** A size change (`MapEditor::resize`) is one
`EditStep::Resize`: the whole map before and after, so the cells that
land past the new edge are dropped and one undo brings them back.
- A run of presses while the panel stays open folds into one step.
- An anchor on a side keeps that edge where it was, and the middle anchor
  shares the change, counted from the halves of both sizes, so one-cell
  presses come out centred.
- The ground is made again on the new field, and a zoomed view moves with
  the map (and back on undo).
- A map that crosses the arena limit (36 x 18) is framed as a field map
  from the next frame.

**CLI overrides.** Flags (`--enemies`, `--mission`, `--spawn`,
`--weather`, ...) still outrank the map's values at PLAY, exactly as they
outrank a file's. The panel shows the map's value and marks it `(cli)`
when a flag overrides it: by the label, and on the group's heading or
tab. A player who started the program with `-e 8` sees why TANKS is not
honoured.

## 10. Touch

- The canvas reads the touch points themselves (`BuilderInput::touches`,
  every finger with its id, through `View::to_bitmap`), not raylib's
  gestures, which read two points with 0.3 s thresholds. `editor::gesture`
  turns them into what they mean, with no timing but the tap's:
  - one finger paints once it has moved past `builder_touch_slop_pt`, or
    on a tap; a resting finger paints nothing;
  - two fingers pan and pinch about their midpoint, the pinch easing onto
    the nearest whole-block zoom when they lift; a second finger landing
    on a stroke takes the stroke back, since it meant a pinch;
  - a two-finger tap is undo and a three-finger tap redo, decided when
    the last finger lifts within `builder_tap_seconds`.
- **The paint threshold.** Below a cell of `builder_paint_min_cell_mm` on
  the glass a finger cannot hit one cell: a tap zooms in to a cell of
  `builder_tap_zoom_cell_mm` at that point and a one-finger drag pans.
  Millimetres come from the screen's points (`indicators::POINTS_PER_MM`,
  the window's units per point).
- A painting finger held near the canvas's edge scrolls it (edge scroll,
  the mouse's too).
- The plates, the strip, the navigator and the popups still take a finger
  as a press: a finger that lands on one is not the canvas's.
- Targets: the chrome is in UI points, so every button on the plates and
  the strip is 44 pt both ways on a touch screen (`hud::UI_TOUCH_PT`), and
  every list row, stepper and dialog button 44 pt or more. Each plate
  button's hit rect reaches `EDITOR_BAR_HIT_SLACK` (8 pt) above and below
  its drawn box, so a slightly low tap still lands.
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
| `mode` | none | Reports `mode` (`play`/`build`), `back` (`{puts_away}` in build mode with a round to go back to, else null), the builder's `dirty` flag, map name, active tool and category, which popup is open. Cheap; `status` carries the same `mode` and `back` fields. |
| `build` | none | Presses BUILD: enters the builder at once, the round paused behind it. Replies like `mode`. |
| `back` | none | Presses BACK in Build: the paused round comes back where it stood, edits since BUILD put away as one undo step. A no-op in Play and with no round to go back to. Replies like `mode`. |
| `play` | `intro?: bool` | Presses PLAY from Build: the edited map becomes the round's map and a fresh round starts, frozen in lockstep like `restart` (so `step` counts play frames; `resume` for real time). Replies with `status`. Fails in Play mode. |
| `builder_tool` | `tool?: name`, `shape?: "pen" \| "rect" \| "fill" \| "scatter"` | Selects a brush by name - every `editor::TOOLS` name (`brick`, `iron`, `wood`, `glass`, `sandbag`, `barrel`, `oil_drum`, `fuel_drum`, `fence`, `tesla`, `tesla_enemy`, `gun_tower`, `gun_tower_enemy`, `bio_slush`, `bio_slush_enemy`, `road`, `water`, `tall_grass`, `bush`, `berry_bush`, `juniper`, `fern`, `autumn_bush`, `reeds`, `tree`, `pine`, `spruce`, `scots`, `fir`, `birch`, `willow`, `palm`, `snag`, `forest`, `oil_trail`, `gate`, `portal`, `start`, `start2`, `bong`, `enemy_bong`, `health`, `ammo`, `laser`, `minigun`, `plasma`, `missiles`, `speedup`, `shield`, `flamethrower`, `bong_health`, `tower_pack`), `eraser` or `select` (the select tool) - through the category's own selection path, so the category button updates; `shape` is how it paints (BRUSH's list, docs/large-maps-follow-camera.md section 9). Without either, lists the categories with their current tool and the active one, and the shape. |
| `builder_paint` | `cells: [[col,row], ...]`, `tool?: name`, `shape?`, `button?: "left" \| "right"` | One **stroke**: presses on the first cell and drags through the rest, so the toggle-erase rule, singleton moves and the one-undo-step-per-stroke rule all apply exactly as for a mouse - with `rect` the first and last cells are the rectangle's corners, with `fill` only the first counts, with `scatter` every cell's footprint is laid. `button: right` erases. Replies with each cell's object before and after, the undo depth and the status line's `message` (a fill refused for its size). |
| `builder_undo` / `builder_redo` | `steps?: n` (default 1) | Undo or redo that many steps. Replies with the depth left on each side and the cells the last step changed. |
| `builder_settings` | `tanks?`, `tank?`, `tank2?`, `mission?`, `spawn?`, `waves?`, `wave_size?`, `wave_growth?`, `tier_start?`, `tier_end?`, `theme?`, `weather?` (each `null` = auto), `anchor?`, `size?: [cols, rows]`, `reset?: bool` | Sets the MAP ▾ values (each changed field is one undo step, in field order) or, with `reset`, reverts cells and settings to the baseline. `size` resizes the map about the `anchor` (section 9). Without parameters, reports the current values, `size` and `anchor`, and which ones a CLI flag is overriding. |
| `builder_camera` | `x?`, `y?` (a world point for the canvas's middle), `zoom?` (times FIT), `fit?: bool` | Frames the builder's own camera for a screenshot, or reports it: replies like `status.builder.camera` (`fit`, the world `rect` shown, `scale`, `zoom`, `fit_scale`, `device_scale`, `cell_mm`, the canvas `area`). |
| `builder_touch` | `frames: [[{id, x, y}, ...], ...]` in bitmap pixels, `dt?` | Raw touch frames through the same `BuilderInput` a touch screen fills, all fingers lifted after the last: strokes past the slop, pinch and pan, two- and three-finger taps, the paint threshold (section 10). |
| `builder_select` | `rect?: [c0, r0, c1, r1]`, `clear?: bool`, `move_by?: [dx, dy]`, `action?: "copy" \| "cut" \| "delete" \| "flip_h" \| "flip_v" \| "stamp" \| "paste" \| "place" \| "cancel"`, `at?: [col, row]` | The select tool as a drag and its strip drive it: a rectangle selected (the select tool taken), let go, its cells carried, then an action - `paste` stands the clipboard as a ghost with its middle on `at`, `place` puts the ghost down with its top-left on `at`. Each edit one undo step. Replies with the `selection`, the `ghost`, the `clipboard`, the `changes` and the undo depth. |
| `builder_stamp` | `name?: key`, `at?: [col, row]`, `place?: bool` | The STAMPS list: every stamp's `key`, name and size; with `name`, that stamp as the paste ghost (its top-left on `at`), put down at once with `place`. |
| `builder_map` | `name?: string`, `map_toml?: string`, `map?: path` | Without parameters: the **builder's** map as TOML, its name, `dirty`, and a diff against the baseline (cells added, removed, changed; settings changed). With one: loads that map (by Load-list name, as inline text, or from a path) into the canvas as a single undo step and makes it the new baseline - the "load a map to test" path, and the way an agent hands back a map it edited as text. `map_get` keeps returning the map the *current round* was built from, which differs from this once the builder is dirty. |
| `builder_files` | none | What `FILE > LOAD` offers: every loadable map with `on_disk`, and `can_save`. |
| `builder_save` | `name?: string` | `FILE > SAVE` / `SAVE AS`: writes `maps/<name>.toml` (native), makes it the baseline. Replies like `builder_map`. |
| `click` | `x`, `y` (window pixels), `button?: "left" \| "right"`, `drag_to?: [x, y]` | A raw press at a window position, in either mode: the plates' buttons (`status.builder.buttons` names each one's rect), a dropdown row, a dialog button, a stepper, a field cell. With `drag_to`, a press, a straight drag to the second point and a release, crossing every cell on the way. In Play mode a left `click` on the field is a `tap`; in Build mode the reply names the world point and the cell under the press, through the builder's camera. This is how the *UI* is tested, as opposed to the model the tools above address directly. |
| `key` | `key: "tab" \| "escape" \| "enter" \| "undo" \| "redo" \| "copy" \| "cut" \| "paste" \| "delete" \| "zoom_in" \| "zoom_out" \| "left" \| "right" \| "up" \| "down" \| ...`, `text?: string` | Presses one key (or types `text` into an open prompt) for one frame through the same `BuilderInput`. `undo`/`redo` are Ctrl+Z / Ctrl+Y, `copy`/`cut`/`paste` Ctrl+C / Ctrl+X / Ctrl+V and `delete` Delete on the selection, `zoom_in`/`zoom_out` are `+`/`-` (one step about the canvas's middle) and the arrows pan the canvas for a frame. |

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
build                                  -> build, tool wall/brick, the round paused
screenshot                             -> the canvas as the round showed it, the plates over it
status                                 -> builder.buttons: each button's window rect by name
click {x, y} inside list_wall          -> WALL's list open, risen from the tray
screenshot
click {x, y} inside tool_iron          -> picked iron from the list
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
  points too. *Status: in place (sections 7 and 10).*
- **Validation**: a `maplint`-light pass at PLAY that warns in the bar
  (`no start cell`, `bong unreachable`) is a natural next step once the
  linter's checks are callable on a `MapFile` without a full `Game::init`.
- **New/Load for players**, a map picker, sharing a map as text.

## 13. Code seams

| Where | Change |
| --- | --- |
| `mode.rs` (new) | `Session { driver: Driver, game: Game, builder: MapEditor, .. }` with the transitions as methods: `press_build`, `back`, `play`, `replace_map` (what `restart {map_toml}` calls). Both the window and the dev server go through these, so a tool and a click are the same path. No `RaylibHandle` in this file. |
| `main.rs` | Gathers a `BuilderInput` (mouse/touch press, drag, wheel, `Tab`/`Esc`/`Enter`/Ctrl+Z/Ctrl+Y, typed text) in Build mode the way it gathers `simulation::Input` in Play, hands it to `Session`, and renders whichever mode is live. The `--editor` branch collapses into "start in Build". Feature `#[cfg]`s go. |
| `Cargo.toml`, `justfile`, `.github/` | Remove `map-editor`; the builder is unconditional. `dev-tools` gates only the dev `SAVE`. |
| `hud.rs` | `BUILD` is a corner button (`hud::corners`, `CornerButton::Build`), its rect the one the painter and the hit test read, so play-mode chrome stays in one file. |
| `game.rs::render` | Draws the play chrome last, after the bar, since `render` owns `begin_drawing`. |
| `editor.rs` | The palette, the hamburger-era `Close`, `New`, `Load` and the yellow debug line go. New: `Category` with a current tool each, dropdown state, the toggle-erase rule in `place`, an `UndoStack` of `Vec<(cell, before, after)>` steps plus settings steps, the settings panel, `EditorAction::Play`. **`update` takes a `BuilderInput` instead of reading raylib**, so the builder is drivable headlessly; the builder's controls are one geometry table (`editor::chrome`: `Bar`, `PopupLayout`) the painter and every hit test read. `MapEditor::new` takes the baseline map and keeps a copy for RESET/dirty. Direct entry points for the tools: `select_tool(name)`, `stroke(cells, button)`, `undo/redo(n)`, `set_settings(patch)`, `load(map)`, `map_diff()`. |
| `map.rs` | Nothing: `MapFile` already derives `Clone`, and the settings panel edits existing fields. |
| `lib.rs` | The `EDITOR_PALETTE_*` constants become dropdown/panel constants; `Layout` unchanged. |
| `devserver.rs`, `bin/bbmcp.rs` | `before_frame`/`advance` take the `Session` rather than a bare `Game`. `status` gains `mode`; the ten tools of section 11 are ten `ToolSpec` rows plus `dispatch` arms (`bbmcp` needs nothing, it reads `TOOLS`); the game-only tools refuse in Build with an `isError` naming `play`. Screenshots work in both modes (the presented frame). |
| `site/` | No change. |
| CLAUDE.md | Module map entries for `editor.rs`, `hud.rs`, `main.rs` once it lands. |

## 14. Testing

Headless (`cargo test --lib`, extend `editor_tests`/`hud_tests`):

- The plates' buttons do not overlap and stand inside their plates and the
  safe area on every window, with DAY and without, PLAY in the box play's
  corner draws BUILD in; every popup fits its room, its rows at least
  44 pt (`chrome_tests`).
- The canvas is a round's picture: the same tiles, trees and tufts with
  the same variants, edge masks and crowns as a round of the map
  (`still_tests`); BUILD dresses the canvas in the round's looks and keeps
  them through BACK and PLAY, the plates swap on BUILD, BACK and PLAY, and
  PLAY starts with no dissolve, even from an end screen (`session_tests`); DAY shows only
  under a dark sky and draws the canvas by daylight (`editor_tests`).
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
- Dev server, through `DevServer::headless()` on a `Session`: `build`
  enters at once and `back` comes back; `play` starts a round whose obstacles match the builder's map
  and leaves it frozen; `builder_paint` obeys the stroke rules and
  `builder_undo` reverses it; `builder_settings` round-trips `null` as
  auto and reports CLI overrides; `builder_map {map_toml}` loads and
  resets the baseline; `click` on the BUILD slot equals `build`; `key
  {tab}` equals it too; `restart {map_toml}` replaces the builder's map;
  `step` in Build is refused; `status.mode` reports the mode.
- `bbmcp`'s existing test that the advertised tool list matches `TOOLS`
  covers the new rows for free.

Windowed (`just run-dev` and the `mcp__bongbong__*` tools): the agent
session of section 11, screenshotting play's corners with `BUILD`, the
builder's plates with a list open, the settings panel and the round after
`play`; `click` on the field in Build and confirm no order is
made; `click` outside an open dropdown and confirm the cell under it was
not painted.

On a phone: a PR preview (`bongbong-pr-<N>` on workers.dev), BUILD -> pick
a tool from a dropdown -> paint and toggle-erase a few cells -> undo
-> MAP ▾ change tanks -> PLAY, with no mis-taps at phone width.

## 15. Open questions

1. Skip the leave dialog on the end screen? Settled by BB-140: there is no
   leave dialog; BUILD pauses the round everywhere and BACK comes back.
2. `Tab` as the mode key; is a second control wanted on the web page's
   overlay, next to Full screen?
3. Should PLAY from the builder also clear `--seed` so a player always gets
   a fresh layout, or keep the flag's meaning (assumed keep)?
