//! A training round's rules (docs/training-stage.md): the map's
//! `[[training.beat]]` script run one beat at a time. A beat starts what it
//! names - a shell at the frog, crates dropped from the air, enemy tanks
//! rolled in through the map's gates - and is done once every condition it
//! sets holds; its doors (`map::CellObject::Door`) then open and the next
//! beat begins. The last beat done wins the round.
//!
//! Nothing a player does can stall a beat. An enemy wreck fades off the
//! field after `training_wreck_seconds`, so none keeps a gate's lane from
//! the next beat's tank, and a beat's tank that finds no free lane for
//! `training_lane_wait_seconds` drops onto the field out of sight the way
//! a band round places one. A beat started again takes back the tanks and
//! the crates it put down.
//!
//! Training is never lost. A wrecked seat comes back as a fresh tank in the
//! last door opened after `training_respawn_seconds`, and a frog that goes
//! down is back on its feet after `training_frog_revive_seconds`, with the
//! beat it fell in started again.
//!
//! Every check is a pure test of the round's state and the frame's events,
//! and every start is fixed by the map, so the phase draws no RNG of its
//! own; only what it sets going does (a roll-in's lane and chassis rolls,
//! the shell's damage). A map without a script runs none of it.

use hecs::Entity;

use crate::map::{CellObject, cell_to_world};
use crate::math::Vec2;
use crate::obstacle::{Material, Obstacle};
use crate::ai::{Ai, Role};
use crate::pickup::{Pickup, PickupKind};
use crate::shell::{Owner, Shell};
use crate::tank::{Tank, TankKind};
use crate::training::{Beat, Edge, RollInSpec, TrainingAi};
use crate::tuning::tuning;
use crate::{MAX_SEATS, OBSTACLE_GRID_SIZE, Position};

use crate::level::SpawnPlan;

use super::{Event, Frame, Game, with_frog, with_frog_mut, with_tank, with_tank_mut};

/// A flag on the field and whether a seat has it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Flag {
    pub at: Position,
    pub taken: bool,
}

/// Where a training round stands. Made by `init` from the map's script
/// and run by `Game::training_phase`.
#[derive(Clone, Debug, Default)]
pub(crate) struct Run {
    /// The beat running, 0-based; the script's length once every beat is
    /// done.
    pub beat: usize,
    /// The beats in the script.
    beats: usize,
    /// Whether the running beat's starts have gone.
    begun: bool,
    /// The round clock when the running beat began.
    since: f32,
    /// Enemy wrecks counted when the running beat began.
    wrecks_at: u32,
    /// The crate kinds a seat took since the running beat began.
    collected: Vec<PickupKind>,
    /// Roll-ins of the running beat still to come.
    rolls: Vec<RollInSpec>,
    /// The round clock since which a roll-in due has found no free lane,
    /// `None` while none waits.
    lane_busy_since: Option<f32>,
    /// The edge the running beat's shot at the frog comes from, until it
    /// has hurt the frog, and the round clock it was last fired at.
    shot: Option<(Edge, Option<f32>)>,
    /// The crates the running beat dropped, taken away if it starts
    /// again (one taken is already gone).
    crates: Vec<Entity>,
    /// The enemy tanks the running beat brought in, taken away if it
    /// starts again.
    tanks: Vec<Entity>,
    /// The map's flags, in cell order.
    pub flags: Vec<Flag>,
    /// Each seat's own start, where it comes back before any door opens.
    homes: [Position; MAX_SEATS],
    /// The round clock at which each wrecked seat comes back.
    respawn: [Option<f32>; MAX_SEATS],
    /// The round clock at which the fallen frog gets up again.
    revive: Option<f32>,
    /// The seats' shells do not refill on their own: the script starts
    /// them with a set number, and no seat has opened an ammo crate yet.
    pub shells_held: bool,
}

impl Run {
    /// Whether every beat is done.
    pub fn finished(&self) -> bool {
        self.beat >= self.beats
    }
}

/// What a training round shows: the beat running (1-based) of how many,
/// and its id. `None` on every other round.
#[derive(Clone, Debug, PartialEq, serde::Serialize)]
pub struct TrainingStatus {
    pub beat: usize,
    pub beats: usize,
    pub id: String,
    pub finished: bool,
}

impl Game {
    /// Set up a training map's run, from `init` once the seats and the map
    /// are on the field: the flags, each seat's home, and the seats'
    /// starting health and shells where the script sets them.
    pub(super) fn init_training(&mut self) {
        self.training = None;
        let Some(script) = self.map.training.clone() else { return };
        let flags = self
            .map
            .iter_cells()
            .filter(|(_, _, o)| matches!(o, CellObject::Flag))
            .map(|(c, r, _)| Flag { at: cell_to_world(c, r), taken: false })
            .collect();
        let mut homes = [Position::default(); MAX_SEATS];
        for (seat, entity) in self.players().into_iter().enumerate() {
            let Some(entity) = entity else { continue };
            homes[seat] = with_tank(&self.world, entity, |t| t.position);
            with_tank_mut(&self.world, entity, |t| {
                if let Some(share) = script.start_health {
                    t.damage = crate::MAX_DAMAGE * (1.0 - share.clamp(0.05, 1.0));
                }
                if let Some(shells) = script.start_shells {
                    t.shells_ammo = shells.max(0);
                }
            });
        }
        let shells_held = script.start_shells.is_some();
        self.training = Some(Run { beats: script.beat.len(), flags, homes, shells_held, ..Run::default() });
    }

    /// Take every training door away at once - what the linter checks a
    /// course on, so its pens read as the ground they will be once their
    /// beats are done.
    pub fn open_every_door(&mut self) {
        let doors: Vec<(Entity, rapier2d::prelude::RigidBodyHandle)> = self
            .world
            .query::<(Entity, &Obstacle)>()
            .iter()
            .filter(|(_, o)| o.material == Material::Door)
            .map(|(e, o)| (e, o.body))
            .collect();
        if doors.is_empty() {
            return;
        }
        for (entity, body) in doors {
            self.physics.remove_body(body);
            self.world.despawn(entity).ok();
        }
        self.refresh_edge_masks();
    }

