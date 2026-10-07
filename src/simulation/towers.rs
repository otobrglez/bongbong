//! The defence towers' rules (docs/defence-towers-prd.md): the tower phase
//! (targeting, the tesla coil's charge and strike, the gun tower's bursts,
//! bio slush's globs), the globs' landing, ooze on tanks and on the ground,
//! a tower's own fire, what each leaves when it dies, and the tower pack's
//! repair. The towers' state and drawing are `crate::tower`.
//!
//! Determinism: towers are walked in cell order (`Game::towers` is a
//! `BTreeMap`), targets break ties on owner slot, and the only RNG drawn is
//! a strike's and a bullet's damage roll and a bullet's spread - so a map
//! without towers draws nothing here, and bio slush draws nothing at all.

use hecs::Entity;
use rand::RngExt;

use super::props::DamageCause;
use super::*;
use crate::ai::in_sight_box;
use crate::bullet::BulletState;
use crate::tower::{Glob, OozePuddle, TeslaBolt, Tower, TowerKind, TowerRuin, side_of_variant};

/// How far from its pivot a gun tower's bullets leave (px): the barrels'
/// tips on the sheet.
const GUN_MUZZLE_PX: f32 = 21.0;

/// How far from its pivot the mortar's mouth is (px).
const BIO_MUZZLE_PX: f32 = 13.0;

/// A tank a tower might fight this frame: a snapshot taken once per phase,
/// seats first then enemies, so every walk over it is in slot order.
#[derive(Clone, Copy, Debug)]
struct Candidate {
    entity: Entity,
    owner: Owner,
    pos: Position,
    /// The hull box (centre, half-extents).
    hull: (Position, Position),
    vel: Vec2,
    concealed: bool,
}

/// A drone a tower might fight this frame (`air.rs`): the air target and,
/// for a seat's, where that seat's tank stands - a tower engages a seat's
/// drone only from inside the seat's sight box, as it engages the seat.
#[derive(Clone, Copy, Debug)]
struct AirCandidate {
    target: crate::air::AirTarget,
    seat_pos: Option<Position>,
}

/// The number a tower's shots carry as their owner's `cell`: its row in
/// the high byte, its column in the low one.
pub(crate) fn tower_cell_id(cell: (i32, i32)) -> u16 {
    ((cell.1.clamp(0, 255) as u16) << 8) | cell.0.clamp(0, 255) as u16
}

/// Whether `tower` may fight `c` under the sight-box rule
/// (docs/large-maps-follow-camera.md section 5): a seat only while the
/// tower stands inside that seat's sight box (`ai::in_sight_box`), as every
/// enemy fires at a seat. At the defaults only the gun tower reaches past
/// the box - straight up or down, its reach is longer than the box's half
/// height - while the tesla's strike and chain and the bio slush's lob stay
/// inside it; the test is applied to all three alike.
fn box_allows(tower: &Tower, c: &Candidate) -> bool {
    !c.owner.is_player() || in_sight_box(c.pos, tower.position)
}

/// Distance from `p` to the nearest point of the box (`center`, `half`).
fn box_distance(p: Position, (center, half): (Position, Position)) -> f32 {
    let dx = ((p.x - center.x).abs() - half.x).max(0.0);
    let dy = ((p.y - center.y).abs() - half.y).max(0.0);
    (dx * dx + dy * dy).sqrt()
}

/// The heading (degrees, 0 = up, clockwise) from `from` to `to`.
fn heading_to(from: Position, to: Position) -> f32 {
    (to.x - from.x).atan2(-(to.y - from.y)).to_degrees()
}

/// `b - a` wrapped into -180..=180.
fn angle_diff(a: f32, b: f32) -> f32 {
    (b - a + 540.0).rem_euclid(360.0) - 180.0
}

/// `heading` turned toward `want` by at most `step` degrees, the short way.
fn turn_toward(heading: f32, want: f32, step: f32) -> f32 {
    let diff = angle_diff(heading, want);
    (heading + diff.clamp(-step, step)).rem_euclid(360.0)
}

fn unit(heading: f32) -> Vec2 {
    let r = heading.to_radians();
    Vec2::new(r.sin(), -r.cos())
}

/// A 32-bit hash of a cell and a frame, for the choices that must look
/// random and may not draw from the round's RNG.
fn cell_frame_hash(cell: (i32, i32), frame: u64, salt: u32) -> u32 {
    let mut h = (cell.0 as u32).wrapping_mul(0x27d4_eb2d)
        ^ (cell.1 as u32).wrapping_mul(0x1656_67b1)
        ^ (frame as u32).wrapping_mul(0x9e37_79b9)
        ^ salt.wrapping_mul(0x85eb_ca6b);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h
}

/// Whether `tower` may engage the drone `a` under the sight-box rule: a
/// seat's only from inside that seat's box, an enemy's from anywhere.
fn air_box_allows(tower: &Tower, a: &AirCandidate) -> bool {
    !a.target.owner.is_player() || a.seat_pos.is_none_or(|seat| in_sight_box(seat, tower.position))
}

