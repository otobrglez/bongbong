//! Read/mutate surface for tooling that drives a *live* `Game` between
//! frames - the dev server (`crate::devserver`) and headless tests. Every
//! method here runs at the frame boundary (never inside `update`): the
//! round RNG is back in `Game::rng`, no query is active, and mutations land
//! before the next frame's phases read them. Nothing consumes the round
//! RNG except `debug_spawn_enemy`, which says so.

use hecs::Entity;
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sola_raylib::core::math::Vector2;

use crate::ai::{Ai, AiSnapshot};
use crate::bullet::{Bullet, BulletState};
use crate::frog::Frog;
use crate::obstacle::Obstacle;
use crate::pickup::{Pickup, PickupKind};
use crate::plasma::{Plasma, PlasmaState};
use crate::shell::{Shell, ShellState};
use crate::tank::{ActiveWeapon, Tank};
use crate::tuning::{TANK_NAMES, tuning};
use crate::{DAMAGE_VARIANTS, MAX_DAMAGE, Position, TANK_SHELL_VARIANT_BY_ROW};

use super::{
    Frame, Game, Outcome, PLAYER_OWNER_SLOT, TANK_SPRITE_ORDER, TANK_VARIANTS, enemy_owner_slot, roll_track_distortion,
    with_frog, with_tank,
};

/// How much of the world `Game::debug_snapshot` serialises.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Detail {
    /// Tanks without AI memory or contact stats; projectiles, pickups, frog.
    #[default]
    Compact,
    /// Everything, including each enemy's `AiSnapshot`.
    Full,
}

/// One frame's world state as plain numbers (`Vector2` is not
/// serialisable, hence the `x`/`y` pairs). Values are rounded to 0.1 so a
/// full snapshot of a busy round stays a few kilobytes.
#[derive(Serialize, Debug)]
pub struct DebugSnapshot {
    pub frame: u64,
    pub time: f32,
    /// Hex, the form `--seed` accepts.
    pub seed: String,
    pub outcome: Outcome,
    pub paused: bool,
    pub restart_timer: f32,
    pub width: f32,
    pub height: f32,
    pub tanks: Vec<TankDebug>,
    /// At most `PROJECTILE_CAP` entries; `projectiles_total` is the real count.
    pub projectiles: Vec<ProjectileDebug>,
    pub projectiles_total: usize,
    pub pickups: Vec<PickupDebug>,
    pub frog: Option<FrogDebug>,
    pub obstacles_alive: usize,
    /// Last enemy phase's engagement-slot targets, by owner slot.
    pub engage: Vec<EngageDebug>,
}

#[derive(Serialize, Debug)]
pub struct TankDebug {
    /// `Tank::owner_slot`: 0 is the player, enemies count from 1.
    pub slot: usize,
    pub is_player: bool,
    pub row: i32,
    pub chassis: &'static str,
    pub x: f32,
    pub y: f32,
    pub rotation: f32,
    /// Real physics velocity.
    pub vx: f32,
    pub vy: f32,
    /// Commanded velocity (`Tank::velocity`), `Full` only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cvx: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cvy: Option<f32>,
    pub damage: f32,
    pub hp: f32,
    pub wreck: bool,
    pub shells: i32,
    pub minigun: i32,
    pub plasma: i32,
    pub laser: i32,
    pub weapon: &'static str,
    pub shield: f32,
    pub boost: f32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub touching_static: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ai: Option<AiSnapshot>,
}

#[derive(Serialize, Debug)]
pub struct ProjectileDebug {
    pub kind: &'static str,
    /// Owner slot (see `TankDebug::slot`).
    pub owner: usize,
    pub x: f32,
    pub y: f32,
    pub vx: f32,
    pub vy: f32,
    pub state: &'static str,
}

#[derive(Serialize, Debug)]
pub struct PickupDebug {
    pub kind: PickupKind,
    pub x: f32,
    pub y: f32,
}

#[derive(Serialize, Debug)]
pub struct FrogDebug {
    pub x: f32,
    pub y: f32,
    pub health: f32,
    pub max_health: f32,
    pub dead: bool,
    pub hopping: bool,
}

#[derive(Serialize, Debug)]
pub struct EngageDebug {
    pub slot: usize,
    pub x: f32,
    pub y: f32,
}

/// Fields `Game::debug_set_tank` overwrites; anything left `None` is untouched.
#[derive(Default, Deserialize, Debug)]
#[serde(default)]
pub struct TankPatch {
    pub damage: Option<f32>,
    pub shells_ammo: Option<i32>,
    pub minigun_ammo: Option<i32>,
    pub plasma_ammo: Option<i32>,
    pub laser_charges: Option<i32>,
    pub shield_timer: Option<f32>,
    pub speed_boost_timer: Option<f32>,
}

