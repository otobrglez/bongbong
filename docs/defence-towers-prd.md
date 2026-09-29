# PRD: defence towers (the tesla coil and the machine-gun tower)

Status: **draft, 2026-09-29.** Nothing is implemented. Section 2 lists every
decision this PRD needs from the author: four are agreed, the rest carry the
answer the document assumes until the author replaces it. The art is being chosen separately in
the "Defence tower picks" artifact from the session that wrote this: eight
tower proposals, three lightning styles, three ways of marking a tower's side
and three repair pickups, drawn at the game's own pixel density next to the
real sprites. Section 11 names the proposals by their codes (T1-T4, G1-G4,
L1-L3, M1-M3, R1-R3) and is filled in once the picks are made.

The request, as written:

> Two defence towers. A "tesla coil" that zaps tanks with lightning if an
> enemy approaches. A machine gun that shoots bullets, similar to those from
> tank guns. Both are placed on the map via the map builder. Both have
> different visuals when they are hurt and when they are destroyed. They can
> also catch fire and leave smoke. Sometimes bullets and weapons bounce away
> from these buildings. Picking up "rainbow health" restores them.

Contents

1. Why
2. Decisions: proposed answers
3. What a tower is
4. The tesla coil
5. The machine-gun tower
6. Damage, fire, smoke and the ruin
7. Ricochet
8. Repair: the rainbow pack
9. Sides, friendly fire and the AI
10. Map format, builder and linter
11. Presentation
12. Online co-op
13. Determinism
14. Knobs
15. Tests
16. Non-goals and open questions
17. Phases

## 1. Why

Everything a map places today is passive. Walls, props and trees block, burn
and explode when something else acts on them; the only map-placed things that
act on their own are the two frogs, and a frog is a fail state, not a weapon.
A map maker can shape where a fight happens but never who holds a position.

Towers are the first placed thing that fights. For the player they are a way
to hold ground in a Protect round: put a tesla coil beside the frog and the
hunters that reach it pay for it. For a map maker they are the first way to
make a *place* dangerous: an enemy gun tower on a bridge makes the bridge a
decision. And for the AI they are a new kind of pressure that the routing
layer (`route_lane_cost`, the flow fields) already knows how to express.

The two towers are deliberately different. The tesla coil is **short-range,
telegraphed and heavy**: it charges visibly while a tank is in reach and then
strikes, so a player can see it coming and back off. The gun tower is
**long-range, constant and light**: it tracks and chips, and it is the one you
flank. Neither moves, both can be shot down, and both can be repaired.

## 2. Decisions: proposed answers

Every row is a question for the author. Rows marked **agreed** were answered
on 2026-09-29; the others are **proposed**, and the rest of this document is
written against them until they are answered. A different answer changes the
sections named in the last column.

| # | Question | Answer | Changes |
|---|---|---|---|
| 1 | Whose towers are they? | Agreed: **either side, set per tower in the builder.** A player's tower shoots enemies; an enemy tower shoots the players. A map without the `side` key gets player towers. | 9, 10 |
| 2 | What is "rainbow health"? | Agreed: **a new pickup, the tower pack**, which repairs towers only. The game's existing "rainbow health pack" is the shield pickup, and making the shield repair towers too would tie tower balance to a drop that already rolls on every Health slot. | 8, 11 |
| 3 | What does the pack restore? | Proposed: **every standing tower on the collector's side**, to full health, fire put out, anywhere on the map. | 8 |
| 4 | Is a destroyed tower gone? | Agreed: **yes, for the round.** It leaves a passable ruin decal and a scorch, and the pack does not rebuild it. | 6, 8, 12 |
| 5 | How big is a tower? | Proposed: **one cell**, like a tree: a 32 px collider and nav-grid cell, a 48 px sprite that overhangs its neighbours. | 3, 10, 11 |
| 6 | Does the tesla chain or stun? | Proposed: **one chain jump at half damage, no stun.** | 4 |
| 7 | Do towers hurt their own side? | Proposed: **no.** A tower never targets its own side, holds fire while a friendly hull is in its line, and its bullets pass through its own side's tanks. | 5, 9 |
| 8 | Do towers see through tall grass? | Proposed: **the tesla does, the gun does not.** The tesla senses proximity; the gun needs to see its target, like an enemy tank does (`Terrain::conceals`). | 4, 5 |
| 9 | How do enemies treat towers? | Agreed: **they avoid a player tower's reach and shoot back when one hits them.** No enemy hunts towers as an objective. | 9 |
| 10 | Can a tank ram a tower down? | Proposed: **no.** A tower is a building; ramming it does nothing. | 6 |
| 11 | Is online co-op in scope? | Proposed: **yes, in the last phase**, with a protocol bump. | 12, 17 |

