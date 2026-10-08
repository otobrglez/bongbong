//! The gravity well's world half (docs/gravity-well.md; `well.rs` is the
//! orb, the field, what a well holds and the pictures). A press launches an
//! orb off the gun line (`fire_well`, put in the world by `place_orbs`); a
//! second press anchors it (`anchor_press` on the trigger), and so does what
//! it meets or its range (`resolve_orbs`). The anchor puts a well on the
//! field as a zone; `well_phase` turns its stage on the clock, lifts the
//! drums in its reach when its pull starts, pulls the frogs and the crates,
//! swirls the tread marks and bows the grass, and collapses it
//! (`collapse_well`) - the drums it holds going off together, the hulls in
//! its reach flung out, the side opposing its owner hurt, frogs hopped out,
//! grenades thrown out in a ring, what flies turned out, crates slid out.
//! The pull on hulls goes through the drive (`Footing::well`), on shots
//! through the fixed-step loop (`Projectile::bend`), on grenades, missiles
//! and drones where they move. The well draws no RNG of its own.

use std::collections::{BTreeMap, BTreeSet};

use hecs::Entity;

use crate::air::AirStrike;
use crate::frog::Frog;
use crate::map::world_to_cell;
use crate::math::Vec2;
use crate::obstacle::{Drum, Material, Obstacle};
use crate::shell::Owner;
use crate::shockwave::Shockwave;
use crate::tank::{ActiveWeapon, Dir, Tank};
use crate::tuning::{tuning, Tuning};
use crate::well::{self, AnchorBy, HeldDrum, Orb, Swallow, WellField, WellFx, WellFxKind, WellStage, WellZone};
use crate::zone::{Zone, ZoneKind};
use crate::{MAX_DAMAGE, OBSTACLE_GRID_SIZE, Position};

use super::props::{BlastShape, PendingBlast};
use super::{Event, Footing, Frame, Game, HitCause, HitTarget, Spectacle};

/// An orb launched this frame, waiting for `place_orbs` to put it in the
/// world.
pub(super) struct PendingOrb {
    pub owner: Owner,
    pub muzzle: Position,
    pub dir: Vec2,
}

/// Launch an orb (docs/gravity-well.md "The orb"): false with no well left.
/// Else a well spent, `well_reload_seconds` before the next launch, the
/// projector's launch cell, `Event::Fired` and the orb queued for
/// `place_orbs`, leaving the gun line's muzzle along the hull's facing.
pub(super) fn fire_well(f: &mut Frame, tank: &mut Tank, owner: Owner) -> bool {
    if tank.wells <= 0 || tank.orb.is_some() {
        return false;
    }
    tank.wells -= 1;
    tank.fire_cooldown = tuning().well_reload_seconds;
    tank.kick_well();
    f.events.push(Event::Fired { slot: tank.owner_slot(), weapon: ActiveWeapon::GravityWell.name() });
    let dir = Dir::from_rotation(tank.rotation).unwrap_or(Dir::Up).vec();
    f.pending_orbs.push(PendingOrb { owner, muzzle: tank.gun_line_muzzle(dir), dir });
    true
}

/// A press that anchors `tank`'s orb, when `well::anchor_press` lets it:
/// queued for `place_orbs` (which anchors it where it stands this tick),
/// the projector's anchor cell. True if the press was the anchor's - or a
/// press inside the grace after its orb anchored itself, which does
/// nothing at all.
pub(super) fn take_anchor_press(f: &mut Frame, tank: &mut Tank) -> bool {
    if let Some(id) = tank.orb
        && well::anchor_press(tank)
    {
        f.pending_anchors.push(id);
        tank.kick_well_anchor();
        return true;
    }
    tank.orb.is_none() && tank.well_grace > 0.0
}

impl Game {
    /// The orbs in flight, sorted by id.
    pub fn orbs(&self) -> &[Orb] {
        &self.orbs
    }

    /// The drums the wells hold, sorted by id.
    pub fn held_drums(&self) -> &[HeldDrum] {
        &self.held_drums
    }

    /// One fixed step of the wells on what flies over the ground (from
    /// `step_world`, after it moved): every missile in flight and every
    /// drone in the air dragged by `well_air_pull` toward the cores
    /// (`Missile::drag`, `Drone::drag`), and one whose ground point is now
    /// within `well_core_px` of a core swallowed - a missile removed with no
    /// blast, a drone downed (`AirStrike::Well`). In id order.
    pub(super) fn pull_air(&mut self, f: &mut Frame, dt: f32) {
        if f.wells.is_empty() {
            return;
        }
        let t = tuning();
        let field = &f.wells;
        let core = |p: Position| field.sources.iter().any(|&(_, c)| p.distance_to(c) <= t.well_core_px);
        let mut swallowed: Vec<(u32, Entity, Position)> = Vec::new();
        for (entity, m) in self.world.query_mut::<(Entity, &mut crate::missile::Missile)>() {
            if m.arrived {
                continue;
            }
            if core(m.position) {
                swallowed.push((m.id, entity, m.position));
                continue;
            }
            let accel = field.pull(m.position, &t) * t.well_air_pull;
            if accel.x != 0.0 || accel.y != 0.0 {
                m.drag(accel, dt);
            }
        }
        swallowed.sort_by_key(|&(id, ..)| id);
        for (_, entity, at) in swallowed {
            self.world.despawn(entity).ok();
            f.events.push(Event::Swallowed { what: Swallow::Missile, x: at.x, y: at.y });
            self.swallow_show(at);
        }
        let mut downed: Vec<(u32, Position)> = Vec::new();
        for d in self.world.query_mut::<&mut crate::fpv::Drone>() {
            if !d.in_air() {
                continue;
            }
            if core(d.ground) {
                downed.push((d.id, d.ground));
                continue;
            }
            let accel = field.pull(d.ground, &t) * t.well_air_pull;
            if accel.x != 0.0 || accel.y != 0.0 {
                d.drag(accel, dt);
            }
        }
        downed.sort_by_key(|&(id, _)| id);
        for (id, at) in downed {
            if self.strike_air(f, crate::air::AirKey::Drone(id), AirStrike::Well) {
                f.events.push(Event::Swallowed { what: Swallow::Drone, x: at.x, y: at.y });
                self.swallow_show(at);
            }
        }
    }

    /// The wells' moments being drawn (`WellFx`).
    pub fn well_fx(&self) -> &[WellFx] {
        &self.well_fx
    }

    /// The field this tick (`Frame::wells`'s copy): what the drive, the
    /// cosmetic drain and the tooling read.
    pub fn well_field(&self) -> &WellField {
        &self.well_field
    }

