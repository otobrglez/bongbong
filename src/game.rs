use rand::RngExt;
use sola_raylib::prelude::*;

use crate::ai::{Ai, Intent, Mover};
use crate::damage_stage::draw_damage;
use crate::shell::{Shell, ShellState, draw_shell};
use crate::tank::{Dir, Tank, draw_tank};
use crate::track::{Track, draw_track};
use crate::{
    ENEMY_COUNT, ENEMY_DAMAGE_MAX, ENEMY_DAMAGE_MIN, ENEMY_SPEED, ENEMY_SPEED_VARIANCE, MAX_DAMAGE,
    PLAYER_DAMAGE_MAX, PLAYER_DAMAGE_MIN, Position, RAM_DAMAGE_COOLDOWN, RESTART_DELAY,
    TANK_TEXTURE_SIZE, TRACK_SCALE_FRACTION, TRACK_SPACING,
};

/// How the current round is going.
#[derive(Clone, Copy, PartialEq)]
pub enum Outcome {
    Playing,
    Won,  // all enemies destroyed
    Lost, // player destroyed
}

impl Default for Outcome {
    fn default() -> Self {
        Outcome::Playing
    }
}

#[derive(Default)]
pub struct Game {
    tank: Tank,
    enemies: Vec<Tank>,
    /// One brain per enemy, indexed in lockstep with `enemies`.
    ais: Vec<Ai>,
    shells: Vec<Shell>,
    /// Fading tread marks left behind as tanks drive, oldest first.
    tracks: Vec<Track>,
    /// Seconds elapsed since the game started; drives damage-overlay animation.
    time: f32,
    /// Result of the current round.
    outcome: Outcome,
    /// Seconds counting down after the round ends; at zero the game restarts.
    restart_timer: f32,
}

/// The tank hulls live in row 0 of tanks.png, indexed 0..8 left-to-right by
/// column. Barrel count is fixed per index: 1, 2, 3, 4 are twin-barrel, the rest
/// (0, 5, 6, 7) are single-barrel.
const TANK_ROW: i32 = 0;
const TANK_COUNT: i32 = 8;

/// A spawn order over all 8 hulls that alternates twin- and single-barrel tanks,
/// so however many tanks are on the field, both kinds appear (and appear early).
/// Interleaves twins (1,2,3,4) with singles (0,5,6,7).
const TANK_SPRITE_ORDER: [i32; 8] = [1, 0, 2, 5, 3, 6, 4, 7];

impl Game {
    /// Set up the player and spawn the enemy tanks. Also used to restart a round.
    pub fn init(&mut self, rl: &RaylibHandle) {
        let (width, height) = (rl.get_screen_width() as f32, rl.get_screen_height() as f32);

        let mut rng = rand::rng();

        // Fresh round state.
        self.tank = Tank::default();
        self.shells.clear();
        self.tracks.clear();
        self.time = 0.0;
        self.outcome = Outcome::Playing;
        self.restart_timer = 0.0;

        // Pick a random hull (any of the 8; a mix of single/twin barrel) and center it.
        self.tank.row = TANK_ROW;
        self.tank.col = rng.random_range(0..TANK_COUNT);
        self.tank.position = Position::new(width / 2.0, height / 2.0);

        // Spawn enemy tanks at random positions, kept off the edges and away from
        // the player's starting spot in the middle.
        let margin = TANK_TEXTURE_SIZE; // keep whole sprite on screen
        let center = self.tank.position;
        let clear = self.tank.size() * 2.0; // don't spawn on top of the player

        self.enemies.clear();
        self.ais.clear();
        while self.enemies.len() < ENEMY_COUNT {
            let pos = Position::new(
                rng.random_range(margin..(width - margin)),
                rng.random_range(margin..(height - margin)),
            );
            if pos.distance_to(center) < clear {
                continue;
            }
            // Walk the alternating spawn order so each enemy looks distinct and the
            // group mixes single- and twin-barrel hulls.
            let ecol = TANK_SPRITE_ORDER[self.enemies.len() % TANK_SPRITE_ORDER.len()];
            // Vary speed within +/- ENEMY_SPEED_VARIANCE so enemies don't all move
            // in lockstep; each keeps this speed for the round.
            let factor = 1.0 + rng.random_range(-ENEMY_SPEED_VARIANCE..ENEMY_SPEED_VARIANCE);
            self.enemies.push(Tank {
                row: TANK_ROW,
                col: ecol,
                position: pos,
                rotation: 180.0,             // facing down, toward the player's start
                speed: ENEMY_SPEED * factor, // enemies drive slower than the player
                ..Tank::default()
            });
            self.ais.push(Ai::default());
        }
    }

