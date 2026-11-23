# Levels: playing the maps in order

Status: **implemented 2026-09-29**. docs/maps-to-levels.md turned a map
into a level (a mission and a spawn plan). This document strings the levels
together: `levels.toml` lists them in order, a win opens the next one, and
the end screen counts down to where the level goes next - the next level
after a win, the same one after a loss - instead of restarting the round.

## Decisions (2026-09-29)

| Topic | Decision |
|---|---|
| The list | `levels.toml` at the repository root: `[[level]]` entries in play order, each naming a map (`map`, its file stem under `maps/`, one of `SHIPPED_MAPS`) and the `title` its banner shows. The maps stay separate files. Compiled in. |
| Order | Boot Camp, the training stage (docs/training-stage.md), first and `skippable`, numbered `LEVEL 0` (`Levels::number`) so Lotus Lagoon is still level 1; then roughly easy to hard: Lotus Lagoon (5 tanks) next, the two big fields among them - Wildwood (level 7) and Longwater, last (level 17). Edit `levels.toml` to change it. |
| Progression | Only a win opens the next level, and only a win of the level as it ships ("Honest locks" below). A loss offers the same level again. Winning the last level shows "all levels complete" and leads back to level 1 (Lotus Lagoon - never back to Boot Camp). |
| Start and save | A session opens on the furthest level reached. On the web that is the page's `localStorage` (`bongbong.level`); on a desktop or a phone, a file. Progress is kept by map name, so reordering the list keeps it. Winning the last level is kept beside it (`complete` in the file, `bongbong.complete` on the web), so its tile shows as won; a store from before that reads as not complete. |
| End screen | Counts down like free play's (`restart_delay`, 3 s), inside the button it will press: `NEXT LEVEL IN 3, 2, 1` after a win, `PLAY AGAIN IN 3, 2, 1` after a loss, then takes that way by itself - a press on the counting button takes it at once. The last level's win counts nothing down: `ALL 17 LEVELS COMPLETE!` waits for `BACK TO LEVEL 1`, going round being the player's call. The buttons - `LEVELS` and `PLAY AGAIN` always, `NEXT LEVEL` after a win - take a way at once; Enter takes the way on after a win and plays again after a loss; R plays again, as it always did; Esc opens the level select, which, like a dialog or the builder, stops the countdown. Whatever way it takes, nothing goes black: the screen's words lift out over its last `round_verdict_lift_seconds` (0.3 s) while its dim stays, and the screen dissolves block by block into the round it leads to over `round_dissolve_seconds` (0.5 s; docs/effects.md, "Between rounds"). |
| Stats | Time (the round clock, which stands still behind the banner and while paused) and enemies destroyed out of all the round brings, split by seat in a couch round. |
| Banner | `LEVEL 3 / 17` over the mission banner (Boot Camp is level 0 and is not counted: `Levels::count`, which the last win's line reads too), the level's title under it at half the banner's size (36 px under 72). |
| Builder edits | A level edited in the builder is played as edited for the rest of the session, every time it comes round. On a desktop the builder's Save writes it to `maps/<map>.toml`, which later sessions read first. |
| Free play | Any map that is not a level (`-m`, the builder's Load list, a Save As under a new name) restarts on its own after `restart_delay`, as every round did before; its end screen shows the same numbers. |
| Online | Unchanged: a room's round counts down to the room's lobby. |
| Going back | The level select: every level as a tile, the ones won and the furthest reached open, the rest locked. Opened from the bar's level button (`LEVEL 3`, in the mission word's place on a level), the end screen's `LEVELS` and Esc; the round behind it stands still. A replay moves nothing: only a win on the furthest level reached, played as it ships, opens another. No best times. |
| Stalls (BB-189) | A round never waits on what nobody can reach: its last enemies are called to the fight after a lull, a wave gets past seats parked on its lanes, and a fallen seat comes back on the final wave and on a map with no gate ("A round that cannot stall" below). |

## The flow

**A level is a map by name.** `Session::level` is the index in the list of
the local round's map name, so there is no "current level" to keep in step
with anything: a level loaded into the builder and played is still that
level, `-m maps/carnival.toml` is the Carnival level, and a map under any
other name is free play. Every place the round's map changes (`play`,
`replace_map`, `start_level`, `set_campaign`) sets `Game::hold_end_screen`
from it: a level's end screen counts down to zero and holds there, free
play's counts down and restarts inside `Game::update` as before.

