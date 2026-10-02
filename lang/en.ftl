# The English catalogue - the source language, and what every other
# language falls back to per message (docs/localization-prd.md section
# 4.1). Every string a player reads is a message here; `src/text.rs` names
# each one as a `text::keys` constant and a test holds the two lists
# together, so a message with no constant or a constant with no message
# fails `cargo test`.
#
# Style, for every language:
# - The chrome is in capitals by design. Write the final form; nothing is
#   upper-cased at runtime.
# - Every string is drawn *folded*: a letter outside the game's font
#   (Basic Latin and Latin-1) is drawn as its base letter - Č as C, Š as
#   S, Ž as Z (`text::fold`). Write the language properly, with its
#   diacritics; they are folded on the way to the screen.
# - The comment above a message names where it is drawn and how much room
#   it has (its budget in pixels at its text size, roughly as letters).
#   `text_tests::every_language_fits_every_budget` measures every
#   language against every budget, so a translation that does not fit
#   fails the build rather than overflowing its neighbour.
# - Placeholders are named (`{ $n }`) and may be reordered. Plurals are
#   Fluent selectors, so a language writes as many forms as it has.

## The HUD bar (src/hud.rs, src/render/hud.rs)

# Label over a 36 px gauge in a 40 px slot, 10 px text: about 6 letters.
hud-speed = SPEED
hud-shield = SHIELD
hud-frog = FROG

# The bar's buttons at its right end, 18 px text. BUILD/PLAY share a 72 px
# slot (about 6 letters), LEAVE the same, ONLINE an 80 px one.
button-build = BUILD
button-play = PLAY
button-leave = LEAVE
button-online = ONLINE

# The players dialog: a 28 px title and a 16 px line across a 440 px
# panel, then two 176 px buttons in 18 px text (about 14 letters each).
# The line names the controls in the hands that last pressed something:
# the keys after a key press, and after a touch the stick and the tap of
# player 1 - player 2 is always the keyboard's.
players-title = How many players?
players-keys = P1 arrows + Space    P2 WASD + L.Shift
players-touch = P1 drag + tap    P2 WASD + L.Shift
players-one = 1 PLAYER
players-two = 2 PLAYERS

# The leave-round dialog, the same shape.
leave-title = Leave this round?
leave-sub = Your progress is lost. The map is kept.
leave-confirm = LEAVE ROUND
leave-stay = KEEP PLAYING

## The round (src/render/game.rs, src/level.rs, src/simulation/waves.rs)

# The end screen: a 72 px banner, measured and centred over the whole
# field, and the 28 px countdown under it.
round-won = YOU WIN
round-lost = YOU LOSE
round-restarting = Restarting in { $seconds }...
round-back-to-lobby = Back to the lobby in { $seconds }...
paused = PAUSED

# The mission as one word: the HUD bar's title, where it shares a 158 px
# slot in 18 px text with " 12/12" in a wave round (about 8 letters), and
# the lobby's mission stepper.
mission-protect = PROTECT
mission-hunt = HUNT
mission-destroy = DESTROY

# The 72 px banner the round opens with, centred over the field.
mission-protect-banner = PROTECT THE FROG!
mission-hunt-banner = HUNT THE FROG!
mission-destroy-banner = DESTROY!

# The 48 px banner during the breather before a wave rolls in.
wave-banner = WAVE { $n }
wave-final = FINAL WAVE

## Levels (src/levels.rs, docs/levels.md)
#
# A level's title is written in levels.toml, in English; another language
# translates it as `level-<map>` (level-lotus-lagoon = ...), drawn at
# 36 px under the mission banner in about 700 px (some 30 letters).

# Over the mission banner as a level opens, 28 px: "LEVEL 3 / 14".
level-number = LEVEL { $n } / { $count }

# The end screen's numbers under YOU WIN / YOU LOSE, 28 px, the two side
# by side in about 340 px each (some 17 letters): how long the round took
# ($time is m:ss) and how many enemies it wrecked of all it brought.
result-time = TIME { $time }
result-wrecks = DESTROYED { $n } / { $total }

# Over those once the last level is won, 28 px, about 700 px.
result-all-clear = ALL { $count } LEVELS COMPLETE!

