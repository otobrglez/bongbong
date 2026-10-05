//! What a client draws in the present, tested against the world as it
//! draws it (docs/online-coop-prd.md §4.16).
//!
//! A replica is never updated and the room's hit test never runs on it:
//! every hit is the room's word, and arrives as an event. But stage 4
//! draws some things in the present rather than in the interpolated past
//! - this seat's own shots from the press, everyone else's shots along
//! their path by the local lead - and a shot drawn in the present has to
//! stop where it visibly meets something, or it sails through a wall or a
//! tank for the round trip it takes the room to say so. These are those
//! tests: **presentation only**, over the replica's drawn world - its
//! tiles, the field's edge, every live tank but the shooter's own as it
//! is drawn this frame, the frogs - with the room's lag compensation
//! (the room judges a seat's shots against the enemies as that seat drew
//! them) making the two agree.
//!
//! `Terrain::sweep` cannot serve: it finds enemies by their `Ai`, which a
//! replica strips from every tank it holds.
//!
//! Also here: `ProvisionalShot`'s own flight, which runs the real
//! projectile's state machine - the muzzle frames, then flight, then the
//! impact frames - so a shot drawn on the press leaves the muzzle exactly
//! as the room's copy of it does.

use crate::Position;
use crate::bullet::{Bullet, BulletState};
use crate::laser::LaserVariant;
use crate::math::Vec2;
use crate::plasma::{Plasma, PlasmaState};
use crate::shell::{Owner, Shell, ShellState};
use crate::simulation::hits::Terrain;
use crate::simulation::{Game, ProvisionalKind, ProvisionalShot};
use crate::tank::Tank;
use crate::tuning::tuning;

/// What a present-time shot met first.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Contact {
    /// A live tank: a seat's (`Some`) or an enemy's.
    Tank { seat: Option<u8> },
    Frog,
    Tile,
    /// The field's edge.
    Edge,
}

/// One tank's boxes as drawn this frame.
struct TankBoxes {
    seat: Option<u8>,
    hull: (Position, Position),
    turret: (Position, Position),
}

/// The replica's world as drawn this frame, for present-time hit tests.
/// Built once a frame by `Game::present_world` and asked any number of
/// times.
pub struct PresentWorld {
    terrain: Terrain,
    tanks: Vec<TankBoxes>,
    frogs: Vec<Position>,
    width: f32,
    height: f32,
    /// The portals a shot goes into (`Game::shot_portals`).
    portals: Vec<Position>,
}

impl Game {
    /// The world as this replica draws it this frame: tiles, the field's
    /// edge, every live tank with a body, the frogs.
    pub fn present_world(&self) -> PresentWorld {
        let (width, height) = self.map.field_size();
        let terrain = Terrain::build(&self.world, width, height, &self.grass_cells, &self.water);
        let tanks = self
            .world
            .query::<&Tank>()
            .iter()
            .filter(|t| !t.is_wreck() && t.body.is_some())
            .map(|t| TankBoxes {
                seat: match t.owner() {
                    Owner::Player(seat) => Some(seat),
                    Owner::Enemy(_) | Owner::Tower { .. } => None,
                },
                hull: t.hull_bbox_world(),
                turret: t.turret_bbox_world(),
            })
            .collect();
        let frogs = [self.frog, self.enemy_frog]
            .into_iter()
            .flatten()
            .filter_map(|e| self.world.get::<&crate::frog::Frog>(e).ok().filter(|f| !f.is_dead()).map(|f| f.position))
            .collect();
        PresentWorld { terrain, tanks, frogs, width, height, portals: self.shot_portals().to_vec() }
    }