impl Game {
    /// Every drone in the air as a tower sees it, by key; empty in a round
    /// with none, which costs the towers nothing.
    fn air_candidates(&self) -> Vec<AirCandidate> {
        let targets = self.air_targets();
        if targets.is_empty() {
            return Vec::new();
        }
        let seats = self.players();
        targets
            .into_iter()
            .map(|target| {
                let seat_pos = match target.owner {
                    Owner::Player(seat) => seats
                        .get(seat as usize)
                        .copied()
                        .flatten()
                        .and_then(|e| self.world.get::<&Tank>(e).ok().map(|t| t.position)),
                    _ => None,
                };
                AirCandidate { target, seat_pos }
            })
            .collect()
    }

    /// The drone a gun tower fights: the nearest opposing one in `range`
    /// (to its ground point) that its level bullets can reach - a line of
    /// sight to the ground point - under the sight-box rule, kept until
    /// another is `tower_switch_margin_px` nearer, ties to the lower key.
    fn pick_air(&self, f: &Frame, tower: &Tower, air: &[AirCandidate], range: f32) -> Option<AirCandidate> {
        if air.is_empty() {
            return None;
        }
        let dist = |a: &AirCandidate| a.target.ground.distance_to(tower.position);
        let valid = |a: &AirCandidate| {
            tower.opposes(a.target.owner)
                && dist(a) <= range
                && air_box_allows(tower, a)
                && f.terrain.line_of_sight_from(tower.entity, tower.position, a.target.ground)
        };
        let best = air.iter().filter(|a| valid(a)).min_by(|a, b| dist(a).total_cmp(&dist(b)).then(a.target.key.cmp(&b.target.key))).copied();
        let current = tower.air_target.and_then(|k| air.iter().find(|a| a.target.key == k)).filter(|a| valid(a)).copied();
        match (current, best) {
            (Some(cur), Some(b)) if b.target.key != cur.target.key && dist(&b) + tuning().tower_switch_margin_px < dist(&cur) => Some(b),
            (Some(cur), _) => Some(cur),
            (None, b) => b,
        }
    }

    /// A tesla coil's arc at a drone (docs/fpv-swarm.md): on its own clock,
    /// every `tesla_air_gap_seconds` (slower while it burns), the nearest
    /// opposing drone within `tesla_range` of its ground point - under the
    /// sight-box rule, no line of sight needed, it is in the air - is struck
    /// and falls. No charge spent, no chain, no roll: its charge on a tank
    /// runs on beside it.
    fn tesla_air(&mut self, f: &mut Frame, cell: (i32, i32), tower: &mut Tower, air: &[AirCandidate]) {
        let t = tuning();
        tower.air_cooldown = (tower.air_cooldown - f.dt).max(0.0);
        if tower.air_cooldown > 0.0 || air.is_empty() {
            return;
        }
        let dist = |a: &AirCandidate| a.target.ground.distance_to(tower.position);
        let Some(a) = air
            .iter()
            .filter(|a| tower.opposes(a.target.owner) && dist(a) <= t.tesla_range && air_box_allows(tower, a))
            .min_by(|a, b| dist(a).total_cmp(&dist(b)).then(a.target.key.cmp(&b.target.key)))
            .copied()
        else {
            return;
        };
        let to = a.target.drawn();
        self.tesla_bolts.push(TeslaBolt::new(tower.position, to, cell_frame_hash(cell, self.frame, 200)));
        f.events.push(Event::TeslaStrike { x0: tower.position.x, y0: tower.position.y, x1: to.x, y1: to.y, chained: false });
        self.strike_air(f, a.target.key, crate::air::AirStrike::Tesla, to);
        tower.air_cooldown = t.tesla_air_gap_seconds / tower.fire_factor().max(0.05);
    }

    /// One `Tower` per tower tile `spawn_from_map` placed, keyed by its
    /// cell; the tile's variant is its side. No RNG.
    /// A turning tower starts aimed at the middle of the field, where the
    /// fight will come from, rather than straight up.
    pub(super) fn build_towers(&mut self, width: f32, height: f32) {
        let middle = Position::new(width * 0.5, height * 0.5);
        self.towers = self
            .world
            .query::<(Entity, &Obstacle)>()
            .iter()
            .filter_map(|(e, o)| {
                TowerKind::from_material(o.material).map(|kind| {
                    let mut tower = Tower::new(kind, side_of_variant(o.variant), e, o.position);
                    if kind.turns() && o.position.distance_to(middle) > 1.0 {
                        tower.heading = heading_to(o.position, middle);
                    }
                    (o.cell(), tower)
                })
            })
            .collect();
    }

    /// Every live, on-field tank, seats first then enemies by slot.
    fn tower_candidates(&self, f: &Frame) -> Vec<Candidate> {
        let mut order: Vec<Entity> = self.seats_on_field().into_iter().flatten().collect();
        let mut enemies: Vec<(usize, Entity)> = self
            .world
            .query::<(Entity, &Tank)>()
            .with::<&Ai>()
            .without::<&RollIn>()
            .iter()
            .map(|(e, t)| (t.owner_slot(), e))
            .collect();
        enemies.sort_unstable_by_key(|&(slot, _)| slot);
        order.extend(enemies.into_iter().map(|(_, e)| e));
        order
            .into_iter()
            .filter_map(|entity| {
                let mut q = self.world.query_one::<&Tank>(entity);
                let t = q.get().ok()?;
                if t.is_wreck() {
                    return None;
                }
                let vel = t.body.map_or(Vec2::zero(), |b| self.physics.velocity(b));
                Some(Candidate {
                    entity,
                    owner: t.owner(),
                    pos: t.position,
                    hull: t.hull_bbox_world(),
                    vel,
                    concealed: f.terrain.conceals(t.position),
                })
            })
            .collect()
    }

