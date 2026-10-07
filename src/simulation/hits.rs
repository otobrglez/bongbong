//! Projectile hit geometry. `Terrain` is a per-frame snapshot of the static
//! things a shot can hit (obstacle tiles with their seams closed, the frog,
//! the four boundary walls) and `Terrain::sweep` is the one hit test every
//! shell, bullet, plasma bolt and laser beam resolves against: a
//! segment-vs-box sweep over the projectile's whole movement this frame,
//! so nothing tunnels through a thin target however large `dt` was and
//! nothing threads the seam between two touching wall tiles. Rapier is not
//! involved - projectiles have no physics body.
//!
//! `HitBoxHistory` is lag compensation's memory (docs/online-coop-prd.md
//! §4.16, "favor the shooter"): the enemy tanks' and frogs' boxes at the
//! end of each of the last `REWIND_MAX_TICKS` ticks, so a seat's shot can
//! be swept against the world that seat's client was drawing. Only those
//! are rewound - tiles, walls and the seats stay current.

use std::collections::{HashSet, VecDeque};

use hecs::Entity;

use crate::ai::Ai;
use crate::battlefield;
use crate::frog::Frog;
use crate::obstacle::{Material, Obstacle};
use crate::shell::Owner;
use crate::tank::Tank;
use crate::tuning::tuning;
use crate::{FROG_COLLIDER_HALF_EXTENT, Position};

use super::with_tank;

/// What a projectile or beam hit. Read-only: the caller applies the effect
/// (see `Game::apply_hit`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShellTarget {
    /// Any tank, a player's or an enemy's; `Tank::owner` says whose.
    Tank(Entity),
    Frog(Entity),
    Obstacle(Entity),
    Wall,
}

/// One obstacle tile's hit box this frame.
pub(crate) struct TerrainBox {
    pub entity: Entity,
    pub center: Position,
    /// Half-extents, widened to the full half-cell on any axis with an
    /// adjacent tile (`battlefield::tile_hull_half_extent`) - the same box
    /// the tile's physics collider has.
    pub half: Position,
    pub material: Material,
    /// Wood already alight: still solid, but further damage is a no-op.
    pub burning: bool,
}

/// Static terrain snapshot for one frame - see the module doc. Built once
/// per `Game::update`, after the frog's hop tick, and shared by the AI's
/// line-of-sight checks, engagement-slot validation and every hit test.
pub(crate) struct Terrain {
    obstacles: Vec<TerrainBox>,
    frogs: Vec<(Entity, Position)>,
    walls: [(Position, Position); 4],
    /// Centres of the map's tall-grass cells (`grass.rs`). Not obstacles:
    /// they block nothing and are not in the nav grid - they only answer
    /// `conceals`.
    grass: Vec<Position>,
    /// The map's water (docs/water.md), for the frog's hop. Not an
    /// obstacle here: deep water stops hulls through its physics
    /// colliders and never a shot, so `sweep` must not see it.
    water: crate::ground::WaterLayout,
    /// `player_shot_hit_pad_px` as the table held it when the snapshot was
    /// taken: the extra half-extent a player's shot gets against an
    /// enemy's hull and turret boxes in `sweep`.
    player_shot_pad: f32,
}

/// How far back lag compensation reaches, in ticks: 250 ms at 60 Hz. A
/// client drawing the world further behind than this is judged against
/// the oldest tick kept, which is further back than any playable link
/// draws.
pub(crate) const REWIND_MAX_TICKS: u8 = 15;

/// One enemy tank's hit boxes as `Terrain::sweep` tests them: the hull
/// and the turret, each a centre and half-extents, unpadded.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct TankBoxes {
    pub entity: Entity,
    pub hull: (Position, Position),
    pub turret: (Position, Position),
}

/// The rewindable hit boxes of one tick, as they stood when its update
/// ended - which is what a snapshot of that tick shows a client. Both
/// lists are sorted by entity bits, so a lookup is a binary search and
/// nothing depends on a query's order.
#[derive(Clone, Debug, Default)]
pub(crate) struct HitBoxFrame {
    pub frame: u64,
    pub tanks: Vec<TankBoxes>,
    pub frogs: Vec<(Entity, Position)>,
}