/// Cap on serialised projectiles per snapshot (a minigun crossfire can
/// have hundreds in flight).
const PROJECTILE_CAP: usize = 64;

fn r1(v: f32) -> f32 {
    (v * 10.0).round() / 10.0
}

fn shell_state_name(s: ShellState) -> &'static str {
    match s {
        ShellState::Fire0 | ShellState::Fire1 | ShellState::Fire2 => "fire",
        ShellState::Flying => "flying",
        ShellState::Hit0 | ShellState::Hit1 | ShellState::Hit2 => "hit",
    }
}

fn plasma_state_name(s: PlasmaState) -> &'static str {
    match s {
        PlasmaState::Fire0 | PlasmaState::Fire1 | PlasmaState::Fire2 => "fire",
        PlasmaState::Flying => "flying",
        PlasmaState::Hit0 | PlasmaState::Hit1 | PlasmaState::Hit2 => "hit",
    }
}

fn bullet_state_name(s: BulletState) -> &'static str {
    match s {
        BulletState::Muzzle => "fire",
        BulletState::Flying => "flying",
        BulletState::Hit => "hit",
    }
}

impl Game {
    /// The world as plain values - see `DebugSnapshot`. Tanks are sorted by
    /// owner slot so two snapshots line up entry for entry.
    pub fn debug_snapshot(&self, width: f32, height: f32, detail: Detail) -> DebugSnapshot {
        let full = detail == Detail::Full;
        let mut tanks: Vec<TankDebug> = self
            .world
            .query::<(&Tank, Option<&Ai>)>()
            .iter()
            .map(|(tank, ai)| {
                let body = tank.body.expect("tank should always have a physics body once spawned");
                let vel = self.physics.velocity(body);
                TankDebug {
                    slot: tank.owner_slot,
                    is_player: tank.owner_slot == PLAYER_OWNER_SLOT,
                    row: tank.row,
                    chassis: TANK_NAMES[tank.row as usize],
                    x: r1(tank.position.x),
                    y: r1(tank.position.y),
                    rotation: tank.rotation,
                    vx: r1(vel.x),
                    vy: r1(vel.y),
                    cvx: full.then(|| r1(tank.velocity.x)),
                    cvy: full.then(|| r1(tank.velocity.y)),
                    damage: r1(tank.damage),
                    hp: r1(MAX_DAMAGE - tank.damage),
                    wreck: tank.is_wreck(),
                    shells: tank.shells_ammo,
                    minigun: tank.minigun_ammo,
                    plasma: tank.plasma_ammo,
                    laser: tank.laser_charges,
                    weapon: tank.active_weapon().name(),
                    shield: r1(tank.shield_timer),
                    boost: r1(tank.speed_boost_timer),
                    touching_static: full.then(|| self.physics.contact_stats(body).touching_static),
                    ai: if full { ai.map(Ai::snapshot) } else { None },
                }
            })
            .collect();
        tanks.sort_by_key(|t| t.slot);

        let mut projectiles: Vec<ProjectileDebug> = Vec::new();
        for s in self.world.query::<&Shell>().iter() {
            projectiles.push(ProjectileDebug {
                kind: "shell",
                owner: s.owner.slot(),
                x: r1(s.position.x),
                y: r1(s.position.y),
                vx: r1(s.velocity.x),
                vy: r1(s.velocity.y),
                state: shell_state_name(s.state),
            });
        }
        for p in self.world.query::<&Plasma>().iter() {
            projectiles.push(ProjectileDebug {
                kind: "plasma",
                owner: p.owner.slot(),
                x: r1(p.position.x),
                y: r1(p.position.y),
                vx: r1(p.velocity.x),
                vy: r1(p.velocity.y),
                state: plasma_state_name(p.state),
            });
        }
        for b in self.world.query::<&Bullet>().iter() {
            projectiles.push(ProjectileDebug {
                kind: "bullet",
                owner: b.owner.slot(),
                x: r1(b.position.x),
                y: r1(b.position.y),
                vx: r1(b.velocity.x),
                vy: r1(b.velocity.y),
                state: bullet_state_name(b.state),
            });
        }
        let projectiles_total = projectiles.len();
        projectiles.truncate(PROJECTILE_CAP);

        let mut pickups: Vec<PickupDebug> = self
            .world
            .query::<&Pickup>()
            .iter()
            .map(|p| PickupDebug { kind: p.kind, x: r1(p.position.x), y: r1(p.position.y) })
            .collect();
        pickups.sort_by(|a, b| (a.x, a.y).partial_cmp(&(b.x, b.y)).unwrap_or(std::cmp::Ordering::Equal));

        let frog = self.frog.map(|entity| {
            with_frog(&self.world, entity, |fr| FrogDebug {
                x: r1(fr.position.x),
                y: r1(fr.position.y),
                health: r1(fr.health),
                max_health: fr.max_health,
                dead: fr.is_dead(),
                hopping: fr.hop_timer > 0.0,
            })
        });

        let mut engage: Vec<EngageDebug> = self
            .last_engage_targets
            .iter()
            .filter_map(|(&entity, &target)| {
                let mut q = self.world.query_one::<&Tank>(entity);
                q.get().ok().map(|t| EngageDebug { slot: t.owner_slot, x: r1(target.x), y: r1(target.y) })
            })
            .collect();
        engage.sort_by_key(|e| e.slot);

        DebugSnapshot {
            frame: self.frame,
            time: r1(self.time),
            seed: format!("{:#x}", self.round_seed),
            outcome: self.outcome,
            paused: self.paused,
            restart_timer: r1(self.restart_timer),
            width,
            height,
            tanks,
            projectiles,
            projectiles_total,
            pickups,
            frog,
            obstacles_alive: self.world.query::<&Obstacle>().iter().filter(|o| !o.destroyed).count(),
            engage,
        }
    }

