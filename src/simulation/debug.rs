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

use super::engage::{EngageStatus, Rejections};
use super::{
    Frame, Game, Outcome, PLAYER_OWNER_SLOT, TANK_SPRITE_ORDER, TANK_VARIANTS, enemy_owner_slot, roll_track_distortion,
    with_frog, with_tank,
};

/// Two live enemies closer than this are "clustered" for `DebugSnapshot::clusters`
/// and the dev server's history aggregates - the probe's clustering anomaly
/// uses the same distance (its comment explains why 90 px is above the
/// ring's tightest legitimate pairing).
pub const CLUSTER_RADIUS_PX: f32 = 90.0;

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
    /// What the last enemy phase's engagement-slot assignment decided.
    pub engage: EngageSnapshot,
    /// Groups of two or more live enemies transitively within
    /// `CLUSTER_RADIUS_PX` of each other, as owner slots.
    pub clusters: Vec<Vec<usize>>,
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
    /// Distance to the closest other live enemy; live enemies only.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub nearest_ally_px: Option<f32>,
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
pub struct EngageSnapshot {
    /// False when fewer than two enemies were engaged, so no ring was built.
    pub built: bool,
    /// Every enemy, by owner slot.
    pub tanks: Vec<EngageTankDebug>,
    /// The 16 ring slots (`Full` only, and only when built).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub slots: Option<Vec<EngageSlotDebug>>,
}

#[derive(Serialize, Debug)]
pub struct EngageTankDebug {
    pub slot: usize,
    pub status: EngageStatus,
    /// Index into `slots` of the ring slot held; `None` = steering at the
    /// player directly.
    pub ring: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub x: Option<f32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub y: Option<f32>,
    /// Kept the slot from last frame without a new search.
    pub sticky: bool,
    /// Candidates the search passed over, by reason (`Full` only).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub rejected: Option<Rejections>,
}

#[derive(Serialize, Debug)]
pub struct EngageSlotDebug {
    pub i: u8,
    pub axis: &'static str,
    pub rank: u8,
    pub side: i8,
    /// `None` = off the map for this player position.
    pub x: Option<f32>,
    pub y: Option<f32>,
    /// Line of sight to the player; `None` = never checked this frame.
    pub los: Option<bool>,
    /// Owner slot of the tank holding it.
    pub claimed_by: Option<usize>,
}

/// One tank's per-frame row for the dev server's history ring - the few
/// fields worth accumulating over time, read cheaply enough to record on
/// every stepped frame.
#[derive(Clone, Serialize, Debug)]
pub struct TrackRow {
    pub slot: usize,
    pub x: f32,
    pub y: f32,
    pub action: Option<&'static str>,
    pub ring: Option<u8>,
    pub stuck: bool,
    pub touching_static: bool,
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

/// Groups of two or more tanks transitively within `radius` of each other
/// (union-find over the pairs), each group's slots ascending, groups by
/// their first slot.
pub fn clusters(tanks: &[(usize, Position)], radius: f32) -> Vec<Vec<usize>> {
    let n = tanks.len();
    let mut parent: Vec<usize> = (0..n).collect();
    fn find(parent: &mut [usize], i: usize) -> usize {
        let mut i = i;
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for a in 0..n {
        for b in a + 1..n {
            if tanks[a].1.distance_to(tanks[b].1) <= radius {
                let (ra, rb) = (find(&mut parent, a), find(&mut parent, b));
                if ra != rb {
                    parent[ra] = rb;
                }
            }
        }
    }
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut roots: Vec<usize> = Vec::new();
    for i in 0..n {
        let root = find(&mut parent, i);
        match roots.iter().position(|&r| r == root) {
            Some(g) => groups[g].push(tanks[i].0),
            None => {
                roots.push(root);
                groups.push(vec![tanks[i].0]);
            }
        }
    }
    groups.retain(|g| g.len() >= 2);
    for g in &mut groups {
        g.sort_unstable();
    }
    groups.sort_by_key(|g| g[0]);
    groups
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
        let live_enemies: Vec<(usize, Position)> = self
            .world
            .query::<(&Tank, &Ai)>()
            .iter()
            .filter(|(t, _)| !t.is_wreck())
            .map(|(t, _)| (t.owner_slot, t.position))
            .collect();
        let mut tanks: Vec<TankDebug> = self
            .world
            .query::<(&Tank, Option<&Ai>)>()
            .iter()
            .map(|(tank, ai)| {
                let body = tank.body.expect("tank should always have a physics body once spawned");
                let vel = self.physics.velocity(body);
                let nearest_ally_px = (ai.is_some() && !tank.is_wreck())
                    .then(|| {
                        live_enemies
                            .iter()
                            .filter(|(slot, _)| *slot != tank.owner_slot)
                            .map(|(_, p)| p.distance_to(tank.position))
                            .fold(f32::INFINITY, f32::min)
                    })
                    .filter(|d| d.is_finite())
                    .map(r1);
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
                    nearest_ally_px,
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

        let report = &self.last_engage;
        let owner_of = |entity: hecs::Entity| report.tanks.iter().find(|t| t.entity == entity).map(|t| t.owner);
        let engage = EngageSnapshot {
            built: report.built,
            tanks: report
                .tanks
                .iter()
                .map(|t| EngageTankDebug {
                    slot: t.owner,
                    status: t.status,
                    ring: t.slot.map(|s| s.index() as u8),
                    x: t.target.map(|p| r1(p.x)),
                    y: t.target.map(|p| r1(p.y)),
                    sticky: t.sticky,
                    rejected: (full && t.status == EngageStatus::Engaged).then_some(t.rejected),
                })
                .collect(),
            slots: (full && report.built).then(|| {
                report
                    .slots
                    .iter()
                    .enumerate()
                    .map(|(i, s)| {
                        let slot = super::engage::EngageSlot::from_index(i);
                        EngageSlotDebug {
                            i: i as u8,
                            axis: slot.axis_name(),
                            rank: slot.rank,
                            side: slot.side,
                            x: s.point.map(|p| r1(p.x)),
                            y: s.point.map(|p| r1(p.y)),
                            los: s.line_of_sight,
                            claimed_by: s.claimed_by.and_then(owner_of),
                        }
                    })
                    .collect()
            }),
        };

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
            clusters: clusters(&live_enemies, CLUSTER_RADIUS_PX),
        }
    }

    /// One `TrackRow` per live tank, sorted by owner slot - see `TrackRow`.
    pub fn debug_track_rows(&self) -> Vec<TrackRow> {
        let mut rows: Vec<TrackRow> = self
            .world
            .query::<(Entity, &Tank, Option<&Ai>)>()
            .iter()
            .filter(|(_, t, _)| !t.is_wreck())
            .map(|(entity, tank, ai)| {
                let body = tank.body.expect("tank should always have a physics body once spawned");
                let ai = ai.map(Ai::snapshot);
                TrackRow {
                    slot: tank.owner_slot,
                    x: r1(tank.position.x),
                    y: r1(tank.position.y),
                    action: ai.and_then(|a| a.last_action),
                    ring: self.last_engage.slot_of(entity).map(|s| s.index() as u8),
                    stuck: ai.is_some_and(|a| a.stuck_timer > 0.0),
                    touching_static: self.physics.contact_stats(body).touching_static,
                }
            })
            .collect();
        rows.sort_by_key(|r| r.slot);
        rows
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
            tank.ring_position = pos;
            tank.ring_velocity = Vector2::new(0.0, 0.0);
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