    /// The owner slot's tank, if it stands in the world.
    fn tank_of(&self, owner: Owner) -> Option<Entity> {
        let slot = owner.slot();
        self.world.query::<(Entity, &Tank)>().iter().find(|(_, t)| t.owner_slot() == slot).map(|(e, _)| e)
    }

    /// Put this frame's orbs in the world - each with an id from the
    /// projectile counter, its tank told it has one out - then anchor every
    /// orb a press this frame asked for, where it stands.
    pub(super) fn place_orbs(&mut self, f: &mut Frame) {
        let t = tuning();
        for pending in std::mem::take(&mut f.pending_orbs) {
            let mut orb = Orb::launch(pending.muzzle, pending.dir, pending.owner, &t);
            orb.id = self.take_shot_id();
            orb.rewind = self.seat_rewind(pending.owner);
            if let Some(e) = self.tank_of(pending.owner)
                && let Ok(mut tank) = self.world.get::<&mut Tank>(e)
            {
                tank.orb = Some(orb.id);
            }
            self.orbs.push(orb);
        }
        for id in std::mem::take(&mut f.pending_anchors) {
            if let Some(i) = self.orbs.iter().position(|o| o.id == id) {
                let at = self.orbs[i].position;
                self.anchor_orb(f, i, at, AnchorBy::Press);
            }
        }
    }

    /// The orb at `index` anchors at `at`: it leaves the world and a
    /// forming well stands there - its id the orb's, `until` the pull's
    /// start - with `Event::WellAnchored` and the snap. Its tank no longer
    /// has an orb out; one that anchored by itself swallows the next press
    /// for `well_anchor_grace_seconds`.
    fn anchor_orb(&mut self, f: &mut Frame, index: usize, at: Position, by: AnchorBy) {
        let t = tuning();
        let orb = self.orbs.remove(index);
        let seat = match orb.owner {
            Owner::Player(seat) => Some(seat),
            _ => None,
        };
        if let Some(e) = self.tank_of(orb.owner)
            && let Ok(mut tank) = self.world.get::<&mut Tank>(e)
            && tank.orb == Some(orb.id)
        {
            tank.orb = None;
            if by != AnchorBy::Press {
                tank.well_grace = t.well_anchor_grace_seconds;
            }
        }
        let zone = Zone {
            id: orb.id,
            kind: ZoneKind::Well(WellZone { stage: WellStage::Forming, by, seat, emp_collapse: false }),
            owner: orb.owner,
            centre: at,
            until: self.time + t.well_form_seconds,
        };
        f.events.push(Event::WellAnchored { id: orb.id, slot: orb.owner.slot(), seat: crate::net::encode::owner_seat(orb.owner), x: at.x, y: at.y, by });
        self.zones.push(zone);
        self.zones.sort_by_key(|z| z.id);
        let mut show = Spectacle::default();
        self.well_anchor_show(&mut show, at);
        f.stage(show);
    }

    /// An orb leaves the world with nothing anchored (`OrbFizzled`, or
    /// swallowed): its tank no longer has one out.
    fn drop_orb(&mut self, index: usize) -> Orb {
        let orb = self.orbs.remove(index);
        if let Some(e) = self.tank_of(orb.owner)
            && let Ok(mut tank) = self.world.get::<&mut Tank>(e)
            && tank.orb == Some(orb.id)
        {
            tank.orb = None;
        }
        orb
    }

    /// The orb `id` goes out in flight - an EMP's ring - with no well
    /// (`Event::OrbFizzled`).
    pub(super) fn fizzle_orb(&mut self, f: &mut Frame, id: u32) {
        let Some(i) = self.orbs.iter().position(|o| o.id == id) else { return };
        let orb = self.drop_orb(i);
        f.events.push(Event::OrbFizzled { id, x: orb.position.x, y: orb.position.y });
        self.swallow_show(orb.position);
    }

    /// One fixed step of every orb in flight: bent by the field (the wells
    /// already pulling), then straight on. Its stretch of the tick starts
    /// where it stood.
    pub(super) fn advance_orbs(&mut self, dt: f32, field: &WellField) {
        let t = tuning();
        for orb in &mut self.orbs {
            orb.prev_position = orb.position;
            if !field.is_empty() {
                orb.bend(field.shot_accel(orb.position, &t), dt);
            }
            orb.advance(dt);
        }
    }

    /// Every orb in flight, in id order, after this tick's step: one whose
    /// stretch passes a pulling well's core is swallowed; one whose sweep
    /// meets a hull (not its shooter's), a frog, a tile that blocks sight or
    /// the field's edge anchors there, backed off its half width; one that
    /// has flown its range anchors where it is.
    pub(super) fn resolve_orbs(&mut self, f: &mut Frame) {
        if self.orbs.is_empty() {
            return;
        }
        let t = tuning();
        let players = self.seats_on_field();
        // The tiles an orb floats over: those that do not block sight.
        let low: Vec<Entity> = self
            .world
            .query::<(Entity, &Obstacle)>()
            .iter()
            .filter(|(_, o)| !o.destroyed && !o.material.blocks_sight())
            .map(|(e, _)| e)
            .collect();
        let field = f.wells.clone();
        let mut i = 0;
        while i < self.orbs.len() {
            let orb = self.orbs[i];
            if let Some((_, k)) = field.core_hit(orb.prev_position, orb.position, &t) {
                let at = orb.prev_position + (orb.position - orb.prev_position) * k;
                self.drop_orb(i);
                f.events.push(Event::Swallowed { what: Swallow::Orb, x: at.x, y: at.y });
                self.swallow_show(at);
                continue;
            }
            let past = self.rewound_boxes(orb.rewind);
            let hit = f.terrain.sweep_rewound(&self.world, players, orb.owner, orb.prev_position, orb.position, t.well_orb_half_px, &low, past);
            if let Some((_, k)) = hit {
                let travel = orb.position - orb.prev_position;
                let len = travel.length();
                let back = if len > 1e-4 { travel * ((t.well_orb_half_px + 2.0) / len) } else { Vec2::zero() };
                let at = orb.prev_position + travel * k - back;
                self.anchor_orb(f, i, at, AnchorBy::Contact);
                continue;
            }
            if orb.flown >= t.well_orb_range_px {
                self.anchor_orb(f, i, orb.position, AnchorBy::Range);
                continue;
            }
            i += 1;
        }
    }

