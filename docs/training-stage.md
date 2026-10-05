# Boot Camp: the training stage

Status: **built** (steps 2-4). The course is the first entry of `levels.toml`, shown as `LEVEL 0`,
and can be passed over; what is left is tuning by play. The concept lab
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
| Placement | **Level 0, skippable.** First in `levels.toml` (shown as `LEVEL 0`, so Lotus Lagoon stays `LEVEL 1`; the last level leads round to Lotus Lagoon, never back to training); a new progress file opens on it, a file already further opens where it was, and Lotus Lagoon is open in the level select from the start. |
| Scope | Seven lessons: drive, supply, fire, the drums, the frog, the first enemy, the shield and a second enemy. |
| Map | A **field map**, 60 x 24 cells (the course in rows 5-17), past the arena size on purpose: the follow camera leads the player east, and the off-screen arrows (an enemy's, the frog's) get introduced in passing. |
| Script | **Data in the map**: a `[[training.beat]]` list in the map's TOML, so the course is edited like a map and another map could use the same machinery. |
| Seats | **Solo.** One seat, local only. A couch session plays it with one seat; the lobby's map stepper never offers it. |
| Words | Every line a `frog-*` message in `lang/en.ftl`, with Slovenian drafted in `lang/sl.ftl` for review. |
| Delivery | This spec first, then the engine in the PRs listed at the end. |

## The course

Five pens along one east-west road (row 11), each closed by a door the beat
before it opens. The sketch is not to scale; `maps/boot-camp.toml` is the
layout.

```
col  0         10        19        28          40                  59
     ###################################################################
     #  F     F  #  A      #     W W #   WWWW  ff #   ~~~~~           #
     #           #         #  b      # o~~~oF     #           s s     #
     #   frog    D  frog   D    frog B            D     PAD   s       E
     =================================================================== road
     #           D         D         B   sss      D           s       E
     #  F     F  #  S  H   #  M b W W#   frog     #   kit     s s     E
     ###################################################################
       pen 1        pen 2     pen 3     pen 4 (drums)    pen 5 (arena)
```

`D` a door (three cells, rows 10-12), `B` the brick wall (in column 25, two
cells short of door 3, so a lane runs round it), `E` the enemy
gate on the east edge, `F` a flag, `A` ammo, `H` health, `S` speed-up, `M`
minigun, `W` wood, `b` brick targets, `s` sandbags, `~` the pond. In pen 4
`o` an oil drum, `F` a fuel drum, `~` an oil trail, `f` a fence and `WWWW`
a wooden shed: a lone oil drum (31,7) four rows north of the road, the
trail east of it and a cluster of two oil and two fuel drums (35-36, 6-7)
under the shed. One drum set off takes the rest: the oil drums burn, the
trail carries the fire across, and the fuel drums are thrown north and
east and blast the shed, the fence and the wood and brick by them where
they land. Nothing reaches the road with more than a scratch, and the frog
watches from behind the sandbags at (33,15).

| Beat | Teaches | The frog says (keys / touch) | Done when | Starts |
|---|---|---|---|---|
| 1 Drive | Four directions, momentum into turns | Hi! I'm your frog. / `<ARROWS>` to drive. Ease off before turns. (`<STICK>` Drag your left thumb to drive.) / Roll over the three flags. | Three flags taken | - |
| 2 Supply | Crates open on contact; ammo, health, speed | No shells yet. Bump the crates. / The red cross fixes your hull. | The ammo crate taken | Door 1 opens |
| 3 Fire | Aim by facing; wood, brick, iron | `<SPACE>` fires where your gun points. (`<TAP>` the right half to fire.) / Wood breaks fast. Brick takes a few. / Iron never breaks. Shoot the brick wall! | A cell of the brick wall gone | Door 2 opens |
| - Through | - | Nice shot! Follow me. | The tank past door 3 | Door 3 opens |
| 4 Drums | Drums explode and chain; oil burns, fire runs along a trail, fuel drums fly; a blast breaks walls | Drums blow up. Shoot from afar. / Red ones burn. Grey ones fly. / Fire runs along spilled oil. | Every drum on the range gone (`destroyed_all`) | - |
| - Onward | - | Boom! Blasts break walls too. Follow me. | The tank past door 5 | Door 5 opens |
| 5 Your frog | The frog matters; the frog kit; air drops | This is my pad. Come on in. / *(shot)* Ow! That came from the east! / Grab my kit, quick! | The frog kit taken (a kit can only be taken while the frog is hurt) | A shot from the east edge hits the frog; the frog kit air-drops |
| 6 Enemy | Gates, red rings, lining up; the frog bites | Here it comes! / Line up and fire! | The tank destroyed | A scout rolls in through the east gate, a *dummy* that fires only at the frog |
| 7 Shield | Shields; an enemy that fires back | Shield first. / This one shoots back! | The second tank destroyed | A shield crate air-drops, then 6 s later an ordinary scout rolls in |