**The countdown** is the round's own `restart_timer` over
`restart_delay`, the one free play has always shown; `Game::update` ticks
it on a held end screen too, only never below zero and never into `init`,
since where a level goes next is the session's to decide. After every
frame's steps `app.rs` calls `Session::follow_countdown`, which takes the
way the screen counted down to once the timer stands at zero: `next_level`
after a win, `play_again` after a loss. `ResultButtons::countdown` is the
number the screen shows, whole seconds and never 0, in the label of the
button it counts down to - no line of its own, so the press that skips the
wait is where the eye already is; it is `None` after the last level's win,
which waits for a button. A screen behind the level
select, a dialog or the builder never moves on by itself: nothing updates
the round, so nothing counts it down.

**Progress** is `levels::Campaign`: the list, `reached` (the index of the
furthest level reached - only a win moves it, only forward) and the
session's edits by map name. `app.rs` calls `Session::note_outcome` after
every frame's steps, so a win is progress the frame it happens and a player
who closes the window on the end screen has still reached the next level;
`take_progress` hands each change to the store once. The store is the
platform's:

- web: `localStorage['bongbong.level']`, read at startup through
  `page_string` and written through `emscripten_run_script`, both wrapped so
  a throwing `localStorage` (a private window, blocked storage) only
  forgets;