    /// The wells, in id order (docs/gravity-well.md "The well"): an EMP's
    /// collapse first; a forming well whose time has come starts pulling -
    /// its `until` now the collapse - and lifts the drums in its reach; a
    /// pulling well whose time has come collapses. Then, with `live`, the
    /// frogs and the crates are pulled; and either way a held drum's fuse
    /// burns, the tread marks swirl and the grass bows. On the end screen
    /// (`live` false) a well forms and collapses as a show: it lifts, pulls,
    /// flings and hurts nothing, and the drums it already holds go off as
    /// blasts that hurt nobody.
    pub(super) fn well_phase(&mut self, f: &mut Frame, live: bool) {
        let t = tuning();
        if !self.zones.iter().any(|z| z.well().is_some()) && self.held_drums.is_empty() && self.pickups_drifting.is_empty() {
            return;
        }
        let now = self.time;
        let ids: Vec<u32> = self.zones.iter().filter(|z| z.well().is_some()).map(|z| z.id).collect();
        for id in ids {
            let Some(i) = self.zones.iter().position(|z| z.id == id) else { continue };
            let zone = self.zones[i];
            let Some(w) = zone.well() else { continue };
            if w.emp_collapse {
                self.zones.remove(i);
                self.collapse_well(f, zone, live, true);
                continue;
            }
            match w.stage {
                WellStage::Forming if well::due(zone.until, now) => {
                    let pulling = Zone {
                        kind: ZoneKind::Well(WellZone { stage: WellStage::Pulling, ..w }),
                        until: zone.until + t.well_pull_seconds,
                        ..zone
                    };
                    self.zones[i] = pulling;
                    if live {
                        self.lift_drums(f, &pulling);
                    }
                }
                WellStage::Pulling if well::due(zone.until, now) => {
                    self.zones.remove(i);
                    self.collapse_well(f, zone, live, false);
                }
                _ => {}
            }
        }
        if live {
            self.pull_frogs(f, &t);
            self.slide_pickups(f, &t);
        }
        // A held drum's fuse burns on, on the end screen too, as a tile's
        // does (`tick_fuses`).
        self.burn_held_fuses(f);
        let field = std::mem::take(&mut f.wells);
        self.drain_marks(f.dt, &field);
        self.lean_grass(&field);
        f.wells = field;
    }

    /// Lift every standing drum whose cell centre lies within `zone`'s reach
    /// out of its cell: the tile dies with no blast (`ObstacleDestroyed`,
    /// the hammer's throw's way) and the drum is held, spiralling in to the
    /// ring (`well::held_at`). In cell order; a drum another well already
    /// holds is no tile.
    fn lift_drums(&mut self, f: &mut Frame, zone: &Zone) {
        let t = tuning();
        let mut drums: Vec<(Entity, (i32, i32), i32, Option<f32>, Position)> = self
            .world
            .query::<(Entity, &Obstacle)>()
            .iter()
            .filter(|(_, o)| !o.destroyed && o.material.is_explosive())
            .filter(|(_, o)| o.position.distance_to(zone.centre) <= t.well_radius_px)
            .map(|(e, o)| (e, o.cell(), o.variant, o.fuse.map(|fz| fz.left), o.position))
            .collect();
        drums.sort_by_key(|d| d.1);
        for (entity, cell, variant, fuse, at) in drums {
            if let Ok(mut o) = self.world.get::<&mut Obstacle>(entity) {
                o.destroyed = true;
            }
            f.events.push(Event::ObstacleDestroyed { material: Material::Barrel, x: at.x, y: at.y });
            let id = self.take_shot_id();
            self.held_drums.push(HeldDrum { id, well: zone.id, cell, drum: Drum::from_variant(variant), fuse, lifted_at: self.time, centre: zone.centre });
        }
    }

    /// A held drum whose fuse runs out goes off where it is.
    fn burn_held_fuses(&mut self, f: &mut Frame) {
        let dt = f.dt;
        let mut gone = Vec::new();
        for d in &mut self.held_drums {
            if let Some(left) = d.fuse.as_mut() {
                *left -= dt;
                if *left <= 0.0 {
                    gone.push(d.id);
                }
            }
        }
        for id in gone {
            self.set_off_held(f, id);
        }
    }

    /// The held drum `id` goes off where it is: a chained blast of its kind
    /// (`tick_launches`' path).
    fn set_off_held(&mut self, f: &mut Frame, id: u32) {
        let t = tuning();
        let Some(i) = self.held_drums.iter().position(|d| d.id == id) else { return };
        let drum = self.held_drums.remove(i);
        let centre = drum.centre;
        let (at, _) = well::held_at(&drum, centre, self.time, &t);
        f.events.push(Event::Blast { x: at.x, y: at.y, chained: true, drum: drum.drum });
        f.pending_blasts.push(PendingBlast { center: at, drum: drum.drum, shape: BlastShape::Chained { from: centre, smoulder: 1.0 } });
    }

    /// A drum's blast at `center` reaching `radius` sets off every held drum
    /// whose ground point it covers - the chain (`apply_blast`).
    pub(super) fn chain_held_drums(&mut self, f: &mut Frame, center: Position, radius: f32) {
        if self.held_drums.is_empty() {
            return;
        }
        let t = tuning();
        let now = self.time;
        let hit: Vec<u32> = self
            .held_drums
            .iter()
            .filter(|d| well::held_at(d, d.centre, now, &t).0.distance_to(center) <= radius)
            .map(|d| d.id)
            .collect();
        for id in hit {
            self.set_off_held(f, id);
        }
    }

    /// A well collapses (docs/gravity-well.md "The collapse"), in this
    /// order: the drums it holds go off together where they circle; with
    /// `live`, every live hull in its reach is flung out (`knock_from`) and
    /// the side opposing its owner hurt, and frogs hop out, the opposing
    /// side's hurt; grenades it holds are thrown out in a ring; what flies
    /// in its reach is turned straight out; with `live`, crates slide out.
    /// On the end screen (`live` false) the drums' blasts hurt nobody
    /// (`explosions(f, false)`) and nothing on the ground is moved.
    /// `Event::WellCollapsed` and the show either way.
    fn collapse_well(&mut self, f: &mut Frame, zone: Zone, live: bool, early: bool) {
        let t = tuning();
        let c = zone.centre;
        let r = t.well_radius_px;
        f.events.push(Event::WellCollapsed { id: zone.id, x: c.x, y: c.y, early });
        let held: Vec<u32> = self.held_drums.iter().filter(|d| d.well == zone.id).map(|d| d.id).collect();
        for id in held {
            self.set_off_held(f, id);
        }
        if live {
            self.collapse_hulls(f, c, zone.owner, &t);
            self.collapse_frogs(f, c, zone.owner, &t);
        }
        // Grenades on the ring: out along their own radials, lobbed.
        for g in self.world.query_mut::<&mut crate::grenade::Grenade>() {
            if g.orbit.is_some_and(|o| o.well == zone.id) {
                g.orbit = None;
                let off = g.position - c;
                let len = off.length();
                let dir = if len > 1e-3 { off / len } else { Vec2::new(0.0, -1.0) };
                g.velocity = dir * t.well_fling_grenade_speed;
                g.climb = t.well_fling_grenade_climb;
            }
        }
        self.turn_out_flying(c, r);
        if live {
            for p in self.world.query_mut::<&mut crate::pickup::Pickup>() {
                let at = p.at();
                let d = at.distance_to(c);
                if d < r {
                    let off = at - c;
                    let dir = if d > 1e-3 { off / d } else { Vec2::new(0.0, -1.0) };
                    p.slide = dir * (t.well_fling_crate_speed * (1.0 - d / r));
                }
            }
            self.refresh_drifting();
        }
        let mut show = Spectacle::default();
        self.well_collapse_show(&mut show, c);
        f.stage(show);
    }

