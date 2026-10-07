//! The volcano's and the lava's world half (docs/volcano.md); `volcano.rs`
//! is the cycle and a bomb in flight, `lava.rs` the layout and its heat.
//!
//! - `build_volcanoes` (at `init`): one `Volcano` per crater cell, its
//!   gullies cut toward the lava leaving it. No RNG.
//! - `volcano_phase` (live, after `tower_phase`): an erupting volcano
//!   throws the bombs whose launch falls in this frame - some at a spot
//!   round the crater, some at a seat in range, every one hashed from the
//!   eruption and the bomb's number, never rolled.
//! - `tick_lava_bombs` (both branches, beside `tick_launches`): a bomb that
//!   lands bursts through the drums' blast path as `Drum::Lava`.
//! - `lava_phase` (live, beside `tick_fires`): heat burns every hull on hot
//!   ground by how far the heat there stands over `heat_hurt_from`, and a
//!   hull in the lava keeps burning after it leaves (afterburn) - none of
//!   it through a heat shield.
//! - `eruption_show` (every frame, a replica's too): the shock ring, the
//!   flash and the rumble's tremor the cycle's clock says are due, each
//!   once per eruption.
//!
//! A map without a volcano and without lava runs none of this and draws no
//! RNG; with them nothing here draws RNG either - only the blasts' damage
//! rolls do, as a drum's do.

use hecs::Entity;

use super::props::{BlastShape, PendingBlast};
use super::{Ai, Event, Frame, Game, Spectacle};
use crate::ground::Depth;
use crate::obstacle::Drum;
use crate::shockwave::Shockwave;
use crate::tank::Tank;
use crate::tuning::tuning;
use crate::volcano::{LavaBomb, Stage, Volcano};
use crate::{MAX_DAMAGE, OBSTACLE_GRID_SIZE, Position};

impl Game {
    /// One `Volcano` per crater cell, in the map's cell order, each with
    /// the directions the lava leaves its cone in. No RNG.
    pub(super) fn build_volcanoes(&mut self) {
        let t = tuning();
        self.volcanoes = self
            .map
            .volcano_cells()
            .into_iter()
            .map(|cell| {
                let mut outlets: Vec<f32> = self
                    .lava
                    .cells()
                    .filter(|&(c, r)| {
                        (-1..=1).any(|dr| (-1..=1).any(|dc| crate::volcano::in_footprint(cell.0, cell.1, c + dc, r + dr)))
                    })
                    .map(|(c, r)| crate::trig::atan2((r - cell.1) as f32, (c - cell.0) as f32))
                    .collect();
                outlets.sort_by(f32::total_cmp);
                outlets.dedup_by(|a, b| (*a - *b).abs() < 0.3);
                Volcano::new(cell, outlets, &t)
            })
            .collect();
        self.eruptions_shown = vec![(-1, -1); self.volcanoes.len()];
    }

    /// The volcanoes on the field.
    pub fn volcanoes(&self) -> &[Volcano] {
        &self.volcanoes
    }

    /// The lava bombs in the air.
    pub fn lava_bombs(&self) -> &[LavaBomb] {
        &self.lava_bombs
    }

    /// The lava's layout: its depth, flow and heat (`lava::LavaLayout`).
    pub fn lava(&self) -> &crate::lava::LavaLayout {
        &self.lava
    }

    /// The heat at a world position, 0..1: the lava's, which radiates onto
    /// its banks.
    pub fn heat_at(&self, pos: Position) -> f32 {
        self.lava.heat_at(pos)
    }

