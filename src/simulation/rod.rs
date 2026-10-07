//! The rod from god's world half (docs/rod-from-god.md; `rod.rs` is the
//! reticle, the measures and the pictures, `zone.rs` the call standing on
//! the field). A released charge calls a rod on its reticle's cell
//! (`fire_rod`), put on the field as a zone once the tank loops are done
//! (`place_calls`); at its end the rod lands (`resolve_zones`): every hull
//! with any part in its circle crushed, frogs killed, drones downed, every
//! breakable tile within the break radius crushed, hulls out to the shove
//! radius knocked off their tracks, a crater left on dry ground and a
//! volcano it struck set off. Also what the AI is handed - the standing
//! seats (`tick_seat_still`), each rod tank's pick (`rod_senses`), the
//! calls as dangers (`zone_dangers`) - and the reticle report a room sets
//! on a seat (`set_seat_reticle`). The rod draws no RNG of its own.

use hecs::Entity;

use crate::ai::{Ai, Danger, RodPick, RodSense, Role};
use crate::frog::{Frog, Side};
use crate::map::{cell_to_world, world_to_cell};
use crate::math::Vec2;
use crate::obstacle::{face_toward, Obstacle};
use crate::rod::{self, Ground, RodCall, RodImpactFx};
use crate::shell::Owner;
use crate::shockwave::Shockwave;
use crate::tank::{ActiveWeapon, Dir, Tank};
use crate::tuning::{tuning, Tuning};
use crate::zone::{Zone, ZoneKind};
use crate::{MAX_DAMAGE, MAX_SEATS, OBSTACLE_GRID_SIZE, Position};

use super::combat::BlastParams;
use super::props::DamageCause;
use super::{Event, Footing, Frame, Game, HitCause, HitTarget, RollIn, SHOCK_FROG, Spectacle};

/// A rod called this frame, waiting for `place_calls` to put it on the
/// field.
pub(super) struct PendingCall {
    pub owner: Owner,
    pub cell: (i32, i32),
}

/// Call a rod on a released charge's reticle (docs/rod-from-god.md "The
/// call"): false - nothing spent, the release a fizzle - with no reticle, no
/// call left, or the reticle on the caller's own cell (the cancel). Else a
/// call spent, `rod_reload_seconds` before the next reticle, the uplink's
/// cell, `Event::Fired` and the call queued for `place_calls`. No RNG.
pub(super) fn fire_rod(f: &mut Frame, tank: &mut Tank, owner: Owner, reticle: Option<(i32, i32)>) -> bool {
    let Some(cell) = reticle else { return false };
    if tank.rods <= 0 || cell == world_to_cell(tank.position) {
        return false;
    }
    tank.rods -= 1;
    tank.fire_cooldown = tuning().rod_reload_seconds;
    tank.kick_rod();
    f.events.push(Event::Fired { slot: tank.owner_slot(), weapon: ActiveWeapon::RodFromGod.name() });
    f.pending_calls.push(PendingCall { owner, cell });
    true
}

/// What a rod-carrying enemy is handed about one seat (`Game::rod_senses`).
#[derive(Clone, Copy, Debug)]
pub(super) struct RodSeat {
    pub seat: u8,
    pub pos: Position,
    /// On the field and not a wreck.
    pub live: bool,
    /// In tall grass that hides it.
    pub concealed: bool,
    /// How far an enemy sees it under the sky (`Game::sight_on`).
    pub sight: f32,
}

impl Game {
    /// The zones standing on the field (`zone.rs`), sorted by id.
    pub fn zones(&self) -> &[Zone] {
        &self.zones
    }

    /// The craters the rods have left this round.
    pub fn craters(&self) -> &rod::Craters {
        &self.craters
    }

    /// How seat `seat` has been moving (`rod::SeatStill`).
    pub fn seat_still(&self, seat: usize) -> rod::SeatStill {
        self.seat_still.get(seat).copied().unwrap_or_default()
    }

    /// Put this frame's calls on the field: each a zone with an id from the
    /// projectile counter, landing `rod_countdown_seconds` on, and
    /// `Event::RodCalled` after its `Fired`.
    pub(super) fn place_calls(&mut self, f: &mut Frame) {
        if f.pending_calls.is_empty() {
            return;
        }
        let t = tuning();
        for call in std::mem::take(&mut f.pending_calls) {
            let id = self.take_shot_id();
            let seat = match call.owner {
                Owner::Player(seat) => Some(seat),
                _ => None,
            };
            let zone = Zone {
                id,
                kind: ZoneKind::Rod(RodCall { cell: call.cell, seat }),
                owner: call.owner,
                centre: cell_to_world(call.cell.0, call.cell.1),
                until: self.time + t.rod_countdown_seconds,
            };
            f.events.push(Event::RodCalled {
                id,
                slot: call.owner.slot(),
                seat: crate::net::encode::owner_seat(call.owner),
                cell: call.cell,
                land: zone.until,
            });
            self.zones.push(zone);
        }
        self.zones.sort_by_key(|z| z.id);
    }

