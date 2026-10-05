//! The waves spawn plan at run time (docs/maps-to-levels.md "Spawn plan",
//! "Gates and roll-in"): `WaveState` schedules waves, queues their tanks
//! and rolls each one in through an edge gate (`battlefield::Gate`) as a
//! kinematic `RollIn` entity that only becomes a real enemy - physics body
//! plus `Ai` - once it reaches the gate's inside point. Also the wave-round
//! wreck despawn, and the one other thing that comes in through a gate: a
//! **seat that fell while the team fought on** drives back in with the next
//! wave (docs/online-coop-prd.md section 4.11, decision 7). Everything here
//! runs inside `Game::update`'s frame, draws only from the frame's round
//! RNG, and is a no-op under the band plan and in any round that seats one,
//! so a band round's and a solo round's RNG streams are untouched.

use std::collections::VecDeque;

use hecs::Entity;
use rand::RngExt;
use serde::Serialize;
use crate::math::Vec2;

use crate::ai::{Ai, Role};
use crate::battlefield::{self, Gate};
use crate::level::{SpawnPlan, Tier};

/// The `WAVE N` banner as data: the wave about to roll in, and whether it
/// is the round's last.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WaveBanner {
    pub next: u32,
    pub is_final: bool,
}
use crate::obstacle::Obstacle;
use crate::pathfind::Grid;
use crate::tank::Tank;
use crate::tuning::tuning;
use crate::Position;

use super::{field, lay_tracks, roll_enemy_tank, roll_role, with_frog, with_tank, with_tank_mut, Event, Frame, Game, TANK_SPRITE_ORDER};

/// What lane `Game::pick_gate` found for the next roll-in.
#[derive(Clone, Copy, Debug)]
enum GatePick {
    /// A free lane, already marked used for this wave.
    Free(Gate),
    /// The map has gates but every lane is busy this frame.
    Busy,
    /// The map offers no gate at all.
    None,
}

/// A tank still driving in from outside the battlefield toward `to` (its
/// gate's inside point) - a wave's, or a seat coming back. While it
/// carries this it has no physics body, and only `Game::rollin_phase`
/// moves it.
///
/// An entering *enemy* has no `Ai` either, so every enemy query
/// (`.with::<&Ai>()`) - hit sweep, ram, explosions, engagement, the AI
/// phase, the round-end wreck count - leaves it alone for free. A seat has
/// no `Ai` to take away, so `Game::seats_on_field()` is what leaves it
/// alone instead.
pub struct RollIn {
    pub to: Position,
}

/// A straggler taken off the field and rolling in again through a nearer
/// gate (`Game::reroll_stragglers`), beside its `RollIn`: the role it
/// keeps once it is back, so its arrival rolls none. A training beat's
/// tank rolls in with one too, the role and the dummy's flag its script
/// asked for (`Game::training_roll_in`).
pub(crate) struct Rejoin {
    role: Role,
    frog_only: bool,
}

/// How often the round looks for stragglers, in ticks: once a second. A
/// straggler has been lost for half a minute by then, so a second either
/// way changes nothing, and the walk it is measured by is a Dijkstra over
/// the whole map that a tick has no business paying for.
const STRAGGLER_CHECK_TICKS: u64 = 60;

/// The wave scheduler's memory for one round.
#[derive(Default)]
pub(crate) struct WaveState {
    /// Waves called so far; wave `called` (0-based) is the next to call.
    called: u32,
    /// Seconds since the current wave was called, for the timeout.
    elapsed: f32,
    /// The breather before the next wave: `Some(seconds left)` only while
    /// it runs (the `WAVE N` banner shows meanwhile).
    gap: Option<f32>,
    /// Chassis rows still to roll in, in order.
    pending: VecDeque<i32>,
    /// Seconds until the next queued tank may start its roll-in.
    stagger: f32,
    /// Owner slot the next wave tank takes; only ever counts up, so a slot
    /// is never reused even once its wreck is gone.
    next_slot: usize,
    /// Gates (edge index, cell) the current wave has used, so its tanks
    /// spread over distinct lanes before any repeats.
    used_gates: Vec<(usize, (usize, usize))>,
    /// Seats whose tank is a wreck and which the next wave brings back,
    /// in seat order (docs/online-coop-prd.md section 4.11, decision 7).
    /// Filled by `call_wave`, drained by the same staggered roll-in the
    /// wave's own tanks use. Empty in every round that seats one.
    returning: VecDeque<usize>,
}

impl WaveState {
    /// A fresh scheduler whose first tank takes owner slot `first_slot`.
    pub fn new(first_slot: usize) -> Self {
        WaveState { next_slot: first_slot, ..Default::default() }
    }
}

/// Where a waves round stands, for the HUD and the dev server's `status`.
#[derive(Clone, Copy, Debug, Serialize)]
pub struct WaveStatus {
    /// The current wave, 1-based (0 before the first is called).
    pub index: u32,
    pub total: u32,
    /// Live enemies: on the field and not wrecked, plus those rolling in.
    pub alive: usize,
    /// Tanks queued but not yet rolling in.
    pub pending: usize,
    /// Seconds until the next wave is called, while the breather runs.
    pub next_in: Option<f32>,
}