- desktop and iOS: `levels::progress_path` - `%APPDATA%`, `~/Library/
  Application Support` (on iOS `HOME` is the app's container), or
  `$XDG_DATA_HOME`/`~/.local/share`, then `bongbong/progress.toml`;
  `BONGBONG_PROGRESS` names a file outright;
- Android: the activity's `internalDataPath` (`app::android::data_dir`,
  read through raylib's `GetAndroidApp`), then `progress.toml`.

The file is TOML, `level = "hedge-maze"`. A name the list no longer has
starts from the first level.

**The way on.** `Session::next_level` is live only on a won level:
`start_level` opens the next map - the builder's edit if there is one,
else `map::open_map` (a desktop's `maps/<map>.toml`, else the shipped
copy) - on the field and in the builder, where it is a new document with
an empty undo history (`MapEditor::open`), and starts a round on it with the
banner. `play_again` is `init` on the same map. `press_result` and
`enter_result` are the one entry for a click, a tap, the dev server's
`click`/`key` and Enter; the hit test reads `hud::result_layout`, the same
geometry the painter draws from, so a button that is not drawn cannot be
pressed. `LEVELS` opens the level select (below). A tap that presses one is claimed from the touch scheme
(`TouchScheme::claim`) so it is not also the fire press that skips the next
level's banner.

**The numbers** are `Game::round_stats`: `ended_at` (the round clock when
`end_round` ran), the enemy wrecks counted where every wreck is made
(`explosions`, through `credit_wreck`), and the round's enemy total (the
band, or the sum of every wave's size). A wreck is credited to the seat
that last damaged it - `Tank::last_hit_by`, set through `Tank::credit` by a
seat's shell, bullet, bolt, beam, flame, missile burst or ram. A wreck no
seat touched (a drum, a fire, a bong's bite) counts for the team alone. All
of it is bookkeeping: no RNG, so every seeded replay and probe fixture is
unchanged.

## Honest locks

A win opens the next level only when the level was played as it ships
(`Session::counts_for_level`, decided on the win's first frame by
`note_outcome`): the round is on a level, its map is that level's shipped
map to the revision (`map::shipped_revision` against `MapFile::revision` -
not the session's builder edit, not a modified copy the player's store or
`maps/` gave), it is set up as the map is authored (`set_up_as_authored`:
the map's own start and sky, and no enemy count, mission, spawn plan, wave
or chassis from the command line, a tool or the `player_tank` knob) and it
seats one or the couch's two. `played_as_authored`, the clear check's test,
is the same with exactly one seat.

A win that does not count leads on only where the way was open already - a
level won before, or the one after a skippable level - and otherwise shows
no `NEXT LEVEL`: the line over its numbers says `PLAYED WITH CHANGES: NOT
COUNTED` (`ResultButtons::uncounted`, laid out in the all-clear line's slot
by `hud::result_layout`), and the countdown plays the level again, as after
a loss. `next_level` refuses whatever the end screen does not offer.

## A round that cannot stall

A round is won on wrecks, so it must never wait on a tank nobody can reach
or one that cannot get onto the field. Three rules, none drawing RNG, each
a no-op in a round that never needs it (docs/gameplay-verification-design.md,
"Re-baseline: round flow (BB-189)"):

- **The last tanks are called** (`simulation/stragglers.rs`,
  `Game::straggler_phase`, run before the enemies think). Once no enemy is
  still to come (`spawn_plan_finished`) and `straggler_call_alive` (3) or
  fewer are left, a lull of `straggler_call_after_seconds` (20 s) - no live
  enemy with a seat within the range it is seen at (`sight_on`) or the
  players' bong within enemy sight, none freshly hit - calls every one of
  them to the nearest live seat (`FieldMind::called`, `field::call_target`),
  with an `Event::StragglersCalled` (not sent: a replica sees them come).
  On a field map that wakes a band tank nothing ever woke and takes a tank
  off its home leash; on an arena it heads the last tanks at a seat the
  shared alert has lost. The call ends as a wave's does, within sight range
  of a seat or on a hit, and another lull calls again. Only a Protect or
  Destroy round with enemies in its plan - never a Hunt, a sandbox or a
  training script - and a guard keeping its live bong stays on it. The
  field map's re-roll (`reroll_stragglers`, docs/large-maps-follow-camera.md
  section 12) still takes a wave tank lost far off the screens.
- **A blocked lane is waited out** (`waves.rs`, `wave_phase`). When every
  lane is busy - seats parked on every gate's inside point, say - for
  `wave_lane_wait_seconds` (6 s), the one waiting drops onto the field as a
  gate-less map's would: a wave tank through `spawn_in_band` (out of sight
  on a field map), a seat coming back as below.
- **A fallen seat comes back** in a wave round of two or more. The final
  wave has no next wave to bring it with, so it brings back a seat that has
  been a wreck for `wave_final_return_seconds` (10 s; 0 keeps it where it
  fell). On a map with no gate at all, or past the lane wait, the seat is
  dropped in instead of driving in (`seat_drops_in`): on the cell nearest
  the first living seat that `Game::drop_cell` takes, a tank can drive to
  from there and stands clear of every tank, with its body and the stick at
  once, announced with `Event::TankEntered`.

## Going back: the level select

`level_select.rs` is the screen and `render/level_select.rs` its painting,
the lobby's shape: a fixed 704 x 336 panel over the dimmed field (it fits
the smallest field the game ships, 768 x 384), `tile_rect` and `back_rect`
the one geometry the painter and every hit test read, and a
`LevelSelectView` of plain data. Twenty-one tiles in three rows of seven,
89 x 58 px - the campaign's eighteen and room for three more: the number
(20 px) at the top, the title under it on at most two balanced lines
(`wrap`, 81 px a line at 12 px), a tick on a level won, a padlock on a
locked one, the amber of the furthest reached, a white edge where the
keyboard's focus is and a pip on the level the round behind is. The text's
offsets in a tile (`TILE_NUMBER_TOP`, `TILE_TITLE_TOP`) are the headless
module's, so a test holds a two-line title inside the tile, and another
holds `levels.toml` to the one page.