    /// The flags a training round draws, empty on every other round.
    pub fn training_flags(&self) -> &[Flag] {
        self.training.as_ref().map_or(&[], |run| &run.flags)
    }

    /// Where a training round stands, `None` on every other round.
    pub fn training_status(&self) -> Option<TrainingStatus> {
        let run = self.training.as_ref()?;
        let script = self.map.training.as_ref()?;
        let id = script.beat(run.beat).map(|b| b.id.clone()).unwrap_or_default();
        Some(TrainingStatus { beat: (run.beat + 1).min(run.beats), beats: run.beats, id, finished: run.finished() })
    }

    /// Run the beat: take what the frame took, bring back what fell, start
    /// the beat if it has not begun, roll in what is due and, once it is
    /// done, open its doors and move on. After the frame's shots, blasts
    /// and pickups, before the doors' tiles are swept away.
    pub(super) fn training_phase(&mut self, f: &mut Frame) {
        let Some(mut run) = self.training.take() else { return };
        let Some(script) = self.map.training.clone() else { return };
        let seats = self.players.count();
        for event in &f.events {
            if let Event::PickupCollected { slot, kind, .. } = *event
                && slot < seats
            {
                run.collected.push(kind);
                run.shells_held &= kind != PickupKind::Ammo;
            }
        }
        self.take_flags(&mut run, f);
        self.fade_training_wrecks(f.dt);
        self.bring_back_seats(&mut run, f);
        if self.bring_back_frog(&mut run, f) {
            self.restart_beat(&mut run);
        }
        if !run.finished() {
            let beat = &script.beat[run.beat];
            if !run.begun {
                self.begin_beat(&mut run, beat);
            }
            self.roll_in_due(&mut run, f);
            self.walk_frog(beat);
            self.aim_at_frog(&mut run, beat, f);
            if self.beat_done(&run, beat) {
                self.open_doors(run.beat + 1, f);
                f.events.push(Event::BeatDone { beat: run.beat + 1 });
                run.beat += 1;
                run.begun = false;
            }
        }
        self.training = Some(run);
    }

    /// Hand a flag to the first seat whose hull reaches it, seats in
    /// order.
    fn take_flags(&self, run: &mut Run, f: &mut Frame) {
        let reach = tuning().training_flag_reach_px;
        let half_cell = OBSTACLE_GRID_SIZE * 0.5;
        for (seat, entity) in self.seats_on_field().into_iter().enumerate() {
            let Some(entity) = entity else { continue };
            let (pos, half, wreck) = with_tank(&self.world, entity, |t| (t.position, t.size() * 0.5, t.is_wreck()));
            if wreck {
                continue;
            }
            for flag in run.flags.iter_mut().filter(|flag| !flag.taken) {
                let near = half + half_cell + reach;
                if (pos.x - flag.at.x).abs() < near && (pos.y - flag.at.y).abs() < near {
                    flag.taken = true;
                    f.events.push(Event::FlagTaken { seat, x: flag.at.x, y: flag.at.y });
                }
            }
        }
    }

    /// Start the clock on every wrecked seat and put each back on the
    /// field once its clock runs out: a fresh tank of its own chassis in
    /// the last door opened, or at its own start before any has, keeping
    /// the shells it had.
    fn bring_back_seats(&mut self, run: &mut Run, f: &mut Frame) {
        let delay = tuning().training_respawn_seconds;
        for seat in 0..self.players.count() {
            let Some(entity) = self.seat(seat) else { continue };
            if !with_tank(&self.world, entity, Tank::is_wreck) {
                run.respawn[seat] = None;
                continue;
            }
            let Some(at) = run.respawn[seat] else {
                run.respawn[seat] = Some(self.time + delay);
                continue;
            };
            if self.time < at {
                continue;
            }
            run.respawn[seat] = None;
            let to = self.last_door_opened(run.beat).unwrap_or(run.homes[seat]);
            self.respawn_seat(seat, entity, to);
            f.events.push(Event::TankEntered { slot: seat });
        }
    }

    /// The middle cell of the last door a done beat opened, if any has.
    fn last_door_opened(&self, done: usize) -> Option<Position> {
        let mut best: Option<(u8, Vec<(i32, i32)>)> = None;
        for (c, r, o) in self.map.iter_cells() {
            let CellObject::Door { beat } = *o else { continue };
            if beat == 0 || beat as usize > done {
                continue;
            }
            match &mut best {
                Some((b, cells)) if *b == beat => cells.push((c, r)),
                Some((b, _)) if *b > beat => {}
                _ => best = Some((beat, vec![(c, r)])),
            }
        }
        let (_, cells) = best?;
        let (c, r) = cells[cells.len() / 2];
        Some(cell_to_world(c, r))
    }

    /// Seat `seat`'s wreck made a fresh tank of its own chassis standing at
    /// `at`, facing up, with a body: what `init` would have spawned there,
    /// less the shield roll, keeping its shells.
    fn respawn_seat(&mut self, seat: usize, entity: Entity, at: Position) {
        if let Some(body) = with_tank(&self.world, entity, |t| t.body) {
            self.physics.remove_body(body);
        }
        with_tank_mut(&self.world, entity, |t| {
            *t = Tank {
                row: t.row,
                shell_variant: t.shell_variant,
                damage_variant: t.damage_variant,
                track_wobble_amp: t.track_wobble_amp,
                track_wobble_freq: t.track_wobble_freq,
                track_wobble_phase: t.track_wobble_phase,
                track_scale_jitter: t.track_scale_jitter,
                shells_ammo: t.shells_ammo,
                position: at,
                ring_position: at,
                owner: t.owner,
                ..Tank::default()
            };
        });
        let (half, mass) = with_tank(&self.world, entity, |t| (t.move_half_extents(false), t.mass()));
        let body = self.physics.spawn_tank(at, half, mass);
        with_tank_mut(&self.world, entity, |t| t.body = Some(body));
        self.player_fire_held_last_frame[seat] = false;
        self.engage[seat].clear();
    }