# A level's end-screen buttons, 18 px text in a 224 px button (about 18
# letters): the same level again, the next one after a win, and the first
# one again after the last.
result-again = PLAY AGAIN
result-next = NEXT LEVEL
result-first = BACK TO LEVEL 1

# Beside those, in a 160 px button: the level select.
result-levels = LEVELS

# The same two buttons while the end screen counts down to taking them by
# itself - NEXT LEVEL after a win, PLAY AGAIN after a loss - in the same
# 224 px (about 17 letters with a two-digit $seconds, which counts 3, 2, 1).
result-next-in = NEXT LEVEL IN { $seconds }
result-again-in = PLAY AGAIN IN { $seconds }

# The level select, over the dimmed field: its title (22 px) and the line
# under it (12 px), both in about 660 px, and its one button (18 px text in
# 140 px) that goes back to the round or the end screen it opened over.
levels-title = LEVELS
levels-sub = Pick a level to play. Winning one opens the next.
levels-back = BACK

# The bar's level button, in the mission word's place on a level: this
# word small (10 px) with the level's number after it, in about 44 px.
bar-level = LEVEL

# The locate label over a player's tank and the lobby's seat column: the
# seat's number. Keep it short - it sits over a 48 px hull.
seat-label = P{ $n }

# The touch hints over each half of the field, 20 px text, drawn once,
# the first time a touch lands - and not while the keys are in use.
touch-steer = DRAG TO STEER
touch-fire = TAP TO FIRE

## The lobby (src/lobby.rs, src/render/lobby.rs)

# The 22 px title across the 664 px panel.
lobby-title-start = ONLINE CO-OP
lobby-title-code = JOIN A ROOM
lobby-title-waiting = REACHING THE ROOM
lobby-title-closed = THE ROOM IS GONE
lobby-title-host = YOUR ROOM
lobby-title-guest = IN THE ROOM

# The 12 px line under the title, one line across the 664 px panel.
lobby-sub-start = Host a room and share the code, or join one.
lobby-sub-code = Five characters, from the code you were given.
# `{ $host }` is the rooms server being dialled.
lobby-sub-waiting = { $host }...
lobby-sub-closed = Nothing is listening any more.
# In the room the line runs along the bottom between the buttons, 10 px.
lobby-sub-host = Scan the code or read it out. START when everyone is ready.
lobby-sub-guest = Waiting for the host to start the round.
# Once a round is over: the outcome, then whose move it is.
lobby-outcome-won = ROUND WON.
lobby-outcome-lost = ROUND LOST.
lobby-outcome-over = ROUND OVER.
lobby-rematch-host = REMATCH when everyone is ready.
lobby-rematch-guest = Waiting for the host's rematch.

# The two steppers' labels, 18 px in a 152 px column.
lobby-map = MAP
lobby-mission = MISSION

# A seat row: its state at the right in 10 px text (about 20 letters),
# and what an empty row says.
seat-away = AWAY
seat-host = HOST
seat-ready = READY
seat-waiting = WAITING
seat-empty = EMPTY
# Seats past the four rows the panel draws, 10 px under the rows.
lobby-more = { $n ->
    [one] +{ $n } MORE
   *[other] +{ $n } MORE
}

# The buttons, 18 px text: the two wide ones are 200 px (about 16
# letters), the rest 140 px (about 11 letters), KICK 72 px (about 5).
lobby-host = HOST A ROOM
lobby-join = JOIN A ROOM
lobby-back = BACK
lobby-close = CLOSE
lobby-delete = DELETE
lobby-confirm = JOIN
lobby-ready = READY
lobby-im-ready = I'M READY
lobby-start = START
lobby-rematch = REMATCH
lobby-leave = LEAVE
lobby-kick = KICK

# What the code entry says about a code that is not one.
code-error-length = a room code is { $expected } letters, not { $got }
code-error-character = '{ $char }' is not part of a room code