`Session::level_select` is the screen's state; while it is up `playing()`
is false, so the round is frozen the way the dialogs freeze it and resumes
where it stood on `BACK`, Esc, Tab or a press outside the panel. A press on
an open tile - or Enter on the focus the arrow keys move - is
`start_level`, which is a new round with its banner; a locked tile is no
button. It opens from three places: the bar's level button
(`hud::level_button_rect`, `Session::level_button` - which the painter and
the hit tests both read, so it is pressable exactly where it is drawn), the
end screen's `LEVELS` (left of `PLAY AGAIN`, so the way on stays on the
right), and Esc in play mode. The level button takes the bar's title slot
on a level - `LEVEL 3` with a wave round's `2/5` beside it; the mission word
is the opening banner's - and free play keeps its mission word, Esc being
its way to the level select.

## The weapons along the way

Every special weapon is met somewhere in the campaign, about one new one a
level or two, the plain ones early and the ones that change a fight late. A
level's crates are its map's fixed `pickup` cells: no slot rolls its kind,
no random roll or bonus drop makes a special (only health's bonus rolls
shields and packs), and a map cannot set tuning - so a crate placed in a
level is the only way a campaign player meets a weapon, and the only way an
enemy there carries one of the six BB-36 weapons.

| Level | Weapon crates | Met first here |
|---|---|---|
| 0 Boot Camp | minigun | minigun |
| 1 Lotus Lagoon | missiles, minigun, plasma | missiles, plasma |
| 2 Serpent River | laser (18,13), plasma, missiles, laser (56,18) | laser - in the meadow by the start; the second on the far bank, for crossing |
| 3 Glasshouses | plasma, laser, minigun, missiles, flamethrower, sonic hammer (8,7) | flamethrower; sonic hammer - to break the glass |
| 4 Oasis Bazaar | flamethrower, plasma, missiles, grenades (24,23) | grenades - lobbed over the compound's towered gate |
| 5 Carnival | EMP burst (13,21), laser, missiles, plasma, flamethrower, minigun, grenades, sonic hammer | EMP - six waves of light tanks among a hundred drums, enemies arming from the crates |
| 6 Jungle Temple | plasma, laser, minigun, flamethrower, missiles, FPV swarm (13,23) | FPV swarm - over the temple walls; the miniguns beside it are the counter |
| 7 Wildwood | gravity well (31,57), jet (29,57) and helicopter (31,61) strikes, tower pack (24,57), EMP burst, FPV swarm, sonic hammer, grenades, flamethrower, laser, plasma, missiles, minigun | gravity well, air strikes, tower pack - by the start, with seconds of each out in the forest (the crypt, the hangar, the secret grove) |
| 8 Hedge Maze | laser, plasma, missiles | |
| 9 Archipelago | laser, missiles, plasma | |
| 10 Harbor Lights | laser (6,7), plasma, missiles, gauss rail (4,20) | gauss rail - through brick and glass warehouses, little iron |
| 11 No Man's Land | gauss rail (15,17), gauss rail (16,17), helicopter and jet strikes, grenades, missiles, minigun, plasma, flamethrower | |
| 12 Scrapyard | flamethrower, missiles, minigun | |
| 13 Vulkan | sonic hammer (68,37), helicopter strikes (three in the station), missiles, plasma, minigun, sonic hammer (48,21) | the hammer again - shove a tank into the lava |
| 14 Black Gold | flamethrower, plasma, minigun, missiles, gravity well (1,13) | the well again - drums pulled together on the fuse lines |
| 15 Castle Moat | laser, minigun, missiles | |
| 16 Grand Campaign | plasma, laser, flamethrower, missiles, minigun, rod from god (31,17) | rod from god - against the super-heavies |
| 17 Longwater | plasma, missiles, minigun, laser, flamethrower | |

The placements follow these rules:

- **The player's side.** A weapon's first crate lies near the start, inside
  its sight box where it can, so the players reach it before an enemy does.
  Enemies collect a weapon crate while they are on shells
  (`Tank::wants_pickup`), so an enemy that gets there first fights with it.
