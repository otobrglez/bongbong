# Boot Camp: the training stage

Status: **in progress**. The map, its doors and flags and the beat script
run (build step 2); the frog's voice, the dummy and level 0 follow. The
concept lab
(`docs/lab/boot-camp-lab.html`, open it from the repository so it finds
`static/`) is a playable browser prototype of everything below: drive it,
then read this.

Boot Camp is level 0: a short course that teaches a new player to drive, open
crates, fire, look after the frog and beat the first enemy, in about three
minutes, before Lotus Lagoon. It is a field map like any other plus a short
script of *beats*, and the player's own frog is the only guide.

## Decisions (2026-10-03)

| Topic | Decision |
|---|---|
| Teaching | **The frog alone.** No signs, floor paint or HUD checklist: the player's own frog hops ahead from pen to pen and says each rule in a speech bubble. |
| The frog's voice | Short, light lines and no name. At most eight words a line, four or fewer once an enemy is on the field. |
| Placement | **Level 0, skippable.** First in `levels.toml`; a new progress file opens on it, a file already further opens where it was, and Lotus Lagoon is open in the level select from the start. |
| Scope | All six beats: drive, supply, fire, the frog, the first enemy, the shield and a second enemy. |
| Map | A **field map**, 48 x 13 cells, past the arena size on purpose: the follow camera leads the player east, and the off-screen arrows (an enemy's, the frog's) get introduced in passing. |
| Script | **Data in the map**: a `[[training.beat]]` list in the map's TOML, so the course is edited like a map and another map could use the same machinery. |
| Seats | **Solo.** One seat, local only. A couch session plays it with one seat; the lobby's map stepper never offers it. |
| Words | Every line a `frog-*` message in `lang/en.ftl`, with Slovenian drafted in `lang/sl.ftl` for review. |
| Delivery | This spec first, then the engine in the PRs listed at the end. |

## The course

Four pens along one east-west road (row 6), each closed by a door the beat
before it opens. The sketch is not to scale; `maps/boot-camp.toml` is the
layout.

```
col  0         10        19        28                  47
     #########################################################
     #  F     F  #  A      #     W W #   ~~~~~           #
     #           #         #  b      #           s s     #
     #   frog    D  frog   D    frog B     PAD   s       E
     ======================================================== road
     #           D         D         B           s       E
     #  F     F  #  S  H   #  M b W W#   kit     s s     E
     #########################################################
       pen 1        pen 2     pen 3          pen 4 (arena)
```

`D` a door (three cells, rows 5-7), `B` the brick wall (in column 25, two
cells short of door 3, so a lane runs round it), `E` the enemy
gate on the east edge, `F` a flag, `A` ammo, `H` health, `S` speed-up, `M`
minigun, `W` wood, `b` brick targets, `s` sandbags, `~` the pond.

| Beat | Teaches | The frog says (keys / touch) | Done when | Starts |
|---|---|---|---|---|
| 1 Drive | Four directions, momentum into turns | Hi! I'm your frog. / `[ARROWS]` to drive. Ease off before turns. (`[STICK]` Drag your left thumb to drive.) / Roll over the three flags. | Three flags taken | - |
| 2 Supply | Crates open on contact; ammo, health, speed | No shells yet? Bump the crates. / The red cross fixes your hull. | The ammo crate taken | Door 1 opens |
| 3 Fire | Aim by facing; wood, brick, iron | `[SPACE]` fires where your gun points. (`[TAP]` the right half to fire.) / Wood breaks fast. Brick takes a few. / Iron never breaks. Shoot the brick wall! | A cell of the brick wall gone | Door 2 opens |
| - Through | - | Nice shot! Follow me. | The tank past door 3 | Door 3 opens |
| 4 Your frog | The frog matters; the frog kit; air drops | This is my pad. Come on in. / *(shot)* Ow! That came from the east! / Grab my kit, quick! | The frog kit taken (a kit can only be taken while the frog is hurt) | A shot from the east edge hits the frog; the frog kit air-drops |
| 5 Enemy | Gates, red rings, lining up; the frog bites | Here it comes! / Line up and fire! | The tank destroyed | A scout rolls in through the east gate, a *dummy* that fires only at the frog |
| 6 Shield | Shields; an enemy that fires back | Shield first. / This one shoots back! | The second tank destroyed | A shield crate air-drops, then 6 s later an ordinary scout rolls in |

The script has seven beats: the six lessons and a short *through* beat
between the wall and the frog, so the shot at the frog comes once the
player is in its pen.

