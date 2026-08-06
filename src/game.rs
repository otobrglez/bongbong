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
    EXPLOSION_DAMAGE_MIN, EXPLOSION_KNOCKBACK_SPEED, EXPLOSION_RADIUS, HUD_CRITICAL_THRESHOLD,
    HUD_WARN_THRESHOLD, IMPACT_FLASH_DURATION, IMPACT_FLASH_QUAD_RADIUS, KNOCKBACK_MAX_SPEED,
    KNOCKBACK_STRENGTH, MAX_DAMAGE, MAX_SHELLS, MUZZLE_FLASH_DURATION, MUZZLE_FLASH_QUAD_RADIUS,
    PHYSICS_FIXED_DT, PHYSICS_MAX_CATCHUP_SECONDS, PLAYER_DAMAGE_MAX, PLAYER_DAMAGE_MIN, Position,
    RAM_DAMAGE_COOLDOWN, RESTART_DELAY, SHELL_HIT_HALF_EXTENT, SHELL_IMPACT_KNOCKBACK_SPEED,
    SHELL_SPEED, SHELL_VARIANTS, SHOCKWAVE_DURATION, TANK_ACCEL_FORCE, TANK_DECEL_FORCE,
    TANK_TURN_GRIP_FORCE, TRACK_SCALE_FRACTION, TRACK_SCALE_JITTER, TRACK_SPACING,
    TRACK_WOBBLE_AMP_MAX_DEG, TRACK_WOBBLE_AMP_MIN_DEG, TRACK_WOBBLE_WAVELENGTH_MAX,
    TRACK_WOBBLE_WAVELENGTH_MIN, WALL_THICKNESS,
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
        // Pick a random shells.png row-variant for this round's shots (see
        // Tank::shell_variant).
        self.tank.shell_variant = rng.random_range(0..SHELL_VARIANTS);
        roll_track_distortion(&mut self.tank, &mut rng);
        self.tank.position = Position::new(width / 2.0, height / 2.0);
        self.tank.body = Some(self.physics.spawn_tank(
            self.tank.position,
            self.tank.hull_size() * 0.5,
            self.tank.mass(),
        ));
        self.tank.hit_sensor = Some(
            self.physics
                .add_hit_sensor(self.tank.body.unwrap(), self.tank.size() * 0.5),
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
                shell_variant: rng.random_range(0..SHELL_VARIANTS),
                position: pos,
                rotation: 180.0,             // facing down, toward the player's start
                speed: ENEMY_SPEED * factor, // enemies drive slower than the player
                ..Tank::default()
            };
            roll_track_distortion(&mut enemy, &mut rng);
            enemy.body = Some(
                self.physics
                    .spawn_tank(pos, enemy.hull_size() * 0.5, enemy.mass()),
            );
            enemy.hit_sensor = Some(
                self.physics
                    .add_hit_sensor(enemy.body.unwrap(), enemy.size() * 0.5),
            );
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
            let mut shell = Shell::spawn(&self.tank, Owner::Player, 0.0);
            shell.body = Some(
                self.physics
                    .spawn_shell(shell.position, SHELL_HIT_HALF_EXTENT),
            );
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
                let mut shell =
                    Shell::spawn(&self.enemies[i], Owner::Enemy(i), intent.fire_aim_offset);
                shell.body = Some(
                    self.physics
                        .spawn_shell(shell.position, SHELL_HIT_HALF_EXTENT),
                );
                self.muzzle_flashes.push(Shockwave {
                    center: shell.position,
                    time: 0.0,
                });
                self.shells.push(shell);
            }
        }

        // --- Shells: advance movement/animation, then sync into physics ---
        // A shell's position is still hand-integrated (velocity * dt) rather
        // than physics-driven, matching its existing state machine - but
        // pushing that position into its kinematic sensor here, before the
        // physics step below, is what lets the intersection queries after
        // that step (see further down) reflect this frame's movement. See
        // docs/physics-engine-design.md.
        for shell in &mut self.shells {
            let was_flying = shell.state == ShellState::Flying;
            shell.update(dt, width, height);
            // Shell::update self-detonates a Flying shell that crosses the
            // screen edge (see its doc comment) - the only detonation path
            // that isn't already covered by the tank-hit checks further
            // below. Without this, a shell that flies off the battlefield
            // instead of hitting a tank vanished with no impact flash at
            // all, unlike every other way a shell can end.
            if was_flying && shell.state != ShellState::Flying {
                self.impact_flashes.push(Shockwave {
                    center: shell.position,
                    time: 0.0,
                });
            }
            let handle = shell
                .body
                .expect("shell should always have a physics body once spawned");
            self.physics.set_kinematic_position(handle, shell.position);
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
        // doesn't call this). "Touching" is read straight from rapier's own
        // narrow-phase contact state (see `Physics::touching`), not a
        // hand-rolled geometric re-check.
        for enemy in &mut self.enemies {
            if tanks_touching(&self.physics, &self.tank, enemy) {
                ram(
                    &mut self.tank,
                    false,
                    enemy,
                    true,
                    &mut self.physics,
                    &mut rng,
                    &mut kills,
                );
                break;
            }
        }
        lay_tracks(&mut self.tracks, &mut self.tank, tank_before);
        for (enemy, &before) in self.enemies.iter_mut().zip(enemies_before.iter()) {
            if tanks_touching(&self.physics, enemy, &self.tank) {
                ram(
                    enemy,
                    true,
                    &mut self.tank,
                    false,
                    &mut self.physics,
                    &mut rng,
                    &mut kills,
                );
            }
            lay_tracks(&mut self.tracks, enemy, before);
        }

        // --- Shells: damage whatever tank they're intersecting (Flying only) ---
        // A shell can hit any tank except the one that fired it - including
        // other enemies, so enemy fire is a real hazard to the whole field,
        // not just the player. Excluding the shooter's own tank still
        // matters: a shell spawns right on its own tank's hit sensor, so
        // without this it would detonate on itself the instant it starts
        // flying. Damage amount depends on who fired it (PLAYER_DAMAGE_*/
        // ENEMY_DAMAGE_*), not who it hits. Hit detection reads real physics
        // intersections (a shell's sensor vs. a tank's hit sensor - see
        // `Physics::intersecting` and `add_hit_sensor`) rather than a
        // hand-rolled point-in-box check.
        for shell in &mut self.shells {
            if shell.state != ShellState::Flying {
                continue;
            }
            let shell_handle = shell
                .body
                .expect("shell should always have a physics body once spawned");
            let shell_collider = self.physics.collider_of(shell_handle);
            let (dmg_min, dmg_max) = if shell.owner == Owner::Player {
                (PLAYER_DAMAGE_MIN, PLAYER_DAMAGE_MAX)
            } else {
                (ENEMY_DAMAGE_MIN, ENEMY_DAMAGE_MAX)
            };

            if shell.owner != Owner::Player {
                let sensor = self
                    .tank
                    .hit_sensor
                    .expect("tank should always have a hit sensor once spawned");
                if self.physics.intersecting(shell_collider, sensor) {
                    if !self.tank.is_wreck() {
                        let dmg = rng.random_range(dmg_min..dmg_max);
                        self.tank.damage = (self.tank.damage + dmg).min(MAX_DAMAGE);
                        self.impact_flashes.push(Shockwave {
                            center: shell.position,
                            time: 0.0,
                        });
                        if self.tank.is_wreck() {
                            kills.push((self.tank.position, false));
                        } else {
                            shell_impact(&mut self.tank, shell, &mut self.physics);
                        }
                    }
                    shell.detonate();
                    continue;
                }
            }

            for (i, enemy) in self.enemies.iter_mut().enumerate() {
                if shell.owner == Owner::Enemy(i) {
                    continue; // never hit the tank that fired it
                }
                let sensor = enemy
                    .hit_sensor
                    .expect("tank should always have a hit sensor once spawned");
                if self.physics.intersecting(shell_collider, sensor) {
                    if !enemy.is_wreck() {
                        let dmg = rng.random_range(dmg_min..dmg_max);
                        enemy.damage = (enemy.damage + dmg).min(MAX_DAMAGE);
                        self.impact_flashes.push(Shockwave {
                            center: shell.position,
                            time: 0.0,
                        });
                        if enemy.is_wreck() {
                            kills.push((enemy.position, true));
                        } else {
                            shell_impact(enemy, shell, &mut self.physics);
                        }
                    }
                    shell.detonate();
                    break; // one shell hits at most one tank
                }
            }
        }
        // Remove physics bodies for shells finishing their bang animation
        // this frame, then drop them.
        for shell in self.shells.iter().filter(|s| s.done) {
            if let Some(handle) = shell.body {
                self.physics.remove_body(handle);
            }
        }
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

    /// A tank's death deals a small splash of damage, plus an outward shove,
    /// to live tanks within EXPLOSION_RADIUS of `center`. The shove reaches
    /// every live tank regardless of side - a real shockwave doesn't check
    /// allegiance - but the damage stays side-restricted exactly as before:
    /// only the side opposing whoever died takes the chip of extra damage
    /// (an enemy's death can still catch the player nearby, but never chips
    /// other enemies standing next to it, and vice versa), so it's a chip
    /// and a nudge, not another kill shot chaining into further explosions.
    fn apply_explosion(
        &mut self,
        center: Position,
        victim_was_enemy: bool,
        rng: &mut rand::rngs::ThreadRng,
    ) {
        explosion_hit(
            &mut self.tank,
            center,
            victim_was_enemy,
            &mut self.physics,
            rng,
        );
        for enemy in &mut self.enemies {
            explosion_hit(enemy, center, !victim_was_enemy, &mut self.physics, rng);
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

        // Precompute the SHELLS/HP HUD as separate runs so each number can
        // carry its own color (hud_number_color) while the labels stay
        // neutral - raylib's draw_text is single-color per call, so the line
        // is drawn as four adjacent segments rather than one string. Widths
        // measured here for the same reason as version_hud_w above.
        let hp = (MAX_DAMAGE - self.tank.damage).max(0.0).round() as i32;
        let shells = self.tank.shells_ammo;
        let shells_color = hud_number_color(shells as f32, MAX_SHELLS as f32);
        let hp_color = hud_number_color(hp as f32, MAX_DAMAGE);
        let hud_shells_label = "SHELLS: ";
        let hud_shells_num = format!("{shells}");
        let hud_mid = "   HP: ";
        let hud_hp_num = format!("{hp}");
        let hud_shells_label_w = rl.measure_text(hud_shells_label, 24);
        let hud_shells_num_w = rl.measure_text(&hud_shells_num, 24);
        let hud_mid_w = rl.measure_text(hud_mid, 24);

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
            let hud_y = screen_height - 50;
            let mut hud_x = 50;
            d.draw_text(hud_shells_label, hud_x, hud_y, 24, Color::DARKGRAY);
            hud_x += hud_shells_label_w;
            d.draw_text(&hud_shells_num, hud_x, hud_y, 24, shells_color);
            hud_x += hud_shells_num_w;
            d.draw_text(hud_mid, hud_x, hud_y, 24, Color::DARKGRAY);
            hud_x += hud_mid_w;
            d.draw_text(&hud_hp_num, hud_x, hud_y, 24, hp_color);
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
/// the same bound the old hand-rolled `Tank::clamp_to_field` used to
/// enforce, since a tank's collider now stops flush against a wall's
/// surface instead. Padded outward by `WALL_THICKNESS` so corners are
/// covered with no gap; that padding is otherwise arbitrary since it's
/// never rendered.
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

/// Color for a HUD number (SHELLS or HP) given its current value and max -
/// default gray above HUD_WARN_THRESHOLD, orange between the warn and
/// critical thresholds, red below. Shared by both since they're the same
/// current/max shape, just different units.
fn hud_number_color(current: f32, max: f32) -> Color {
    let frac = if max > 0.0 { current / max } else { 0.0 };
    if frac < HUD_CRITICAL_THRESHOLD {
        Color::RED
    } else if frac < HUD_WARN_THRESHOLD {
        Color::ORANGE
    } else {
        Color::DARKGRAY
    }
}

/// True if `rotation` faces along the x axis (Right/Left) rather than y
/// (Up/Down). `Tank::rotation` is always exactly one of 0/90/180/270 (see
/// `Dir::rotation`), so this is an exact match, not a fuzzy angle check.
fn facing_along_x(rotation: f32) -> bool {
    let r = rotation.rem_euclid(360.0);
    (r - 90.0).abs() < 1.0 || (r - 270.0).abs() < 1.0
}

/// Turn an intent into hull rotation plus a mass-aware acceleration impulse
/// nudging a tank's physics body toward its commanded velocity - not an
/// instant snap, and not a car-like blend either. `Tank::control` still
/// decides the *target* velocity (unchanged). Velocity always splits against
/// the hull's own facing (`Tank::rotation`, updated by `control` above),
/// never against whether a key happens to be held this frame: the axis along
/// the hull (forward/back) chases the target using `TANK_ACCEL_FORCE` when
/// speeding up or the deliberately weaker `TANK_DECEL_FORCE` when
/// slowing/reversing/coasting to a stop - both divided by `Tank::mass` and
/// scaled by `Tank::speed_factor`, so a damaged tank is sluggish too. The
/// axis perpendicular to the hull gets scrubbed toward zero hard by
/// `TANK_TURN_GRIP_FORCE` instead, unscaled by mass/damage factors beyond
/// `Tank::mass` itself - see its doc comment in `lib.rs` for why. Real tank
/// tracks resist lateral sliding mechanically, all the time, not just while
/// the driver is actively steering, so this applies whether or not a
/// direction is currently held: a corner reads as the hull snapping onto the
/// new axis rather than sliding through it, and a ram/explosion/shell
/// knockback that shoves a tank sideways to wherever it's currently facing
/// gets killed almost as fast, instead of skidding out at the same weak rate
/// as a voluntary stop. (While a direction *is* held, `control` has already
/// set `tank.rotation` to that same direction this frame, so the axis this
/// picks is identical to the driven axis - unchanged from before.) Shared by
/// the player and every enemy so both drive identically; a free function
/// (not a `Game` method) so it can borrow `physics` and one `tank`
/// independently of the rest of `self`.
fn drive_tank(physics: &mut Physics, tank: &mut Tank, intent: Intent, dt: f32) {
    let handle = tank
        .body
        .expect("tank should always have a physics body once spawned");
    let current = physics.velocity(handle);

    tank.control(intent.move_dir, intent.face);
    let target = tank.velocity;

    let along_x = facing_along_x(tank.rotation);
    let (current_on, target_on, current_off) = if along_x {
        (current.x, target.x, current.y)
    } else {
        (current.y, target.y, current.x)
    };

    let want_on = target_on - current_on;
    let speeding_up = want_on * current_on >= 0.0;
    let force = if speeding_up {
        TANK_ACCEL_FORCE
    } else {
        TANK_DECEL_FORCE
    };
    let max_on = force * tank.speed_factor() / tank.mass() * dt;
    let delta_on = want_on.clamp(-max_on, max_on);

    let max_off = TANK_TURN_GRIP_FORCE / tank.mass() * dt;
    let delta_off = (-current_off).clamp(-max_off, max_off);

    let delta = if along_x {
        Position::new(delta_on, delta_off)
    } else {
        Position::new(delta_off, delta_on)
    };

    physics.apply_impulse(
        handle,
        Position::new(delta.x * tank.mass(), delta.y * tank.mass()),
    );
}

/// Read a tank's position back from its physics body after the world steps.
/// A free function for the same borrow-splitting reason as `drive_tank`.
fn sync_tank_from_physics(physics: &Physics, tank: &mut Tank) {
    let handle = tank
        .body
        .expect("tank should always have a physics body once spawned");
    tank.position = physics.position(handle);
}

/// True if `a` and `b`'s physics bodies currently have an active contact.
/// See `Physics::touching`.
fn tanks_touching(physics: &Physics, a: &Tank, b: &Tank) -> bool {
    let a = a
        .body
        .expect("tank should always have a physics body once spawned");
    let b = b
        .body
        .expect("tank should always have a physics body once spawned");
    physics.touching(a, b)
}

/// Roll a fresh set of per-tank track-distortion parameters (see
/// TRACK_WOBBLE_AMP_MIN_DEG etc. in lib.rs) onto `tank`. Shared by the player
/// and every enemy spawn site in `Game::init` so both roll the same way.
fn roll_track_distortion(tank: &mut Tank, rng: &mut rand::rngs::ThreadRng) {
    tank.track_wobble_amp = rng.random_range(TRACK_WOBBLE_AMP_MIN_DEG..TRACK_WOBBLE_AMP_MAX_DEG);
    let wavelength = rng.random_range(TRACK_WOBBLE_WAVELENGTH_MIN..TRACK_WOBBLE_WAVELENGTH_MAX);
    // Radians per mark: each mark represents TRACK_SPACING px of travel, so a
    // full 2*PI cycle should span `wavelength` px, i.e. wavelength/TRACK_SPACING
    // marks.
    tank.track_wobble_freq = std::f32::consts::TAU * TRACK_SPACING / wavelength;
    tank.track_wobble_phase = rng.random_range(0.0..std::f32::consts::TAU);
    tank.track_scale_jitter =
        rng.random_range((1.0 - TRACK_SCALE_JITTER)..(1.0 + TRACK_SCALE_JITTER));
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

    // Heading of this frame's straight-line travel chord (same 0-degrees-up,
    // clockwise convention as `Dir::rotation`) - not the hull's cosmetic
    // `tank.rotation`, which snaps instantly on a keypress (see
    // Tank::control). This is deliberately the *raw*, un-smoothed heading:
    // an earlier version of this function eased each mark's rotation toward
    // it over several marks to fake a rounder-looking turn, but that
    // fabricated curvature where none physically exists - a straight
    // reversal (e.g. right into left) never leaves its axis, so the real
    // heading jumps directly (90 <-> 270) with no genuine in-between
    // direction, and easing through one anyway visibly rotated marks sitting
    // on a perfectly straight line. A real 90-degree turn, by contrast,
    // *does* have real in-between headings - the tank's velocity has both
    // axes' components at once for a stretch while TANK_TURN_GRIP_FORCE
    // scrubs the old axis out and TANK_ACCEL_FORCE ramps the new one up (see
    // Game::drive_tank) - so sampling this raw heading densely enough (see
    // TRACK_SPACING) is enough on its own to trace that real curve, with
    // nothing invented. This also makes sliding sideways from a ram or
    // explosion lay tracks that follow where the tank is actually going, not
    // where it's pointed.
    let mut heading = (-back.x).atan2(back.y).to_degrees();
    if heading < 0.0 {
        heading += 360.0;
    }

    // Push marks out to the tank's rear edge so the trail comes out from behind
    // the hull and never pokes ahead of it.
    let rear = tank.hull_size() * 0.5;
    let scale = tank.scale * TRACK_SCALE_FRACTION * tank.track_scale_jitter;

    tank.track_accum += moved;
    // Step up the path in TRACK_SPACING increments so the spacing stays even
    // regardless of speed or frame rate. `dist_back` is how far behind the current
    // position each mark sits.
    while tank.track_accum >= TRACK_SPACING {
        tank.track_accum -= TRACK_SPACING;
        let dist_back = rear + tank.track_accum;
        // Wobble this mark's rotation around the true heading using this
        // tank's own amplitude/frequency/phase (see roll_track_distortion),
        // so a straight drive doesn't stamp a perfectly repeated mark and
        // each tank's trail reads as its own tread pattern rather than
        // identical to every other tank's.
        let wobble = tank.track_wobble_amp
            * (tank.track_mark_count as f32 * tank.track_wobble_freq + tank.track_wobble_phase)
                .sin();
        tracks.push(Track {
            position: Position::new(
                tank.position.x + back.x * dist_back,
                tank.position.y + back.y * dist_back,
            ),
            rotation: heading + wobble,
            scale,
            age: 0.0,
        });
        tank.track_mark_count += 1;
    }
}

/// Give a tank a small shove along the shell's travel direction when it's
/// hit - a "tap", much weaker than a ram or explosion. Only ever called when
/// the tank isn't (and didn't just become) a wreck, matching `ram` and
/// `explosion_hit`'s convention that a fresh wreck doesn't get knocked
/// around; `shell.velocity` is a fixed-magnitude (`SHELL_SPEED`) vector set
/// once at spawn, so dividing by it is a cheap, exact way to get the unit
/// travel direction without a fresh sqrt.
fn shell_impact(tank: &mut Tank, shell: &Shell, physics: &mut Physics) {
    let dir = Vector2::new(
        shell.velocity.x / SHELL_SPEED,
        shell.velocity.y / SHELL_SPEED,
    );
    let handle = tank
        .body
        .expect("tank should always have a physics body once spawned");
    physics.apply_impulse(
        handle,
        Position::new(
            dir.x * SHELL_IMPACT_KNOCKBACK_SPEED * tank.mass(),
            dir.y * SHELL_IMPACT_KNOCKBACK_SPEED * tank.mass(),
        ),
    );
}

/// Apply one-off ramming damage to two touching tanks on opposing sides,
/// debounced by each tank's collision cooldown so continuous contact doesn't
/// drain damage every frame. Only ever called for player-vs-enemy contact -
/// enemies bumping each other are kept apart by the physics engine's own
/// contact response (see `Game::update`) without dealing damage, so this
/// doesn't need to guard against that case. Records the position of either
/// tank freshly killed by the collision, tagged with which side it was on,
/// so the caller can trigger its shockwave and (opposing-side-only)
/// explosion splash.
fn ram(
    a: &mut Tank,
    a_is_enemy: bool,
    b: &mut Tank,
    b_is_enemy: bool,
    physics: &mut Physics,
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

        // Knockback: shove both tanks apart along the line between their
        // centers, harder the faster they were closing (using this frame's
        // `velocity`), split by mass so the lighter tank gets shoved further.
        // A tank that's now a wreck (freshly killed by this very hit, or
        // already one) doesn't move. The desired velocity change per tank is
        // still worked out by hand (same formula as before); what's real now
        // is the *application* - `physics.apply_impulse` on each tank's own
        // body, converting the desired push into `push * that tank's own
        // mass` so the resulting velocity change is exact.
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
                let handle = a
                    .body
                    .expect("tank should always have a physics body once spawned");
                physics.apply_impulse(
                    handle,
                    Position::new(axis.x * a_push * a.mass(), axis.y * a_push * a.mass()),
                );
            }
            if !b.is_wreck() {
                let b_push = (push * 2.0 * a.mass() / total_mass).min(KNOCKBACK_MAX_SPEED);
                let handle = b
                    .body
                    .expect("tank should always have a physics body once spawned");
                physics.apply_impulse(
                    handle,
                    Position::new(-axis.x * b_push * b.mass(), -axis.y * b_push * b.mass()),
                );
            }
        }
    }
}