    /// Throw this frame's lava bombs: every erupting volcano's bombs whose
    /// launch time falls in the frame (`volcano::bomb_launch`). A bomb is
    /// aimed at a seat - one on the field within range, picked by hash -
    /// at `volcano_bomb_aimed_share` odds by hash, else at a spot round the
    /// crater between the two ranges. An aimed bomb lands a hashed step
    /// off its seat, inside the seat's sight box, so it is always on that
    /// player's screen for its whole flight.
    pub(super) fn volcano_phase(&mut self, f: &mut Frame) {
        if self.volcanoes.is_empty() {
            return;
        }
        let t = tuning();
        let count = t.volcano_bombs_per_eruption.max(0);
        if count == 0 {
            return;
        }
        let (now, before) = (self.time, self.time - f.dt);
        let seats: Vec<Position> = self
            .seats_on_field()
            .into_iter()
            .flatten()
            .filter_map(|e| self.world.get::<&Tank>(e).ok().filter(|t| !t.is_wreck()).map(|t| t.position))
            .collect();
        let mut thrown = Vec::new();
        let mut surged = Vec::new();
        for v in &self.volcanoes {
            let phase = v.phase(now, &t);
            if phase.stage != Stage::Erupt {
                continue;
            }
            let n = phase.eruption;
            let start = crate::volcano::eruption_start(n, v.offset, &t);
            if start > before && start <= now {
                surged.push(n);
            }
            let centre = v.centre();
            for k in 0..count {
                let at = crate::volcano::bomb_launch(start, k, count, &t);
                if !(at > before && at <= now) {
                    continue;
                }
                let salt = |s: i32| crate::lava::hash3(n as i32 * 97 + k, v.cell.0 * 31 + v.cell.1, s);
                let in_range: Vec<Position> =
                    seats.iter().copied().filter(|p| p.distance_to(centre) <= t.volcano_bomb_range_px + OBSTACLE_GRID_SIZE).collect();
                let to = if !in_range.is_empty() && salt(1) < t.volcano_bomb_aimed_share {
                    let seat = in_range[((salt(2) * in_range.len() as f32) as usize).min(in_range.len() - 1)];
                    let angle = salt(3) * std::f32::consts::TAU;
                    let off = OBSTACLE_GRID_SIZE * 1.5 * salt(4);
                    Position::new(seat.x + angle.cos() * off, seat.y + angle.sin() * off)
                } else {
                    let angle = salt(5) * std::f32::consts::TAU;
                    let (lo, hi) = (t.volcano_bomb_min_range_px, t.volcano_bomb_range_px.max(t.volcano_bomb_min_range_px));
                    let dist = lo + (hi - lo) * salt(6).sqrt();
                    Position::new(centre.x + angle.cos() * dist, centre.y + angle.sin() * dist)
                };
                let inset = OBSTACLE_GRID_SIZE;
                let to = Position::new(to.x.clamp(inset, f.width - inset), to.y.clamp(inset, f.height - inset));
                // Nothing lands on the cone: a bomb short of its foot
                // carries on out.
                let to = if to.distance_to(centre) < t.volcano_bomb_min_range_px {
                    let d = (to - centre).length().max(1.0);
                    let scale = t.volcano_bomb_min_range_px / d;
                    Position::new(centre.x + (to.x - centre.x) * scale, centre.y + (to.y - centre.y) * scale)
                } else {
                    to
                };
                thrown.push((centre, to));
            }
        }
        for (from, to) in thrown {
            f.events.push(Event::LavaBombLaunched { x: from.x, y: from.y, to_x: to.x, to_y: to.y });
            self.bomb_in_flight(from, to);
        }
        // The surge an eruption sends down the rivers sets what grows on
        // their banks alight: a flammable tile on ground at least a bank's
        // heat catches at even odds, hashed by its cell and the eruption -
        // a range board too, which catches from any fire.
        for n in surged {
            let bank = t.lava_heat_falloff;
            for o in self.world.query::<&mut crate::obstacle::Obstacle>().iter() {
                if o.destroyed || o.burning || !(o.flammable || o.material.catches_fire()) {
                    continue;
                }
                let (c, r) = crate::map::world_to_cell(o.position);
                if self.lava.heat(c, r) >= bank && crate::lava::hash3(c * 131 + r, n as i32, 17) < 0.5 {
                    o.health = 0.0;
                    o.burning = true;
                    f.events.push(Event::Ignited { x: o.position.x, y: o.position.y, what: if o.material.is_tree() { "tree" } else if o.material.catches_fire() { "target" } else { "wood" } });
                }
            }
        }
    }

    /// Put a bomb in the air. Both ends stage it: the round from
    /// `volcano_phase`, a replica from the `LavaBombLaunched` event.
    pub(crate) fn bomb_in_flight(&mut self, from: Position, to: Position) {
        self.lava_bombs.push(LavaBomb::new(from, to));
    }