impl HitBoxFrame {
    pub fn tank(&self, entity: Entity) -> Option<&TankBoxes> {
        self.tanks
            .binary_search_by_key(&entity.to_bits(), |t| t.entity.to_bits())
            .ok()
            .map(|i| &self.tanks[i])
    }

    pub fn frog(&self, entity: Entity) -> Option<Position> {
        self.frogs
            .binary_search_by_key(&entity.to_bits(), |&(e, _)| e.to_bits())
            .ok()
            .map(|i| self.frogs[i].1)
    }
}

/// The last `REWIND_MAX_TICKS` ticks of enemy and frog hit boxes, newest
/// last. `Game::update` records one entry at the end of every tick it
/// simulates; a paused tick or the intro banner records none, and a tick
/// with no entry is judged against the present. Recording draws no RNG
/// and changes nothing in the world, so a round that never rewinds plays
/// exactly as it would without it.
#[derive(Debug, Default)]
pub(crate) struct HitBoxHistory {
    frames: VecDeque<HitBoxFrame>,
}

impl HitBoxHistory {
    /// Record the boxes the world holds now as tick `frame`'s: every live
    /// enemy (a tank with an `Ai` - a seat and a tank still rolling in
    /// through a gate carry none) and every frog. The oldest entry's
    /// buffers are reused once the ring is full.
    pub fn record(&mut self, world: &hecs::World, frame: u64) {
        let mut entry = if self.frames.len() >= REWIND_MAX_TICKS as usize {
            self.frames.pop_front().unwrap_or_default()
        } else {
            HitBoxFrame::default()
        };
        entry.frame = frame;
        entry.tanks.clear();
        entry.frogs.clear();
        entry.tanks.extend(
            world
                .query::<(Entity, &Tank)>()
                .with::<&Ai>()
                .iter()
                .filter(|(_, t)| !t.is_wreck())
                .map(|(entity, t)| TankBoxes { entity, hull: t.hull_bbox_world(), turret: t.turret_bbox_world() }),
        );
        entry.tanks.sort_by_key(|t| t.entity.to_bits());
        entry.frogs.extend(world.query::<(Entity, &Frog)>().iter().map(|(e, f)| (e, f.position)));
        entry.frogs.sort_by_key(|&(e, _)| e.to_bits());
        self.frames.push_back(entry);
    }

    /// Tick `frame`'s entry, if the ring still holds it.
    pub fn at(&self, frame: u64) -> Option<&HitBoxFrame> {
        self.frames.iter().rev().find(|e| e.frame == frame)
    }

    pub fn clear(&mut self) {
        self.frames.clear();
    }
}

fn frog_half() -> Position {
    Position::new(FROG_COLLIDER_HALF_EXTENT.0, FROG_COLLIDER_HALF_EXTENT.1)
}

impl Terrain {
    /// Snapshot the world's static terrain. Tiles already flagged
    /// `destroyed` (removed at the end of this frame) are left out.
    pub fn build(world: &hecs::World, width: f32, height: f32, grass: &[Position], water: &crate::ground::WaterLayout) -> Self {
        // Trees are left out: they do not seam-close, here or in physics
        // (`battlefield::tile_half_extent`), so they must not appear as a
        // neighbour that closes somebody else's seam either.
        let cells: HashSet<(i32, i32)> = world
            .query::<&Obstacle>()
            .iter()
            .filter(|o| !o.destroyed && !o.material.is_tree())
            .map(|o| battlefield::pos_to_cell(o.position))
            .collect();
        let obstacles = world
            .query::<(Entity, &Obstacle)>()
            .iter()
            .filter(|(_, o)| !o.destroyed)
            .map(|(entity, o)| {
                let (gx, gy) = battlefield::pos_to_cell(o.position);
                TerrainBox {
                    entity,
                    center: o.position,
                    half: battlefield::tile_half_extent(o.material, &cells, gx, gy, o.hull_size() * 0.5),
                    material: o.material,
                    burning: o.burning,
                }
            })
            .collect();
        let frogs = world
            .query::<(Entity, &Frog)>()
            .iter()
            .map(|(e, f)| (e, f.position))
            .collect();
        Terrain {
            obstacles,
            frogs,
            walls: battlefield::wall_rects(width, height),
            grass: grass.to_vec(),
            water: water.clone(),
            player_shot_pad: tuning().player_shot_hit_pad_px,
        }
    }