impl Game {
    /// The waves round's progress, `None` under the band plan.
    pub fn wave_status(&self) -> Option<WaveStatus> {
        let SpawnPlan::Waves { waves, .. } = self.spawn_plan else { return None };
        Some(WaveStatus {
            index: self.wave.called,
            total: waves,
            alive: self.live_enemy_count(),
            pending: self.wave.pending.len(),
            next_in: self.wave.gap,
        })
    }

    /// The banner for the wave about to arrive, while the breather before
    /// it runs: which wave, and whether it is the last. The words are the
    /// renderer's (`text::keys::WAVE_BANNER`/`WAVE_FINAL`); the
    /// simulation names no language.
    pub fn wave_banner(&self) -> Option<WaveBanner> {
        let SpawnPlan::Waves { waves, .. } = self.spawn_plan else { return None };
        self.wave.gap?;
        let next = self.wave.called + 1;
        Some(WaveBanner { next, is_final: next >= waves })
    }

    /// Every wave called, the queue drained and no enemy still rolling
    /// in. A seat driving back in is not counted: the round is won on the
    /// enemies, and a team-mate still in the gate lane is not one.
    pub(super) fn waves_finished(&self) -> bool {
        let SpawnPlan::Waves { waves, .. } = self.spawn_plan else { return true };
        self.wave.called >= waves
            && self.wave.pending.is_empty()
            && !self.world.query::<(&Tank, &RollIn)>().iter().any(|(t, _)| !t.is_player())
    }

    /// Enemies that count against `wave_max_alive` and toward the next
    /// wave: live tanks on the field plus tanks rolling in. Wrecks don't.
    /// Counted by owner rather than by `Ai` so a client replica, whose
    /// enemies carry no `Ai` (`net::apply`), reports the same number the
    /// server does; in a round every enemy tank either has its `Ai` or is
    /// rolling in, so the two readings agree.
    pub(super) fn live_enemy_count(&self) -> usize {
        self.world.query::<&Tank>().iter().filter(|t| !t.is_player() && !t.is_wreck()).count()
    }

    /// Put the scheduler at wave `called` with `pending` tanks still to
    /// roll in and `next_in` seconds of breather left, as a snapshot
    /// reports it (`net::apply`): the queue is filled with placeholder
    /// rows, since a replica never spawns from it, and the breather is
    /// only ever the `WAVE N` banner's timer there.
    pub(crate) fn set_wave_progress(&mut self, called: u32, pending: usize, next_in: Option<f32>) {
        self.wave.called = called;
        self.wave.pending.clear();
        self.wave.pending.extend(std::iter::repeat_n(TANK_SPRITE_ORDER[0], pending));
        self.wave.gap = next_in;
    }

    /// Whether the tank entity is still rolling in.
    pub(crate) fn is_entering(&self, entity: Entity) -> bool {
        self.world.get::<&RollIn>(entity).is_ok()
    }

    /// Drive every rolling-in tank straight at its gate's inside point at
    /// `wave_rollin_speed_factor` of its driving speed - kinematically,
    /// with treads animating and tracks laid. On arrival the tank gets its
    /// physics body, loses its `RollIn` and is announced with
    /// `Event::TankEntered`. An enemy is given its `Ai` there too; a seat
    /// driving back in (`seat_returns`) takes none - it is the stick's
    /// again the frame it is through the gate.
    pub(super) fn rollin_phase(&mut self, f: &mut Frame) {
        let factor = tuning().wave_rollin_speed_factor;
        let mut arrived: Vec<(Entity, usize)> = Vec::new();
        for (entity, tank, roll) in self.world.query::<(Entity, &mut Tank, &RollIn)>().iter() {
            let before = tank.position;
            let dx = roll.to.x - before.x;
            let dy = roll.to.y - before.y;
            let dist = (dx * dx + dy * dy).sqrt();
            let speed = tank.effective_speed() * factor;
            let step = speed * f.dt;
            if dist <= step || dist <= f32::EPSILON {
                tank.position = roll.to;
                tank.velocity = Vec2::new(0.0, 0.0);
                arrived.push((entity, tank.owner_slot()));
            } else {
                let (ux, uy) = (dx / dist, dy / dist);
                tank.position = Position::new(before.x + ux * step, before.y + uy * step);
                tank.velocity = Vec2::new(ux * speed, uy * speed);
            }
            tank.ease_visual_rotation(f.dt);
            tank.ease_turret_visual_rotation(f.dt);
            lay_tracks(&mut self.tracks, tank, before, self.water.depth_at(tank.position));
        }
        for (entity, slot) in arrived {
            let (pos, half, mass) =
                with_tank(&self.world, entity, |t| (t.position, t.move_half_extents(t.facing_along_x()), t.mass()));
            let body = self.physics.spawn_tank(pos, half, mass);
            with_tank_mut(&self.world, entity, |t| t.body = Some(body));
            self.world.remove_one::<RollIn>(entity).expect("entity from this frame's roll-in query still exists");
            if !self.is_player(entity) {
                // Roles roll on arrival, the moment the tank joins the
                // fight - but for a straggler coming back, which keeps the
                // role it had.
                let (role, frog_only) = match self.world.remove_one::<Rejoin>(entity) {
                    Ok(rejoin) => (rejoin.role, rejoin.frog_only),
                    Err(_) => (roll_role(self.mission, &mut f.rng), false),
                };
                let mut ai = Ai::with_role(role);
                ai.frog_only = frog_only;
                // On a field map the wave is called to the fight: the walk
                // its gate was picked for (`field`).
                ai.field.called = self.field_map;
                ai.field.wave = self.field_map;
                self.world.insert_one(entity, ai).expect("entity from this frame's roll-in query still exists");
            }
            f.events.push(Event::TankEntered { slot });
        }
    }