/// Apply one tank's share of a nearby explosion: an outward knockback shove
/// that fades linearly with distance (full strength at the blast center,
/// nothing at EXPLOSION_RADIUS), reaching every live tank in range regardless
/// of side - a real shockwave doesn't check allegiance - plus, only when
/// `damage` is true (the caller passes this for the side opposing whoever
/// died, never a tank's own side), a small chip of extra damage. No-op on a
/// wreck (immovable, and past caring about a chip of damage) or a tank
/// outside the blast radius.
fn explosion_hit(
    tank: &mut Tank,
    center: Position,
    damage: bool,
    physics: &mut Physics,
    rng: &mut rand::rngs::ThreadRng,
) {
    if tank.is_wreck() {
        return;
    }
    let dx = tank.position.x - center.x;
    let dy = tank.position.y - center.y;
    let dist = (dx * dx + dy * dy).sqrt();
    if dist > EXPLOSION_RADIUS {
        return;
    }

    if damage {
        let dmg = rng.random_range(EXPLOSION_DAMAGE_MIN..EXPLOSION_DAMAGE_MAX);
        tank.damage = (tank.damage + dmg).min(MAX_DAMAGE);
    }

    // Push harder the closer the tank was to the blast, and divide by mass
    // relative to a default tank's so a heavier one (see Tank::mass) resists
    // the shove more. As in `ram`, the desired push is still worked out by
    // hand; applying it as `physics.apply_impulse` (impulse = push * this
    // tank's own mass) is what makes it a real physics impulse.
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
    let handle = tank
        .body
        .expect("tank should always have a physics body once spawned");
    physics.apply_impulse(
        handle,
        Position::new(axis.x * push * tank.mass(), axis.y * push * tank.mass()),
    );
}