    /// This snapshot with another pad on enemy boxes for a player's shot -
    /// a test's way to hold the exact boxes against the padded ones.
    #[cfg(test)]
    pub fn with_player_shot_pad(mut self, pad: f32) -> Self {
        self.player_shot_pad = pad;
        self
    }

    /// Is `p` standing in tall grass?
    ///
    /// A point query against the grass *cells*, not against the drawn
    /// tufts: cover is a property of the ground a tank is on, and testing
    /// the sprites would make being hidden depend on which way the wind
    /// happened to be blowing. Deliberately not part of `line_of_sight` -
    /// grass hides what is *in* it, it does not block sight *through* it.
    /// A field that blocked sight would cut the map in half for the AI,
    /// which is exactly what the probe's `never-arrived` detector exists to
    /// catch.
    pub fn conceals(&self, p: Position) -> bool {
        crate::grass::conceals(&self.grass, p)
    }

    /// The water under `p` (docs/water.md).
    pub fn depth_at(&self, p: Position) -> crate::ground::Depth {
        self.water.depth_at(p)
    }

    /// Every tile's box, as (centre, half-extents) in the snapshot's
    /// order: what a grenade on the ground bounces off
    /// (`grenade::Surroundings::solids`).
    pub fn tile_boxes(&self) -> Vec<(Position, Position)> {
        self.obstacles.iter().map(|b| (b.center, b.half)).collect()
    }

    /// The field's four walls, as (centre, half-extents): what stops a
    /// grenade in the air too (`grenade::Surroundings::edges`).
    pub fn edge_boxes(&self) -> Vec<(Position, Position)> {
        self.walls.to_vec()
    }

    /// The entry fraction (0..1) of the first solid tile along the
    /// segment `p0..p1`, if any - what caps a flame stream's reach. Every
    /// tile counts, sandbags and fences included: a stream does not sail
    /// over a knee-high wall the way a shell can.
    pub fn first_solid_along(&self, p0: Position, p1: Position) -> Option<f32> {
        self.obstacles
            .iter()
            .filter_map(|b| segment_hits_aabb(p0, p1, b.center, b.half))
            .min_by(|a, b| a.total_cmp(b))
    }