    /// The wave scheduler: the first wave on the first playing frame, the
    /// next when live enemies drop to `wave_next_when_alive` with the
    /// queue empty or `wave_timeout_seconds` after the current one, each
    /// after a `wave_gap_seconds` breather - which on a field map the
    /// pacing director paces instead (`director`: held while the team is
    /// at its peak, stretched to a rest after one, shortened while nothing
    /// happens). Queued tanks start their roll-in `wave_stagger_seconds`
    /// apart while live enemies stay under `wave_max_alive`, and a seat
    /// the wave is bringing back takes the first of those windows.
    pub(super) fn wave_phase(&mut self, f: &mut Frame) {
        let SpawnPlan::Waves { waves, .. } = self.spawn_plan else { return };
        let (gap_seconds, timeout, next_when_alive, stagger_seconds, max_alive) = {
            let t = tuning();
            (t.wave_gap_seconds, t.wave_timeout_seconds, t.wave_next_when_alive, t.wave_stagger_seconds, t.wave_max_alive)
        };
        // The team's intensity, on a field map with the director on.
        let team = (self.field_map && tuning().director_enabled).then(|| self.observe_pressure(f.dt));
        if self.wave.called == 0 {
            self.call_wave(f);
        } else if self.wave.gap.is_some() {
            match (team, self.wave.gap) {
                (Some(team), Some(left)) => self.wave.gap = Some(self.director.pace_breather(left, team, f.dt, &tuning())),
                _ => self.tick_wave_banner(f.dt),
            }
            if self.wave.gap == Some(0.0) {
                self.wave.gap = None;
                self.call_wave(f);
            }
        } else if self.wave.called < waves {
            self.wave.elapsed += f.dt;
            let cleared = self.live_enemy_count() <= next_when_alive && self.wave.pending.is_empty();
            if cleared || self.wave.elapsed >= timeout {
                let breather = if team.is_some() { self.director.breather_length(&tuning()) } else { gap_seconds };
                self.wave.gap = Some(breather);
            }
        }

        self.wave.stagger = (self.wave.stagger - f.dt).max(0.0);
        if self.wave.stagger <= 0.0 {
            let queued = !self.wave.pending.is_empty() && self.live_enemy_count() < max_alive;
            if queued || !self.wave.returning.is_empty() {
                // One lane per stagger window, the seat coming back first:
                // a player who lost their tank drives in at the head of
                // the wave rather than behind all of it. A wave with every
                // lane busy tries again next frame. On a field map the seat
                // takes the lane nearest the living seats rather than a
                // wave's (`pick_return_gate`).
                let returning = self.field_map && self.wave.returning.front().is_some_and(|&seat| self.seat(seat).is_some());
                let gate = if returning { self.pick_return_gate(f) } else { self.pick_gate(f) };
                let took = self.seat_returns(gate) || (queued && self.spawn_wave_tank(f, gate));
                if took {
                    self.wave.stagger = stagger_seconds;
                }
            }
        }

        // A field map rolls its stragglers in again through a nearer gate.
        if self.field_map && tuning().field_reroll && self.frame.is_multiple_of(STRAGGLER_CHECK_TICKS) {
            self.reroll_stragglers(f);
        }
    }