    /// Land every zone whose end has come, in id order (`rod_impact`); on
    /// the end screen (`live` false) with its whole show and nothing else.
    pub(super) fn resolve_zones(&mut self, f: &mut Frame, live: bool) {
        if self.zones.is_empty() {
            return;
        }
        let now = self.time + f.dt * 0.5;
        let (due, standing): (Vec<Zone>, Vec<Zone>) = std::mem::take(&mut self.zones).into_iter().partition(|z| z.until <= now);
        self.zones = standing;
        for zone in due {
            self.rod_impact(f, zone, live);
        }
    }

    /// A rod lands on `zone`'s cell (docs/rod-from-god.md "The impact"):
    /// with `live`, in this order - every live hull on the field with any
    /// part of its box in the circle crushed, every other within the shove
    /// radius knocked out from the strike; frogs in the circle killed and in
    /// the ring stunned; drones over the circle downed; every breakable tile
    /// whose cell reaches into the break radius crushed (iron, a cone and a
    /// door sooted, standing), drums going off at once; grenades in reach
    /// set off, lanterns broken, the oil trail lit, breakable crates broken,
    /// the grass flattened; a crater on dry ground; a volcano struck set
    /// off. Its show either way. No RNG: what it sets off draws as it
    /// always does.
    fn rod_impact(&mut self, f: &mut Frame, zone: Zone, live: bool) {
        let t = tuning();
        let Some(call) = zone.rod() else { return };
        let c = zone.centre;
        let cells = if live { self.crater_cells_at(call.cell) } else { Vec::new() };
        let erupted = live && !self.struck_volcanoes(c, &t).is_empty();
        // What it struck, read before the crater is made (and filled).
        let ground = self.ground_struck(c);
        f.events.push(Event::RodImpact { id: zone.id, cell: call.cell, crater: !cells.is_empty(), erupted });
        if live {
            self.rod_hulls(f, c, zone.owner, &t);
            self.rod_frogs(f, c, &t);
            for target in self.air_targets() {
                if target.ground.distance_to(c) <= t.rod_kill_radius_px {
                    self.strike_air(f, target.key, crate::air::AirStrike::Rod);
                }
            }
            self.rod_tiles(f, c, &t);
            self.rod_ground(f, c, &t);
            if !cells.is_empty() {
                self.make_crater(call.cell, self.time, &cells);
            }
            if erupted {
                self.set_off_volcanoes(c, &t);
            }
        }
        let mut show = Spectacle::default();
        self.rod_show(&mut show, c, ground);
        f.stage(show);
    }

    /// The hulls: every live one on the field with a body (`hulls_in_order`)
    /// with its box's nearest point inside the circle is crushed, then every
    /// other out to the shove radius knocked (`knock_from`).
    fn rod_hulls(&mut self, f: &mut Frame, c: Position, by: Owner, t: &Tuning) {
        for entity in self.hulls_in_order() {
            let Ok(mut tank) = self.world.get::<&mut Tank>(entity) else { continue };
            let (centre, half) = tank.hull_bbox_world();
            if crate::emp::box_reach(c, centre, half) > t.rod_kill_radius_px {
                continue;
            }
            crush(&mut tank);
            tank.mark_hit();
            tank.credit(by);
            let target = match tank.owner() {
                Owner::Player(player) => HitTarget::Player { player },
                Owner::Enemy(slot) => HitTarget::Enemy { slot },
                Owner::Tower { .. } => unreachable!("no tank is owned by a tower"),
            };
            let at = tank.position;
            f.events.push(Event::Hit { target, damage: MAX_DAMAGE, killed: true, x: at.x, y: at.y, cause: HitCause::Rod });
            f.kills.push((at, tank.owner()));
        }
        self.knock_from(f, c, t.rod_kill_radius_px, t.rod_shove_radius_px, |mass_factor, d| rod::shove_speed(t, mass_factor, d));
    }

