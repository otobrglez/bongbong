use rand::RngExt;
use sola_raylib::prelude::*;

use crate::ai::{Ai, Intent, Mover};
use crate::damage_stage::draw_damage;
use crate::physics::Physics;
use crate::shell::{Owner, Shell, ShellState, draw_shell};
use crate::shockwave::{RippleFx, Shockwave, screen_to_ripple_uv};
use crate::tank::{Dir, Tank, draw_tank};
use crate::track::{Track, draw_track};
use crate::{
    ENEMY_COUNT_MAX, ENEMY_COUNT_MIN, ENEMY_DAMAGE_MAX, ENEMY_DAMAGE_MIN, ENEMY_SPAWN_MARGIN_MAX,
    ENEMY_SPAWN_MARGIN_MIN, ENEMY_SPEED, ENEMY_SPEED_VARIANCE, EXPLOSION_DAMAGE_MAX,
    EXPLOSION_DAMAGE_MIN, EXPLOSION_KNOCKBACK_SPEED, EXPLOSION_RADIUS, IMPACT_FLASH_DURATION,
    IMPACT_FLASH_QUAD_RADIUS, KNOCKBACK_MAX_SPEED, KNOCKBACK_STRENGTH, MAX_DAMAGE,
    MUZZLE_FLASH_DURATION, MUZZLE_FLASH_QUAD_RADIUS, PHYSICS_FIXED_DT, PHYSICS_MAX_CATCHUP_SECONDS,
    PLAYER_DAMAGE_MAX, PLAYER_DAMAGE_MIN, Position, RAM_DAMAGE_COOLDOWN, RESTART_DELAY,
    SHOCKWAVE_DURATION, TRACK_SCALE_FRACTION, TRACK_SPACING, WALL_THICKNESS,
};

/// How the current round is going.
#[derive(Clone, Copy, PartialEq, Default)]
pub enum Outcome {
    #[default]
    Playing,
    Won,  // all enemies destroyed
    Lost, // player destroyed
}

/// The sprite atlases `Game::render` draws from, bundled into one param instead
/// of four so the signature doesn't grow with every new texture.
pub struct Textures<'a> {
    pub tanks: &'a Texture2D,
    pub shells: &'a Texture2D,
    pub damage: &'a Texture2D,
    pub tracks: &'a Texture2D,
}