- **No rod or rail on a Hunt.** The rod's circle kills any bong in it and
  two slugs do too, so on a Hunt map either would end the hunt in one press.
- **Blasts away from the bong.** Friendly fire is full
  (docs/november-26-rebuild-upgrade.md decision 2): a blast hurts the
  players' bong as surely as an enemy, and the bong has `bong_max_health`
  (40), so a grenade landing within about 25 px of it on a high roll ends a
  Protect round. On a map with the players' bong a blast weapon's crate -
  grenades, FPV swarm, air support, the gravity well - lies at least two of
  its blast's radii from that bong (256 px for the grenade), so the first
  fight it is used in is not at the bong. The campaign keeps it: Jungle
  Temple's swarm lies 716 px from its bong, Oasis Bazaar's grenades 559 px.
  `armory`, free play, does not: its grenades lie 233 px from the bong and
  its swarm and well 172 px (BB-188 left the map as it was).
- **Kept setup draws.** A crate takes over a spare ammo or speed-up cell, or
  one cell of a cluster, never a health cell (only health draws its bonus
  roll when a round starts, so the round's opening draws are unchanged),
  never a level's only ammo and never its only crate of an older weapon. No
  Man's Land, with no spare slot, gains one cell.

`armory` (free play, `-m maps/armory.toml`) keeps every weapon side by side:
the BB-36 six and the grenades down the player's column, the laser, plasma,
minigun and flamethrower beside them, and the enemy's own crates opposite.

## Words

The English titles are `levels.toml`'s. Another language translates a title
as `level-<map>` in its `lang/*.ftl` (`text_tests` allows those ids beyond
English's and measures every title against the smallest field). The
chrome is `level-number`, `result-time`, `result-wrecks`,
`result-all-clear`, `result-uncounted`, `result-again`, `result-next`, `result-first`,
`result-levels`, `result-next-in`, `result-again-in`, `levels-title`,
`levels-sub`, `levels-back` and `bar-level`, each with its budget in `text_tests::budgets`; every title is
also held to its tile, whole, on two lines.

## Tests

- `levels::level_tests` - every level is a shipped map; a broken list is
  refused by name; `--level` by number or name; progress moves once, only
  forward, and round-trips through a file; an edit is the level from then on.
- `simulation::mechanics_tests::round_flow` - a field map's sleeping band
  and an arena's lost tank are called after the lull, a band over the count
  is left alone, a sandbox and the rule off call nobody, a seat in sight
  holds the lull; a seat parked on every gate cannot hold a wave out; the
  final wave brings a fallen seat back (and 0 keeps it); a map with no gate,
  or a lane blocked past its wait, drops it in near the living.
- `mode::session_tests` - a win under an enemy count or on an edit opens
  no level and says so, a couch's counts; a won level counts down to the
  next and takes it by itself, the last waits for its button back to the first, a lost one
  counts down to the same level again but not behind the level select,
  Enter and the buttons do not wait, an edit is played every time the level
  comes round, a map that is no level is free play.
- `simulation::mechanics_tests` - the numbers and the seat credit, a wave
  round's total, a held end screen counts down, waits at zero, and R still
  restarts it.
- `hud::hud_tests` - the end screen fits the smallest field in every form,
  its three buttons in one centred row and a level's countdown in a button; `touch` - a claimed touch neither
  fires nor steers; `devserver` - the end screen takes `click` and `key`.
- `level_select::level_select_tests` - the shipped levels fit one page,
  every tile and `BACK` are finger-sized inside the panel on both field
  sizes, a locked tile is no button, the keys walk only the open tiles, the
  pointer takes the focus only by moving, titles wrap balanced;
  `mode::session_tests` - the screen freezes the round, `BACK` resumes it
  where it stood, a replay moves nothing, the end screen's `LEVELS` opens it;
  `devserver` - the bar's button, Esc, the arrows and Enter, and `step`
  refusing by name while the screen is up; `render::hud::bar_tests` - the
  level button and a wave count fit the title slot.