    /// Every live hull on the field with a body, in the walk order: the
    /// seats in index order, then the enemies by slot; a tank rolling in
    /// through a gate is not on the field.
    pub(super) fn hulls_in_order(&self) -> Vec<Entity> {
        let mut hulls: Vec<Entity> = self.seats_on_field().into_iter().flatten().collect();
        let mut enemies: Vec<(usize, Entity)> = self.world.query::<(Entity, &Tank)>().with::<&Ai>().iter().map(|(e, t)| (t.owner_slot(), e)).collect();
        enemies.sort_by_key(|&(slot, _)| slot);
        hulls.extend(enemies.into_iter().map(|(_, e)| e));
        hulls.retain(|&e| {
            self.world.get::<&RollIn>(e).is_err() && self.world.get::<&Tank>(e).is_ok_and(|t| !t.is_wreck() && t.body.is_some())
        });
        hulls
    }

    /// Knock every live hull on the field (`hulls_in_order`) whose box's
    /// nearest point stands more than `inner` and at most `outer` from
    /// `center` along the line out from it (straight ahead of its facing
    /// where it stands on the centre), at `speed(mass factor, distance)`
    /// px/s - the hammer's `knock`, the footing it stands on taken in. A
    /// rod's shove ring; a gravity well's collapse throws with it too.
    pub(super) fn knock_from(&mut self, f: &mut Frame, center: Position, inner: f32, outer: f32, speed: impl Fn(f32, f32) -> f32) {
        for entity in self.hulls_in_order() {
            let footing = super::with_tank(&self.world, entity, |tank| Footing::at(&self.water, &self.lava, &self.craters, self.weather, tank.position, self.time));
            let Ok(mut tank) = self.world.get::<&mut Tank>(entity) else { continue };
            let (centre, half) = tank.hull_bbox_world();
            let d = crate::emp::box_reach(center, centre, half);
            if d <= inner || d > outer {
                continue;
            }
            let line = tank.position - center;
            let len = line.length();
            let dir = if len > 1e-3 { line / len } else { Dir::from_rotation(tank.rotation).unwrap_or(Dir::Up).vec() };
            let mass_factor = tank.mass() / (tank.scale * tank.scale);
            let v = speed(mass_factor, d);
            if v > 0.0 {
                super::sonic::knock(&mut self.physics, f, &mut tank, dir, v, footing);
            }
        }
    }

    /// The frogs, either side's: one whose collider reaches into the circle
    /// is killed, one in the shove ring stunned.
    fn rod_frogs(&mut self, f: &mut Frame, c: Position, t: &Tuning) {
        let half = Vec2::new(crate::FROG_COLLIDER_HALF_EXTENT.0, crate::FROG_COLLIDER_HALF_EXTENT.1);
        let mut dead = Vec::new();
        for frog in self.world.query_mut::<&mut Frog>() {
            if frog.is_dead() {
                continue;
            }
            let d = crate::emp::box_reach(c, frog.position, half);
            if d <= t.rod_kill_radius_px {
                frog.damage(f32::MAX);
                f.events.push(Event::Hit { target: HitTarget::Frog { side: frog.side }, damage: MAX_DAMAGE, killed: true, x: frog.position.x, y: frog.position.y, cause: HitCause::Rod });
                dead.push(frog.position);
            } else if frog.position.distance_to(c) <= t.rod_shove_radius_px {
                frog.stun(t.rod_frog_stun_seconds);
            }
        }
        for at in dead {
            f.shocks.push(Shockwave::scaled(at, SHOCK_FROG));
        }
    }

    /// The tiles whose cell reaches into the break radius, in cell order: a
    /// permanent one (iron, a cone, a door) sooted on its face toward the
    /// strike, every other crushed (`DamageCause::Crush`) - a drum going
    /// off at once, one already on a fuse left to it.
    fn rod_tiles(&mut self, f: &mut Frame, c: Position, t: &Tuning) {
        let mut tiles: Vec<((i32, i32), Entity)> = Vec::new();
        for (entity, o) in self.world.query::<(Entity, &mut Obstacle)>().iter() {
            if o.destroyed || rod::cell_reach(c, o.cell()) > t.rod_break_radius_px {
                continue;
            }
            if o.material.is_permanent() {
                if o.material.is_wall() {
                    o.scorched |= face_toward(o.position, c);
                }
                continue;
            }
            tiles.push((o.cell(), entity));
        }
        tiles.sort_by_key(|&(cell, _)| cell);
        for (_, entity) in tiles {
            self.damage_obstacle(f, entity, f32::MAX, DamageCause::Crush { from: c });
        }
    }

