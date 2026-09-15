# PRD: the frog health pack

Status: agreed with the author 2026-09-14 (the interview answers are in section
2), implemented on the same branch. Section 10 lists where the build departs
from the text below.

Contents

1. Why
2. The interview: what was decided
3. The rule
4. The heal
5. Where packs come from
6. Determinism
7. Presentation
8. Knobs
9. Tests
10. Non-goals and as built

## 1. Why

The frog is a pure fail-state. `Frog::damage` (`src/frog.rs`) is the only
mutator of `Frog::health`; there is no heal, no regen and no knob for one. Four
independent sources chip at it - shells, bullets, plasma and the laser through
`Game::apply_hit` (`simulation/combat.rs`), the flame cone
(`simulation/flame.rs`), barrel blasts (`simulation/props.rs`) - and the
player's frog reaching zero is an instant `Outcome::Lost` in both Protect and
Hunt (`Game::check_round_end`).

With `frog_max_health` at 40 against a tank's 100, two stray hits early in a
round put the player on a clock they cannot stop. Every other loss condition in
this game can be fought: a hurt tank finds a health pack, a flanked tank drives
away, a breached wall can be rebuilt around. The frog alone only ever gets
worse. The frog health pack is the repair - the first thing the player can *do*
about the objective they are told to protect.

## 2. The interview: what was decided

| Question | Decision |
|---|---|
| How is it delivered? | **Instant heal on pickup.** Drive over it anywhere on the field and the frog heals where it stands. No carry state, no escort trip. |
| How much does it heal? | **Full heal.** One pack restores the frog to `max_health`. |
| Who can collect it? | **Each side heals its own frog.** A player heals the player's frog; an enemy in a Hunt round heals the enemy frog. |
| What if the frog is already at full health? | **Not collectible.** The tank drives over it and the pack stays where it is - a full heal is too valuable to throw away by accident. |
| What about a side with no frog at all? | **Collected and wasted.** An enemy in a Protect round has nothing to heal and consumes the pack anyway: denial pressure, grab it before they blunder into it. |
| Where do packs come from? | **Map slots, plus a bonus drop beside a Health slot while the frog is hurt.** The pack appears when it matters and still has to be fetched. |
| What does the icon look like? | **A health pack with a frog glyph**, green - derived from `health.png` the way `gen_shield_pickup.py` derives the shield. |

## 3. The rule

The three collection cases above are one sentence, and that sentence is what the
code implements and what `PickupKind::FrogHealth`'s doc comment says:

> A tank collects a frog pack **unless its own side's frog is alive and already
> at full health.**

Reading it back out:

| The collector's own frog | Outcome |
|---|---|
| alive, below `max_health` | collected, healed to full |
| alive, at `max_health` | **not collected** - the pack stays on the field |
| absent (enemy in Protect, anyone in Destroy) | collected, wasted |
| dead | collected, wasted - a dead frog is deliberately not "full" (`Frog::heal` is a no-op; the round has already ended anyway) |

"Its own side's frog" is `Game::frog` for a player and `Game::enemy_frog` for an
enemy, picked by `Game::is_player`. Nothing about this rule cares which *player*
collects it in a two-player round - both players share one frog.

## 4. The heal

`Frog::heal(&mut self, amount: f32)` sits directly beside `Frog::damage` and
mirrors its shape: a no-op once dead, otherwise `health` raised and clamped at
`max_health`. A frog pack cannot revive a dead frog, and there is no case where
it would matter - the player's frog dying ends the round on the same frame.

The amount is `max_health * frog_pack_heal_fraction`, which defaults to 1.0: the
agreed full heal, with a knob for QA to weaken it without a rebuild.

No animation state is added. The frog's ground-ring gauge (`draw_frog_ring`)
refills on its own from `health_fraction`, and the filmstrip set
(docs/FROG_SPEC.md) has no "healed" clip to play.

The heal happens in `pickup_phase`, but it **cannot** happen inside the per-kind
effect match: that block holds a live `&mut Tank` borrow from `query_one`. The
frog is healed after the borrow closes, next to the `Event::PickupCollected`
push.

## 5. Where packs come from

**Map slots**, like every other pickup: a `{ kind = "pickup", pickup =
"frog_health" }` cell, paintable in the builder, respawning on the normal
`pickup_respawn_seconds` timer through `respawn_from_slots`.

**Plus a bonus drop beside a Health slot.** The rainbow shield already works
this way (`maybe_spawn_bonus_shield`): each time a Health slot is spawned or
respawned, a roll drops an un-slotted shield in a free neighbouring cell. The
frog pack reuses that path rather than copying it - `maybe_spawn_bonus_shield`
and `bonus_shield_cell` are generalised to take the kind they are placing and
looking for - with one extra gate: **the roll only happens while the frog is
alive and below `frog_pack_bonus_below` of its max health.**

On a Health slot the two bonuses roll shield first, frog pack second, with
`occupied` recomputed between them so they cannot land on the same neighbour.

Being un-slotted, a bonus pack does not count toward `slot_backed_count`, so it
can never make the field look full to the respawn timer - the same reason the
bonus shield does not.

## 6. Determinism

Round randomness is one seeded `SmallRng` stream, and the draw *order* is part
of a replay (docs/gameplay-verification-design.md). The bonus roll is therefore
gated on the frog being hurt **before** any `rng.random_range` call, never after.
That buys three properties:

- `Game::init`'s spawn pass never rolls a frog pack: the frog is pristine at
  init, so **round setup draws exactly the RNG it drew before this feature**.
- A round in which the frog is never hurt is byte-identical to the same seed
  before this change.
- A round in which the frog *is* hurt shifts the stream from that Health-slot
  respawn onward. This is unavoidable if the feature is to exist at all.