The script has nine beats: the seven lessons and two short *through*
beats, one into the drum range and one after it, so the shot at the frog
comes once the player is in its pen. The drum range waits for the last
drum, not the first, so a player who shoots the cluster rather than the
lone drum still sees the whole chain, and a drum the fire missed can be
shot.

The player starts with **no shells and 60 of 100 health**, so beat 2 is not
optional and the red cross has a point. The speed-up and the minigun are
there to try; nothing waits on them.

**Nudges.** Each beat has one line the frog repeats when nothing has been
said for `training_nudge_seconds` (14): "Three flags. Then the gate opens.",
"The ammo crate is up north.", "Face the brick wall, press `<SPACE>`.",
"Shoot a drum. Watch the chain!", "My kit! The green crate!", "Get in line with it!", "Last one. You've got
this." This is what a sign would have done for a player who is lost.

**Situational lines**, once each: "Hold fire. It eats ammo fast." (the
minigun), "Ow! That came from the east!" and "Grab my kit, quick!" (the
frog hit in its beat), "Better! If I go down, we lose." (healed), "It only
wants me. Keep firing!" (the first enemy hit), "Chomp!" (the frog bites),
and "You're down! Back to the gate." each time the tank is wrecked. "Nice
shot! Follow me." is the *through* beat's line. A press with no shells says
nothing: the supply beat's nudge already points at the ammo crate.

**Shells.** A script that sets `start_shells` holds the seats' shell
refill until one of them opens an ammo crate, so the empty gun is the
lesson until the crate is found.

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