    /// What lies on the ground within the break radius: grenades set off,
    /// lanterns broken, the oil trail lit, breakable crates broken, the
    /// tall grass laid flat and hiding nobody for `rod_grass_flat_seconds`.
    fn rod_ground(&mut self, f: &mut Frame, c: Position, t: &Tuning) {
        let reach = t.rod_break_radius_px;
        let mut grenades: Vec<(u32, Entity)> =
            self.world.query::<(Entity, &crate::grenade::Grenade)>().iter().filter(|(_, g)| g.position.distance_to(c) <= reach).map(|(e, g)| (g.id, e)).collect();
        grenades.sort_by_key(|&(id, _)| id);
        for (_, e) in grenades {
            if let Ok(mut g) = self.world.get::<&mut crate::grenade::Grenade>(e) {
                g.fuse = 0.0;
            }
        }
        let mut broken = Vec::new();
        self.lanterns.retain(|l| {
            let hit = l.position.distance_to(c) <= reach;
            if hit {
                broken.push(l.position);
            }
            !hit
        });
        for at in broken {
            f.events.push(Event::LanternBroken { x: at.x, y: at.y });
        }
        let mut lit: Vec<(i32, i32)> = self.oil_cells.iter().copied().filter(|&cell| rod::cell_reach(c, cell) <= reach).collect();
        lit.sort_unstable();
        for cell in lit {
            self.light_cell(f, cell, t.oil_trail_burn_seconds, false);
        }
        self.blast_crates(f, c, &BlastParams { radius: reach, damage: (1.0e6, 1.0e6), knockback: 0.0, by: None });
        if t.rod_grass_flat_seconds > 0.0 && !self.grass_cells.is_empty() {
            let cells: Vec<(i32, i32)> = self.grass_cells.iter().map(|&p| world_to_cell(p)).filter(|&cell| rod::cell_reach(c, cell) <= reach).collect();
            for cell in cells {
                let left = self.grass_flat.entry(cell).or_insert(0.0);
                *left = left.max(t.rod_grass_flat_seconds);
                crate::grass::pin(&mut self.grass, cell, t.rod_grass_flat_seconds);
            }
        }
    }

    /// The cells a rod struck at `cell` leaves a crater on: the struck cell
    /// and its neighbours within `rod_crater_reach` that are in the field,
    /// dry - no water, no lava - and off every volcano's cone.
    pub(crate) fn crater_cells_at(&self, cell: (i32, i32)) -> Vec<(i32, i32)> {
        let t = tuning();
        let (w, h) = self.map.field_size();
        let (cols, rows) = ((w / OBSTACLE_GRID_SIZE).floor() as i32, (h / OBSTACLE_GRID_SIZE).floor() as i32);
        let dry = |c: (i32, i32)| {
            let p = cell_to_world(c.0, c.1);
            self.water.depth_of_cell(c.0, c.1) == crate::ground::Depth::Dry
                && self.lava.depth_at(p) == crate::ground::Depth::Dry
                && !self.volcanoes.iter().any(|v| crate::volcano::in_footprint(v.cell.0, v.cell.1, c.0, c.1))
        };
        rod::crater_cells(cell, t.rod_crater_reach.max(0), cols, rows, dry)
    }

    /// Leave a crater struck at `cell` at round time `at` over `cells`
    /// (docs/rod-from-god.md "The crater"): a pit from now on, and a ford
    /// at once under a sky that fills it (`weather::fills_craters`). The
    /// kept nav grid is built again: a pit and a puddle change prices, not
    /// occupancy. A replica makes the room's the same way.
    pub(crate) fn make_crater(&mut self, cell: (i32, i32), at: f32, cells: &[(i32, i32)]) {
        if !self.craters.add(cell, at, cells) {
            return;
        }
        if crate::weather::fills_craters(self.weather, &tuning()) {
            self.water.fill(cells.iter().copied());
        }
        self.nav.clear();
    }

    /// Fill every crater with water (the sky turned to rain mid-round,
    /// `change_weather`).
    pub(crate) fn fill_craters(&mut self) {
        if self.craters.is_empty() {
            return;
        }
        let cells: Vec<(i32, i32)> = self.craters.cells().collect();
        if self.water.fill(cells) {
            self.nav.clear();
        }
    }

    /// The volcanoes a rod at `c` struck - one whose cone has a cell whose
    /// box reaches into the circle - by their place in `volcanoes`.
    fn struck_volcanoes(&self, c: Position, t: &Tuning) -> Vec<usize> {
        self.volcanoes
            .iter()
            .enumerate()
            .filter(|(_, v)| crate::volcano::footprint(v.cell.0, v.cell.1).any(|cell| rod::cell_reach(c, cell) <= t.rod_kill_radius_px))
            .filter(|(_, v)| v.set_off_shift(self.time, t).is_some())
            .map(|(i, _)| i)
            .collect()
    }