    /// Kick one seat's hull back from a shot it just fired along
    /// `velocity`, exactly as `weapons::apply_recoil` does in the room:
    /// the kind's recoil speed, normalised to the chassis-free mass and
    /// capped. For a client firing from a hull it owns (`net::predict`),
    /// so the kick lands on the press rather than a round trip later.
    pub fn seat_recoil(&mut self, seat: usize, kind: ProvisionalKind, velocity: Vec2) {
        let Some(entity) = self.seats.get(seat).copied().flatten() else { return };
        let Ok(tank) = self.world.get::<&Tank>(entity) else { return };
        let Some(handle) = tank.body else { return };
        let t = tuning();
        let (speed, max_speed) = match kind {
            ProvisionalKind::Shell => (t.shell_recoil_speed, t.shell_recoil_max_speed),
            ProvisionalKind::Bullet => (t.minigun_bullet_recoil_speed, t.minigun_bullet_recoil_max_speed),
            ProvisionalKind::Plasma => (t.plasma_recoil_speed, t.plasma_recoil_max_speed),
        };
        let len = (velocity.x * velocity.x + velocity.y * velocity.y).sqrt();
        if len <= f32::EPSILON {
            return;
        }
        let reference_mass = tank.scale * tank.scale;
        let push = (speed * reference_mass / tank.mass()).min(max_speed);
        let impulse = push * tank.mass() / len;
        drop(tank);
        self.physics.apply_impulse(handle, Position::new(-velocity.x * impulse, -velocity.y * impulse));
    }

    /// Where one seat's laser would be judged from right now (the gun
    /// line's muzzle), where it would be drawn from (the laser module's
    /// lens), which way, and with which beam: `weapons::laser_shot`'s
    /// geometry with no misfire skew. For a client drawing its own beam on
    /// the press (`net::predict`), off the sandbox's pose.
    pub fn seat_beam(&self, seat: usize) -> Option<(Position, Position, Vec2, LaserVariant)> {
        let entity = self.seats.get(seat).copied().flatten()?;
        let tank = self.world.get::<&Tank>(entity).ok()?;
        let rot = tank.rotation.to_radians();
        let dir = Vec2::new(rot.sin(), -rot.cos());
        let lens = tank.turret_point(crate::tank_art::LASER_MUZZLE[tank.row as usize]);
        Some((tank.gun_line_muzzle(dir), lens, dir, tank.laser_variant))
    }
}

/// A shot of somebody else's in flight, as the replica holds it this
/// frame: what `net::round` carries forward into the present.
#[derive(Clone, Copy, Debug)]
pub struct ForeignShot {
    pub id: u16,
    pub position: Position,
    pub velocity: Vec2,
    pub half_extent: f32,
    /// A shell of the side opposing this seat: the room's
    /// `shell_vs_shell` bursts it and this seat's shells where they meet.
    pub opposing_shell: bool,
}

impl Game {
    /// This seat's shots as the replica holds them this frame - the room's
    /// copies, keyed by the room's id - for `net::predict` to pair with
    /// its provisionals.
    pub fn seat_shots(&self, seat: u8) -> Vec<crate::net::predict::ServerShot> {
        let own = Owner::Player(seat);
        let mut out = Vec::new();
        let base = crate::net::predict::PROVISIONAL_ID_BASE;
        for s in self.world.query::<&Shell>().iter().filter(|s| s.owner == own && s.id < base) {
            out.push(crate::net::predict::ServerShot {
                id: s.id as u16,
                kind: ProvisionalKind::Shell,
                position: s.position,
                velocity: s.velocity,
                flying: s.state == ShellState::Flying,
                impact: matches!(s.state, ShellState::Hit0 | ShellState::Hit1 | ShellState::Hit2),
            });
        }
        for b in self.world.query::<&Bullet>().iter().filter(|b| b.owner == own && b.id < base) {
            out.push(crate::net::predict::ServerShot {
                id: b.id as u16,
                kind: ProvisionalKind::Bullet,
                position: b.position,
                velocity: b.velocity,
                flying: b.state == BulletState::Flying,
                impact: b.state == BulletState::Hit,
            });
        }
        for p in self.world.query::<&Plasma>().iter().filter(|p| p.owner == own && p.id < base) {
            out.push(crate::net::predict::ServerShot {
                id: p.id as u16,
                kind: ProvisionalKind::Plasma,
                position: p.position,
                velocity: p.velocity,
                flying: p.state == PlasmaState::Flying,
                impact: matches!(p.state, PlasmaState::Hit0 | PlasmaState::Hit1 | PlasmaState::Hit2),
            });
        }
        out
    }

