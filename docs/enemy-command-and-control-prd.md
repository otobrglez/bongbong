# PRD: enemy command & control (C2)

Status: agreed with the author 2026-09-14 (the interview answers are in section
2). Not yet implemented; section 11 is the phase order. Builds on the
engagement-slot ring (`simulation/engage.rs`), which is the only existing code
that reasons about enemies *as a set*, and copies its shape.

Contents

1. Why
2. The interview: what was decided
3. The switch
4. Architecture
5. The collision arbiter
6. Ordered rams
7. Pickup assignment
8. Coordinated attack
9. Determinism
10. Measurement
11. Phases
12. Non-goals

## 1. Why

Enemies have no shared awareness of each other. An enemy's only knowledge of its
peers is `movers: &[Mover]` - `{ position, velocity, radius, is_player }`, with
no identity at all - plus one supervisor-supplied point, `engage_target`. No
enemy publishes its committed heading, its dodge, its waypoint or its yield state
to any other. There is no arbiter, no claim system and no message bus.

Three independent per-tank mechanisms each solve a slice of "don't hit each
other" and leave a hole between them:

- `Ai::avoid_collisions` predicts closest approach and sidesteps - but
  **explicitly ignores any pair already closer than
  `radius_a + radius_b + avoid_margin`**: *"Already overlapping is the ram
  system's job, not ours."* That ignored regime is exactly where collisions
  happen.
- `crowded_ahead` + `Ai::yield_timer` brakes a tank with something directly in
  front - but it is per-tank and blind. Both tanks in a head-on brake, neither
  knows the other is braking, and only the `enemy_yield_seconds` ceiling breaks
  the deadlock.
- `EngageRing` spreads *destinations* over 16 slots but allocates no routes and
  no right of way. Contention en route is unmanaged.

Two `is_multiple_of(2)` index-parity hacks exist solely so two tanks do not
mirror into each other - the symptom of having no arbiter. Parity is a
2-colouring, not an order: three tanks in one jam get two answers and two of them
still mirror. And the index is the *mover* index, an archetype-layout artefact, so
a wave tank arriving or a wreck despawning can flip every parity mid-jam.

### The brake has stopped working, and the codebase already knows

`ai::separation_tests` was re-measured when the nav grid's cell size dropped to
the map grid's. Over 16 seeds x 1200 frames, `enemy_separation_px` 0 (brake off)
vs 12 (on):

```text
                    rams into player   enemy-vs-enemy rams
  48px grid  off           12                  79
  48px grid  on             6                  46
  32px grid  off           27                  37
  32px grid  on            18                  60
```

Read the bottom two rows. **Under the current nav grid, turning the brake on
makes enemy-vs-enemy ramming worse - 37 to 60.** The test's own comment says why:
*"braked tanks hold station in contact rather than shoving past."*

That is the case for C2 in one line. Braking is the wrong mitigation for a tank
that is *already touching*: it freezes the jam instead of clearing it, and
because the brake sets `move_dir = None`, the stuck detector - which only counts
tanks *commanded* to move - resets, so nothing underneath notices.

And the brake cannot be tuned out of it. Sweeping `enemy_separation_px`
12 -> 20 -> 28 -> 36 -> 48 -> 64 over 16 seeds moved rams-into-player
21 / 24 / 21 / 24 / 18 / 23, inside the seed-to-seed spread. The knob is dead as
a lever. Whatever fixes this is a new mechanism, not a new default: *a brake
alone converts a collision into a standoff, it does not resolve who goes first.*

The canonical failure has a documented specimen
(docs/gameplay-verification-design.md, 2026-09-04): a three-tank jam in open
ground, where *"the rounded tank colliders slid the whole jam east at up to
100px/s until the wall stopped it - and because every tank in it was moving,
`Ai`'s stuck escape never fired, while predictive avoidance skips tanks already
in contact."* Nothing in the game can see that, because seeing it requires
looking at several tanks at once.

## 2. The interview: what was decided