    /// The towers fight: each in cell order burns if it is on fire, then
    /// charges, turns and fires by its kind. Runs after the enemies have
    /// fired, so a tower's shots spawn with theirs.
    pub(super) fn tower_phase(&mut self, f: &mut Frame) {
        if self.towers.is_empty() {
            return;
        }
        let cands = self.tower_candidates(f);
        let air = self.air_candidates();
        let cells: Vec<(i32, i32)> = self.towers.keys().copied().collect();
        for cell in cells {
            let Some(mut tower) = self.towers.get(&cell).cloned() else { continue };
            if !self.tower_upkeep(f, &mut tower) || !self.towers.contains_key(&cell) {
                continue;
            }
            // Offline (an EMP): its fire burns on, its weapon does nothing.
            if tower.disabled > 0.0 {
                self.towers.insert(cell, tower);
                continue;
            }
            match tower.kind {
                TowerKind::Tesla => {
                    self.tesla_air(f, cell, &mut tower, &air);
                    self.tick_tesla(f, cell, &mut tower, &cands)
                }
                TowerKind::Gun => self.tick_gun(f, cell, &mut tower, &cands, &air),
                TowerKind::Bio => self.tick_bio(f, cell, &mut tower, &cands),
            }
            if self.towers.contains_key(&cell) {
                self.towers.insert(cell, tower);
            }
        }
    }

    /// A tower's own fire: it catches below `tower_burn_below` or under the
    /// flamethrower's heat, then burns `tower_burn_dps` a second. False once
    /// the tile is gone (burnt out this frame or already destroyed).
    fn tower_upkeep(&mut self, f: &mut Frame, tower: &mut Tower) -> bool {
        let t = tuning();
        let state = {
            let mut q = self.world.query_one::<&Obstacle>(tower.entity);
            q.get().ok().map(|o| (o.destroyed, o.health, o.max_health, o.heat))
        };
        let Some((destroyed, health, max_health, heat)) = state else { return false };
        if destroyed {
            return false;
        }
        if !tower.burning && (health < max_health * t.tower_burn_below || heat >= t.flame_ignite_seconds) {
            tower.burning = true;
            f.events.push(Event::Ignited { x: tower.position.x, y: tower.position.y, what: "tower" });
        }
        if tower.burning && t.tower_burn_dps > 0.0 && self.damage_obstacle(f, tower.entity, t.tower_burn_dps * f.dt, DamageCause::Fire) {
            return false;
        }
        true
    }

    /// The nearest opposing tank between `min` and `max` px that the tower
    /// can see - unconcealed too when `needs_sight`, a seat only from
    /// inside its sight box (`box_allows`) - keeping its current target
    /// until another is `tower_switch_margin_px` nearer.
    fn pick_target(&self, f: &Frame, tower: &Tower, cands: &[Candidate], min: f32, max: f32, needs_sight: bool) -> Option<Candidate> {
        let valid = |c: &Candidate| {
            let d = c.pos.distance_to(tower.position);
            tower.opposes(c.owner)
                && d >= min
                && d <= max
                && !(needs_sight && c.concealed)
                && box_allows(tower, c)
                && f.terrain.line_of_sight_from(tower.entity, tower.position, c.pos)
        };
        let dist = |c: &Candidate| c.pos.distance_to(tower.position);
        let best = cands
            .iter()
            .filter(|c| valid(c))
            .min_by(|a, b| dist(a).total_cmp(&dist(b)).then(a.owner.slot().cmp(&b.owner.slot())))
            .copied();
        let current = tower.target.and_then(|e| cands.iter().find(|c| c.entity == e)).filter(|c| valid(c)).copied();
        match (current, best) {
            (Some(cur), Some(b)) if b.entity != cur.entity && dist(&b) + tuning().tower_switch_margin_px < dist(&cur) => Some(b),
            (Some(cur), _) => Some(cur),
            (None, b) => b,
        }
    }

    /// The tesla coil: charges while an opposing tank is in reach - by its
    /// hull box, concealed or not, a seat only from inside its sight box
    /// (`box_allows`) - drains while none is, and at full charge strikes
    /// the nearest.
    fn tick_tesla(&mut self, f: &mut Frame, cell: (i32, i32), tower: &mut Tower, cands: &[Candidate]) {
        let t = tuning();
        tower.cooldown = (tower.cooldown - f.dt).max(0.0);
        if tower.cooldown > 0.0 {
            tower.charge = 0.0;
            tower.target = None;
            return;
        }
        let nearest = cands
            .iter()
            .filter(|c| {
                tower.opposes(c.owner)
                    && box_distance(tower.position, c.hull) <= t.tesla_range
                    && box_allows(tower, c)
                    && f.terrain.line_of_sight_from(tower.entity, tower.position, c.pos)
            })
            .min_by(|a, b| {
                let (da, db) = (a.pos.distance_to(tower.position), b.pos.distance_to(tower.position));
                da.total_cmp(&db).then(a.owner.slot().cmp(&b.owner.slot()))
            })
            .copied();
        let Some(first) = nearest else {
            tower.target = None;
            tower.charge = (tower.charge - t.tesla_drain_per_second * f.dt).max(0.0);
            return;
        };
        tower.target = Some(first.entity);
        tower.charge += f.dt / t.tesla_charge_seconds.max(1e-3) * tower.fire_factor();
        if tower.charge >= 1.0 {
            self.tesla_strike(f, cell, tower, first, cands);
            tower.charge = 0.0;
            tower.target = None;
            tower.cooldown = t.tesla_cooldown_seconds;
        }
    }