    /// The nearest obstacle tile a shot fired from `from` along `dir` (a
    /// unit vector) would strike within `reach` px, and whether it is
    /// already burning - the AI's breach perception (`ai::WallAhead`).
    /// `pad` is the shot's half-extent, as in `sweep`.
    pub fn obstacle_ahead(&self, from: Position, dir: Position, reach: f32, pad: f32) -> Option<(Material, bool)> {
        let to = Position::new(from.x + dir.x * reach, from.y + dir.y * reach);
        let pad = Position::new(pad, pad);
        self.obstacles
            .iter()
            .filter_map(|b| segment_hits_aabb(from, to, b.center, b.half + pad).map(|t| (t, b)))
            .min_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, b)| (b.material, b.burning))
    }

    /// The hit box of one obstacle entity, if it is in this snapshot.
    pub fn obstacle(&self, entity: Entity) -> Option<&TerrainBox> {
        self.obstacles.iter().find(|b| b.entity == entity)
    }

    /// Whether a frog-sized box centred on `p` clears every obstacle tile -
    /// the landing test for the evasive hop (`combat::frog_hop_target`).
    ///
    /// A box-vs-box test against the tiles' own hulls, not a radius from
    /// their centres: a tile is 32 px across and the frog 44 x 32, so the
    /// contact distance is ~38 px. A radial cordon any wider than that
    /// spreads over whole cells of open ground and, on a map with any real
    /// amount of terrain on it, leaves a crowded frog with nowhere at all
    /// to land - which reads as the hop being broken rather than tight.
    pub fn frog_fits(&self, p: Position) -> bool {
        let half = frog_half();
        !self.obstacles.iter().any(|b| {
            (p.x - b.center.x).abs() < b.half.x + half.x && (p.y - b.center.y).abs() < b.half.y + half.y
        })
    }

    /// Whether a zero-width line from `from` to `to` clears every obstacle
    /// and the frog - "could a shot get there". Deliberately not the
    /// pathfinding grid (its cells are inflated by a tank's clearance
    /// margin, far wider than a shell, and reject plenty of clear shots on
    /// a dense map). Blind to tanks (`ai::Brain::friendly_blocks_shot`'s
    /// job) and to the boundary walls (both endpoints are always interior).
    pub fn line_of_sight(&self, from: Position, to: Position) -> bool {
        self.line_of_sight_to_frog(from, to, None)
    }

    /// `line_of_sight` from inside the tile `own`: a tower looking out of
    /// its own cell, whose box every such segment starts in.
    pub fn line_of_sight_from(&self, own: Entity, from: Position, to: Position) -> bool {
        self.obstacles
            .iter()
            .filter(|b| b.entity != own && b.material.blocks_sight())
            .all(|b| segment_hits_aabb(from, to, b.center, b.half).is_none())
            && self.frogs.iter().all(|&(_, p)| segment_hits_aabb(from, to, p, frog_half()).is_none())
    }

    /// `line_of_sight` for a shot aimed *at* the frog `target`: that frog's
    /// own box is not an obstruction (a segment ending at its centre always
    /// enters it), every other frog and tile still is. `None` ignores
    /// nothing - the plain `line_of_sight`. Tiles that don't block sight
    /// (sandbags, fences - `Material::blocks_sight`) are looked over.
    pub fn line_of_sight_to_frog(&self, from: Position, to: Position, target: Option<Entity>) -> bool {
        self.obstacles
            .iter()
            .filter(|b| b.material.blocks_sight())
            .all(|b| segment_hits_aabb(from, to, b.center, b.half).is_none())
            && self
                .frogs
                .iter()
                .filter(|&&(e, _)| Some(e) != target)
                .all(|&(_, p)| segment_hits_aabb(from, to, p, frog_half()).is_none())
    }

    /// `line_of_sight_to_frog` for a hunter deciding whether a shot at the
    /// frog `target` is worth taking: destructible tiles in the way do not
    /// count, since shells that stop short knock them down and open the
    /// line - only Iron (never destroyed) and other frogs obstruct. This
    /// is what lets a hunter shoot its way into a brick bunker instead of
    /// circling it for a line of sight that never comes.
    pub fn line_of_fire_to_frog(&self, from: Position, to: Position, target: Option<Entity>) -> bool {
        self.obstacles
            .iter()
            .filter(|b| b.material.is_permanent())
            .all(|b| segment_hits_aabb(from, to, b.center, b.half).is_none())
            && self
                .frogs
                .iter()
                .filter(|&&(e, _)| Some(e) != target)
                .all(|&(_, p)| segment_hits_aabb(from, to, p, frog_half()).is_none())
    }

    /// The first thing the segment `p0..p1`, inflated by `half_extent` per
    /// side, hits. Every candidate box - each player's hull and turret,
    /// each enemy's hull and turret, the frog, every obstacle tile, the
    /// four walls - is scored by its entry time and the nearest wins, so a
    /// long segment can never skip what it would really have struck first.
    /// A player's shot is tested against an enemy's hull and turret grown
    /// by `player_shot_hit_pad_px` besides - the forgiveness a thumb
    /// needs, on the one target it is aimed at - while enemy shots, the
    /// seats, the frog, tiles and walls keep the exact boxes.
    /// Exact ties go players > enemies > frog > obstacles > walls. The
    /// shooter's own boxes are skipped. `players` is `Game::players()` -
    /// the seats in index order, `None` where a seat holds no tank.
    /// Returns the target plus `t` in `0..=1` along `p0..p1` (so a beam
    /// can be clipped to where it hit). The game itself sweeps through
    /// `sweep_rewound`, which this is with nothing ignored and nothing
    /// rewound.
    #[cfg(test)]
    pub fn sweep(
        &self,
        world: &hecs::World,
        players: [Option<Entity>; crate::MAX_SEATS],
        shooter: Owner,
        p0: Position,
        p1: Position,
        half_extent: f32,
    ) -> Option<(ShellTarget, f32)> {
        self.sweep_rewound(world, players, shooter, p0, p1, half_extent, &[], None)
    }

    /// `sweep`, the one hit test, with two additions. The obstacle tiles
    /// in `ignore` are left out - the ones a projectile already rolled a
    /// pass-over on (`Projectile::passed_over`), so it keeps flying past
    /// them to whatever is behind. And the enemy tanks and the frogs stand
    /// where `past` had them: lag compensation for a seat's shot
    /// (docs/online-coop-prd.md §4.16). Only the boxes move back - who is
    /// a candidate is decided now, so a tank wrecked or gone since is not
    /// hit again, and one with no entry in `past` (it arrived since) is
    /// tested where it stands. The seats, the tiles and the walls are the
    /// present's. `past: None` is the present, box for box.
    #[allow(clippy::too_many_arguments)]
    pub fn sweep_rewound(
        &self,
        world: &hecs::World,
        players: [Option<Entity>; crate::MAX_SEATS],
        shooter: Owner,
        p0: Position,
        p1: Position,
        half_extent: f32,
        ignore: &[Entity],
        past: Option<&HitBoxFrame>,
    ) -> Option<(ShellTarget, f32)> {
        let pad = Position::new(half_extent, half_extent);
        let enemy_pad = match shooter {
            Owner::Player(_) => pad + Position::new(self.player_shot_pad, self.player_shot_pad),
            Owner::Enemy(_) | Owner::Tower { .. } => pad,
        };
        // A tower's shots pass through its own side's tanks
        // (docs/defence-towers-prd.md decision 7); a tank's shots only skip
        // the tank that fired them.
        let skips = |owner: Owner| owner == shooter || (shooter.is_tower() && shooter.same_side(owner));
        let mut best: Option<(f32, u8, ShellTarget)> = None;

        // Wrecks are see-through to gunfire. A hulk kept its full hull and
        // turret boxes here while `combat::apply_hit` threw the damage
        // away, so it was a free bullet sponge: a shot into it was
        // consumed for nothing, and the AI could not see the cover it was
        // getting either (`Terrain::line_of_sight` ignores tanks entirely,
        // and `ai::Brain::friendly_blocks_shot` skips wrecks). Blocking
        // shots that nobody can reason about is the worst of both, so they
        // pass through.
        for player in players.into_iter().flatten() {
            let (owner, wrecked, hull, turret) = with_tank(world, player, |t| {
                (t.owner(), t.is_wreck(), t.hull_bbox_world(), t.turret_bbox_world())
            });
            if !skips(owner) && !wrecked {
                consider_hit(&mut best, segment_hits_aabb(p0, p1, hull.0, hull.1 + pad), 0, ShellTarget::Tank(player));
                consider_hit(&mut best, segment_hits_aabb(p0, p1, turret.0, turret.1 + pad), 0, ShellTarget::Tank(player));
            }
        }

        for (entity, tank) in world.query::<(Entity, &Tank)>().with::<&Ai>().iter() {
            if skips(tank.owner()) || tank.is_wreck() {
                continue;
            }
            let ((hc, hh), (tc, th)) = match past.and_then(|p| p.tank(entity)) {
                Some(then) => (then.hull, then.turret),
                None => (tank.hull_bbox_world(), tank.turret_bbox_world()),
            };
            consider_hit(&mut best, segment_hits_aabb(p0, p1, hc, hh + enemy_pad), 1, ShellTarget::Tank(entity));
            consider_hit(&mut best, segment_hits_aabb(p0, p1, tc, th + enemy_pad), 1, ShellTarget::Tank(entity));
        }

        for &(entity, now) in &self.frogs {
            let pos = past.and_then(|p| p.frog(entity)).unwrap_or(now);
            consider_hit(&mut best, segment_hits_aabb(p0, p1, pos, frog_half() + pad), 2, ShellTarget::Frog(entity));
        }

        for b in &self.obstacles {
            if ignore.contains(&b.entity) {
                continue;
            }
            // A tower fires from above the battlefield, over the low cover
            // its aim already sees past (`Material::blocks_sight`).
            if shooter.is_tower() && !b.material.blocks_sight() {
                continue;
            }
            consider_hit(&mut best, segment_hits_aabb(p0, p1, b.center, b.half + pad), 3, ShellTarget::Obstacle(b.entity));
        }

        for &(center, half) in &self.walls {
            consider_hit(&mut best, segment_hits_aabb(p0, p1, center, half + pad), 4, ShellTarget::Wall);
        }

        best.map(|(t, _, target)| (target, t))
    }
}