    /// Stragglers (docs/large-maps-follow-camera.md section 12): a wave
    /// tank that has gone `field_reroll_after_seconds` without a live seat
    /// or the players' frog within its sight (`FieldMind::lost`), and
    /// stands farther from them by walk than any wave's gate is paced for
    /// (`field::far_by_walk`) - typically one that went through a portal
    /// for a pickup, lost its call and kept to its gate's leash - is taken
    /// off the field and rolled in again through a gate nearer the fight.
    /// Only where no screen could see it go (`field::beyond_every_screen`
    /// of every seat, a wreck or a seat in a gate lane included), never a
    /// guard keeping its frog, never a burning hull, and only through a
    /// lane `pick_gate` would give a wave, outside every seat's sight box
    /// and shorter by walk than where the tank stands
    /// (`field::reroll_gates`); with none free it waits for the next look.
    ///
    /// In owner-slot order, each re-roll's lane drawn from the round's RNG
    /// like a wave's; a round with no straggler draws nothing.
    fn reroll_stragglers(&mut self, f: &mut Frame) {
        let t = tuning();
        // Every seat's camera, wherever its tank is.
        let screens: Vec<Position> =
            self.players().into_iter().flatten().map(|e| with_tank(&self.world, e, |tank| tank.position)).collect();
        let guarding = self.enemy_frog.is_some_and(|e| with_frog(&self.world, e, |fr| !fr.is_dead()));
        let mut lost: Vec<(usize, Entity, Position)> = self
            .world
            .query::<(Entity, &Tank, &Ai)>()
            .iter()
            .filter(|(_, tank, ai)| {
                ai.field.wave
                    && ai.field.lost >= t.field_reroll_after_seconds
                    && !tank.is_wreck()
                    && tank.burn_timer <= 0.0
                    && !(ai.role == Role::Guard && guarding)
                    && screens.iter().all(|&s| field::beyond_every_screen(s, tank.position, &t))
            })
            .map(|(entity, tank, _)| (tank.owner_slot(), entity, tank.position))
            .collect();
        if lost.is_empty() {
            return;
        }
        lost.sort_by_key(|&(slot, ..)| slot);
        let seats = self.live_seat_positions();
        let mut fight = seats.clone();
        fight.extend(self.frog_position());
        if fight.is_empty() {
            return;
        }
        let grid = self.nav_grid(f.width, f.height);
        let walk = grid.walk_costs(&fight);
        // The lanes depend on the grid, the seats and the frog, none of
        // which a re-roll moves.
        let lanes = self.wave_lanes(&grid, f);
        for (slot, entity, at) in lost {
            let from = walk.from_hull(at);
            if !field::far_by_walk(from) {
                continue;
            }
            let gates = field::reroll_gates(lanes.clone(), &walk, &seats, from);
            if let GatePick::Free(gate) = self.draw_lane(gates, f) {
                self.reroll(entity, slot, at, gate, f);
            }
        }
    }

    /// Take the straggler `entity` (owner slot `slot`, standing at `from`)
    /// off the field and start it rolling in again through `gate`: its
    /// body and its `Ai` go, as a wave tank still outside has neither, and
    /// it keeps everything else - its slot, its chassis, its damage, its
    /// ammunition and weapons - and its role and a dummy's flag, which a
    /// `Rejoin` carries to its arrival. It arrives called to the fight like
    /// any wave tank, with a fresh memory and its new gate for home.
    fn reroll(&mut self, entity: Entity, slot: usize, from: Position, gate: Gate, f: &mut Frame) {
        let ai = self.world.remove_one::<Ai>(entity).expect("a straggler is an enemy on the field");
        let (role, frog_only) = (ai.role, ai.frog_only);
        if let Some(body) = with_tank(&self.world, entity, |t| t.body) {
            self.physics.remove_body(body);
        }
        let rotation = gate.heading().rotation();
        with_tank_mut(&self.world, entity, |t| {
            t.body = None;
            t.position = gate.outside;
            t.velocity = Vec2::new(0.0, 0.0);
            t.rotation = rotation;
            t.visual_rotation = rotation;
            t.turret_visual_rotation = rotation;
            t.track_from = None;
            t.flame_held = false;
            t.pending_plasma_shot = None;
            t.minigun_burst = None;
            t.missile_volley = None;
        });
        self.world.insert(entity, (RollIn { to: gate.inside }, Rejoin { role, frog_only })).expect("a straggler is never despawned");
        // The slots it held were round the fight it never reached.
        for ring in &mut self.engage {
            ring.release(entity);
        }
        self.engage_frog.release(entity);
        f.events.push(Event::Rerolled { slot, x: from.x, y: from.y });
    }

    /// Queue wave `called`'s tanks: `wave_size` chassis drawn from the
    /// wave's tier, each with `wave_tier_mix` odds of the tier below.
    fn call_wave(&mut self, f: &mut Frame) {
        let i = self.wave.called;
        let size = self.spawn_plan.wave_size(i);
        let tier = self.spawn_plan.wave_tier(i);
        let mix = tuning().wave_tier_mix;
        for _ in 0..size {
            let lower = f.rng.random_range(0.0..1.0) < mix && tier != Tier::Light;
            let drawn = if lower { Tier::from_index(tier.index() - 1) } else { tier };
            let rows = drawn.rows();
            self.wave.pending.push_back(rows[f.rng.random_range(0..rows.len())]);
        }
        self.wave.called = i + 1;
        self.wave.elapsed = 0.0;
        self.wave.stagger = 0.0;
        self.wave.used_gates.clear();
        self.queue_returning_seats();
        f.events.push(Event::WaveStarted { wave: i + 1, size, tier });
    }