    /// One strike: a bolt to `first`, then up to `tesla_chain_jumps` jumps,
    /// each to the nearest other opposing tank within `tesla_chain_radius`
    /// of the last one hit (a seat only while the coil stands inside its
    /// sight box, `box_allows`), for `tesla_chain_factor` of the damage
    /// before.
    fn tesla_strike(&mut self, f: &mut Frame, cell: (i32, i32), tower: &Tower, first: Candidate, cands: &[Candidate]) {
        let t = tuning();
        let owner = tower.owner(tower_cell_id(cell));
        let jumps = t.tesla_chain_jumps.max(0) as u32;
        let mut from = tower.position;
        let mut target = first;
        let mut factor = 1.0;
        let mut hit: Vec<Entity> = Vec::new();
        for jump in 0..=jumps {
            let to = target.pos;
            self.tesla_bolts.push(TeslaBolt::new(from, to, cell_frame_hash(cell, self.frame, jump)));
            // No impact flash: the flares are fire-coloured, and the strike
            // draws its own violet burst off the event (`fx.rs`).
            f.events.push(Event::TeslaStrike { x0: from.x, y0: from.y, x1: to.x, y1: to.y, chained: jump > 0 });
            let dmg = (t.tesla_damage_min * factor, t.tesla_damage_max * factor);
            self.apply_hit(f, ShellTarget::Tank(target.entity), to, dmg, HitEffects::none(super::HitCause::Tesla), owner);
            hit.push(target.entity);
            if jump == jumps {
                break;
            }
            let next = cands
                .iter()
                .filter(|c| {
                    tower.opposes(c.owner)
                        && !hit.contains(&c.entity)
                        && c.pos.distance_to(to) <= t.tesla_chain_radius
                        && box_allows(tower, c)
                        && f.terrain.line_of_sight(to, c.pos)
                        && !with_tank(&self.world, c.entity, Tank::is_wreck)
                })
                .min_by(|a, b| {
                    a.pos.distance_to(to).total_cmp(&b.pos.distance_to(to)).then(a.owner.slot().cmp(&b.owner.slot()))
                })
                .copied();
            let Some(n) = next else { break };
            from = to;
            target = n;
            factor *= t.tesla_chain_factor;
        }
    }

    /// The gun tower: tracks, leads its target, and fires bursts when its
    /// turret is on the aim and no friend is in the line.
    fn tick_gun(&mut self, f: &mut Frame, cell: (i32, i32), tower: &mut Tower, cands: &[Candidate], air: &[AirCandidate]) {
        let t = tuning();
        tower.cooldown = (tower.cooldown - f.dt).max(0.0);
        // A drone in reach comes before any tank: the gun tower is
        // anti-air (docs/fpv-swarm.md).
        let drone = self.pick_air(f, tower, air, t.gun_tower_range);
        let target = if drone.is_some() { None } else { self.pick_target(f, tower, cands, 0.0, t.gun_tower_range, true) };
        tower.target = target.map(|c| c.entity);
        tower.air_target = drone.map(|a| a.target.key);
        let aimed = match (drone, target) {
            (Some(a), _) => Some((a.target.ground, a.target.velocity, t.gun_tower_air_lead)),
            (None, Some(c)) => Some((c.pos, c.vel, t.gun_tower_lead)),
            (None, None) => None,
        };
        if let Some((pos, vel, lead)) = aimed {
            let flight = pos.distance_to(tower.position) / t.minigun_bullet_speed.max(1.0);
            let aim = pos + vel * (flight * lead);
            let want = heading_to(tower.position, aim);
            tower.heading = turn_toward(tower.heading, want, t.gun_tower_turn_deg_per_second * f.dt);
            let on_aim = angle_diff(tower.heading, want).abs() <= t.gun_tower_fire_cone_deg;
            if tower.burst_left == 0 && tower.cooldown <= 0.0 && on_aim && !friend_in_line(tower, pos, cands) {
                tower.burst_left = t.gun_tower_burst_size.max(1) as u32;
                tower.burst_timer = 0.0;
                let muzzle = tower.position + unit(tower.heading) * GUN_MUZZLE_PX;
                f.events.push(Event::TowerFired { kind: TowerKind::Gun.name(), x: muzzle.x, y: muzzle.y, heading: tower.heading });
                f.muzzle_flashes.push(Shockwave::new(muzzle));
            }
        }
        // A burst runs out whether or not its target is still there.
        if tower.burst_left > 0 {
            let rate = tower.fire_factor().max(0.05);
            tower.burst_timer -= f.dt;
            while tower.burst_left > 0 && tower.burst_timer <= 0.0 {
                self.fire_tower_bullet(f, cell, tower);
                tower.burst_left -= 1;
                tower.burst_timer += t.gun_tower_bullet_delay_seconds / rate;
            }
            if tower.burst_left == 0 {
                tower.cooldown = t.gun_tower_burst_gap_seconds / rate;
            }
        }
    }