    /// The tank entity in owner slot `slot` (0 = player), if any.
    pub fn tank_entity_by_slot(&self, slot: usize) -> Option<Entity> {
        self.world
            .query::<(Entity, &Tank)>()
            .iter()
            .find(|(_, t)| t.owner_slot == slot)
            .map(|(e, _)| e)
    }

    /// Move the tank in `slot` to `pos` (velocity zeroed) and optionally
    /// snap its facing to `rotation` degrees. Goes through the physics body
    /// - `sync_tanks_and_ram` would otherwise overwrite the position next
    /// frame - and re-orients the movement collider when the facing axis
    /// changed, the same way `drive_tank` does on a turn.
    pub fn debug_teleport(&mut self, slot: usize, pos: Position, rotation: Option<f32>) -> Result<(), String> {
        let entity = self.tank_entity_by_slot(slot).ok_or_else(|| format!("no tank in slot {slot}"))?;
        let (body, half_extents) = {
            let mut q = self.world.query_one::<&mut Tank>(entity);
            let tank = q.get().map_err(|e| e.to_string())?;
            tank.position = pos;
            tank.velocity = Vector2::new(0.0, 0.0);
            if let Some(rot) = rotation {
                tank.rotation = rot;
                tank.visual_rotation = rot;
                tank.turret_visual_rotation = rot;
            }
            let body = tank.body.expect("tank should always have a physics body once spawned");
            (body, tank.move_half_extents(tank.facing_along_x()))
        };
        self.physics.set_position(body, pos);
        let collider = self.physics.collider_of(body);
        self.physics.resize_collider(collider, half_extents);
        Ok(())
    }

    /// Overwrite the given fields of the tank in `slot`. Setting a special
    /// weapon's stock above zero also queues that weapon, exactly as
    /// collecting its pickup would, so it can actually fire.
    pub fn debug_set_tank(&mut self, slot: usize, patch: &TankPatch) -> Result<(), String> {
        let entity = self.tank_entity_by_slot(slot).ok_or_else(|| format!("no tank in slot {slot}"))?;
        let mut q = self.world.query_one::<&mut Tank>(entity);
        let tank = q.get().map_err(|e| e.to_string())?;
        if let Some(d) = patch.damage {
            tank.damage = d.clamp(0.0, MAX_DAMAGE);
        }
        if let Some(n) = patch.shells_ammo {
            tank.shells_ammo = n.max(0);
        }
        if let Some(n) = patch.minigun_ammo {
            tank.minigun_ammo = n.max(0);
            if n > 0 {
                tank.enqueue_weapon(ActiveWeapon::Minigun);
            }
        }
        if let Some(n) = patch.plasma_ammo {
            tank.plasma_ammo = n.max(0);
            if n > 0 {
                tank.enqueue_weapon(ActiveWeapon::Plasma);
            }
        }
        if let Some(n) = patch.laser_charges {
            tank.laser_charges = n.max(0);
            if n > 0 {
                tank.enqueue_weapon(ActiveWeapon::Laser);
            }
        }
        if let Some(t) = patch.shield_timer {
            tank.shield_timer = t.max(0.0);
        }
        if let Some(t) = patch.speed_boost_timer {
            tank.speed_boost_timer = t.max(0.0);
        }
        Ok(())
    }

    /// Queue the tank in `slot` to die at the top of the next playing
    /// frame - through `Frame::kills`, so it gets its shockwave, explosion
    /// splash, `Event::Wreck` and round-end check like any other kill.
    pub fn debug_kill(&mut self, slot: usize) -> Result<(), String> {
        let entity = self.tank_entity_by_slot(slot).ok_or_else(|| format!("no tank in slot {slot}"))?;
        if with_tank(&self.world, entity, Tank::is_wreck) {
            return Err(format!("slot {slot} is already a wreck"));
        }
        if !self.debug_kills.contains(&slot) {
            self.debug_kills.push(slot);
        }
        Ok(())
    }