    /// The collapse's hulls: every live one on the field with its box's
    /// nearest point within the reach knocked out from the centre, falling
    /// off to nothing at the reach; the side opposing `owner` hurt by
    /// `well_collapse_damage` the same way, no roll.
    fn collapse_hulls(&mut self, f: &mut Frame, c: Position, owner: Owner, t: &Tuning) {
        let r = t.well_radius_px;
        for entity in self.hulls_in_order() {
            let Ok(mut tank) = self.world.get::<&mut Tank>(entity) else { continue };
            let (centre, half) = tank.hull_bbox_world();
            let d = crate::emp::box_reach(c, centre, half);
            if d > r || owner.same_side(tank.owner()) || t.well_collapse_damage <= 0.0 {
                continue;
            }
            let landed = tank.take_damage(t.well_collapse_damage * (1.0 - d / r), MAX_DAMAGE);
            tank.mark_hit();
            tank.credit(owner);
            let killed = tank.is_wreck();
            let target = match tank.owner() {
                Owner::Player(player) => HitTarget::Player { player },
                Owner::Enemy(slot) => HitTarget::Enemy { slot },
                Owner::Tower { .. } => unreachable!("no tank is owned by a tower"),
            };
            let at = tank.position;
            f.events.push(Event::Hit { target, damage: landed, killed, x: at.x, y: at.y, cause: HitCause::Well });
            if killed {
                f.kills.push((at, tank.owner()));
            } else if let Ok(mut ai) = self.world.get::<&mut crate::ai::Ai>(entity) {
                ai.notify_hit();
            }
        }
        self.knock_from(f, c, -1.0, r, |mass_factor, d| {
            let falloff = (1.0 - d / r).max(0.0);
            (t.well_fling_speed * falloff / mass_factor.max(0.05).powf(t.well_fling_mass_exponent)).min(t.well_fling_max_speed)
        });
    }

    /// The collapse's frogs: each in the reach hops out along the line from
    /// the centre (`well::frog_fling_target`, no jitter); the side opposing
    /// `owner` takes the collapse's damage too.
    fn collapse_frogs(&mut self, f: &mut Frame, c: Position, owner: Owner, t: &Tuning) {
        let r = t.well_radius_px;
        let terrain = &f.terrain;
        let mut hurt = Vec::new();
        for frog in self.world.query_mut::<&mut Frog>() {
            if frog.is_dead() {
                continue;
            }
            let d = frog.position.distance_to(c);
            if d >= r {
                continue;
            }
            frog.pulled = false;
            let cells = ((t.well_fling_frog_cells as f32) * (1.0 - d / r)).round().max(1.0) as i32;
            if let Some(to) = well::frog_fling_target(c, frog.position, cells, |p| terrain.frog_fits(p)) {
                frog.start_hop(to);
            }
            let side_owner = match frog.side {
                crate::frog::Side::Player => Owner::Player(0),
                crate::frog::Side::Enemy => Owner::Enemy(0),
            };
            if !owner.same_side(side_owner) && t.well_collapse_damage > 0.0 {
                let dmg = t.well_collapse_damage * (1.0 - d / r);
                frog.damage(dmg);
                hurt.push((frog.side, frog.position, dmg, frog.is_dead()));
            }
        }
        for (side, at, dmg, killed) in hurt {
            f.events.push(Event::Hit { target: HitTarget::Frog { side }, damage: dmg, killed, x: at.x, y: at.y, cause: HitCause::Well });
        }
    }

    /// Every shot, orb, missile and drone within `r` of `c` turned straight
    /// out from it at its own speed (the collapse's fling).
    fn turn_out_flying(&mut self, c: Position, r: f32) {
        fn out(p: Position, c: Position, v: Vec2) -> Option<Vec2> {
            let off = p - c;
            let d = off.length();
            (d > 1e-3).then(|| off * (v.length() / d))
        }
        for s in self.world.query_mut::<&mut crate::shell::Shell>() {
            if s.position.distance_to(c) < r
                && let Some(v) = out(s.position, c, s.velocity)
            {
                s.velocity = v;
                s.rotation = well::heading_deg(v);
            }
        }
        for b in self.world.query_mut::<&mut crate::bullet::Bullet>() {
            if b.position.distance_to(c) < r
                && let Some(v) = out(b.position, c, b.velocity)
            {
                b.velocity = v;
                b.rotation = well::heading_deg(v);
            }
        }
        for p in self.world.query_mut::<&mut crate::plasma::Plasma>() {
            if p.position.distance_to(c) < r
                && let Some(v) = out(p.position, c, p.velocity)
            {
                p.velocity = v;
                p.rotation = well::heading_deg(v);
            }
        }
        for o in &mut self.orbs {
            if o.position.distance_to(c) < r
                && let Some(v) = out(o.position, c, o.velocity)
            {
                o.velocity = v;
                o.rotation = well::heading_deg(v);
            }
        }
        for m in self.world.query_mut::<&mut crate::missile::Missile>() {
            if m.position.distance_to(c) < r {
                m.turn_out_from(c);
            }
        }
        for d in self.world.query_mut::<&mut crate::fpv::Drone>() {
            if d.in_air() && d.ground.distance_to(c) < r {
                d.turn_out_from(c);
            }
        }
    }

    /// The frogs in a pull, either side's, alive: pinned (no hop) and slid
    /// toward the core at `well_frog_speed` times the field's pull, wherever
    /// a frog fits; its collider moved with it.
    fn pull_frogs(&mut self, f: &mut Frame, t: &Tuning) {
        let field = &f.wells;
        let terrain = &f.terrain;
        let mut moved = Vec::new();
        for frog in self.world.query_mut::<&mut Frog>() {
            if frog.is_dead() {
                frog.pulled = false;
                continue;
            }
            let pull = if field.is_empty() { Vec2::zero() } else { field.pull(frog.position, t) };
            if pull.x == 0.0 && pull.y == 0.0 {
                frog.pulled = false;
                continue;
            }
            frog.pulled = true;
            let to = frog.position + pull * (t.well_frog_speed * f.dt);
            if terrain.frog_fits(to) {
                frog.position = to;
                moved.push((frog.body, to));
            }
        }
        for (body, to) in moved {
            self.physics.set_position(body, to);
        }
    }