impl Terrain {
    /// Every box a gauss rail's slug (docs/gauss-rail.md) along `p0..p1`,
    /// `half_extent` either side, enters - with `sweep_rewound`'s
    /// candidates, boxes, pads and rewind - in order along it, cut after
    /// the first **stopper**: the field's edge, or a permanent tile (iron,
    /// a volcano's cone, a training door), iron only while `iron_stops`
    /// (an overcharged slug cuts it). Each tank and frog appears once, at
    /// the first of its boxes the segment enters; the shooter never does.
    /// Order: entry `t`, then the sweep's rank (seats, enemies, frogs,
    /// tiles, the edge), then owner slot, frog or the tile's cell. Every
    /// box is first held to the segment's own swept box, so a trace costs
    /// a comparison a tile.
    #[allow(clippy::too_many_arguments)]
    pub fn pierce_rewound(
        &self,
        world: &hecs::World,
        players: [Option<Entity>; crate::MAX_SEATS],
        shooter: Owner,
        p0: Position,
        p1: Position,
        half_extent: f32,
        past: Option<&HitBoxFrame>,
        iron_stops: bool,
    ) -> Vec<(ShellTarget, f32)> {
        let pad = Position::new(half_extent, half_extent);
        let enemy_pad = match shooter {
            Owner::Player(_) => pad + Position::new(self.player_shot_pad, self.player_shot_pad),
            Owner::Enemy(_) | Owner::Tower { .. } => pad,
        };
        let (lo, hi) = (
            Position::new(p0.x.min(p1.x) - half_extent, p0.y.min(p1.y) - half_extent),
            Position::new(p0.x.max(p1.x) + half_extent, p0.y.max(p1.y) + half_extent),
        );
        let near = |c: Position, h: Position| c.x + h.x >= lo.x && c.x - h.x <= hi.x && c.y + h.y >= lo.y && c.y - h.y <= hi.y;
        let first = |boxes: &[(Position, Position)]| {
            boxes.iter().filter(|(c, h)| near(*c, *h)).filter_map(|&(c, h)| segment_hits_aabb(p0, p1, c, h)).min_by(f32::total_cmp)
        };
        // (t, rank, tie key, target)
        let mut hits: Vec<(f32, u8, i64, ShellTarget)> = Vec::new();
        for player in players.into_iter().flatten() {
            let (owner, wrecked, hull, turret) = with_tank(world, player, |t| (t.owner(), t.is_wreck(), t.hull_bbox_world(), t.turret_bbox_world()));
            if owner == shooter || wrecked {
                continue;
            }
            if let Some(t) = first(&[(hull.0, hull.1 + pad), (turret.0, turret.1 + pad)]) {
                hits.push((t, 0, owner.slot() as i64, ShellTarget::Tank(player)));
            }
        }
        for (entity, tank) in world.query::<(Entity, &Tank)>().with::<&Ai>().iter() {
            if tank.owner() == shooter || tank.is_wreck() {
                continue;
            }
            let ((hc, hh), (tc, th)) = match past.and_then(|p| p.tank(entity)) {
                Some(then) => (then.hull, then.turret),
                None => (tank.hull_bbox_world(), tank.turret_bbox_world()),
            };
            if let Some(t) = first(&[(hc, hh + enemy_pad), (tc, th + enemy_pad)]) {
                hits.push((t, 1, tank.owner_slot() as i64, ShellTarget::Tank(entity)));
            }
        }
        for (i, &(entity, now)) in self.frogs.iter().enumerate() {
            let pos = past.and_then(|p| p.frog(entity)).unwrap_or(now);
            if let Some(t) = first(&[(pos, frog_half() + pad)]) {
                hits.push((t, 2, i as i64, ShellTarget::Frog(entity)));
            }
        }
        let cell_key = |c: Position| ((c.y / crate::OBSTACLE_GRID_SIZE).floor() as i64) * 65_536 + (c.x / crate::OBSTACLE_GRID_SIZE).floor() as i64;
        let mut stoppers: Vec<f32> = Vec::new();
        for b in &self.obstacles {
            if let Some(t) = first(&[(b.center, b.half + pad)]) {
                hits.push((t, 3, cell_key(b.center), ShellTarget::Obstacle(b.entity)));
                if b.material.is_permanent() && (iron_stops || b.material != Material::Iron) {
                    stoppers.push(t);
                }
            }
        }
        for &(center, half) in &self.walls {
            if let Some(t) = segment_hits_aabb(p0, p1, center, half + pad) {
                hits.push((t, 4, 0, ShellTarget::Wall));
                stoppers.push(t);
            }
        }
        hits.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)));
        let stop = stoppers.into_iter().min_by(f32::total_cmp);
        let mut out = Vec::with_capacity(hits.len());
        for (t, _, _, target) in hits {
            if stop.is_some_and(|s| t > s) {
                break;
            }
            let stopper = match target {
                ShellTarget::Wall => true,
                ShellTarget::Obstacle(e) => self.obstacle(e).is_some_and(|b| b.material.is_permanent() && (iron_stops || b.material != Material::Iron)),
                _ => false,
            };
            out.push((target, t));
            if stopper {
                break;
            }
        }
        out
    }

    /// Where a slug along `p0..p1` stops, as a fraction of it: its first
    /// stopper's entry (`pierce_rewound`'s rule), 1 with none.
    pub fn rail_stop(&self, p0: Position, p1: Position, half_extent: f32, iron_stops: bool) -> f32 {
        let pad = Position::new(half_extent, half_extent);
        self.obstacles
            .iter()
            .filter(|b| b.material.is_permanent() && (iron_stops || b.material != Material::Iron))
            .map(|b| (b.center, b.half + pad))
            .chain(self.walls.iter().map(|&(c, h)| (c, h + pad)))
            .filter_map(|(c, h)| segment_hits_aabb(p0, p1, c, h))
            .min_by(f32::total_cmp)
            .unwrap_or(1.0)
    }
}