    /// One of the gun tower's bullets: the minigun's, from the barrels'
    /// tips along the turret with `gun_tower_spread_deg` of roll. It never
    /// hits the tile it was fired from.
    fn fire_tower_bullet(&mut self, f: &mut Frame, cell: (i32, i32), tower: &Tower) {
        let t = tuning();
        let spread = if t.gun_tower_spread_deg > 0.0 { f.rng.random_range(-t.gun_tower_spread_deg..t.gun_tower_spread_deg) } else { 0.0 };
        let rotation = tower.heading + spread;
        let dir = unit(rotation);
        let at = tower.position + dir * GUN_MUZZLE_PX;
        let mut bullet = Bullet::at(0, at, at, dir * t.minigun_bullet_speed, rotation, 0, tower.owner(tower_cell_id(cell)));
        bullet.state = BulletState::Muzzle;
        bullet.shadow_offset = t.minigun_bullet_shadow_offset_min;
        bullet.passed_over.push(tower.entity);
        f.pending_bullets.push(bullet);
    }

    /// Bio slush: turns toward a target between its minimum and maximum
    /// range and lobs a glob at where it will be when the glob lands.
    fn tick_bio(&mut self, f: &mut Frame, cell: (i32, i32), tower: &mut Tower, cands: &[Candidate]) {
        let t = tuning();
        let interval = t.bio_lob_interval_seconds / tower.fire_factor().max(0.05);
        tower.cooldown = (tower.cooldown - f.dt).max(0.0);
        tower.charge = 1.0 - (tower.cooldown / interval.max(1e-3)).clamp(0.0, 1.0);
        let target = self.pick_target(f, tower, cands, t.bio_min_range, t.bio_range, true);
        tower.target = target.map(|c| c.entity);
        let Some(c) = target else { return };
        let flight = t.bio_glob_flight_seconds;
        let h = cell_frame_hash(cell, self.frame, 7);
        let scatter_angle = (h % 3600) as f32 / 3600.0 * std::f32::consts::TAU;
        let scatter = t.bio_scatter_px * ((h >> 12) % 1000) as f32 / 1000.0;
        let lead = c.pos + c.vel * (flight * t.bio_lead);
        let margin = OBSTACLE_GRID_SIZE * 0.5;
        let aim = Position::new(
            (lead.x + scatter_angle.cos() * scatter).clamp(margin, f.width - margin),
            (lead.y + scatter_angle.sin() * scatter).clamp(margin, f.height - margin),
        );
        let want = heading_to(tower.position, aim);
        tower.heading = turn_toward(tower.heading, want, t.bio_turn_deg_per_second * f.dt);
        if tower.cooldown > 0.0 || angle_diff(tower.heading, want).abs() > t.bio_fire_cone_deg {
            return;
        }
        let muzzle = tower.position + unit(tower.heading) * BIO_MUZZLE_PX;
        let id = self.take_shot_id();
        self.globs.push(Glob {
            id,
            from: muzzle,
            to: aim,
            age: 0.0,
            flight,
            apex: t.bio_glob_apex_px,
            owner: tower.owner(tower_cell_id(cell)),
        });
        tower.cooldown = interval;
        tower.charge = 0.0;
        // No muzzle flash: a lob is a spit of ooze, not a bang (`fx.rs`
        // throws it off the event).
        f.events.push(Event::TowerFired { kind: TowerKind::Bio.name(), x: muzzle.x, y: muzzle.y, heading: tower.heading });
    }

    /// Age the globs in the air and splash the ones that landed. `live` is
    /// false on the end screen, where a glob lands without coating anyone.
    pub(super) fn resolve_globs(&mut self, f: &mut Frame, live: bool) {
        if self.globs.is_empty() {
            return;
        }
        for glob in &mut self.globs {
            glob.age += f.dt;
        }
        let (landed, flying): (Vec<Glob>, Vec<Glob>) = std::mem::take(&mut self.globs).into_iter().partition(Glob::landed);
        self.globs = flying;
        for glob in landed {
            self.splash(f, &glob, live);
        }
    }

    /// A glob's landing: the splash coats and hurts every tank opposing its
    /// tower within `bio_splash_radius`, and leaves ooze on the dry, open
    /// cells around it. In water it is only a splash.
    fn splash(&mut self, f: &mut Frame, glob: &Glob, live: bool) {
        let t = tuning();
        let at = glob.to;
        f.events.push(Event::GlobSplashed { x: at.x, y: at.y });
        if !live || self.water.depth_at(at) != crate::ground::Depth::Dry {
            return;
        }
        self.lay_ooze(f, at, t.bio_splash_radius, t.bio_puddle_seconds);
        let cands = self.tower_candidates(f);
        for c in cands {
            if glob.owner.same_side(c.owner) || box_distance(at, c.hull) > t.bio_splash_radius {
                continue;
            }
            self.coat(f, c.entity, t.bio_splash_damage, Some(glob.owner));
        }
    }