## 3. What a tower is

**A tower is an `Obstacle`**, with two new materials appended after `Pine`:
`Material::Tesla` and `Material::GunTower` (the first four materials index
`wall_max_health` and must stay first). Being a tile gives a tower everything
a tile already has, with no new code:

- a static rapier body (`Physics::spawn_static`) that blocks movement;
- a box in `hits::Terrain`, so shots, lasers and the flame's reach stop on it;
- a blocked cell in the nav grid, and a solid cell to `maplint`'s floods;
- `blocks_sight` true, so it blocks line of sight like a wall;
- damage through `props::damage_obstacle`, death through `obstacle_died`,
  blast damage from `apply_blast`, heat from the flamethrower;
- a `TileState` on the wire (health and the burning flag) and an entry in a
  `Welcome`'s `dead_cells` once it is gone.

**What a tile does not have is a weapon**, so the tower's own state lives
beside it in `Game::towers: BTreeMap<u16, Tower>`, keyed by cell index
(`row * cols + col`, the wire's tile key). A `BTreeMap` because the tower
phase walks it and draws RNG (section 13). `init` builds it from the map;
`obstacle_died` removes the entry when the tile dies.

```rust
pub struct Tower {
    pub kind: TowerKind,          // Tesla | Gun
    pub side: Side,               // frog::Side: Player | Enemy
    pub heading: f32,             // gun: turret heading, degrees, 0 = up
    pub target: Option<Entity>,   // the tank it is tracking or charging at
    pub charge: f32,              // tesla: 0..=1
    pub cooldown: f32,            // seconds until it may start again
    pub burst_left: u32,          // gun: bullets left in this burst
    pub burst_timer: f32,         // gun: seconds to the next bullet
    pub burning: bool,            // section 6
}
```

**One cell, drawn like a tree** (decision 5). `Obstacle::size()` stays the
32 px cell for the collider, the grid and the map; the sprite is 48 px, the
tree convention from docs/TREES_SPEC.md §1 (one design pixel = two screen
pixels, the extra 8 px each side overhangs). Unlike a tree, a tower's hit box
is the full cell and it takes part in seam-closing like a wall.

**Drawn standing, not flat.** The art rises north of its base, so a tank that
drives behind (north of) a tower must be drawn behind it. Towers go into
`paint_standing`'s y-sorted walk with the tanks and frogs, not into
`paint_tiles`. Their shadow is drawn in `paint_tiles`' shadow pass so it falls
under everything.

**A new `Owner` variant.** Shots a tower fires carry
`Owner::Tower { side: Side, cell: u16 }`. `same_side` treats it as its side's
(`Tower { side: Player }` is on the players' side). The research for this PRD
counted about ten exhaustive `match`es on `Owner` outside tests
(`shell.rs`'s `slot`, `encode::owner_seat`, `weapons::side_damage`,
`hits::sweep_rewound`'s pad, `present.rs`, `combat.rs`, `flame.rs`, two in
`tank.rs`, `debug.rs`); each needs an arm, most of them unreachable for a
tower. Borrowing an existing owner does not work: `Owner::Player(0)` would
make a tower's bullets pass through player 1 only and take player 1's
lag-compensation rewind, and `Owner::Enemy(slot)` needs a slot that names no
tank, which `Fired`/`Ricochet` events would then misreport.

**The tower phase** runs in `Game::update` after `enemy_phase` and before
`spawn_pending`/`resolve_lasers`, so a tower's bullets spawn and its strikes
resolve on the same frame as everyone else's shots.

## 4. The tesla coil

**Reach and the telegraph.** The coil looks for opposing live tanks whose hull
box comes within `tesla_range` (112 px, three and a half cells) of its centre,
with line of sight from its terminal (`Terrain::line_of_sight`, its own tile
ignored; walls block, sandbags and fences do not). While one is in reach, the
coil **charges**: `charge` rises to 1 over `tesla_charge_seconds` (1.3 s) and
the coil visibly lights up. If every target leaves reach, the charge drains at
`tesla_drain_per_second` (1.0). This is the whole point of the tower: a player
sees the coil wind up and can back out; a tank that stays is hit.

**The strike.** At full charge the coil strikes the nearest target in reach
(ties broken on owner slot, never RNG). The strike is instant, exactly like
the laser: `apply_hit` with `HitEffects::none()`, damage rolled from
`tesla_damage_min`..`tesla_damage_max` (18..26), through `take_damage` so a
shield absorbs it, `mark_hit`, and `Ai::notify_hit` for an enemy. Charge
returns to 0 and `tesla_cooldown_seconds` (0.35) must pass before it charges
again.

**The chain** (decision 6). The bolt then jumps once from the target to the
nearest *other* opposing tank within `tesla_chain_radius` (64 px), with line
of sight, for `tesla_chain_factor` (0.5) of the rolled damage.
`tesla_chain_jumps` 0 turns it off.

**What it zaps.** Tanks only. Not frogs, not tiles, not its own side.
Concealment in tall grass does not hide a tank from it (decision 8).

**Events.** `Event::TeslaStrike { cell, x0, y0, x1, y1, target: HitTarget,
chained: bool }` per bolt (the chain jump is a second event), then the usual
`Event::Hit`. The strike is presentation-only data, like `LaserBeam`: a
replica draws the bolt from the event.

## 5. The machine-gun tower

**Targeting.** The nearest opposing live tank within `gun_tower_range` (256
px, eight cells), in line of sight from the pivot and not concealed in tall
grass (decision 8). It keeps its current target while that target stays
valid and switches only when another is `gun_tower_switch_margin_px` (48)
nearer, the same hysteresis `Ai::target_player` uses.

**Turret.** Any angle, turning at `gun_tower_turn_deg_per_second` (220)
toward where the target will be: its centre plus its body velocity times the
bullet's flight time, scaled by `gun_tower_lead` (0.5; 0 aims at where the
tank is). It opens fire when the turret is within `gun_tower_fire_cone_deg`
(6) of that aim. The sprite snaps to 16 baked directions (section 11); the
bullets fly at the true heading.

**Bursts.** `gun_tower_burst_size` (5) bullets, `gun_tower_bullet_delay_seconds`
(0.06) apart, then `gun_tower_burst_gap_seconds` (0.9). Unlimited ammunition.
Each bullet is a real `Bullet`, built directly (every field is `pub`; the
constructor `Bullet::spawn` wants a `&Tank`) and pushed onto
`Frame::pending_bullets`, with the minigun's speed, a spread of
`gun_tower_spread_deg` (3), and damage `gun_tower_damage_min`..`max` (3..5)
through a `Owner::Tower` arm in `weapons::side_damage`. Everything downstream
of the spawn - the sweep, pass-over, ricochet, `apply_hit`, the net family -
is the minigun's.

**Friendly fire** (decision 7). It holds fire while a live tank of its own
side is within `gun_tower_friendly_block_px` (20) of the line to its target,
and its bullets pass through its own side's tanks in `sweep_rewound` (the same
skip a shooter gets today).

