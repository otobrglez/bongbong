use rand::RngExt;
use sola_raylib::prelude::*;

use crate::damage_stage::draw_damage;
use crate::shell::{Shell, ShellState, draw_shell};
use crate::tank::{Tank, draw_tank};
use crate::{
    ENEMY_COUNT, MAX_DAMAGE, Position, RAM_DAMAGE_COOLDOWN, TANK_MAX_SPEED, TANK_TEXTURE_SIZE,
    TANK_TURN_SPEED,
};

#[derive(Default)]
pub struct Game {
    tank: Tank,
    enemies: Vec<Tank>,
    shells: Vec<Shell>,
    /// Seconds elapsed since the game started; drives damage-overlay animation.
    time: f32,
}

impl Game {
    /// Set up the player and spawn the enemy tanks.
    pub fn init(&mut self, rl: &RaylibHandle) {
        let (width, height) = (rl.get_screen_width() as f32, rl.get_screen_height() as f32);

        let mut rng = rand::rng();

        // Pick a random tank sprite from the 8x8 atlas and center it on screen.
        self.tank.row = rng.random_range(0..8);
        self.tank.col = rng.random_range(0..8);
        self.tank.position = Position::new(width / 2.0, height / 2.0);

        // Spawn enemy tanks at random positions, kept off the edges and away from
        // the player's starting spot in the middle.
        let margin = TANK_TEXTURE_SIZE; // keep whole sprite on screen
        let center = self.tank.position;
        let clear = self.tank.size() * 2.0; // don't spawn on top of the player

        self.enemies.clear();
        while self.enemies.len() < ENEMY_COUNT {
            let pos = Position::new(
                rng.random_range(margin..(width - margin)),
                rng.random_range(margin..(height - margin)),
            );
            if pos.distance_to(center) < clear {
                continue;
            }
            self.enemies.push(Tank {
                // Use a few different single-turret sprites so enemies look distinct.
                row: 2,
                col: (self.enemies.len() as i32 * 2) % 8,
                position: pos,
                rotation: 180.0, // facing down, toward the player's start
                ..Tank::default()
            });
        }
    }

    /// Step the simulation one frame.
    pub fn update(&mut self, rl: &RaylibHandle) {
        let dt = rl.get_frame_time();

        // Advance the global animation clock and per-tank timers.
        self.time += dt;
        self.tank.tick_recharge(dt);
        self.tank.ram_cooldown = (self.tank.ram_cooldown - dt).max(0.0);
        self.tank.tick_wreck(dt);
        for enemy in &mut self.enemies {
            enemy.ram_cooldown = (enemy.ram_cooldown - dt).max(0.0);
            enemy.tick_wreck(dt);
        }

        let mut rng = rand::rng();

        // Real-tank driving: Left/Right rotate the hull (it can pivot in place),
        // Up/Down are throttle forward/reverse, and with no throttle the tank
        // coasts to a stop. Velocity carries momentum along the hull heading.
        // A wrecked tank is a stationary burning hulk: no steering, no throttle,
        // and it sheds any leftover momentum (but can still fire).
        if self.tank.is_wreck() {
            self.tank.drive(0.0, dt);
        } else {
            // Steer. Turning is a touch quicker while nearly stopped, so pivoting
            // in place feels responsive without making high-speed turns twitchy.
            let mut turn = 0.0;
            if rl.is_key_down(KeyboardKey::KEY_LEFT) {
                turn -= 1.0;
            }
            if rl.is_key_down(KeyboardKey::KEY_RIGHT) {
                turn += 1.0;
            }
            let speed_frac = (self.tank.velocity.abs() / TANK_MAX_SPEED).clamp(0.0, 1.0);
            let turn_rate = TANK_TURN_SPEED * (1.0 - 0.4 * speed_frac);
            self.tank.rotate(turn * turn_rate * dt);

            // Throttle: Up drives forward, Down reverses, neither coasts.
            let throttle = if rl.is_key_down(KeyboardKey::KEY_UP) {
                1.0
            } else if rl.is_key_down(KeyboardKey::KEY_DOWN) {
                -1.0
            } else {
                0.0
            };
            self.tank.drive(throttle, dt);
        }

        // Move along the heading, then resolve tank-vs-tank collisions: if the
        // step would overlap an enemy footprint, revert the move, kill momentum
        // (the tank stops against the obstacle), and apply one-off ramming damage
        // to both tanks (debounced by each tank's collision cooldown).
        let before = self.tank.position;
        self.tank.advance(dt);
        for enemy in &mut self.enemies {
            if self.tank.overlaps(enemy) {
                self.tank.position = before;
                self.tank.velocity = 0.0;
                if self.tank.ram_cooldown <= 0.0 && enemy.ram_cooldown <= 0.0 {
                    let dmg = rng.random_range(2.0..6.0);
                    self.tank.damage = (self.tank.damage + dmg).min(MAX_DAMAGE);
                    enemy.damage = (enemy.damage + dmg).min(MAX_DAMAGE);
                    self.tank.ram_cooldown = RAM_DAMAGE_COOLDOWN;
                    enemy.ram_cooldown = RAM_DAMAGE_COOLDOWN;
                }
                break; // already reverted; one contact is enough
            }
        }

        // Fire a shell: one per distinct spacebar press, only while ammo remains.
        if rl.is_key_pressed(KeyboardKey::KEY_SPACE) && self.tank.shells_ammo >= 1 {
            self.tank.shells_ammo -= 1;
            self.shells.push(Shell::spawn(&self.tank));
        }

        let (width, height) = (rl.get_screen_width() as f32, rl.get_screen_height() as f32);
        for shell in &mut self.shells {
            shell.update(dt, width, height);

            // A flying shell that overlaps an enemy tank hits it: deal a random
            // chunk of damage and detonate the shell at the impact point.
            if shell.state == ShellState::Flying {
                for enemy in &mut self.enemies {
                    if enemy.contains(shell.position) {
                        // Wrecks still stop the shell but take no further damage.
                        if !enemy.is_wreck() {
                            enemy.damage =
                                (enemy.damage + rng.random_range(10.0..30.0)).min(MAX_DAMAGE);
                        }
                        shell.detonate();
                        break; // one shell hits at most one tank
                    }
                }
            }
        }
        // Drop shells that have finished their bang animation.
        self.shells.retain(|s| !s.done);
    }

    /// Draw the whole scene for this frame.
    pub fn render(
        &self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        tanks_texture: &Texture2D,
        shells_texture: &Texture2D,
        damage_texture: &Texture2D,
    ) {
        let screen_height = rl.get_screen_height();
        rl.draw(thread, |mut d| {
            d.clear_background(Color::RAYWHITE);

            for enemy in &self.enemies {
                draw_tank(&mut d, tanks_texture, enemy);
                draw_damage(&mut d, damage_texture, enemy, self.time);
            }

            draw_tank(&mut d, tanks_texture, &self.tank);
            draw_damage(&mut d, damage_texture, &self.tank, self.time);

            for shell in &self.shells {
                draw_shell(&mut d, shells_texture, shell);
            }

            let hud = format!("SHELLS: {}", self.tank.shells_ammo);
            d.draw_text(&hud, 50, screen_height - 50, 24, Color::DARKGRAY);
        });
    }
}