/// The ripple post-effects `Game::render` drives, bundled into one param for the
/// same reason as `Textures`. See `shockwave.rs` for what each one does.
pub struct Effects<'a> {
    pub shock: &'a mut RippleFx,
    pub muzzle: &'a mut RippleFx,
    pub impact: &'a mut RippleFx,
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
    /// The screen-distortion ring from the most recent tank kill, if one is
    /// still playing out.
    shock: Option<Shockwave>,
    /// Small heat-haze ripples at the barrel of every shot fired recently,
    /// oldest first. Unlike `shock` there can be several in flight at once
    /// (any tank can fire independently), so these are tracked as a list.
    muzzle_flashes: Vec<Shockwave>,
    /// Small impact-flash ripples at the point a shell lands on a tank,
    /// oldest first. Same list-of-many shape as `muzzle_flashes` - several
    /// shells can land in the same frame or overlap in flight.
    impact_flashes: Vec<Shockwave>,
    /// True while the game is paused (toggled by the P key); simulation is
    /// frozen and rendering shows a "PAUSED" overlay.
    paused: bool,
    /// The rapier physics world (see docs/physics-engine-design.md). Owns
    /// every tank's rigid body plus the battlefield wall colliders.
    physics: Physics,
    /// Leftover real time not yet consumed by a fixed physics step; see
    /// `PHYSICS_FIXED_DT` and the accumulator loop in `update`.
    physics_accumulator: f32,
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
        self.shock = None;
        self.muzzle_flashes.clear();
        self.impact_flashes.clear();

        // Fresh physics world each round rather than trying to reuse/resize
        // the previous one - cheap at this scale, and simplest if the
        // battlefield size ever changes between rounds. See
        // docs/physics-engine-design.md.
        self.physics = Physics::new();
        self.physics_accumulator = 0.0;
        spawn_walls(&mut self.physics, width, height);

        // Pick a random hull (any of the 8; a mix of single/twin barrel) and center it.
        self.tank.row = TANK_ROW;
        self.tank.col = rng.random_range(0..TANK_COUNT);
        self.tank.position = Position::new(width / 2.0, height / 2.0);
        self.tank.body = Some(
            self.physics
                .spawn_tank(self.tank.position, self.tank.hull_size() * 0.5),
        );

        // Spawn enemy tanks in a band that's 20%-40% of the shorter screen
        // dimension away from the nearest edge of the battlefield, and away from
        // the player's starting spot in the middle.
        let center = self.tank.position;
        let clear = self.tank.size() * 2.0; // don't spawn on top of the player
        // Also keep spawned enemies clear of each other - without this, a
        // crowded high-count round can drop two tanks on top of one another,
        // and they start ramming (and damaging) each other before the round
        // even properly begins.
        let enemy_clear = self.tank.size() * 1.5;
        let short_side = width.min(height);
        let margin_min = short_side * ENEMY_SPAWN_MARGIN_MIN;
        let margin_max = short_side * ENEMY_SPAWN_MARGIN_MAX;
        let enemy_count = rng.random_range(ENEMY_COUNT_MIN..=ENEMY_COUNT_MAX);

        self.enemies.clear();
        self.ais.clear();
        while self.enemies.len() < enemy_count {
            let pos = Position::new(
                rng.random_range(margin_min..(width - margin_min)),
                rng.random_range(margin_min..(height - margin_min)),
            );
            let border_dist = pos.x.min(width - pos.x).min(pos.y).min(height - pos.y);
            if border_dist > margin_max {
                continue;
            }
            if pos.distance_to(center) < clear {
                continue;
            }
            if self
                .enemies
                .iter()
                .any(|e| pos.distance_to(e.position) < enemy_clear)
            {
                continue;
            }
            // Walk the alternating spawn order so each enemy looks distinct and the
            // group mixes single- and twin-barrel hulls.
            let ecol = TANK_SPRITE_ORDER[self.enemies.len() % TANK_SPRITE_ORDER.len()];
            // Vary speed within +/- ENEMY_SPEED_VARIANCE so enemies don't all move
            // in lockstep; each keeps this speed for the round.
            let factor = 1.0 + rng.random_range(-ENEMY_SPEED_VARIANCE..ENEMY_SPEED_VARIANCE);
            let mut enemy = Tank {
                row: TANK_ROW,
                col: ecol,
                position: pos,
                rotation: 180.0,             // facing down, toward the player's start
                speed: ENEMY_SPEED * factor, // enemies drive slower than the player
                ..Tank::default()
            };
            enemy.body = Some(self.physics.spawn_tank(pos, enemy.hull_size() * 0.5));
            self.enemies.push(enemy);
            self.ais.push(Ai::default());
        }
    }

    /// Step the simulation one frame.
    pub fn update(&mut self, rl: &RaylibHandle) {
        if rl.is_key_pressed(KeyboardKey::KEY_P) {
            self.paused = !self.paused;
        }
        if self.paused {
            return;
        }

        let dt = rl.get_frame_time();

        // Advance any in-flight shockwave regardless of round state, so it
        // finishes fading even if this frame's damage just ended the round.
        if let Some(shock) = &mut self.shock {
            shock.time += dt;
            if shock.time >= SHOCKWAVE_DURATION {
                self.shock = None;
            }
        }
        // Same, but for every in-flight muzzle-flash heat haze.
        self.muzzle_flashes.retain_mut(|flash| {
            flash.time += dt;
            flash.time < MUZZLE_FLASH_DURATION
        });
        // Same, but for every in-flight shell-impact flash.
        self.impact_flashes.retain_mut(|flash| {
            flash.time += dt;
            flash.time < IMPACT_FLASH_DURATION
        });

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
        // Tanks freshly destroyed this frame (by ramming or by shellfire
        // below), tagged with whether the victim was an enemy; each gets a
        // shockwave and a small splash of explosion damage to nearby tanks on
        // the *opposing* side, processed once all of this frame's movement
        // and shell hits are resolved.
        let mut kills: Vec<(Position, bool)> = Vec::new();

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

        // Hand the player's commanded velocity to its physics body. Actual
        // movement and collision (walls, tank-vs-tank blocking) happen below
        // when the physics world steps - not here.
        drive_tank(&mut self.physics, &mut self.tank, player_intent, dt);
        if player_intent.fire && self.tank.shells_ammo >= 1 {
            self.tank.shells_ammo -= 1;
            // The player always fires straight down the barrel.
            let shell = Shell::spawn(&self.tank, Owner::Player, 0.0);
            self.muzzle_flashes.push(Shockwave {
                center: shell.position,
                time: 0.0,
            });
            self.shells.push(shell);
        }

        // --- Enemies: each brain decides an intent, then hands it to physics too ---
        // Snapshot every live tank's motion for predictive collision avoidance:
        // slot 0 is the player, slots 1.. are the enemies in order. Built from
        // last frame's synced positions (nothing has moved yet this frame), so
        // an enemy can predict the others' paths without borrowing the mutable
        // enemy list mid-loop.
        let movers = self.motion_snapshot();
        for i in 0..self.enemies.len() {
            let intent = self.ais[i].think(
                &self.enemies[i],
                &self.tank,
                width,
                height,
                dt,
                &movers,
                i + 1,
                &mut rng,
            );
            drive_tank(&mut self.physics, &mut self.enemies[i], intent, dt);
            if intent.fire && self.enemies[i].shells_ammo >= 1 {
                self.enemies[i].shells_ammo -= 1;
                // Point-blank shots may be thrown off-aim (see roll_misfire).
                let shell = Shell::spawn(&self.enemies[i], Owner::Enemy(i), intent.fire_aim_offset);
                self.muzzle_flashes.push(Shockwave {
                    center: shell.position,
                    time: 0.0,
                });
                self.shells.push(shell);
            }
        }

        // --- Physics: advance the world in fixed steps ---
        // Every tank's body already has this frame's commanded velocity (set
        // above); stepping resolves all of this frame's movement and collision
        // (walls, tank-vs-tank blocking) for every body at once, rather than
        // the old per-tank sequential move-then-revert-if-overlapping dance. A
        // fixed step keeps the contact solver's behavior consistent regardless
        // of the render frame rate. See docs/physics-engine-design.md.
        self.physics_accumulator = (self.physics_accumulator + dt).min(PHYSICS_MAX_CATCHUP_SECONDS);
        while self.physics_accumulator >= PHYSICS_FIXED_DT {
            self.physics.step();
            self.physics_accumulator -= PHYSICS_FIXED_DT;
        }

        // --- Read positions back, then resolve ram damage and lay tracks ---
        let tank_before = self.tank.position;
        sync_tank_from_physics(&self.physics, &mut self.tank);
        let enemies_before: Vec<Position> = self.enemies.iter().map(|e| e.position).collect();
        for enemy in &mut self.enemies {
            sync_tank_from_physics(&self.physics, enemy);
        }

        // A tank touching the opposing side takes a cooldown-gated ram-damage
        // hit; the collider contact itself already stopped/redirected their
        // movement during the physics step above, so this only handles the
        // damage roll (see `ram`'s doc comment for why enemy-vs-enemy contact
        // doesn't call this).
        for enemy in &mut self.enemies {
            if self.tank.overlaps(enemy) {
                ram(&mut self.tank, false, enemy, true, &mut rng, &mut kills);
                break;
            }
        }
        lay_tracks(&mut self.tracks, &mut self.tank, tank_before);
        for (enemy, &before) in self.enemies.iter_mut().zip(enemies_before.iter()) {
            if enemy.overlaps(&self.tank) {
                ram(enemy, true, &mut self.tank, false, &mut rng, &mut kills);
            }
            lay_tracks(&mut self.tracks, enemy, before);
        }

        // --- Shells: move, then damage whatever enemy tank they hit ---
        // Enemy shells only hit the player; they pass harmlessly through
        // other enemies (no friendly fire), so a shell's only possible target
        // is whichever side didn't fire it. Excluding the shooter's own tank
        // still matters for the player: a shell spawns right on its own
        // tank's hit-box boundary, so without this it would detonate on
        // itself the instant it starts flying (moot for enemies now, but
        // harmless to keep).
        for shell in &mut self.shells {
            shell.update(dt, width, height);
            if shell.state != ShellState::Flying {
                continue;
            }

            if shell.owner != Owner::Player && self.tank.contains(shell.position) {
                if !self.tank.is_wreck() {
                    let dmg = rng.random_range(ENEMY_DAMAGE_MIN..ENEMY_DAMAGE_MAX);
                    self.tank.damage = (self.tank.damage + dmg).min(MAX_DAMAGE);
                    self.impact_flashes.push(Shockwave {
                        center: shell.position,
                        time: 0.0,
                    });
                    if self.tank.is_wreck() {
                        kills.push((self.tank.position, false));
                    }
                }
                shell.detonate();
                continue;
            }

            if shell.owner == Owner::Player {
                for enemy in self.enemies.iter_mut() {
                    if enemy.contains(shell.position) {
                        if !enemy.is_wreck() {
                            let dmg = rng.random_range(PLAYER_DAMAGE_MIN..PLAYER_DAMAGE_MAX);
                            enemy.damage = (enemy.damage + dmg).min(MAX_DAMAGE);
                            self.impact_flashes.push(Shockwave {
                                center: shell.position,
                                time: 0.0,
                            });
                            if enemy.is_wreck() {
                                kills.push((enemy.position, true));
                            }
                        }
                        shell.detonate();
                        break; // one shell hits at most one tank
                    }
                }
            }
        }
        // Drop shells that have finished their bang animation.
        self.shells.retain(|s| !s.done);

        // Every tank destroyed this frame gets a shockwave (the most recent
        // kill's ring is the one that plays - see the field comment on
        // `shock`) plus a small splash of damage to nearby tanks on the
        // opposing side.
        for (center, victim_was_enemy) in kills {
            self.shock = Some(Shockwave { center, time: 0.0 });
            self.apply_explosion(center, victim_was_enemy, &mut rng);
        }

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

    /// A tank's death deals a small splash of damage plus an outward shove to
    /// live tanks within EXPLOSION_RADIUS of `center`, on the side opposing
    /// whoever died - an enemy's death can still catch the player nearby, but
    /// never chips other enemies standing next to it (and vice versa). A chip
    /// of extra damage and a nudge, not another kill shot, so it doesn't
    /// chain into further explosions.
    fn apply_explosion(
        &mut self,
        center: Position,
        victim_was_enemy: bool,
        rng: &mut rand::rngs::ThreadRng,
    ) {
        if victim_was_enemy {
            explosion_hit(&mut self.tank, center, rng);
        } else {
            for enemy in &mut self.enemies {
                explosion_hit(enemy, center, rng);
            }
        }
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

    /// Draw the whole scene for this frame.
    pub fn render(
        &self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        scene_target: &mut RenderTexture2D,
        effects: &mut Effects,
        textures: &Textures,
    ) {
        let screen_width = rl.get_screen_width();
        let screen_height = rl.get_screen_height();

        // Precompute the bottom-right version/build HUD (text width must be
        // measured on the RaylibHandle, outside the draw closure).
        let version_hud = format!(
            "v{} ({})",
            env!("CARGO_PKG_VERSION"),
            env!("BONGBONG_GIT_COMMIT")
        );
        let version_hud_w = rl.measure_text(&version_hud, 24);

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
            let sub = format!(
                "Restarting in {}...",
                self.restart_timer.ceil().max(0.0) as i32
            );
            let sub_size = 28;
            let sub_w = rl.measure_text(&sub, sub_size);
            (text, color, title_size, title_w, sub, sub_size, sub_w)
        });

        // Pass 1: draw the world (tracks, tanks, shells) into an offscreen
        // render texture, so a shockwave can distort the finished frame as a
        // whole-screen shader pass in pass 2.
        rl.draw_texture_mode(thread, scene_target, |mut d| {
            d.clear_background(Color::RAYWHITE);

            // Tread marks go down first so tanks and everything else draw on top.
            for track in &self.tracks {
                draw_track(&mut d, textures.tracks, track);
            }

            for enemy in &self.enemies {
                draw_tank(&mut d, textures.tanks, enemy);
                draw_damage(&mut d, textures.damage, enemy, self.time);
            }

            draw_tank(&mut d, textures.tanks, &self.tank);
            draw_damage(&mut d, textures.damage, &self.tank, self.time);

            for shell in &self.shells {
                draw_shell(&mut d, textures.shells, shell);
            }
        });

        // If a shockwave is playing, push its current center/time to the shader
        // before the blit below samples through it.
        if let Some(shock) = &self.shock {
            let uv = screen_to_ripple_uv(shock.center, screen_width as f32, screen_height as f32);
            effects
                .shock
                .shader
                .set_shader_value(effects.shock.center_loc, uv);
            effects
                .shock
                .shader
                .set_shader_value(effects.shock.time_loc, shock.time);
        }

        // The render texture is stored upside-down relative to the screen; a
        // negative source height flips it back on the way out.
        let source = Rectangle {
            x: 0.0,
            y: 0.0,
            width: screen_width as f32,
            height: -(screen_height as f32),
        };

        rl.draw(thread, |mut d| {
            d.clear_background(Color::BLACK);

            if self.shock.is_some() {
                d.draw_shader_mode(&mut effects.shock.shader, |mut sd| {
                    sd.draw_texture_rec(
                        &*scene_target,
                        source,
                        Vector2::new(0.0, 0.0),
                        Color::WHITE,
                    );
                });
            } else {
                d.draw_texture_rec(&*scene_target, source, Vector2::new(0.0, 0.0), Color::WHITE);
            }

            // Layer each muzzle flash's tiny heat-haze ripple on top, one small
            // quad at a time: source and dest are the same on-screen patch (just
            // re-sampling that bit of the already-composited scene through the
            // ripple shader), so this reads as a localized wobble rather than
            // redistorting the whole frame.
            for flash in &self.muzzle_flashes {
                let uv =
                    screen_to_ripple_uv(flash.center, screen_width as f32, screen_height as f32);
                effects
                    .muzzle
                    .shader
                    .set_shader_value(effects.muzzle.center_loc, uv);
                effects
                    .muzzle
                    .shader
                    .set_shader_value(effects.muzzle.time_loc, flash.time);

                let r = MUZZLE_FLASH_QUAD_RADIUS;
                let flash_source = Rectangle {
                    x: flash.center.x - r,
                    y: (screen_height as f32 - flash.center.y) - r,
                    width: r * 2.0,
                    height: -(r * 2.0),
                };
                let flash_dest = Rectangle {
                    x: flash.center.x,
                    y: flash.center.y,
                    width: r * 2.0,
                    height: r * 2.0,
                };
                let origin = Vector2::new(r, r);

                d.draw_shader_mode(&mut effects.muzzle.shader, |mut sd| {
                    sd.draw_texture_pro(
                        &*scene_target,
                        flash_source,
                        flash_dest,
                        origin,
                        0.0,
                        Color::WHITE,
                    );
                });
            }

            // Same treatment for every in-flight shell-impact flash.
            for flash in &self.impact_flashes {
                let uv =
                    screen_to_ripple_uv(flash.center, screen_width as f32, screen_height as f32);
                effects
                    .impact
                    .shader
                    .set_shader_value(effects.impact.center_loc, uv);
                effects
                    .impact
                    .shader
                    .set_shader_value(effects.impact.time_loc, flash.time);

                let r = IMPACT_FLASH_QUAD_RADIUS;
                let flash_source = Rectangle {
                    x: flash.center.x - r,
                    y: (screen_height as f32 - flash.center.y) - r,
                    width: r * 2.0,
                    height: -(r * 2.0),
                };
                let flash_dest = Rectangle {
                    x: flash.center.x,
                    y: flash.center.y,
                    width: r * 2.0,
                    height: r * 2.0,
                };
                let origin = Vector2::new(r, r);

                d.draw_shader_mode(&mut effects.impact.shader, |mut sd| {
                    sd.draw_texture_pro(
                        &*scene_target,
                        flash_source,
                        flash_dest,
                        origin,
                        0.0,
                        Color::WHITE,
                    );
                });
            }

            // HUD and the end-of-round banner draw undistorted, on top of the
            // (possibly rippling) scene.
            let hp = (MAX_DAMAGE - self.tank.damage).max(0.0).round() as i32;
            let hud = format!("SHELLS: {}   HP: {}", self.tank.shells_ammo, hp);
            d.draw_text(&hud, 50, screen_height - 50, 24, Color::DARKGRAY);
            d.draw_text(
                &version_hud,
                screen_width - 50 - version_hud_w,
                screen_height - 50,
                24,
                Color::DARKGRAY,
            );

            // End-of-round banner over a dimming overlay.
            if let Some((title, color, title_size, title_w, sub, sub_size, sub_w)) = &banner {
                d.draw_rectangle(0, 0, screen_width, screen_height, Color::new(0, 0, 0, 120));
                let cx = screen_width / 2;
                let cy = screen_height / 2;
                d.draw_text(
                    title,
                    cx - title_w / 2,
                    cy - title_size,
                    *title_size,
                    *color,
                );
                d.draw_text(sub, cx - sub_w / 2, cy + 20, *sub_size, Color::RAYWHITE);
            }

            // Paused overlay draws over everything else, including the
            // end-of-round banner (its countdown is frozen too).
            if self.paused {
                d.draw_rectangle(0, 0, screen_width, screen_height, Color::new(0, 0, 0, 120));
                let title = "PAUSED";
                let title_size = 72;
                let title_w = d.measure_text(title, title_size);
                d.draw_text(
                    title,
                    screen_width / 2 - title_w / 2,
                    screen_height / 2 - title_size / 2,
                    title_size,
                    Color::RAYWHITE,
                );
            }
        });
    }
}

