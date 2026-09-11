# PRD: Player indicator improvements

It is very hard to tell the player's tank from the enemies, in one- and
two-player rounds alike. An enemy can spawn with the exact chassis the
player drives (band enemy 1 is always `assault`; a wave rolls any row in its
tier), so the only difference is the ground ring, which sits under the hull,
peeks out on the flanks of most chassis and shows nothing at all on `titan`
and `leviathan`. Player 1's ring uses the same white/gold/red ramp a just-hit
enemy draws, and player 2's blue (`#04A0B4`) is literally the `flak` hull
colour. Goal of this work is to make both player tanks visible and
distinguishable at a glance, from the enemies and from each other.

# Acceptance criteria

- The player's tank is a different colour than every enemy tank, in
  single-player too.
- The second player's tank is a different colour again.
- Player 1 and player 2 are never the same colour, whatever chassis either
  drives.
- The colour is the identity everywhere the player is represented: the
  hull, the ground ring, the HUD readouts and players button, and the
  editor's start markers.
- At the start of a round the player can locate their tank in a crowd
  within a second.
- Single-player seeded replays and the probe fixture baselines stay
  byte-identical (no change to RNG draw order).

# Decisions (2026-09-11)

Picked on a side-by-side of the real sheet recoloured in the browser
(https://claude.ai/code/artifact/02f8915a-43a3-428e-8457-0f0083340b15).

## Team colours

The Puny Palette has no free hue: every family (sand, wood, red, teal,
cyan, green, grey, gold) is already some chassis's body or accent, and the
eight base colours no tank uses sit within 13-38 RGB units of one that
does. So the team colours come from the Resurrect 64 set the art was built
on before the Puny pass, from the three hue regions nothing on screen
occupies (true blue, violet, magenta):

| Team | Base | Ramp (dk, md, base, lt) | Nearest tank colour |
|---|---|---|---|
| Player 1 | sky blue `#4D9BE6` | `#484A77` `#4D65B4` `#4D9BE6` `#8FD3FF` | cyan `flak`/`glacier`, 77 units |
| Player 2 | hot pink `#F04F78` | `#831C5D` `#C32454` `#F04F78` `#ED8099` | red `titan`, 92 units |

Both ramps are Resurrect's own steps, not `mul()` products - `mul()` snaps
back onto the Puny set and would destroy them. They are off the Puny
Palette on purpose, the same deliberate exemption `plasma.png` and the
pickup icons already have: an identity colour has to be loud against the
terrain, not sit in it. They live in `tools/punypalette.py` as a
`PUNY_TEAM` family that is **not** part of `PUNY_PALETTE`/`PUNY_EXTRA`
(opt-in, so no other generator re-quantises), and `check_sheets.py` allows
them on `scifi_tanks_sheet.png` only.

Violet (`#A884F3`) was the other finalist and is the fallback if sky blue
turns out too close to the cyan hulls in play.

## Mechanism: baked team rows, body + accent

`gen_tanks.py` emits three copies of every chassis: the enemy row as today,
then a player-1 row and a player-2 row with the **body ramp and the accent**
swapped for the team ramp (accent maps to the team's light step). Outline,
gunmetal barrel greys and greebles stay. The sheet grows from 12 to 36
rows (13 columns unchanged, so wrecks, broken turret and track marks of a
player tank are team-coloured too); `docs/SPRITESHEET_SPEC.md` gets the
row-block layout.

Engine side the chassis tables stay 12 wide: `Tank::row` is still the
chassis, and the atlas row is `row + TANK_ROWS_PER_TEAM * team`, where team
comes from `Tank::owner` (`Enemy` = 0, `Player(0)` = 1, `Player(1)` = 2).
`source_rec` is the one place that adds the offset. `damage.png`, the
minigun mount and `tracks.png` are shared art and unchanged.

Rejected: a runtime multiply tint (can only darken; already tried and
rejected for plasma), and a palette-swap shader (needs a hand-ported GLSL
ES 100 twin for web and would be the first per-sprite shader).

Known wrinkle: `wraith`'s body is `STONE_MD`, the same grey as every
chassis's barrel, so a naive colour swap would tint its barrel. The
generator must substitute the ramp by *role* (body/accent), not by colour.

## Chassis are not exclusive

Enemies may still roll the player's chassis and player 2 may roll player
1's. Colour does the job, and this keeps every seeded round and
`just probe-fixtures` byte-identical - no RNG draw is added or reordered.

## Ground rings

- Player 1's ring is the sky-blue ramp, player 2's the hot-pink ramp:
  team base for the top half of health, then gold, then bright red, in
  place of today's white/gold/red (`HealthRamp::Blue` becomes the P1 ramp,
  a `Pink` ramp is added, `White` is no longer used by a player). The
  missing arc is the dimmed team colour, as today.
- The player rings grow **10 % in radius with the same band thickness**:
  a new `player_ring_radius_scale` knob (1.1) on top of the shared
  `shield_glow_radius_factor`, the band staying `0.385 * size * 0.22` px
  (`draw_ground_ring_scaled`). The player's shield ring uses the same
  scale so the cross-fade stays concentric; enemy rings are unchanged.
- The enemy ring stops sharing player 1's ramp: it draws with
  `HealthRamp::Red`, the enemy frog's ramp, so red-on-hit reads hostile.
- `PLAYER2_RING_COLOR` is replaced by `tank::TEAM_COLORS: [Color; 2]`
  (plus the full ramps), the single source every surface below imports.
  Colours are `pub const`s in `tank.rs`, not tunables - the knob table has
  no colour type, and these are identity, not feel.

## HUD

- Two-player readouts `60|70`: each side is drawn in its team colour when
  healthy; the `hud_warn_threshold`/`hud_critical_threshold` colours still
  override per side. The live-weapon underline is in the team colour.
- The players button glyphs: one sky-blue tank, or sky blue + hot pink.
- Single player: the heart/HP readout stays as it is (one player needs no
  disambiguation).

## Editor

- The `start` marker draws a sky-blue ring under its tank icon (today it
  has none); `start2` draws the hot-pink ring (was `PLAYER2_RING_COLOR`).
- The palette icons use the team rows: `icon_source_rec` takes a team, so
  the start brushes show a blue and a pink scout.
- `ENEMY_RING_COLOR` for the enemy frog stays red.

## Round-start locate cue

For `player_locate_seconds` (2.0) after the round becomes playable - after
the mission banner fades, or at once when there is no intro - each live
player's ring pulses between its normal radius and 1.6x at
`player_locate_pulse_hz` (2.0), fading as it swells, with a `P1`/`P2`
label in `HUD_TEXT_SIZE` above the hull drawn after the trees. Purely
presentation: driven from `Game::time`, which resets per round and does
not run behind the banner, no RNG, nothing in `simulation/` changes. Runs on every restart (R, auto restart, PLAY, the
dev server's `restart`), not on wave changes. Both knobs go in the
`cosmetics` group.

# Out of scope

- Recolouring shells, bullets or plasma per team (projectiles are
  deliberately not chassis- or side-matched, docs/BULLETS_SPEC.md).
- An off-screen arrow or camera cue (the field is one screen).
- Excluding the player's chassis from enemy rolls (see above).
- Team-coloured tread marks.

# Verification

- `SPRITE_OUT=static python3 tools/spritegen/gen_tanks.py` (via the
  nix-shell Pillow incantation in CLAUDE.md) produces a 416x1152 sheet;
  `just check-sheets` passes with the team family allowed on the tank
  sheet only; the enemy block (rows 0-11) is byte-identical to today's
  sheet.
- `cargo test --lib`: `chassis_tests`, `health_ring_tests` (new radius and
  ramp rules), `hud_tests` (coloured pairs still fit the slots),
  `editor_tests` (start markers), `determinism_tests`.
- `just probe-fixtures` and a JSON diff of every fixture's probe records
  stay byte-identical.
- Dev server: `restart {players: 2}`, `step`, `screenshot` on the default
  map and `maps/test/maze.toml`; then `set_tank` the player onto each of
  the 12 rows and screenshot - the player must read as blue on every
  chassis, including `titan`/`leviathan`, with the ring visible outside the
  hull. `spawn_enemy` with the player's row beside it for the same-chassis
  case. Screenshot the first frame after the banner for the locate pulse.
- Web build (`just build-web-dev`): the larger sheet loads and the pulse
  runs; no new shader, so nothing to port.
