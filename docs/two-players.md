# Two player gameplay

Purpose of this PRD is to introduce second player to the gameplay. Meaning that there will be two player tanks on the battlefield. Player one will use different keys than player two.

## Switching between players

- In the HUD bar of gameplay there should be some kind of tank/share icon.
- Visually Between "FROG" indicator and BUILD.
- There should be some kind "pop-up" screen (visually similar then "Leave this round?")
that will ask if you want to play in different modes.

## Player modes

- Single player. Same gameplay as we have now. Space is for fire.
- Two players. Player one fires with right shift key not space key.

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

- P1 arrows + Right Shift; 
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
- **Keys.** Single: arrows + Space, as before. Two players: player 1 on the
  arrows + Right Shift (Space is not read), player 2 on WASD + Left Shift.
  Taps are ignored in a two-player round - the keyboard is the whole
  interface - so the tap-order state stays clear.
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
  outline. A dead player shows zeros. Player 2's ground ring is the
  palette's mid blue with a blue health ramp.
- **Tools.** `restart {players, tank2_row}`, `players {count}`, `step`/`input`
  `p2_*` fields, `tap` refused in two-player rounds, `builder_settings
  {tank2}`, the `start2` brush, `key 1|2`; the probe's `--players`,
  `--tank2`, `--p2-scenario`, anomaly checks against the nearest live
  player. The linter adds `no-start`, `player2-unreachable` (error) and
  `players-too-close` (warning).
- **Web.** Emscripten's GLFW layer reports the DOM Shift key as Left Shift
  whichever side was pressed, so the page keeps a `window.bbShift` bitmask
  from `event.code` and the game reads it once a frame; native reads
  raylib's two keys.