/// Build the battlefield boundary: four static wall colliders positioned so
/// their inner faces sit exactly at the screen edges (0..width, 0..height) -
/// the same effective bound `Tank::clamp_to_field` used to enforce by hand,
/// since a tank's collider stops flush against a wall's surface. Padded
/// outward by `WALL_THICKNESS` so corners are covered with no gap; that
/// padding is otherwise arbitrary since it's never rendered.
fn spawn_walls(physics: &mut Physics, width: f32, height: f32) {
    let t = WALL_THICKNESS;
    physics.spawn_wall(
        Position::new(-t / 2.0, height / 2.0),
        Position::new(t / 2.0, height / 2.0 + t),
    );
    physics.spawn_wall(
        Position::new(width + t / 2.0, height / 2.0),
        Position::new(t / 2.0, height / 2.0 + t),
    );
    physics.spawn_wall(
        Position::new(width / 2.0, -t / 2.0),
        Position::new(width / 2.0 + t, t / 2.0),
    );
    physics.spawn_wall(
        Position::new(width / 2.0, height + t / 2.0),
        Position::new(width / 2.0 + t, t / 2.0),
    );
}

/// Turn an intent into hull rotation plus the velocity written to a tank's
/// physics body this frame: `Tank::control`'s commanded cardinal velocity
/// plus any residual knockback drift (still hand-decayed, not yet a real
/// physics impulse - see docs/physics-engine-design.md). Shared by the player
/// and every enemy so both drive identically; a free function (not a `Game`
/// method) so it can borrow `physics` and one `tank` independently of the
/// rest of `self`.
fn drive_tank(physics: &mut Physics, tank: &mut Tank, intent: Intent, dt: f32) {
    tank.decay_knockback(dt);
    tank.control(intent.move_dir, intent.face);
    let handle = tank
        .body
        .expect("tank should always have a physics body once spawned");
    let velocity = Position::new(
        tank.velocity.x + tank.knockback.x,
        tank.velocity.y + tank.knockback.y,
    );
    physics.set_velocity(handle, velocity);
}