    /// The pickups: one sliding (the collapse's) slides on, braking; one in
    /// a pull drifts toward the core at `well_crate_speed` times the
    /// field's pull. Either stops where its centre would enter a solid
    /// cell, deep water, deep lava or the field's half-cell inset. It keeps
    /// its slot (`Pickup::drift`).
    fn slide_pickups(&mut self, f: &mut Frame, t: &Tuning) {
        let field = f.wells.clone();
        if field.is_empty() && self.pickups_drifting.is_empty() {
            return;
        }
        let solid: BTreeSet<(i32, i32)> = self.world.query::<&Obstacle>().iter().filter(|o| !o.destroyed).map(|o| o.cell()).collect();
        let (w, h) = (f.width, f.height);
        let inset = OBSTACLE_GRID_SIZE * 0.5;
        let water = &self.water;
        let lava = &self.lava;
        let free = |p: Position| {
            (inset..=w - inset).contains(&p.x)
                && (inset..=h - inset).contains(&p.y)
                && !solid.contains(&world_to_cell(p))
                && water.depth_at(p) != crate::ground::Depth::Deep
                && lava.depth_at(p) != crate::ground::Depth::Deep
        };
        let dt = f.dt;
        for p in self.world.query_mut::<&mut crate::pickup::Pickup>() {
            let speed = p.slide.length();
            if speed > 0.0 {
                let to = p.at() + p.slide * dt;
                if free(to) {
                    p.drift = p.drift + p.slide * dt;
                    let fall = (t.well_crate_friction * dt).min(speed);
                    p.slide = p.slide * ((speed - fall) / speed);
                    if p.slide.length() < 1.0 {
                        p.slide = Vec2::zero();
                    }
                } else {
                    p.slide = Vec2::zero();
                }
                continue;
            }
            if field.is_empty() {
                continue;
            }
            let pull = field.pull(p.at(), t);
            if pull.x == 0.0 && pull.y == 0.0 {
                continue;
            }
            let step = pull * (t.well_crate_speed * dt);
            if free(p.at() + step) {
                p.drift = p.drift + step;
            }
        }
        self.refresh_drifting();
    }

    /// Keep `pickups_drifting` - whether any pickup still slides - up to
    /// date, so a round with none skips the walk.
    fn refresh_drifting(&mut self) {
        self.pickups_drifting.clear();
        for p in self.world.query::<&crate::pickup::Pickup>().iter() {
            if p.slide.x != 0.0 || p.slide.y != 0.0 {
                self.pickups_drifting.push(p.position);
            }
        }
    }

    /// The drain (the "at 11", cosmetic): tread marks within a pulling
    /// well's reach creep toward its core and turn about it, clockwise; one
    /// that reaches the core is gone. Run by `well_phase`'s round and by a
    /// replica's `tick_presentation`, from the field alike. No RNG.
    pub(crate) fn drain_marks(&mut self, dt: f32, field: &WellField) {
        if field.is_empty() {
            return;
        }
        let t = tuning();
        for track in &mut self.tracks {
            let Some((_, c, s)) = field.strongest(track.position, &t) else { continue };
            let off = track.position - c;
            let d = off.length();
            if d <= t.well_core_px {
                track.age = f32::MAX * 0.5;
                continue;
            }
            let turn = t.well_mark_twist * s * dt;
            let (sin, cos) = turn.sin_cos();
            let rotated = Vec2::new(off.x * cos - off.y * sin, off.x * sin + off.y * cos);
            let pulled = rotated * ((d - t.well_mark_speed * s * dt).max(0.0) / d);
            track.position = c + pulled;
            track.rotation += turn.to_degrees();
        }
    }

    /// Tall grass within a pulling well's reach bows toward its core's side
    /// and lies half flat (cosmetic; `GrassTuft::push` and `crush`, which
    /// `tick_grass` recovers once the pull is gone).
    pub(crate) fn lean_grass(&mut self, field: &WellField) {
        if field.is_empty() {
            return;
        }
        let t = tuning();
        for tuft in &mut self.grass {
            let Some((_, c, s)) = field.strongest(tuft.base, &t) else { continue };
            let side = if c.x >= tuft.base.x { 1.0 } else { -1.0 };
            tuft.push = side * t.well_grass_lean_px * s;
            tuft.crush = tuft.crush.max(0.5 * s);
        }
    }

    /// The cosmetic half of an anchor, for the round and for a replica's
    /// `WellAnchored`: the snap on the list and the ripple pinched inward,
    /// running in from the reach.
    pub(crate) fn well_anchor_show(&mut self, show: &mut Spectacle, at: Position) {
        let t = tuning();
        self.well_fx.push(WellFx { kind: WellFxKind::Snap, at, age: 0.0 });
        show.shocks.push(Shockwave::inward(at, t.well_snap_shock, t.well_radius_px));
    }

    /// The cosmetic half of a collapse, for the round and for a replica's
    /// `WellCollapsed`: the implosion and its rings on the list, the
    /// outward ripple, the screen flash.
    pub(crate) fn well_collapse_show(&mut self, show: &mut Spectacle, at: Position) {
        let t = tuning();
        self.well_fx.push(WellFx { kind: WellFxKind::Collapse, at, age: 0.0 });
        show.shocks.push(Shockwave::scaled(at, t.well_shock));
        self.flash_screen_with(t.well_screen_flash);
    }

    /// The projector's launch or anchor cell on `seat`'s tank, for a
    /// client drawing its own press (`net::round`).
    pub(crate) fn flash_seat_well(&mut self, seat: u8, anchor: bool) {
        let Some(entity) = self.seats.get(seat as usize).copied().flatten() else { return };
        if let Ok(mut tank) = self.world.get::<&mut Tank>(entity) {
            if anchor { tank.kick_well_anchor() } else { tank.kick_well() }
        }
    }

    /// An anchor drawn by a client ahead of the room's - its own press or
    /// its own orb's contact - or the room's one it did not draw: the snap
    /// and the anchor cell on `seat`'s projector.
    pub(crate) fn show_well_anchor(&mut self, seat: u8, at: Position) {
        self.flash_seat_well(seat, true);
        let mut show = Spectacle::default();
        self.well_anchor_show(&mut show, at);
        self.show(show);
    }

    /// A swallow's pop on the list.
    pub(crate) fn swallow_show(&mut self, at: Position) {
        self.well_fx.push(WellFx { kind: WellFxKind::Swallow, at, age: 0.0 });
    }

