//! The sonic hammer's world half (docs/sonic-hammer.md; `sonic.rs` is the
//! cone, the wave and the drawing): a press queues a blast, `resolve_sonic`
//! casts it into a wave once the tank loops are done, and
//! `tick_sonic_waves` strikes whatever the wave's front passes, once -
//! hulls knocked off their tracks (`knock`), glass shattered, grass
//! flattened, drums thrown, frogs stunned, lanterns broken, grenades
//! pushed. Also the general pieces later weapons share: `knock`, the
//! cover a flattened cell leaves (`Game::cover_cells`) and the hashed
//! spawn swap (`swap_spawn_special`). No RNG anywhere here.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use hecs::Entity;

use crate::ai::{Ai, HammerAim, HammerSense, Role};
use crate::frog::Frog;
use crate::math::Vec2;
use crate::obstacle::Obstacle;
use crate::physics::Physics;
use crate::shell::Owner;
use crate::shockwave::Shockwave;
use crate::sonic::{self, Block, Floor, SonicCone, SonicWave};
use crate::tank::{ActiveWeapon, Dir, Tank};
use crate::tuning::{Tuning, tuning};
use crate::{MAX_DAMAGE, OBSTACLE_GRID_SIZE, Position};

use super::props::DamageCause;
use super::{Event, Footing, Frame, Game, HitCause, HitTarget, RollIn, Spectacle};

/// A blast fired this frame, waiting for `resolve_sonic` to cast it.
pub(super) struct PendingSonic {
    pub origin: Position,
    pub facing: Dir,
    pub owner: Owner,
}

/// Fire one blast of `tank`'s sonic hammer: the dish's firing cell, a kick
/// back along the facing (not on `Frame::shoves` - a client that owns its
/// hull kicks it on the press, `Game::seat_kick`), `Event::SonicBlast`
/// right after the dispatch's `Fired`, and the blast queued for
/// `resolve_sonic`. No RNG.
pub(super) fn fire_sonic(physics: &mut Physics, f: &mut Frame, tank: &mut Tank, owner: Owner) {
    let t = tuning();
    let facing = Dir::from_rotation(tank.rotation).unwrap_or(Dir::Up);
    tank.kick_sonic();
    super::weapons::apply_recoil(physics, tank, facing.vec(), t.sonic_recoil_speed, t.sonic_recoil_max_speed);
    let origin = tank.position;
    f.events.push(Event::SonicBlast { slot: tank.owner_slot(), x: origin.x, y: origin.y, dir: facing.name() });
    f.pending_sonic.push(PendingSonic { origin, facing, owner });
}

/// Knock `tank` along the unit `dir` at `speed` px/s and off its tracks
/// (docs/sonic-hammer.md "The knock"): a real impulse sized by its own
/// mass, then `Tank::skid` for as long as the skid's friction takes to
/// stop it on its ground (`footing`), and the speed it was left at for the
/// pose validator. A client-owned seat hears of it as `Event::Shoved` with
/// its skid. Returns the velocity change. Any weapon that throws hulls
/// about knocks them through here.
pub(super) fn knock(physics: &mut Physics, f: &mut Frame, tank: &mut Tank, dir: Vec2, speed: f32, footing: Footing) -> Vec2 {
    knock_with(physics, f, tank, dir, speed, footing, true)
}

/// `knock`, saying whether a client-owned seat hears of it (`echo`): a
/// knock its client puts on itself - a gauss rail's recoil, kicked on the
/// release - is not sent, but the pose validator still allows its speed
/// (`Shoves::allow_knock`).
pub(super) fn knock_with(physics: &mut Physics, f: &mut Frame, tank: &mut Tank, dir: Vec2, speed: f32, footing: Footing, echo: bool) -> Vec2 {
    let Some((dv, skid)) = knock_hull(physics, tank, dir, speed, footing) else { return Vec2::zero() };
    if echo {
        f.shoves.push_knock(tank.owner(), dv, skid);
    } else {
        f.shoves.allow_knock(tank.owner(), dv, skid);
    }
    dv
}

/// The knock itself, with nobody told (`knock`): the impulse sized by the
/// hull's own mass, then its skid and the speed it was left at. The
/// velocity change and the skid; `None` for a hull with no body. What a
/// client's sandbox kicks its own hull with too (`Game::predict_seat_with`).
pub(super) fn knock_hull(physics: &mut Physics, tank: &mut Tank, dir: Vec2, speed: f32, footing: Footing) -> Option<(Vec2, f32)> {
    let handle = tank.body?;
    let t = tuning();
    let dv = dir * speed;
    physics.apply_impulse(handle, Position::new(dv.x * tank.mass(), dv.y * tank.mass()));
    let v = physics.velocity(handle);
    let rel = Vec2::new(v.x - footing.flow.x, v.y - footing.flow.y).length();
    let skid = sonic::skid_seconds(&t, rel, footing.grip);
    tank.skid = tank.skid.max(skid);
    tank.skid_speed = rel;
    Some((dv, skid))
}