    /// Apply `debug_kill` requests (first thing in a playing frame).
    pub(super) fn apply_debug_kills(&mut self, f: &mut Frame) {
        for slot in std::mem::take(&mut self.debug_kills) {
            let Some(entity) = self.tank_entity_by_slot(slot) else { continue };
            let mut q = self.world.query_one::<&mut Tank>(entity);
            let Ok(tank) = q.get() else { continue };
            if tank.is_wreck() {
                continue;
            }
            // A shield would absorb the blow.
            tank.shield_timer = 0.0;
            tank.take_damage(MAX_DAMAGE, MAX_DAMAGE);
            tank.mark_hit();
            f.kills.push((tank.position, slot != PLAYER_OWNER_SLOT, slot));
        }
    }

    /// Add an enemy at `pos` in the next free owner slot, facing down, with
    /// the same per-tank rolls `init` makes (damage variant, speed spread,
    /// track wobble) - **these draw from the round RNG**, so the round is
    /// no longer the seeded replay afterwards. `row` picks the chassis
    /// (0..12, default from the spawn order). Returns the new slot.
    pub fn debug_spawn_enemy(&mut self, pos: Position, row: Option<i32>) -> Result<usize, String> {
        let rng = self.rng.as_mut().ok_or_else(|| "spawn_enemy only works between frames".to_string())?;
        let max_slot = self.world.query::<&Tank>().iter().map(|t| t.owner_slot).max().unwrap_or(PLAYER_OWNER_SLOT);
        let slot = max_slot + 1;
        if slot > enemy_owner_slot(30) {
            return Err("owner slots are full (31 enemies)".to_string());
        }
        let row = row.unwrap_or(TANK_SPRITE_ORDER[(slot - 1) % TANK_SPRITE_ORDER.len()]);
        if !(0..TANK_VARIANTS).contains(&row) {
            return Err(format!("row {row} is outside 0..{TANK_VARIANTS}"));
        }
        let factor = 1.0 + rng.random_range(-tuning().enemy_speed_variance..tuning().enemy_speed_variance);
        let mut enemy = Tank {
            row,
            shell_variant: TANK_SHELL_VARIANT_BY_ROW[row as usize],
            damage_variant: rng.random_range(0..DAMAGE_VARIANTS),
            position: pos,
            rotation: 180.0,
            speed_scale: factor,
            owner_slot: slot,
            ..Tank::default()
        };
        roll_track_distortion(&mut enemy, rng);
        enemy.body = Some(self.physics.spawn_tank(pos, enemy.move_half_extents(false), enemy.mass()));
        self.world.spawn((enemy, Ai::default()));
        Ok(slot)
    }

    /// The nav grid as text, top row first: `#` blocked, `.` open, then
    /// what stands on each cell - `P` player, a digit or `E` for an enemy's
    /// slot, `x` a wreck, `F` the frog, `*` a pickup. Later markers win.
    pub fn nav_grid_ascii(&self, width: f32, height: f32) -> String {
        let grid = self.nav_grid(width, height);
        let (cols, rows, cell) = grid.dims();
        let mut cells: Vec<Vec<char>> = (0..rows)
            .map(|r| (0..cols).map(|c| if grid.is_blocked(c, r) { '#' } else { '.' }).collect())
            .collect();
        let mut mark = |p: Position, ch: char| {
            if p.x < 0.0 || p.y < 0.0 {
                return;
            }
            let (c, r) = ((p.x / cell) as usize, (p.y / cell) as usize);
            if c < cols && r < rows {
                cells[r][c] = ch;
            }
        };
        for p in self.world.query::<&Pickup>().iter() {
            mark(p.position, '*');
        }
        for fr in self.world.query::<&Frog>().iter() {
            mark(fr.position, 'F');
        }
        let mut tanks: Vec<(usize, Position, bool)> =
            self.world.query::<&Tank>().iter().map(|t| (t.owner_slot, t.position, t.is_wreck())).collect();
        tanks.sort_by_key(|t| std::cmp::Reverse(t.0));
        for (slot, pos, wreck) in tanks {
            let ch = if wreck {
                'x'
            } else if slot == PLAYER_OWNER_SLOT {
                'P'
            } else if slot < 10 {
                char::from(b'0' + slot as u8)
            } else {
                'E'
            };
            mark(pos, ch);
        }
        let mut out = String::with_capacity((cols + 1) * rows + 64);
        for row in cells {
            out.extend(row);
            out.push('\n');
        }
        out.push_str(&format!("{cols}x{rows} cells of {cell}px; # blocked . open P player 1-9/E enemy x wreck F frog * pickup\n"));
        out
    }
}
