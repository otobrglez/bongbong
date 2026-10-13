# PRD: the bong health pack

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

The bong is a pure fail-state. `Bong::damage` (`src/bong.rs`) is the only
mutator of `Bong::health`; there is no heal, no regen and no knob for one. Four
independent sources chip at it - shells, bullets, plasma and the laser through
`Game::apply_hit` (`simulation/combat.rs`), the flame cone
(`simulation/flame.rs`), barrel blasts (`simulation/props.rs`) - and the
player's bong reaching zero is an instant `Outcome::Lost` in both Protect and
Hunt (`Game::check_round_end`).

With `bong_max_health` at 40 against a tank's 100, two stray hits early in a
round put the player on a clock they cannot stop. Every other loss condition in
this game can be fought: a hurt tank finds a health pack, a flanked tank drives
away, a breached wall can be rebuilt around. The bong alone only ever gets
worse. The bong health pack is the repair - the first thing the player can *do*
about the objective they are told to protect.

## 2. The interview: what was decided

| Question | Decision |
|---|---|
| How is it delivered? | **Instant heal on pickup.** Drive over it anywhere on the field and the bong heals where it stands. No carry state, no escort trip. |
| How much does it heal? | **Full heal.** One pack restores the bong to `max_health`. |
| Who can collect it? | **Each side heals its own bong.** A player heals the player's bong; an enemy in a Hunt round heals the enemy bong. |
| What if the bong is already at full health? | **Not collectible.** The tank drives over it and the pack stays where it is - a full heal is too valuable to throw away by accident. |
| What about a side with no bong at all? | **Collected and wasted.** An enemy in a Protect round has nothing to heal and consumes the pack anyway: denial pressure, grab it before they blunder into it. |
| Where do packs come from? | **Map slots, plus a bonus drop beside a Health slot while the bong is hurt.** The pack appears when it matters and still has to be fetched. |
| What does the icon look like? | **A health pack with a bong glyph**, green - derived from `health.png` the way `gen_shield_pickup.py` derives the shield. |

## 3. The rule

The three collection cases above are one sentence, and that sentence is what the
code implements and what `PickupKind::BongHealth`'s doc comment says:

> A tank collects a bong pack **unless its own side's bong is alive and already
> at full health.**

Reading it back out:

| The collector's own bong | Outcome |
|---|---|
| alive, below `max_health` | collected, healed to full |
| alive, at `max_health` | **not collected** - the pack stays on the field |
| absent (enemy in Protect, anyone in Destroy) | collected, wasted |
| dead | collected, wasted - a dead bong is deliberately not "full" (`Bong::heal` is a no-op; the round has already ended anyway) |

"Its own side's bong" is `Game::bong` for a player and `Game::enemy_bong` for an
enemy, picked by `Game::is_player`. Nothing about this rule cares which *player*
collects it in a two-player round - both players share one bong.

## 4. The heal

`Bong::heal(&mut self, amount: f32)` sits directly beside `Bong::damage` and
mirrors its shape: a no-op once dead, otherwise `health` raised and clamped at
`max_health`. A bong pack cannot revive a dead bong, and there is no case where
it would matter - the player's bong dying ends the round on the same frame.

The amount is `max_health * bong_pack_heal_fraction`, which defaults to 1.0: the
agreed full heal, with a knob for QA to weaken it without a rebuild.

No animation state is added. The bong's ground-ring gauge (`draw_bong_ring`)
refills on its own from `health_fraction`, and the filmstrip set
(docs/BONG_SPEC.md) has no "healed" clip to play.

The heal happens in `pickup_phase`, but it **cannot** happen inside the per-kind
effect match: that block holds a live `&mut Tank` borrow from `query_one`. The
bong is healed after the borrow closes, next to the `Event::PickupCollected`
push.

## 5. Where packs come from

**Map slots**, like every other pickup: a `{ kind = "pickup", pickup =
"bong_health" }` cell, paintable in the builder, respawning on the normal
`pickup_respawn_seconds` timer through `respawn_from_slots`.

**Plus a bonus drop beside a Health slot.** The rainbow shield already works
this way (`maybe_spawn_bonus_shield`): each time a Health slot is spawned or
respawned, a roll drops an un-slotted shield in a free neighbouring cell. The
bong pack reuses that path rather than copying it - `maybe_spawn_bonus_shield`
and `bonus_shield_cell` are generalised to take the kind they are placing and
looking for - with one extra gate: **the roll only happens while the bong is
alive and below `bong_pack_bonus_below` of its max health.**

On a Health slot the two bonuses roll shield first, bong pack second, with
`occupied` recomputed between them so they cannot land on the same neighbour.

Being un-slotted, a bonus pack does not count toward `slot_backed_count`, so it
can never make the field look full to the respawn timer - the same reason the
bonus shield does not.

## 6. Determinism

Round randomness is one seeded `SmallRng` stream, and the draw *order* is part
of a replay (docs/gameplay-verification-design.md). The bonus roll is therefore
gated on the bong being hurt **before** any `rng.random_range` call, never after.
That buys three properties:

- `Game::init`'s spawn pass never rolls a bong pack: the bong is pristine at
  init, so **round setup draws exactly the RNG it drew before this feature**.
- A round in which the bong is never hurt is byte-identical to the same seed
  before this change.
- A round in which the bong *is* hurt shifts the stream from that Health-slot
  respawn onward. This is unavoidable if the feature is to exist at all.