/// Keep `*best` as the candidate with the smallest entry time so far, ties
/// broken by `rank` ascending. `hit` is `segment_hits_aabb`'s result.
fn consider_hit(best: &mut Option<(f32, u8, ShellTarget)>, hit: Option<f32>, rank: u8, target: ShellTarget) {
    let Some(t) = hit else { return };
    let better = match best {
        None => true,
        Some((best_t, best_rank, _)) => (t, rank) < (*best_t, *best_rank),
    };
    if better {
        *best = Some((t, rank, target));
    }
}

/// Which axis a shell reflects on to bounce off `hit`, given where it was
/// just before this frame's motion crossed into the box: whichever axis
/// `prev` was more clearly still outside on is the face that was struck.
/// Returns `(reflect_x, reflect_y)`.
pub(super) fn obstacle_reflect_axis(prev: Position, hit: &TerrainBox) -> (bool, bool) {
    let dx = (prev.x - hit.center.x).abs() - hit.half.x;
    let dy = (prev.y - hit.center.y).abs() - hit.half.y;
    if dx > dy { (true, false) } else { (false, true) }
}

/// If the segment `p0..p1` passes through the axis-aligned box at `center`
/// with half-extents `half`, the parametric time `t` (`0..=1`) at which it
/// first enters - `None` if it never does. Slab clipping per axis;
/// `t_enter` starts at `0.0`, so a segment that begins inside the box
/// reports an immediate hit rather than a negative time, and a zero-length
/// segment degenerates to a point-in-box test.
pub(super) fn segment_hits_aabb(p0: Position, p1: Position, center: Position, half: Position) -> Option<f32> {
    let d = Position::new(p1.x - p0.x, p1.y - p0.y);
    let mut t_enter = 0.0f32;
    let mut t_exit = 1.0f32;
    for axis in 0..2 {
        let (p0a, da, min_b, max_b) = if axis == 0 {
            (p0.x, d.x, center.x - half.x, center.x + half.x)
        } else {
            (p0.y, d.y, center.y - half.y, center.y + half.y)
        };
        if da.abs() < f32::EPSILON {
            if p0a < min_b || p0a > max_b {
                return None;
            }
        } else {
            let inv_d = 1.0 / da;
            let (mut t1, mut t2) = ((min_b - p0a) * inv_d, (max_b - p0a) * inv_d);
            if t1 > t2 {
                std::mem::swap(&mut t1, &mut t2);
            }
            t_enter = t_enter.max(t1);
            t_exit = t_exit.min(t2);
            if t_enter > t_exit {
                return None;
            }
        }
    }
    Some(t_enter)
}