    /// Set off every volcano a rod at `c` struck (docs/rod-from-god.md "A
    /// volcano"): its cycle moved so the eruption it leads up to begins on
    /// the next tick (`Volcano::set_off_shift`).
    fn set_off_volcanoes(&mut self, c: Position, t: &Tuning) {
        for i in self.struck_volcanoes(c, t) {
            if let Some((shift, _)) = self.volcanoes[i].set_off_shift(self.time, t) {
                self.shift_volcano(i, shift);
            }
        }
    }

    /// Move volcano `i`'s cycle by `shift` ticks - the room on a rod's
    /// impact, a replica from the round state - and, standing in the rumble
    /// of an eruption a rod brought forward, mark that rumble shown, so no
    /// tremor plays for it.
    pub(crate) fn shift_volcano(&mut self, i: usize, shift: i32) {
        let t = tuning();
        let Some(v) = self.volcanoes.get_mut(i) else { return };
        if v.shift == shift {
            return;
        }
        v.set_shift(shift);
        let p = v.phase(self.time, &t);
        if p.stage == crate::volcano::Stage::Rumble
            && let Some(shown) = self.eruptions_shown.get_mut(i)
        {
            shown.0 = shown.0.max(p.eruption);
        }
    }

    /// What the rod struck at `c`, for its dust: water, lava, snow and ice,
    /// or dry ground.
    pub(crate) fn ground_struck(&self, c: Position) -> Ground {
        match self.water.depth_at(c) {
            crate::ground::Depth::Shallow | crate::ground::Depth::Deep => Ground::Water,
            crate::ground::Depth::Ice => Ground::Snow,
            crate::ground::Depth::Dry if self.lava.depth_at(c) != crate::ground::Depth::Dry => Ground::Lava,
            crate::ground::Depth::Dry if self.weather == crate::map::Weather::Snow => Ground::Snow,
            crate::ground::Depth::Dry => Ground::Dry,
        }
    }

    /// A rod's impact as it is drawn at `c` - the round's, a replica's off
    /// `Event::RodImpact`: the column, the dust and the debris
    /// (`rod_impacts`), the screen flash, the ripple and the impact flash,
    /// the fireball; on dry ground a scorch and a ring of rubble thrown out
    /// round the crater; the grass laid flat. Hashed, no RNG.
    pub(crate) fn rod_show(&mut self, show: &mut Spectacle, c: Position, ground: Ground) {
        let t = tuning();
        self.rod_impacts.push(RodImpactFx::new(c, ground));
        self.flash_screen_with(t.rod_screen_flash);
        if t.rod_shock > 0.0 {
            show.shocks.push(Shockwave::scaled(c, t.rod_shock));
        }
        show.impact_flashes.push(Shockwave::new(c));
        let mut fx = crate::blast::BlastFx::shaped(c, crate::blast::BlastKind::Fuel, crate::blast::BlastShape::Fire);
        fx.scale *= t.rod_fireball_scale;
        show.blast_fx.push(fx);
        crate::grass::flatten(&mut self.grass, c, t.rod_shove_radius_px);
        if ground == Ground::Dry {
            show.scorches.push(crate::blast::Scorch::with(c, t.rod_scorch_scale, None));
            let seed = crate::blast::seed_at(c, 0x5255);
            let pieces = t.rod_rubble_pieces.max(0) as u32;
            for i in 0..pieces {
                let a = (i as f32 + crate::pyro::unit(seed, i)) / pieces.max(1) as f32 * std::f32::consts::TAU;
                let d = t.rod_kill_radius_px + (t.rod_break_radius_px - t.rod_kill_radius_px).max(0.0) * crate::pyro::unit(seed, 100 + i);
                let to = Position::new(c.x + a.cos() * d, c.y + a.sin() * d);
                let row = if i % 2 == 0 { crate::RUBBLE_ROW_BRICK } else { crate::RUBBLE_ROW_SANDBAG };
                show.decals.push(crate::decal::Decal::thrown(row, c, to, 60 + i * 5));
            }
        }
    }

    /// Every seat's motion record stepped a tick (`rod::SeatStill`): its
    /// hull's centre, or nothing for a wreck or a seat off the field.
    pub(super) fn tick_seat_still(&mut self, dt: f32) {
        let t = tuning();
        let seats = self.seats_on_field();
        for (i, record) in self.seat_still.iter_mut().enumerate() {
            let at = seats.get(i).copied().flatten().and_then(|e| self.world.get::<&Tank>(e).ok().filter(|t| !t.is_wreck()).map(|t| t.position));
            record.step(at, dt, &t);
        }
    }