/// The centre and the four corners of a hull's box: the points a wave
/// reaches it by, the nearest first.
fn hull_points(tank: &Tank) -> [Position; 5] {
    let (c, h) = tank.hull_bbox_world();
    box_points(c, h)
}

/// The centre `c` and the four corners of a box `h` either way of it.
fn box_points(c: Position, h: Vec2) -> [Position; 5] {
    [
        c,
        Position::new(c.x - h.x, c.y - h.y),
        Position::new(c.x + h.x, c.y - h.y),
        Position::new(c.x - h.x, c.y + h.y),
        Position::new(c.x + h.x, c.y + h.y),
    ]
}

/// How far behind the front (px) a hull may be found and still be struck:
/// one driving into the wave crosses the front between two frames.
const FRONT_SLACK_PX: f32 = 8.0;

/// The `Spawn` table of the BB-36 weapons' hashed swap (docs/sonic-hammer.md
/// "The probe's `--crate` and the spawn swap"): each weapon and the knob
/// that is its share, in order.
type ShareOf = fn(&Tuning) -> f32;
const SPAWN_SWAPS: [(ActiveWeapon, ShareOf); 3] = [
    (ActiveWeapon::SonicHammer, |t| t.enemy_special_weapon_sonic_share),
    (ActiveWeapon::Emp, |t| t.enemy_special_weapon_emp_share),
    (ActiveWeapon::GaussRail, |t| t.enemy_special_weapon_gauss_share),
];

/// The salt of the spawn swap's hash.
const SWAP_SALT: u32 = 0x5A_A9;

/// Swap the special an enemy spawning in owner slot `slot` at `spawn` drew
/// for one of the BB-36 weapons: for each entry of `SPAWN_SWAPS` with a
/// share above 0, a hash of the spawn point, the slot and the entry's place
/// - never a draw from the round's RNG - under the share takes it up
/// (`Tank::take_weapon`) and ends the walk. A tank that drew no special
/// keeps none. At the defaults (every share 0) nothing changes.
pub(super) fn swap_spawn_special(enemy: &mut Tank, slot: usize, spawn: Position) {
    swap_spawn_special_with(&tuning(), enemy, slot, spawn);
}

/// `swap_spawn_special` under the table `t`.
pub(super) fn swap_spawn_special_with(t: &Tuning, enemy: &mut Tank, slot: usize, spawn: Position) {
    if enemy.special().is_none() {
        return;
    }
    let seed = crate::blast::seed_at(spawn, SWAP_SALT ^ slot as u32);
    for (i, (weapon, share)) in SPAWN_SWAPS.iter().enumerate() {
        let share = share(t);
        if share > 0.0 && crate::pyro::unit(seed, i as u32) < share {
            enemy.take_weapon(*weapon);
            return;
        }
    }
}

/// Where each closer stands to shout at the seat it fights
/// (`HammerSense::spot`, docs/sonic-hammer.md "AI"). For every live seat,
/// the `sonic_ai_closers` hammer tanks fighting it nearest it (ties on
/// slot) are its closers; the seat's spots are the four on its row and
/// column `sonic_ai_breaker_px` less half a cell out, and a spot holds
/// where a hull can be routed (`Grid::usable`), a blast from it at the
/// seat reaches the seat's hull within `sonic_ai_breaker_px`, no tank but
/// a closer stands on it and no fellow enemy but a closer stands in that
/// blast's cone. In slot order, each closer takes the spot that holds
/// nearest it, reachable from where it stands, where neither its blast nor
/// a blast of a closer before it would reach the other's hull - at the
/// defaults the spot across the seat from the first - so the two never
/// stand in each other's way; one that finds none is left to the tree.
/// `armed` is every live hammer tank healthy enough to close in (entity,
/// owner slot, position, the seat it fights), `half_of` a hammer tank's hull half extents, `tanks`
/// every live tank with a body by slot and `friends` every live enemy by
/// slot with its hull points. No RNG.
#[allow(clippy::too_many_arguments)]
pub(super) fn closer_spots(
    t: &Tuning,
    armed: &[(Entity, usize, Position, u8)],
    half_of: impl Fn(Entity) -> Vec2,
    seats: &[HammerSeat],
    grid: &crate::pathfind::Grid,
    field: (f32, f32),
    tanks: &[(usize, Position)],
    friends: &[(usize, [Position; 6])],
    block: impl Fn((i32, i32)) -> Block,
) -> BTreeMap<Entity, Position> {
    let mut out = BTreeMap::new();
    if t.sonic_ai_closers <= 0 {
        return out;
    }
    let out_by = (t.sonic_ai_breaker_px - OBSTACLE_GRID_SIZE * 0.5).max(CLOSER_SPOT_MIN_PX);
    for seat in seats.iter().filter(|s| s.live) {
        let mut closers: Vec<(Entity, usize, Position)> = armed.iter().filter(|a| a.3 == seat.seat).map(|a| (a.0, a.1, a.2)).collect();
        closers.sort_by(|a, b| a.2.distance_to(seat.pos).total_cmp(&b.2.distance_to(seat.pos)).then(a.1.cmp(&b.1)));
        closers.truncate(t.sonic_ai_closers as usize);
        if closers.is_empty() {
            continue;
        }
        closers.sort_by_key(|c| c.1);
        let closing = |slot: usize| closers.iter().any(|c| c.1 == slot);
        let spots: Vec<(Position, SonicCone)> = Dir::ALL
            .into_iter()
            .filter_map(|side| {
                let at = seat.pos + side.vec() * out_by;
                if !grid.usable(at) {
                    return None;
                }
                let facing = Dir::toward(at, seat.pos);
                let cone = SonicCone::cast(at, facing, t.sonic_reach_px, t.sonic_half_angle_deg.to_radians(), field, &block, |_| Floor::Dry);
                let reaches = cone.nearest_reached(&seat.points).is_some_and(|d| d <= t.sonic_ai_breaker_px);
                let stood_on = tanks.iter().any(|&(slot, p)| !closing(slot) && p.distance_to(at) < CLOSER_SPOT_CLEAR_PX);
                let friend = friends.iter().any(|(slot, points)| !closing(*slot) && cone.nearest_reached(&points[..5]).is_some());
                (reaches && !stood_on && !friend).then_some((at, cone))
            })
            .collect();
        let mut taken: Vec<(Position, Vec2, &SonicCone)> = Vec::new();
        for (entity, _, me) in closers {
            let half = half_of(entity);
            let clear = |at: Position, cone: &SonicCone| {
                taken.iter().all(|&(other, other_half, other_cone)| {
                    other != at && cone.nearest_reached(&box_points(other, other_half)).is_none() && other_cone.nearest_reached(&box_points(at, half)).is_none()
                })
            };
            let pick = spots
                .iter()
                .filter(|(at, cone)| clear(*at, cone) && grid.connected(me, *at))
                .min_by(|a, b| me.distance_to(a.0).total_cmp(&me.distance_to(b.0)));
            if let Some((spot, cone)) = pick {
                taken.push((*spot, half, cone));
                out.insert(entity, *spot);
            }
        }
    }
    out
}