The player starts with **no shells and 60 of 100 health**, so beat 2 is not
optional and the red cross has a point. The speed-up and the minigun are
there to try; nothing waits on them.

**Nudges.** Each beat has one line the frog repeats when nothing has been
said for `training_nudge_seconds` (14): "Three flags. Then the gate opens.",
"The ammo crate is up north.", "Face the brick wall, press `[SPACE]`.", "My
kit! The green crate!", "Get in line with it!", "Last one. You've got
this." This is what a sign would have done for a player who is lost.

**Situational lines**, once each: "No shells. Find the ammo crate." (a fire
press with none), "Hold fire. It eats ammo fast." (the minigun), "Nice shot!
Follow me." (the wall broken), "Better! If I go down, we lose." (healed),
"It only wants me. Keep firing!" (the dummy hit), "Chomp!" (the frog bites).

**Failure is never the end.** A wrecked tank comes back at the last opened
door after `training_respawn_seconds` (2) with full health ("You're down!
Back to the gate."). A frog that goes down puts its beat back: the frog
revived at its spot, the beat's enemies and shots removed, its starts run
again. The round is never lost.

**The end.** The last beat done ends the round won: the end screen reads
`TRAINING COMPLETE` with the round clock and counts down into Lotus Lagoon
like any level ("You're ready. Next: Lotus Lagoon.").

## The frog's lines

Rules for every line, held by tests where a test can hold them:

- **Length.** At most eight words; four or fewer in beats 5 and 6. The
  bubble's width is a budget in `every_language_fits_every_budget`
  (220 pt, three lines at the bubble's size).
- **Input.** A line that names a control has a keys and a touch message
  (`frog-fire`, `frog-fire-touch`), picked when it is drawn from
  `UiFrame::hints`, so the words follow the input last used like the rest
  of the chrome. A control is a token in the text (`[ARROWS]`, `[SPACE]`,
  `[STICK]`, `[TAP]`) the bubble draws as a key cap or a thumb.
- **Reveal.** Word by word, then held for `training_line_seconds` plus
  `training_line_seconds_per_char` of the line, so a long line stays as
  long as it takes to read.
- **Off screen.** While the frog is outside the view the off-screen arrow
  already drawn for it (`ArrowKind` frog, the FROG gauge's green) carries
  the bubble's text clipped to one line, so nothing the frog says is lost
  behind the camera.

| Message | en | sl (draft) |
|---|---|---|
| `frog-hello` | Hi! I'm your frog. | Živjo! Jaz sem tvoja žaba. |
| `frog-drive` | [ARROWS] to drive. Ease off before turns. | [ARROWS] za vožnjo. Pred zavojem popusti. |
| `frog-drive-touch` | [STICK] Drag your left thumb to drive. | [STICK] Za vožnjo vleci z levim palcem. |
| `frog-flags` | Roll over the three flags. | Zapelji čez tri zastavice. |
| `frog-flags-nudge` | Three flags. Then the gate opens. | Tri zastavice. Potem se vrata odprejo. |
| `frog-crates` | No shells yet? Bump the crates. | Brez granat? Zaleti se v zaboje. |
| `frog-health` | The red cross fixes your hull. | Rdeči križ popravi oklep. |
| `frog-crates-nudge` | The ammo crate is up north. | Zaboj s strelivom je na severu. |
| `frog-no-shells` | No shells. Find the ammo crate. | Ni granat. Poišči zaboj s strelivom. |
| `frog-fire` | [SPACE] fires where your gun points. | [SPACE] strelja, kamor kaže top. |
| `frog-fire-touch` | [TAP] the right half to fire. | [TAP] desno polovico za strel. |
| `frog-materials` | Wood breaks fast. Brick takes a few. | Les hitro poči. Opeka zdrži več. |
| `frog-iron` | Iron never breaks. Shoot the brick wall! | Železo nikoli. Ustreli opečni zid! |
| `frog-fire-nudge` | Face the brick wall, press [SPACE]. | Obrni se k zidu, pritisni [SPACE]. |
| `frog-fire-nudge-touch` | Face the brick wall and [TAP]. | Obrni se k zidu in [TAP]. |
| `frog-minigun` | Hold fire. It eats ammo fast. | Drži strel. Hitro porablja strelivo. |
| `frog-wall` | Nice shot! Follow me. | Lep strel! Za mano. |
| `frog-pad` | This is my pad. Come on in. | To je moj dom. Kar naprej. |
| `frog-ow` | Ow! That came from the east! | Au! To je priletelo z vzhoda! |
| `frog-kit` | Grab my kit, quick! | Hitro, poberi moj zaboj! |
| `frog-kit-nudge` | My kit! The green crate! | Moj zaboj! Zeleni! |
| `frog-healed` | Better! If I go down, we lose. | Bolje! Če padem, izgubiva. |
| `frog-enemy` | Here it comes! | Prihaja! |
| `frog-line-up` | Line up and fire! | Poravnaj se in streljaj! |
| `frog-enemy-nudge` | Get in line with it! | Poravnaj se z njim! |
| `frog-only-me` | It only wants me. Keep firing! | Hoče samo mene. Streljaj! |
| `frog-chomp` | Chomp! | Hrsk! |
| `frog-shield` | Shield first. | Najprej ščit. |
| `frog-shoots-back` | This one shoots back! | Ta strelja nazaj! |
| `frog-wave-nudge` | Last one. You've got this. | Še zadnji. Zmoreš. |
| `frog-down` | You're down! Back to the gate. | Tank je uničen! Nazaj k vratom. |
| `frog-ready` | You're ready. | Vse je nared. |
| `frog-next` | Next: { $title }. | Naprej: { $title }. |
| `mission-training` | TRAINING | URJENJE |
| `mission-training-banner` | BOOT CAMP | VADIŠČE |
| `result-training` | TRAINING COMPLETE | URJENJE KONČANO |
| `level-boot-camp` | *(title in levels.toml: Boot Camp)* | Vadišče |

Every letter folds onto the default font through `text::fold` (č, š, ž).

## How it lands in the engine

### The map

`maps/boot-camp.toml`, `size = [48.0, 13.0]`, a Protect map on the band
plan with no enemies (`tanks = 0`), in `SHIPPED_MAPS` (so the builder's
Load list offers it on every build) and left out of
`map::hostable_maps`, which the lobby's stepper walks. Training is not a
mission of its own: a map with a script is a training round, whatever its
mission, and the script decides how it ends. Two new cell kinds:

- **`door`** (`kind = "door", beat = N`): `Material::Door`, permanent like
  iron - a seam-closed static collider, a nav-grid wall, stops shots and
  sight and takes no damage - until beat `N` (1-based, in script order)
  is done, when the round marks every door of that beat destroyed and the
  frame's cleanup takes it away with no rubble and no `ObstacleDestroyed`
  (`Event::DoorOpened { beat, x, y }`, one per cell). Its `Obstacle::variant`
  carries the beat, as a barrel's carries its drum. Drawn by
  `training::draw_door`: banded iron with a hazard band.
- **`flag`**: not solid and not an obstacle; the training run keeps the
  map's flags (`Game::training_flags`), and the first seat whose hull box,
  grown by `training_flag_reach_px`, touches one takes it
  (`Event::FlagTaken`). Drawn by `training::draw_flag` in player 1's team
  colour, gold once taken.

### The script

`MapFile::training: Option<training::Training>`, written last as
`[training]` and `[[training.beat]]` tables so a re-save round-trips it:

```toml
[training]
start_health = 0.6
start_shells = 0

[[training.beat]]
id = "drive"
frog = [5, 4]
say = ["frog-hello", "frog-drive", "frog-flags"]
nudge = "frog-flags-nudge"
done = { flags = 3 }

[[training.beat]]
id = "frog"
frog = [34, 6]
say = ["frog-pad"]
start = { shoot_frog = "east", drop = [{ kind = "frog_health", at = [31, 10] }] }
done = { collect = "frog_health" }

[[training.beat]]
id = "enemy"
say = ["frog-enemy", "frog-line-up"]
nudge = "frog-enemy-nudge"
start = { roll_in = [{ tank = "scout", ai = "dummy", after = 1.5 }] }
done = { wrecks = 1 }
```

Triggers (`done`, every one set must hold): `flags` (taken over the
round), `collect` (a crate kind a seat took since the beat began),
`destroyed` (any of these cells' tiles gone), `past_col` (seat 1's hull
east of a column), `frog_full`, `wrecks` (enemy wrecks since the beat
began); a beat whose tanks are still to roll in is not done. Starts:
`shoot_frog` (one heavy enemy shell from a cell inside that edge along the
frog's row or column, landing like any other), `drop` (crates air-dropped
onto cells), `roll_in` (tanks through the map's gates, `after` seconds into
the beat). `start_health` and `start_shells` set the seats up at the start.
`frog`, `say` and `nudge` are read by the frog's voice (build step 3).

### The simulation

- **`simulation/training.rs`**: `Game::init_training` makes the run
  (`training::Run`) once the seats and the map are down; `training_phase`
  runs after the frame's blasts and wreck removals and before the
  cleanup that sweeps away the doors it opens: the crates seats took, the
  flags, the seats and the frog brought back, the beat's starts, its
  roll-ins, then its triggers - and on its end its doors and the next beat.
  **No RNG of its own**: the checks are pure and the starts fixed by the
  map; only what they set going draws (a roll-in's lane and rolls, a
  shell's damage). A map without a script runs none of it.
- **The round**: won when the last beat is done, never lost
  (`check_round_end` defers to the run). A wrecked seat comes back after
  `training_respawn_seconds` as a fresh tank of its chassis, keeping its
  shells, in the middle cell of the last door opened (its own start before
  any), announced as `TankEntered`. A frog down for
  `training_frog_revive_seconds` gets up at full health where it fell
  (`Event::FrogRevived`) and its beat starts again: the beat's live tanks
  taken away, its starts run once more.
- **Roll-ins**: `Game::training_roll_in` is a wave tank's roll-in through
  `pick_gate`'s lane, arriving with the role the script asks for
  (`ai = "dummy"` is a hunter until the dummy exists).
- **Knobs**: the `training` tuning group (`training_respawn_seconds`,
  `training_frog_revive_seconds`, `training_flag_reach_px`).
- **Events**: `BeatDone`, `DoorOpened`, `FlagTaken` and `FrogRevived`;
  none travels on the wire, since no room plays a training map.
- **The linter** opens every door before it checks (`Game::open_every_door`),
  so a course is judged as it plays once its pens are open, and reports a
  door that names no beat of the script, or a door or flag on a map
  without one (`training-door`).

### Still to come

- **The frog's walk** to each beat's `frog` cell through the open doors,
  hopping, and **its lines**: a queue on the round clock (so a lockstep
  replays them and `status.training` reports them), nudges after
  `training_nudge_seconds`, the bubble (`bubble.rs` and
  `render/bubble.rs`, in UI points over the world) and the off-screen
  arrow carrying the line. Only the drawing picks the keys or touch text.
- **The dummy**: an `Ai` role that targets only the opposing frog and
  fires every `training_dummy_fire_seconds` (2.6).
- **Words on screen**: the HUD's mission word `TRAINING`, the banner
  `BOOT CAMP`, the end screen's `TRAINING COMPLETE`.

### Levels and skipping

`levels.toml` gains Boot Camp first with `skippable = true`. A skippable
level keeps the next level open in the level select while it is the
furthest reached, so a player who knows the game picks Lotus Lagoon from
the select (Esc, or the bar's level button) and never sees training again
once Lotus Lagoon is won. A progress file saved before Boot Camp existed
names a later map and opens there, untouched. That makes sixteen levels,
exactly one page of the select (`SELECT_TILES`); a seventeenth needs a
second page. A couch round on Boot Camp seats one tank.

### Tests

- `simulation::training::tests`: a door stands until its beat and opens
  after it; the last beat wins; a wrecked seat comes back in the last door;
  a fallen frog gets up and the round goes on; a shot from the east hurts
  the frog and its kit drops and ends the beat; a beat's tank rolls in
  through a gate and its wreck ends the beat; Boot Camp parses, is never
  hostable and lints with no error. `training::tests` round-trips a script.
- Still to come: a headless run of the whole course from a scripted input,
  Boot Camp's seeded replay, and the text budgets of the frog's lines.

## Build order

1. **This spec** and the lab.
2. **Doors, flags and the script** (done): the map cells, `[training]`
   parsing and writing, `training_phase` with every trigger and start,
   respawn, `maps/boot-camp.toml` - playable from the builder's Load list
   with the frog silent.
3. **The frog's voice**: the walk, the line queue, nudges, the bubble, the
   off-screen arrow's line, the `frog-*` messages.
4. **The dummy, the words and level 0**: the dummy role, the mission word,
   banner and end screen, `maps/boot-camp.toml` tuned by play,
   `levels.toml` with `skippable`, the level select's rule.

## Open

- The course's par time (the lab plays in about three minutes with no
  stops).
- Whether the frog's first line should name the skip ("Know all this? Esc
  takes you to the levels.") for players who start a fresh device.