    /// Coat `entity` in ooze and deal it `damage` - a splash's, or a
    /// puddle's. A coat `by` a shooter marks it hit and alerts an enemy, as
    /// a shot would - and one by a standing tower gives it a grudge.
    fn coat(&mut self, f: &mut Frame, entity: Entity, damage: f32, by: Option<Owner>) {
        let hit = by.is_some();
        let tower = by.and_then(|owner| self.tower_by_owner(owner));
        let slime = tuning().bio_slime_seconds;
        let (killed, fresh, slot, pos, owner) = {
            let mut q = self.world.query_one::<&mut Tank>(entity);
            let Ok(tank) = q.get() else { return };
            if tank.is_wreck() {
                return;
            }
            let fresh = !tank.is_slimed();
            tank.slime_timer = slime;
            if damage > 0.0 {
                tank.take_damage(damage, MAX_DAMAGE);
            }
            if hit {
                tank.mark_hit();
            }
            (tank.is_wreck(), fresh, tank.owner_slot(), tank.position, tank.owner())
        };
        if fresh {
            f.events.push(Event::Slimed { slot });
        }
        if killed {
            f.kills.push((pos, owner));
        } else if hit && let Ok(mut ai) = self.world.get::<&mut Ai>(entity) {
            match tower {
                Some(at) => ai.notify_tower_hit(at),
                None => ai.notify_hit(),
            }
        }
    }

    /// Ooze on every dry, open cell within `radius` of `at` - always the
    /// cell `at` is in - lasting `seconds`, or longer if a puddle there
    /// already would.
    fn lay_ooze(&mut self, f: &mut Frame, at: Position, radius: f32, seconds: f32) {
        if seconds <= 0.0 {
            return;
        }
        let solid: HashSet<(i32, i32)> =
            self.world.query::<&Obstacle>().iter().filter(|o| !o.destroyed).map(|o| o.cell()).collect();
        let burning: HashSet<(i32, i32)> = self.fires.iter().map(|g| g.cell).collect();
        let centre = map::world_to_cell(at);
        let cols = (f.width / OBSTACLE_GRID_SIZE).ceil() as i32;
        let rows = (f.height / OBSTACLE_GRID_SIZE).ceil() as i32;
        for dy in -2..=2 {
            for dx in -2..=2 {
                let cell = (centre.0 + dx, centre.1 + dy);
                if cell != centre && map::cell_to_world(cell.0, cell.1).distance_to(at) > radius {
                    continue;
                }
                let inside = cell.0 >= 0 && cell.1 >= 0 && cell.0 < cols && cell.1 < rows;
                let dry = self.water.depth_at(map::cell_to_world(cell.0, cell.1)) == crate::ground::Depth::Dry;
                if !inside || !dry || solid.contains(&cell) || burning.contains(&cell) {
                    continue;
                }
                let puddle = self.ooze.entry(cell).or_insert(OozePuddle { left: 0.0, total: seconds });
                if puddle.left < seconds {
                    *puddle = OozePuddle { left: seconds, total: seconds };
                }
            }
        }
    }

    /// Ooze over time: puddles dry, a fire takes the ooze in its cell, a
    /// hull over a puddle is coated, a coated tank corrodes, and one in
    /// water is washed clean. Seats first, then enemies.
    pub(super) fn tick_ooze(&mut self, f: &mut Frame) {
        let t = tuning();
        if !self.ooze.is_empty() {
            let burning: HashSet<(i32, i32)> = self.fires.iter().map(|g| g.cell).collect();
            self.ooze.retain(|cell, p| {
                p.left -= f.dt;
                p.left > 0.0 && !burning.contains(cell)
            });
        }
        let mut order: Vec<Entity> = self.seats_on_field().into_iter().flatten().collect();
        let mut enemies: Vec<(usize, Entity)> =
            self.world.query::<(Entity, &Tank)>().with::<&Ai>().without::<&RollIn>().iter().map(|(e, t)| (t.owner_slot(), e)).collect();
        enemies.sort_unstable_by_key(|&(slot, _)| slot);
        order.extend(enemies.into_iter().map(|(_, e)| e));
        for entity in order {
            let mut q = self.world.query_one::<&mut Tank>(entity);
            let Ok(tank) = q.get() else { continue };
            if tank.is_wreck() {
                continue;
            }
            if self.water.depth_at(tank.position) != crate::ground::Depth::Dry {
                if tank.is_slimed() {
                    tank.slime_timer = 0.0;
                    f.events.push(Event::SlimeWashed { slot: tank.owner_slot() });
                }
                continue;
            }
            if !self.ooze.is_empty() && over_ooze(&self.ooze, tank.hull_bbox_world()) {
                if !tank.is_slimed() {
                    f.events.push(Event::Slimed { slot: tank.owner_slot() });
                }
                tank.slime_timer = t.bio_slime_seconds;
            }
            if !tank.is_slimed() {
                continue;
            }
            if t.bio_slime_dps > 0.0 {
                tank.take_damage(t.bio_slime_dps * f.dt, MAX_DAMAGE);
            }
            tank.slime_timer = (tank.slime_timer - f.dt).max(0.0);
            if tank.is_wreck() {
                f.kills.push((tank.position, tank.owner()));
            }
        }
    }