    /// Age the wells' moments and drop the spent ones (`tick_effects`).
    pub(super) fn tick_well_fx(&mut self, dt: f32) {
        for fx in &mut self.well_fx {
            fx.age += dt;
        }
        self.well_fx.retain(|fx| !fx.done());
    }

    /// The wells pulling at this game's present - its clock plus the zones'
    /// lead (`set_zone_lead`): what a client's sandbox bends its own shots
    /// by and a replica's cosmetics read. Empty with no well.
    pub(crate) fn present_wells(&self) -> WellField {
        if self.zones.is_empty() { WellField::default() } else { WellField::at(&self.zones, self.time + self.zone_lead) }
    }

    /// Where one seat's orb would leave from right now and which way: the
    /// sandbox's gun-line muzzle and facing, for a client drawing its own
    /// press (`net::predict`).
    pub(crate) fn seat_orb(&self, seat: usize) -> Option<(Position, Vec2)> {
        let entity = self.seats.get(seat).copied().flatten()?;
        let tank = self.world.get::<&Tank>(entity).ok()?;
        let dir = Dir::from_rotation(tank.rotation).unwrap_or(Dir::Up).vec();
        Some((tank.gun_line_muzzle(dir), dir))
    }

    /// A well anchored at once at the cell centre nearest `at` (the dev
    /// server's `well_at`): player 1's, or with `enemy` the first enemy
    /// slot's; forming. `None` off the field or inside a solid cell. No
    /// RNG.
    pub fn debug_well(&mut self, at: Position, enemy: bool) -> Option<(u32, f32)> {
        let (w, h) = self.map.field_size();
        if !(0.0..w).contains(&at.x) || !(0.0..h).contains(&at.y) {
            return None;
        }
        let (col, row) = world_to_cell(at);
        if self.map.solid_at(col, row) {
            return None;
        }
        let t = tuning();
        let owner = if enemy { Owner::Enemy(self.first_enemy_slot().max(crate::MAX_SEATS)) } else { Owner::Player(0) };
        let id = self.take_shot_id();
        let seat = (!enemy).then_some(0);
        let until = self.time + t.well_form_seconds;
        let zone = Zone { id, kind: ZoneKind::Well(WellZone { stage: WellStage::Forming, by: AnchorBy::Press, seat, emp_collapse: false }), owner, centre: at, until };
        self.events.push(Event::WellAnchored { id, slot: owner.slot(), seat: crate::net::encode::owner_seat(owner), x: at.x, y: at.y, by: AnchorBy::Press });
        self.zones.push(zone);
        self.zones.sort_by_key(|z| z.id);
        let mut show = Spectacle::default();
        self.well_anchor_show(&mut show, at);
        self.show(show);
        Some((id, until))
    }
}

/// A hull a well pulls looks a step's travel ahead, as a skidding one does
/// (`sonic::skid_look_ahead`, which runs first and this only widens): the
/// current along its tracks and the side pull against their grip drive it
/// at whatever stands between it and the core, at up to the body's speed
/// cap, and rapier, looking 0.02 px ahead, would otherwise let a step carry
/// it up to four and a half pixels into a wall or past the field's edge;
/// with it, under a pixel on the step it first meets the face, pushed back
/// out within a second. Run for every hull before every solver step, the
/// room's and a client's sandbox's; with no well pulling, nothing is
/// touched.
pub(super) fn pull_look_ahead(physics: &mut crate::physics::Physics, tank: &Tank, field: &WellField) {
    if field.is_empty() || tank.is_wreck() {
        return;
    }
    let Some(handle) = tank.body else { return };
    if field.strongest(tank.position, &tuning()).is_some() {
        let ahead = physics.max_step_travel();
        physics.set_look_ahead(handle, ahead);
    }
}

/// A seat as the well's senses read it (`Game::well_senses`).
pub(super) struct WellSeat {
    pub seat: u8,
    pub entity: Entity,
    pub pos: Position,
    pub live: bool,
    pub concealed: bool,
    /// How far an enemy sees it (`Game::sight_on`).
    pub sight: f32,
}

/// The step between two anchor points the senses weigh along a line
/// (docs/gravity-well.md "The candidates").
const WELL_AI_STEP_PX: f32 = 16.0;

/// The step a seat's line to an anchor is sampled at for trouble: the
/// hammer's.
const TROUBLE_STEP_PX: f32 = 8.0;

impl Game {
    /// Whether some live enemy on the field carries an online well or has
    /// an orb out: what the senses are built for.
    pub(super) fn any_well(&self) -> bool {
        self.world
            .query::<&Tank>()
            .with::<&crate::ai::Ai>()
            .iter()
            .any(|t| !t.is_wreck() && t.body.is_some() && ((t.active_weapon() == ActiveWeapon::GravityWell && !t.special_down()) || t.orb.is_some()))
    }