## The online round's status line (src/net/round.rs), 14 px along the
## field's top edge. `{ $room }` is the word ROOM below, `{ $code }` the
## room's code, `{ $seat }` this seat's number.
# The word before the code. The key is `status-label-` and the label the
# round was opened under; a rig's round says RIG, which is no word.
status-label-room = ROOM
status-connecting = { $room } - CONNECTING
status-greeting = { $room } - ASKING FOR A SEAT
status-lobby = { $room } { $code } - SEAT { $seat } - IN THE LOBBY
# `{ $rtt }` is empty, or ` - ` and `status-ping` once a round trip is known.
status-ping = PING { $ms } MS
status-buffer = { $room } { $code } - SEAT { $seat }{ $rtt } - BUFFER { $ms } MS
status-waiting = { $room } { $code } - SEAT { $seat }{ $rtt } - WAITING FOR THE ROOM
status-offline = { $room } - OFFLINE: { $reason }
note-tuning-refused = the room's tuning was refused: { $detail }
note-welcome-refused = the room's round could not be built: { $detail }
# `--host -m` on a map nobody has won as it stands.
note-not-cleared = this map is not cleared: win it from PLAY in the builder, then save it

## What the room server refused, and why (net::wire::Refusal). Shown
## under the lobby's title or on the status line. `{ $detail }` is the
## server's own text where it carries one, shown as it came.
refusal-already-in-room = already in a room
refusal-not-in-room = not in a room
refusal-not-yours = that message is the server's to send
refusal-bad-message = the room could not read that: { $detail }
refusal-bad-map = bad map: { $detail }
refusal-bad-code = not a room code: { $detail }
refusal-no-such-room = no room { $code } here
refusal-room-gone = room { $code } is gone
refusal-server-draining = this server is closing for a restart; try again in a minute
refusal-server-full = this server is full ({ $rooms } rooms)
refusal-room-full = the room is full: { $seats } seats
refusal-already-started = the round has started; this room takes no new seats
refusal-kicked = the host removed you from this room
refusal-reconnected = reconnected from another socket
refusal-grace-over = away too long; the seat was freed
refusal-left-room = you left the room
refusal-only-host-starts = only the host starts
refusal-only-host-kicks = only the host kicks
refusal-kick-self = the host cannot kick themself
refusal-no-such-seat = no such seat
refusal-in-progress = the round is in progress
refusal-not-ready = { $nick } is not ready
refusal-server-restarting = this server is restarting; make a new room in a moment
refusal-room-closed = the room closed

## The builder (src/editor/mod.rs, src/editor/render.rs)

# The build bar, in points: BUILD at 18 in 64, FILE and MAP at 18 in 42
# beside a caret (about 4 letters); UNDO/REDO/FIT at 11 inside 36 with a
# mouse and at 12 inside 40 on a touch screen.
editor-build = BUILD
editor-undo = UNDO
editor-redo = REDO
editor-file = FILE
editor-map = MAP
# The camera back to the whole canvas.
editor-fit = FIT
# A round from the middle of the view rather than the map's start, just
# before PLAY: 11 pt in 64 with a mouse, 12 in 72 on a touch screen
# (about 9 letters).
editor-play-here = PLAY HERE
# The map's check (its findings and quick fixes), after FIT: 11 pt in 52
# with a mouse, 12 in 60 on a touch screen (about 6 letters).
editor-check = CHECK

# The CHECK panel, 500 pt wide under its button: the title (16 pt, about
# 20 letters, left of the counts), the line under it (12 pt, about 70
# letters), the line a map with no findings shows (16 pt) and the button
# that makes a finding's one fix (16 pt in 80, about 6 letters).
check-title = MAP CHECK
check-hint = Pick a problem to see it on the map. FIX makes the change it asks for.
check-none = NO PROBLEMS FOUND
check-fix = FIX
# The clear check's row in the panel: whether this revision of the map has
# been won from PLAY, its par (the clear time, m:ss) and what clearing
# means, 12 pt under them.
check-cleared = CLEARED
check-not-cleared = NOT CLEARED
check-par = PAR { $time }
check-cleared-hint = Won from PLAY as it stands. An edit is a new revision to clear.
check-not-cleared-hint = Win it from PLAY, alone, with no edits since, to host it in a room.