/// The least a closer's spot stands off its seat (px): a hull's width and
/// a half, whatever `sonic_ai_breaker_px` says.
const CLOSER_SPOT_MIN_PX: f32 = OBSTACLE_GRID_SIZE * 1.5;

/// How near another tank may stand to a closer's spot before it no longer
/// holds (px): about a hull's width.
const CLOSER_SPOT_CLEAR_PX: f32 = OBSTACLE_GRID_SIZE * 1.25;

impl Game {
    /// The cells `Terrain` and the indicators take as hiding a hull: the
    /// tall grass, less the cells a sonic wave flattened a while ago
    /// (`grass_flat`). The grass itself while nothing is flattened.
    pub fn cover_cells(&self) -> Cow<'_, [Position]> {
        if self.grass_flat.is_empty() {
            return Cow::Borrowed(&self.grass_cells);
        }
        Cow::Owned(self.grass_cells.iter().copied().filter(|&c| !self.grass_flat.contains_key(&crate::map::world_to_cell(c))).collect())
    }

    /// How each map cell answers a sonic wave, from the tiles standing now.
    fn sound_blocks(&self) -> BTreeMap<(i32, i32), Block> {
        self.world
            .query::<&Obstacle>()
            .iter()
            .filter(|o| !o.destroyed && o.material.blocks_sound())
            .map(|o| (o.cell(), if o.material.breaks_by_sound() { Block::Glass } else { Block::Wall }))
            .collect()
    }

    /// The cone a blast from `origin` along `facing` is cast as over this
    /// round's tiles, water and lava.
    pub(crate) fn sonic_cone(&self, origin: Position, facing: Dir) -> SonicCone {
        let t = tuning();
        let blocks = self.sound_blocks();
        let floor = |cell: (i32, i32)| {
            let at = crate::map::cell_to_world(cell.0, cell.1);
            if self.lava.depth_at(at) != crate::ground::Depth::Dry {
                Floor::Lava
            } else if self.water.depth_at(at).is_wet() {
                Floor::Water
            } else {
                Floor::Dry
            }
        };
        SonicCone::cast(
            origin,
            facing,
            t.sonic_reach_px,
            t.sonic_half_angle_deg.to_radians(),
            self.map.field_size(),
            |cell| blocks.get(&cell).copied().unwrap_or(Block::Open),
            floor,
        )
    }

    /// The show of a blast from `origin` along `facing` by `owner`: its wave
    /// on the field and a weak ripple. What the round puts on for a press,
    /// a replica for the room's `SonicBlast` and a client for its own press
    /// (`draw_press_show`).
    pub(crate) fn sonic_show(&mut self, show: &mut Spectacle, origin: Position, facing: Dir, owner: Owner) {
        let t = tuning();
        let cone = self.sonic_cone(origin, facing);
        self.sonic_waves.push(SonicWave::new(cone, owner));
        if t.sonic_shock > 0.0 {
            show.shocks.push(Shockwave::scaled(origin, t.sonic_shock));
        }
    }

    /// Cast this frame's blasts into waves.
    pub(super) fn resolve_sonic(&mut self, f: &mut Frame) {
        let pending: Vec<PendingSonic> = f.pending_sonic.drain(..).collect();
        for p in pending {
            let mut show = Spectacle::default();
            self.sonic_show(&mut show, p.origin, p.facing, p.owner);
            f.stage(show);
        }
    }

    /// Run every wave's front on by the frame and strike what it passes;
    /// drop the waves whose picture is done. `live` is false on the end
    /// screen, where the waves only finish their picture.
    pub(super) fn tick_sonic_waves(&mut self, f: &mut Frame, live: bool) {
        if self.sonic_waves.is_empty() {
            return;
        }
        let t = tuning();
        let mut waves = std::mem::take(&mut self.sonic_waves);
        for wave in waves.iter_mut() {
            wave.age += f.dt;
            if wave.swept > wave.cone.longest() {
                continue;
            }
            let (from, to) = (wave.swept, wave.front(&t));
            self.flatten_swept(wave, from, to);
            if live {
                self.strike(f, wave, from, to, &t);
            }
            wave.swept = to;
        }
        waves.retain(|w| !w.done(&t));
        waves.append(&mut self.sonic_waves);
        self.sonic_waves = waves;
    }

    /// A replica's waves (docs/sonic-hammer.md "Wire"): age them, flatten
    /// the grass the front passes in the picture, drop the done ones. A
    /// round that simulates its waves never calls this.
    pub(crate) fn tick_sonic_pictures(&mut self, dt: f32) {
        if self.sonic_waves.is_empty() {
            return;
        }
        let t = tuning();
        let mut waves = std::mem::take(&mut self.sonic_waves);
        for wave in waves.iter_mut() {
            wave.age += dt;
            if wave.swept <= wave.cone.longest() {
                let (from, to) = (wave.swept, wave.front(&t));
                self.flatten_swept(wave, from, to);
                wave.swept = to;
            }
        }
        waves.retain(|w| !w.done(&t));
        waves.append(&mut self.sonic_waves);
        self.sonic_waves = waves;
    }

    /// The tall grass in the cells `wave`'s front entered between `from`
    /// and `to`: it hides nobody for `sonic_grass_flat_seconds`
    /// (`grass_flat`) and its tufts lie flat as long (`grass::pin`).
    fn flatten_swept(&mut self, wave: &SonicWave, from: f32, to: f32) {
        if self.grass_cells.is_empty() {
            return;
        }
        let seconds = tuning().sonic_grass_flat_seconds;
        if seconds <= 0.0 {
            return;
        }
        let grass: BTreeSet<(i32, i32)> = self.grass_cells.iter().map(|&c| crate::map::world_to_cell(c)).collect();
        for c in wave.cone.cells.iter().filter(|c| c.at >= from && c.at <= to && grass.contains(&c.cell)) {
            let left = self.grass_flat.entry(c.cell).or_insert(0.0);
            *left = left.max(seconds);
            crate::grass::pin(&mut self.grass, c.cell, seconds);
        }
    }

    /// Everything `wave` reaches between the front's radii `from` and `to`,
    /// struck once, in a fixed order: panes, drums, the hulls (the seats in
    /// index order, then the enemies by slot), the frogs, the lanterns, the
    /// grenades.
    fn strike(&mut self, f: &mut Frame, wave: &mut SonicWave, from: f32, to: f32, t: &Tuning) {
        let band = |d: f32| d >= from && d <= to;
        let shooter = wave.owner;
        // Panes shatter, along the wave's line.
        let panes: Vec<(i32, i32)> = wave.cone.panes.iter().filter(|p| band(p.at)).map(|p| p.cell).collect();
        if !panes.is_empty() {
            let glass: Vec<(Entity, (i32, i32), Position, f32)> = self
                .world
                .query::<(Entity, &Obstacle)>()
                .iter()
                .filter(|(_, o)| !o.destroyed && o.material.breaks_by_sound() && panes.contains(&o.cell()))
                .map(|(e, o)| (e, o.cell(), o.position, o.max_health))
                .collect();
            for cell in panes {
                for &(e, c, at, health) in glass.iter().filter(|g| g.1 == cell) {
                    let _ = c;
                    let dir = (at - wave.cone.origin) / at.distance_to(wave.cone.origin).max(1e-3);
                    self.damage_obstacle(f, e, health.max(1.0), DamageCause::Shot { dir: Some(dir) });
                }
            }
        }
        // Drums are thrown.
        let cells: Vec<((i32, i32), f32)> = wave.cone.cells.iter().filter(|c| band(c.at)).map(|c| (c.cell, c.at)).collect();
        if !cells.is_empty() {
            // In the order the cone entered their cells (entry distance,
            // then cell), whatever order the world keeps them in.
            let mut drums: Vec<(f32, (i32, i32), Entity, Position, i32)> = self
                .world
                .query::<(Entity, &Obstacle)>()
                .iter()
                .filter(|(_, o)| !o.destroyed && o.material.is_explosive())
                .filter_map(|(e, o)| cells.iter().find(|c| c.0 == o.cell()).map(|c| (c.1, o.cell(), e, o.position, o.variant)))
                .collect();
            drums.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
            for (_, _, entity, at, variant) in drums {
                self.throw_drum(f, wave, entity, at, variant, t);
            }
        }
        // Hulls: seats in index order, then the enemies by slot.
        let mut hulls: Vec<(Entity, usize)> =
            self.seats_on_field().into_iter().flatten().map(|e| (e, super::with_tank(&self.world, e, |t| t.owner_slot()))).collect();
        let mut enemies: Vec<(Entity, usize)> =
            self.world.query::<(Entity, &Tank)>().with::<&Ai>().iter().map(|(e, t)| (e, t.owner_slot())).collect();
        enemies.sort_by_key(|&(_, slot)| slot);
        hulls.extend(enemies);
        for (entity, slot) in hulls {
            if wave.struck.contains(&slot) {
                continue;
            }
            let reached = {
                let Ok(tank) = self.world.get::<&Tank>(entity) else { continue };
                if tank.is_wreck() || tank.body.is_none() || tank.owner() == shooter || self.world.get::<&RollIn>(entity).is_ok() {
                    continue;
                }
                wave.cone.nearest_reached(&hull_points(&tank)).filter(|&d| d <= to && d >= from - FRONT_SLACK_PX)
            };
            if let Some(d) = reached {
                wave.struck.push(slot);
                self.strike_hull(f, entity, d, wave.cone.origin, shooter, t);
            }
        }
        // Frogs are stunned.
        for frog in self.world.query_mut::<&mut Frog>() {
            if frog.is_dead() {
                continue;
            }
            if wave.cone.reaches(frog.position).is_some_and(|d| d <= to && d >= from - FRONT_SLACK_PX) {
                frog.stun(t.sonic_frog_stun_seconds);
            }
        }
        // A seat's lanterns are broken, as a blast breaks them.
        let mut broken = Vec::new();
        self.lanterns.retain(|l| {
            let hit = wave.cone.reaches(l.position).is_some_and(band);
            if hit {
                broken.push(l.position);
            }
            !hit
        });
        for at in broken {
            f.events.push(Event::LanternBroken { x: at.x, y: at.y });
        }
        // Grenades are pushed along the wave's line, each once.
        let origin = wave.cone.origin;
        for g in self.world.query_mut::<&mut crate::grenade::Grenade>() {
            let Some(d) = wave.cone.reaches(g.position).filter(|&d| band(d)) else { continue };
            let dir = (g.position - origin) / d.max(1e-3);
            g.velocity += dir * (t.sonic_grenade_push_speed * sonic::falloff(t, d));
        }
    }

    /// One hull the front reached at `d` px from `origin`: the side
    /// opposing `shooter` takes `sonic_damage` times the falloff (no roll,
    /// through a shield first) and logs a sonic `Hit`; a hull it does not
    /// kill is knocked along the line from the pivot (`knock`) and, an
    /// enemy, told it was hit.
    fn strike_hull(&mut self, f: &mut Frame, entity: Entity, d: f32, origin: Position, shooter: Owner, t: &Tuning) {
        let footing = super::with_tank(&self.world, entity, |tank| Footing::at(&self.water, &self.lava, self.weather, tank.position, self.time));
        let survived = {
            let Ok(mut tank) = self.world.get::<&mut Tank>(entity) else { return };
            let falloff = sonic::falloff(t, d);
            if !shooter.same_side(tank.owner()) {
                let landed = tank.take_damage(t.sonic_damage * falloff, MAX_DAMAGE);
                tank.mark_hit();
                tank.credit(shooter);
                let killed = tank.is_wreck();
                let target = match tank.owner() {
                    Owner::Player(player) => HitTarget::Player { player },
                    Owner::Enemy(slot) => HitTarget::Enemy { slot },
                    Owner::Tower { .. } => unreachable!("no tank is owned by a tower"),
                };
                let at = tank.position;
                f.events.push(Event::Hit { target, damage: landed, killed, x: at.x, y: at.y, cause: HitCause::Sonic });
                if killed {
                    f.kills.push((tank.position, tank.owner()));
                    return;
                }
            }
            let line = tank.position - origin;
            let len = line.length();
            let dir = if len > 1e-3 { line / len } else { Dir::from_rotation(tank.rotation).unwrap_or(Dir::Up).vec() };
            let mass_factor = tank.mass() / (tank.scale * tank.scale);
            let speed = sonic::shove_speed(t, mass_factor, d);
            knock(&mut self.physics, f, &mut tank, dir, speed, footing);
            !shooter.same_side(tank.owner())
        };
        if survived && let Ok(mut ai) = self.world.get::<&mut Ai>(entity) {
            ai.notify_hit();
        }
    }

    /// A drum the wave reached: thrown along the wave's line onto whoever
    /// stands behind it, or as far as the falloff throws it
    /// (`sonic::drum_landing`), through the drums' launch - it goes off
    /// where it lands. With nowhere to land it goes off where it stands.
    fn throw_drum(&mut self, f: &mut Frame, wave: &SonicWave, entity: Entity, at: Position, variant: i32, t: &Tuning) {
        let origin = wave.cone.origin;
        let falloff = sonic::falloff(t, origin.distance_to(at));
        let tanks: Vec<(usize, Position)> = self
            .world
            .query::<&Tank>()
            .iter()
            .filter(|tank| !tank.is_wreck() && tank.body.is_some() && tank.owner() != wave.owner)
            .map(|tank| (tank.owner_slot(), tank.position))
            .collect();
        let solid: BTreeSet<(i32, i32)> =
            self.world.query::<&Obstacle>().iter().filter(|o| !o.destroyed).map(|o| o.cell()).collect();
        let field = self.map.field_size();
        match sonic::drum_landing(t, at, origin, falloff, &tanks, field, |c| solid.contains(&c)) {
            Some(to) => self.launch_drum(f, entity, at, to, variant),
            None => {
                let dir = (at - origin) / origin.distance_to(at).max(1e-3);
                let fused = self.world.get::<&Obstacle>(entity).is_ok_and(|o| o.fuse.is_some());
                if fused {
                    if let Ok(mut o) = self.world.get::<&mut Obstacle>(entity)
                        && let Some(fuse) = o.fuse.as_mut()
                    {
                        fuse.left = 0.0;
                    }
                } else {
                    self.damage_obstacle(f, entity, f32::MAX, DamageCause::Shot { dir: Some(dir) });
                }
            }
        }
    }

    /// Where one seat's sonic hammer would fire from right now and which
    /// way: the sandbox's pose, for a client drawing its own press
    /// (`net::predict`).
    pub(crate) fn seat_sonic(&self, seat: usize) -> Option<(Position, Dir)> {
        let entity = self.seats.get(seat).copied().flatten()?;
        let tank = self.world.get::<&Tank>(entity).ok()?;
        Some((tank.position, Dir::from_rotation(tank.rotation).unwrap_or(Dir::Up)))
    }

    /// What each hammer-carrying enemy in `enemies` would do with a blast
    /// each way it could face (`ai::HammerSense`, docs/sonic-hammer.md
    /// "AI"), for `Ai::think`: built in `enemy_phase` before the tanks
    /// think, only when one of them carries a hammer. `seats` is every seat
    /// as the enemies see it this frame, in index order; `alerts` each
    /// enemy's alert point; `grid` the frame's nav grid, which a closer's
    /// spot is routed on (`closer_spots`).
    pub(super) fn hammer_senses(
        &self,
        f: &Frame,
        seats: &[HammerSeat],
        alerts: &BTreeMap<Entity, Option<Position>>,
        grid: &crate::pathfind::Grid,
    ) -> BTreeMap<Entity, HammerSense> {
        let t = tuning();
        let mut out = BTreeMap::new();
        let armed: Vec<(Entity, usize, Position, u8)> = self
            .world
            .query::<(Entity, &Tank, &Ai)>()
            .iter()
            .filter(|(_, tank, _)| !tank.is_wreck() && tank.active_weapon() == ActiveWeapon::SonicHammer)
            .map(|(e, tank, ai)| (e, tank.owner_slot(), tank.position, ai.target_player()))
            .collect();
        if armed.is_empty() {
            return out;
        }
        // The enemies whose lanes are trouble: not one an EMP has
        // disabled, which fires nothing.
        let enemies: Vec<(usize, Position)> = self
            .world
            .query::<&Tank>()
            .with::<&Ai>()
            .iter()
            .filter(|tank| !tank.is_wreck() && tank.body.is_some() && !tank.is_disabled())
            .map(|tank| (tank.owner_slot(), tank.position))
            .collect();
        // Each fellow enemy as the points a wave would strike it by, now
        // and where its motion carries it by the time a blast started now
        // would reach it (the tell and the wave's run out), so a friend
        // driving into the cone is not shouted at either.
        let lead = ActiveWeapon::SonicHammer.tell_seconds().unwrap_or(0.0) + t.sonic_reach_px / t.sonic_wave_speed.max(1.0);
        let friends: Vec<(usize, [Position; 6])> = self
            .world
            .query::<&Tank>()
            .with::<&Ai>()
            .iter()
            .filter_map(|tank| {
                let body = tank.body.filter(|_| !tank.is_wreck())?;
                let v = self.physics.velocity(body);
                let [c, a, b, d, e] = hull_points(tank);
                Some((tank.owner_slot(), [c, a, b, d, e, Position::new(c.x + v.x * lead, c.y + v.y * lead)]))
            })
            .collect();
        let blocks = self.sound_blocks();
        let field = self.map.field_size();
        let solid: BTreeSet<(i32, i32)> =
            self.world.query::<&Obstacle>().iter().filter(|o| !o.destroyed).map(|o| o.cell()).collect();
        let drums: Vec<((i32, i32), Position)> = self
            .world
            .query::<&Obstacle>()
            .iter()
            .filter(|o| !o.destroyed && o.material.is_explosive())
            .map(|o| (o.cell(), o.position))
            .collect();
        let cover: BTreeSet<(i32, i32)> = self.cover_cells().iter().map(|&c| crate::map::world_to_cell(c)).collect();
        let towers: Vec<(Position, f32)> = self
            .standing_towers()
            .into_iter()
            .filter_map(|(at, _)| {
                // An offline tower (an EMP) is no trouble while it is out.
                self.towers
                    .values()
                    .find(|tw| tw.position == at && tw.side == crate::frog::Side::Enemy && tw.disabled <= 0.0)
                    .map(|tw| (at, tw.kind.range()))
            })
            .collect();
        let quarry = self.frog.and_then(|e| self.world.get::<&Frog>(e).ok().filter(|fr| !fr.is_dead()).map(|fr| (fr.position, fr.is_stunned())));
        let all_tanks: Vec<(usize, Position)> = self
            .world
            .query::<&Tank>()
            .iter()
            .filter(|tank| !tank.is_wreck() && tank.body.is_some())
            .map(|tank| (tank.owner_slot(), tank.position))
            .collect();
        let (half_w, half_h) = t.sight_box_half_px();
        let half_of = |e: Entity| self.world.get::<&Tank>(e).map_or(Vec2::new(OBSTACLE_GRID_SIZE * 0.5, OBSTACLE_GRID_SIZE * 0.5), |t| t.hull_bbox_world().1);
        // A tank hurt enough to flee closes in on nobody, so it takes no
        // closer's place.
        let fit: Vec<(Entity, usize, Position, u8)> =
            armed.iter().copied().filter(|a| self.world.get::<&Tank>(a.0).is_ok_and(|tank| tank.damage < t.enemy_flee_damage)).collect();
        let spots = closer_spots(&t, &fit, half_of, seats, grid, field, &all_tanks, &friends, |cell| blocks.get(&cell).copied().unwrap_or(Block::Open));
        for &(entity, slot, me, target) in &armed {
            let Ok(ai) = self.world.get::<&Ai>(entity) else { continue };
            let mut sense = HammerSense { spot: spots.get(&entity).copied(), ..HammerSense::default() };
            let alert = alerts.get(&entity).copied().flatten();
            for dir in Dir::ALL {
                let cone = SonicCone::cast(
                    me,
                    dir,
                    t.sonic_reach_px,
                    t.sonic_half_angle_deg.to_radians(),
                    field,
                    |cell| blocks.get(&cell).copied().unwrap_or(Block::Open),
                    |_| Floor::Dry,
                );
                let mut aim = HammerAim {
                    friend: friends.iter().any(|(s, points)| *s != slot && cone.nearest_reached(points).is_some()),
                    ..HammerAim::default()
                };
                for seat in seats.iter().filter(|s| s.live) {
                    if ai.frog_only || !crate::ai::in_sight_box_of((half_w, half_h), seat.pos, me) {
                        continue;
                    }
                    let hidden = seat.concealed && !ai.is_hit_alerted();
                    let reached = cone.nearest_reached(&seat.points);
                    if let Some(d) = reached
                        && !hidden
                    {
                        if aim.breaker.is_none() && d <= t.sonic_ai_breaker_px {
                            aim.breaker = Some(seat.seat);
                        }
                        if aim.trouble.is_none() && self.lands_in_trouble(f, seat, me, d, slot, &enemies, &solid, &towers, &t) {
                            aim.trouble = Some(seat.seat);
                        }
                    }
                    // A drum in the cone thrown onto this seat's cell.
                    if aim.drum.is_none() && !hidden {
                        for &(cell, at) in &drums {
                            if cone.entered(cell).is_none() {
                                continue;
                            }
                            let falloff = sonic::falloff(&t, me.distance_to(at));
                            let others: Vec<(usize, Position)> = all_tanks.iter().copied().filter(|&(s, _)| s != slot).collect();
                            let landing = sonic::drum_landing(&t, at, me, falloff, &others, field, |c| solid.contains(&c));
                            if landing.is_some_and(|l| crate::map::world_to_cell(l) == crate::map::world_to_cell(seat.pos)) {
                                aim.drum = Some(seat.seat);
                                break;
                            }
                        }
                    }
                    // Flush the grass at the alert, for the seat it fights.
                    if aim.flush.is_none() && seat.seat == target && hidden && let Some(alert) = alert {
                        let (ac, ar) = crate::map::world_to_cell(alert);
                        let near_grass = cone.reaches(alert).is_some()
                            && cone.cells.iter().any(|c| cover.contains(&c.cell) && (c.cell.0 - ac).abs() <= 1 && (c.cell.1 - ar).abs() <= 1);
                        if near_grass {
                            aim.flush = Some(seat.seat);
                        }
                    }
                }
                if ai.role == Role::Hunter
                    && let Some((at, stunned)) = quarry
                {
                    aim.frog = !stunned && cone.reaches(at).is_some();
                }
                sense.aims[dir.index()] = aim;
            }
            out.insert(entity, sense);
        }
        out
    }

    /// Whether a knock from `origin` on `seat`, reached at `d` px, slides it
    /// into trouble: along the line from the pivot as far as the skid's own
    /// model says (`sonic::slide` on its ground), cut at the first cell
    /// that stops a hull or the field's edge, any sample hot enough to hurt
    /// (lava and its banks, unless it carries a heat shield), on a burning
    /// cell or a puddle of ooze; or where it comes to rest, inside a
    /// standing enemy tower's reach or lined up for another live enemy - on
    /// its row or column within `enemy_fire_align_px`, inside its attack
    /// range, its sight clear and standing inside the resting point's sight
    /// box - when it does not stand so already, since a slide across a lane
    /// leaves the seat in none and a seat already in one is not shoved into
    /// it.
    #[allow(clippy::too_many_arguments)]
    fn lands_in_trouble(
        &self,
        f: &Frame,
        seat: &HammerSeat,
        origin: Position,
        d: f32,
        shooter: usize,
        enemies: &[(usize, Position)],
        solid: &BTreeSet<(i32, i32)>,
        towers: &[(Position, f32)],
        t: &Tuning,
    ) -> bool {
        let footing = Footing::at(&self.water, &self.lava, self.weather, seat.pos, self.time);
        let speed = sonic::shove_speed(t, seat.mass_factor, d);
        let reach = sonic::slide(t, speed, footing.grip);
        let line = seat.pos - origin;
        let len = line.length();
        if len < 1e-3 {
            return false;
        }
        let dir = line / len;
        let (width, height) = self.map.field_size();
        let burning: BTreeSet<(i32, i32)> = self.fires.iter().filter(|fire| fire.left > 0.0).map(|fire| fire.cell).collect();
        let steps = (reach / sonic::SONIC_TROUBLE_STEP_PX).ceil().max(1.0) as i32;
        let mut rest = seat.pos;
        for i in 1..=steps {
            let p = seat.pos + dir * (reach * i as f32 / steps as f32);
            let cell = crate::map::world_to_cell(p);
            let deep = self.water.depth_at(p) == crate::ground::Depth::Deep || self.lava.depth_at(p) == crate::ground::Depth::Deep;
            if p.x < 0.0 || p.y < 0.0 || p.x > width || p.y > height || solid.contains(&cell) || deep {
                break;
            }
            if seat.takes_heat && self.heat_at(p) >= t.heat_hurt_from {
                return true;
            }
            if burning.contains(&cell) || self.ooze.contains_key(&cell) {
                return true;
            }
            rest = p;
        }
        let in_reach = |p: Position| towers.iter().any(|&(at, range)| at.distance_to(p) <= range);
        let in_lane = |p: Position| {
            enemies.iter().any(|&(slot, at)| {
                if slot == shooter || !crate::ai::in_sight_box(p, at) {
                    return false;
                }
                let (off, forward) = crate::ai::axis_offsets(at, p, Dir::toward(at, p));
                off <= t.enemy_fire_align_px && forward > 0.0 && forward <= t.enemy_attack_range && f.terrain.line_of_sight(at, p)
            })
        };
        (in_reach(rest) && !in_reach(seat.pos)) || (in_lane(rest) && !in_lane(seat.pos))
    }

    /// Put a crate of `kind` down at the map cell nearest `at`, in its air
    /// drop: the dev server's `spawn_pickup`. Not a slot - it never
    /// respawns. Refused outside the field, on a solid tile and where a
    /// pickup already stands. No RNG.
    pub fn debug_spawn_pickup(&mut self, kind: crate::pickup::PickupKind, at: Position) -> Result<Position, String> {
        let (col, row) = crate::map::world_to_cell(at);
        let pos = crate::map::cell_to_world(col, row);
        let (width, height) = self.map.field_size();
        let inset = OBSTACLE_GRID_SIZE * 0.5;
        if !(inset..=width - inset).contains(&pos.x) || !(inset..=height - inset).contains(&pos.y) {
            return Err(format!("({}, {}) is outside the field", at.x, at.y));
        }
        if self.world.query::<&Obstacle>().iter().any(|o| !o.destroyed && o.cell() == (col, row)) {
            return Err(format!("cell {col},{row} holds a tile"));
        }
        if self.world.query::<&crate::pickup::Pickup>().iter().any(|p| p.position.distance_to(pos) < 0.5) {
            return Err(format!("cell {col},{row} already holds a pickup"));
        }
        self.world.spawn((crate::pickup::Pickup::dropped(kind, pos, Some(self.time)),));
        Ok(pos)
    }
}

/// A seat as a hammer-carrying enemy sees it this frame
/// (`Game::hammer_senses`).
pub(super) struct HammerSeat {
    pub seat: u8,
    pub pos: Position,
    /// The points a wave reaches its hull by (`hull_points`).
    pub points: [Position; 5],
    /// On the field and not a wreck.
    pub live: bool,
    /// In tall grass that hides it (`Terrain::conceals`).
    pub concealed: bool,
    /// Its chassis's mass factor (`Tank::mass` over its scale squared).
    pub mass_factor: f32,
    /// Heat reaches it (no heat shield).
    pub takes_heat: bool,
}

impl HammerSeat {
    /// `tank`, seat `seat`, as the senses read it.
    pub(super) fn of(seat: u8, tank: &Tank, live: bool, concealed: bool) -> HammerSeat {
        HammerSeat {
            seat,
            pos: tank.position,
            points: hull_points(tank),
            live,
            concealed,
            mass_factor: tank.mass() / (tank.scale * tank.scale),
            takes_heat: tank.takes_heat(),
        }
    }
}