    /// Everybody else's shots in flight - every shot not `seat`'s and not
    /// a provisional - as the replica holds them this frame.
    pub fn foreign_flying_shots(&self, seat: u8) -> Vec<ForeignShot> {
        let own = Owner::Player(seat);
        let base = crate::net::predict::PROVISIONAL_ID_BASE;
        let t = tuning();
        let mut out = Vec::new();
        for s in self.world.query::<&Shell>().iter().filter(|s| s.owner != own && s.id < base && s.state == ShellState::Flying) {
            out.push(ForeignShot {
                id: s.id as u16,
                position: s.position,
                velocity: s.velocity,
                half_extent: t.shell_hit_half_extent,
                opposing_shell: !s.owner.same_side(own),
            });
        }
        for b in self.world.query::<&Bullet>().iter().filter(|b| b.owner != own && b.id < base && b.state == BulletState::Flying) {
            out.push(ForeignShot {
                id: b.id as u16,
                position: b.position,
                velocity: b.velocity,
                half_extent: t.minigun_bullet_hit_half_extent,
                opposing_shell: false,
            });
        }
        for p in self.world.query::<&Plasma>().iter().filter(|p| p.owner != own && p.id < base && p.state == PlasmaState::Flying) {
            out.push(ForeignShot {
                id: p.id as u16,
                position: p.position,
                velocity: p.velocity,
                half_extent: t.plasma_hit_half_extent,
                opposing_shell: false,
            });
        }
        out
    }

    /// Whether the replica holds a shot with this id.
    pub fn has_shot(&self, id: u16) -> bool {
        let id = id as u32;
        self.world.query::<&Shell>().iter().any(|s| s.id == id)
            || self.world.query::<&Bullet>().iter().any(|b| b.id == id)
            || self.world.query::<&Plasma>().iter().any(|p| p.id == id)
    }

    /// Draw a shot at `to` this frame, from where it was (presentation
    /// only: `net::apply` puts it back where the room has it next frame).
    pub fn move_shot(&mut self, id: u16, to: Position) {
        let id = id as u32;
        for s in self.world.query_mut::<&mut Shell>().into_iter().filter(|s| s.id == id) {
            s.prev_position = s.position;
            s.position = to;
        }
        for b in self.world.query_mut::<&mut Bullet>().into_iter().filter(|b| b.id == id) {
            b.prev_position = b.position;
            b.position = to;
        }
        for p in self.world.query_mut::<&mut Plasma>().into_iter().filter(|p| p.id == id) {
            p.prev_position = p.position;
            p.position = to;
        }
    }

    /// Take shots off the picture by id, for this frame (presentation
    /// only: `net::apply` puts back what the room still lists).
    pub fn remove_shots(&mut self, ids: &[u16]) {
        if ids.is_empty() {
            return;
        }
        let doomed: Vec<hecs::Entity> = self
            .world
            .query::<(hecs::Entity, hecs::Or<&Shell, hecs::Or<&Bullet, &Plasma>>)>()
            .iter()
            .filter(|(_, shot)| {
                let id = match shot {
                    hecs::Or::Left(s) => s.id,
                    hecs::Or::Right(hecs::Or::Left(b)) => b.id,
                    hecs::Or::Right(hecs::Or::Right(p)) => p.id,
                    hecs::Or::Right(hecs::Or::Both(b, _)) => b.id,
                    hecs::Or::Both(s, _) => s.id,
                };
                id < crate::net::predict::PROVISIONAL_ID_BASE && ids.contains(&(id as u16))
            })
            .map(|(e, _)| e)
            .collect();
        for e in doomed {
            self.world.despawn(e).ok();
        }
    }

