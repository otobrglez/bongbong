//! A training round's rules (docs/training-stage.md): the map's
//! `[[training.beat]]` script run one beat at a time. A beat starts what it
//! names - a shell at the frog, crates dropped from the air, enemy tanks
//! rolled in through the map's gates - and is done once every condition it
//! sets holds; its doors (`map::CellObject::Door`) then open and the next
//! beat begins. The last beat done wins the round.
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

use crate::ai::Role;
use crate::map::{CellObject, cell_to_world};
use crate::math::Vec2;
use crate::obstacle::{Material, Obstacle};
use crate::pickup::PickupKind;
use crate::shell::{Owner, Shell};
use crate::tank::{Tank, TankKind};
use crate::training::{Beat, Edge, RollInSpec, TrainingAi};
use crate::tuning::tuning;
use crate::{MAX_SEATS, OBSTACLE_GRID_SIZE, Position};

use super::{Event, Frame, Game, spawn_pickup_at, with_frog, with_frog_mut, with_tank, with_tank_mut};

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
        self.training = Some(Run { beats: script.beat.len(), flags, homes, ..Run::default() });
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
            }
        }
        self.take_flags(&mut run, f);
        self.bring_back_seats(&mut run, f);
        if self.bring_back_frog(&mut run, f) {
            self.restart_beat(&mut run);
        }
        if !run.finished() {
            let beat = &script.beat[run.beat];
            if !run.begun {
                self.begin_beat(&mut run, beat, f);
            }
            self.roll_in_due(&mut run, f);
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
        run.begun = false;
        run.rolls.clear();
    }

    /// The running beat's starts.
    fn begin_beat(&mut self, run: &mut Run, beat: &Beat, f: &mut Frame) {
        run.begun = true;
        run.since = self.time;
        run.wrecks_at = self.enemies_destroyed;
        run.collected.clear();
        run.rolls = beat.start.roll_in.clone();
        if let Some(edge) = beat.start.shoot_frog {
            self.shoot_frog(edge, f);
        }
        for drop in &beat.start.drop {
            let (c, r) = drop.at;
            spawn_pickup_at(&mut self.world, cell_to_world(c, r), drop.kind, Some(self.time));
        }
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

    /// Roll in every tank of the beat whose time has come, through a free
    /// lane; one with every lane busy waits for the next frame.
    fn roll_in_due(&mut self, run: &mut Run, f: &mut Frame) {
        let elapsed = self.time - run.since;
        let mut waiting = Vec::new();
        for spec in std::mem::take(&mut run.rolls) {
            if elapsed < spec.after {
                waiting.push(spec);
                continue;
            }
            let role = match spec.ai {
                Some(TrainingAi::Dummy) => Role::Hunter,
                None => Role::Player,
            };
            match self.training_roll_in(f, spec.tank.row(), role) {
                Some(entity) => run.tanks.push(entity),
                None => waiting.push(spec),
            }
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
        if !done.destroyed.is_empty() {
            let standing: Vec<(i32, i32)> = self
                .world
                .query::<&Obstacle>()
                .iter()
                .filter(|o| !o.destroyed)
                .map(|o| o.cell())
                .collect();
            if done.destroyed.iter().all(|cell| standing.contains(cell)) {
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
    fn boot_camp_reads_and_lints_with_no_error() {
        let map = MapFile::from_toml_str(include_str!("../../maps/boot-camp.toml")).expect("boot-camp parses");
        assert_eq!(map.training.as_ref().map(|t| t.beat.len()), Some(7));
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