    /// A tower tile died (`obstacle_died`): its weapon goes, its ruin is
    /// left on the ground, and each kind goes out its own way - the tesla
    /// discharges into everything close, the gun tower's ammunition cooks
    /// off, bio slush bursts into a spill.
    pub(super) fn tower_died(&mut self, f: &mut Frame, kind: TowerKind, side: Side, pos: Position) {
        let t = tuning();
        let cell = map::world_to_cell(pos);
        self.towers.remove(&cell);
        self.tower_ruins.push(TowerRuin { kind, side, position: pos, age: 0.0 });
        let mut show = Spectacle::default();
        self.wreck_show(&mut show, pos);
        f.stage(show);
        match kind {
            TowerKind::Tesla => {
                let radius = t.tesla_death_blast_radius;
                if radius > 0.0 {
                    let cands = self.tower_candidates(f);
                    for (i, c) in cands.into_iter().enumerate() {
                        let d = c.pos.distance_to(pos);
                        if d > radius {
                            continue;
                        }
                        self.tesla_bolts.push(TeslaBolt::new(pos, c.pos, cell_frame_hash(cell, self.frame, 100 + i as u32)));
                        f.events.push(Event::TeslaStrike { x0: pos.x, y0: pos.y, x1: c.pos.x, y1: c.pos.y, chained: true });
                        let damage = t.tesla_death_blast_damage * (1.0 - d / radius);
                        let (killed, owner, at) = {
                            let mut q = self.world.query_one::<&mut Tank>(c.entity);
                            let Ok(tank) = q.get() else { continue };
                            if tank.is_wreck() {
                                continue;
                            }
                            tank.take_damage(damage, MAX_DAMAGE);
                            tank.mark_hit();
                            (tank.is_wreck(), tank.owner(), tank.position)
                        };
                        if killed {
                            f.kills.push((at, owner));
                        }
                    }
                }
            }
            TowerKind::Gun => {
                let count = t.cookoff_count.max(0) as u32 + 1;
                let window = t.cookoff_window_seconds;
                for i in 0..count {
                    let h = crate::blast::seed_at(pos, 150 + i * 7);
                    let delay = window * (0.15 + 0.85 * (h % 1000) as f32 / 1000.0);
                    let off = ((h >> 10) % 25) as f32 - 12.0;
                    let off2 = ((h >> 16) % 25) as f32 - 12.0;
                    self.cookoffs.push((Position::new(pos.x + off, pos.y + off2), delay));
                }
            }
            TowerKind::Bio => {
                self.lay_ooze(f, pos, OBSTACLE_GRID_SIZE * 1.5, t.bio_spill_seconds);
                // The tile is gone this frame but still in `solid` until
                // cleanup: its own cell gets its ooze here.
                if self.water.depth_at(pos) == crate::ground::Depth::Dry && t.bio_spill_seconds > 0.0 {
                    self.ooze.insert(cell, OozePuddle { left: t.bio_spill_seconds, total: t.bio_spill_seconds });
                }
                for c in self.tower_candidates(f) {
                    if box_distance(pos, c.hull) <= t.bio_splash_radius {
                        self.coat(f, c.entity, 0.0, None);
                    }
                }
            }
        }
    }

    /// Whether a tower pack is taken by a tank on `side`: yes unless every
    /// standing tower of that side is already at full health and not
    /// burning (a side with none takes it, and wastes it).
    pub(super) fn tower_pack_wanted(&self, side: Side) -> bool {
        let mut any = false;
        for tower in self.towers.values().filter(|t| t.side == side) {
            let mut q = self.world.query_one::<&Obstacle>(tower.entity);
            let Ok(o) = q.get() else { continue };
            if o.destroyed {
                continue;
            }
            any = true;
            if tower.burning || o.health < o.max_health {
                return true;
            }
        }
        !any
    }

    /// A player tower is hurt or burning - the tower pack's bonus-drop
    /// gate, read before any RNG is drawn.
    pub(super) fn player_tower_hurt(&self) -> bool {
        self.towers.values().any(|t| t.side == Side::Player) && self.tower_pack_wanted(Side::Player)
    }

    /// The tower pack's effect: every standing tower of `side` back to full
    /// health, put out.
    pub(super) fn repair_towers(&mut self, f: &mut Frame, side: Side) {
        for tower in self.towers.values_mut().filter(|t| t.side == side) {
            let mut q = self.world.query_one::<&mut Obstacle>(tower.entity);
            let Ok(o) = q.get() else { continue };
            if o.destroyed {
                continue;
            }
            o.health = o.max_health;
            o.heat = 0.0;
            tower.burning = false;
            f.events.push(Event::TowerRepaired { side, x: tower.position.x, y: tower.position.y });
        }
    }

    /// Age what towers leave that only the picture needs: the bolts on
    /// screen and the ruins' smoulder clock. Runs on the end screen and on
    /// a replica too.
    pub(super) fn tick_tower_effects(&mut self, dt: f32) {
        self.tesla_bolts.retain_mut(|b| !b.tick(dt));
        for tower in self.towers.values_mut() {
            tower.ease_droop(dt);
        }
        for ruin in &mut self.tower_ruins {
            ruin.age += dt;
        }
    }

    /// Where the standing tower that fired as `owner` is, if `owner` is a
    /// tower and it still stands.
    pub(super) fn tower_by_owner(&self, owner: Owner) -> Option<Position> {
        let Owner::Tower { cell, .. } = owner else { return None };
        let key = ((cell & 0xff) as i32, (cell >> 8) as i32);
        let tower = self.towers.get(&key)?;
        let standing = self.world.get::<&Obstacle>(tower.entity).is_ok_and(|o| !o.destroyed);
        standing.then_some(tower.position)
    }