    /// Draw a laser beam from `start` to `end` on this replica, fading as
    /// a local round's does (presentation only).
    pub fn draw_beam(&mut self, start: Position, end: Position, variant: LaserVariant) {
        self.laser_beams.push(crate::laser::LaserBeam::new(start, end, variant));
        self.muzzle_flashes.push(crate::shockwave::Shockwave::new(start));
    }

    /// Draw an impact flash at `at` on this replica (presentation only).
    pub fn draw_impact(&mut self, at: Position) {
        self.impact_flashes.push(crate::shockwave::Shockwave::new(at));
    }

    /// Draw a muzzle ripple at `at` on this replica (presentation only):
    /// the one a trigger pull puts at the barrel, for a shot this client
    /// drew leaving it.
    pub fn draw_muzzle(&mut self, at: Position) {
        self.muzzle_flashes.push(crate::shockwave::Shockwave::new(at));
    }

    /// Kick `seat`'s turret on this replica (presentation only): the recoil
    /// a shell or plasma bolt this client drew leaving its main gun puts on
    /// it (`Tank::kick`), on the press rather than a round trip later.
    pub fn kick_seat(&mut self, seat: u8, kind: ProvisionalKind) {
        let Some(entity) = self.seats.get(seat as usize).copied().flatten() else { return };
        let Ok(mut tank) = self.world.get::<&mut Tank>(entity) else { return };
        match kind {
            ProvisionalKind::Shell => tank.kick(false),
            ProvisionalKind::Plasma => tank.kick(true),
            ProvisionalKind::Bullet => {}
        }
    }

    /// Flash `seat`'s laser lens on this replica (presentation only), for
    /// a beam this client drew (`Tank::kick_laser`).
    pub fn flash_seat_laser(&mut self, seat: u8) {
        let Some(entity) = self.seats.get(seat as usize).copied().flatten() else { return };
        if let Ok(mut tank) = self.world.get::<&mut Tank>(entity) {
            tank.kick_laser();
        }
    }
}

impl PresentWorld {
    /// Where a shot `shooter` fired, `half` wide, first meets something
    /// along `p0..p1` - a tank that is not the shooter's (grown by the
    /// player shot's pad against enemies, as the room grows it), a frog, a
    /// tile or the field's edge - or `None` if the segment is clear.
    pub fn shot_contact(&self, shooter: Option<u8>, p0: Position, p1: Position, half: f32) -> Option<(Position, Contact)> {
        let pad = Position::new(half, half);
        let enemy_pad = if shooter.is_some() {
            let p = tuning().player_shot_hit_pad_px;
            pad + Position::new(p, p)
        } else {
            pad
        };
        let mut best: Option<(f32, Contact)> = None;
        let mut consider = |t: Option<f32>, c: Contact| {
            if let Some(t) = t
                && best.is_none_or(|(b, _)| t < b)
            {
                best = Some((t, c));
            }
        };
        for tank in &self.tanks {
            if tank.seat.is_some() && tank.seat == shooter {
                continue;
            }
            // A seat's shot grows the enemies it is aimed at; everything
            // else keeps the exact boxes (`Terrain::sweep`'s rule).
            let grow = if tank.seat.is_none() { enemy_pad } else { pad };
            consider(segment_box(p0, p1, tank.hull.0, tank.hull.1 + grow), Contact::Tank { seat: tank.seat });
            consider(segment_box(p0, p1, tank.turret.0, tank.turret.1 + grow), Contact::Tank { seat: tank.seat });
        }
        let frog_half = Position::new(crate::FROG_COLLIDER_HALF_EXTENT.0, crate::FROG_COLLIDER_HALF_EXTENT.1);
        for &frog in &self.frogs {
            consider(segment_box(p0, p1, frog, frog_half + pad), Contact::Frog);
        }
        consider(self.terrain.first_solid_along(p0, p1), Contact::Tile);
        consider(self.edge_along(p0, p1), Contact::Edge);
        best.map(|(t, c)| (Position::new(p0.x + (p1.x - p0.x) * t, p0.y + (p1.y - p0.y) * t), c))
    }