# What the map linter found, one line per finding in the CHECK panel, 16 px
# beside its mark and before its FIX button (about 28 letters). The key is
# `lint-` and the finding's kind as the dev server's `lint` tool spells it.
lint-unreachable-frog = FROG OUT OF REACH
lint-unreachable-pickup = PICKUP OUT OF REACH
lint-gated-pickup = PICKUP BEHIND WALLS
lint-disconnected-region = CUT-OFF GROUND
lint-boxed-in-cell = BOXED-IN SPOT
lint-spawn-band-too-tight = NO ROOM TO PLACE ENEMIES
lint-planner-physics-mismatch = ROUTE THROUGH A WALL
lint-narrow-corridor = ONE-CELL PASSAGE
lint-gate-not-on-edge = GATE OFF THE EDGE
lint-gate-blocked = GATE LANE BLOCKED
lint-waves-no-gates = WAVES WITH NO WAY IN
lint-hunt-missing-enemy-frog = HUNT WITH NO ENEMY FROG
lint-enemy-frog-unreachable = ENEMY FROG OUT OF REACH
lint-no-start = NO PLAYER START
lint-start-penned = START PENNED IN
lint-player2-unreachable = PLAYER 2 CUT OFF
lint-players-too-close = STARTS TOO CLOSE
lint-portal-alone = LONE PORTAL
lint-portal-blocked = PORTAL BLOCKED
lint-tower-at-start = TOWER COVERS A START
lint-tower-no-reach = TOWER CAN'T REACH ANYTHING
lint-too-many-towers = MANY TOWERS ON ONE SIDE
# The status line's fallback before a category is active.
editor-tool = TOOL

# The five tool groups, in the status line and, at 12 pt in 60, beside
# their row of the palette a narrow bar folds them into.
category-wall = WALL
category-prop = PROP
category-ground = GROUND
category-actor = ACTOR
category-pickup = PICKUP

# The FILE menu's rows, 18 px in a 200 px menu.
file-load = LOAD...
file-save = SAVE
file-save-as = SAVE AS...
file-clear = CLEAR MAP

# The MAP settings rows, 16 px labels in a 120 px column.
settings-tanks = TANKS
settings-tank = TANK
settings-tank2 = TANK 2
settings-mission = MISSION
settings-spawn = SPAWN
settings-waves = WAVES
settings-size = SIZE
settings-growth = GROWTH
settings-tier-start = TIER START
settings-tier-end = TIER END
settings-theme = THEME
settings-weather = WEATHER
# The map's size in cells, and where the old map sits when it changes.
settings-width = WIDTH
settings-height = HEIGHT
settings-anchor = ANCHOR
settings-reset = RESET MAP
# A settings value the map leaves to the game.
settings-auto = auto
# Beside a value a command-line flag outranks, 12 pt.
settings-cli = (cli)

# The popups. The Save prompt's line, 12 pt in 276 pt, names the keys,
# or after a touch the tap outside the prompt that cancels it.
editor-save-as = Save as:
editor-save-hint = Enter to save, Esc to cancel
editor-save-hint-touch = Enter to save, tap outside to cancel
editor-no-maps = no maps to load
editor-shipped = shipped
# A long list's pager: the span on screen, 12 pt between its < and > in
# a 340 pt row, and how to turn it - the mouse's wheel, or after a touch a
# tap on either arrow.
editor-page = { $from }-{ $to } of { $n }  (wheel)
editor-page-touch = { $from }-{ $to } of { $n }  (tap < or >)
editor-untitled = untitled

# The status line's answers.
editor-saved = saved { $name }.toml
editor-loaded = loaded { $name }
editor-saving-unavailable = saving is not available in this build: edits stay in memory for the session
editor-no-name = the map has no name yet: use SAVE AS
editor-bad-name = map name "{ $name }" may only use letters, digits, - and _
editor-copied = copied { $n } cells
editor-cut = cut { $n } cells
editor-stamp-kept = kept as { $name } in STAMPS
editor-stamp-empty = nothing to keep: the selection is empty
editor-fill-too-large = too large to fill: more than { $n } cells