    /// The centre of every routing cell inside a standing player tower's
    /// reach - what `route_grid` surcharges with `route_tower_cost`, so the
    /// enemies route round the player's defences where a way round exists.
    pub(super) fn player_tower_reach(&self, width: f32, height: f32) -> Vec<Position> {
        let size = PATHFIND_CELL_SIZE;
        let (cols, rows) = ((width / size).ceil() as i32, (height / size).ceil() as i32);
        let mut cells = Vec::new();
        for (at, _) in self.standing_towers() {
            let Some(tower) = self.towers.values().find(|t| t.position == at && t.side == Side::Player) else { continue };
            // An offline tower (an EMP) fires at nobody: no detour for it.
            if tower.disabled > 0.0 {
                continue;
            }
            let range = tower.kind.range();
            let c0 = (((at.x - range) / size).floor() as i32).max(0);
            let c1 = (((at.x + range) / size).floor() as i32).min(cols - 1);
            let r0 = (((at.y - range) / size).floor() as i32).max(0);
            let r1 = (((at.y + range) / size).floor() as i32).min(rows - 1);
            for r in r0..=r1 {
                for c in c0..=c1 {
                    let p = Position::new((c as f32 + 0.5) * size, (r as f32 + 0.5) * size);
                    if p.distance_to(at) <= range {
                        cells.push(p);
                    }
                }
            }
        }
        cells
    }

    /// The centre of every routing cell a puddle lies on - what
    /// `route_grid` surcharges with `bio_puddle_path_cost`. A map cell is
    /// centred on a routing cell corner, so each puddle touches four, and
    /// a cell two puddles share is counted once.
    pub(super) fn ooze_route_cells(&self) -> Vec<Position> {
        let size = PATHFIND_CELL_SIZE;
        let cells: std::collections::BTreeSet<(i32, i32)> = self
            .ooze
            .keys()
            .flat_map(|&(c, r)| [(c - 1, r - 1), (c, r - 1), (c - 1, r), (c, r)])
            .filter(|&(c, r)| c >= 0 && r >= 0)
            .collect();
        cells.into_iter().map(|(c, r)| Position::new((c as f32 + 0.5) * size, (r as f32 + 0.5) * size)).collect()
    }

    /// Every standing tower's position and tile, for `enemy_phase`'s
    /// grudge sight test.
    pub(super) fn standing_towers(&self) -> Vec<(Position, Entity)> {
        self.towers
            .values()
            .filter(|t| self.world.get::<&Obstacle>(t.entity).is_ok_and(|o| !o.destroyed))
            .map(|t| (t.position, t.entity))
            .collect()
    }

    /// The ruins dead towers left, for the smoke `fx.rs` raises off them.
    pub fn tower_ruins(&self) -> &[crate::tower::TowerRuin] {
        &self.tower_ruins
    }

    /// Every live tank wearing ooze, by slot, with its position - the drips
    /// `fx.rs` sheds.
    pub fn slimed(&self) -> Vec<(usize, Position)> {
        let mut out: Vec<(usize, Position)> = self
            .world
            .query::<&Tank>()
            .iter()
            .filter(|t| t.is_slimed() && !t.is_wreck())
            .map(|t| (t.owner_slot(), t.position))
            .collect();
        out.sort_by_key(|s| s.0);
        out
    }

    /// Standing towers with what the picture needs: kind, side, position,
    /// health stage (0..=3), heading, charge (0..=1) and whether it burns.
    pub fn tower_views(&self) -> Vec<crate::tower::TowerView> {
        self.towers
            .values()
            .filter_map(|tower| {
                let mut q = self.world.query_one::<&Obstacle>(tower.entity);
                let o = q.get().ok()?;
                (!o.destroyed).then(|| crate::tower::TowerView {
                    kind: tower.kind,
                    side: tower.side,
                    position: tower.position,
                    stage: crate::tower::stage_of(o.health, o.max_health),
                    heading: tower.heading,
                    charge: tower.charge,
                    burning: tower.burning,
                    disabled: tower.disabled,
                    droop: tower.droop,
                })
            })
            .collect()
    }
}

/// Whether a same-side tank stands within `gun_tower_friendly_block_px`
/// of its hull of the segment from the tower to `target`.
fn friend_in_line(tower: &Tower, target: Position, cands: &[Candidate]) -> bool {
    let block = tuning().gun_tower_friendly_block_px;
    let (a, b) = (tower.position, target);
    let ab = b - a;
    let len2 = ab.x * ab.x + ab.y * ab.y;
    if len2 <= 0.0 {
        return false;
    }
    cands.iter().filter(|c| !tower.opposes(c.owner)).any(|c| {
        let ap = c.pos - a;
        let u = (ap.x * ab.x + ap.y * ab.y) / len2;
        if !(0.0..=1.0).contains(&u) {
            return false;
        }
        let closest = a + ab * u;
        let reach = block + c.hull.1.x.max(c.hull.1.y);
        closest.distance_to(c.pos) <= reach
    })
}

/// Whether the hull box (`center`, `half`) overlaps a cell with ooze.
fn over_ooze(ooze: &BTreeMap<(i32, i32), OozePuddle>, (center, half): (Position, Position)) -> bool {
    // Map cells are centred on multiples of the grid size (`map::world_to_cell`
    // rounds), so a coordinate's cell is the nearest multiple.
    let size = OBSTACLE_GRID_SIZE;
    let c0 = ((center.x - half.x) / size).round() as i32;
    let c1 = ((center.x + half.x) / size).round() as i32;
    let r0 = ((center.y - half.y) / size).round() as i32;
    let r1 = ((center.y + half.y) / size).round() as i32;
    (r0..=r1).any(|r| (c0..=c1).any(|c| ooze.contains_key(&(c, r))))
}