    /// Where a shot flying `p0..p1`, leaving the portal `leaving` if any,
    /// goes into a portal, the way the room puts it through
    /// (`simulation::portals::shot_entry`). Which portal it comes out of is
    /// the room's draw, so a shot drawn in the present goes no further than
    /// this.
    pub fn portal_entry(&self, p0: Position, p1: Position, leaving: Option<usize>) -> Option<Position> {
        crate::simulation::portals::shot_entry(p0, p1, &self.portals, tuning().portal_shot_radius, leaving).map(|(_, at)| at)
    }

    /// The portal whose swirl `p` stands inside, if any - the one a shot
    /// fired there is leaving (`simulation::portals::inside`).
    pub fn portal_inside(&self, p: Position) -> Option<usize> {
        crate::simulation::portals::inside(p, &self.portals, tuning().portal_shot_radius)
    }


    /// Where `p0..p1` first meets a tile or the field's edge - the statics
    /// only, for a shot drawn ahead of where the room has it.

    pub fn static_contact(&self, p0: Position, p1: Position) -> Option<Position> {
        let t = [self.terrain.first_solid_along(p0, p1), self.edge_along(p0, p1)].into_iter().flatten().min_by(f32::total_cmp)?;
        Some(Position::new(p0.x + (p1.x - p0.x) * t, p0.y + (p1.y - p0.y) * t))
    }

    /// Whether an enemy at `from` sees `to`: the line of sight an enemy
    /// needs before it settles its aim (`Terrain::line_of_sight` - every
    /// tile but the knee-high sandbags and see-through fences, and the
    /// frogs, block it). The off-screen arrows' lane warning asks it
    /// (`indicators.rs`).
    pub fn line_of_sight(&self, from: Position, to: Position) -> bool {
        self.terrain.line_of_sight(from, to)
    }

    /// One seat's hull box as drawn this frame, if it has a live tank.
    pub fn seat_hull(&self, seat: u8) -> Option<(Position, Position)> {
        self.tanks.iter().find(|t| t.seat == Some(seat)).map(|t| t.hull)
    }

    /// Where `p0..p1` leaves the field, as a fraction of it.
    fn edge_along(&self, p0: Position, p1: Position) -> Option<f32> {
        let inside = |p: Position| (0.0..=self.width).contains(&p.x) && (0.0..=self.height).contains(&p.y);
        if !inside(p0) {
            return Some(0.0);
        }
        if inside(p1) {
            return None;
        }
        let mut t = 1.0f32;
        let (dx, dy) = (p1.x - p0.x, p1.y - p0.y);
        for (p, d, lo, hi) in [(p0.x, dx, 0.0, self.width), (p0.y, dy, 0.0, self.height)] {
            if d > 0.0 && p + d > hi {
                t = t.min((hi - p) / d);
            } else if d < 0.0 && p + d < lo {
                t = t.min((lo - p) / d);
            }
        }
        Some(t.clamp(0.0, 1.0))
    }
}

/// The fraction along `p0..p1` at which it enters the box at `center` with
/// half extents `half`, or `None`: the slab test `hits::segment_hits_aabb`
/// is, with a segment starting inside reporting 0.
/// Where two shells moving over the same frame - `a0` to `a1` and `b0` to
/// `b1` - come within `reach` of each other, the room's own rule
/// (`Game::shell_vs_shell`: the closest approach over the frame's motion):
/// the fraction of the frame and the midpoint between them there.
pub fn shells_meet(a0: Position, a1: Position, b0: Position, b1: Position, reach: f32) -> Option<(f32, Position)> {
    let rel = Vec2::new(a0.x - b0.x, a0.y - b0.y);
    let rel_step = Vec2::new((a1.x - a0.x) - (b1.x - b0.x), (a1.y - a0.y) - (b1.y - b0.y));
    let denom = rel_step.x * rel_step.x + rel_step.y * rel_step.y;
    let t = if denom > 0.0 { (-(rel.x * rel_step.x + rel.y * rel_step.y) / denom).clamp(0.0, 1.0) } else { 0.0 };
    let a = Position::new(a0.x + (a1.x - a0.x) * t, a0.y + (a1.y - a0.y) * t);
    let b = Position::new(b0.x + (b1.x - b0.x) * t, b0.y + (b1.y - b0.y) * t);
    let (dx, dy) = (a.x - b.x, a.y - b.y);
    (dx * dx + dy * dy <= reach * reach).then(|| (t, Position::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5)))
}