    /// Age every bomb in the air and hand back the ones that have landed,
    /// already out of the list: the round bursts them, a replica waits for
    /// the room's `Blast`.
    pub(crate) fn age_lava_bombs(&mut self, dt: f32) -> Vec<LavaBomb> {
        let t = tuning();
        let mut landed = Vec::new();
        for bomb in self.lava_bombs.iter_mut() {
            bomb.age += dt;
            if bomb.landed(&t) {
                landed.push(*bomb);
            }
        }
        self.lava_bombs.retain(|b| !b.landed(&t));
        landed
    }

    /// A bomb that lands bursts where it was aimed: a lava drum's blast,
    /// resolved by `explosions` with the rest of the frame's blasts.
    pub(super) fn tick_lava_bombs(&mut self, f: &mut Frame) {
        if self.lava_bombs.is_empty() {
            return;
        }
        for bomb in self.age_lava_bombs(f.dt) {
            f.events.push(Event::Blast { x: bomb.to.x, y: bomb.to.y, chained: false, drum: Drum::Lava });
            f.pending_blasts.push(PendingBlast { center: bomb.to, drum: Drum::Lava, shape: BlastShape::Plain });
        }
    }

    /// Heat burns: every hull on ground hotter than `heat_hurt_from` takes
    /// `lava_damage_per_second` scaled by how far over it the heat stands
    /// - the full rate in the lava itself, a little on a bank - and a hull
    /// in the lava keeps burning `lava_afterburn_seconds` after it leaves
    /// (the flamethrower's afterburn, which `resolve_flames` charges). A
    /// heat shield keeps all of it off. Seats first, then the enemies, like
    /// every damage walk; no RNG.
    pub(super) fn lava_phase(&mut self, f: &mut Frame) {
        if self.lava.is_empty() {
            return;
        }
        let t = tuning();
        let from = t.heat_hurt_from.clamp(0.0, 0.99);
        let mut order: Vec<Entity> = self.seats_on_field().into_iter().flatten().collect();
        order.extend(self.world.query::<(Entity, &Tank)>().with::<&Ai>().iter().map(|(e, _)| e));
        for entity in order {
            let mut q = self.world.query_one::<&mut Tank>(entity);
            let Ok(tank) = q.get() else { continue };
            if tank.is_wreck() {
                continue;
            }
            if !tank.takes_heat() {
                continue;
            }
            let heat = self.lava.heat_at(tank.position);
            if heat <= from {
                continue;
            }
            let k = (heat - from) / (1.0 - from);
            tank.take_damage(t.lava_damage_per_second * k * f.dt, MAX_DAMAGE);
            tank.mark_hit();
            if self.lava.depth_at(tank.position) != Depth::Dry {
                tank.burn_timer = tank.burn_timer.max(t.lava_afterburn_seconds);
            }
            if tank.is_wreck() {
                f.kills.push((tank.position, tank.owner()));
            }
        }
    }

    /// How the lava looks now: the strongest surge any volcano sends down
    /// the rivers, and the bands' speed with it.
    pub fn lava_look(&self) -> crate::lava::Look {
        let t = tuning();
        let surge = self.volcanoes.iter().map(|v| v.phase(self.time, &t).surge).fold(0.0, f32::max);
        crate::lava::Look { surge, speed: t.lava_flow_speed * (1.0 + surge) }
    }

    /// Whether the seats get lanterns this round: its sky is dark - night,
    /// a storm, fog or dusk - or night falls in it (`map::MapFile::
    /// nightfall`).
    pub fn lamps_in_play(&self) -> bool {
        use crate::map::Weather;
        self.map.nightfall.is_some() || matches!(self.weather, Weather::Night | Weather::Storm | Weather::Fog | Weather::Dusk)
    }

    /// The lamp posts still standing.
    pub fn lamp_posts(&self) -> Vec<Position> {
        self.world
            .query::<&crate::obstacle::Obstacle>()
            .iter()
            .filter(|o| o.material == crate::obstacle::Material::Lamp && !o.destroyed)
            .map(|o| o.position)
            .collect()
    }

    /// The lanterns set down this round.
    pub fn lanterns(&self) -> &[crate::lamp::Lantern] {
        &self.lanterns
    }

    /// Lanterns seat `seat` has left to set down.
    pub fn lamps_left(&self, seat: usize) -> u8 {
        self.lamps_left.get(seat).copied().unwrap_or(0)
    }