#[cfg(test)]
mod shell_sweep_tests {
    use super::*;

    #[test]
    fn stationary_point_inside_box_hits() {
        let p = Position::new(5.0, 5.0);
        assert_eq!(segment_hits_aabb(p, p, Position::new(0.0, 0.0), Position::new(10.0, 10.0)), Some(0.0));
    }

    #[test]
    fn stationary_point_outside_box_misses() {
        let p = Position::new(50.0, 50.0);
        assert_eq!(segment_hits_aabb(p, p, Position::new(0.0, 0.0), Position::new(10.0, 10.0)), None);
    }

    #[test]
    fn fast_pass_through_a_thin_box_is_still_caught() {
        // A shell jumping from well left of a 24px-wide obstacle to well
        // right of it in one step - the case a point check misses.
        let p0 = Position::new(-100.0, 0.0);
        let p1 = Position::new(100.0, 0.0);
        assert!(segment_hits_aabb(p0, p1, Position::new(0.0, 0.0), Position::new(12.0, 12.0)).is_some());
    }

    #[test]
    fn segment_that_never_comes_close_misses() {
        let p0 = Position::new(-100.0, 500.0);
        let p1 = Position::new(100.0, 500.0);
        assert_eq!(segment_hits_aabb(p0, p1, Position::new(0.0, 0.0), Position::new(12.0, 12.0)), None);
    }