**Its own tile.** The muzzle is 21 px from the pivot, outside the cell except
on the diagonals, so the tower's own tile goes into the sweep's `ignore` list
for its bullets rather than relying on geometry.

**Events.** `Event::TowerFired { cell, x, y, heading }` once per burst, the
muzzle ripple and sparks hang off it.

## 6. Damage, fire, smoke and the ruin

**Health.** `tesla_max_health` 120, `gun_tower_max_health` 150, baked in at
spawn like every tile's. For scale: a tank is 100, a brick wall 20, iron 220;
a player shell does 10..30. Both fit `TileState`'s `u8` whole points.

**Four looks while standing, one after.** `visible_stages` is 4: intact,
scuffed (below 75 %), damaged (below 50 %: a bent barrel or broken ring,
sparks, a thin smoke), critical (below 25 %: it catches fire). Destroyed is
the fifth look, a ruin (below). The artifact draws all five for every
proposal.

**Fire.** A tower catches fire when its health falls below `tower_burn_below`
(0.25 of max), or when anything that lights a flammable tile lights it: the
flamethrower's heat (`flame_ignite_seconds`), a burning ground cell under it,
a drum blast. A burning tower is *not* a burning plank - it does not burn out
on a timer and die - it takes `tower_burn_dps` (2) until it is repaired or
destroyed, and keeps fighting at `tower_burning_fire_factor` (0.5) of its
rate: the tesla charges at half speed, the gun fires every other bullet. That
is the "it barely works" look the artifact shows. Water puts nothing out; the
pack does.