    /// Step the simulation one frame.
    pub fn update(&mut self, rl: &RaylibHandle) {
        let dt = rl.get_frame_time();

        // Round is over: count down and restart, but keep animating the scene
        // (burning wrecks etc.) so the end screen stays lively.
        if self.outcome != Outcome::Playing {
            self.time += dt;
            for enemy in &mut self.enemies {
                enemy.tick_wreck(dt);
            }
            self.tank.tick_wreck(dt);
            self.tracks.retain_mut(|t| !t.tick(dt));
            self.restart_timer -= dt;
            if self.restart_timer <= 0.0 {
                self.init(rl);
            }
            return;
        }

        // Advance the global animation clock and per-tank timers.
        self.time += dt;
        self.tank.tick_recharge(dt);
        self.tank.ram_cooldown = (self.tank.ram_cooldown - dt).max(0.0);
        self.tank.tick_wreck(dt);
        for enemy in &mut self.enemies {
            enemy.tick_recharge(dt);
            enemy.ram_cooldown = (enemy.ram_cooldown - dt).max(0.0);
            enemy.tick_wreck(dt);
        }
        // Age existing marks and drop the ones that have fully faded.
        self.tracks.retain_mut(|t| !t.tick(dt));

        let mut rng = rand::rng();
        let (width, height) = (rl.get_screen_width() as f32, rl.get_screen_height() as f32);

        // --- Player: build an intent from the keyboard ---
        // Classic 4-direction driving: an arrow key faces the tank that way and
        // moves it at constant speed; releasing stops instantly. If several are
        // held the last one checked wins. A wreck can't move but may still fire.
        let mut player_intent = Intent::default();
        if !self.tank.is_wreck() {
            if rl.is_key_down(KeyboardKey::KEY_UP) {
                player_intent.move_dir = Some(Dir::Up);
            } else if rl.is_key_down(KeyboardKey::KEY_DOWN) {
                player_intent.move_dir = Some(Dir::Down);
            } else if rl.is_key_down(KeyboardKey::KEY_LEFT) {
                player_intent.move_dir = Some(Dir::Left);
            } else if rl.is_key_down(KeyboardKey::KEY_RIGHT) {
                player_intent.move_dir = Some(Dir::Right);
            }
        }
        player_intent.fire = rl.is_key_pressed(KeyboardKey::KEY_SPACE);

        // Drive the player and resolve movement against every enemy footprint.
        self.apply_movement_player(player_intent, dt, width, height, &mut rng);
        if player_intent.fire && self.tank.shells_ammo >= 1 {
            self.tank.shells_ammo -= 1;
            // The player always fires straight down the barrel.
            self.shells.push(Shell::spawn(&self.tank, true, 0.0));
        }

        // --- Enemies: each brain decides an intent, then drives + maybe fires ---
        // Snapshot every live tank's motion for predictive collision avoidance:
        // slot 0 is the player, slots 1.. are the enemies in order. Rebuilt each
        // frame so positions/velocities are current; lets an enemy read the others
        // without borrowing the mutable enemy list mid-loop.
        let movers = self.motion_snapshot();
        for i in 0..self.enemies.len() {
            let intent =
                self.ais[i].think(&self.enemies[i], &self.tank, width, height, dt, &movers, i + 1, &mut rng);
            self.apply_movement_enemy(i, intent, dt, width, height, &mut rng);
            if intent.fire && self.enemies[i].shells_ammo >= 1 {
                self.enemies[i].shells_ammo -= 1;
                // Point-blank shots may be thrown off-aim (see roll_misfire).
                let shell = Shell::spawn(&self.enemies[i], false, intent.fire_aim_offset);
                self.shells.push(shell);
            }
        }

        // --- Shells: move, then hit the opposing side ---
        for shell in &mut self.shells {
            shell.update(dt, width, height);
            if shell.state != ShellState::Flying {
                continue;
            }
            if shell.from_player {
                // Player fire damages enemies.
                for enemy in &mut self.enemies {
                    if enemy.contains(shell.position) {
                        if !enemy.is_wreck() {
                            let dmg = rng.random_range(PLAYER_DAMAGE_MIN..PLAYER_DAMAGE_MAX);
                            enemy.damage = (enemy.damage + dmg).min(MAX_DAMAGE);
                        }
                        shell.detonate();
                        break; // one shell hits at most one tank
                    }
                }
            } else if self.tank.contains(shell.position) {
                // Enemy fire damages the player (weaker than player shells).
                if !self.tank.is_wreck() {
                    let dmg = rng.random_range(ENEMY_DAMAGE_MIN..ENEMY_DAMAGE_MAX);
                    self.tank.damage = (self.tank.damage + dmg).min(MAX_DAMAGE);
                }
                shell.detonate();
            }
        }
        // Drop shells that have finished their bang animation.
        self.shells.retain(|s| !s.done);

        // Check for a round end. Losing (player destroyed) takes precedence over
        // winning in case the last enemy and the player die on the same frame.
        if self.tank.is_wreck() {
            self.end_round(Outcome::Lost);
        } else if self.enemies.iter().all(|e| e.is_wreck()) {
            self.end_round(Outcome::Won);
        }
    }