    /// Every seat whose tank is a wreck comes back with this wave
    /// (docs/online-coop-prd.md section 4.11, decision 7): somebody is
    /// still fighting, or `check_round_end` would have lost the round the
    /// frame the last seat went, so a seat that fell holds a place in the
    /// next wave instead of watching the rest of the round. In seat order,
    /// and never twice - a seat still queued from the wave before keeps
    /// the place it has.
    ///
    /// A round that seats one queues nothing: its one wreck is every seat
    /// wrecked. A band round never calls a wave at all. Both replay
    /// exactly as they did.
    fn queue_returning_seats(&mut self) {
        if self.players.count() < 2 {
            return;
        }
        for seat in 0..self.players.count() {
            let Some(entity) = self.seat(seat) else { continue };
            if self.wave.returning.contains(&seat) {
                continue;
            }
            if with_tank(&self.world, entity, Tank::is_wreck) {
                self.wave.returning.push_back(seat);
            }
        }
    }

    /// Put the seat at the head of the return queue back on the field
    /// through `gate`, as a fresh tank of its own chassis: the same
    /// entity, owner slot, chassis and worn tread look, and everything a
    /// round is fought with - health, shells, any picked-up weapon, a
    /// shield - as `init` would have spawned it. It drives in like a wave
    /// tank (`rollin_phase`) and answers the stick the frame it arrives.
    ///
    /// `false` when nothing is waiting or no lane is free this frame: the
    /// seat keeps its place and the wave's own tanks roll on, so a map
    /// with no gate at all never deadlocks behind a seat waiting to come
    /// back (it is a `waves-no-gates` lint error in the first place).
    fn seat_returns(&mut self, gate: GatePick) -> bool {
        let Some(&seat) = self.wave.returning.front() else { return false };
        let GatePick::Free(gate) = gate else { return false };
        let Some(entity) = self.seat(seat) else {
            self.wave.returning.pop_front();
            return false;
        };
        self.wave.returning.pop_front();
        // The hulk's body goes with it: the roll-in is kinematic and
        // `rollin_phase` spawns a fresh one at the gate's inside point.
        if let Some(body) = with_tank(&self.world, entity, |t| t.body) {
            self.physics.remove_body(body);
        }
        let rotation = gate.heading().rotation();
        with_tank_mut(&self.world, entity, |t| {
            *t = Tank {
                row: t.row,
                shell_variant: t.shell_variant,
                damage_variant: t.damage_variant,
                track_wobble_amp: t.track_wobble_amp,
                track_wobble_freq: t.track_wobble_freq,
                track_wobble_phase: t.track_wobble_phase,
                track_scale_jitter: t.track_scale_jitter,
                position: gate.outside,
                rotation,
                visual_rotation: rotation,
                turret_visual_rotation: rotation,
                owner: t.owner,
                ..Tank::default()
            };
        });
        self.world.insert_one(entity, RollIn { to: gate.inside }).expect("a seat's tank is never despawned");
        // A trigger held when the tank went keeps `dispatch_fire` waiting
        // for a release that already happened, and the slots claimed
        // around the hulk are around nothing now.
        self.player_fire_held_last_frame[seat] = false;
        self.engage[seat].clear();
        true
    }

    /// The lane the next tank rolls in through: one of the lanes a wave
    /// may take (`wave_lanes`) - on a field map those outside every seat's
    /// sight box and about its walk where it has any
    /// (`field::prefer_gates`) - drawn by `draw_lane`. A map with no gate
    /// at all answers `None`, one whose every lane is busy `Busy`, so the
    /// caller retries next frame.
    ///
    /// The draw is the round RNG's, and the chosen lane is marked used, so
    /// this is called once per roll-in attempt and its answer handed to
    /// whoever takes it.
    fn pick_gate(&mut self, f: &mut Frame) -> GatePick {
        let grid = self.nav_grid(f.width, f.height);
        let (lanes, player) = self.open_lanes(&grid, f);
        // Every preference below keeps some of the lanes it is given, so
        // with every lane busy the answer is `Busy` whichever it would
        // keep - said before the components and the walk are worked out
        // for nothing on every frame a wave waits for a lane.
        if !lanes.is_empty() && self.free_lanes(&lanes).is_empty() {
            return GatePick::Busy;
        }
        let mut gates = Self::toward_the_fight(&grid, lanes, player);
        // A field map keeps a wave out of sight and within reach: lanes
        // outside every seat's sight box, those whose walk to the nearest
        // seat is about `field_walk_seconds` where there are any
        // (`field::prefer_gates`).
        if self.field_map {
            gates = field::prefer_gates(gates, &grid, &self.live_seat_positions());
        }
        self.draw_lane(gates, f)
    }

    /// Every lane a wave may roll in through on `grid`, the live nav grid
    /// (`open_lanes`), and of those the ones that lead to the fight where
    /// any does (`toward_the_fight`).
    fn wave_lanes(&self, grid: &Grid, f: &Frame) -> Vec<Gate> {
        let (lanes, player) = self.open_lanes(grid, f);
        Self::toward_the_fight(grid, lanes, player)
    }