    /// Night falls on a map with `nightfall` once the round clock reaches
    /// it: the round is fought under night's rules from then on - its
    /// sight, its lights - though the map keeps its own key, so the next
    /// round opens under it again. A pure function of the clock, so a
    /// replica's night falls on the room's tick. No RNG.
    pub(crate) fn tick_nightfall(&mut self) {
        if self.night_fallen {
            return;
        }
        let Some(at) = self.map.nightfall else { return };
        if self.time < at {
            return;
        }
        self.night_fallen = true;
        let t = tuning();
        self.weather = crate::weather::in_force(crate::map::Weather::Night.into(), self.round_seed(), self.weather_from_map, &t);
    }

    /// Whether the map's `nightfall` has passed this round, and the sky in
    /// force is night's.
    pub fn night_has_fallen(&self) -> bool {
        self.night_fallen
    }

    /// How far night has fallen, 0 (the map's sky) to 1 (night), over the
    /// `nightfall_seconds` before the map's `nightfall`: what the renderer
    /// eases the light by. 0 on a map without one.
    pub fn nightfall_mix(&self) -> f32 {
        let Some(at) = self.map.nightfall else { return 0.0 };
        if self.night_fallen {
            return 1.0;
        }
        let span = tuning().nightfall_seconds.max(1e-3);
        ((self.time - (at - span)) / span).clamp(0.0, 1.0)
    }

    /// The eruption's show, from the cycle's clock: the tremor a rumble
    /// opens with, then the shock ring and the flash an eruption opens
    /// with, each once per eruption (`eruptions_shown`). Every caller's
    /// round - a local one, a room's, a replica's - stages it, so the ring
    /// lands on the room's tick wherever it is drawn. No RNG.
    pub(crate) fn eruption_show(&mut self) -> Spectacle {
        let mut show = Spectacle::default();
        if self.volcanoes.is_empty() {
            return show;
        }
        let t = tuning();
        let mut flash = false;
        for (i, v) in self.volcanoes.iter().enumerate() {
            let phase = v.phase(self.time, &t);
            let (rumbled, erupted) = self.eruptions_shown[i];
            match phase.stage {
                Stage::Rumble if phase.eruption > rumbled => {
                    self.eruptions_shown[i].0 = phase.eruption;
                    show.shocks.push(Shockwave::scaled(v.centre(), 0.35 * t.volcano_shock_scale));
                }
                Stage::Erupt if phase.eruption > erupted => {
                    self.eruptions_shown[i] = (phase.eruption, phase.eruption);
                    show.shocks.push(Shockwave::scaled(v.centre(), t.volcano_shock_scale));
                    show.impact_flashes.push(Shockwave::new(v.centre()));
                    flash = true;
                }
                _ => {}
            }
        }
        if flash {
            self.flash_screen();
        }
        show
    }
}

#[cfg(test)]
mod tests {
    use crate::map::{MapFile, cell_to_world};
    use crate::simulation::{Event, Game, Input};
    use crate::tuning::Tuning;
    use crate::volcano::Stage;

    const W: f32 = 1280.0;
    const H: f32 = 720.0;

    fn game_on(extra: &str) -> Game {
        let toml = format!("version = 1\ntanks = 0\ncells.\"5,5\" = {{ kind = \"start\" }}\n{extra}");
        let mut game = Game::default();
        game.seed_override = Some(7);
        game.level_overrides.mission = Some(crate::level::Mission::Destroy);
        game.map = MapFile::from_toml_str(&toml).expect("map parses");
        game.init(W, H);
        game
    }

    fn step(game: &mut Game, frames: usize) {
        for _ in 0..frames {
            game.update(Input::default(), 1.0 / 60.0, W, H);
        }
    }