| Question | Decision |
|---|---|
| Scope | Enemy-vs-enemy **and** enemy-vs-player collisions. |
| Authority | **A commander that can reassign goals**, not merely an intent filter. |
| Levers | Throttle, brake, path nudge, right-of-way priority, and pickup assignment. |
| Pickups | **Assign and deconflict only.** One claimant per pickup, gated on real need. **Denial was asked for and then withdrawn** (2026-09-14): see section 7. |
| Aggression | **Protective + coordinated attack.** C2 may order a ram, a focus-fire or a pincer. |
| Player rams | **Prevent accidental, allow ordered.** |
| Consolidation | Layer on top, but **C2 suppresses the personal-space brake per pair** where it has taken charge. Revised mid-interview, once the brake was measured as counterproductive. |
| `ai.rs` surface | One additive parameter, phase 4 only. Everything else needs zero `ai.rs` change. |

"A commander that can reassign goals" and "leave `ai.rs` alone" cannot both be
literal, and the resolution is a split by kind of order. **Movement orders**
(throttle, brake, nudge) are applied to the collected intents after `think` has
returned - zero `ai.rs` change. **Goal orders** (fetch, focus, ram, hold) reach
the tree through one parameter that absorbs the existing `engage_target`, the
precedent already in place for exactly this. `ai.rs` keeps every mechanism it
has; it gains one input.

## 3. The switch

C2 must be turnable on and off **in a live round**, so the author can judge
whether the gameplay is actually better. This is a requirement, not a debug
affordance, and it shapes the design: every pass is written so "off" is a *total*
no-op - the commander is not consulted, no intent is rewritten, no ram is denied,
no RNG is drawn - rather than a strength of zero that still runs the machinery.

Four surfaces, one source of truth:

1. **`c2_enabled: bool` in `tuning.rs`** is the source of truth. Bools are
   already supported (`impl Knob for bool`), and tuning writes are staged and
   applied at the frame boundary, so flipping mid-round is safe by construction.
2. **A key toggle in dev builds** - the thing that answers "is this better?".
   Press it mid-fight and the same round continues with C2 off; press again and
   it returns. Follows the `F11` precedent, guarded by `!crate::EMBEDDED`. The
   dev overlay label, which already prints `DEV overlays: <preset>`, gains a
   `C2 on`/`C2 off` readout so there is never doubt which mode is live.
3. **`--no-c2` on both binaries**, mirroring `--no-dev-server`. The probe takes
   it too, which makes the A/B sweep a one-word change rather than a rebuild.
4. **Everything the tuning system already reaches, for free**: the web dev
   panel's toggle, `--tuning` with live mtime reload on native, and
   `mcp__bongbong__tuning_set {"c2_enabled": false}`.

The switch is also the measurement instrument: with C2 off the build must
reproduce a pre-C2 build's numbers **exactly**, not approximately.

## 4. Architecture

Two modules, split on the extensibility axis the ask names.

**`simulation/comms.rs`** - the vocabulary, pure data. `Signal` is a *fact*
broadcast to the pack (spotted, took fire, claimed, contact); `Order` is a
*directive* addressed to one tank (throttle, hold, nudge, fetch, focus, ram,
yield, fall back); `Blackboard` carries the frame's signals and the standing
orders. Adding "reserve this corridor" later is one `Signal` variant and one
`Order` variant. Adding "focus player 2" is an order that already exists.

**`simulation/command.rs`** - the `Commander`, shaped exactly like `engage.rs`,
and that shape is the point: no `World`, `Physics`, `Frame`, `Tank` or `Ai`;
world access as `&dyn Fn` closures on a context struct; pre-sorted input slices;
state with memory in one small field and documented hysteresis; and a
`Serialize` report that records **why the alternatives were passed over**, the
way `Rejections` does, so a pile-up is diagnosable from one frame of JSON.

Passes are explicit methods in a fixed order - `deconflict`, `assign_pickups`,
`coordinate` - not a plugin trait. A trait-object pipeline would buy nothing and
would make the determinism argument harder to state.

### Where it runs, and the trap that decides whether the A/B works

`Game::enemy_phase` ends with one loop that, per enemy, calls `Ai::think`, diffs
`AiSnapshot` into events, then `drive_tank`, `tick_queued_shots`,
`dispatch_fire`. C2 needs every intent before any tank moves, so the loop splits.

**The trap: firing draws RNG.** `fire_shell`, `fire_plasma` and `fire_bullet`
all pull from the round stream for shadow offsets and minigun spread. Today the
draws interleave per tank - `think(e1), fire(e1), think(e2), fire(e2)`. A naive
split batches them - `think(e1), think(e2), fire(e1), fire(e2)` - and that
reorders the stream **no matter what the switch says**, destroying the one A/B
the whole measurement plan rests on.