    /// The lanes a wave may roll in through on `grid`, the live nav grid:
    /// the map's own `gate` cells when it has any, else the lanes the grid
    /// offers (`battlefield::gate_candidates`, so walls shot away since
    /// open new lanes), either way none within `wave_gate_min_player_dist`
    /// of a seat or the players' frog where any lane is farther - with
    /// where player 1 stands, the fight `toward_the_fight` routes to.
    fn open_lanes(&self, grid: &Grid, f: &Frame) -> (Vec<Gate>, Position) {
        let (inward, min_dist) = {
            let t = tuning();
            (t.wave_gate_inward_cells, t.wave_gate_min_player_dist)
        };
        // Every seat that holds a tank; `avoid[0]` stays player 1, the one
        // the connectivity preference routes to.
        let mut avoid: Vec<Position> = self
            .players()
            .into_iter()
            .flatten()
            .map(|p| with_tank(&self.world, p, |t| t.position))
            .collect();
        if let Some(frog) = self.frog {
            avoid.push(with_frog(&self.world, frog, |fr| fr.position));
        }
        let explicit = self.map.gate_cells();
        let mut gates = if explicit.is_empty() {
            Vec::new()
        } else {
            let all = battlefield::gates_from_cells(grid, f.width, f.height, &explicit, inward);
            let far: Vec<Gate> =
                all.iter().copied().filter(|g| avoid.iter().all(|p| p.distance_to(g.inside) >= min_dist)).collect();
            // A map whose every gate sits by the player still uses them.
            if far.is_empty() { all } else { far }
        };
        if gates.is_empty() {
            gates = battlefield::gate_candidates(grid, f.width, f.height, &avoid, min_dist, inward);
        }
        (gates, avoid[0])
    }

    /// Of `lanes`, the ones that lead to the fight - connected to `player`
    /// on `grid` - where any does: a lane open on its own can still end in
    /// a pocket walled off from the player (the default map's gated
    /// strips). Only when no lane connects is any lane used.
    fn toward_the_fight(grid: &Grid, lanes: Vec<Gate>, player: Position) -> Vec<Gate> {
        let components = grid.components();
        let connected: Vec<Gate> = lanes.iter().copied().filter(|g| components.connected(grid, g.inside, player)).collect();
        if connected.is_empty() { lanes } else { connected }
    }

    /// One of `gates` for a tank to start down now: of the free ones
    /// (`free_lanes`) a lane this wave has not used where there is one,
    /// drawn from the round's RNG and marked used. `None` with no lane at
    /// all, `Busy` with every one taken.
    fn draw_lane(&mut self, gates: Vec<Gate>, f: &mut Frame) -> GatePick {
        if gates.is_empty() {
            return GatePick::None;
        }
        let free = self.free_lanes(&gates);
        if free.is_empty() {
            return GatePick::Busy;
        }
        let unused: Vec<Gate> =
            free.iter().copied().filter(|g| !self.wave.used_gates.contains(&(g.edge.index(), g.cell))).collect();
        let pool = if unused.is_empty() { &free } else { &unused };
        let gate = pool[f.rng.random_range(0..pool.len())];
        self.wave.used_gates.push((gate.edge.index(), gate.cell));
        GatePick::Free(gate)
    }

    /// The lanes of `gates` a tank may start down now, in the order given:
    /// none with a tank still rolling along it or anyone standing on its
    /// inside point.
    fn free_lanes(&self, gates: &[Gate]) -> Vec<Gate> {
        let clearance = Tank::default().size() * 1.5;
        let entering: Vec<Position> = self.world.query::<(&Tank, &RollIn)>().iter().map(|(t, _)| t.position).collect();
        let standing: Vec<Position> =
            self.world.query::<&Tank>().iter().filter(|t| t.body.is_some()).map(|t| t.position).collect();
        gates
            .iter()
            .copied()
            .filter(|g| {
                entering.iter().all(|&p| segment_distance(p, g.outside, g.inside) >= clearance)
                    && standing.iter().all(|&p| p.distance_to(g.inside) >= clearance)
            })
            .collect()
    }

    /// The lane a wrecked seat comes back through on a field map: of every
    /// gate the map offers, the free one nearest the living seats by path
    /// (`field::nearest_gates`), so a team-mate rejoins the fight rather
    /// than a wave's walk away from it (docs/large-maps-follow-camera.md
    /// section 13, item 5). No draw: ties go to the gates' own order. The
    /// `pick_gate` contract holds - `Busy` while every lane is taken,
    /// `None` on a map with no gate - and the lane is marked used.
    fn pick_return_gate(&mut self, f: &mut Frame) -> GatePick {
        let inward = tuning().wave_gate_inward_cells;
        let grid = self.nav_grid(f.width, f.height);
        // The map's own gates, else the lanes the live grid offers - the
        // same lanes `pick_gate` draws from, with nobody kept away from.
        let explicit = self.map.gate_cells();
        let mut gates = battlefield::gates_from_cells(&grid, f.width, f.height, &explicit, inward);
        if gates.is_empty() {
            gates = battlefield::gate_candidates(&grid, f.width, f.height, &[], 0.0, inward);
        }
        if gates.is_empty() {
            return GatePick::None;
        }
        // The free lanes first, so a frame with every lane busy works out
        // no walk; the order by walk is stable, so the nearest free lane is
        // the one it would be in the order of every lane.
        let free = self.free_lanes(&gates);
        if free.is_empty() {
            return GatePick::Busy;
        }
        let walk = grid.walk_costs(&self.live_seat_positions());
        let gate = field::nearest_gates(free, &walk)[0];
        self.wave.used_gates.push((gate.edge.index(), gate.cell));
        GatePick::Free(gate)
    }