    #[test]
    fn a_volcano_throws_its_bombs_when_it_erupts_and_they_burst() {
        let mut game = game_on("cells.\"20,11\" = { kind = \"volcano\" }\n");
        assert_eq!(game.volcanoes().len(), 1);
        let t = Tuning::DEFAULT;
        let v = game.volcanoes()[0].clone();
        let start = crate::volcano::eruption_start(0, v.offset, &t);
        let mut launched = 0;
        let mut burst = 0;
        let frames = ((start + t.volcano_erupt_seconds + t.volcano_bomb_flight_seconds + 0.5) * 60.0) as usize;
        for _ in 0..frames {
            step(&mut game, 1);
            for e in game.events() {
                match e {
                    Event::LavaBombLaunched { .. } => launched += 1,
                    Event::Blast { drum: crate::obstacle::Drum::Lava, .. } => burst += 1,
                    _ => {}
                }
            }
        }
        assert_eq!(launched, t.volcano_bombs_per_eruption);
        assert_eq!(burst, t.volcano_bombs_per_eruption);
        assert_eq!(v.phase(game.time, &t).stage, Stage::Cool);
    }

    fn lamp_press(game: &mut Game, held: bool) {
        let mut input = Input::default();
        input.seats[0].lamp = held;
        game.update(input, 1.0 / 60.0, W, H);
    }

    #[test]
    fn a_seat_sets_a_lantern_down_on_each_press_while_it_has_any() {
        let mut game = game_on("weather = \"night\"\n");
        let t = Tuning::DEFAULT;
        assert!(game.lamps_in_play());
        assert_eq!(game.lamps_left(0) as i32, t.lamps_per_seat);
        lamp_press(&mut game, true);
        assert_eq!(game.lanterns().len(), 1);
        assert!(game.events().iter().any(|e| matches!(e, Event::LanternSet { seat: 0, .. })));
        // Held, it sets no second one.
        step(&mut game, 0);
        lamp_press(&mut game, true);
        lamp_press(&mut game, true);
        assert_eq!(game.lanterns().len(), 1);
        for _ in 0..t.lamps_per_seat + 2 {
            lamp_press(&mut game, false);
            lamp_press(&mut game, true);
        }
        assert_eq!(game.lanterns().len() as i32, t.lamps_per_seat, "no more than it was given");
        assert_eq!(game.lamps_left(0), 0);
        let ids: Vec<u16> = game.lanterns().iter().map(|l| l.id).collect();
        assert_eq!(ids, (0..t.lamps_per_seat as u16).collect::<Vec<_>>());
    }

    #[test]
    fn a_day_round_gives_no_lanterns() {
        let game = game_on("");
        assert!(!game.lamps_in_play());
        assert_eq!(game.lamps_left(0), 0);
    }

    #[test]
    fn light_shows_a_seat_to_the_enemy_at_full_range_at_night() {
        let t = Tuning::DEFAULT;
        let game = game_on("weather = \"night\"\ncells.\"30,11\" = { kind = \"lamp\" }\n");
        let dark = game.enemy_sight();
        assert!(dark < t.enemy_view_range);
        assert_eq!(game.sight_on(cell_to_world(30, 13)), t.enemy_view_range, "beside the lamp");
        assert_eq!(game.sight_on(cell_to_world(10, 13)), dark, "far from it");
        // Under a clear sky the lamp changes nothing.
        let day = game_on("cells.\"30,11\" = { kind = \"lamp\" }\n");
        assert_eq!(day.sight_on(cell_to_world(30, 13)), day.enemy_sight());
    }

    #[test]
    fn night_falls_on_the_round_clock() {
        let mut game = game_on("weather = \"dusk\"\nnightfall = 1.0\n");
        assert!(game.lamps_in_play());
        let dusk = game.enemy_sight();
        assert_eq!(game.weather(), crate::map::Weather::Dusk);
        assert!(game.nightfall_mix() > 0.0, "the dark is already falling");
        step(&mut game, 70);
        assert_eq!(game.weather(), crate::map::Weather::Night);
        assert!(game.enemy_sight() < dusk);
        assert_eq!(game.nightfall_mix(), 1.0);
        assert_eq!(game.map.weather, crate::map::Weather::Dusk.into(), "the map keeps its own sky");
    }

    #[test]
    fn the_cone_is_solid_round_its_crater() {
        let game = game_on("cells.\"20,11\" = { kind = \"volcano\" }\n");
        let cone = game.world.query::<&crate::obstacle::Obstacle>().iter().filter(|o| o.material == crate::obstacle::Material::Volcano).count();
        assert_eq!(cone, 21);
        let grid = game.nav_grid(W, H);
        assert!(!grid.usable(cell_to_world(20, 11)));
    }
}