`just probe-fixtures` is consequently a required step, and any ceiling it
exceeds must be shown to be the stream shift rather than a behaviour change -
re-run with `bong_pack_near_health_chance = 0` through `--tuning` to confirm -
before re-baselining per that recipe's own policy.

## 7. Presentation

**The pickup** is a supply crate with a top-down bong sprayed on its lid in
the HUD's bong green (`static/crates_sheet.png`, docs/CRATES_SPEC.md). The
symbol's ink is off the palette on purpose - pickups are loud so they're
spotted at a glance.

**The collect cue** is a small green burst at the bong, driven out of
`Event::BongHealed` by `Fx::observe`'s one-shot half. `fx.rs` is presentation
only and may use `rand::rng()` freely; nothing about the burst can reach a
simulation value.

**No HUD change.** The bar already carries a `BONG` gauge of the bong's health
fraction, which is exactly the feedback a heal needs - it refills. The pack is
not a weapon and takes no `WEAPON_SLOTS` slot.

**No change to `draw_pickup`**: the sprite draws flat like every other pickup,
no glow and no bob.

## 8. Knobs

Three rows in `group pickups`, beside `shield_near_health_chance`:

| Knob | Default | Range | What it does |
|---|---|---|---|
| `bong_pack_heal_fraction` | 1.0 | 0.0 ..= 1.0 | Fraction of `bong_max_health` one pack restores. 1.0 is the agreed full heal. |
| `bong_pack_near_health_chance` | 0.35 | 0.0 ..= 1.0 | Odds of dropping a bonus bong pack beside a Health slot when it spawns, *while the bong is hurt*. 0 disables the bonus entirely. |
| `bong_pack_bonus_below` | 0.75 | 0.0 ..= 1.0 | The "bong is hurt" threshold: the bonus only rolls below this fraction of the bong's max health. |

## 9. Tests

Headless `mechanics_tests` on tiny inline maps, one per promised rule:

1. A player collecting a pack restores a damaged bong to `max_health`, and
   emits `PickupCollected` + `BongHealed`.
2. A player whose bong is at full health drives over the pack and it is still
   there - no `PickupCollected`.
3. A hurt bong is healed even when the collecting tank is itself at full health:
   the pack is not a tank heal.
4. Protect: an enemy driving over a pack consumes it and no bong is healed.
5. Hunt: an enemy collecting heals the *enemy* bong and leaves the player's
   alone.
6. A dead bong is not revived.
7. The bonus drop lands on a free neighbouring cell when the bong is hurt, and
   with a pristine bong never rolls - asserted against a same-seed control round
   so "draws no RNG" is checked, not just "spawns nothing".

Plus the counts a new tool moves: the editor's `TOOLS` length and the dev
server's PICKUP category count.

## 10. Non-goals and as built

Non-goals:

- **No AI seek tier.** Enemies collect a pack by driving over it, never by
  wanting it - no `act_seek_*` leaf and no behaviour-tree registration. Same
  stance the flamethrower took. Making enemies fetch packs for their bong in
  Hunt is a real idea, and a separate one.
- **No carry or charge state**, no partial heal, no auto-apply-later, no HUD
  slot.
- **No revive.**
- **Not authored into Destroy maps** - there is no bong there, so a pack would
  be inert flavour.

As built, the deltas from the text above:

- **`Bong::at_full_health`** was added beside `heal`, so the collect rule
  lives in one predicate rather than being spelled out at the call site. It
  is `at_full_health`, not `needs_healing`, on purpose: a dead bong is not
  full, so a pack driven over after one dies is consumed like any other
  side with nothing to heal, exactly as the rule sentence reads.
- **The heal reports its own position.** `Event::BongHealed { side, slot,
  amount, x, y }` carries the *bong's* coordinates, not the pickup's: the
  bong is generally nowhere near the pack, and `fx.rs` needs to put the
  burst on the thing that got healed.
- **`maybe_spawn_bonus_shield` became two functions**, not one:
  `maybe_spawn_health_slot_bonuses` (what a Health slot rolls - the shield,
  then the bong pack behind its gate) and `maybe_spawn_bonus` (one kind,
  one chance), with `bonus_shield_cell` renamed `bonus_pickup_cell`. The
  shield's own behaviour is unchanged, and its two tests pass untouched.
- **The determinism gate is tested directly**, not only by its effect:
  `a_pristine_bongs_health_slot_draws_no_more_rng_than_the_shield_roll_alone`
  runs `maybe_spawn_health_slot_bonuses` with `bong_hurt: false` and the
  shield roll alone from the same seed, and asserts the next draw off each
  RNG matches. That is the property section 6 claims, stated as a test.
- **`just probe-fixtures` needed no re-baselining.** None of the seven
  `maps/test/` fixtures has a Health slot, so `maybe_spawn_health_slot_bonuses`
  is never reached in any of them and their RNG streams are byte-identical
  by construction, hurt bong or not.
- **The bonus gate reads the player's bong only.** A Hunt round's enemy
  bong never summons a pack; it is welcome to whatever the player leaves
  on the ground. The bonus exists to give the *player* a way back.
- Editor spellings: tool name `bong_health`, dropdown label `bong pack`,
  cursor/bar short label `bong+`. `TOOLS` is 29, the dev server's PICKUP
  category 9.