    /// Start the frog's clock once it is down and stand it up again when
    /// it runs out, at full health where it fell. True on the frame it
    /// gets up: the beat starts again.
    fn bring_back_frog(&mut self, run: &mut Run, f: &mut Frame) -> bool {
        let Some(frog) = self.frog else { return false };
        if !with_frog(&self.world, frog, crate::frog::Frog::is_dead) {
            run.revive = None;
            return false;
        }
        let Some(at) = run.revive else {
            run.revive = Some(self.time + tuning().training_frog_revive_seconds);
            return false;
        };
        if self.time < at {
            return false;
        }
        run.revive = None;
        let pos = with_frog_mut(&self.world, frog, |fr| {
            fr.health = fr.max_health;
            fr.death_elapsed = None;
            fr.hurt_timer = 0.0;
            fr.position
        });
        f.events.push(Event::FrogRevived { x: pos.x, y: pos.y });
        true
    }

    /// The running beat from the top: the tanks it brought in taken away
    /// (their wrecks stay), its counts and its roll-ins reset, and its
    /// starts to go again.
    fn restart_beat(&mut self, run: &mut Run) {
        for entity in std::mem::take(&mut run.tanks) {
            let Ok((wreck, body)) = self.world.get::<&Tank>(entity).map(|t| (t.is_wreck(), t.body)) else { continue };
            if wreck {
                continue;
            }
            if let Some(body) = body {
                self.physics.remove_body(body);
            }
            self.world.despawn(entity).ok();
        }
        for entity in std::mem::take(&mut run.crates) {
            if self.world.get::<&Pickup>(entity).is_ok() {
                self.world.despawn(entity).ok();
            }
        }
        run.begun = false;
        run.rolls.clear();
        run.lane_busy_since = None;
    }