    /// A room's report of the cell seat `seat`'s client has its reticle on
    /// (`net::mailbox::Mailbox::reticle`), for this update alone: the
    /// reticle stands where the client drew it (docs/rod-from-god.md "The
    /// reticle report").
    pub fn set_seat_reticle(&mut self, seat: usize, cell: Option<(i32, i32)>) {
        if let Some(slot) = self.seat_reticle.get_mut(seat) {
            *slot = cell.map(|c| (self.frame + 1, c));
        }
    }

    /// This update's reticle report for `seat`, if a room set one.
    pub(super) fn seat_reticle_report(&self, seat: usize) -> Option<(i32, i32)> {
        self.seat_reticle.get(seat).copied().flatten().filter(|&(frame, _)| frame == self.frame).map(|(_, c)| c)
    }

    /// Whether some live tank on the field carries an online rod or holds a
    /// rod charge: what the senses are built for.
    pub(super) fn any_rod(&self) -> bool {
        self.world
            .query::<&Tank>()
            .with::<&Ai>()
            .iter()
            .any(|t| !t.is_wreck() && (t.active_weapon() == ActiveWeapon::RodFromGod || t.charge.is_some_and(|c| c.weapon == ActiveWeapon::RodFromGod)))
    }

    /// The dangers every enemy keeps out of because of the zones standing
    /// (docs/rod-from-god.md "Reacting to a call"), in id order.
    pub(super) fn zone_dangers(&self) -> Vec<Danger> {
        let t = tuning();
        self.zones.iter().filter_map(|z| z.danger(&t)).collect()
    }

    /// The nav cells the router surcharges because of the zones standing,
    /// each with its extra cost: every cell centred inside a zone's route
    /// disc (`Zone::route`).
    pub(super) fn zone_route_cells(&self, grid: &crate::pathfind::Grid) -> Vec<(Position, u32)> {
        let t = tuning();
        let (cols, rows, cell) = grid.dims();
        let mut out = Vec::new();
        for z in &self.zones {
            let Some((radius, cost)) = z.route(&t) else { continue };
            let span = |v: f32, n: usize| {
                let lo = (((v - radius) / cell) - 0.5).floor().max(0.0) as usize;
                let hi = ((((v + radius) / cell) - 0.5).ceil().max(0.0) as usize).min(n.saturating_sub(1));
                lo..=hi
            };
            for row in span(z.centre.y, rows) {
                for col in span(z.centre.x, cols) {
                    let at = Position::new((col as f32 + 0.5) * cell, (row as f32 + 0.5) * cell);
                    if at.distance_to(z.centre) <= radius {
                        out.push((at, cost));
                    }
                }
            }
        }
        out
    }