This needs one change to shared code: today `Obstacle::burning` means "this
tile is on its way to dying" (wood), and `net::apply::apply_tiles` sets a
burning tile's health to 0. A tower's burning is kept in `Tower::burning`, and
the `BURNING` wire flag is read per material.

**Smoke and sparks.** Sampled from state in `Fx::sample_world`, the way
burning tiles and wrecks are: `Game::damaged_towers()` (sparks at the
design's spark points, a thin smoke) and `Game::burning_towers()` (flames at
the fire points, heavy smoke, embers). A tesla's sparks are cyan.

**Death.** Through `obstacle_died` like any tile, which pushes
`ObstacleDestroyed`, drops the ruin and removes the `Tower` entry. Each tower
dies its own way:

- **Tesla:** a discharge. A `PendingBlast` of `tesla_death_blast_radius` (64
  px) that shoves and does `tesla_death_blast_damage` (10) to every tank in
  it, both sides, plus a ring of arcs to anything in reach.
- **Gun tower:** its ammunition cooks off - two or three hashed `CookOff`
  pops over a second, the wreck mechanism already in `props.rs`.

**The ruin** (decision 4). A decal, not a tile: the cell is open and passable
from the frame the tower dies, with a scorch and the tower's ruin art on the
ground, smouldering (`tower_ruin_smoke_seconds`, 8). The ruin art lives on
the towers sheet, not the walls sheet's rubble rows, because it is 48 px.

**Ramming** (decision 10). A tower has no `ram_seconds`; a tank pushing into
one just stops, as it does against a wall.

## 7. Ricochet

Shells and bullets glance off a tower at `tesla_deflect_chance` (0.15) and
`gun_tower_deflect_chance` (0.25) - the gun tower is armour, the coil is a
shell of copper. This is the barrel's mechanism exactly:
`Material::deflect_chance` returns the knob, `resolve_projectiles` rolls it
for any projectile whose `can_bounce()` (shells and bullets; plasma never),
and the shot reflects off the hit face and flies on with its owner unchanged,
pushing the existing `Event::Ricochet`. The laser and the tesla's own bolt
never ricochet; missiles have no hit test and only their blast touches a
tower. Nothing passes over a tower (`pass_over_chance` 0: it is tall).

## 8. Repair: the rainbow pack

Written against decisions 2, 3 and 4. Decision 2 settled the artifact's R1
(the shield also repairs) out; R2 and R3 are the two icons left to choose.

**A new pickup, `PickupKind::TowerPack`**, built like the frog pack
(docs/frog-health-pack-prd.md): a map slot `{ kind = "pickup", pickup =
"tower_pack" }`, respawning on `pickup_respawn_seconds`.

**The rule**, in the frog pack's shape:

> A tank collects a tower pack **unless every standing tower on its side is
> already at full health and not burning.**