    /// Count every enemy wreck's fade down and arm a new one's, so
    /// `despawn_wrecks` takes it off the field `training_wreck_seconds`
    /// after it went - a band round keeps its wrecks otherwise, and one in
    /// a gate's lane would keep the next beat's tank out for good. A wave
    /// round's wrecks fade on their own clock (`fade_wrecks`).
    fn fade_training_wrecks(&mut self, dt: f32) {
        if matches!(self.spawn_plan, SpawnPlan::Waves { .. }) {
            return;
        }
        let seconds = tuning().training_wreck_seconds;
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

    /// The running beat's starts.
    fn begin_beat(&mut self, run: &mut Run, beat: &Beat) {
        run.begun = true;
        run.since = self.time;
        run.wrecks_at = self.enemies_destroyed;
        run.collected.clear();
        run.rolls = beat.start.roll_in.clone();
        run.lane_busy_since = None;
        run.shot = beat.start.shoot_frog.map(|edge| (edge, None));
        for drop in &beat.start.drop {
            let (c, r) = drop.at;
            run.crates.push(self.world.spawn((Pickup::dropped(drop.kind, cell_to_world(c, r), Some(self.time)),)));
        }
    }

    /// The running beat's shot at the frog, once the frog stands on the
    /// beat's cell - so it flies along the row the beat was laid out for,
    /// not into a wall the frog is still hopping past - and again every
    /// `training_frog_shot_retry_seconds` until one has hurt it: a beat
    /// that waits on the frog's kit can only end once the frog needs it.
    fn aim_at_frog(&mut self, run: &mut Run, beat: &Beat, f: &mut Frame) {
        let (Some((edge, fired)), Some(frog)) = (run.shot, self.frog) else { return };
        if with_frog(&self.world, frog, |fr| fr.is_dead() || fr.health < fr.max_health) {
            run.shot = None;
            return;
        }
        let arrived = beat.frog.is_none_or(|(c, r)| {
            with_frog(&self.world, frog, |fr| fr.position.distance_to(cell_to_world(c, r)) <= OBSTACLE_GRID_SIZE * 1.5)
        });
        if !arrived || fired.is_some_and(|at| self.time - at < tuning().training_frog_shot_retry_seconds) {
            return;
        }
        self.shoot_frog(edge, f);
        run.shot = Some((edge, Some(self.time)));
    }

    /// One heavy shell fired at the players' frog from `edge`, a cell in
    /// from it, along the frog's row or column. It flies and lands like
    /// any enemy shell.
    fn shoot_frog(&mut self, edge: Edge, f: &mut Frame) {
        let Some(frog) = self.frog else { return };
        let target = with_frog(&self.world, frog, |fr| fr.position);
        let inset = OBSTACLE_GRID_SIZE * 1.5;
        let (from, rotation) = match edge {
            Edge::North => (Position::new(target.x, inset), 180.0_f32),
            Edge::East => (Position::new(f.width - inset, target.y), 270.0),
            Edge::South => (Position::new(target.x, f.height - inset), 0.0),
            Edge::West => (Position::new(inset, target.y), 90.0),
        };
        let rad = rotation.to_radians();
        let speed = tuning().shell_speed;
        let velocity = Vec2::new(rad.sin() * speed, -rad.cos() * speed);
        let id = self.take_shot_id();
        let slot = self.take_slot();
        let row = TankKind::Titan.row();
        let shell = Shell::at(id, from, from, velocity, rotation, 0, row, Owner::Enemy(slot));
        self.spawn_shot(shell, 0);
    }

    /// Whether the players' frog is on its way to the running beat's cell,
    /// more than a cell and a half from it.
    pub(super) fn frog_walking(&self) -> bool {
        let (Some(run), Some(script), Some(frog)) = (&self.training, &self.map.training, self.frog) else { return false };
        let Some((c, r)) = script.beat(run.beat).and_then(|b| b.frog) else { return false };
        with_frog(&self.world, frog, |fr| fr.position.distance_to(cell_to_world(c, r)) > OBSTACLE_GRID_SIZE * 1.5)
    }

    /// The frog hops on toward the running beat's cell, a hop at a time
    /// along the nav grid, so it goes through the doors its beats opened
    /// and waits for the player in the next pen. At its cell it still bites
    /// an enemy and shies from one; it never shies from a seat
    /// (`frog_reflexes`).
    fn walk_frog(&mut self, beat: &Beat) {
        let (Some(frog), Some((c, r))) = (self.frog, beat.frog) else { return };
        let target = cell_to_world(c, r);
        let (pos, ready, reach) = with_frog(&self.world, frog, |fr| (fr.position, fr.can_hop() && fr.hop_timer <= 0.0, fr.hop_distance()));
        if !ready || pos.distance_to(target) < 2.0 {
            return;
        }
        // No route while a door still stands in the way: the frog waits
        // rather than hop through it. Within a cell and a half of where it
        // is going - the grid has no step left to give inside the goal's
        // cell - the last hop is straight there.
        let next = match self.nav.layer().next_step(pos, target) {
            Some(next) => next,
            None if pos.distance_to(target) <= OBSTACLE_GRID_SIZE * 1.5 => target,
            None => return,
        };
        let dist = pos.distance_to(next);
        if dist < 0.5 {
            return;
        }
        let step = dist.min(reach * tuning().training_frog_stride);
        let to = Position::new(pos.x + (next.x - pos.x) * step / dist, pos.y + (next.y - pos.y) * step / dist);
        with_frog_mut(&self.world, frog, |fr| {
            fr.start_hop(to);
            fr.hop_cooldown = 0.0;
        });
    }

    /// Roll in every tank of the beat whose time has come, through a free
    /// lane; one with every lane busy waits for the next frame, and once
    /// the lanes have stayed busy `training_lane_wait_seconds` it drops onto
    /// the field out of sight instead (`spawn_in_band`), so a seat parked
    /// in a lane never holds the beat up.
    fn roll_in_due(&mut self, run: &mut Run, f: &mut Frame) {
        let elapsed = self.time - run.since;
        let wait = tuning().training_lane_wait_seconds;
        let mut waiting = Vec::new();
        for spec in std::mem::take(&mut run.rolls) {
            if elapsed < spec.after {
                waiting.push(spec);
                continue;
            }
            // A dummy hunts the frog and never fires at a seat.
            let (role, frog_only) = match spec.ai {
                Some(TrainingAi::Dummy) => (Role::Hunter, true),
                None => (Role::Player, false),
            };
            if let Some(entity) = self.training_roll_in(f, spec.tank.row(), role, frog_only) {
                run.tanks.push(entity);
                run.lane_busy_since = None;
                continue;
            }
            let since = *run.lane_busy_since.get_or_insert(self.time);
            if self.time - since < wait {
                waiting.push(spec);
                continue;
            }
            let entity = self.spawn_in_band(f, spec.tank.row());
            let mut ai = Ai::with_role(role);
            ai.frog_only = frog_only;
            ai.field.called = self.field_map;
            ai.field.wave = self.field_map;
            self.world.insert_one(entity, ai).expect("the tank spawn_in_band just placed");
            run.tanks.push(entity);
            run.lane_busy_since = None;
        }
        run.rolls = waiting;
    }

    /// Whether every condition `beat` sets holds now.
    fn beat_done(&self, run: &Run, beat: &Beat) -> bool {
        let done = &beat.done;
        if let Some(n) = done.flags
            && (run.flags.iter().filter(|f| f.taken).count() as u32) < n
        {
            return false;
        }
        if let Some(kind) = done.collect
            && !run.collected.contains(&kind)
        {
            return false;
        }
        if !done.destroyed.is_empty() || !done.destroyed_all.is_empty() {
            let standing: Vec<(i32, i32)> = self
                .world
                .query::<&Obstacle>()
                .iter()
                .filter(|o| !o.destroyed)
                .map(|o| o.cell())
                .collect();
            if !done.destroyed.is_empty() && done.destroyed.iter().all(|cell| standing.contains(cell)) {
                return false;
            }
            if done.destroyed_all.iter().any(|cell| standing.contains(cell)) {
                return false;
            }
        }
        if let Some(col) = done.past_col {
            let east = (col + 1) as f32 * OBSTACLE_GRID_SIZE;
            let past = self.seats_on_field()[0]
                .is_some_and(|e| with_tank(&self.world, e, |t| !t.is_wreck() && t.position.x >= east));
            if !past {
                return false;
            }
        }
        if done.frog_full {
            let full = self.frog.is_some_and(|e| with_frog(&self.world, e, |fr| !fr.is_dead() && fr.health >= fr.max_health));
            if !full {
                return false;
            }
        }
        if let Some(n) = done.wrecks
            && self.enemies_destroyed.saturating_sub(run.wrecks_at) < n
        {
            return false;
        }
        // The beat's tanks still to come keep it running: a wave of them
        // is done when the last is down, not when the first is.
        run.rolls.is_empty()
    }

    /// Take away every door the end of beat `beat` (1-based) opens.
    fn open_doors(&mut self, beat: usize, f: &mut Frame) {
        for o in self.world.query::<&mut Obstacle>().iter() {
            if o.material == Material::Door && o.variant as usize == beat && !o.destroyed {
                o.destroyed = true;
                f.events.push(Event::DoorOpened { beat, x: o.position.x, y: o.position.y });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::MapFile;
    use crate::simulation::{Input, Outcome};

    /// A 16 x 9 course: the seat starts at (2, 4), a flag at (6, 4), a
    /// door of beat 1 across column 9 (rows 3-5, iron above and below),
    /// the frog at (12, 4) and gates on the east edge. `script` is
    /// appended as the map's `[[training.beat]]` tables.
    fn course(script: &str) -> Game {
        let mut map = String::from(
            "version = 1\ntanks = 0\nsize = [16.0, 9.0]\nmission.kind = \"protect\"\n\
             cells.\"2,4\" = { kind = \"start\" }\ncells.\"6,4\" = { kind = \"flag\" }\n\
             cells.\"12,4\" = { kind = \"frog\" }\n",
        );
        for r in 0..9 {
            let cell = if (3..=5).contains(&r) { "{ kind = \"door\", beat = 1 }" } else { "{ kind = \"wall\", material = \"iron\" }" };
            map.push_str(&format!("cells.\"9,{r}\" = {cell}\n"));
        }
        for r in 3..=5 {
            map.push_str(&format!("cells.\"15,{r}\" = {{ kind = \"gate\" }}\n"));
        }
        map.push_str(script);
        let mut game = Game::default();
        game.seed_override = Some(7);
        game.map = MapFile::from_toml_str(&map).expect("course parses");
        let (w, h) = game.map.field_size();
        game.init(w, h);
        game
    }

    fn step(game: &mut Game, frames: usize) {
        let (w, h) = game.map.field_size();
        for _ in 0..frames {
            game.update(Input::default(), 1.0 / 60.0, w, h);
        }
    }

    fn doors(game: &Game) -> usize {
        game.world.query::<&Obstacle>().iter().filter(|o| o.material == Material::Door).count()
    }

    fn put_seat(game: &mut Game, col: i32, row: i32) {
        game.debug_teleport(0, cell_to_world(col, row), None).expect("seat 0 is on the field");
    }

    const FLAG_THEN_PAST: &str = "\n[[training.beat]]\ndone = { flags = 1 }\n\n[[training.beat]]\ndone = { past_col = 9 }\n";

    #[test]
    fn a_door_stands_until_its_beat_is_done_then_opens() {
        let mut game = course(FLAG_THEN_PAST);
        step(&mut game, 30);
        assert_eq!(doors(&game), 3, "the door holds while its beat runs");
        assert_eq!(game.training_status().map(|s| s.beat), Some(1));
        put_seat(&mut game, 6, 4);
        step(&mut game, 2);
        assert_eq!(doors(&game), 0, "taking the flag finished beat 1 and opened its door");
        assert!(game.training_flags()[0].taken);
        assert_eq!(game.training_status().map(|s| s.beat), Some(2));
        assert_eq!(game.outcome(), Outcome::Playing);
    }

    #[test]
    fn the_last_beat_done_wins_the_round() {
        let mut game = course(FLAG_THEN_PAST);
        put_seat(&mut game, 6, 4);
        step(&mut game, 2);
        put_seat(&mut game, 11, 4);
        step(&mut game, 2);
        assert_eq!(game.outcome(), Outcome::Won);
    }

    #[test]
    fn a_wrecked_seat_comes_back_in_the_last_door_opened() {
        let mut game = course(FLAG_THEN_PAST);
        put_seat(&mut game, 6, 4);
        step(&mut game, 2);
        game.debug_kill(0).expect("seat 0 can be killed");
        step(&mut game, 2);
        let seat = game.player().expect("seat 0");
        assert!(with_tank(&game.world, seat, Tank::is_wreck));
        assert_eq!(game.outcome(), Outcome::Playing, "training is never lost");
        let frames = (tuning().training_respawn_seconds * 60.0) as usize + 2;
        step(&mut game, frames);
        let (wreck, at) = with_tank(&game.world, seat, |t| (t.is_wreck(), t.position));
        assert!(!wreck, "the seat came back");
        assert!(at.distance_to(cell_to_world(9, 4)) < 1.0, "in the door's middle cell, at {at:?}");
    }

    #[test]
    fn a_fallen_frog_gets_up_and_the_round_goes_on() {
        let mut game = course(FLAG_THEN_PAST);
        let frog = game.frog.expect("a protect round has a frog");
        with_frog_mut(&game.world, frog, |fr| fr.damage(10_000.0));
        step(&mut game, 2);
        assert_eq!(game.outcome(), Outcome::Playing, "training is never lost");
        let frames = (tuning().training_frog_revive_seconds * 60.0) as usize + 2;
        step(&mut game, frames);
        assert!(with_frog(&game.world, frog, |fr| !fr.is_dead() && fr.health >= fr.max_health));
    }

    #[test]
    fn a_shot_from_the_east_hurts_the_frog_and_its_kit_drops() {
        let script = "\n[[training.beat]]\nstart = { shoot_frog = \"east\", drop = [{ kind = \"frog_health\", at = [11, 7] }] }\ndone = { collect = \"frog_health\" }\n";
        let mut game = course(script);
        let frog = game.frog.expect("a frog");
        let mut hurt = false;
        for _ in 0..180 {
            step(&mut game, 1);
            hurt |= with_frog(&game.world, frog, |fr| fr.health < fr.max_health);
        }
        assert!(hurt, "the shell reached the frog");
        let kit = game.world.query::<&crate::pickup::Pickup>().iter().any(|p| p.kind == PickupKind::FrogHealth);
        assert!(kit, "the frog kit dropped");
        put_seat(&mut game, 11, 7);
        step(&mut game, 2);
        assert_eq!(game.outcome(), Outcome::Won, "taking the kit finished the only beat");
    }

    #[test]
    fn a_beats_tank_rolls_in_through_a_gate_and_its_wreck_ends_the_beat() {
        let script = "\n[[training.beat]]\nstart = { roll_in = [{ tank = \"scout\", ai = \"dummy\" }] }\ndone = { wrecks = 1 }\n";
        let mut game = course(script);
        let mut entered = None;
        for _ in 0..600 {
            step(&mut game, 1);
            entered = entered.or(game.events().iter().find_map(|e| match *e {
                Event::TankEntered { slot } if slot >= game.first_enemy_slot() => Some(slot),
                _ => None,
            }));
            if entered.is_some() {
                break;
            }
        }
        let slot = entered.expect("the scout rolled in");
        assert_eq!(game.outcome(), Outcome::Playing);
        game.debug_kill(slot).expect("the scout can be killed");
        step(&mut game, 2);
        assert_eq!(game.outcome(), Outcome::Won);
    }

    #[test]
    fn the_frog_walks_to_its_next_beats_cell_through_the_door_that_opened() {
        let script = "\n[[training.beat]]\nfrog = [12, 4]\ndone = { flags = 1 }\n\n[[training.beat]]\nfrog = [5, 4]\ndone = { flags = 2 }\n";
        let mut game = course(script);
        let frog = game.frog.expect("a frog");
        step(&mut game, 60);
        let still = with_frog(&game.world, frog, |fr| fr.position);
        assert!(still.distance_to(cell_to_world(12, 4)) < 1.0, "it stays on its cell while the door is shut, at {still:?}");
        put_seat(&mut game, 6, 4);
        step(&mut game, 2);
        // Out of its way, so the tank does not stand in its path.
        put_seat(&mut game, 14, 7);
        step(&mut game, 600);
        let at = with_frog(&game.world, frog, |fr| fr.position);
        assert!(at.distance_to(cell_to_world(5, 4)) < 4.0, "the frog hopped to beat 2's cell, at {at:?}");
    }

    /// Open the course's door and drive seat 0 east along the frog's row
    /// from (6, 4) for `frames`, and answer the frog's position at the
    /// start and whether it hopped.
    fn drive_up_to_the_frog(game: &mut Game, frames: usize) -> (Position, bool) {
        let frog = game.frog.expect("a frog");
        game.open_every_door();
        put_seat(game, 6, 4);
        step(game, 1);
        let start = with_frog(&game.world, frog, |fr| fr.position);
        let (w, h) = game.map.field_size();
        let east = Input::single(crate::ai::Intent { move_dir: Some(crate::tank::Dir::Right), ..Default::default() });
        let mut hopped = false;
        for _ in 0..frames {
            game.update(east, 1.0 / 60.0, w, h);
            hopped |= with_frog(&game.world, frog, |fr| fr.position.distance_to(start) > 0.5);
        }
        (start, hopped)
    }

    /// A tank driven right up to a training frog standing on its beat's
    /// cell leaves it where it is: the frog leads the seat and never shies
    /// from it. The same drive on a map with no script sends it hopping.
    #[test]
    fn a_training_frog_lets_the_seat_drive_right_up_to_it() {
        let script = "\n[[training.beat]]\nfrog = [12, 4]\ndone = { flags = 9 }\n";
        let mut game = course(script);
        let (start, hopped) = drive_up_to_the_frog(&mut game, 180);
        let seat = game.player().expect("seat 0");
        let near = with_tank(&game.world, seat, |t| t.position.distance_to(start));
        let avoid = game.frog.map(|e| with_frog(&game.world, e, crate::frog::Frog::avoid_range)).expect("a frog");
        assert!(near < avoid, "the seat came within the frog's avoid range ({near} px of {avoid})");
        assert!(!hopped, "the training frog stayed put");

        let mut plain = course("");
        assert!(plain.training_status().is_none(), "no script, no training round");
        let (_, hopped) = drive_up_to_the_frog(&mut plain, 180);
        assert!(hopped, "a frog in any other round shies from the tank");
    }

    /// A training frog still hops away from an enemy tank.
    #[test]
    fn a_training_frog_still_shies_from_an_enemy_tank() {
        let script = "\n[[training.beat]]\nfrog = [12, 4]\ndone = { flags = 9 }\n";
        let mut game = course(script);
        put_seat(&mut game, 2, 7);
        step(&mut game, 2);
        let frog = game.frog.expect("a frog");
        let start = with_frog(&game.world, frog, |fr| fr.position);
        game.debug_spawn_enemy(cell_to_world(12, 6), Some(TankKind::Scout.row()), Some(Role::Guard)).expect("an enemy spawns");
        let mut hopped = false;
        for _ in 0..30 {
            step(&mut game, 1);
            hopped |= with_frog(&game.world, frog, |fr| fr.position.distance_to(start) > 0.5);
        }
        assert!(hopped, "the frog shied from the enemy");
    }

    #[test]
    fn a_script_that_starts_with_no_shells_holds_the_refill_until_an_ammo_crate() {
        let script = "\nstart_shells = 0\n\n[[training.beat]]\ndone = { flags = 1 }\n";
        let mut map_script = String::from("\n[training]");
        map_script.push_str(script);
        let mut game = course(&map_script);
        let seat = game.player().expect("seat 0");
        step(&mut game, 600);
        assert_eq!(with_tank(&game.world, seat, |t| t.shells_ammo), 0, "ten seconds and not a shell");
        let at = with_tank(&game.world, seat, |t| t.position);
        crate::simulation::spawn_pickup_at(&mut game.world, at, PickupKind::Ammo, None);
        step(&mut game, 2);
        let after_crate = with_tank(&game.world, seat, |t| t.shells_ammo);
        assert!(after_crate > 0, "the crate's shells");
        if after_crate < tuning().max_shells {
            step(&mut game, (tuning().shell_recharge_seconds * 60.0) as usize + 2);
            assert!(with_tank(&game.world, seat, |t| t.shells_ammo) > after_crate, "and the refill runs again");
        }
    }

    #[test]
    fn a_dummy_goes_for_the_frog_and_never_fires_at_the_seat() {
        let script = "\n[[training.beat]]\nstart = { roll_in = [{ tank = \"scout\", ai = \"dummy\" }] }\ndone = { wrecks = 1 }\n";
        let mut game = course(script);
        // The seat stands clear of the gate's lane - a tank in a lane keeps
        // it busy, and nothing rolls in through it.
        put_seat(&mut game, 7, 7);
        let mut at_frog = false;
        for _ in 0..1800 {
            step(&mut game, 1);
            for event in game.events() {
                match event {
                    Event::Hit { target: crate::simulation::HitTarget::Player { .. }, .. } => panic!("a dummy hit the seat"),
                    Event::Hit { target: crate::simulation::HitTarget::Frog { .. }, .. } => at_frog = true,
                    _ => {}
                }
            }
            let dummy = game.world.query::<&crate::ai::Ai>().iter().next().map(|ai| ai.frog_only);
            assert!(dummy != Some(false), "the beat's tank is a dummy");
        }
        assert!(at_frog, "and it fired at the frog");
    }

    /// Run `game` until an enemy rolls in, and answer its slot.
    fn next_arrival(game: &mut Game, frames: usize) -> Option<usize> {
        for _ in 0..frames {
            step(game, 1);
            let slot = game.events().iter().find_map(|e| match *e {
                Event::TankEntered { slot } if slot >= game.first_enemy_slot() => Some(slot),
                _ => None,
            });
            if slot.is_some() {
                return slot;
            }
        }
        None
    }

    /// The wreck of one beat's tank, killed the moment it rolls in - in
    /// its gate's lane - never keeps the next beat's tank out.
    #[test]
    fn a_wreck_in_the_lane_never_keeps_the_next_beats_tank_out() {
        let script = "\n[[training.beat]]\nstart = { roll_in = [{ tank = \"scout\", ai = \"dummy\" }] }\ndone = { wrecks = 1 }\n\n\
                      [[training.beat]]\nstart = { roll_in = [{ tank = \"scout\", ai = \"dummy\", after = 1.0 }] }\ndone = { wrecks = 1 }\n";
        let mut game = course(script);
        put_seat(&mut game, 7, 7);
        let first = next_arrival(&mut game, 600).expect("the first beat's scout rolled in");
        game.debug_kill(first).expect("the scout can be killed");
        let second = next_arrival(&mut game, 60 * 60).expect("the second beat's scout rolled in past the first one's wreck");
        game.debug_kill(second).expect("the scout can be killed");
        step(&mut game, 2);
        assert_eq!(game.outcome(), Outcome::Won);
    }

    /// A seat parked on the gate's lane holds a beat's tank only for
    /// `training_lane_wait_seconds`: then it drops onto the field anyway.
    #[test]
    fn a_seat_parked_in_the_lane_holds_the_beats_tank_only_so_long() {
        let script = "\n[[training.beat]]\nstart = { roll_in = [{ tank = \"scout\", ai = \"dummy\" }] }\ndone = { wrecks = 1 }\n";
        let mut game = course(script);
        let gate = game.map.gate_cells()[1];
        put_seat(&mut game, gate.0 - 1, gate.1);
        let wait = tuning().training_lane_wait_seconds;
        let slot = next_arrival(&mut game, ((wait + 2.0) * 60.0) as usize).expect("the scout came in all the same");
        let entity = game.world.query::<(hecs::Entity, &Tank)>().iter().find(|(_, t)| t.owner_slot() == slot).map(|(e, _)| e).unwrap();
        assert_eq!(game.world.get::<&Ai>(entity).map(|ai| ai.frog_only).ok(), Some(true), "still the beat's dummy");
        game.debug_kill(slot).expect("the scout can be killed");
        step(&mut game, 2);
        assert_eq!(game.outcome(), Outcome::Won);
    }

    /// A beat started again - its frog fell - puts down its crates once,
    /// not once more for every start.
    #[test]
    fn a_beat_started_again_takes_back_the_crates_it_dropped() {
        let script = "\n[[training.beat]]\nstart = { drop = [{ kind = \"shield\", at = [4, 7] }] }\ndone = { flags = 1 }\n";
        let mut game = course(script);
        step(&mut game, 2);
        let crates = |game: &Game| game.world.query::<&Pickup>().iter().filter(|p| p.kind == PickupKind::Shield).count();
        assert_eq!(crates(&game), 1);
        let frog = game.frog.expect("a frog");
        for _ in 0..3 {
            with_frog_mut(&game.world, frog, |fr| fr.damage(fr.max_health));
            step(&mut game, ((tuning().training_frog_revive_seconds + 0.5) * 60.0) as usize);
        }
        assert_eq!(crates(&game), 1, "one shield crate after three starts");
    }

    fn boot_camp(seed: u64) -> Game {
        let mut game = Game::default();
        game.seed_override = Some(seed);
        game.map = MapFile::from_toml_str(include_str!("../../maps/boot-camp.toml")).expect("boot-camp parses");
        let (w, h) = game.map.field_size();
        game.init(w, h);
        game
    }

    /// A beat whose `destroyed_all` lists several cells runs until the
    /// last of them is gone, not the first.
    #[test]
    fn a_beat_that_needs_every_cell_gone_waits_for_the_last() {
        let mut script = String::new();
        for r in [2, 6] {
            script.push_str(&format!("cells.\"5,{r}\" = {{ kind = \"wall\", material = \"wood\" }}\n"));
        }
        script.push_str("\n[[training.beat]]\ndone = { destroyed_all = [[5, 2], [5, 6]] }\n");
        let mut game = course(&script);
        let knock = |game: &mut Game, cell: (i32, i32)| {
            for o in game.world.query::<&mut Obstacle>().iter() {
                if o.cell() == cell {
                    o.destroyed = true;
                }
            }
        };
        step(&mut game, 2);
        knock(&mut game, (5, 2));
        step(&mut game, 2);
        assert_eq!(game.outcome(), Outcome::Playing, "one of the two walls still stands");
        knock(&mut game, (5, 6));
        step(&mut game, 2);
        assert_eq!(game.outcome(), Outcome::Won, "both gone ends the only beat");
    }

    /// Boot Camp's drum range from the road: a shot or two north at the
    /// lone oil drum - a shell can fly over a drum, glance off it or leave
    /// it standing on a low roll - and the fire and the blasts take every
    /// drum on the range, the shed by them goes too, a fuel drum is thrown,
    /// and the frog behind its sandbags and the seat on the road come
    /// through it - over a handful of seeds, since the rolls are the
    /// round's.
    #[test]
    fn boot_camps_drum_range_goes_up_from_the_lone_drum_and_spares_the_frog_and_the_seat() {
        let range = [(31, 7), (35, 6), (36, 6), (35, 7), (36, 7)];
        for seed in 1..=6 {
            let mut game = boot_camp(seed);
            game.open_every_door();
            let run = game.training.as_mut().expect("a training run");
            run.beat = 4;
            run.begun = false;
            run.shells_held = false;
            assert_eq!(game.training_status().map(|s| s.id), Some("barrels".to_string()));
            let frog = game.frog.expect("a frog");
            let seat = game.player().expect("seat 0");
            // Out of the frog's way while it walks to its cell.
            put_seat(&mut game, 38, 16);
            let home = cell_to_world(33, 15);
            for _ in 0..60 * 90 {
                if with_frog(&game.world, frog, |fr| fr.position.distance_to(home) < 4.0) {
                    break;
                }
                step(&mut game, 1);
            }
            assert!(with_frog(&game.world, frog, |fr| fr.position.distance_to(home) < 4.0), "seed {seed}: the frog took cover");
            game.debug_teleport(0, cell_to_world(31, 11), Some(0.0)).expect("seat 0 is on the field");
            with_tank_mut(&game.world, seat, |t| {
                t.shells_ammo = 10;
                t.damage = 0.0;
            });
            let (w, h) = game.map.field_size();
            let fire = crate::ai::Intent { face: Some(crate::tank::Dir::Up), fire: true, ..Default::default() };
            let mut launched = 0;
            let mut frog_hurt = false;
            let mut shots = 0;
            for frame in 0..60 * 30 {
                // Fire again every two seconds while the lone drum stands.
                let lone_stands = game.world.query::<&Obstacle>().iter().any(|o| !o.destroyed && o.cell() == range[0]);
                let input = if frame % 120 == 0 && lone_stands {
                    shots += 1;
                    Input::single(fire)
                } else {
                    Input::default()
                };
                game.update(input, 1.0 / 60.0, w, h);
                launched += game.events().iter().filter(|e| matches!(e, Event::DrumLaunched { .. })).count();
                frog_hurt |= with_frog(&game.world, frog, |fr| fr.health < fr.max_health);
                assert!(!with_tank(&game.world, seat, Tank::is_wreck), "seed {seed}: the seat on the road was wrecked");
            }
            let standing: Vec<(i32, i32)> = game.world.query::<&Obstacle>().iter().filter(|o| !o.destroyed).map(|o| o.cell()).collect();
            assert!(range.iter().all(|c| !standing.contains(c)), "seed {seed}: every drum went up");
            assert!(shots <= 4, "seed {seed}: the lone drum took {shots} shots");
            assert!(game.training_status().is_some_and(|s| s.id == "onward"), "seed {seed}: the range is done");
            assert!(launched >= 1, "seed {seed}: a fuel drum was thrown");
            assert!((33..=36).any(|c| !standing.contains(&(c, 4))), "seed {seed}: a blast took the shed");
            assert!(!frog_hurt, "seed {seed}: the frog was hurt");
            let lost = with_tank(&game.world, seat, |t| t.damage);
            assert!(lost < crate::MAX_DAMAGE * 0.25, "seed {seed}: the seat on the road lost {lost}");
        }
    }

    /// Boot Camp from the first beat to the last, the player's way: the
    /// flags, the ammo crate, the brick wall, the drum range, the pen, the
    /// frog's kit, the dummy shot the moment it rolls in - its wreck in the
    /// gate's lane - and the last beat's scout.
    #[test]
    fn boot_camp_plays_through_to_the_end() {
        let mut game = boot_camp(11);
        let beat = |game: &Game| game.training_status().map(|s| s.beat).unwrap_or(0);
        let until = |game: &mut Game, b: usize, what: &str| {
            for _ in 0..60 * 60 {
                if beat(game) >= b || game.outcome() == Outcome::Won {
                    return;
                }
                step(game, 1);
            }
            panic!("stuck before beat {b}: {what} (on beat {})", beat(game));
        };
        step(&mut game, 2);
        for (c, r) in [(8, 7), (8, 15), (3, 15)] {
            put_seat(&mut game, c, r);
            step(&mut game, 2);
        }
        until(&mut game, 2, "the flags");
        put_seat(&mut game, 17, 7);
        until(&mut game, 3, "the ammo crate");
        for o in game.world.query::<&mut Obstacle>().iter() {
            if o.cell() == (25, 11) {
                o.destroyed = true;
            }
        }
        until(&mut game, 4, "the brick wall");
        put_seat(&mut game, 30, 11);
        until(&mut game, 5, "into the drum range");
        game.debug_detonate(cell_to_world(31, 7)).expect("the lone oil drum");
        until(&mut game, 6, "the drums");
        put_seat(&mut game, 42, 11);
        until(&mut game, 7, "into the pen");
        // The shot at the frog lands, then the seat takes its kit.
        let frog = game.frog.expect("a frog");
        let mut hurt = false;
        for _ in 0..60 * 40 {
            step(&mut game, 1);
            hurt = with_frog(&game.world, frog, |fr| fr.health < fr.max_health);
            if hurt {
                break;
            }
        }
        assert!(hurt, "the shot found the frog once it reached its pen");
        put_seat(&mut game, 43, 15);
        until(&mut game, 8, "the frog's kit");
        put_seat(&mut game, 42, 14);
        let dummy = next_arrival(&mut game, 60 * 20).expect("the dummy rolled in");
        game.debug_kill(dummy).expect("the dummy can be killed");
        until(&mut game, 9, "the dummy");
        let scout = next_arrival(&mut game, 60 * 30).expect("the last beat's scout rolled in past the dummy's wreck");
        game.debug_kill(scout).expect("the scout can be killed");
        step(&mut game, 2);
        assert_eq!(game.outcome(), Outcome::Won, "the course is done");
    }

    /// A couch of two plays the course alone, and has its second seat
    /// back on the next map without a script.
    #[test]
    fn a_training_round_seats_one_and_the_couch_gets_its_second_back_after() {
        let mut game = Game::default();
        game.seed_override = Some(5);
        game.players = crate::simulation::PlayerCount::TWO;
        game.map = MapFile::from_toml_str(include_str!("../../maps/boot-camp.toml")).expect("boot-camp parses");
        let (w, h) = game.map.field_size();
        game.init(w, h);
        assert_eq!(game.players, crate::simulation::PlayerCount::ONE, "the course seats one");
        assert_eq!(game.players().iter().flatten().count(), 1);
        game.init(w, h);
        assert_eq!(game.players().iter().flatten().count(), 1, "and goes on seating one when it starts over");
        game.map = MapFile::from_toml_str(include_str!("../../maps/lotus-lagoon.toml")).expect("lotus-lagoon parses");
        let (w, h) = game.map.field_size();
        game.init(w, h);
        assert_eq!(game.players, crate::simulation::PlayerCount::TWO, "the couch's count is back");
        assert_eq!(game.players().iter().flatten().count(), 2);
    }

    #[test]
    fn boot_camp_reads_and_lints_with_no_error() {
        let map = MapFile::from_toml_str(include_str!("../../maps/boot-camp.toml")).expect("boot-camp parses");
        assert_eq!(map.training.as_ref().map(|t| t.beat.len()), Some(9));
        assert!(!map.hostable(), "a training map is never hosted");
        let setup = crate::maplint::LintSetup {
            seed: 0xB0B5,
            players: Default::default(),
            enemy_count_override: None,
            player_row_override: None,
            player2_row_override: None,
            level_overrides: Default::default(),
        };
        let (_, findings) = crate::maplint::lint_map(&map, &setup);
        let errors: Vec<String> =
            findings.iter().filter(|f| f.severity == crate::maplint::LintSeverity::Error).map(|f| f.to_string()).collect();
        assert!(errors.is_empty(), "boot-camp lints clean: {errors:#?}");
    }
}
