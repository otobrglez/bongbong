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
players-title = How many players?
players-keys = P1 arrows + Space    P2 WASD + L.Shift
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

# The locate label over a player's tank and the lobby's seat column: the
# seat's number. Keep it short - it sits over a 48 px hull.
seat-label = P{ $n }

# The touch hints over each half of the field, 20 px text, drawn once.
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

# The build bar, 18 px: BUILD in 64 px, FILE and MAP in 42 px beside a
# caret (about 4 letters), UNDO/REDO in 10 px inside 40 px buttons.
editor-build = BUILD
editor-undo = UNDO
editor-redo = REDO
editor-file = FILE
editor-map = MAP
# The status line's fallback before a category is active.
editor-tool = TOOL

# The five tool groups, in the field's status line.
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
settings-reset = RESET MAP
# A settings value the map leaves to the game.
settings-auto = auto
# Beside a value a command-line flag outranks.
settings-cli = (cli)

# The popups.
editor-save-as = Save as:
editor-save-hint = Enter to save, Esc to cancel
editor-no-maps = no maps to load
editor-shipped = shipped
editor-page = { $from }-{ $to } of { $n }  (wheel)
editor-untitled = untitled

# The status line's answers.
editor-saved = saved { $name }.toml
editor-loaded = loaded { $name }
editor-saving-unavailable = saving is not available in this build: edits stay in memory for the session
editor-no-name = the map has no name yet: use SAVE AS
editor-bad-name = map name "{ $name }" may only use letters, digits, - and _

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
tool-eraser = eraser

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

## Data names shown as words. The key is the data spelling (the map
## format's, the CLI's), which never changes.

theme-grass = grass
theme-desert = desert
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
