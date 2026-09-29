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
| Order | Roughly easy to hard: Lotus Lagoon (5 tanks) first, Grand Campaign (7 waves) last. Edit `levels.toml` to change it. |
| Progression | Only a win opens the next level. A loss offers the same level again. Winning the last level shows "all levels complete" and leads back to level 1. |
| Start and save | A session opens on the furthest level reached. On the web that is the page's `localStorage` (`bongbong.level`); on a desktop or a phone, a file. Progress is kept by map name, so reordering the list keeps it. |
| End screen | Counts down like free play's (`restart_delay`, 3 s): `NEXT LEVEL IN 3, 2, 1` after a win, `PLAY AGAIN IN 3, 2, 1` after a loss, then takes that way by itself. The last level's win counts nothing down: `ALL 14 LEVELS COMPLETE!` waits for `BACK TO LEVEL 1`, going round being the player's call. The buttons - `LEVELS` and `PLAY AGAIN` always, `NEXT LEVEL` after a win - take a way at once; Enter takes the way on after a win and plays again after a loss; R plays again, as it always did; Esc opens the level select, which, like a dialog or the builder, stops the countdown. |
| Stats | Time (the round clock, which stands still behind the banner and while paused) and enemies destroyed out of all the round brings, split by seat in a couch round. |
| Banner | `LEVEL 3 / 14` over the mission banner, the level's title under it at half the banner's size (36 px under 72). |
| Builder edits | A level edited in the builder is played as edited for the rest of the session, every time it comes round. On a desktop the builder's Save writes it to `maps/<map>.toml`, which later sessions read first. |
| Free play | Any map that is not a level (`-m`, the builder's Load list, a Save As under a new name) restarts on its own after `restart_delay`, as every round did before; its end screen shows the same numbers. |
| Online | Unchanged: a room's round counts down to the room's lobby. |
| Going back | The level select: every level as a tile, the ones won and the furthest reached open, the rest locked. Opened from the bar's level button (`LEVEL 3`, in the mission word's place on a level), the end screen's `LEVELS` and Esc; the round behind it stands still. A replay moves nothing: only a win on the furthest level reached opens another. No best times. |

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
number the screen shows, whole seconds and never 0; it is `None` after the
last level's win, which waits for a button. A screen behind the level
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
seat touched (a drum, a fire, a frog's bite) counts for the team alone. All
of it is bookkeeping: no RNG, so every seeded replay and probe fixture is
unchanged.

## Going back: the level select

`level_select.rs` is the screen and `render/level_select.rs` its painting,
the lobby's shape: a fixed 704 x 336 panel over the dimmed field (it fits
the smallest field the game ships, 768 x 384), `tile_rect` and `back_rect`
the one geometry the painter and every hit test read, and a
`LevelSelectView` of plain data. Fourteen tiles in two rows of seven, 88 x
90 px: the number, the title on at most two balanced lines (`wrap`), a tick
on a level won, a padlock on a locked one, the amber of the furthest
reached, a white edge where the keyboard's focus is and a pip on the level
the round behind is. A test holds `levels.toml` to the one page.

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

## Words

The English titles are `levels.toml`'s. Another language translates a title
as `level-<map>` in its `lang/*.ftl` (`text_tests` allows those ids beyond
English's and measures every title against the smallest field). The
chrome is `level-number`, `result-time`, `result-wrecks`,
`result-all-clear`, `result-again`, `result-next`, `result-first`,
`result-levels`, `result-next-in`, `result-again-in`, `levels-title`,
`levels-sub`, `levels-back` and `bar-level`, each with its budget in `text_tests::budgets`; every title is
also held to its tile, whole, on two lines.

## Tests

- `levels::level_tests` - every level is a shipped map; a broken list is
  refused by name; `--level` by number or name; progress moves once, only
  forward, and round-trips through a file; an edit is the level from then on.
- `mode::session_tests` - a won level counts down to the next and takes
  it by itself, the last waits for its button back to the first, a lost one
  counts down to the same level again but not behind the level select,
  Enter and the buttons do not wait, an edit is played every time the level
  comes round, a map that is no level is free play.
- `simulation::mechanics_tests` - the numbers and the seat credit, a wave
  round's total, a held end screen counts down, waits at zero, and R still
  restarts it.
- `hud::hud_tests` - the end screen fits the smallest field in every form,
  its countdown over its three buttons in one centred row; `touch` - a claimed touch neither
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