/// Read a tank's position back from its physics body after the world steps.
/// A free function for the same borrow-splitting reason as `drive_tank`.
fn sync_tank_from_physics(physics: &Physics, tank: &mut Tank) {
    let handle = tank
        .body
        .expect("tank should always have a physics body once spawned");
    tank.position = physics.position(handle);
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

/// Apply one-off ramming damage to two touching tanks on opposing sides,
/// debounced by each tank's collision cooldown so continuous contact doesn't
/// drain damage every frame. Only ever called for player-vs-enemy contact -
/// enemies bumping each other just block movement (see the call sites in
/// `apply_movement_enemy`), so this doesn't need to guard against that case.
/// Records the position of either tank freshly killed by the collision,
/// tagged with which side it was on, so the caller can trigger its shockwave
/// and (opposing-side-only) explosion splash.
fn ram(
    a: &mut Tank,
    a_is_enemy: bool,
    b: &mut Tank,
    b_is_enemy: bool,
    rng: &mut rand::rngs::ThreadRng,
    kills: &mut Vec<(Position, bool)>,
) {
    if a.ram_cooldown <= 0.0 && b.ram_cooldown <= 0.0 {
        let a_was_wreck = a.is_wreck();
        let b_was_wreck = b.is_wreck();
        let dmg = rng.random_range(2.0..6.0);
        a.damage = (a.damage + dmg).min(MAX_DAMAGE);
        b.damage = (b.damage + dmg).min(MAX_DAMAGE);
        a.ram_cooldown = RAM_DAMAGE_COOLDOWN;
        b.ram_cooldown = RAM_DAMAGE_COOLDOWN;
        if !a_was_wreck && a.is_wreck() {
            kills.push((a.position, a_is_enemy));
        }
        if !b_was_wreck && b.is_wreck() {
            kills.push((b.position, b_is_enemy));
        }

        // Physics-lite knockback: shove both tanks apart along the line
        // between their centers, harder the faster they were closing (using
        // this frame's already-set `velocity`, even though the position move
        // that produced it just got reverted by the caller), split by mass so
        // the lighter tank gets shoved further. A tank that's now a wreck
        // (freshly killed by this very hit, or already one) doesn't move.
        let dx = a.position.x - b.position.x;
        let dy = a.position.y - b.position.y;
        let dist = (dx * dx + dy * dy).sqrt();
        if dist > 0.001 {
            let axis = Vector2::new(dx / dist, dy / dist);
            let rel_x = a.velocity.x - b.velocity.x;
            let rel_y = a.velocity.y - b.velocity.y;
            let impact_speed = (rel_x * rel_x + rel_y * rel_y).sqrt();
            let push = (impact_speed * KNOCKBACK_STRENGTH).min(KNOCKBACK_MAX_SPEED);
            let total_mass = a.mass() + b.mass();

            if !a.is_wreck() {
                let a_push = (push * 2.0 * b.mass() / total_mass).min(KNOCKBACK_MAX_SPEED);
                a.knockback.x += axis.x * a_push;
                a.knockback.y += axis.y * a_push;
            }
            if !b.is_wreck() {
                let b_push = (push * 2.0 * a.mass() / total_mass).min(KNOCKBACK_MAX_SPEED);
                b.knockback.x -= axis.x * b_push;
                b.knockback.y -= axis.y * b_push;
            }
        }
    }
}

/// Apply one tank's share of a nearby explosion: a small chip of damage plus
/// an outward knockback shove that fades linearly with distance (full
/// strength at the blast center, nothing at EXPLOSION_RADIUS). No-op on a
/// wreck (immovable, and past caring about a chip of damage) or a tank
/// outside the blast radius.
fn explosion_hit(tank: &mut Tank, center: Position, rng: &mut rand::rngs::ThreadRng) {
    if tank.is_wreck() {
        return;
    }
    let dx = tank.position.x - center.x;
    let dy = tank.position.y - center.y;
    let dist = (dx * dx + dy * dy).sqrt();
    if dist > EXPLOSION_RADIUS {
        return;
    }

    let dmg = rng.random_range(EXPLOSION_DAMAGE_MIN..EXPLOSION_DAMAGE_MAX);
    tank.damage = (tank.damage + dmg).min(MAX_DAMAGE);

    // Push harder the closer the tank was to the blast, and divide by mass
    // relative to a default tank's so a heavier one (see Tank::mass) resists
    // the shove more.
    let falloff = 1.0 - dist / EXPLOSION_RADIUS;
    let reference_mass = Tank::default().mass();
    let push = (EXPLOSION_KNOCKBACK_SPEED * falloff * reference_mass / tank.mass())
        .min(KNOCKBACK_MAX_SPEED);
    let axis = if dist > 0.001 {
        Vector2::new(dx / dist, dy / dist)
    } else {
        // Degenerate case: sitting exactly on the blast center - push in an
        // arbitrary direction rather than dividing by zero.
        Vector2::new(1.0, 0.0)
    };
    tank.knockback.x += axis.x * push;
    tank.knockback.y += axis.y * push;
}