So the split is not think-all / drive-all. It is:

- **Collect pass** - everything up to and including `think`, the event diff,
  `tank.control(...)` to set the aim, `tick_queued_shots` and `dispatch_fire`.
  Firing stays here, so the interleave is untouched. Aiming stays here too, or
  shots would fly along last frame's facing. The pass also captures the two
  values `drive_tank` reads before it writes - the body velocity and the hull
  facing - because by the apply pass both have been disturbed.
- **Command pass** - the `Commander`, no world, no RNG.
- **Apply pass** - the impulse only, via a `drive_tank_with(.., current,
  facing_before)` factoring, walking the collect pass's captured order.

Replaying the captured order matters independently: `motion_snapshot`'s own doc
comment records that *"a later query over the same archetype has no guaranteed
iteration order"*, so the apply pass must not re-run the query, and must not
sort either, since today's order is the raw ECS order.

## 5. The collision arbiter

**Detection.** C2 sees last frame's positions - `drive_tank` applies an impulse
in `enemy_phase`, but bodies do not move until `step_world` and `Tank::position`
is not refreshed until `sync_tanks_and_ram`. Conflict is either a predicted hull
contact within `c2_horizon_seconds`, or a pair already inside
`c2_contact_margin_px`. The second clause is the point: it is precisely the
regime `avoid_collisions` refuses, so the two never fight. The horizon is shorter
than `avoid_lookahead` and the margin tighter than `avoid_margin`, so the bands
are disjoint by construction.

One further separation matters: `Mover::velocity` is the *commanded* cardinal, so
`avoid_collisions` predicts entirely in commanded space and is structurally blind
to knockback, blast shove, grind residue and jam drift. C2 predicts in **real**
velocity. That is exactly the population the existing layer cannot see - including
the 2026-09-04 specimen, where every tank commanded north while the pile slid
east.

O(n^2) over at most 31 enemies and 2 players is a few hundred pairs of a few
flops; `ram_enemy_pairs` already does the same walk with a narrow-phase query per
surviving pair. Plain O(n^2) with a distance cull is fine.

**Right of way** is a total order, so the loser is never ambiguous, and bucketed
so a flip needs a real difference rather than float noise: mission rank (an
ordered attack outranks a firing-line slot, which outranks transit), then speed,
then closeness to objective, then owner slot. Owner slot is the final tie-break
and the reason it is total; it is also the key every other deterministic ordering
in the codebase already sorts by, and unlike the mover index it is never reused
within a round, so it cannot flip mid-jam when a wave tank arrives.

**Mitigation ladder**, applied to the yielder only - a symmetric intervention is
how the deadlock is re-created:

1. **Throttle** - scale commanded speed. The tank keeps its heading and keeps
   closing, just slower.
2. **Nudge** - one cardinal off the committed heading, only when a side is free
   of both terrain and other movers.
3. **Brake** - the existing behaviour, now the last rung rather than the first.

**Throttle first is the design's best idea, and it is not cosmetic.** The brake
sets `move_dir = None`, which resets the stuck detector, which is why
`enemy_yield_seconds` is load-bearing against permanent deadlock. A throttled
tank is still *commanded* to move, so the stuck clock keeps running and the
existing escape hatch stays armed underneath C2. That removes the deadlock class
rather than time-boxing it.

The nudge must go through `Ai::commit` rather than rewriting `move_dir` directly.
Committing the nudge restarts the direction-hold clock on the nudge heading, so
the tree's own hysteresis carries it - which is why the nudge duration is matched
to `ai_dir_hold_seconds`, and why the nudge is free hysteresis rather than a
fight. The terrain half of the free-side test reuses the frame's existing
`Grid::blocked_ahead` through a closure, so C2, `steer_toward` and
`avoid_collisions` cannot disagree about what is passable.

**Suppressing the existing brake, per pair.** The brake stays in `ai.rs` and
keeps working everywhere C2 is not looking, but where C2 has taken charge it
tells that tank not to brake - otherwise C2's throttle and the tree's brake
fight, and the tree wins by setting `move_dir = None`, which is the behaviour
measured at 60. Preferred implementation is a post-hoc undo of a braked intent,
which keeps the phase's "zero `ai.rs` change" property; a one-line suppression
flag is the fallback if that measures worse.

## 6. Ordered rams

