# Levels: playing the maps in order

Status: **implemented 2026-09-29**. docs/maps-to-levels.md turned a map
into a level (a mission and a spawn plan). This document strings the levels
together: `levels.toml` lists them in order, a win opens the next one, and
the end screen waits for the player instead of restarting the round.

## Decisions (2026-09-29)

| Topic | Decision |
|---|---|
| The list | `levels.toml` at the repository root: `[[level]]` entries in play order, each naming a map (`map`, its file stem under `maps/`, one of `SHIPPED_MAPS`) and the `title` its banner shows. The maps stay separate files. Compiled in. |
| Order | Roughly easy to hard: Lotus Lagoon (5 tanks) first, Grand Campaign (7 waves) last. Edit `levels.toml` to change it. |
| Progression | Only a win opens the next level. A loss offers the same level again. Winning the last level shows "all levels complete" and leads back to level 1. |
| Start and save | A session opens on the furthest level reached. On the web that is the page's `localStorage` (`bongbong.level`); on a desktop or a phone, a file. Progress is kept by map name, so reordering the list keeps it. |
| End screen | Waits for a button: `PLAY AGAIN` always, `NEXT LEVEL` after a win (`BACK TO LEVEL 1` after the last). Enter takes the way on after a win and plays again after a loss; R plays again, as it always did. |
| Stats | Time (the round clock, which stands still behind the banner and while paused) and enemies destroyed out of all the round brings, split by seat in a couch round. |
| Banner | `LEVEL 3 / 14` over the mission banner, the level's title under it at half the banner's size (36 px under 72). |
| Builder edits | A level edited in the builder is played as edited for the rest of the session, every time it comes round. On a desktop the builder's Save writes it to `maps/<map>.toml`, which later sessions read first. |
| Free play | Any map that is not a level (`-m`, the builder's Load list, a Save As under a new name) restarts on its own after `restart_delay`, as every round did before; its end screen shows the same numbers. |
| Online | Unchanged: a room's round counts down to the room's lobby. |

## The flow

**A level is a map by name.** `Session::level` is the index in the list of
the local round's map name, so there is no "current level" to keep in step
with anything: a level loaded into the builder and played is still that
level, `-m maps/carnival.toml` is the Carnival level, and a map under any
other name is free play. Every place the round's map changes (`play`,
`replace_map`, `start_level`, `set_campaign`) sets `Game::hold_end_screen`
from it: a level's end screen waits for its buttons, free play's counts
down and restarts inside `Game::update` as before.

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
pressed. A tap that presses one is claimed from the touch scheme
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

## Words

The English titles are `levels.toml`'s. Another language translates a title
as `level-<map>` in its `lang/*.ftl` (`text_tests` allows those ids beyond
English's and measures every title against the smallest field). The
chrome is `level-number`, `result-time`, `result-wrecks`,
`result-all-clear`, `result-again`, `result-next` and `result-first`, each
with its budget in `text_tests::budgets`.

## Tests

- `levels::level_tests` - every level is a shipped map; a broken list is
  refused by name; `--level` by number or name; progress moves once, only
  forward, and round-trips through a file; an edit is the level from then on.
- `mode::session_tests` - a won level waits and opens the next, the last
  leads back to the first, a loss offers only `PLAY AGAIN`, an edit is
  played every time the level comes round, a map that is no level is free
  play.
- `simulation::mechanics_tests` - the numbers and the seat credit, a wave
  round's total, a held end screen waits and R still restarts it.
- `hud::hud_tests` - the end screen fits the smallest field in every form;
  `touch` - a claimed touch neither fires nor steers; `devserver` - the end
  screen takes `click` and `key`.