pub fn segment_box(p0: Position, p1: Position, center: Position, half: Position) -> Option<f32> {
    let (mut t0, mut t1) = (0.0f32, 1.0f32);
    for (p, d, lo, hi) in [
        (p0.x, p1.x - p0.x, center.x - half.x, center.x + half.x),
        (p0.y, p1.y - p0.y, center.y - half.y, center.y + half.y),
    ] {
        if d.abs() < f32::EPSILON {
            if p < lo || p > hi {
                return None;
            }
            continue;
        }
        let (mut near, mut far) = ((lo - p) / d, (hi - p) / d);
        if near > far {
            std::mem::swap(&mut near, &mut far);
        }
        t0 = t0.max(near);
        t1 = t1.min(far);
        if t0 > t1 {
            return None;
        }
    }
    Some(t0)
}

/// The hit half extent a projectile of `kind` is swept with.
pub fn shot_half_extent(kind: ProvisionalKind) -> f32 {
    let t = tuning();
    match kind {
        ProvisionalKind::Shell => t.shell_hit_half_extent,
        ProvisionalKind::Bullet => t.minigun_bullet_hit_half_extent,
        ProvisionalKind::Plasma => t.plasma_hit_half_extent,
    }
}

impl ProvisionalShot {
    /// One frame of flight by the real projectile's own state machine: the
    /// muzzle frames (`Fire0..Fire2` for a shell, 183 ms at the muzzle),
    /// then straight flight, then the impact frames once `detonate`d, then
    /// `done`. Rebuilt from its pose and stepped with `update`, so it can
    /// never disagree with the room's copy about when it leaves.
    pub fn advance(&mut self, dt: f32) {
        if self.done {
            return;
        }
        let owner = Owner::Player(self.seat);
        let before = self.position;
        match self.kind {
            ProvisionalKind::Shell => {
                let mut s = Shell::at(0, self.position, self.prev_position, self.velocity, self.rotation, self.variant, self.shooter_row, owner);
                s.state = ShellState::from_col(self.state).unwrap_or(ShellState::Flying);
                s.timer = self.timer;
                s.update(dt);
                (self.position, self.state, self.timer, self.done) = (s.position, s.state.col(), s.timer, s.done);
            }
            ProvisionalKind::Bullet => {
                let mut b = Bullet::at(0, self.position, self.prev_position, self.velocity, self.rotation, self.shooter_row, owner);
                b.state = BulletState::from_col(self.state).unwrap_or(BulletState::Flying);
                b.timer = self.timer;
                b.update(dt);
                (self.position, self.state, self.timer, self.done) = (b.position, b.state.col(), b.timer, b.done);
            }
            ProvisionalKind::Plasma => {
                let mut p = Plasma::at(0, self.position, self.prev_position, self.velocity, self.rotation, self.plasma_variant, self.shooter_row, owner);
                p.state = PlasmaState::from_col(self.state).unwrap_or(PlasmaState::Flying);
                p.timer = self.timer;
                p.update(dt);
                (self.position, self.state, self.timer, self.done) = (p.position, p.state.col(), p.timer, p.done);
            }
        }
        self.prev_position = before;
    }

    /// Whether it is in flight - past the muzzle frames, not yet an impact.
    pub fn is_flying(&self) -> bool {
        match self.kind {
            ProvisionalKind::Shell => ShellState::from_col(self.state) == Some(ShellState::Flying),
            ProvisionalKind::Bullet => BulletState::from_col(self.state) == Some(BulletState::Flying),
            ProvisionalKind::Plasma => PlasmaState::from_col(self.state) == Some(PlasmaState::Flying),
        }
    }