`combat::ram` fires on any narrow-phase touch. Guaranteeing contact never happens
is not achievable - knockback, blasts, a hopping frog, wall funnels and the
player driving in all produce contact C2 has no lever over - so the mechanism is
an authorisation flag `ram` consults, checked **after** the wreck and cooldown
guards and **before** the RNG draw. That placement is load-bearing: an
unauthorised touch consumes no RNG, which is what lets the gate-off setting
reproduce today's stream exactly.

Player-initiated rams always resolve; denying them would silently remove melee
from the player's kit. An enemy-player contact is authorised when the *player*
was the one closing, measured from the real closing components `ram` already
computes.

## 7. Pickups: enemies take only what they need

**The governing rule, and it is not negotiable for tuning:**

> An enemy collects a pickup only if collecting it would actually do it some
> good. A player always collects.

A pack that hoovers up every crate it drives past - at full health, with a full
magazine, already stocked with the weapon - strips the field of everything the
player was going to use and gains nothing itself. It is not a difficulty knob,
because it does not make the enemies better at anything; it only makes the round
more annoying. The asymmetry with the player is the point: choosing to take
something you do not strictly need - denying it to the other side, topping up
before a push, grabbing a shield you will want in ten seconds - is a decision the
person at the controls is entitled to make, and an AI helping itself to the same
latitude reads as spite rather than as intelligence.

**This was already wrong before C2, and has been fixed at the source.** The
behaviour tree's `seek_*` tiers were need-gated, but *collection* was pure
proximity (`Game::pickup_phase`), so an undamaged enemy driving over a health
pack ate it. Collection now gates on `Tank::wants_pickup`, and the five seek
tiers call that same predicate rather than repeating their conditions.

**Sharing one predicate is a correctness requirement, not tidiness.** If
collection were ever stricter than seeking, a tank would drive to a pickup it
then refused to take, and sit on top of it forever.

**Measured, gate off vs on, same seed, everything else held** (default map, afk,
4 enemies, 1800 frames, 30 rounds, `--seed 1000`):

| | gate off | gate on |
|---|---|---|
| enemy-pair rams | 175 | **144** |
| rams into the player | 2 | **0** |
| ram damage | 748 | **575** |
| clustering | 14 | **11** |
| jitter / spin | 14 / 10 | 17 / 7 |
| rounds flagged | 20/30 | 23/30 |

The mechanism is not subtle: an enemy that no longer detours to, and lingers on,
a crate it has no use for spends less time converging on the same few spots. So
politeness is also, incidentally, a fifth of the ram problem. Fixture budgets all
still pass unchanged.

The two numbers that went the wrong way are small and honest: jitter +3 and three
more rounds flagged. Both are well inside the standing ceilings and neither is
explained away here - if they grow when C2's arbiter lands, this is where to look
first.

What C2 adds on top is the thing a single tank cannot know: **deconfliction** -
one claimant per pickup. Today three enemies with no laser charges all drive to
the same laser, because `Brain::nearest_pickup` is per-tank nearest with no
mutual exclusion. Two of those trips are wasted whatever the need gate says.

**Denial is explicitly not built.** It was chosen in the interview and withdrawn
by the author the same day, on the grounds above. Nothing in the vocabulary
prevents it being added later - it would be a need bonus on a contested pickup -
but it is out of scope, and the `c2_deny_*` knobs the earlier draft specified are
not implemented.

Three composition rules keep the rest from fighting the tree: C2 never lowers a
tank's priority, so a tank in a fight is not pulled out of it; where the tree
already chose to seek that kind, C2 only redirects which instance; and a fresh
order goes through `Ai::steer`, so it inherits commitment, the obstacle override
and the stuck escape.

## 8. Coordinated attack

Minimum viable: an **ordered ram** (which is not optional once accidental rams
are gated off - something must turn melee back on, or the group silently gets
tougher by no longer chipping itself), and **focus fire** as a veto on the
opportunistic retarget pass plus a rank bump. A **pincer** is a one-line
candidate-ordering bias in `EngageRing::assign`, since the ring already owns the
geometry.

Deferred: formations, synchronised timing, bait and flank roles, dynamic role
reassignment, and anything requiring C2 to do its own pathfinding.

## 9. Determinism

**C2 draws no RNG. Ever.** Enforced structurally - the commander is never handed
`&mut SmallRng` - rather than by test, the way `engage.rs` enforces "no world
access". Every tie is broken by owner slot; every map is keyed by owner slot, so
the "never iterate a `HashMap` where the body breaks ties" rule is satisfied by
construction rather than by discipline.