    /// Enter the end-of-round state and start the restart countdown.
    fn end_round(&mut self, outcome: Outcome) {
        self.outcome = outcome;
        self.restart_timer = RESTART_DELAY;
    }

    /// Snapshot every live tank's motion for the AI's collision avoidance: slot 0
    /// is the player, slots 1.. are the enemies in order. Wrecks are included at
    /// zero velocity so tanks still steer around burning hulks.
    fn motion_snapshot(&self) -> Vec<Mover> {
        let to_mover = |t: &Tank| Mover {
            position: t.position,
            // A wreck can't move; treat it as stationary regardless of the velocity
            // it carried into death so tanks steer around it as a fixed obstacle.
            velocity: if t.is_wreck() {
                Position::new(0.0, 0.0)
            } else {
                t.velocity
            },
            radius: t.hull_size() * 0.5,
        };
        let mut movers = Vec::with_capacity(self.enemies.len() + 1);
        movers.push(to_mover(&self.tank));
        movers.extend(self.enemies.iter().map(to_mover));
        movers
    }

    /// Drive the player one frame, reverting into any enemy it would overlap and
    /// applying one-off ramming damage (debounced by each tank's cooldown).
    fn apply_movement_player(
        &mut self,
        intent: Intent,
        dt: f32,
        width: f32,
        height: f32,
        rng: &mut rand::rngs::ThreadRng,
    ) {
        let before = self.tank.position;
        self.tank.control(intent.move_dir, intent.face, dt);
        // Keep the tank on the battlefield before resolving tank collisions.
        self.tank.clamp_to_field(width, height);
        for enemy in &mut self.enemies {
            if self.tank.overlaps(enemy) {
                self.tank.position = before;
                ram(&mut self.tank, enemy, rng);
                break;
            }
        }
        lay_tracks(&mut self.tracks, &mut self.tank, before);
    }

    /// Drive enemy `i` one frame, blocking against the player and the other
    /// enemies so tanks never pass through each other, with ramming damage.
    fn apply_movement_enemy(
        &mut self,
        i: usize,
        intent: Intent,
        dt: f32,
        width: f32,
        height: f32,
        rng: &mut rand::rngs::ThreadRng,
    ) {
        let before = self.enemies[i].position;
        self.enemies[i].control(intent.move_dir, intent.face, dt);
        // Keep the enemy on the battlefield before resolving tank collisions.
        self.enemies[i].clamp_to_field(width, height);

        // Block against the player.
        if self.enemies[i].overlaps(&self.tank) {
            self.enemies[i].position = before;
            ram(&mut self.enemies[i], &mut self.tank, rng);
            return;
        }
        // Block against the other enemies (split_at_mut to borrow two entries).
        let (left, right) = self.enemies.split_at_mut(i);
        let (me, rest) = right.split_first_mut().unwrap();
        for other in left.iter_mut().chain(rest.iter_mut()) {
            if me.overlaps(other) {
                me.position = before;
                ram(me, other, rng);
                break;
            }
        }
        lay_tracks(&mut self.tracks, &mut self.enemies[i], before);
    }