    /// Start one tank of chassis `row` rolling in through the lane
    /// `pick_gate` found. With no gate at all the tank is placed in the
    /// spawn band instead, fully on the field, so a gate-less map still
    /// plays; with every lane busy nothing spawns and `false` comes back
    /// so the caller retries next frame.
    fn spawn_wave_tank(&mut self, f: &mut Frame, gate: GatePick) -> bool {
        let gate = match gate {
            GatePick::Free(gate) => gate,
            GatePick::Busy => return false,
            GatePick::None => {
                let row = self.wave.pending[0];
                self.spawn_in_band(f, row);
                self.wave.pending.pop_front();
                return true;
            }
        };
        let row = self.wave.pending[0];
        let slot = self.take_slot();
        let mut tank = roll_enemy_tank(&mut f.rng, row, gate.outside, slot);
        let rotation = gate.heading().rotation();
        tank.rotation = rotation;
        tank.visual_rotation = rotation;
        tank.turret_visual_rotation = rotation;
        self.world.spawn((tank, RollIn { to: gate.inside }));
        self.wave.pending.pop_front();
        true
    }

    /// A training beat's tank (docs/training-stage.md): one of chassis
    /// `row` rolling in through a free lane, as a wave's does, to fight as
    /// `role` once it arrives - a dummy (`frog_only`) never at a seat.
    /// `None` while no lane is free - or the map has no gate - so the beat
    /// asks again next frame.
    pub(super) fn training_roll_in(&mut self, f: &mut Frame, row: i32, role: Role, frog_only: bool) -> Option<Entity> {
        let GatePick::Free(gate) = self.pick_gate(f) else { return None };
        let slot = self.take_slot();
        let mut tank = roll_enemy_tank(&mut f.rng, row, gate.outside, slot);
        let rotation = gate.heading().rotation();
        tank.rotation = rotation;
        tank.visual_rotation = rotation;
        tank.turret_visual_rotation = rotation;
        Some(self.world.spawn((tank, RollIn { to: gate.inside }, Rejoin { role, frog_only })))
    }

    /// The gate-less fallback: place the tank in the spawn band exactly as
    /// the band plan does at init (`battlefield::enemy_spawn_legal`, then
    /// `Grid::nearest_open` on the attempt cap), with its body and `Ai`,
    /// and announce it as entered.
    pub(super) fn spawn_in_band(&mut self, f: &mut Frame, row: i32) -> Entity {
        let (margin_min, margin_max) = {
            let t = tuning();
            let short_side = f.width.min(f.height);
            (short_side * t.enemy_spawn_margin_min, short_side * t.enemy_spawn_margin_max)
        };
        let player = self.player().expect("player entity spawned in init");
        let player_pos = with_tank(&self.world, player, |t| t.position);
        // Every seat past the first, in index order - the same clearance
        // the band placement at `init` keeps from them.
        let seats: Vec<Position> = self
            .players()
            .into_iter()
            .flatten()
            .skip(1)
            .map(|p| with_tank(&self.world, p, |t| t.position))
            .collect();
        let size = Tank::default().size();
        let (clear, enemy_clear) = (size * 2.0, size * 1.5);
        let grid = self.nav_grid(f.width, f.height);
        let walls: Vec<Position> =
            self.world.query::<&Obstacle>().iter().filter(|o| !o.destroyed).map(|o| o.position).collect();
        let others: Vec<Position> = self.world.query::<&Tank>().iter().map(|t| t.position).collect();
        // A field map places it the way its band places one at `init`:
        // out of sight, by its walk to the fight (`field::spawn_cells`).
        let field_pick = if self.field_map {
            let live = self.live_seat_positions();
            let cells = field::spawn_cells(&grid, &live, &walls);
            field::pick_spawn(&cells, &mut f.rng, &others, |pos| {
                pos.distance_to(player_pos) >= clear
                    && seats.iter().all(|&p| pos.distance_to(p) >= clear)
                    && others.iter().all(|&p| pos.distance_to(p) >= enemy_clear)
            })
        } else {
            None
        };
        let pos = field_pick.unwrap_or_else(|| {
            battlefield::sample_clear_position(&mut f.rng, f.width, f.height, margin_min, |pos| {
                battlefield::enemy_spawn_legal(pos, f.width, f.height, margin_min, margin_max, player_pos, clear, &grid, &walls)
                    && seats.iter().all(|&p| pos.distance_to(p) >= clear)
                    && others.iter().all(|&p| pos.distance_to(p) >= enemy_clear)
            })
            .unwrap_or_else(|| {
                let sample = Position::new(
                    f.rng.random_range(margin_min..(f.width - margin_min)),
                    f.rng.random_range(margin_min..(f.height - margin_min)),
                );
                grid.nearest_open(sample, &others, enemy_clear)
            })
        });
        let slot = self.take_slot();
        let mut tank = roll_enemy_tank(&mut f.rng, row, pos, slot);
        tank.visual_rotation = tank.rotation;
        tank.turret_visual_rotation = tank.rotation;
        tank.body = Some(self.physics.spawn_tank(pos, tank.move_half_extents(false), tank.mass()));
        let mut ai = Ai::with_role(roll_role(self.mission, &mut f.rng));
        // A wave tank all the same: called to the fight on a field map.
        ai.field.called = self.field_map;
        ai.field.wave = self.field_map;
        let entity = self.world.spawn((tank, ai));
        f.events.push(Event::TankEntered { slot });
        entity
    }