What that buys: with the switch off the round is byte-identical to a pre-C2 build
at the same seed, so any later movement in `just probe-fixtures` is attributable
to behaviour and never to stream drift - which is exactly the distinction
`determinism_tests` cannot make, since it compares two runs of one build.

## 10. Measurement

**There is no existing metric that counts enemy-tank-vs-enemy-tank contact.** The
probe's `bump-rate` counts `touching_static` only, and the probe never reads
`Game::events()` at all, so `Event::Ram` has zero consumers outside one unit
test. `ContactStats` already computes `touching_tank`, parked in the design doc
as *"ready for future ram/pile-up metrics"*. This is that future.

**Two caveats decide how the headline claim may be worded.**

*Clustering has largely been spent.* Default-map clustering went 78 -> 7 from a
nav-grid change and 14 -> 5 from a frog hop fix, neither for reasons involving
tanks avoiding each other - an immobile frog was not registering as terrain
grinding, it was registering as a pile-up. C2 must not claim that ground.

*Ram counts saturate.* `combat::ram`'s cooldown is set on **both** participants
and the guard tests both, so one ram anywhere suppresses every ram involving
either tank for half a second: in a three-tank jam, A-B also suppresses A-C and
B-C. Ram counts measure distinct impacts and compress as density rises,
flattering any change that turns one big pile-up into several small ones. So
beating the un-braked 37 is **necessary but not sufficient**, and part of the
37 -> 60 rise may be re-triggering across the cooldown rather than more
collisions - the direction of that inversion is trustworthy, the magnitude is
not.

**The companion metric that does not saturate is contact-frames** - tank-frames
with `touching_tank` set, which grows with both duration and participant count
where a ram count grows with neither. That is the number a three-tank jam
actually shows up in, and it is why Phase 1 is instrumentation-first.

New instrumentation, all before any behaviour change: `touching_tank` into
`TankSnapshot`, `TrackRow` and `TankDebug`; the probe's first `Event::Ram` tally
split into enemy-pair and into-player; contact-frames per round; and two new
anomaly kinds - `tank-grind` (the `wall-grind` analogue against another tank) and
`pile-up` (three hulls actually touching, at a radius below `CLUSTER_RADIUS_PX`
so the two kinds cannot be the same reading counted twice).

Re-baselining policy is the justfile's, unchanged: *never bump a ceiling just to
go green* - and the converse, that a ceiling which genuinely drops should be
lowered, or the gate stops gating.

## 10a. Phase 2 as built, and what it measured

The plumbing landed in four steps, each verified byte-identical over all seven
fixtures plus the default map at `--seed 1000` before the next began: the
throttle field, the `drive_tank_with` factoring, the collect/command/apply split,
and the substrate. **All four passed**, which is what proves the split did not
reorder the RNG stream - the one failure that would have invalidated every
measurement below.

Then the arbiter, measured three ways on the same seeds:

| | enemy-pair rams | into player | contact | clustering | zero-ceiling kinds |
|---|---|---|---|---|---|
| **default map**, C2 off | 144 | 0 | 116.6 s | 11 | clean |
| throttle + brake | **123** | 3 | **106.4 s** | 9 | stall 1, low-progress 2, never-arrived 1 |
| throttle only | 152 | 4 | 132.9 s | 15 | clean |
| **bare map**, C2 off | 21 | 7 | - | 0 | clean |
| bare map, C2 on | **6** | 5 | - | 0 | clean |

Three findings, two of them against this document's own predictions:

1. **In open ground the arbiter is a large win** - 21 enemy-vs-enemy rams down to
   6 over 16 seeds, comfortably past the un-braked 37 bar section 10 set. That is
   the regime it was designed for: pairs genuinely converging on predictable
   headings.
2. **Throttle alone makes ramming worse**, 144 → 152 on the cluttered map, and
   contact-seconds confirm why: 116.6 → 132.9. Section 5 called throttle "the
   design's best idea"; it is not, on its own. Easing a tank off keeps it *in*
   the contested space longer, and ram damage re-triggers every
   `ram_damage_cooldown`, so a slower tank in contact registers more rams, not
   fewer. The brake - the rung this document argued against - is what actually
   shortens contact.