| The collector's side | Outcome |
|---|---|
| has a hurt or burning tower | collected; every standing tower of that side restored to full and put out |
| has towers, all full | **not collected**; the pack stays |
| has no standing tower | collected and wasted (denial, as the frog pack) |

Enemies collect it by driving over it, as with the frog pack; no AI seek tier.

**Bonus drop.** Beside a Health slot, like the shield and the frog pack
(`maybe_spawn_health_slot_bonuses`), at `tower_pack_near_health_chance`
(0.25), gated on "a player tower is hurt" **before** any RNG draw, so a map
without towers - or with pristine ones - draws exactly what it drew before.

**Events.** `Event::TowersRepaired { side, count }` plus one
`Event::TowerRepaired { cell, x, y }` per tower, which `fx.rs` turns into the
rainbow burst at the tower.

The icon is derived from `health.png` the way the shield's and the frog
pack's are, by a `tools/gen_tower_pack_pickup.py` in the same raw-PNG-bytes
convention: the box swept through the rainbow, the cross replaced by the
chosen glyph (R2 a wrench, R3 a small coil tower).

## 9. Sides, friendly fire and the AI

**Side.** `frog::Side`, set per tower in the map (decision 1). A player tower
targets enemies; an enemy tower targets seats that are on the field
(`seats_on_field()` - never a seat in a gate lane).

**Enemies and player towers** (decision 9):

- **Routing.** Every cell within reach of a live player tower gets
  `route_tower_cost` (4) added in `Game::route_grid`, the way the cells down a
  player's barrel get `route_lane_cost`: the flow field bends around the
  tesla's reach and the gun's lanes when there is another way, and goes
  through when there is not. 0 turns it off.
- **Retaliation.** An enemy hit by a tower (`notify_hit` from an
  `Owner::Tower`) remembers that tower for `enemy_tower_grudge_seconds` (4);
  while no player is within its attack range, it attacks the tower through
  the existing attack tier - a tower is a position plus a line-of-fire check
  that ignores its own tile, which is what the frog already is to a hunter.
- **Breach.** Unchanged: a tower blocking the way is a destructible tile and
  gets shot like one.

**Players and enemy towers** need nothing: they shoot them.

**Hunt** gets its natural shape for free: enemy towers around the enemy frog
guard it.

## 10. Map format, builder and linter

**Map.** Two cell kinds, the side optional:

```toml
cells."12,6" = { kind = "tesla" }
cells."14,6" = { kind = "gun_tower", side = "enemy" }
```

`CellObject::Tesla { side: Option<Side> }` and `CellObject::GunTower { side:
Option<Side> }` (`#[serde(rename = "gun_tower")]`), with `side` defaulted and
skipped when absent, exactly as `Barrel`'s `drum` is. `material()` maps both,
so `is_solid()` holds.

**Builder.** Four tools in the PROP category after `pine`: `tesla`,
`tesla_enemy`, `gun_tower`, `gun_tower_enemy`. Four, not two with a side
switch, because the builder has no per-cell attribute editor and its cursor
readout names a cell by the tool whose `object()` matches it exactly - the
drum kinds are two tools for the same reason. PROP grows from 7 to 11 rows,
which is exactly what the dropdown fits on the 1088 x 544 field; a sixth
category would not fit the bar (`bar_slots_fit_the_default_bar_without_overlapping`).
While a tower tool is under the cursor the builder draws its reach circle.

Text: `tool-tesla`, `tool-tesla_enemy`, `tool-gun_tower`,
`tool-gun_tower_enemy` and their `tool-short-*` in `lang/en.ftl` and
`lang/sl.ftl`, inside `every_language_fits_every_budget` (144 px for the long
name, 48 px for the short). Proposed short names: `tesla`, `e.tsl`, `gun`,
`e.gun`. The dev server's `builder_tool` description and
docs/game-editor-fusion.md list the tool names and need the four added.