    /// Draw the whole scene for this frame.
    pub fn render(
        &self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        tanks_texture: &Texture2D,
        shells_texture: &Texture2D,
        damage_texture: &Texture2D,
        tracks_texture: &Texture2D,
    ) {
        let screen_width = rl.get_screen_width();
        let screen_height = rl.get_screen_height();

        // Precompute the centered end-of-round banner (text width must be
        // measured on the RaylibHandle, outside the draw closure).
        let banner = match self.outcome {
            Outcome::Playing => None,
            Outcome::Won => Some(("YOU WIN", Color::DARKGREEN)),
            Outcome::Lost => Some(("YOU LOSE", Color::MAROON)),
        };
        let banner = banner.map(|(text, color)| {
            let title_size = 72;
            let title_w = rl.measure_text(text, title_size);
            let sub = format!("Restarting in {}...", self.restart_timer.ceil().max(0.0) as i32);
            let sub_size = 28;
            let sub_w = rl.measure_text(&sub, sub_size);
            (text, color, title_size, title_w, sub, sub_size, sub_w)
        });

        rl.draw(thread, |mut d| {
            d.clear_background(Color::RAYWHITE);

            // Tread marks go down first so tanks and everything else draw on top.
            for track in &self.tracks {
                draw_track(&mut d, tracks_texture, track);
            }

            for enemy in &self.enemies {
                draw_tank(&mut d, tanks_texture, enemy);
                draw_damage(&mut d, damage_texture, enemy, self.time);
            }

            draw_tank(&mut d, tanks_texture, &self.tank);
            draw_damage(&mut d, damage_texture, &self.tank, self.time);

            for shell in &self.shells {
                draw_shell(&mut d, shells_texture, shell);
            }

            let hp = (MAX_DAMAGE - self.tank.damage).max(0.0).round() as i32;
            let hud = format!("SHELLS: {}   HP: {}", self.tank.shells_ammo, hp);
            d.draw_text(&hud, 50, screen_height - 50, 24, Color::DARKGRAY);

            // End-of-round banner over a dimming overlay.
            if let Some((title, color, title_size, title_w, sub, sub_size, sub_w)) = &banner {
                d.draw_rectangle(
                    0,
                    0,
                    screen_width,
                    screen_height,
                    Color::new(0, 0, 0, 120),
                );
                let cx = screen_width / 2;
                let cy = screen_height / 2;
                d.draw_text(title, cx - title_w / 2, cy - title_size, *title_size, *color);
                d.draw_text(sub, cx - sub_w / 2, cy + 20, *sub_size, Color::RAYWHITE);
            }
        });
    }
}

/// Lay fresh tread marks along the distance a tank travelled this frame, dropping
/// one mark every TRACK_SPACING pixels. `before` is where the tank was at the
/// start of the frame; if it didn't actually move (blocked/idle) nothing is laid.
fn lay_tracks(tracks: &mut Vec<Track>, tank: &mut Tank, before: Position) {
    let moved = tank.position.distance_to(before);
    if moved <= 0.0 {
        return;
    }
    // Unit vector pointing back along the segment the tank just travelled, used
    // to place marks evenly along the path rather than stacking them at the end.
    let back = Vector2::new(
        (before.x - tank.position.x) / moved,
        (before.y - tank.position.y) / moved,
    );

    // Push marks out to the tank's rear edge so the trail comes out from behind
    // the hull and never pokes ahead of it.
    let rear = tank.hull_size() * 0.5;
    let scale = tank.scale * TRACK_SCALE_FRACTION;

    tank.track_accum += moved;
    // Step up the path in TRACK_SPACING increments so the spacing stays even
    // regardless of speed or frame rate. `dist_back` is how far behind the current
    // position each mark sits.
    while tank.track_accum >= TRACK_SPACING {
        tank.track_accum -= TRACK_SPACING;
        let dist_back = rear + tank.track_accum;
        tracks.push(Track {
            position: Position::new(
                tank.position.x + back.x * dist_back,
                tank.position.y + back.y * dist_back,
            ),
            rotation: tank.rotation,
            scale,
            age: 0.0,
        });
    }
}

/// Apply one-off ramming damage to two touching tanks, debounced by each tank's
/// collision cooldown so continuous contact doesn't drain damage every frame.
fn ram(a: &mut Tank, b: &mut Tank, rng: &mut rand::rngs::ThreadRng) {
    if a.ram_cooldown <= 0.0 && b.ram_cooldown <= 0.0 {
        let dmg = rng.random_range(2.0..6.0);
        a.damage = (a.damage + dmg).min(MAX_DAMAGE);
        b.damage = (b.damage + dmg).min(MAX_DAMAGE);
        a.ram_cooldown = RAM_DAMAGE_COOLDOWN;
        b.ram_cooldown = RAM_DAMAGE_COOLDOWN;
    }
}