# BRUSH: how a press on the canvas paints - a pen, a rectangle filled when
# the drag ends, a flood fill of the region pressed, a scatter of a share
# of the cells round the drag -, the select tool and the stamps. The
# palette's row of them is named at 12 pt in 60, like a category; the
# list's rows at 18 px in a 200 px row after a 32 px icon (about 12
# letters), like the tools.
editor-brush = BRUSH
brush-pen = pen
brush-rect = rectangle
brush-fill = fill
brush-scatter = scatter
brush-stamps = stamps...

# The select tool's strip under the bar: a word on each button, 11 pt in
# 60 with a mouse and 12 in 72 on a touch screen (about 8 letters). COPY,
# CUT, PASTE and DELETE act on the selection; + STAMP keeps it as a stamp
# for the session; STAMPS opens their list; PLACE puts a paste down where
# it stands and CANCEL takes it away.
select-copy = COPY
select-cut = CUT
select-paste = PASTE
select-delete = DELETE
select-save-stamp = + STAMP
select-stamps = STAMPS
select-place = PLACE
select-cancel = CANCEL

# The stamps in the STAMPS list, 18 px between a stamp's picture and its
# size (about 12 letters): the shipped ones by their file's name under
# maps/stamps/, and the ones kept this session numbered.
stamp-fort = fort
stamp-bunker = bunker
stamp-river-bend = river bend
stamp-saved = my stamp { $n }

# The tools, as the dropdown rows spell them, 18 px in a 200 px row
# after a 32 px icon (about 12 letters). The key is `tool-` and the
# tool's own name, which is also how the dev server spells it.
tool-brick = brick
tool-iron = iron
tool-wood = wood
tool-glass = glass
tool-sandbag = sandbag
tool-barrel = barrel
tool-fence = fence
tool-tree = tree
tool-pine = pine
tool-oil_drum = oil drum
tool-fuel_drum = fuel drum
tool-oil_trail = oil trail
tool-road = road
tool-water = water
tool-tall_grass = tall grass
tool-gate = gate
tool-portal = portal
tool-start = p1 start
tool-start2 = p2 start
tool-frog = frog
tool-enemy_frog = enemy frog
tool-health = health
tool-ammo = ammo
tool-laser = laser
tool-minigun = minigun
tool-plasma = plasma
tool-missiles = missiles
tool-speedup = speed-up
tool-shield = shield
tool-flamethrower = flamethrower
tool-frog_health = frog pack
tool-tower_pack = tower pack
tool-tesla = tesla coil
tool-tesla_enemy = enemy tesla
tool-gun_tower = gun tower
tool-gun_tower_enemy = enemy gun
tool-bio_slush = bio slush
tool-bio_slush_enemy = enemy slush
tool-eraser = eraser
tool-select = select

# The short spelling for the bar's 10 px line and the cursor readout,
# where a tool's full name has no room (about 6 letters). A tool without
# one is spelled by its `tool-` message.
tool-short-start = start
tool-short-start2 = start2
tool-short-tall_grass = grass
tool-short-oil_drum = oil
tool-short-fuel_drum = fuel
tool-short-oil_trail = oil
tool-short-enemy_frog = e.frog
tool-short-flamethrower = flame
tool-short-frog_health = frog+
tool-short-tower_pack = tower+
tool-short-tesla = tesla
tool-short-tesla_enemy = e.tsl
tool-short-gun_tower = gun
tool-short-gun_tower_enemy = e.gun
tool-short-bio_slush = bio
tool-short-bio_slush_enemy = e.bio

## Data names shown as words. The key is the data spelling (the map
## format's, the CLI's), which never changes.

theme-grass = grass
theme-desert = desert
weather-clear = clear
weather-night = night
weather-dusk = dusk
weather-rain = rain
weather-storm = storm
weather-fog = fog
weather-sandstorm = sand
weather-snow = snow
weather-heat_haze = haze
weather-random = random
spawn-band = band
spawn-waves = waves
tier-light = light
tier-medium = medium
tier-heavy = heavy
tier-super = super

# The twelve chassis. Names, not words - a language may keep them.
tank-scout = scout
tank-assault = assault
tank-breaker = breaker
tank-longbow = longbow
tank-flak = flak
tank-wraith = wraith
tank-warden = warden
tank-ravager = ravager
tank-glacier = glacier
tank-obelisk = obelisk
tank-titan = titan
tank-leviathan = leviathan