    /// What each well-carrying enemy would launch for, and its orb in
    /// flight (docs/gravity-well.md "What it is handed"), in owner-slot
    /// order: along its facing and then `Dir::ALL`, the anchor points
    /// `WELL_AI_STEP_PX` apart from `well_ai_min_px` out of the muzzle to
    /// the orb's range or its stopper, each weighed by its arms - two seats
    /// pulled together, a seat pulled into trouble, a seat pulled off the
    /// frog it guards, a well across the line a seat shoots it along - and
    /// held to dragging no more allies than seats, to no other well within
    /// twice the reach, and to one well a seat. A seat counts only from
    /// inside its sight box, within the tank's sight of it and not hidden
    /// from it. No RNG: the arm, then the facing, then the distance.
    pub(super) fn well_senses(&self, f: &Frame, seats: &[WellSeat]) -> BTreeMap<Entity, crate::ai::WellSense> {
        use crate::ai::{Ai, WellPlan, WellSense};
        let t = tuning();
        let mut out = BTreeMap::new();
        let mut armed: Vec<(usize, Entity)> = self
            .world
            .query::<(Entity, &Tank)>()
            .with::<&Ai>()
            .iter()
            .filter(|(_, tank)| !tank.is_wreck() && tank.body.is_some())
            .filter(|(_, tank)| (tank.active_weapon() == ActiveWeapon::GravityWell && !tank.special_down()) || tank.orb.is_some())
            .map(|(e, tank)| (tank.owner_slot(), e))
            .collect();
        if armed.is_empty() {
            return out;
        }
        armed.sort_by_key(|a| a.0);
        let (r, margin) = (t.well_radius_px, t.well_ai_friend_margin_px);
        let (half_w, half_h) = t.sight_box_half_px();
        // The seats with their boxes and facings.
        struct Seat {
            seat: u8,
            pos: Position,
            centre: Position,
            half: Vec2,
            facing: Dir,
            sight: f32,
            concealed: bool,
        }
        let seat_boxes: Vec<Seat> = seats
            .iter()
            .filter(|s| s.live)
            .filter_map(|s| {
                let tank = self.world.get::<&Tank>(s.entity).ok()?;
                let (centre, half) = tank.hull_bbox_world();
                let facing = Dir::from_rotation(tank.rotation).unwrap_or(Dir::Up);
                Some(Seat { seat: s.seat, pos: s.pos, centre, half: Vec2::new(half.x, half.y), facing, sight: s.sight, concealed: s.concealed })
            })
            .collect();
        // Every live enemy, any role, disabled or not: its box and motion.
        let allies: Vec<(Position, Vec2, Vec2)> = self
            .world
            .query::<&Tank>()
            .with::<&Ai>()
            .iter()
            .filter(|tank| !tank.is_wreck() && tank.body.is_some())
            .map(|tank| {
                let (centre, half) = tank.hull_bbox_world();
                (centre, Vec2::new(half.x, half.y), tank.body.map_or(Vec2::zero(), |h| self.physics.velocity(h)))
            })
            .collect();
        let alive = |e: Option<Entity>| e.and_then(|e| self.world.get::<&Frog>(e).ok().filter(|f| !f.is_dead()).map(|f| f.position));
        let (enemy_frog, quarry) = (alive(self.enemy_frog), alive(self.frog));
        let drums: Vec<Position> =
            self.world.query::<&Obstacle>().iter().filter(|o| !o.destroyed && o.material.is_explosive()).map(|o| o.position).collect();
        let burning: BTreeSet<(i32, i32)> = self.fires.iter().filter(|fire| fire.left > 0.0).map(|fire| fire.cell).collect();
        let trouble = |p: Position| {
            let cell = world_to_cell(p);
            self.heat_at(p) >= t.heat_hurt_from
                || burning.contains(&cell)
                || self.ooze.contains_key(&cell)
                || matches!(self.water.depth_at(p), crate::ground::Depth::Shallow | crate::ground::Depth::Deep)
                || self.zones.iter().any(|z| z.rod().is_some() && p.distance_to(z.centre) <= z.radius(&t))
        };
        let crosses = |from: Position, to: Position| {
            let len = from.distance_to(to);
            let steps = (len / TROUBLE_STEP_PX).ceil().max(1.0) as i32;
            (0..=steps).any(|i| trouble(from + (to - from) * (i as f32 / steps as f32)))
        };
        // A well on its way stands as one: every orb in flight where it
        // will anchor - an enemy's at its planned distance, a seat's at its
        // range - for "not twice", and an enemy's seats in its pull are
        // spoken for ("one well a seat").
        let coming: Vec<(Position, bool)> = self
            .orbs
            .iter()
            .map(|o| {
                let enemy = matches!(o.owner, Owner::Enemy(_));
                let plan = if enemy { self.tank_of(o.owner).and_then(|e| self.world.get::<&Ai>(e).ok().map(|ai| ai.well_anchor_px())) } else { None };
                let left = (plan.unwrap_or(t.well_orb_range_px).min(t.well_orb_range_px) - o.flown).max(0.0);
                let speed = o.velocity.length();
                (if speed > 1e-3 { o.position + o.velocity * (left / speed) } else { o.position }, enemy)
            })
            .collect();
        let wells: Vec<Position> = self.zones.iter().filter(|z| z.well().is_some()).map(|z| z.centre).chain(coming.iter().map(|c| c.0)).collect();
        // The tiles an orb floats over.
        let low: Vec<Entity> = self
            .world
            .query::<(Entity, &Obstacle)>()
            .iter()
            .filter(|(_, o)| !o.destroyed && !o.material.blocks_sight())
            .map(|(e, _)| e)
            .collect();
        let players = self.seats_on_field();
        let allies_at = |p: Position, lead: f32| {
            allies.iter().filter(|&&(c, half, v)| crate::emp::box_reach(p, c, half) <= r + margin || crate::emp::box_reach(p, c + v * lead, half) <= r + margin).count()
                + usize::from(enemy_frog.is_some_and(|fp| fp.distance_to(p) <= r + margin))
        };
        let mut planned: Vec<u8> = seat_boxes
            .iter()
            .filter(|s| coming.iter().any(|&(c, enemy)| enemy && crate::emp::box_reach(c, s.centre, s.half) <= r))
            .map(|s| s.seat)
            .collect();
        for (_, entity) in armed {
            let Ok(tank) = self.world.get::<&Tank>(entity) else { continue };
            let Ok(ai) = self.world.get::<&Ai>(entity) else { continue };
            let me = tank.position;
            let mut sense = WellSense::default();
            let seen: Vec<&Seat> = seat_boxes
                .iter()
                .filter(|s| !(s.concealed && !ai.is_hit_alerted()))
                .filter(|s| me.distance_to(s.pos) <= s.sight && crate::ai::in_sight_box_of((half_w, half_h), s.pos, me))
                .collect();
            let counted: Vec<&Seat> = seen.iter().copied().filter(|s| !planned.contains(&s.seat)).collect();
            // Its own orb in flight weighs every seat it sees, spoken for or
            // not.
            if let Some(id) = tank.orb
                && let Some(o) = self.orbs.iter().find(|o| o.id == id)
            {
                let seats_in = seen.iter().filter(|s| crate::emp::box_reach(o.position, s.centre, s.half) <= r).count();
                sense.orb = Some((o.flown, allies_at(o.position, t.well_form_seconds) > seats_in));
            }
            if ai.frog_only || tank.orb.is_some() || tank.special_down() || counted.is_empty() {
                out.insert(entity, sense);
                continue;
            }
            let facing = Dir::from_rotation(tank.rotation).unwrap_or(Dir::Up);
            let faces: Vec<Dir> = std::iter::once(facing).chain(Dir::ALL.into_iter().filter(|&d| d != facing)).collect();
            // The best so far: (arm, facing's place, the pull it puts on
            // its seats - stronger first, in 64ths so float noise never
            // decides -, distance), the plan and the seats it counts.
            let mut best: Option<((u8, usize, i32, f32), WellPlan, Vec<u8>)> = None;
            let mut offer = |key: (u8, usize, i32, f32), plan: WellPlan, seats: Vec<u8>| {
                let better = |k: &(u8, usize, i32, f32)| (key.0, key.1, -key.2).cmp(&(k.0, k.1, -k.2)).then(key.3.total_cmp(&k.3)).is_lt();
                if best.as_ref().is_none_or(|(k, ..)| better(k)) {
                    best = Some((key, plan, seats));
                }
            };
            for (fi, &dir) in faces.iter().enumerate() {
                let muzzle = tank.gun_line_muzzle(dir.vec());
                let end = muzzle + dir.vec() * t.well_orb_range_px;
                let stop = f
                    .terrain
                    .sweep_rewound(&self.world, players, tank.owner(), muzzle, end, t.well_orb_half_px, &low, None)
                    .map_or(t.well_orb_range_px, |(_, k)| (k * t.well_orb_range_px - t.well_orb_half_px - 2.0).max(0.0));
                if stop < t.well_ai_min_px {
                    continue;
                }
                let mut ks: Vec<f32> = Vec::new();
                let mut k = t.well_ai_min_px;
                while k < stop {
                    ks.push(k);
                    k += WELL_AI_STEP_PX;
                }
                ks.push(stop);
                for &k in &ks {
                    let p = muzzle + dir.vec() * k;
                    if wells.iter().any(|c| c.distance_to(p) <= 2.0 * r) {
                        continue;
                    }
                    let lead = k / t.well_orb_speed.max(1.0) + t.well_form_seconds;
                    let allies_in = allies_at(p, lead);
                    let seats_in: Vec<&&Seat> = counted.iter().filter(|s| crate::emp::box_reach(p, s.centre, s.half) <= r).collect();
                    let n = seats_in.len();
                    let ids: Vec<u8> = seats_in.iter().map(|s| s.seat).collect();
                    let lowest = ids.iter().copied().min();
                    let plan = |why| WellPlan { face: dir, anchor_px: k, at_seat: lowest, why };
                    let pull: f32 = seats_in.iter().map(|s| well::strength(crate::emp::box_reach(p, s.centre, s.half), &t)).sum();
                    let pull = (pull * 64.0).round() as i32;
                    if n >= 2 && allies_in <= n {
                        offer((0, fi, pull, k), plan("clump"), ids.clone());
                    } else if n >= 1 && allies_in <= n && (drums.iter().any(|d| d.distance_to(p) <= r) || seats_in.iter().any(|s| crosses(s.pos, p))) {
                        offer((1, fi, pull, k), plan("trouble"), ids.clone());
                    } else if n >= 1
                        && allies_in <= n
                        && let Some(frog) = quarry
                        && frog.distance_to(p) > r + margin
                        && seats_in.iter().any(|s| s.pos.distance_to(frog) <= t.well_ai_guard_px && p.distance_to(frog) > s.pos.distance_to(frog))
                    {
                        offer((2, fi, pull, k), plan("guard"), ids.clone());
                    }
                }
                // The shield: a seat lined up on this tank along this
                // facing, shooting it, the well across its line.
                if ai.is_hit_alerted() {
                    let p = muzzle + dir.vec() * t.well_ai_min_px;
                    for s in &counted {
                        let (off, ahead) = crate::ai::axis_offsets(me, s.pos, dir);
                        let lined = Dir::toward(me, s.pos) == dir
                            && off <= t.enemy_fire_align_px
                            && ahead > t.well_ai_min_px
                            && s.facing == Dir::toward(s.pos, me)
                            && f.terrain.line_of_sight(me, s.pos);
                        if lined
                            && t.well_ai_min_px <= stop
                            && s.pos.distance_to(p) > r + margin
                            && allies_at(p, t.well_ai_min_px / t.well_orb_speed.max(1.0) + t.well_form_seconds) == 0
                            && !wells.iter().any(|c| c.distance_to(p) <= 2.0 * r)
                        {
                            offer((3, fi, 0, t.well_ai_min_px), WellPlan { face: dir, anchor_px: t.well_ai_min_px, at_seat: Some(s.seat), why: "shield" }, vec![s.seat]);
                            break;
                        }
                    }
                }
            }
            if let Some((_, plan, ids)) = best {
                planned.extend(ids);
                sense.plan = Some(plan);
            }
            out.insert(entity, sense);
        }
        out
    }

