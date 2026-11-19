# November 2026: the rebuild and upgrade plan

An audit of the whole game outside online co-op - main loop, renderer,
physics, effects, the rules of the weapons and the round, enemy AI, build and
test health - and the plan that came out of it (BB-153). Co-op (`src/net/`,
`server/`, `netlab/`) was out of scope; where an item here would touch the
wire or a replica, the item says so.

This is a plan, not a record of what is built. Each item is filed as its own
Linear issue when it is picked up, and the design doc of the module it
changes is updated then. Item ids (`P0-1`, `D-3`, ...) exist so an issue can
name the item it does.

## How to read the evidence

The audit was five parallel code reads on 2026-11-19 at `b89fba5` (0.2.10).
Only one of them ran anything. Every finding carries one of three marks:

- **measured** - a command was run and the number is its output.
- **read** - the code was read on both sides of the claim; nothing was run.
- **estimate** - arithmetic from tuning rows or reasoning about cost; needs a
  dev-server or probe run before it is trusted.

Line numbers are as of `b89fba5` and will drift; the function names are the
durable reference.

## Decisions taken (Oto, 2026-11-19)

1. **Four-direction movement is the model, and it gets a dedicated quality
   track** (section "Track D"). Free-steer (BB-128, BB-129, BB-130, PR #155)
   stays the proof of concept it is: nothing here waits for it, nothing here
   is built for it, and the work goes into making four-direction good.
2. **Friendly fire is full damage from every source.** A shell, a beam, a
   blast, a flame or a ram hurts whoever it reaches, the shooter and the
   shooter's side included. `friendly_fire_damage_factor` stays 1.0 and the
   side-sparing in `side_blast` goes (item `M-2`).
3. **The default font stays.** No bitmap font; revisit when more languages
   arrive.
4. **The plan lives in this document.**

## State of the game

### Measured

| What | Result |
|---|---|
| `cargo test --lib --no-default-features --features dev-tools` | 2053 passed, 0 failed, 7 ignored (six timing benchmarks, one net measurement); two runs, no flake; about 40 s |
| `cargo build --no-default-features --features dev-tools` | 0 warnings |
| `cargo clippy --lib --no-default-features --features dev-tools` | **fails**: 5 deny-level errors (`approx_constant` x3 at `volcano.rs:416`, x1 at `weather/plain.rs:260`; `modulo_one` at `net/rig.rs:534`), 209 warnings (96 are doc-list indentation) |
| `cargo fmt --check` | 9616 hunks in 235 files; no config is checked in, so this is not a defect |
| `just probe-fixtures` | passes. `maze` sits at its jitter ceiling (9) and spin ceiling (1); `props` and `u-trap` at border-stuck (1); `tight-corridors` at spin (1). Clustering and pile-up are 0 everywhere |
| `just probe-fields` | **fails on master**: `hedge-maze` jitter 33 against a ceiling of 31. The other six maps pass (run one by one) |
| Simulation cost, release, headless | 0.075 ms/tick on the default map, 0.27 on longwater (112x63), 0.32-0.36 on a mostly empty 250x250 |
| Dependencies | nothing a major version behind; 44 semver-compatible updates. `sola-raylib` 6.4.0 is published (the game is on 6.3.1); `rapier2d` 0.36.1, `rand` 0.10.4, `toml` 1.2.1 |
| Markers | no TODO/FIXME/HACK in `src/` |

Two facts about the probe gate matter for everything below. Every AFK round
is lost in 4 to 15 seconds, so a "1800 frames x 10 rounds" sweep simulates
250 to 920 ticks a round: CI's gate watches about five seconds of AI. And
three traced anomalies were not defects: a staircase along a diagonal route
counted as jitter, a 3-second patrol re-roll counted as churn, and a sensible
flee-heal-return trip counted as churn.

### What CI does not run

Clippy; `just probe-fields` (which is how it went red unnoticed),
`probe-defend`, `probe-sweep`, `probe-waves`; `cargo test -p bongbong-server`;
`tests/waves_sanity.rs` (`--lib` skips integration tests); anything gated on
`dev-tools` (61 tests in `devserver.rs`, 4 in `capi.rs`); the headless test
configuration; `just check-sheets`.

### Not measured

Frame times, draw calls and quad counts on any platform (no counters exist);
GPU cost of the weather passes on phones; wasm simulation and AI cost; a
dense 250x250 map with eight seats; whether ProMotion iPhones present at
120 Hz with this SDL build.

### Already solid - leave alone

- The fixed-step contract: whole 1/60 s steps, a 4-step cap with the
  remainder dropped, the clock reset when the round is not playing.
- The world-resolution offscreen pipeline, culling on followed maps, the
  `Canvas` trait with pinned `CpuCanvas` hashes, HiDPI handling.
- The swept segment-vs-box hit test and its tie order; the speed cap and
  soft-CCD look-ahead (a hull stops flush at up to 1920 px/s); the skid model
  in `sonic.rs`, which is the reference for how every knock should work.
- The fixed-body bong and uniform static friction: both were measured and
  documented in `physics.rs`.
- `pyro.rs` as the effects vocabulary; the hashed composers; the shake design
  (capped stack, 2 px snap); ground memory and driving effects; the night
  light map; the `*_show` / `Spectacle` split.
- All nine shader twins are in sync today.

## Findings

### Defects

| Id | Finding | Where | Mark |
|---|---|---|---|
| F-1 | A missile, grenade, FPV or air-strike blast near a seat rolling back in through a gate panics: `side_blast_sparing` walks `players()` with no `RollIn` filter and `knockback` does `tank.body.expect(..)` on a seat that has no body yet. `guide_missiles` builds targets from `players()` too. Wave rounds of two or more seats | `simulation/missiles.rs` `side_blast_sparing`, `guide_missiles`; `combat.rs` `knockback` | read |
| F-2 | Deep-lava colliders are spawned by iterating a `HashSet`, so rapier's handles differ between two runs of one seed. The water path beside it iterates the Vec | `simulation/mod.rs` `init` (`deep_lava`) | read |
| F-3 | A bong stays pinned for the round after an EMP collapses a pulling well: `pull_bongs` runs against the frame's stale well list and sets `pulled` again after the collapse cleared it | `simulation/well.rs` | read |
| F-4 | A tuning range with min >= max panics the round: `random_range` on the shell, bong, spall and ram damage ranges is unguarded, where `BlastParams::roll_damage` guards the same case. Also `enemy_count_min > max`, `enemy_speed_variance = 0`, `tesla_chain_factor = 0` | `combat.rs`; `simulation/mod.rs`; `towers.rs` | read |
| F-5 | Particles and emitters keep running while the round is paused or a dialog is open, over frozen fireballs | `app.rs` (`fx.tick(fx_dt)`) | read |
| F-6 | The emitter rate limiter clears every accumulator each tick once it holds more than 256 keys, so every emitter under 60/s (damage smoke, wreck smoke, embers, drive dust, lava motes) stops. Shipped maps stay under the limit (`vulkan` has 72 lava cells); a builder map with a lava lake does not | `fx.rs` (`accum`) | read |
| F-7 | A dead tile's neighbours keep their seam-widened collider, so a one-cell breach is 24 px wide for hulls and 32 px for shots and the nav grid | `simulation/mod.rs` `cleanup_done`; `props.rs` `fit_tile_body` | read |
| F-8 | A player gun tower can shoot its own side's bong: the tower skip and `friend_in_line` cover tanks only | `hits.rs`; `towers.rs` | read |
| F-9 | The flame cone is wall-checked on its zero-width centre line only, so its flank burns a tank behind a wall corner, and flame passes through tanks to burn what is behind | `simulation/flame.rs` | read |
| F-10 | The last wave has no straggler timeout, and arena and band rounds have no straggler rule; a band enemy nobody wakes on a field map sleeps for the round | `simulation/waves.rs`; `field.rs` | read |
| F-11 | Seats parked on every free gate lane starve the wave forever | `waves.rs` | read |
| F-12 | A wrecked seat does not return on the final wave or on a gate-less wave map | `waves.rs` | read |
| F-13 | Contact sparks key on the contact pixel and so almost never fire; `Event::Ram` has no effect arm at all | `fx.rs` | read |
| F-14 | The campaign is counted two ways ("LEVEL 15 / 15", then "ALL 16 LEVELS COMPLETE!"); the last level can never show as won; Boot Camp's end screen reads "DESTROYED 2 / 0" | `mode.rs`; `level_select.rs`; `simulation/mod.rs` | read |
| F-15 | Level locks are cosmetic: any win of a level's map, including under `--enemies 1` or on an edited copy, unlocks past it | `mode.rs`; `levels.rs` | read |

### Frame pacing and pixel quality

| Id | Finding | Mark |
|---|---|---|
| R-1 | No render interpolation. The world is drawn at the last simulation step whatever the display rate: 120 Hz shows each world frame twice, 144/90/75/50 Hz give uneven step patterns, and a followed map scrolls with that judder. Particles run at display rate over a stepping world | read |
| R-2 | The step clock takes raw frame time with no smoothing, so vsync wobble around 16.67 ms yields occasional 0-step then 2-step frames | read; frequency unmeasured |
| R-3 | Desktop runs without vsync, capped at 120 fps by sleep: tearing, and on a 60 Hz monitor the pipeline renders twice per step for nothing | read |
| R-4 | The end-of-round slow motion starves the step clock, so the showcase moment plays at 30 steps a second | read |
| R-5 | Translucent draws lower the render target's alpha and the target is alpha-blended again, twice. 50% smoke over opaque ground is darkened 25% and tinted toward the backdrop. Three code comments already work around it | read; visual size unmeasured |
| R-6 | Upscaling is nearest-neighbour at arbitrary scales: a 1280x720 window scales the arena 1.18x, so a 2 px block is 2 or 3 device pixels wide depending on its column | read |
| R-7 | On a followed map the camera moves in device pixels while sprites sit at unrounded world positions, so the followed tank can wobble a texel against the ground | estimate |
| R-8 | Camera shake shifts the scene with no overscan (the black clear shows at the edge); ripple shaders sample outside [0,1] and the wrap mode differs by platform | read |
| R-9 | Hull and turret visual rotations ease through intermediate angles with nearest sampling: pixel crawl during every turn | estimate |

### Render pipeline

| Id | Finding | Mark |
|---|---|---|
| R-10 | Nothing counts draw calls, quads, flushes or GPU time; `frame_stages` has CPU averages only, in dev builds only; no benchmarks | read |
| R-11 | About 22 separate atlases and no shapes texture, so sprite/rectangle alternation in the y-sorted walk breaks batches. `tank_modules*.png` is 2120 px wide, above the 2048 limit of the oldest ES 2 GPUs | read; count unmeasured |
| R-12 | `paint_field_glowing` is 590 lines with about 15 additive blocks, each two forced flushes | read |
| R-13 | About 50 `Vec` builds a frame in the draw path; `tuning()` (a lock and an `Arc` clone) is called per grass tuft, per smoke particle and several times per tank; HUD strings run Fluent formatting every frame | read; cost estimate |
| R-14 | Several walks are per map, not per view: every obstacle is visited about 8 times a frame, every grass tuft, every worn cell, every scorch and decal | read |
| R-15 | Effects are CPU-rasterised into 2 px rectangles every frame; an estimated 10-20 thousand quads at the 900-particle cap | estimate |
| R-16 | Arena margins are redrawn through four window-sized targets every frame, under a clear sky too; the end screen is rendered twice a frame; every render target carries an unused depth buffer | read |
| R-17 | Render-target and texture failures panic in the main loop while weather, shot and dissolve shaders fail soft. Ripple shaders load from disk by path and `expect`; the rest are embedded | read |
| R-18 | The web build evaluates JavaScript strings every frame (`page_string`, the canvas CSS size) | read |
| R-19 | `app::run` is 1,569 lines with a 990-line frame closure and 131 `#[cfg]` attributes; the weather stage orchestration is written three times (round, builder, margins) | measured (line counts) |

### Physics, locomotion and collision

| Id | Finding | Mark |
|---|---|---|
| X-1 | Every chassis drives with the same 4200 force and brakes with a mass-blind viscous term, so a titan and a scout pushing head-on stalemate and any tank shoves any parked tank at about 52 px/s | read; numbers estimate |
| X-2 | Wrecks are easier to push than parked live tanks: damping 4.0 resists less than the drive force, and `wreck_friction` does almost nothing with no ground normal force | estimate |
| X-3 | Blast knockback is clamped to `knockback_max_speed` 60 px/s and sets no skid, so the next frame's drive erases it: about 3 px of displacement from a wreck, barrel, fuel, grenade or missile blast alike. Five tuned rows are indistinguishable | read; displacement estimate |
| X-4 | Knock mass-scaling follows four rules: shell impact is mass-blind, blast and recoil use `reference_mass / mass`, ram splits by mass, the specials use curves | read |
| X-5 | Blasts ignore line of sight and measure to hull centres, so an iron wall gives no cover from a barrel and a titan is hit from further inside its hull than a scout. Rod, EMP and well collapse measure to the nearest hull point | read |
| X-6 | Blast side effects differ by source: which bongs are hurt, whether lanterns break, whether grenades on the ground move | read |
| X-7 | Turning swaps the collider in place with no fit check. A longbow or obelisk grows 9 px on one axis and rapier resolves overlap at 3 px/s, so a turn beside a wall can leave the hull sunk for seconds | estimate; a candidate cause of wedging |
| X-8 | Each weapon reads "what is solid" its own way: shells use per-chunk boxes and pass a loophole or a rebar cage; the rail, the flame cap and a grenade use the whole seam-closed box | read |
| X-9 | Five geometries answer "is this tank hit": hull plus turret box (shots), nearest hull point (rod, EMP, well), centre plus corners (sonic), centre plus a quarter hull (flame), centre (blasts) | read |
| X-10 | The AI's line of sight passes fences, lamps, firs, palms and snags; the shot sweep stops on all of them. Lasers and rails skip the pass-over and deflect rolls that shells make | read |
| X-11 | Grenades have their own swept test and their own solids: no bongs, no deep water or lava (a grenade rolls across a lake), wrecks solid | read |
| X-12 | Bong hops and well pulls check tiles only, so a bong can land on or be dragged into a tank | read |
| X-13 | Several RNG-drawing walks follow hecs storage order (`enemy_phase` apply and think, blast loops, `resolve_projectiles`). Deterministic today; a hecs bump or a component that moves an entity between archetypes reorders every pinned stream | read |
| X-14 | The terrain snapshot is a frame old when shots resolve: a tile a laser killed earlier in the frame still eats a shell | read; rare |
| X-15 | Two fixed-step accumulators are stacked (`StepClock` and `physics_accumulator`); with the exact dt every driver passes, the inner one is always one step | read |
| X-16 | No spatial index: `Terrain::build`, every sweep, `ram_props`, `nav_tiles_unchanged` and `refresh_edge_masks` are linear in tile count each frame. Fine at the largest shipped map (3,495 cells); the format allows 62,500 | read; cost unmeasured |
| X-17 | A prop's ram timer runs on commanded velocity with no direction test: grazing along a sandbag wears it, a skidding hull never pops a barrel | read |
| X-18 | `docs/physics-engine-design.md` describes a removed model (no acceleration, sensors, `spawn_shell`, ASYNCIFY); `Tank::control`'s doc says "no momentum"; `place_tank`, a gameplay function, lives in `debug.rs` | read |

### Effects and feel

| Id | Finding | Mark |
|---|---|---|
| E-1 | Grenade blasts, air strikes, aircraft crashes, bong deaths, rams and well collapses have no particle arm. A grenade in water makes no splash; a bomb makes no screen flash | read |
| E-2 | Shock strength is half constants and half tunables with no common scale: a grenade (1.0) shakes like a tank kill and 3.3 times a missile; a tower death has none of its own; the gauss rail has none | read |
| E-3 | No feedback scaled to the player being hurt: no shake on a non-fatal hit, no edge pulse, no HUD tick. Hit-stop exists only on the round-deciding blow | read |
| E-4 | The hit flash is a self-additive redraw (a dark chassis barely flashes), drawn over tree canopies; bongs get none and spark like steel | read |
| E-5 | The kill's impact ripple quad re-blits the raw scene over the kill's own shockwave through the unshaken camera, so the bend pops in after 0.14 s | read; look unverified |
| E-6 | Smoke and chips are drawn in the glowing pass, after the light multiply, so night never darkens them; aftermath columns are in the lit pass and disagree | read |
| E-7 | Ground fire is never y-sorted (a tank north of a fire draws over flames in front of it) and gets one glow per burning cell, against the "one light per fire" rule | read |
| E-8 | Effects-language breaks: ripples are per pixel, not per 2 px block; scorches draw at non-integer texel scales; scorch, decal and screen-flash fades are linear alpha; laser red exists as four different values; a particle's ramp is chosen by comparing its tint's RGB | read |
| E-9 | Reduced motion does not cover the whole-screen flash, lightning, the fuse strobe or the ripples, and only iOS and the web can turn it on at all | read |
| E-10 | At the particle cap `push` drains oldest-first across all kinds, so a flamethrower plus a missile volley wipes hull damage smoke, which is a gameplay tell | read |
| E-11 | One hit is presented by four systems on two clocks (`Game::impact_flashes`, `Fx::impacts`, `Fx::flashes`, particles), with day and night lights listed separately. A new weapon needs five or six edits | read |
| E-12 | There is no audio (BB-102). The hook exists: `fx`, `follow`, `awareness` and `voice` already observe `game.events()` after every step | read |

### Rules of the game

| Id | Finding | Mark |
|---|---|---|
| M-1 | Tank damage has no common funnel. Only `apply_hit` and spall emit `Event::Hit`, credit a kill and alert the AI; blasts, lava, ground fire, slime, tesla death and afterburn do none of it. The rod writes `damage = MAX_DAMAGE` directly. `last_hit_by` never expires | read |
| M-2 | Friendly fire has five rules (see decision 2 for the one that replaces them) | read |
| M-3 | A seat always collects, so driving over any weapon crate destroys the special it carries | read |
| M-4 | Twelve chassis play as seven. Health, speed, regen and magazine are identical; only damage factor, mass, hull box and barrel count vary, and twin barrels double burst. Scout equals wraith; warden, breaker, longbow and glacier are their twin partner with half the burst; the final level's leviathan has about half the per-pull damage of level 15's titan | read |
| M-5 | Laser and missile crates are downgrades for a seat (seat/enemy damage is split only for shells and plasma) and upgrades for an enemy | read (arithmetic) |
| M-6 | The six specials are finished on enemies with AI rules for all six and almost never seen: `enemy_special_weapon_*_share` is 0 | read |
| M-7 | An unpinned player chassis is a random roll with a 4x firepower spread (20 of 51 maps pin none; all campaign maps do) | read |
| M-8 | Docs drift, about fifteen claims. The ones that matter: CLAUDE.md's "`damage_obstacle` the one place a tile loses health" (five sites zero it directly); `tank.rs` "the one way damage lands on a hull"; `docs/defence-towers-prd.md` "Nothing is implemented"; `docs/enemy-command-and-control-prd.md` "Not yet implemented"; `docs/levels.md` says 14 levels; `docs/two-players.md` says a wrecked seat stays wrecked | read |
| M-9 | Finished and off by default: breakable crates, oil prints, grass trampling, `ember_bed_relight`, `fire_reveals`, `smoke_cover`. No tuning row is unread | read |

### Enemy AI

| Id | Finding | Mark |
|---|---|---|
| A-1 | Wander re-rolls its waypoint every 3 s whether or not it arrived, each roll picking the candidate farthest from every other tank: idle tanks zig-zag in a box | read; traced |
| A-2 | Routes staircase on diagonals in arenas: heading straightening applies on field maps only, and arenas tie-break on cell index. This eats the jitter budget so it cannot catch real flip-flopping | read; traced |
| A-3 | Flee has no exit and never shoots: at 70 damage a tank backs away for the rest of the round unless it finds a health crate | read |
| A-4 | The yield brake releases after 0.5 s and the tank then drives into whatever is ahead: 81 enemy-pair rams across the fixtures | measured |
| A-5 | No target leading: aim is the target's current position on a cardinal axis, held 0.25 s. In the probe's `circle` scenario half the rounds were unfinished after 30 s | read; link estimate |
| A-6 | Every conventional weapon is fired by one generic rule: one range, one interval, cardinal alignment required for homing missiles too | read |
| A-7 | The friendly-fire hold is a 60% coin flip on a 24 px line against tank centres; splash is not considered | read |
| A-8 | Refuge scoring has no cover term; crate seeking ignores reachability and pays two A* searches a frame | read |
| A-9 | Difficulty is counts and chassis tier only; aim settle, fire interval and flee threshold are global, so level 1 and the last level have identical reflexes | read |
| A-10 | Enemy C2 (`c2_enabled`, off) is about one third built: five of eight orders have no producer, and the PRD's own sweep showed more rams into the player with it on | read |
| A-11 | Per-frame cost drivers, none a problem at measured sizes: two whole-grid clones and a compare, flow fields rebuilt from zero, uncached A* per private target, the behaviour tree rebuilt (about 25 `Vec`s) per tank per think | read |

### The follow camera

Measured on 2026-11-19 at `e3211dd` for BB-165: dev-server lockstep on
`longwater`, seed `0xB0B5`, a 1280 x 720 window (a view of 32 x 18 cells, the
sight box leaving 144 px of room sideways and 48 up and down), one step a
frame, the seat's position and the view's centre read after each
(`camera-preview/tools/record.py` on the BB-165 branch). Speeds are world
pixels a frame at 60 Hz; the tank's top speed is 3.5. The camera already had
a dead zone, a look-ahead, an exact critically damped spring with velocity
feed-forward, an eased field edge, cuts and a Voronoi couch split; what was
weak is how the look-ahead moved the view.

| Id | Finding | Mark |
|---|---|---|
| V-1 | **The view keeps moving after the tank stops.** The look-ahead was 35% of its room at rest and all of it at top speed, so a stop from speed sent the view on 9 px, then slid it back 94 px over 2.3 s. Counted over the drive, 92 px of view motion on frames where the tank stood | measured |
| V-2 | **The view outruns the tank on every start.** The same ramp ran the view at 5.6 px a frame beside a tank at 3.5 for about a second; the picture moved 1.28 px for each pixel the tank drove | measured |
| V-3 | **A reversal whips.** The lead flipped after a 0.25 s hold and crossed its whole room in 0.5 s: 260 px of view travel peaking at 9.2 px a frame, 2.6 times the tank. Three 0.4 s dodges back and forth threw the view 150 px each way | measured |
| V-4 | **Every quarter turn slides the view sideways**, about 105 px, and a turn on the spot to line up a shot swings it 46 px over most of a second; on a staircase route the view's sideways speed runs between +5.6 and -2.0 px a frame. A four-way tank turns constantly, so the view is never still | measured |
| V-5 | **The look-ahead is whatever room the screen has**, with no limit of its own: 131 x 35 px on this window, 259 x 91 on a monitor's 40 x 21.5 view, about 185 x 8 on a phone. The same turn moves the picture seven times further on one screen than another, and four to twenty times further sideways than up and down | measured here; the other screens are arithmetic from `Framing::room_outside` |
| V-6 | Settling takes 1.3 to 2 s: a 0.5 s linear ease into a 0.5 s spring, the last 14 px at under a pixel a frame | measured |
| V-7 | Firing does nothing to the view. The only response to the player's own gun is the hull's physical recoil, which the dead zone absorbs | read |
| V-8 | Nothing in the fight frames the view: not the enemies closing, not the bong under attack. The arrows and the minimap carry all of it | read |
| V-9 | A couch pair's shared view and both halves of its split have no look-ahead at all (`Aim::facing` is `None` there) | read |
| V-10 | On a phone the view cannot put the tank anywhere but the middle of the height: the sight box leaves 0.65 cell (21 px) of room up and down, so no bias away from the thumbs and no vertical look-ahead fits. The room is a consequence of the sight box, which the AI's fire gate reads | arithmetic from section 4 of docs/large-maps-follow-camera.md |
| V-11 | There is no way to see the camera's reasoning: no overlay draws the dead zone, the sight box, the lead or the goal | read |

## Friendly fire under decision 2

Decision 2 is one sentence; this is what it does to each source, read from
the code on 2026-11-19, and what has to be built round it so the rule is
fair to play.

### What each source does today, and what changes

| Source | Own tanks today | Own bong today | Under the rule |
|---|---|---|---|
| Shell, bullet, plasma, laser | full (enemy on enemy), x `friendly_fire_damage_factor` 1.0 (seat on seat); shooter skipped | hurt | unchanged |
| Gauss rail | teammate x factor, fellow enemy full | hurt, 20 x what the slug kept | unchanged |
| Flamethrower | other seat x factor; ground fire hurts the shooter too | hurt, 10 dps | unchanged |
| Rod from god | kills everyone, the caller included | killed | unchanged - it is the precedent for the rule |
| Barrel, fuel drum, lava bomb, ground fire | everyone | both bongs | unchanged |
| Missile, grenade, FPV burst, air-strike bomb (`side_blast`) | **spared**, owner included; shoved only | **spared** | **hurt**: owner, own side, own bong |
| Gravity well collapse | flung, **not hurt** | hops, **not hurt** | **hurt** (10 x falloff) |
| Sonic hammer | shoved, **not hurt** | stunned, no damage to either bong | own tanks **hurt** (4 x falloff); bongs stay stunned only, since the hammer damages no bong |
| Tank wreck blast | chips only the side **opposing** the victim | **no bong**, by a comment that calls it a balance change | chips **both sides and both bongs** |
| EMP | disables everyone | no effect | unchanged |
| Ram | enemy-enemy and seat-seat already damage | - | unchanged |
| Gun tower | never targets its own side | **hits its own bong** (F-8) | P0-9 stands: a tower not shooting at its own bong is aim, not a damage exemption |

The open point from the first draft - does a blast hurt the caster's own
bong - is answered **yes**. Shells, the rail, the flame, the rod and every
barrel already do; the three blasts that spare it are the inconsistency, not
the rule.

### What the rule breaks unless these are built with it

These are part of the work, not follow-ups. Without them the rule makes the
game unfair rather than consistent.

| Id | Issue | Evidence | What to build |
|---|---|---|---|
| FF-1 | **A grenade can end a Protect round in one press.** A grenade does 30-50 with linear falloff over 128 px; a bong has 40 health and starts 90-240 px from its seat. 13 of the 23 maps with a mission are Protect | `tuning.rs` (`bong_max_health`, `bong_spawn_*_dist`, the grenade rows) | A blast radius the player can read before it goes off (a 2 px block ring on the last second of the fuse, in the grenade's ramp), and a look at the numbers: either the bong's health or the grenade's damage to a bong (`flame_bong_damage_per_second` and `gauss_bong_damage` are the precedent for a per-weapon bong row) |
| FF-2 | **Air support bombs what stands beside the bong.** A jet's bombs do 75-85 at 64 px and it picks targets in the caller's sight box; enemies converge on the bong | `simulation/air_support.rs` | The aircraft holds a bomb whose circle reaches the caller's own bong or a seat, as a tower holds fire with a friend in line. Missiles from the helicopter the same |
| FF-3 | **Enemies would kill themselves and each other with splash.** The AI's friendly-fire hold tests a 24 px line against tank centres and fires anyway 40% of the time; no blast weapon has a minimum range | `ai.rs` `friendly_blocks_shot`, `enemy_friendly_fire_hold_chance` | P5-12, as a hard dependency: per-weapon minimum safe distance from the weapon's own blast, the hold widened by splash radius, and the enemies' own bong counted for every weapon (today `ShotLine.friend` covers it only for the specials) |
| FF-4 | **A wreck blast now chips the bong, and enemies die beside it.** A bong bites adjacent enemies, and enemies close on it; every wreck within 110 px would chip it 3-8 with falloff | `combat.rs` `apply_explosion` and its comment | Measure before deciding the number: Protect outcomes on the probe (`defend`, `advance`, and P1-4's surviving scenario) with and without the chip. If Protect stops being winnable, change `bong_max_health` or the wreck blast's row - a number, not an exemption |
| FF-5 | **Self-damage has no telegraph today because it never happened.** A missile fired into a wall at point-blank, a grenade that bounces back, an FPV dive beside the pilot | `grenade.rs` (bounce), `missiles.rs` | The radius ring of FF-1 for every blast the player owns; a missile's and a drone's blast only armed after a short travel distance is a possible softening, to be decided on a play-test, not assumed |
| FF-6 | **Kill credit and the end screen.** `last_hit_by` never expires and blasts credit nobody (M-1); a seat wrecking its partner or its own bong must never score | `combat.rs` | In P5-5's `hurt_tank`: no credit for an own-side kill; the end screen names what was lost to friendly fire |
| FF-7 | **The bong dodges shells, not blasts.** A shell that hurts a bong triggers its evasive hop; no blast does | `combat.rs` `apply_hit` (`bong_hop`) | A hop away from the centre of any blast that hurts it, with the jitter draw kept in the same place in the frame's order |
| FF-8 | **Tests and docs pin the sparing.** `tower_tests::a_splash_spares_its_own_side`, the `side_blast` cases in `missiles.rs`, `fpv_tests`, `air_support_tests`, `well_tests`, `sonic_tests`; `docs/grenade-launcher.md`, `fpv-swarm.md`, `air-support.md`, `gravity-well.md`, `sonic-hammer.md` | - | Rewritten with the change, each to assert the new rule; `docs/levels.md`'s crate placement rules gain one for blast weapons on Protect maps (today grenade crates sit on `armory` and `oasis-bazaar`; air crates only on `no-mans-land`) |
| FF-9 | **Co-op rooms.** Out of scope for this plan, but the rule is the room's too: eight seats with grenades is a different game, and the difficulty patch scales by seat count | `server/src/room.rs` | Flagged for the co-op work; the rule itself is one code path |

Not checked: how `credit` is computed for seat-on-seat kills today (the
function was not found under that name), and whether the grenade already
draws any radius tell.

## The plan

Sizes: **S** under a day, **M** a few days, **L** a week or more.
**Re-baseline** means the item moves seeded replays, and so
`determinism_tests::the_one_and_two_seat_streams_are_pinned` and the probe
ceilings must be re-recorded on purpose, never to go green.

The ordering principle: fix what can crash, then build the instruments, then
do presentation work (which moves no replay), then land everything that
moves replays in as few batches as possible.

### Phase 0 - defects

Each fix comes with the test the finding lacked. They ship in stages 1, 2
and 8 (section "Stages").

| Id | Item | Finding | Size | Re-baseline |
|---|---|---|---|---|
| P0-1 | `seats_on_field()` in `side_blast_sparing` and `guide_missiles`, plus a `seat_tests` case with a blast in the gate lane | F-1 | S | only rounds with a seat rolling in |
| P0-2 | Iterate `lava.deep_grid_cells()` directly; a probe run twice on a lava map to confirm | F-2 | S | check the pin on a lava map |
| P0-3 | Clear `pulled` against the live well list after an EMP collapse | F-3 | S | EMP + well + bong rounds only |
| P0-4 | One tolerant roll helper for every damage range; clamp the count and variance rows | F-4 | S | none at defaults |
| P0-5 | `fx_dt = 0` when the round is not playing or is paused | F-5 | S | none |
| P0-6 | Evict accumulator keys not touched this frame instead of clearing; emit only for sources in view | F-6 | S | none |
| P0-7 | Re-baseline `hedge-maze` on purpose (trace the two extra jitters first) and run `probe-fields` in CI | measured | S | that is the task |
| P0-8 | Fix the five clippy errors (`TAU`, `modulo_one`) | measured | S | check: `TAU` must round the same as the literal |
| P0-9 | Player gun tower spares its own bong | F-8 | S | tower fixtures |
| P0-10 | Campaign count, last-level won state, Boot Camp's "2 / 0" | F-14 | S | none |

### Phase 1 - instruments and gates

Nothing in phases 2 and 3 can be ranked or proven without this.

| Id | Item | Size |
|---|---|---|
| P1-1 | Counters in `GpuCanvas` and the `Rl` wrapper: blits, rects, texture switches, blend, shader and target changes; shown in the dev label and `status.frame_stages` | S-M |
| P1-2 | Max and p99 frame time and a steps-per-frame histogram in `frame_stages` | S |
| P1-3 | A repeatable perf scene: a dev-server script (`restart {seed}` + `step`, `fx_max_particles` pinned) and a headless `CpuCanvas` paint benchmark | S-M |
| P1-4 | A probe scenario in which the seat survives, so a round lasts longer than five seconds of AI | M |
| P1-5 | Split the probe's jitter into "turns with net progress" (a staircase) and "turns with none" (a flip-flop); stop counting a flee-heal-return trip as churn; add a counter for enemy-on-enemy hits | M |
| P1-6 | Tighten the ceilings that sit unused at 0 (clustering, pile-up, stall, wall-grind) | S |
| P1-7 | CI: clippy on the headless lib, the `dev-tools` tests, `cargo test -p bongbong-server`, `tests/waves_sanity.rs` | S |
| P1-8 | A dense 250x250 probe fixture and a wasm timing of it, to decide whether X-16 and A-11 are real work | S |

### Track D - four-direction, made great

Decision 1 makes this a track of its own, running through phases 2, 4 and 5.
The audit found that what makes four-direction movement look and feel rough
is mostly not the movement model: it is how the model is presented, how the
hull fits the grid when it turns, and how enemies route on it.

| Id | Item | Finding | Size | Re-baseline |
|---|---|---|---|---|
| D-1 | **Render interpolation.** Previous pose (position, hull and turret visual rotation) kept per tank and projectile as presentation-only fields; `alpha = owed / PHYSICS_FIXED_DT` passed to `Game::render` and the follow camera; projectiles extrapolate. Thumbnails and the dev server's lockstep draw at alpha 1 so pinned hashes and `step` + `screenshot` stay exact. The fields never feed RNG, events or the wire | R-1, R-4 | M-L | none |
| D-2 | Frame-time snapping to the refresh period, and vsync on desktop | R-2, R-3 | S | none |
| D-3 | Whole-texel sprite anchors with the camera's residual derived from the followed tank, so the tank and the ground never disagree by a texel | R-7 | S-M | none |
| D-4 | **The turn.** Measure it first (dev server: obelisk flush beside an iron row, turn, read `snapshot` contacts). Then either a fit test before the collider swap with a nudge along the free axis, or a per-chassis footprint that turning does not change | X-7 | M | yes |
| D-5 | **Turn presentation.** Decide between rotation snapped to a small set of pre-rotated frames and the current eased rotation; a turn should read as one crisp motion with no pixel crawl. Needs a side-by-side preview before the choice | R-9 | S-M | none |
| D-6 | **Corner and lane feel.** Not audited - an investigation item: how a hull behaves when a turn is commanded a few pixels before or after a corridor mouth on the 32 px grid (`TANK_MOVE_CORNER_RADIUS`, the 0.9x movement collider). Classic four-direction tank games buffer the turn and slide the hull onto the lane. Measure how often a commanded turn is refused, then propose turn buffering and lane assist as tunables | - | M | yes if built |
| D-7 | **Weight.** Drive force by chassis and a brake that scales with mass, so heavy shoves light and a parked hull holds its ground | X-1 | M | yes |
| D-8 | Wrecks that resist like the dead metal they are: damping scaled by mass, or fixed once settled with a stated shoulder-aside rule | X-2 | S-M | yes |
| D-9 | **Enemies that drive straight.** Prefer the current heading among equal-cost steps in arenas, or turn once at the far corner | A-2 | M | yes; no RNG shift |
| D-10 | Wander resamples on arrival, not on a 3 s timer, and prefers waypoints ahead | A-1 | S | yes; RNG shifts |
| D-11 | **Enemies that lead on the axis.** Anticipate the seat's crossing of a cardinal axis from its velocity and the shell's speed, pre-face and fire on entry, scaled by tier | A-5 | M | yes |
| D-12 | Yield that re-routes instead of pushing through after 0.5 s | A-4 | M | yes |

D-1 to D-3 and D-5 move no replay and land in phase 2. D-4 and D-6 to D-12
move replays and land in the phase 5 batch.

### Track C - the follow camera

BB-165. The craft the findings were held against is in
`camera-preview/RESEARCH.md` on the issue's branch (Keren's "Scroll Back",
Eiserloh's "Juicing Your Cameras With Math", Cinemachine's position composer,
Nijman's "The Art of Screenshake", Holden's spring roll-call, Xbox
accessibility guideline 117). Three rules came out of it and out of the
measurements: **the view moves when the player's hands do and stops when they
stop**; a camera distance is a number of its own, not whatever a screen has
room for; and anything a shot or a blow does to the picture is a layer over
the follow, never part of it.

| Id | Item | Finding | Size | Re-baseline |
|---|---|---|---|---|
| C-1 | The look-ahead is the same at any speed (`camera_lead_at_rest` 1.0): nothing runs ahead on a start, nothing drifts back after a stop | V-1, V-2 | S | none |
| C-2 | A limit of its own in world pixels (`camera_lead_max_px`), under the room the sight box leaves; 0 by default after play, so the shipped camera has no look-ahead | V-5 | S | none |
| C-3 | The look-ahead turns to a new heading only once the tank has driven along it (`camera_lead_commit_px`, 40 px), so a turn on the spot and a short dodge move nothing; the reversal hold stays on top | V-3, V-4 | S | none |
| C-4 | The swing is paced by the tank: one second from side to side at top speed (`camera_lead_ease_seconds`), and none of it while the tank stands (`camera_lead_idle_pace` 0) | V-3, V-4, V-6 | S | none |
| C-5 | A shot can kick the picture back against the gun (0 px by default after play) and returns in a quarter of a second (`camera_fire_kick_px`, `camera_fire_kick_seconds`), a layer over the spring, off under reduced motion | V-7 | S | none |
| C-6 | **Settling.** With C-1 to C-4 the spring is left to do two things: take up the dead zone on a start and carry a swing. Measure its tail again, then decide between a shorter `camera_spring_seconds` and a rest rule (snap once the distance and the speed are both under a quarter of a device pixel) | V-6 | S | none |
| C-7 | **Framing the fight.** A capped, feathered pull toward the weighted middle of the enemies near the seat and toward the bong while it is being bitten, inside the same room as the look-ahead and smoothed slower than it. It moves the view for something the player did not do, which is the thing C-1 to C-4 removed, so it needs a side-by-side on a real fight before a default is chosen, and it starts at strength 0 | V-8 | M | none |
| C-8 | Look-ahead for a couch pair: each half along its own seat's heading, the shared view along the pair's common heading when they have one | V-9 | S-M | none |
| C-9 | **Room up and down on a phone.** Either the sight box loses a row on screens that cannot show it with room to spare, or the phone's view gains one. Both change what the AI's fire gate reads (`ai::in_sight_box`) or what a room's seats see, so this is a simulation item with a proposal first. Only then can the tank sit above the thumbs or the view lead up and down | V-10 | M | yes; `coop` label |
| C-10 | A dev overlay that draws the dead zone, the sight box, the lead, the committed heading and the spring's goal over the picture, and the same numbers in `status.camera` | V-11 | S | none |
| C-11 | The blow that hurts a seat shoves its view away from where it came from, through the kick's layer, instead of shaking it on a hash of its position | E-3 | S | none |

C-1 to C-5 are built on the BB-165 branch. Every one is presentation only:
nothing under `src/simulation/` reads the camera, and an arena's picture does
not change by a byte.

### Phase 2 - presentation correctness

No replay moves. Every item is checked on a dev-server screenshot.

| Id | Item | Finding | Size |
|---|---|---|---|
| P2-1 | D-1, D-2, D-3, D-5 | | |
| P2-2 | Opaque blits scene -> composite -> window (or a separate-alpha blend that keeps destination alpha at 1), then an art pass over every translucent effect, which all get brighter. Remove the three workarounds | R-5 | S + art |
| P2-3 | Integer pre-scale present: nearest to the next whole multiple, bilinear down to the window. No derivatives extension, so it runs on GL ES 2 | R-6 | S-M |
| P2-4 | Clamp wrap on every render target; overscan equal to the shake cap | R-8 | S |
| P2-5 | Soft failure with a visible message for a missing texture or a failed target; embed the ripple shaders like the rest; allocate targets in size buckets during a resize | R-17 | S |
| P2-6 | Add `dissolve.fs` to the twin test and compare normalised bodies, not only uniform lists | - | S |

### Phase 3 - render pipeline

Order these by what P1-1 and P1-3 show. All presentation only.

| Id | Item | Finding | Size |
|---|---|---|---|
| P3-1 | Pack the sheets into one or two atlases with a white texel and `SetShapesTexture`; `Sheet` stays the lookup so `CpuCanvas` is unchanged | R-11 | M |
| P3-2 | Collect shapes and glows into two frame-level lists per stage and draw each once | R-12 | M |
| P3-3 | Bind `tuning()` once a frame and thread it through the painters; scratch `Vec`s kept across frames; HUD strings cached on their inputs | R-13 | S-M |
| P3-4 | A render-side cell index for obstacles; `partition_point` on the sorted grass; scorches and decals bucketed by cell | R-14 | M |
| P3-5 | Margins keyed on (ground stamp, frame, water frame, sky); the end screen drawn once; colour-only framebuffers | R-16 | S-M |
| P3-6 | Particle ring buffer with a per-kind budget that reserves damage smoke; non-dithered puffs baked into the atlas | E-10, R-15 | M |
| P3-7 | The page writes its values on change instead of being evaluated every frame | R-18 | S |
| P3-8 | `App` struct with `frame_input` / `frame_sim` / `frame_camera` / `frame_draw`; a `DevHooks` type that is a no-op without the feature; one `weather::run_stages` | R-19 | M + M |

### Phase 4 - effects and feel

Any shock goes through a `*_show` so replicas match. Any shader change is
made in both `static/` and `static/web/`.

| Id | Item | Finding | Size |
|---|---|---|---|
| P4-1 | Ram feedback: contact sparks keyed by owner slot, a `Ram` arm with a burst scaled by impulse, a hull flash and a small shock | F-13 | S |
| P4-2 | One `Fx::explosion(at, size, kind)` called from every blast event; one `shock` tuning group with a row per cause and a stated scale | E-1, E-2 | S |
| P4-3 | Being hit: a small shock on the hurt seat's own view scaled by damage, an edge pulse in ramp steps, a HUD tick; a 2-3 frame hit-stop on a local seat's kill (offline only) | E-3 | M |
| P4-4 | A solid-white silhouette flash drawn at the tank's place in the y-sorted walk; `Flashed::Bong`; a bong's own hit particles | E-4 | M |
| P4-5 | Fold muzzle and impact rings into the pass-2 shader on the shaken camera, quantised to 2 px blocks | E-5, E-8 | M |
| P4-6 | Smoke, trail, dust and chips at the end of the lit pass; sparks and embers stay in the glowing pass | E-6 | M |
| P4-7 | Burning cells in the y-sorted walk; one glow per connected group of fire cells | E-7 | M |
| P4-8 | Conformance: scorch scales snapped to whole texels, fades stepped with `pyro::alpha`, tints as a ramp enum on `Particle`, one `LIGHT` table, one laser red | E-8 | M |
| P4-9 | Reduced motion caps the screen flash and steps the strobes; a player-facing setting on every platform | E-9 | S-M |

### Phase 5 - the consistency batch

Everything here moves replays. It lands as a small number of PRs behind one
conscious re-baseline of the pinned streams, `probe-fixtures` and
`probe-fields`, after phase 1 has made the probe worth re-baselining against.
Each PR's body says what was re-recorded and why.

**5a. Order and structure first** (so later changes do not move the stream twice)

| Id | Item | Finding | Size |
|---|---|---|---|
| P5-1 | Sort every RNG-drawing walk by slot, cell or shot id | X-13 | M |
| P5-2 | Delete `physics_accumulator`; `debug_assert!(dt == PHYSICS_FIXED_DT)` | X-15 | S (byte-identical) |
| P5-3 | Refit the neighbours' colliders when a tile dies | F-7 | S-M |
| P5-4 | Move `place_tank` out of `debug.rs`; rewrite `docs/physics-engine-design.md` to the model that exists | X-18 | S |

**5b. One of each**

| Id | Item | Finding | Size |
|---|---|---|---|
| P5-5 | `hurt_tank(entity, amount, cause, by)`: every damage source emits `Hit`, credits and alerts through it; the rod included. A new `HitCause` needs its `WireEvent` mirror and a `PROTOCOL_VERSION` bump | M-1 | M |
| P5-6 | `Knock { speed, mass_exponent, skid }`: every shove through one helper with one mass rule, blasts given a short skid so the tuned rows show | X-3, X-4 | M |
| P5-7 | One blast function over `BlastParams`: reach to the nearest hull point, occlusion by permanent tiles through `Terrain::line_of_fire` (the per-target RNG draw is kept and only the effect is zeroed), explicit flags for bongs, lanterns and grenades | X-5, X-6 | M |
| P5-8 | **Friendly fire per decision 2**, as the section "Friendly fire under decision 2" lays out: `side_blast` stops sparing its owner, its side and its bong; the well and the hammer hurt their own tanks; wreck blasts chip both sides and both bongs. Built together with FF-1 to FF-8 - the radius tell, the aircraft's hold, the bong's hop from a blast, credit, the rewritten tests and docs - and after P5-12 (FF-3), or enemies kill themselves with their own splash. FF-4's measurement comes first | M-2 | L |
| P5-9 | `TerrainBox::entry(p0, p1, pad, mode)` with modes Shot, Solid and Sound, used by the sweep, the rail, the flame cap and the grenade. The flame is capped per lateral ray and stops at the first hull | X-8, X-11, F-9 | M |
| P5-10 | `Tank::target_shape()` with `reach_from`, `in_cone` and `swept`, replacing the five geometries | X-9 | M |
| P5-11 | A pass-over chance for every tile the AI sees over, and a stated rule for lasers and rails | X-10 | S-M |

**5c. Movement and AI** (Track D's replay-moving items, plus)

| Id | Item | Finding | Size |
|---|---|---|---|
| P5-12 | Per-weapon fire profiles: range, interval, needs-alignment, minimum safe distance from its own blast; the friendly-fire hold widened by hull radius and splash | A-6, A-7 | M |
| P5-13 | D-4, D-6 to D-12 | | |
| P5-14 | Flee with an exit: fire from the refuge when aligned, a cornered rule, a time box; a cover term in refuge scoring | A-3, A-8 | S-M |
| P5-15 | Crate seeking filtered by `grid.connected`, the second A* dropped | A-8 | S |
| P5-16 | Bong hops and pulls clear of hulls | X-12 | S |
| P5-17 | A prop's ram timer on body velocity toward the tile | X-17 | S |

**5d. Round flow**

| Id | Item | Finding | Size |
|---|---|---|---|
| P5-18 | A straggler rule for the last wave and for band rounds; a band fallback when every lane is blocked; a wrecked seat returns on the final wave | F-10, F-11, F-12 | M |
| P5-19 | Level unlock only when `played_as_authored` | F-15 | S |

### Phase 6 - design work

These change how the game plays rather than fixing it, and each needs a
proposal Oto approves before code.

| Id | Item | Finding | Size |
|---|---|---|---|
| P6-1 | Chassis identity: a compensating stat for every single-barrel chassis (rate, armour, speed or range), the final level's tank the strongest, the lib.rs role comments made true | M-4 | M |
| P6-2 | Crates: a seat keeps a stocked special unless it chooses the swap; laser and missiles made worth a seat's while | M-3, M-5 | S-M |
| P6-3 | A per-level share of enemies carrying each special | M-6 | S |
| P6-4 | A skill profile per level or tier over the existing aim, interval and flee rows | A-9 | M |
| P6-5 | Per-map keys for breakable crates, oil prints and grass trampling, so finished features ship without moving every stream | M-9 | S |
| P6-6 | Enemy C2: finish phase 3 and switch the arbiter on, or delete it | A-10 | L |

### Larger bets

| Id | Item | Notes | Size |
|---|---|---|---|
| B-1 | Sound (BB-102) | A `Sound::observe_events(&Game)` beside `fx.observe_events`; WebAudio through the `window.bb*` bridge on the web would avoid `-sASYNCIFY`. Whether raylib's audio truly needs ASYNCIFY was not verified | L |
| B-2 | One cue table | The `*_show` functions produce `Cue { kind, at, dir, size }`; one table maps a kind to particles, burst, ripple, shock, flash and light. Removes E-11 and makes B-1 a column in the table | L |
| B-3 | An always-on additive light buffer | One half-resolution target and one disc quad per light, replacing the per-glow rectangle fans and the duplicate day and night light lists. A clear sky must look unchanged | L |
| B-4 | Merged wall colliders | Greedy rectangles or `oriented_polyline` outlines, rebuilt per run in `obstacle_died`. Removes internal seams and P5-3's refit; every contact changes | L |
| B-5 | A persistent cell-indexed tile grid on `Game` with DDA sweeps and a dirty-marked nav grid | Only if P1-8 shows a cost | M-L |

### Dependencies

Nothing needs a migration. A sweep of the 44 compatible updates is one small
PR. `sola-raylib` 6.4.0 may unblock BB-139 (raylib 6.2); its changes were not
read. A `rapier2d` bump beyond 0.36.x belongs inside the phase 5 batch, since
any solver change moves every contact.

## Stages

The work is cut into stages, **one PR and one Linear issue each**. A stage is
something that can be reviewed, play-tested on its web preview and merged on
its own, and master is releasable after every one. The phases above say what
kind of work an item is; the stages say what ships together.

Three rules shaped the cut:

- A stage that moves seeded replays re-records the pinned streams and the
  probe ceilings **once**, at its end, and its PR body says what was
  re-recorded and why. Stages that move replays are never interleaved with
  each other in one PR.
- A stage is either presentation or simulation, not both, so a pixel
  comparison or a replay comparison can prove it.
- None of these stages touches co-op, so none carries the `coop` label and
  none deploys a room server (BB-154). Two exceptions are marked: they bump
  `PROTOCOL_VERSION` and are labelled `coop` so the room and the client are
  tried together.

| Stage | What ships | Items | Size | Replays |
|---|---|---|---|---|
| 0 | **Pipeline.** A PR deploys a room server only with the `coop` label; without it the web preview is built with co-op disconnected | BB-154 | S | none |
| 1 | **Defects.** The gate-lane panic, the lava collider order, the bong pinned after an EMP, tuning ranges that panic, a tower shooting its own bong, the campaign counts | P0-1, P0-2, P0-3, P0-4, P0-9, P0-10 | M | small: only rounds that hit the fixed paths |
| 2 | **Gates.** `probe-fields` green and in CI, clippy clean and in CI, the tests CI skips, a probe scenario where the seat survives, jitter and churn that mean what they say, ceilings tightened | P0-7, P0-8, P1-4, P1-5, P1-6, P1-7 | M | ceilings re-recorded against the better probe; no stream moves |
| 3 | **Instruments.** Draw-call and switch counters, frame-time percentiles, the repeatable perf scene and paint benchmark, a dense 250x250 fixture with a wasm timing | P1-1, P1-2, P1-3, P1-8 | M | none |
| 4 | **Smooth motion.** Render interpolation, frame-time snapping, vsync on desktop, the followed tank and the ground on the same texel | D-1, D-2, D-3 | L | none |
| 5 | **True pixels.** Opaque target blits and the art pass they need, integer pre-scale, clamped targets and shake overscan, soft asset failures, the crisp turn | P2-2, P2-3, P2-4, P2-5, P2-6, D-5 | M-L | none |
| 6 | **Fewer draw calls.** One or two atlases, the glow pass in two lists, tuning bound once a frame, cached margins and end screen, the page's values on change | P3-1, P3-2, P3-3, P3-5, P3-7 | L | none |
| 7 | **Scale and structure.** The render-side cell index, the particle ring buffer with its budget, `App` and `DevHooks`, one weather stage runner | P3-4, P3-6, P3-8 | L | none |
| 8 | **Feedback.** Particles frozen with the round, the emitter limiter, ram feedback, one explosion helper and one shock table, being hit | P0-5, P0-6, P4-1, P4-2, P4-3 | M-L | none (shocks go through `*_show`) |
| 9 | **Effects in their place.** The silhouette hit flash, ripples folded into the pass-2 shader, smoke in the lit pass, y-sorted fire with one glow, conformance, reduced motion | P4-4, P4-5, P4-6, P4-7, P4-8, P4-9 | L | none |
| 10 | **Order.** Every RNG-drawing walk sorted, the second accumulator gone, breach colliders refitted, `place_tank` moved, the physics doc rewritten | P5-1, P5-2, P5-3, P5-4 | M | **first full re-baseline** |
| 11 | **One of each.** One damage funnel, one knock, one blast with cover, one "is it solid", one target shape, pass-over for what the AI sees over | P5-5, P5-6, P5-7, P5-9, P5-10, P5-11 | L | full re-baseline; `PROTOCOL_VERSION` bump, `coop` label |
| 12 | **The drive.** The turn fits, turn buffering and lane assist, weight, wrecks that resist, bongs clear of hulls, the prop ram on body velocity | D-4, D-6, D-7, D-8, P5-16, P5-17 | L | full re-baseline |
| 13 | **Enemies.** Straight routes, wander on arrival, leading on the axis, a yield that re-routes, per-weapon fire profiles with safe distances, flee with an exit and cover, reachable crates | D-9, D-10, D-11, D-12, P5-12, P5-14, P5-15 | L | full re-baseline |
| 14 | **Friendly fire.** Every source hurts everyone, with the radius tell, the aircraft's hold, the bong's hop from a blast, credit, the rewritten tests and docs | P5-8 with FF-1 to FF-8 | L | full re-baseline; `coop` label (the room plays the same rule) |
| 15 | **Round flow.** Stragglers, blocked lanes, the final-wave return, honest level locks | P5-18, P5-19 | M | field-map streams and `probe-fields` |
| 16 | **Camera.** A look-ahead that is the same at any speed, has a limit of its own, turns only once the tank has driven the new way and swings only while it drives; a shot's kick on the picture. Then the settle, the fight's pull, the couch's look-ahead, the overlay and a directional blow | C-1, C-2, C-3, C-4, C-5 (built, BB-165); C-6, C-7, C-8, C-10, C-11 | M | none |

### Why this order

- **0 to 3 make the rest safe**: nothing unneeded deploys, the known crash is
  gone, the gates watch more than five seconds of AI, and the renderer can be
  measured.
- **4 to 9 are everything a player sees** and move no replay, so they can be
  compared frame for frame against master and shipped in any release.
  Stage 4 is the largest single improvement to four-direction movement and
  comes first among them.
- **10 to 15 are everything a replay sees**, in dependency order: sorted
  walks before anything else moves the stream (10), the shared funnels
  before the rules built on them (11), the drive before the AI that drives
  on it (12, 13), the AI's safe distances before friendly fire (14).
- **16 is presentation and can ship at any point after 4.** It stands on
  stage 4's smooth motion (the view rides the tank's drawn pose and its texel
  lock, BB-158) and on stage 8's feedback (a hurt seat's own shake and
  `screen_fx_intensity`, BB-160), both merged. It shares two files with
  stage 5 - `view.rs` and `render/` - and edits neither, so the two merge in
  either order. `C-11` waits for stage 9's reduced-motion setting only for
  its switch on every platform. `C-9` is not in the stage: it changes what
  the fire gate reads, so it belongs with the replay-moving stages (13 is
  the nearest) after a proposal.

### What is honest about the sizes

Stages 4, 6, 7, 9 and 11 to 14 are each a week or more, and their PRs will be
large. That is deliberate for 10 to 14, where splitting further would mean
re-recording the streams twice for one design. If a presentation stage grows
past what one review can hold, it splits along its items (6 into atlases and
the rest; 9 into flash-and-ripples and layering-and-conformance) rather than
shipping half an item.

### Not staged

Phase 6 (chassis identity, crates, specials on enemies, skill profiles,
per-map keys, C2) and the larger bets (`B-1` to `B-5`) each start as a
proposal Oto approves; they become stages then. `M-8`'s docs drift is fixed
by the stage that touches each module, and a dependency sweep is a small PR
of its own at any point before stage 10.

## Existing issues this plan touches

BB-102 (sound, `B-1`), BB-139 (raylib 6.2), BB-56 (spawn-swap share knobs),
BB-55 (sonic-carrying enemy CPU, `A-11`), BB-49 and BB-50 (special-weapon AI,
`P5-12`), BB-99 and BB-100 (field-map AI, `P5-18`), BB-128 to BB-130
(free-steer, not part of this plan).