    /// What each rod-carrying enemy that thinks this tick would call
    /// (docs/rod-from-god.md "What it is handed"), in owner-slot order: a
    /// seat standing still, a slow seat led to where it will be, a player
    /// tower, the players' frog - a hunter's quarry before the towers -
    /// each one it knows of, in its reticle's reach, whose circle holds no
    /// ally and no call already covers, and a seat's only from inside that
    /// seat's sight box; one caller per target. No RNG: the preference, then
    /// distance, then the seat, then the cell.
    pub(super) fn rod_senses(&self, seats: &[RodSeat]) -> std::collections::BTreeMap<Entity, RodSense> {
        let t = tuning();
        let (half_w, half_h) = t.sight_box_half_px();
        let field = self.map.field_size();
        let mut out = std::collections::BTreeMap::new();
        // Each rod tank: its slot, where it stands, whether it hunts, is a
        // training dummy, holds a rod's reticle up, and its weapon's
        // cooldown.
        let mut armed: Vec<(usize, Entity, Position, bool, bool, bool, f32)> = self
            .world
            .query::<(Entity, &Tank, &Ai)>()
            .iter()
            .filter(|(_, tank, _)| !tank.is_wreck() && tank.body.is_some())
            .filter(|(_, tank, _)| tank.active_weapon() == ActiveWeapon::RodFromGod || tank.charge.is_some_and(|c| c.weapon == ActiveWeapon::RodFromGod))
            .map(|(e, tank, ai)| {
                let aiming = tank.charge.is_some_and(|c| c.weapon == ActiveWeapon::RodFromGod);
                (tank.owner_slot(), e, tank.position, ai.role == Role::Hunter, ai.frog_only, aiming, tank.fire_cooldown)
            })
            .collect();
        if armed.is_empty() {
            return out;
        }
        armed.sort_by_key(|a| a.0);
        // The allies a call must not hold: every live enemy's hull, and where
        // its motion carries it in a second.
        let allies: Vec<(Entity, Position, Vec2, Position)> = self
            .world
            .query::<(Entity, &Tank)>()
            .with::<&Ai>()
            .iter()
            .filter(|(_, tank)| !tank.is_wreck() && tank.body.is_some())
            .map(|(e, tank)| {
                let (centre, half) = tank.hull_bbox_world();
                let v = tank.body.map_or(Vec2::zero(), |h| self.physics.velocity(h));
                (e, centre, half, centre + v)
            })
            .collect();
        let enemy_towers: Vec<(i32, i32)> = self.standing_towers_of(Side::Enemy);
        let player_towers: Vec<(i32, i32)> = self.standing_towers_of(Side::Player);
        let enemy_frog = self.enemy_frog.and_then(|e| self.world.get::<&Frog>(e).ok().filter(|f| !f.is_dead()).map(|f| f.position));
        let quarry = self.frog.and_then(|e| self.world.get::<&Frog>(e).ok().filter(|f| !f.is_dead()).map(|f| f.position));
        let sight = self.enemy_sight();
        let margin = t.rod_kill_radius_px + t.rod_ai_friend_margin_px;
        let near = |at: Position, &(_, centre, half, ahead): &(Entity, Position, Vec2, Position)| {
            crate::emp::box_reach(at, centre, half) <= margin || crate::emp::box_reach(at, ahead, half) <= margin
        };
        // Whether an ally - the caller too, unless it is `but` - stands in
        // the circle a call on `cell` would crush, now or a second on.
        let holds_ally = |cell: (i32, i32), but: Option<Entity>| {
            let at = cell_to_world(cell.0, cell.1);
            allies.iter().filter(|a| Some(a.0) != but).any(|a| near(at, a))
                || enemy_towers.iter().any(|&tower| rod::cell_reach(at, tower) <= t.rod_break_radius_px)
                || enemy_frog.is_some_and(|f| f.distance_to(at) <= margin + crate::FROG_COLLIDER_HALF_EXTENT.0)
        };
        let covered = |cell: (i32, i32)| {
            let at = cell_to_world(cell.0, cell.1);
            self.zones.iter().any(|z| z.centre.distance_to(at) <= 2.0 * t.rod_kill_radius_px)
        };
        // Whether a call on `cell` from `me` would crush a seat standing by
        // it - a tower's or the frog's neighbour, a couch partner beside a
        // camper - from outside that seat's sight box: a call that holds a
        // seat is a call on that seat, and the box binds it.
        let seat_half = Vec2::new(crate::battlefield::max_tank_clearance_half_extent(), crate::battlefield::max_tank_clearance_half_extent());
        let holds_seat_offbox = |cell: (i32, i32), me: Position| {
            let at = cell_to_world(cell.0, cell.1);
            seats.iter().any(|s| s.live && crate::emp::box_reach(at, s.pos, seat_half) <= margin && !crate::ai::in_sight_box_of((half_w, half_h), s.pos, me))
        };
        let mut taken: Vec<(i32, i32)> = Vec::new();
        for (_, entity, me, hunter, frog_only, aiming, cooldown) in armed {
            let under_call = self.zones.iter().any(|z| z.holds(me, &t));
            let mut sense = RodSense { pick: None, under_call, keep_from: None, self_blocks: false };
            if frog_only {
                out.insert(entity, sense);
                continue;
            }
            let Ok(ai) = self.world.get::<&Ai>(entity) else { continue };
            let range = rod::Range::of(me, field, &t);
            let mut candidates: Vec<(u8, f32, u8, (i32, i32), RodPick)> = Vec::new();
            for s in seats {
                if !s.live || !crate::ai::in_sight_box_of((half_w, half_h), s.pos, me) || me.distance_to(s.pos) > s.sight {
                    continue;
                }
                if s.concealed && !ai.is_hit_alerted() {
                    continue;
                }
                if sense.keep_from.is_none_or(|k| me.distance_to(s.pos) < me.distance_to(k)) {
                    sense.keep_from = Some(s.pos);
                }
                let still = self.seat_still(s.seat as usize);
                let (rank, cell, why) = if still.still >= t.rod_ai_still_seconds {
                    (0, world_to_cell(s.pos), "camper")
                } else if still.speed() <= t.rod_ai_slow_speed && still.speed() >= rod::SEAT_LEAD_MIN_SPEED {
                    let ahead = s.pos + still.velocity * t.rod_countdown_seconds;
                    let inset = OBSTACLE_GRID_SIZE * 0.5;
                    let ahead = Position::new(ahead.x.clamp(inset, field.0 - inset), ahead.y.clamp(inset, field.1 - inset));
                    (1, world_to_cell(ahead), "lead")
                } else {
                    continue;
                };
                candidates.push((rank, me.distance_to(s.pos), s.seat, cell, RodPick { cell, at_seat: Some(s.seat), why }));
            }
            let (tower_rank, frog_rank) = if hunter { (3, 2) } else { (2, 3) };
            for &tower in &player_towers {
                let at = cell_to_world(tower.0, tower.1);
                if me.distance_to(at) <= sight {
                    candidates.push((tower_rank, me.distance_to(at), u8::MAX, tower, RodPick { cell: tower, at_seat: None, why: "tower" }));
                }
            }
            if let Some(frog) = quarry
                && me.distance_to(frog) <= sight
            {
                let cell = world_to_cell(frog);
                candidates.push((frog_rank, me.distance_to(frog), u8::MAX, cell, RodPick { cell, at_seat: None, why: "frog" }));
            }
            candidates.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)).then(a.3.cmp(&b.3)));
            let open = |p: &RodPick| range.holds(p.cell) && p.cell != world_to_cell(me) && !covered(p.cell) && !taken.contains(&p.cell) && !holds_seat_offbox(p.cell, me);
            sense.pick = candidates.iter().map(|c| c.4).find(|p| open(p) && !holds_ally(p.cell, None));
            // With none: a seat it would call on but for its own hull.
            sense.self_blocks = sense.pick.is_none()
                && candidates.iter().map(|c| c.4).any(|p| p.at_seat.is_some() && open(&p) && !holds_ally(p.cell, Some(entity)));
            // The target is this tank's only while it can call on it: its
            // reticle up, or its trigger free to put one up. One still
            // reloading or waiting on its fire timer leaves it to the tanks
            // after it, so a lower slot never keeps the rest from calling.
            if let Some(p) = sense.pick
                && (aiming || (cooldown <= 0.0 && ai.fire_ready()))
            {
                taken.push(p.cell);
            }
            out.insert(entity, sense);
        }
        out
    }

    /// The standing towers of `side`, by cell.
    fn standing_towers_of(&self, side: Side) -> Vec<(i32, i32)> {
        let mut out: Vec<(i32, i32)> = self
            .world
            .query::<&Obstacle>()
            .iter()
            .filter(|o| !o.destroyed && o.material.is_tower() && crate::tower::side_of_variant(o.variant) == side)
            .map(|o| o.cell())
            .collect();
        out.sort_unstable();
        out
    }

    /// The dev server's `rod_call`: a rod called at once on the map cell
    /// nearest `at` - player 1's, the kills credited to it, or with `enemy`
    /// an enemy's - landing `rod_countdown_seconds` from now. Its id; `None`
    /// off the field. No RNG.
    pub fn debug_call_rod(&mut self, at: Position, enemy: bool) -> Option<u32> {
        let (w, h) = self.map.field_size();
        let cell = world_to_cell(at);
        let (cols, rows) = ((w / OBSTACLE_GRID_SIZE).floor() as i32, (h / OBSTACLE_GRID_SIZE).floor() as i32);
        if cell.0 < 0 || cell.1 < 0 || cell.0 >= cols || cell.1 >= rows {
            return None;
        }
        let owner = if enemy { Owner::Enemy(self.first_enemy_slot().max(MAX_SEATS)) } else { Owner::Player(0) };
        let id = self.take_shot_id();
        let seat = (!enemy).then_some(0);
        let zone = Zone { id, kind: ZoneKind::Rod(RodCall { cell, seat }), owner, centre: cell_to_world(cell.0, cell.1), until: self.time + tuning().rod_countdown_seconds };
        self.events.push(Event::RodCalled { id, slot: owner.slot(), seat: crate::net::encode::owner_seat(owner), cell, land: zone.until });
        self.zones.push(zone);
        self.zones.sort_by_key(|z| z.id);
        Some(id)
    }
}

/// Crush a hull (docs/rod-from-god.md "The impact"): a live rainbow shield
/// pops - `drain_shield_breaks` logs it - and the hull is wrecked whole,
/// past the shield, the armour and friendly fire.
fn crush(tank: &mut Tank) {
    if tank.is_shielded() {
        tank.shield_hp = 0.0;
        tank.shield_timer = 0.0;
        tank.shield_recharge_delay = 0.0;
        tank.shield_broke = true;
    }
    tank.damage = MAX_DAMAGE;
}