- **Length.** At most eight words; four or fewer in the two enemy beats. The
  bubble's width is a budget in `every_language_fits_every_budget`
  (220 pt, three lines at the bubble's size).
- **Input.** A line that names a control has a keys and a touch message
  (`frog-fire`, `frog-fire-touch`), picked when it is drawn from
  `UiFrame::hints`, so the words follow the input last used like the rest
  of the chrome. A control is a token in the text (`<ARROWS>`, `<SPACE>`,
  `<STICK>`, `<TAP>`; angle brackets, since a line opening with `[` would
  read as a Fluent variant) the bubble draws as a key cap or a thumb.
- **No question marks.** The catalogue's drawability test takes a `?` in
  a message for a letter the fold table lacks, so a line asks nothing.
- **Reveal.** Word by word, then held for `training_line_seconds` plus
  `training_line_seconds_per_char` of the line, so a long line stays as
  long as it takes to read.
- **Off screen.** While the frog is outside the view its bubble stands at
  the view's edge nearest it, with no tail, so nothing it says is lost
  behind the camera; the frog's own off-screen arrow points the way.
- **Clear of the HUD.** A bubble that would stand over a corner cluster
  goes under the frog instead.

| Message | en | sl (draft) |
|---|---|---|
| `frog-hello` | Hi! I'm your frog. | Živjo! Jaz sem tvoja žaba. |
| `frog-drive` | <ARROWS> to drive. Ease off before turns. | <ARROWS> za vožnjo. Pred zavojem popusti. |
| `frog-drive-touch` | <STICK> Drag your left thumb to drive. | <STICK> Za vožnjo vleci z levim palcem. |
| `frog-flags` | Roll over the three flags. | Zapelji čez tri zastavice. |
| `frog-flags-nudge` | Three flags. Then the gate opens. | Tri zastavice. Potem se vrata odprejo. |
| `frog-crates` | No shells yet. Bump the crates. | Brez granat. Zaleti se v zaboje. |
| `frog-health` | The red cross fixes your hull. | Rdeči križ popravi oklep. |
| `frog-crates-nudge` | The ammo crate is up north. | Zaboj s strelivom je na severu. |
| `frog-no-shells` | No shells. Find the ammo crate. | Ni granat. Poišči zaboj s strelivom. |
| `frog-fire` | <SPACE> fires where your gun points. | <SPACE> strelja, kamor kaže top. |
| `frog-fire-touch` | <TAP> the right half to fire. | <TAP> desno polovico za strel. |
| `frog-materials` | Wood breaks fast. Brick takes a few. | Les hitro poči. Opeka zdrži več. |
| `frog-iron` | Iron never breaks. Shoot the brick wall! | Železo nikoli. Ustreli opečni zid! |
| `frog-fire-nudge` | Face the brick wall, press <SPACE>. | Obrni se k zidu, pritisni <SPACE>. |
| `frog-fire-nudge-touch` | Face the brick wall and <TAP>. | Obrni se k zidu in <TAP>. |
| `frog-minigun` | Hold fire. It eats ammo fast. | Drži strel. Hitro porablja strelivo. |
| `frog-wall` | Nice shot! Follow me. | Lep strel! Za mano. |
| `frog-drums` | Drums blow up. Shoot from afar. | Sodi eksplodirajo. Streljaj od daleč. |
| `frog-drum-kinds` | Red ones burn. Grey ones fly. | Rdeči gorijo. Sivi letijo. |
| `frog-oil` | Fire runs along spilled oil. | Ogenj steče po razliti nafti. |
| `frog-drums-nudge` | Shoot a drum. Watch the chain! | Ustreli sod. Opazuj verižno reakcijo! |
| `frog-boom` | Boom! Blasts break walls too. Follow me. | Bum! Eksplozija podre tudi zid. Za mano. |
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
`destroyed` (any of these cells' tiles gone), `destroyed_all` (every one
of them gone), `past_col` (seat 1's hull
east of a column), `frog_full`, `wrecks` (enemy wrecks since the beat
began); a beat whose tanks are still to roll in is not done. Starts:
`shoot_frog` (one heavy enemy shell from a cell inside that edge along the
frog's row or column, landing like any other, fired once the frog stands on
the beat's cell and again every `training_frog_shot_retry_seconds` until
one has hurt it - a shell that met a wall or a tank on the way is fired
again), `drop` (crates air-dropped
onto cells), `roll_in` (tanks through the map's gates, `after` seconds into
the beat). `start_health` and `start_shells` set the seats up at the start.
`frog` is where the frog walks for the beat; `say` and `nudge` are its
lines (message keys).

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
  (`ai = "dummy"` is a hunter with `Ai::frog_only`).
- **Nothing a player does stalls a beat.** A band round keeps its wrecks,
  and a wreck a cell and a half from a gate's inside point keeps that lane
  busy as a standing tank does - a dummy shot the moment it rolls in
  would keep the last beat's scout out for good. So an enemy wreck fades
  off the field `training_wreck_seconds` after it went
  (`fade_training_wrecks`, through `despawn_wrecks`), and a roll-in that
  finds no free lane for `training_lane_wait_seconds` - a seat parked in it
  - drops onto the field out of sight as a band round places one
  (`spawn_in_band`), with the role the script asks for. A beat started
  again takes back the tanks still standing and the crates still lying
  that it put down, so its starts never pile up.
- **Knobs**: the `training` tuning group (`training_respawn_seconds`,
  `training_frog_revive_seconds`, `training_flag_reach_px`,
  `training_wreck_seconds`, `training_lane_wait_seconds`,
  `training_frog_shot_retry_seconds`, and the voice's).
- **Events**: `BeatDone`, `DoorOpened`, `FlagTaken` and `FrogRevived`;
  none travels on the wire, since no room plays a training map.
- **The linter** opens every door before it checks (`Game::open_every_door`),
  so a course is judged as it plays once its pens are open, and reports a
  door that names no beat of the script, or a door or flag on a map
  without one (`training-door`).

### The frog's voice

- **The walk** (`simulation/training.rs`, `walk_frog`): the frog hops
  toward the running beat's `frog` cell a hop at a time
  (`training_frog_stride` of its usual hop, the next as soon as it lands),
  routed on the nav cache's layer - the tiles with no frog in them - so it
  waits behind a closed door and goes through it once it opens. It never
  shies from a seat, so the player can drive right up to it and follow
  it; it still hops away from an enemy tank, except while it walks
  (`frog_walking`). It does not dodge shells at all - the frog beat needs
  the shot from the east to land.
- **The lines** (`bubble.rs`, `FrogVoice`, owned by `app.rs` beside
  `fx::Fx`): read after every step - a new beat clears what the last one
  still had to say and queues its `say` list; the frame's events add the
  situational lines - and run on the time the round ran, so a frozen round
  says nothing more. A line stays up `training_line_seconds` plus
  `training_line_seconds_per_char` of its length in the language on screen,
  its words coming in at `training_line_words_per_second`; after
  `training_nudge_seconds` with nothing said the beat's nudge comes again.
  The round itself never reads a word.
- **The bubble**: `bubble::layout` is the one geometry, in the bitmap's
  pixels at the UI's points (`ui.scale / view.scale`, as the indicators
  are), sized for the whole line so it never grows as the words come in;
  `render/bubble.rs` paints it after the indicators, under the HUD.
- **Dev server**: `status.training` is the beat (1-based), the beats, the
  beat's id and whether the course is done.

### The dummy and the words

- **The dummy** (`ai = "dummy"`): a hunter with `Ai::frog_only` set, which
  `Brain::may_fire_at_seat` reads - it drives at the frog and fires at it
  as a hunter does, and never takes a shot at a seat. The flag rides the
  roll-in's `Rejoin`, so a straggler rolled in again keeps it.
- **Words**: a round with a script is a training round whatever its
  mission, so the HUD's mission word reads `TRAINING` (`URJENJE`), the
  opening banner `FOLLOW THE FROG!` (`SLEDI ŽABI!`) and the end screen's
  title `TRAINING DONE!` (`OPRAVLJENO!`); the level's title, Boot Camp
  (`Vadišče`), stands under the banner as every level's does. The frog's
  last line is "You're ready."; the end screen's `NEXT LEVEL` names the
  way on.
- **A lane a seat stands in is busy**, as it is for a wave: a tank parked
  within a cell and a half of the east gate's inside point holds the
  beat's tank outside for `training_lane_wait_seconds`, after which it
  drops onto the field out of sight instead.

### Levels and skipping

`levels.toml` has Boot Camp first with `skippable = true`
(`levels::Level::skippable`). A skippable level keeps the next level open
in the level select while it is the furthest reached
(`Campaign::open_to`, which the select's tiles, focus and presses read), so
a player who knows the game picks Lotus Lagoon from the select (Esc, or
the bar's level button) and never sees training again once Lotus Lagoon is
won (`Campaign::won` moves `reached` past both). A progress file saved before Boot Camp existed
names a later map and opens there, untouched. That makes sixteen levels,
exactly one page of the select (`SELECT_TILES`); a seventeenth needs a
second page. A couch round on Boot Camp seats one: `Game::init` sets the
couch's count aside on a map with a training script and puts it back on
the next map without one, and the corners offer no players button there
(nor does the dialog open), so player 2 is back for Lotus Lagoon.

### Tests

- `simulation::training::tests`: a door stands until its beat and opens
  after it; the last beat wins; a wrecked seat comes back in the last door;
  a fallen frog gets up and the round goes on; a shot from the east hurts
  the frog and its kit drops and ends the beat; a beat's tank rolls in
  through a gate and its wreck ends the beat; a wreck in the lane never
  keeps the next beat's tank out; a seat parked in the lane holds the
  beat's tank only so long; a beat started again puts its crates down once;
  a `destroyed_all` beat waits for its last cell; Boot Camp's drum range
  goes up from the lone drum over six seeds, a fuel drum thrown and the
  shed blasted, with the frog unhurt and the seat on the road whole;
  Boot Camp plays through from the first beat to the last the way a player
  rushing it would (the dummy shot in the lane, the wall breached while the
  frog is still on its way); Boot Camp parses, is never hostable and lints
  with no error. `training::tests` round-trips a script.
- `bubble::tests`: a line wraps inside the bubble with keys at their
  width; the body is sized for the whole line; the bubble stands above the
  frog, under it near the top or over a corner cluster, and at the view's
  edge with no tail when the frog is off it; a touch screen takes a line's
  touch twin; the frog says each beat's lines in turn and nudges when it
  goes quiet; every shipped script's line is a message and a key constant.
  `simulation::training::tests` add the frog's walk through a door that
  opened and the held shell refill.
- Still to come: a headless run of the whole course from a scripted input
  and Boot Camp's seeded replay.

## Build order

1. **This spec** and the lab.
2. **Doors, flags and the script** (done): the map cells, `[training]`
   parsing and writing, `training_phase` with every trigger and start,
   respawn, `maps/boot-camp.toml` - playable from the builder's Load list
   with the frog silent.
3. **The frog's voice** (done): the walk, the line queue, nudges, the
   bubble, the `frog-*` messages.
4. **The dummy, the words and level 0** (done): the dummy, the mission
   word, banner and end screen, `levels.toml` with `skippable`, the level
   select's rule. `maps/boot-camp.toml` is tuned by play from here.

## Open

- The course's par time (the lab plays in about three minutes with no
  stops).
- Whether the frog's first line should name the skip ("Know all this already. Esc
  takes you to the levels.") for players who start a fresh device.