**Linter.** Towers are obstacles, so connectivity already counts them. New:
`tower-in-gate-lane` (error: a tower on a gate's roll-in lane), `tower-at-start`
(warning: an enemy tower whose reach covers a seat's start cell - the round
would open under fire), `tower-no-reach` (info: a tower whose reach covers no
playfield cell).

**Fixtures.** `maps/test/towers.toml` - both kinds, both sides, a tesla
beside the frog - joins `just probe-fixtures` with recorded ceilings.

## 11. Presentation

**Art is chosen in the artifact.** The rows below are filled in from the
author's picks; until then the first card of each is the placeholder.

| Pick | Options | Chosen |
|---|---|---|
| Tesla coil | T1 copper coil, T2 lattice pylon, T3 capacitor dome, T4 twin-fin pylon | *open* |
| Lightning | L1 pixel bolt, L2 glowing arc, L3 violet filaments | *open* |
| Gun tower | G1 sandbag nest, G2 armoured turret, G3 concrete pillbox, G4 gatling platform | *open* |
| Side marking | M1 painted livery, M2 ground ring, M3 pennant | *open* |
| Tower pack icon | R2 rainbow wrench, R3 rainbow tower (R1, the shield also repairing, was ruled out by decision 2) | *open* |

**The sheet.** `static/towers_sheet.png`, generated by
`tools/spritegen/gen_towers.py` with a fixed seed onto the Puny palette, 48 px
cells on the 2 px block grid (24 x 24 design pixels), a 1-design-pixel
`#252525` outline like the tanks, and a spec in `docs/TOWERS_SPEC.md`. Proposed
layout: per tower kind and side, columns 0-3 the four standing stages and
column 4 the ruin; the tesla adds charge frames (the lit winding or strips);
the gun's turret is its own layer, 16 directions x (intact, damaged). The
side's colours come from `TEAM_P1` and the red ramp, so the player rows are
admitted by `check_sheets.py` the way the player tank rows are. Run
`just check-sheets`: no green on anything drawn over the ground.

**The bolt.** Drawn by `render/tesla.rs` from `Event::TeslaStrike`: a jagged
polyline hashed from the frame and the cell (no RNG), redrawn three times over
`tesla_bolt_display_seconds` (0.24) so it crackles, a pool of light under both
ends (`Game::draw_ground_light`), sparks from `fx.rs`, and the target's hull
flashed. L1 needs no shader; L2 uses `shot_fx::glow`; neither needs a
`static/web/` twin.

**Sparks, smoke and fire** as section 6. **Health gauge**: a tower shows the
enemies' ground gauge after it is hit, the same `RingStyle::Gauge` - or always,
in its side's colour, if M2 is picked.

**No HUD change.** Towers are not a weapon or a seat.

## 12. Online co-op

Phase 5, and a `PROTOCOL_VERSION` bump (9 to 10): `Material` gains two
variants and `WireEvent` gains the tower events.

- **Health and fire** travel in `TileState` (`hp`, `BURNING`), with
  `apply_tiles` reading `BURNING` per material (section 6).
- **The turret and the charge** travel in a new keyed family, `TowerState {
  cell: u16, heading: u8, charge: u8 }`, sent only for towers whose values
  changed (delta.rs's `diff_keyed`). The replica eases the heading in
  `tick_presentation` the way a tank's turret is eased, and takes the charge
  from the bracket's near end.
- **Bullets** are `ShotState`s already. `owner` is `NO_SEAT` for a tower;
  the present-time pass for incoming fire (`draw_incoming_in_present`) must
  not stop a *player* tower's bullet on this seat's hull, so the owner byte
  gains a "player tower" value.
- **Strikes** travel as `WireEvent::TeslaStrike`, mirrored from the new
  `Event`; `every_variant_is_sent_or_on_the_not_sent_list` counts it.
- **Lag compensation** needs nothing: towers do not move, and a tower's shots
  are the room's own.
- **Prediction**: this seat's provisional shots already stop on tiles
  (`PresentWorld::shot_contact`), so they stop on towers. A ricochet the client
  did not draw is the existing "room copy flies on" case.

## 13. Determinism

- Towers are walked in cell order (`BTreeMap`), after the enemies.
- Targets break ties on owner slot; no RNG in targeting, turning or charging.
- RNG draws: the strike's damage roll (and its chain's), each bullet's spread
  and damage (`fire_bullet`'s draws), the ricochet roll only for chances
  strictly between 0 and 1, the tower pack's bonus roll behind its gate.
- A map with no towers draws nothing new, so
  `determinism_tests::the_one_and_two_seat_streams_are_pinned` and every probe
  fixture replay unchanged; that is a test (section 15), not a hope.
- Every cosmetic choice - the bolt's jags, spark scatter, ruin variant, the
  cook-off timing - is hashed from position and frame.

## 14. Knobs

A new `group towers` in `tuning.rs`:

| Knob | Default | Range | What it does |
|---|---|---|---|
| `tesla_max_health` | 120 | 1..=1000 @ Spawn | Tesla toughness |
| `tesla_range` | 112 | 32..=600 | Reach, px from the centre to a hull box |
| `tesla_charge_seconds` | 1.3 | 0.1..=10 | Time to full charge with a target in reach |
| `tesla_drain_per_second` | 1.0 | 0..=10 | Charge lost per second with nothing in reach |
| `tesla_cooldown_seconds` | 0.35 | 0..=10 | Pause after a strike before charging again |
| `tesla_damage_min` / `_max` | 18 / 26 | 0..=100 | Strike damage roll |
| `tesla_chain_jumps` | 1 | 0..=4 | Extra targets a strike jumps to |
| `tesla_chain_radius` | 64 | 0..=300 | Jump reach from the last target |
| `tesla_chain_factor` | 0.5 | 0..=1 | Damage of each jump, as a fraction of the roll |
| `tesla_bolt_display_seconds` | 0.24 | 0.05..=1 | How long the bolt is drawn |
| `tesla_deflect_chance` | 0.15 | 0..=1 | Odds a shell or bullet glances off |
| `tesla_death_blast_radius` | 64 | 0..=300 | The discharge when it dies |
| `tesla_death_blast_damage` | 10 | 0..=100 | Damage of that discharge |
| `gun_tower_max_health` | 150 | 1..=1000 @ Spawn | Gun tower toughness |
| `gun_tower_range` | 256 | 32..=1200 | Reach |
| `gun_tower_turn_deg_per_second` | 220 | 10..=2000 | Turret turn rate |
| `gun_tower_lead` | 0.5 | 0..=1 | How far it aims ahead of a moving target |
| `gun_tower_fire_cone_deg` | 6 | 0..=45 | Aim error it opens fire within |
| `gun_tower_switch_margin_px` | 48 | 0..=400 | Target hysteresis |
| `gun_tower_burst_size` | 5 | 1..=30 | Bullets per burst |
| `gun_tower_bullet_delay_seconds` | 0.06 | 0.01..=1 | Between bullets |
| `gun_tower_burst_gap_seconds` | 0.9 | 0..=10 | Between bursts |
| `gun_tower_spread_deg` | 3 | 0..=45 | Bullet spread |
| `gun_tower_damage_min` / `_max` | 3 / 5 | 0..=100 | Bullet damage roll |
| `gun_tower_friendly_block_px` | 20 | 0..=100 | Holds fire when a friend is this close to the line |
| `gun_tower_deflect_chance` | 0.25 | 0..=1 | Odds a shell or bullet glances off |
| `tower_burn_below` | 0.25 | 0..=1 | Health fraction at which a tower catches fire |
| `tower_burn_dps` | 2 | 0..=50 | Damage a burning tower takes per second |
| `tower_burning_fire_factor` | 0.5 | 0..=1 | Fire rate while burning |
| `tower_ruin_smoke_seconds` | 8 | 0..=60 | How long a ruin smoulders |
| `tower_pack_near_health_chance` | 0.25 | 0..=1 | Bonus pack odds beside a Health slot while a player tower is hurt |
| `route_tower_cost` | 4 | 0..=64 | Route surcharge on cells in a player tower's reach |
| `enemy_tower_grudge_seconds` | 4 | 0..=30 | How long an enemy hit by a tower goes after it |

## 15. Tests

`mechanics_tests` on tiny inline maps, one per promised rule:

1. A tesla charges only while an opposing tank is in reach and strikes it at
   full charge; a tank that backs out before full charge takes nothing.
2. The strike chains once to a second tank within the chain radius, for half
   the rolled damage, and not to a tank behind a wall.
3. A tesla never charges at its own side, a frog, or a tank behind a wall.
4. A gun tower turns toward a target at its turn rate and fires only inside
   its fire cone; its bullets pass through its own side's tanks; it holds fire
   while a friend is in the line.
5. A gun tower does not see a tank concealed in tall grass; a tesla does.
6. Damage takes a tower through its four stages; below `tower_burn_below` it
   burns and loses `tower_burn_dps`; burning halves its fire rate.
7. A flamethrower held on a pristine tower lights it.
8. A dead tower leaves an open, passable cell and no `Tower` entry.
9. A tesla's death blast and a gun tower's cook-offs happen.
10. With `deflect_chance` 1 a shell ricochets off each kind with an
    `Event::Ricochet`; plasma never does.
11. The tower pack restores every standing tower of the collector's side and
    puts it out; it is not collected when all are full; an enemy collecting it
    repairs enemy towers only.
12. A map without towers draws exactly the RNG it drew before (a same-seed
    comparison, the frog pack's `a_pristine_frogs_health_slot...` pattern).

Plus: map round-trip of both kinds with and without `side`; the editor's
`TOOLS` count, the dropdown-fit test and `every_tool_has_a_unique_name_that_parses_back`;
`maplint` fixtures for the three new lints; `maps/test/towers.toml` in
`just probe-fixtures`; the thumbnail hashes if a shipped map gains towers;
and in phase 5 the `apply.rs` round-trip tests with towers on the map and the
`net` event-count test.

## 16. Non-goals and open questions

Non-goals:

- **Building towers in a round.** Towers come from the map; there is no
  in-round construction, currency or upgrade.
- **Towers that move, turn to face walls, or target tiles and frogs.**
- **A tower HUD.** No counts, no bar slot.
- **A third tower.** The two share the `Tower` struct so a third (mortar,
  flame tower) is a variant, not a new system - but it is not in this PRD.

Open questions beyond section 2's table, each with a lean:

- **Stun.** A strike could jolt the target (`tesla_stun_seconds`, its drive
  zeroed for a quarter second). Lean: no; it would take control away from a
  player with no counterplay.
- **Wet tanks.** A tank wading (docs/water.md) could take `tesla_wet_factor`
  (1.5) more. Lean: yes, as a knob that defaults to 1.0 until play-tested.
- **Limits.** Should a map cap its towers (the linter warning past N per
  side)? Lean: no cap, a lint `too-many-towers` as info past 6.
- **A shipped map.** One of `SHIPPED_MAPS` gaining towers (a Protect map with
  a tesla at the frog) makes the feature discoverable on the web build, where
  the builder is the only other way in. Lean: yes, in phase 2.
- **Wave difficulty.** An enemy tower is extra difficulty the wave scaling
  (`wave_size_scale`) does not know about. Lean: leave it to the map maker.

## 17. Phases

1. **Simulation and builder.** Materials, `Tower`, `Owner::Tower`, the tower
   phase, both weapons, damage stages, fire, ricochet, sides, map format,
   builder tools, lints, the test fixture, and the tests above. Placeholder
   art: the artifact's picks rendered flat into a first `towers_sheet.png`.
2. **Art and effects.** `gen_towers.py` and `TOWERS_SPEC.md` from the picks,
   the bolt renderer, sparks, smoke and fire in `fx.rs`, the ruin, and a
   shipped map.
3. **The tower pack.**
4. **AI.** Route surcharge and retaliation, re-baselining `just
   probe-fixtures` only if a fixture gains a tower.
5. **Online.** `TowerState`, the events, the protocol bump, the round-trip
   tests.
