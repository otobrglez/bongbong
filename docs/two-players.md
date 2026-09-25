# Two player gameplay

Purpose of this PRD is to introduce second player to the gameplay. Meaning that there will be two player tanks on the battlefield. Player one will use different keys than player two.

## Switching between players

- In the HUD bar of gameplay there should be some kind of tank/share icon.
- Visually Between "FROG" indicator and BUILD.
- There should be some kind "pop-up" screen (visually similar then "Leave this round?")
that will ask if you want to play in different modes.

## Player modes

- Single player. Same gameplay as we have now. Space is for fire.
- Two players. Player one keeps Space for fire; player two fires with Left Shift.

## Two players mode

- Via map editor we can now place second "start".
- Call it "player 1 start" and "player 2 start"
- Map should have at least one player start
- If initial location for second player is not given; place second player near first player 1 start. Obviously don't overlap tanks.
- Game is lost if frog is killed (as with single player missions) or if both player tanks are killed.
- Friendly fire is possibility.

## Two player HUD

In two player mode HUD labels should start to look like: "60 | 70". Where 60 is health for player 1 and 70 for player two.


## Keyboard mapping

- P1 arrows + Space;
- P2 WASD + Left Shift;

## Decisions (2026-09-10, as built)

- **The mode is a session setting.** `Game::players` (`PlayerCount::One|Two`)
  is set before `init` and survives every restart (R, the auto restart,
  PLAY from the builder, the dev server's `restart`), like `--tank`.
  `--players 1|2` starts a session in either; the players button in the
  bar (one or two tiny tank glyphs, left of `BUILD`) opens the "How many
  players?" dialog, styled like "Leave this round?". Picking the other
  count restarts the round at once in that mode (a `--seed` stays pinned);
  the current count just closes it. The dialog works on the end screen too.
- **Keys.** Player 1 is always the arrows + Space, single or two players.
  Two players adds player 2 on WASD + Left Shift. (Player 1 first fired
  with Right Shift in two-player mode; changed to Space on 2026-09-12 so
  the arrow-key player's fire key never moves.)
- **Identity.** A tank stores its `Owner` (`Player(0|1)` or `Enemy(slot)`).
  Slot numbering: players first (0, and 1 with two players), enemies from
  `Game::first_enemy_slot` (1 or 2). Single-player rounds are byte-for-byte
  what they were - same slots, same RNG draw order (player 2's rolls sit in
  one block that only a two-player round enters).
- **Spawn.** Player 2 uses the map's `start2` cell (nudged off a solid
  tile), else the nearest open nav cell to player 1 that keeps two tank
  widths from it - a `start2` inside that clearance is ignored the same way.
  Its chassis: `--tank2`, else the map's `tank2` key (the MAP panel's `TANK 2`
  row), else a seeded roll. Enemies and the frog keep their distance from
  both players.
- **Rules.** Lost when every player tank is a wreck or the frog dies; one
  wreck of two stays on the field, its keys inert. Friendly fire is full
  damage scaled by the `friendly_fire_damage_factor` knob (1.0), rams
  included. A frog bite never finishes either player off.
- **AI.** Each enemy fights the nearer live, unconcealed player, switching
  only past `enemy_target_switch_margin_px` (96 px) so a pair at equal range
  does not flip the pack; a hit-alerted enemy still hunts a concealed
  player, as before. One engagement ring per player; the shared alert is
  the nearest sighting. Hunters and guards keep their frog logic.
- **HUD.** A second slot table: HP, shells and the weapon counts read
  `60|70` (player 1 left), SPEED and SHIELD stack into two thin bars, the
  live weapon is an underline under that player's side instead of a slot
  outline. A dead player shows zeros. Each player's hull, ground ring and
  HUD side are in that player's team colour - sky blue for player 1, hot
  pink for player 2 (docs/player-indicator-improvements.md).
- **Tools.** `restart {players, tank2_row}`, `players {count}`, `step`/`input`
  `p2_*` fields, `builder_settings
  {tank2}`, the `start2` brush, `key 1|2`; the probe's `--players`,
  `--tank2`, `--p2-scenario`, anomaly checks against the nearest live
  player. The linter adds `no-start`, `player2-unreachable` (error) and
  `players-too-close` (warning).
- **Web.** Emscripten's GLFW layer reports the DOM Shift key as Left Shift
  whichever side was pressed, which would let Right Shift fire player 2 on
  the web only, so the page keeps a `window.bbShift` flag for the left key
  from `event.code` and the game reads it once a frame; native reads
  raylib's Left Shift.

## Seats beyond two (2026-09-20, docs/online-coop-prd.md §4.11)

- **The round holds `MAX_SEATS` (8), not two.** `PlayerCount` is a count
  in `1..=MAX_SEATS`, `Game::seats` is one entity per owner slot, and one
  `EngageRing` per seat. Everything above still holds for one and two: the
  seat block `init` runs is the same block, one pass per extra seat, after
  player 1 and before the enemies, so a one- or two-player round draws the
  RNG it always drew and every seeded replay and probe ceiling still
  describes the same round (`determinism_tests::the_one_and_two_seat_streams_are_pinned`).
- **Where they stand.** Seat 0 takes `start`, seat 1 `start2`; the seats
  after that have no authored cell and take the nearest open nav cell to
  player 1 that a tank can *drive* to, keeping a tank's width from every
  seat already down and moved ashore if it lands in a lake. Seat 1's
  fallback is unchanged - the plain outward walk, wall or no wall - because
  a seeded two-player replay is that walk's answer. Numbered starts in the
  builder are a later lane.
- **Who they fight.** `Ai::target_player` picks the nearest live,
  unconcealed seat over all of them and switches only past
  `enemy_target_switch_margin_px`, the two-player rule read over N.
- **What they look like.** The sheet has three blocks, so a seat past the
  second draws player 1's and is told apart by its ring colour
  (`TEAM_COLORS`, eight Resurrect 64 steps from the blue, cyan, violet and
  magenta families - four hues in a bright and a deep register) and its
  `P1`..`P8` locate label. Proper blocks come from `gen_tanks.py` when a
  fourth friend shows up.
- **What is still two.** The players dialog and `--players`' keyboard
  mapping (arrows + Space, WASD + Left Shift); the HUD's slot tables, which
  lay a round of more seats out from the two-player table and show the
  first two - the compact N-seat strip is its own lane; and the room
  server's `SEATS_PLAYABLE`, a room policy rather than a limit of the
  simulation.