3. **The arbiter increases rams into the player**, 3 → 6 on `separation_tests`'
   four seeds. Not a bug and not fixable here: enemies that stop shoving each
   other simply arrive more successfully. The mechanism that addresses player
   rams is Phase 3's authorisation gate, which stops the *damage* rather than the
   approach, and it is meaningless to judge this number before that exists.

**So `c2_enabled` ships `false`.** The fixture gate passes either way and the
shipped default is byte-identical to pre-C2, so nothing regresses; the arbiter is
real, switchable and measured, and flipping the default is Phase 3's decision
once the ram gate can carry the player half. Re-baselining `separation_tests`'
ceiling to make C2-on green was available and deliberately not taken - the
justfile's policy is *never bump a ceiling just to go green*, and that applies
hardest when the ceiling is catching something true.

## 11. Phases

Each phase is independently shippable, independently provable, and one knob away
from off.

0. **Record the baseline** from the tree as it stands, naming that tree.
1. **Instrumentation only.** No behaviour change, so every existing baseline must
   be unmoved - which is itself the test that the instrumentation is inert.
2. **Substrate + collision arbiter**, in four separately verified steps, each
   byte-identical before the next: the throttle field; the `drive_tank`
   factoring; the loop split with a commander that issues nothing; the substrate
   and the report. Only then the arbiter, where the numbers legitimately move.
3. **Ordered rams.**
4. **Pickup deconfliction** - one claimant per pickup. The only `ai.rs` touch.
   (The need gate itself already landed, outside C2, in `Tank::wants_pickup`.)
5. **Coordinated attack.**

## 12. Non-goals

- C2 never writes `Ai` fields. It speaks `Intent` and orders, never memory -
  forcing a committed heading or zeroing a dodge timer would bypass the very
  hysteresis that keeps jitter at 6 instead of 30.
- C2 does not co-own `EngageRing::choice`. It overrides the ring's *result*, not
  its state.
- Enemy-vs-enemy ram damage is not deconflicted away. It is the scoreboard.
- No message queue with delivery latency. Nothing here needs a message that
  outlives a frame except a claim, and a claim is state, not a message.
- No spatial hash. n is at most 33.

## Appendix: the Phase 0 baseline

Recorded 2026-09-14 against **the working tree at commit `f0cab22` plus
uncommitted changes** (`git diff | shasum -a 256` = `71895001f7f39407`). That
tree contains two other sessions' in-flight work - a nav-grid change
(`PATHFIND_CELL_SIZE` 48 -> 32) and a frog change (side-gated bites, fixed
evasive hop) - neither of which is on master. **If either is reverted or
reworked, every number below is void and Phase 0 must be re-run.**

`probe --frames 1800 --rounds 10 --seed 1000` per fixture:

| fixture | border-stuck | jitter | spin | churn | clustering |
|---|---|---|---|---|---|
| choke | 0 | 0 | 0 | 3 | 0 |
| frog-block | 1 | 5 | 0 | 0 | 0 |
| maze | 0 | 3 | 1 | 8 | 4 |
| pockets | 0 | 1 | 0 | 1 | 0 |
| props | 0 | 4 | 0 | 4 | 0 |
| tight-corridors | 0 | 2 | 0 | 4 | 1 |
| u-trap | 1 | 3 | 1 | 2 | 2 |
| **corpus total** | **2** | **18** | **2** | **22** | **7** |

`stale-start`, `stall`, `wall-grind`, `bump-rate`, `low-progress`,
`never-arrived` and `invariant` are **0 on every fixture**.

Default map, `--scenario afk --mission protect --spawn band --enemies 4
--frames 1800 --rounds 30 --seed 1000`: 20/30 rounds flagged, jitter 15,
spin 10, churn 33, clustering 14, never-arrived 1, everything else 0.

`ai::separation_tests` at this tree: 3 rams into the player and 22
enemy-vs-enemy over seeds 77..80, against ceilings of 5 and 30.

**Acceptance bar for the arbiter**, restated against these numbers: enemy-vs-enemy
rams must fall **below the un-braked 37** measured over 16 seeds, not merely
below the braked 60; `pile-up` and `tank-grind`, once they exist, must reach 0
across the corpus; and `jitter`/`spin` must stay flat, because C2 adds no heading
change except a nudge committed for exactly the hold the jitter detector is
calibrated against. A rise in `jitter` is the specific diagnostic that the nudge
is not being committed through `Ai::commit`.