    /// The next owner slot for a tank the scheduler (or the dev server's
    /// `spawn_enemy`) adds mid-round; never handed out twice.
    pub(super) fn take_slot(&mut self) -> usize {
        let slot = self.wave.next_slot;
        self.wave.next_slot = slot + 1;
        slot
    }

    /// Keep `take_slot` above every slot already on the field.
    pub(super) fn reserve_slots_above(&mut self, slot: usize) {
        self.wave.next_slot = self.wave.next_slot.max(slot + 1);
    }

    /// Run every wreck's fade (`fade_wrecks`) and remove the ones whose
    /// `Tank::despawn_timer` has run out, body included, with an
    /// `Event::WreckRemoved`. Band rounds arm no timer and so keep their
    /// wrecks for the whole round.
    pub(super) fn despawn_wrecks(&mut self, f: &mut Frame) {
        self.fade_wrecks(f.dt);
        let mut gone: Vec<(Entity, usize)> = Vec::new();
        for (entity, tank) in self.world.query::<(Entity, &Tank)>().with::<&Ai>().iter() {
            if tank.despawn_timer.is_some_and(|left| left <= 0.0) {
                gone.push((entity, tank.owner_slot()));
            }
        }
        for (entity, slot) in gone {
            if let Some(body) = with_tank(&self.world, entity, |t| t.body) {
                self.physics.remove_body(body);
            }
            self.world.despawn(entity).ok();
            f.events.push(Event::WreckRemoved { slot });
        }
    }

    /// Arm each new wreck's `Tank::despawn_timer` and count it down - the
    /// last second of it is the fade `Tank::alpha` draws. Band rounds and
    /// a `wave_wreck_despawn_seconds` of zero keep their wrecks, so
    /// nothing is armed and nothing fades.
    ///
    /// The fade is all a client replica needs: its wrecks leave when the
    /// server's snapshot stops listing them, so it arms the timer the
    /// first frame it sees a wreck and lets it run
    /// (`Game::tick_presentation`, docs/online-coop-prd.md section 4.5).
    /// A wreck it joined in progress fades late, which costs no bytes and
    /// shows only on a hull that was already burning when the client
    /// arrived.
    pub(crate) fn fade_wrecks(&mut self, dt: f32) {
        if !matches!(self.spawn_plan, SpawnPlan::Waves { .. }) {
            return;
        }
        let seconds = tuning().wave_wreck_despawn_seconds;
        if seconds <= 0.0 {
            return;
        }
        // Every enemy wreck, by owner rather than by `Ai`, so a replica's
        // enemies - which carry none - fade like the server's.
        for tank in self.world.query::<&mut Tank>().iter() {
            if tank.is_player() || !tank.is_wreck() {
                continue;
            }
            tank.despawn_timer = Some(match tank.despawn_timer {
                None => seconds,
                Some(left) => left - dt,
            });
        }
    }

    /// Count the breather before the next wave down - the `WAVE N`
    /// banner's timer, and nothing else. Calling the wave once it reaches
    /// zero is `wave_phase`'s; a replica holds the banner up until the
    /// snapshot that pins the breather says it is over.
    pub(crate) fn tick_wave_banner(&mut self, dt: f32) {
        if let Some(left) = self.wave.gap {
            self.wave.gap = Some((left - dt).max(0.0));
        }
    }
}

/// Distance from `p` to the segment `a..b`.
fn segment_distance(p: Position, a: Position, b: Position) -> f32 {
    let ab = Position::new(b.x - a.x, b.y - a.y);
    let len2 = ab.x * ab.x + ab.y * ab.y;
    let t = if len2 > 0.0 { (((p.x - a.x) * ab.x + (p.y - a.y) * ab.y) / len2).clamp(0.0, 1.0) } else { 0.0 };
    p.distance_to(Position::new(a.x + ab.x * t, a.y + ab.y * t))
}