    /// The pull on every enemy within a forming or pulling well's reach, or
    /// within `enemy_danger_clear_px` past it (docs/gravity-well.md "The
    /// `pull` tier"): the strongest well's centre, the field's pull - every
    /// well as if it pulled, so a forming one is read at its start - and
    /// whether it braces. Empty with no well.
    pub(super) fn pull_senses(&self) -> BTreeMap<Entity, crate::ai::PullSense> {
        let t = tuning();
        let mut out = BTreeMap::new();
        let sources: Vec<(u32, Position)> = self.zones.iter().filter(|z| z.well().is_some()).map(|z| (z.id, z.centre)).collect();
        if sources.is_empty() {
            return out;
        }
        let field = WellField { sources };
        let reach = t.well_radius_px;
        for (entity, tank) in self.world.query::<(Entity, &Tank)>().with::<&crate::ai::Ai>().iter() {
            if tank.is_wreck() || tank.body.is_none() {
                continue;
            }
            let at = tank.position;
            let Some(&(_, nearest)) = field.sources.iter().min_by(|a, b| a.1.distance_to(at).total_cmp(&b.1.distance_to(at)).then(a.0.cmp(&b.0))) else {
                continue;
            };
            let d = nearest.distance_to(at);
            if d > reach + t.enemy_danger_clear_px {
                continue;
            }
            let core = field.strongest(at, &t).map_or(nearest, |(_, c, _)| c);
            let footing = Footing::at(&self.water, &self.lava, &self.craters, self.weather, at, self.time);
            let grip = t.tank_turn_grip_force * footing.grip / tank.mass();
            let heavy = tank.mass_factor() >= t.well_ai_heavy_mass;
            let brace = heavy && well::holds_broadside(&field, at, tank.mass_factor(), grip, &t);
            out.insert(entity, crate::ai::PullSense { core, pull: field.pull(at, &t), brace, inside: d <= reach });
        }
        out
    }

    /// Whether `p` stands inside a forming or pulling well's reach: a crate
    /// there is left alone by the seeks.
    pub(super) fn in_a_pull(&self, p: Position) -> bool {
        let r = tuning().well_radius_px;
        self.zones.iter().any(|z| z.well().is_some() && z.centre.distance_to(p) <= r)
    }

    /// The pulling enemy well a seat at `p` stands in (`enemy_well_holding`).
    pub(super) fn enemy_well_holding(&self, p: Position) -> Option<Position> {
        enemy_well_holding(&self.zones, p)
    }
}

/// The centre of the pulling enemy well among `zones` that `p` stands in,
/// if any: what the pack's ring is herded round (docs/gravity-well.md "The
/// group fires into the clump").
pub(super) fn enemy_well_holding(zones: &[Zone], p: Position) -> Option<Position> {
    let r = tuning().well_radius_px;
    zones
        .iter()
        .filter(|z| z.well().is_some_and(|w| w.stage == WellStage::Pulling) && !z.owner.is_player())
        .find(|z| z.centre.distance_to(p) <= r)
        .map(|z| z.centre)
}