    /// Whether it is playing its impact frames.
    pub fn is_impact(&self) -> bool {
        !self.done && !self.is_flying() && self.has_left_muzzle()
    }

    /// Whether it has left the muzzle frames.
    fn has_left_muzzle(&self) -> bool {
        match self.kind {
            ProvisionalKind::Shell => self.state >= ShellState::Flying.col(),
            ProvisionalKind::Bullet => self.state >= BulletState::Flying.col(),
            ProvisionalKind::Plasma => self.state >= PlasmaState::Flying.col(),
        }
    }

    /// Stop here and play the impact frames.
    pub fn detonate_at(&mut self, at: Position) {
        self.position = at;
        self.timer = 0.0;
        self.state = match self.kind {
            ProvisionalKind::Shell => ShellState::Hit0.col(),
            ProvisionalKind::Bullet => BulletState::Hit.col(),
            ProvisionalKind::Plasma => PlasmaState::Hit0.col(),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::MapFile;

    fn round() -> Game {
        let mut game = Game::default();
        game.seed_override = Some(0xB0B5);
        game.enemy_count_override = Some(3);
        game.level_overrides.spawn = Some(crate::level::SpawnKind::Band);
        game.map = MapFile::from_toml_str(include_str!("../../maps/default.toml")).expect("the default map parses");
        let (w, h) = game.map.field_size();
        game.init(w, h);
        game
    }

    /// A shot from a seat stops at the first enemy hull in its path, grown
    /// by the player's pad, and never at the shooter's own hull.
    #[test]
    fn a_seats_shot_meets_the_first_enemy_and_not_itself() {
        let mut game = round();
        let seat = game.seat_pose(0).expect("the seat");
        let target = Position::new(seat.position.x + 200.0, seat.position.y);
        game.debug_teleport(1, target, Some(0.0)).expect("an enemy");
        let world = game.present_world();
        let (at, contact) = world
            .shot_contact(Some(0), seat.position, Position::new(seat.position.x + 400.0, seat.position.y), 3.0)
            .expect("the enemy is in the way");
        assert_eq!(contact, Contact::Tank { seat: None });
        assert!(at.x < target.x && at.x > target.x - 60.0, "stopped at the enemy's near face: {at:?} vs {target:?}");
        // Straight through its own hull from the centre: nothing.
        let own = world.shot_contact(Some(0), seat.position, Position::new(seat.position.x + 1.0, seat.position.y), 3.0);
        assert!(own.is_none_or(|(_, c)| c != Contact::Tank { seat: Some(0) }), "a shot never meets its own hull");
    }

    /// Leaving the field is a contact at the edge.
    #[test]
    fn the_field_edge_stops_a_shot() {
        let game = round();
        let world = game.present_world();
        let (w, _) = game.map.field_size();
        let from = Position::new(w - 5.0, 300.0);
        let hit = world.static_contact(from, Position::new(w + 50.0, 300.0)).expect("the edge");
        assert!((hit.x - w).abs() < 0.5 || hit.x <= w, "{hit:?}");
    }

    /// The provisional runs the real state machine: it holds at the muzzle
    /// for the shell's muzzle frames, then flies at the shell's speed.
    #[test]
    fn a_provisional_holds_at_the_muzzle_then_flies() {
        let game = round();
        let mut shot = game.seat_shot(0, ProvisionalKind::Shell, 0.0, 0.0).expect("a shell");
        let start = shot.position;
        shot.advance(0.05);
        assert_eq!(shot.position, start, "still in the muzzle frames");
        assert!(!shot.is_flying());
        for _ in 0..12 {
            shot.advance(1.0 / 60.0);
        }
        assert!(shot.is_flying(), "in flight after the muzzle frames");
        assert!(shot.position != start, "and moving");
        shot.detonate_at(shot.position);
        assert!(shot.is_impact());
        for _ in 0..60 {
            shot.advance(1.0 / 60.0);
        }
        assert!(shot.done, "the impact frames end it");
    }
}
