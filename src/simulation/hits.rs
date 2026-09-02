//! Projectile hit geometry. `Terrain` is a per-frame snapshot of the static
//! things a shot can hit (obstacle tiles with their seams closed, the frog,
//! the four boundary walls) and `Terrain::sweep` is the one hit test every
//! shell, bullet, plasma bolt and laser beam resolves against: a
//! segment-vs-box sweep over the projectile's whole movement this frame,
//! so nothing tunnels through a thin target however large `dt` was and
//! nothing threads the seam between two touching wall tiles. Rapier is not
//! involved - projectiles have no physics body.

use std::collections::HashSet;

use hecs::Entity;

use crate::ai::Ai;
use crate::battlefield;
use crate::frog::Frog;
use crate::obstacle::{Material, Obstacle};
use crate::shell::Owner;
use crate::tank::Tank;
use crate::{FROG_COLLIDER_HALF_EXTENT, Position};

use super::with_tank;

/// What a projectile or beam hit. Read-only: the caller applies the effect
/// (see `Game::apply_hit`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ShellTarget {
    PlayerTank,
    EnemyTank(Entity),
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
}

/// Static terrain snapshot for one frame - see the module doc. Built once
/// per `Game::update`, after the frog's hop tick, and shared by the AI's
/// line-of-sight checks, engagement-slot validation and every hit test.
pub(crate) struct Terrain {
    obstacles: Vec<TerrainBox>,
    frogs: Vec<(Entity, Position)>,
    walls: [(Position, Position); 4],
}

fn frog_half() -> Position {
    Position::new(FROG_COLLIDER_HALF_EXTENT.0, FROG_COLLIDER_HALF_EXTENT.1)
}

impl Terrain {
    /// Snapshot the world's static terrain. Tiles already flagged
    /// `destroyed` (removed at the end of this frame) are left out.
    pub fn build(world: &hecs::World, width: f32, height: f32) -> Self {
        let cells: HashSet<(i32, i32)> = world
            .query::<&Obstacle>()
            .iter()
            .filter(|o| !o.destroyed)
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
                    half: battlefield::tile_hull_half_extent(&cells, gx, gy, o.hull_size() * 0.5),
                    material: o.material,
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
        }
    }

    /// The hit box of one obstacle entity, if it is in this snapshot.
    pub fn obstacle(&self, entity: Entity) -> Option<&TerrainBox> {
        self.obstacles.iter().find(|b| b.entity == entity)
    }

    /// Every obstacle tile's center - clearance checks for the frog's hop
    /// landing spot (see `combat::frog_hop_target`).
    pub fn obstacle_centers(&self) -> Vec<Position> {
        self.obstacles.iter().map(|b| b.center).collect()
    }

    /// Whether a zero-width line from `from` to `to` clears every obstacle
    /// and the frog - "could a shot get there". Deliberately not the
    /// pathfinding grid (its cells are inflated by a tank's clearance
    /// margin, far wider than a shell, and reject plenty of clear shots on
    /// a dense map). Blind to tanks (`ai::Brain::friendly_blocks_shot`'s
    /// job) and to the boundary walls (both endpoints are always interior).
    pub fn line_of_sight(&self, from: Position, to: Position) -> bool {
        self.obstacles
            .iter()
            .all(|b| segment_hits_aabb(from, to, b.center, b.half).is_none())
            && self
                .frogs
                .iter()
                .all(|&(_, p)| segment_hits_aabb(from, to, p, frog_half()).is_none())
    }

    /// The first thing the segment `p0..p1`, inflated by `half_extent` per
    /// side, hits. Every candidate box - the player's hull and turret, each
    /// enemy's hull and turret, the frog, every obstacle tile, the four
    /// walls - is scored by its entry time and the nearest wins, so a long
    /// segment can never skip what it would really have struck first.
    /// Exact ties go player > enemies > frog > obstacles > walls. The
    /// shooter's own boxes are skipped. Returns the target plus `t` in
    /// `0..=1` along `p0..p1` (so a beam can be clipped to where it hit).
    pub fn sweep(
        &self,
        world: &hecs::World,
        player: Entity,
        shooter: Owner,
        p0: Position,
        p1: Position,
        half_extent: f32,
    ) -> Option<(ShellTarget, f32)> {
        let pad = Position::new(half_extent, half_extent);
        let mut best: Option<(f32, u8, ShellTarget)> = None;

        let (player_owner, hull, turret) = with_tank(world, player, |t| {
            (t.owner(), t.hull_bbox_world(), t.turret_bbox_world())
        });
        if player_owner != shooter {
            consider_hit(&mut best, segment_hits_aabb(p0, p1, hull.0, hull.1 + pad), 0, ShellTarget::PlayerTank);
            consider_hit(&mut best, segment_hits_aabb(p0, p1, turret.0, turret.1 + pad), 0, ShellTarget::PlayerTank);
        }

        for (entity, tank) in world.query::<(Entity, &Tank)>().with::<&Ai>().iter() {
            if tank.owner() == shooter {
                continue;
            }
            let (hc, hh) = tank.hull_bbox_world();
            consider_hit(&mut best, segment_hits_aabb(p0, p1, hc, hh + pad), 1, ShellTarget::EnemyTank(entity));
            let (tc, th) = tank.turret_bbox_world();
            consider_hit(&mut best, segment_hits_aabb(p0, p1, tc, th + pad), 1, ShellTarget::EnemyTank(entity));
        }

        for &(entity, pos) in &self.frogs {
            consider_hit(&mut best, segment_hits_aabb(p0, p1, pos, frog_half() + pad), 2, ShellTarget::Frog(entity));
        }

        for b in &self.obstacles {
            consider_hit(&mut best, segment_hits_aabb(p0, p1, b.center, b.half + pad), 3, ShellTarget::Obstacle(b.entity));
        }

        for &(center, half) in &self.walls {
            consider_hit(&mut best, segment_hits_aabb(p0, p1, center, half + pad), 4, ShellTarget::Wall);
        }

        best.map(|(t, _, target)| (target, t))
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
        consider_hit(&mut best, Some(0.5), 1, ShellTarget::EnemyTank(Entity::DANGLING));
        assert!(matches!(best, Some((_, 1, ShellTarget::EnemyTank(_)))));
    }

    #[test]
    fn reflect_axis_picks_the_face_the_shell_came_from() {
        let hit = TerrainBox {
            entity: Entity::DANGLING,
            center: Position::new(0.0, 0.0),
            half: Position::new(16.0, 12.0),
            material: Material::Iron,
        };
        // Approaching from the left: clearly outside on X, inside on Y.
        assert_eq!(obstacle_reflect_axis(Position::new(-30.0, 2.0), &hit), (true, false));
        // Approaching from above.
        assert_eq!(obstacle_reflect_axis(Position::new(3.0, -30.0), &hit), (false, true));
    }
}