`just probe-fixtures` is consequently a required step, and any ceiling it
exceeds must be shown to be the stream shift rather than a behaviour change -
re-run with `frog_pack_near_health_chance = 0` through `--tuning` to confirm -
before re-baselining per that recipe's own policy.

## 7. Presentation

**The icon** is `static/pickups/frog_health.png`, generated by
`tools/gen_frog_health_pickup.py` in the raw-PNG-bytes convention every other
generated pickup icon uses (no Pillow). Like `gen_shield_pickup.py` it reads
`static/pickups/health.png` rather than drawing from scratch: the box's
saturated red is hue-shifted to green with its saturation and value preserved,
so the outline, the highlights and every bit of the pack's shading survive
untouched; the white cross is then replaced by a small frog silhouette. Reading
"a health pack, but for the frog" at 32 px is the whole job. Deliberately not
palette-snapped - pickups are loud on purpose (`static/pickups/SOURCE.md`).

**The collect cue** is a small green burst at the frog, driven out of
`Event::FrogHealed` by `Fx::observe`'s one-shot half. `fx.rs` is presentation
only and may use `rand::rng()` freely; nothing about the burst can reach a
simulation value.

**No HUD change.** The bar already carries a `FROG` gauge of the frog's health
fraction, which is exactly the feedback a heal needs - it refills. The pack is
not a weapon and takes no `WEAPON_SLOTS` slot.

**No change to `draw_pickup`**: the sprite draws flat like every other pickup,
no glow and no bob.

## 8. Knobs

Three rows in `group pickups`, beside `shield_near_health_chance`:

| Knob | Default | Range | What it does |
|---|---|---|---|
| `frog_pack_heal_fraction` | 1.0 | 0.0 ..= 1.0 | Fraction of `frog_max_health` one pack restores. 1.0 is the agreed full heal. |
| `frog_pack_near_health_chance` | 0.35 | 0.0 ..= 1.0 | Odds of dropping a bonus frog pack beside a Health slot when it spawns, *while the frog is hurt*. 0 disables the bonus entirely. |
| `frog_pack_bonus_below` | 0.75 | 0.0 ..= 1.0 | The "frog is hurt" threshold: the bonus only rolls below this fraction of the frog's max health. |

## 9. Tests

Headless `mechanics_tests` on tiny inline maps, one per promised rule:

1. A player collecting a pack restores a damaged frog to `max_health`, and
   emits `PickupCollected` + `FrogHealed`.
2. A player whose frog is at full health drives over the pack and it is still
   there - no `PickupCollected`.
3. A hurt frog is healed even when the collecting tank is itself at full health:
   the pack is not a tank heal.
4. Protect: an enemy driving over a pack consumes it and no frog is healed.
5. Hunt: an enemy collecting heals the *enemy* frog and leaves the player's
   alone.
6. A dead frog is not revived.
7. The bonus drop lands on a free neighbouring cell when the frog is hurt, and
   with a pristine frog never rolls - asserted against a same-seed control round
   so "draws no RNG" is checked, not just "spawns nothing".

Plus the counts a new tool moves: the editor's `TOOLS` length and the dev
server's PICKUP category count.

## 10. Non-goals and as built

Non-goals:

- **No AI seek tier.** Enemies collect a pack by driving over it, never by
  wanting it - no `act_seek_*` leaf and no behaviour-tree registration. Same
  stance the flamethrower took. Making enemies fetch packs for their frog in
  Hunt is a real idea, and a separate one.
- **No carry or charge state**, no partial heal, no auto-apply-later, no HUD
  slot.
- **No revive.**
- **Not authored into Destroy maps** - there is no frog there, so a pack would
  be inert flavour.

As built, the deltas from the text above:

- **`Frog::at_full_health`** was added beside `heal`, so the collect rule
  lives in one predicate rather than being spelled out at the call site. It
  is `at_full_health`, not `needs_healing`, on purpose: a dead frog is not
  full, so a pack driven over after one dies is consumed like any other
  side with nothing to heal, exactly as the rule sentence reads.
- **The heal reports its own position.** `Event::FrogHealed { side, slot,
  amount, x, y }` carries the *frog's* coordinates, not the pickup's: the
  frog is generally nowhere near the pack, and `fx.rs` needs to put the
  burst on the thing that got healed.
- **`maybe_spawn_bonus_shield` became two functions**, not one:
  `maybe_spawn_health_slot_bonuses` (what a Health slot rolls - the shield,
  then the frog pack behind its gate) and `maybe_spawn_bonus` (one kind,
  one chance), with `bonus_shield_cell` renamed `bonus_pickup_cell`. The
  shield's own behaviour is unchanged, and its two tests pass untouched.
- **The determinism gate is tested directly**, not only by its effect:
  `a_pristine_frogs_health_slot_draws_no_more_rng_than_the_shield_roll_alone`
  runs `maybe_spawn_health_slot_bonuses` with `frog_hurt: false` and the
  shield roll alone from the same seed, and asserts the next draw off each
  RNG matches. That is the property section 6 claims, stated as a test.
- **`just probe-fixtures` needed no re-baselining.** None of the seven
  `maps/test/` fixtures has a Health slot, so `maybe_spawn_health_slot_bonuses`
  is never reached in any of them and their RNG streams are byte-identical
  by construction, hurt frog or not.
- **The bonus gate reads the player's frog only.** A Hunt round's enemy
  frog never summons a pack; it is welcome to whatever the player leaves
  on the ground. The bonus exists to give the *player* a way back.
- Editor spellings: tool name `frog_health`, dropdown label `frog pack`,
  cursor/bar short label `frog+`. `TOOLS` is 29, the dev server's PICKUP
  category 9.