    #[test]
    fn diagonal_segment_clipping_a_corner_hits() {
        let p0 = Position::new(-20.0, -20.0);
        let p1 = Position::new(20.0, 20.0);
        assert!(segment_hits_aabb(p0, p1, Position::new(15.0, 15.0), Position::new(3.0, 3.0)).is_some());
    }

    #[test]
    fn parallel_segment_outside_the_slab_misses() {
        // Moves only along X, outside the box's Y slab - the zero-movement
        // branch must reject this rather than divide by zero.
        let p0 = Position::new(-100.0, 100.0);
        let p1 = Position::new(100.0, 100.0);
        assert_eq!(segment_hits_aabb(p0, p1, Position::new(0.0, 0.0), Position::new(12.0, 12.0)), None);
    }

    #[test]
    fn entry_time_orders_two_boxes_on_the_same_segment_by_distance() {
        let p0 = Position::new(0.0, 0.0);
        let p1 = Position::new(1000.0, 0.0);
        let near = segment_hits_aabb(p0, p1, Position::new(100.0, 0.0), Position::new(10.0, 10.0));
        let far = segment_hits_aabb(p0, p1, Position::new(900.0, 0.0), Position::new(10.0, 10.0));
        assert!(near.unwrap() < far.unwrap());
    }

    #[test]
    fn consider_hit_keeps_the_nearer_candidate_regardless_of_call_order() {
        let mut best = None;
        consider_hit(&mut best, Some(0.8), 3, ShellTarget::Obstacle(Entity::DANGLING));
        consider_hit(&mut best, Some(0.2), 4, ShellTarget::Wall);
        assert!(matches!(best, Some((t, _, ShellTarget::Wall)) if t == 0.2));
    }

    #[test]
    fn consider_hit_breaks_an_exact_tie_by_rank() {
        let mut best = None;
        consider_hit(&mut best, Some(0.5), 3, ShellTarget::Obstacle(Entity::DANGLING));
        consider_hit(&mut best, Some(0.5), 1, ShellTarget::Tank(Entity::DANGLING));
        assert!(matches!(best, Some((_, 1, ShellTarget::Tank(_)))));
    }

    #[test]
    fn reflect_axis_picks_the_face_the_shell_came_from() {
        let hit = TerrainBox {
            entity: Entity::DANGLING,
            center: Position::new(0.0, 0.0),
            half: Position::new(16.0, 12.0),
            material: Material::Iron,
            burning: false,
        };
        // Approaching from the left: clearly outside on X, inside on Y.
        assert_eq!(obstacle_reflect_axis(Position::new(-30.0, 2.0), &hit), (true, false));
        // Approaching from above.
        assert_eq!(obstacle_reflect_axis(Position::new(3.0, -30.0), &hit), (false, true));
    }
}
