//! The wire into a client replica (docs/online-coop-prd.md §4.5): `welcome`
//! builds a `Game` the way the server built its round - `Game::init` on
//! the same map, seed and overrides, so every roll `init` makes (wall and
//! prop variants, the frog's colour, the band's tanks) comes out the same -
//! then strips the enemies' `Ai` and applies the welcome's snapshot;
//! `snapshot` writes one authoritative snapshot into the entities, matched
//! by id. The replica is never `update`d: a tank in the snapshot but not
//! the world is spawned (an enemy with a `Tank` and no `Ai`, like a rolling
//! in wave tank; a seat with its chassis), a world tank missing from the
//! snapshot is removed, likewise projectiles by id; frogs, tiles, fires,
//! pickups and the round's scalars are overwritten. Tanks keep physics
//! bodies so the drawn hull follows the snapshot through the same
//! `Tank::position`/body pair the renderer reads. No RNG anywhere.
//!
//! What the wire does not say the replica fills from its own rules, all
//! cosmetic: a wreck's art variant and a spawned enemy's damage overlay
//! hash from the slot, a shot's shadow height from its id, a snapshot's
//! flag becomes the full timer it stands for (a boost, an afterburn, the
//! hit window), a frog's secondary clips run at full length, the round
//! clock is the tick at the fixed step. Between two snapshots the whole
//! picture is `Game::tick_presentation`'s, which is why a hull's *drawn*
//! angles are left where they stand here while its facing snaps.
//! `tuning_json` is the transport's to stage (`tuning::submit_json`,
//! applied at the frame boundary) before `welcome` runs, since `init`
//! reads the knobs.
//!
//! Two things a snapshot states by omission: a fire is out once the
//! list stops carrying its cell (burning out is the only way one ever
//! leaves), and the tile a `DrumLaunched` names is in the air rather than
//! anywhere on the field.

use std::collections::{BTreeMap, BTreeSet};

use hecs::Entity;

use crate::bullet::{Bullet, BulletState};
use crate::frog::{Facing, Frog, Side};
use crate::map::{self, MapFile};
use crate::math::Vec2;
use crate::net::PROTOCOL_VERSION;
use crate::net::encode::{cell_from_index, cell_index, field_cols};
use crate::net::events::WireEvent;
use crate::net::wire::{
    ShotKind, ShotState, Snapshot, TankState, Welcome, dequantise_heading, dequantise_pos, dequantise_seconds,
    dequantise_velocity, dir_from_index, frog_flags, tank_flags, tile_flags,
};
use crate::obstacle::{Drum, Fuse, Obstacle};
use crate::pickup::Pickup;
use crate::plasma::{Plasma, PlasmaState};
use crate::shell::{Owner, Shell, ShellState};
use crate::simulation::replica::plasma_variant_from_index;
use crate::simulation::{Game, GroundFire, PlayerCount};
use crate::tank::{ActiveWeapon, Dir, Tank};
use crate::tuning::tuning;
use crate::{DAMAGE_VARIANTS, MAX_DAMAGE, PHYSICS_FIXED_DT, Position, TANK_SHELL_VARIANT_BY_ROW, TANK_WRECK_COLS};

/// The owner a replica's projectile carries: the wire names none, and the
/// replica never resolves a hit, so the slot only has to be one no tank
/// ever holds.
const REPLICA_OWNER: Owner = Owner::Enemy(usize::MAX);

/// A replica of the round `w` describes, at the tick its snapshot was
/// cut. Refuses a welcome from another protocol version or with a map
/// that does not parse.
pub fn welcome(w: &Welcome) -> Result<Game, String> {
    if w.protocol != PROTOCOL_VERSION {
        return Err(format!("server speaks protocol {} but this build speaks {PROTOCOL_VERSION}", w.protocol));
    }
    let mut game = Game::default();
    game.map = MapFile::from_toml_str(&w.map_toml)?;
    game.seed_override = Some(w.seed);
    game.level_overrides = w.overrides.into();
    game.enemy_count_override = w.enemy_count.map(|n| n as usize);
    // As many seats as the roster's highest, so the replica's enemies
    // count from the same slot the room's do.
    let seats = w.roster.iter().map(|s| s.seat as usize + 1).max().unwrap_or(1);
    game.players = PlayerCount::from_count(seats).unwrap_or(PlayerCount::MAX);
    let chassis = |seat: u8| w.roster.iter().find(|s| s.seat == seat).map(|s| s.chassis as i32);
    game.player_row_override = chassis(0);
    game.player2_row_override = chassis(1);
    // The banner's time left travels in the snapshot.
    game.show_intro = false;
    let (width, height) = game.map.field_size();
    game.init(width, height);
    game.strip_ai();
    let cols = field_cols(&game);
    game.oil_cells = w.oil_cells.iter().map(|&i| cell_from_index(cols, i)).collect();
    remove_tiles(&mut game, &w.dead_cells.iter().copied().collect(), cols);
    snapshot(&mut game, &w.snapshot);
    Ok(game)
}

/// Take the tiles in `dead` out of the world, bodies included, and recap
/// the walls around the holes.
fn remove_tiles(game: &mut Game, dead: &BTreeSet<u16>, cols: u16) {
    if dead.is_empty() {
        return;
    }
    let gone: Vec<(Entity, _)> = game
        .world
        .query::<(Entity, &Obstacle)>()
        .iter()
        .filter(|(_, o)| dead.contains(&cell_index(cols, o.cell())))
        .map(|(e, o)| (e, o.body))
        .collect();
    let removed = !gone.is_empty();
    for (entity, body) in gone {
        game.physics_mut().remove_body(body);
        game.world.despawn(entity).ok();
    }
    if removed {
        game.refresh_edge_masks();
    }
}

/// Write `s` into `game`: the frame's events first (a tile death there
/// removes the tile), then every family.
pub fn snapshot(game: &mut Game, s: &Snapshot) {
    let cols = field_cols(game);
    let dead_tiles = apply_events(game, s, cols);
    apply_tiles(game, s, cols, dead_tiles);
    apply_tanks(game, s);
    apply_shots(game, s);
    apply_frogs(game, s);
    apply_pickups(game, s, cols);
    apply_fires(game, s, cols);
    apply_round(game, s);
    game.frame = s.tick as u64;
    game.time = s.tick as f32 * PHYSICS_FIXED_DT;
}

/// Hand the frame's events to the replica's presentation (`fx.rs` reads
/// `Game::events`) and collect the cells whose tile died. A `RoundStarted`
/// among them is the server's round restarting (the end screen ran out):
/// the replica starts over the same way, `init` on the event's seed, since
/// a snapshot alone cannot put back what the old round destroyed. A
/// welcome's frame-0 snapshot carries the same event for the round
/// `welcome` has just built, so a replica already standing at frame 0 on
/// that seed keeps the world it has instead of building an identical one.
///
/// A `DrumLaunched` puts the drum in the air here, since a snapshot has
/// no family for something that is neither a tile nor a shot; its arc is
/// derived from an age `tick_presentation` advances, and the blast it
/// sets off when it lands arrives as the server's own `Blast`.
fn apply_events(game: &mut Game, s: &Snapshot, cols: u16) -> BTreeSet<u16> {
    let restarted = s.events.iter().rev().find_map(|e| match *e {
        WireEvent::RoundStarted { seed, .. } => Some(seed),
        _ => None,
    });
    if let Some(seed) = restarted {
        if game.frame() != 0 || game.round_seed() != seed {
            game.seed_override = Some(seed);
            let (width, height) = game.map.field_size();
            game.init(width, height);
            game.strip_ai();
        }
    }
    let mut dead = BTreeSet::new();
    for event in &s.events {
        match *event {
            WireEvent::ObstacleDestroyed { x, y, .. } => {
                let at = Position::new(dequantise_pos(x), dequantise_pos(y));
                dead.insert(cell_index(cols, map::world_to_cell(at)));
            }
            WireEvent::DrumLaunched { x, y, to_x, to_y } => {
                let from = Position::new(dequantise_pos(x), dequantise_pos(y));
                let to = Position::new(dequantise_pos(to_x), dequantise_pos(to_y));
                // Only a fuel drum ever launches (`props::tick_fuses`).
                game.drum_in_flight(from, to, Drum::Fuel as i32);
            }
            _ => {}
        }
    }
    game.events = s.events.iter().filter_map(WireEvent::to_event).collect();
    dead
}

fn apply_tiles(game: &mut Game, s: &Snapshot, cols: u16, mut dead: BTreeSet<u16>) {
    let listed: BTreeMap<u16, _> = s
        .tiles
        .iter()
        .inspect(|t| {
            if t.flags & tile_flags::DESTROYED != 0 {
                dead.insert(t.cell);
            }
        })
        .filter(|t| t.flags & tile_flags::DESTROYED == 0)
        .map(|t| (t.cell, *t))
        .collect();
    remove_tiles(game, &dead, cols);
    let live: Vec<(Entity, u16)> = game
        .world
        .query::<(Entity, &Obstacle)>()
        .iter()
        .filter(|(_, o)| !o.destroyed)
        .map(|(e, o)| (e, cell_index(cols, o.cell())))
        .collect();
    for (entity, cell) in live {
        let mut q = game.world.query_one::<&mut Obstacle>(entity);
        let Ok(o) = q.get() else { continue };
        match listed.get(&cell) {
            Some(t) => {
                o.health = t.hp as f32;
                o.burning = t.flags & tile_flags::BURNING != 0;
                let fused = t.flags & tile_flags::FUSED != 0;
                if o.burning || fused {
                    o.health = 0.0;
                }
                if fused && o.fuse.is_none() {
                    let total = tuning().barrel_fuse_seconds;
                    o.fuse = Some(Fuse { left: total, total, from: None });
                } else if !fused {
                    o.fuse = None;
                }
                o.scorched = t.faces;
                o.set_lean_strength((t.flags & tile_flags::LEAN_MASK) >> (tile_flags::LEAN_SHIFT + 2));
            }
            // Absent from the list: the tile is as the map made it.
            None => {
                o.health = o.max_health;
                o.burning = false;
                o.fuse = None;
                o.scorched = 0;
                o.ram_timer = 0.0;
            }
        }
    }
}

fn apply_tanks(game: &mut Game, s: &Snapshot) {
    let first_enemy = game.first_enemy_slot();
    let by_slot: BTreeMap<usize, Entity> =
        game.world.query::<(Entity, &Tank)>().iter().map(|(e, t)| (t.owner_slot(), e)).collect();
    let wanted: BTreeSet<usize> = s.tanks.iter().map(|t| t.id as usize).collect();
    // A seat's tank never leaves the picture: the lobby, not the
    // snapshot, says who is playing.
    for (&slot, &entity) in by_slot.iter().filter(|(slot, _)| **slot >= first_enemy) {
        if !wanted.contains(&slot) {
            let body = game.world.get::<&Tank>(entity).ok().and_then(|t| t.body);
            if let Some(body) = body {
                game.physics_mut().remove_body(body);
            }
            game.world.despawn(entity).ok();
        }
    }
    for t in &s.tanks {
        let slot = t.id as usize;
        let entity = match by_slot.get(&slot) {
            Some(&e) => e,
            None => spawn_tank(game, t, slot < first_enemy),
        };
        write_tank(game, entity, t);
    }
}

/// A fresh tank for `t`, a seat's or an enemy's: `Tank` and body, no
/// `Ai`. The cosmetic rolls `init` would have made hash from the slot.
fn spawn_tank(game: &mut Game, t: &TankState, player: bool) -> Entity {
    let slot = t.id as usize;
    let owner = if player { Owner::Player(slot as u8) } else { Owner::Enemy(slot) };
    let row = (t.row as i32).clamp(0, TANK_SHELL_VARIANT_BY_ROW.len() as i32 - 1);
    let rotation = dir_from_index(t.dir).unwrap_or(Dir::Up).rotation();
    let position = Position::new(dequantise_pos(t.x), dequantise_pos(t.y));
    let mut tank = Tank {
        row,
        shell_variant: TANK_SHELL_VARIANT_BY_ROW[row as usize],
        damage_variant: (slot % DAMAGE_VARIANTS as usize) as i32,
        position,
        rotation,
        visual_rotation: rotation,
        turret_visual_rotation: rotation,
        ring_position: position,
        owner,
        ..Tank::default()
    };
    let half = tank.move_half_extents(tank.facing_along_x());
    let mass = tank.mass();
    tank.body = Some(game.physics_mut().spawn_tank(position, half, mass));
    let entity = game.world.spawn((tank,));
    if player {
        if let Some(held) = game.seats.get_mut(slot) {
            *held = Some(entity);
        }
    }
    entity
}

/// Write `t` into the tank `entity` and put its body where the hull is.
fn write_tank(game: &mut Game, entity: Entity, t: &TankState) {
    let position = Position::new(dequantise_pos(t.x), dequantise_pos(t.y));
    let velocity = Position::new(dequantise_velocity(t.vx), dequantise_velocity(t.vy));
    let rotation = dir_from_index(t.dir).unwrap_or(Dir::Up).rotation();
    let on = |bit: u8| t.flags & bit != 0;
    let (body, half_extents, turned) = {
        let mut q = game.world.query_one::<&mut Tank>(entity);
        let Ok(tank) = q.get() else { return };
        tank.position = position;
        // The hull's facing snaps the way `Tank::control` snaps it; the
        // drawn angles stay where they are and swing across in
        // `Game::tick_presentation`, which is the turn the player sees.
        let turned = tank.rotation != rotation;
        tank.rotation = rotation;
        let wreck = on(tank_flags::WRECK);
        tank.damage = if wreck { MAX_DAMAGE } else { MAX_DAMAGE - t.hp as f32 };
        if wreck && tank.wreck_col.is_none() {
            tank.wreck_col = Some(TANK_WRECK_COLS[t.id as usize % TANK_WRECK_COLS.len()]);
        }
        tank.shield_hp = if on(tank_flags::SHIELD) { t.shield.max(1) as f32 } else { 0.0 };
        let knobs = tuning();
        set_timer(&mut tank.speed_boost_timer, on(tank_flags::BOOST), knobs.speed_boost_duration_seconds);
        set_timer(&mut tank.burn_timer, on(tank_flags::BURNING), knobs.flame_afterburn_seconds);
        set_timer(&mut tank.hit_flash_timer, on(tank_flags::HIT), knobs.health_ring_hit_seconds);
        tank.flame_held = on(tank_flags::FLAME);
        let weapon: ActiveWeapon = t.weapon.into();
        tank.weapon_queue = if weapon == ActiveWeapon::Shell { Vec::new() } else { vec![weapon] };
        let ammo = t.ammo as i32;
        match weapon {
            ActiveWeapon::Shell => tank.shells_ammo = ammo,
            ActiveWeapon::Laser => tank.laser_charges = ammo,
            ActiveWeapon::Plasma => tank.plasma_ammo = ammo,
            ActiveWeapon::Minigun => tank.minigun_ammo = ammo,
            ActiveWeapon::Flamethrower => tank.flame_fuel = ammo as f32,
        }
        (tank.body, tank.move_half_extents(tank.facing_along_x()), turned)
    };
    let Some(body) = body else { return };
    let physics = game.physics_mut();
    physics.set_position(body, position);
    physics.set_velocity(body, velocity);
    if turned {
        let collider = physics.collider_of(body);
        physics.resize_collider(collider, half_extents);
    }
}

/// A flag into the timer it stands for: a running window keeps running
/// (or starts at its full length), a cleared one stops.
fn set_timer(timer: &mut f32, on: bool, full: f32) {
    if !on {
        *timer = 0.0;
    } else if *timer <= 0.0 {
        *timer = full.max(f32::EPSILON);
    }
}

fn apply_shots(game: &mut Game, s: &Snapshot) {
    let mut existing: BTreeMap<u16, (ShotKind, Entity)> = BTreeMap::new();
    for (e, sh) in game.world.query::<(Entity, &Shell)>().iter() {
        existing.insert((sh.id & 0xFFFF) as u16, (ShotKind::Shell, e));
    }
    for (e, b) in game.world.query::<(Entity, &Bullet)>().iter() {
        existing.insert((b.id & 0xFFFF) as u16, (ShotKind::Bullet, e));
    }
    for (e, p) in game.world.query::<(Entity, &Plasma)>().iter() {
        existing.insert((p.id & 0xFFFF) as u16, (ShotKind::Plasma, e));
    }
    let wanted: BTreeMap<u16, &ShotState> = s.shots.iter().map(|sh| (sh.id, sh)).collect();
    for (id, (kind, entity)) in &existing {
        if wanted.get(id).is_none_or(|sh| sh.kind != *kind) {
            game.world.despawn(*entity).ok();
        }
    }
    for (id, sh) in wanted {
        let position = Position::new(dequantise_pos(sh.x), dequantise_pos(sh.y));
        let rotation = dequantise_heading(sh.heading);
        let rad = rotation.to_radians();
        let dir = Vec2::new(rad.sin(), -rad.cos());
        let knobs = tuning();
        match existing.get(&id) {
            Some(&(kind, entity)) if kind == sh.kind => match sh.kind {
                ShotKind::Shell => {
                    let mut q = game.world.query_one::<&mut Shell>(entity);
                    if let Ok(shell) = q.get() {
                        shell.prev_position = shell.position;
                        shell.position = position;
                        shell.rotation = rotation;
                        shell.velocity = dir * knobs.shell_speed;
                        let state = ShellState::from_col(sh.state as i32).unwrap_or(ShellState::Flying);
                        if shell.state != state {
                            shell.state = state;
                            shell.timer = 0.0;
                        }
                    }
                }
                ShotKind::Bullet => {
                    let mut q = game.world.query_one::<&mut Bullet>(entity);
                    if let Ok(bullet) = q.get() {
                        bullet.prev_position = bullet.position;
                        bullet.position = position;
                        bullet.rotation = rotation;
                        bullet.velocity = dir * knobs.minigun_bullet_speed;
                        let state = BulletState::from_col(sh.state as i32).unwrap_or(BulletState::Flying);
                        if bullet.state != state {
                            bullet.state = state;
                            bullet.timer = 0.0;
                        }
                    }
                }
                ShotKind::Plasma => {
                    let mut q = game.world.query_one::<&mut Plasma>(entity);
                    if let Ok(plasma) = q.get() {
                        plasma.prev_position = plasma.position;
                        plasma.position = position;
                        plasma.rotation = rotation;
                        plasma.velocity = dir * knobs.plasma_speed;
                        let state = PlasmaState::from_col(sh.state as i32).unwrap_or(PlasmaState::Flying);
                        if plasma.state != state {
                            plasma.state = state;
                            plasma.timer = 0.0;
                        }
                    }
                }
            },
            _ => spawn_shot(game, sh, position, rotation, dir),
        }
    }
}

/// A shot's shadow height, hashed from its id between the kind's knobs:
/// the server rolled one, the picture only needs variety.
fn shadow_offset(id: u16, min: f32, max: f32) -> f32 {
    let h = (id as u32).wrapping_mul(2_654_435_761) >> 22;
    min + (max - min) * (h % 1000) as f32 / 1000.0
}

fn spawn_shot(game: &mut Game, sh: &ShotState, position: Position, rotation: f32, dir: Vec2) {
    let knobs = tuning();
    let id = sh.id as u32;
    match sh.kind {
        ShotKind::Shell => {
            game.world.spawn((Shell {
                state: ShellState::from_col(sh.state as i32).unwrap_or(ShellState::Flying),
                position,
                velocity: dir * knobs.shell_speed,
                rotation,
                timer: 0.0,
                done: false,
                owner: REPLICA_OWNER,
                variant: sh.variant as i32,
                shooter_row: 0,
                shadow_offset: shadow_offset(sh.id, knobs.shell_shadow_offset_min, knobs.shell_shadow_offset_max),
                prev_position: position,
                bounces_left: 0,
                passed_over: Vec::new(),
                id,
            },));
        }
        ShotKind::Bullet => {
            game.world.spawn((Bullet {
                state: BulletState::from_col(sh.state as i32).unwrap_or(BulletState::Flying),
                position,
                velocity: dir * knobs.minigun_bullet_speed,
                rotation,
                timer: 0.0,
                done: false,
                owner: REPLICA_OWNER,
                shooter_row: 0,
                shadow_offset: shadow_offset(
                    sh.id,
                    knobs.minigun_bullet_shadow_offset_min,
                    knobs.minigun_bullet_shadow_offset_max,
                ),
                prev_position: position,
                passed_over: Vec::new(),
                id,
            },));
        }
        ShotKind::Plasma => {
            game.world.spawn((Plasma {
                state: PlasmaState::from_col(sh.state as i32).unwrap_or(PlasmaState::Flying),
                position,
                velocity: dir * knobs.plasma_speed,
                rotation,
                timer: 0.0,
                done: false,
                owner: REPLICA_OWNER,
                variant: plasma_variant_from_index(sh.variant as i32),
                shooter_row: 0,
                shadow_offset: shadow_offset(sh.id, knobs.plasma_shadow_offset_min, knobs.plasma_shadow_offset_max),
                prev_position: position,
                passed_over: Vec::new(),
                id,
            },));
        }
    }
}

fn apply_frogs(game: &mut Game, s: &Snapshot) {
    for f in &s.frogs {
        let position = Position::new(dequantise_pos(f.x), dequantise_pos(f.y));
        let entity = match f.side {
            Side::Player => game.frog,
            Side::Enemy => game.enemy_frog,
        };
        let entity = match entity {
            Some(e) => e,
            None => {
                let e = game.spawn_frog(f.side, position, 0);
                match f.side {
                    Side::Player => game.frog = Some(e),
                    Side::Enemy => game.enemy_frog = Some(e),
                }
                e
            }
        };
        let on = |bit: u8| f.state & bit != 0;
        let body = {
            let mut q = game.world.query_one::<&mut Frog>(entity);
            let Ok(frog) = q.get() else { continue };
            frog.position = position;
            let dead = on(frog_flags::DEAD);
            let hopping = on(frog_flags::HOPPING);
            frog.health = if dead { 0.0 } else { f.hp.max(1) as f32 };
            if hopping {
                frog.hop_start = position;
                frog.hop_end = Position::new(dequantise_pos(f.hop_x), dequantise_pos(f.hop_y));
                if let Some(facing) = Facing::from_dx(frog.hop_end.x - position.x) {
                    frog.facing = facing;
                }
            }
            frog.set_clips(dead, hopping, on(frog_flags::BITING), on(frog_flags::HURT), f.phase);
            frog.body
        };
        game.physics_mut().set_position(body, position);
    }
}

fn apply_pickups(game: &mut Game, s: &Snapshot, cols: u16) {
    let old: Vec<Entity> = game.world.query::<(Entity, &Pickup)>().iter().map(|(e, _)| e).collect();
    for entity in old {
        game.world.despawn(entity).ok();
    }
    let slots: Vec<_> = game.pickup_slots().to_vec();
    for (i, (position, kind)) in slots.into_iter().enumerate().take(64) {
        if s.pickups & (1 << i) != 0 {
            game.world.spawn((Pickup { kind, position },));
        }
    }
    for bonus in &s.bonus_pickups {
        let (col, row) = cell_from_index(cols, bonus.cell);
        game.world.spawn((Pickup { kind: bonus.kind, position: map::cell_to_world(col, row) },));
    }
}

fn apply_fires(game: &mut Game, s: &Snapshot, cols: u16) {
    let trail_seconds = tuning().oil_trail_burn_seconds;
    let fires: Vec<GroundFire> = s
        .fires
        .iter()
        .map(|f| {
            let cell = cell_from_index(cols, f.cell);
            let left = dequantise_seconds(f.left);
            let known = game.fires.iter().find(|g| g.cell == cell);
            GroundFire {
                cell,
                left,
                total: known.map_or(left.max(trail_seconds), |g| g.total),
                spread_at: None,
                pool: known.map_or(!game.oil_cells.contains(&cell), |g| g.pool),
            }
        })
        .collect();
    // Grass in a burning cell chars, as `tick_fires` does on the server.
    let burning: BTreeSet<(i32, i32)> = fires.iter().map(|f| f.cell).collect();
    if !burning.is_empty() {
        game.grass_cells.retain(|c| !burning.contains(&map::world_to_cell(*c)));
        for tuft in game.grass.iter_mut() {
            if burning.contains(&map::world_to_cell(tuft.base)) {
                tuft.burnt = true;
            }
        }
    }
    // A cell only ever leaves the list by burning out, so the snapshot
    // dropping it is the event: darker ground, the oil spent and a charred
    // plank, once each (docs/online-coop-prd.md section 4.5).
    let out: Vec<GroundFire> = game.fires.iter().filter(|f| !burning.contains(&f.cell)).copied().collect();
    for fire in out {
        if let Some(decal) = game.fire_burnt_out(&fire) {
            game.push_decal(decal);
        }
    }
    game.fires = fires;
}

fn apply_round(game: &mut Game, s: &Snapshot) {
    game.intro_timer = dequantise_seconds(s.round.intro);
    // The end screen's countdown is the server's: the replica takes the
    // number rather than running a clock of its own, so the two screens
    // read the same second and the restart lands with the round the
    // server starts.
    game.restart_timer = dequantise_seconds(s.round.restart);
    game.outcome = s.round.outcome.into();
    // Zero tenths of breather is no breather: the banner is either up or
    // it is not, and a last twentieth of a second of it changes nothing.
    let next_in = (s.round.next_wave > 0).then(|| dequantise_seconds(s.round.next_wave));
    game.set_wave_progress(s.round.wave as u32, s.round.pending as usize, next_in);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ai::Intent;
    use crate::level::{Mission, SpawnKind};
    use crate::net::MAX_SEATS;
    use crate::net::codec::{Msg, decode, encode};
    use crate::net::encode as enc;
    use crate::net::wire::Seat;
    use crate::simulation::Input;
    use crate::simulation::replica::DrawableState;

    const DEFAULT_MAP: &str = include_str!("../../maps/default.toml");
    const PROPS_MAP: &str = include_str!("../../maps/test/props.toml");

    /// A seeded band round with the seat's chassis pinned, as a room
    /// server runs one.
    fn authoritative(map: &str, seed: u64, enemies: usize) -> Game {
        let mut game = Game::default();
        game.seed_override = Some(seed);
        game.enemy_count_override = Some(enemies);
        game.player_row_override = Some(3);
        game.level_overrides.spawn = Some(SpawnKind::Band);
        game.map = MapFile::from_toml_str(map).expect("map parses");
        let (width, height) = game.map.field_size();
        game.init(width, height);
        game
    }

    /// A scripted drive: a new heading every 40 frames, the trigger every
    /// twelfth frame.
    fn intent(frame: u32) -> Intent {
        Intent { move_dir: Some(Dir::ALL[(frame / 40) as usize % 4]), fire: frame % 12 == 0, ..Intent::default() }
    }

    fn step(game: &mut Game, frame: u32) {
        let (width, height) = game.map.field_size();
        game.update(Input::single(intent(frame)), PHYSICS_FIXED_DT, width, height);
    }

    fn roster(game: &Game) -> Vec<Seat> {
        let chassis = game.player_chassis().expect("a running round has a player").row() as u8;
        vec![Seat { seat: 0, nick: "host".into(), chassis }]
    }

    fn welcome_through_the_codec(game: &Game) -> Game {
        let w = enc::welcome(game, 0, roster(game), "{}".into(), [0; MAX_SEATS]).expect("the map serialises");
        let Msg::Welcome(w) = decode(&encode(&Msg::Welcome(w))).expect("a welcome decodes") else { panic!("kind") };
        welcome(&w).expect("a welcome builds a replica")
    }

    /// The cosmetic state a replica carries forward on its own between two
    /// snapshots, sorted by key: the drawn hull angles, the burning tiles'
    /// flicker frames, the tread marks on the ground and the ages of the
    /// rubble in flight.
    #[derive(Default, PartialEq, Debug)]
    struct Cosmetics {
        angles: Vec<(usize, i32)>,
        burn_frames: Vec<((i32, i32), i32)>,
        marks: usize,
        decal_ages: Vec<i32>,
    }

    fn cosmetics(game: &Game) -> Cosmetics {
        let mut angles: Vec<(usize, i32)> = game
            .world
            .query::<&Tank>()
            .iter()
            .map(|t| (t.owner_slot(), (t.visual_rotation * 1000.0) as i32))
            .collect();
        angles.sort();
        let mut burn_frames: Vec<((i32, i32), i32)> = game
            .world
            .query::<&Obstacle>()
            .iter()
            .filter(|o| o.burning && !o.destroyed)
            .map(|o| (o.cell(), o.burn_frame))
            .collect();
        burn_frames.sort();
        Cosmetics {
            angles,
            burn_frames,
            marks: game.tracks.len(),
            decal_ages: game.decals.iter().map(|d| (d.age * 1000.0) as i32).collect(),
        }
    }

    /// The families a run touched, so a test can say it exercised them.
    #[derive(Default, Debug)]
    struct Seen {
        wrecks: usize,
        shots: usize,
        changed_tiles: usize,
        fires: usize,
        pickups_taken: usize,
        frog_clips: usize,
        tiles_gone: usize,
        /// Frames on the end screen.
        ended: usize,
        /// Frames between snapshots on which the replica's own tick moved
        /// a hull's drawn angle, a tile's flicker, the tread marks or a
        /// piece of rubble in flight.
        eased: usize,
        flickered: usize,
        marked: usize,
        aged: usize,
    }

    impl Seen {
        fn record(&mut self, state: &DrawableState, first: &DrawableState) {
            self.wrecks += state.tanks.iter().filter(|t| t.wreck).count();
            self.shots += state.shots.len();
            self.changed_tiles += state
                .tiles
                .iter()
                .filter(|t| t.hp != t.material.max_health().round() as u8 || t.burning || t.fused || t.scorched != 0 || t.lean != 0)
                .count();
            self.fires += state.fires.len();
            self.pickups_taken += first.pickups.len().saturating_sub(state.pickups.len());
            self.frog_clips += state.frogs.iter().filter(|f| f.hopping || f.hurt || f.biting).count();
            self.tiles_gone += first.tiles.len().saturating_sub(state.tiles.len());
            self.ended += usize::from(state.outcome != crate::simulation::Outcome::Playing);
        }

        fn record_tick(&mut self, before: &Cosmetics, after: &Cosmetics) {
            self.eased += usize::from(before.angles != after.angles);
            self.flickered += usize::from(before.burn_frames != after.burn_frames);
            self.marked += usize::from(before.marks != after.marks);
            self.aged += usize::from(before.decal_ages != after.decal_ages);
        }
    }

    /// The state-only part of a snapshot: the frame's events and the tile
    /// deaths cut from them differ between a snapshot cut on the server
    /// and one re-encoded from the replica (which holds the interval's
    /// events, not the frame's).
    fn state_only(mut s: Snapshot) -> Snapshot {
        s.events.clear();
        s.tiles.retain(|t| t.flags & tile_flags::DESTROYED == 0);
        s
    }

    /// Run a round for `frames` frames, snapshot every third one (the
    /// interval's events accumulated the way a room server does), build
    /// the replica from a welcome at frame 60 and apply every later
    /// snapshot; on the two frames in between the replica runs
    /// `tick_presentation`, as a client does. After each apply the
    /// replica's picture must equal the round's, and re-encoding the
    /// replica must give the server's bytes - so nothing the replica
    /// animates on its own drifts out of what the wire pins.
    fn round_trip(map: &str, seed: u64, frames: u32, poke: impl Fn(&mut Game, u32)) -> Seen {
        let mut game = authoritative(map, seed, 6);
        let first = game.drawable_state();
        let mut replica: Option<Game> = None;
        let mut interval: Vec<WireEvent> = Vec::new();
        let mut seen = Seen::default();
        for frame in 1..=frames {
            poke(&mut game, frame);
            step(&mut game, frame);
            interval.extend(enc::wire_events(game.events()));
            if frame % 3 != 0 {
                if let Some(replica) = replica.as_mut() {
                    let before = cosmetics(replica);
                    replica.tick_presentation(PHYSICS_FIXED_DT);
                    seen.record_tick(&before, &cosmetics(replica));
                }
                continue;
            }
            let mut snap = enc::snapshot(&game, [0; MAX_SEATS]);
            snap.events = std::mem::take(&mut interval);
            let replica = match replica.as_mut() {
                None if frame >= 60 => replica.insert(welcome_through_the_codec(&game)),
                None => continue,
                Some(r) => {
                    let Msg::Snapshot(snap) = decode(&encode(&Msg::Snapshot(snap.clone()))).expect("decodes") else {
                        panic!("kind")
                    };
                    snapshot(r, &snap);
                    r
                }
            };
            let expected = game.drawable_state();
            assert_eq!(replica.drawable_state(), expected, "frame {frame}: the replica draws another picture");
            assert_eq!(
                state_only(enc::snapshot(replica, [0; MAX_SEATS])),
                state_only(snap),
                "frame {frame}: re-encoding the replica gives other bytes"
            );
            seen.record(&expected, &first);
        }
        seen
    }

    #[test]
    fn the_replica_draws_the_default_map_round() {
        let seen = round_trip(DEFAULT_MAP, 0xB0B5, 600, |game, frame| {
            if frame == 300 {
                game.debug_kill(2).expect("enemy in slot 2");
            }
        });
        assert!(seen.shots > 0, "{seen:?}: nobody fired");
        assert!(seen.wrecks > 0, "{seen:?}: no wreck");
        assert!(seen.changed_tiles > 0, "{seen:?}: no tile was hit");
        assert!(seen.eased > 0, "{seen:?}: no hull swung its drawn angle between snapshots");
        assert!(seen.marked > 0, "{seen:?}: no tread mark was pressed between snapshots");
    }

    #[test]
    fn the_replica_draws_the_props_round_with_fires_and_a_launched_drum() {
        let seen = round_trip(PROPS_MAP, 0xC0FFEE, 900, |game, frame| {
            // The oil drum at the head of the trail: the fire runs the
            // trail to the fuel drum, which launches and blasts.
            if frame == 120 {
                game.debug_detonate(map::cell_to_world(17, 6)).expect("the oil drum at (17,6)");
            }
            if frame == 400 {
                game.debug_kill(1).expect("enemy in slot 1");
            }
        });
        assert!(seen.shots > 0, "{seen:?}: nobody fired");
        assert!(seen.wrecks > 0, "{seen:?}: no wreck");
        assert!(seen.fires > 0, "{seen:?}: nothing burned");
        assert!(seen.tiles_gone > 0, "{seen:?}: no tile died");
        assert!(seen.changed_tiles > 0, "{seen:?}: no tile changed");
        // The round ends and restarts inside these 900 frames, so the end
        // screen and the `RoundStarted` re-init are covered too.
        assert!(seen.ended > 0, "{seen:?}: the round never ended");
        assert!(seen.eased > 0, "{seen:?}: no hull swung its drawn angle between snapshots");
        assert!(seen.aged > 0, "{seen:?}: no rubble aged between snapshots");
    }

    /// An `Obstacle` the replica has lost stays lost when the frame-0
    /// snapshot that built it is applied again: its `RoundStarted` names
    /// the round the replica is already standing in, and re-initing it
    /// would only build an identical world. A snapshot naming another seed
    /// is a real restart and does re-init.
    #[test]
    fn a_welcome_snapshot_does_not_rebuild_the_round_it_just_built() {
        let game = authoritative(DEFAULT_MAP, 0x5EED, 4);
        assert!(
            game.events().iter().any(|e| matches!(e, crate::simulation::Event::RoundStarted { .. })),
            "a round at frame 0 announces itself"
        );
        let snap = enc::snapshot(&game, [0; MAX_SEATS]);
        let mut replica = welcome_through_the_codec(&game);
        let doomed = replica.world.query::<(Entity, &Obstacle)>().iter().map(|(e, _)| e).next().expect("a tile");
        replica.world.despawn(doomed).ok();
        let tiles = replica.world.query::<&Obstacle>().iter().count();
        snapshot(&mut replica, &snap);
        assert_eq!(replica.world.query::<&Obstacle>().iter().count(), tiles, "the round was not rebuilt");
        assert_eq!(replica.round_seed(), game.round_seed());

        let other = authoritative(DEFAULT_MAP, 0xD1FF, 4);
        snapshot(&mut replica, &enc::snapshot(&other, [0; MAX_SEATS]));
        assert_eq!(replica.round_seed(), other.round_seed(), "another seed is a restart");
    }

    /// The end screen's countdown belongs to the server: a replica takes
    /// the number off every snapshot and holds it in between, so the two
    /// screens read the same second and the restart lands with the round
    /// the server starts, rather than the replica counting down from zero.
    #[test]
    fn the_end_screens_countdown_travels() {
        let mut game = authoritative(DEFAULT_MAP, 0x5EED, 1);
        assert_eq!(enc::snapshot(&game, [0; MAX_SEATS]).round.restart, 0, "a live round has no countdown");
        game.debug_kill(0).expect("the player is in slot 0");
        let mut frame = 0;
        while game.outcome() == crate::simulation::Outcome::Playing {
            frame += 1;
            step(&mut game, frame);
            assert!(frame < 60, "the player's death never ended the round");
        }
        // The welcome's own snapshot puts the joiner on the end screen
        // with the seconds the server has left, not with a fresh clock.
        let mut replica = welcome_through_the_codec(&game);
        let started = replica.drawable_state().restart_tenths;
        assert!(started > 0, "the replica joined the end screen with no countdown");
        assert_eq!(replica.drawable_state(), game.drawable_state());

        let mut seen = vec![started];
        for _ in 0..40 {
            for _ in 0..3 {
                frame += 1;
                step(&mut game, frame);
            }
            let snap = enc::snapshot(&game, [0; MAX_SEATS]);
            let Msg::Snapshot(snap) = decode(&encode(&Msg::Snapshot(snap))).expect("decodes") else { panic!("kind") };
            snapshot(&mut replica, &snap);
            assert_eq!(replica.drawable_state(), game.drawable_state(), "frame {frame}");
            seen.push(replica.drawable_state().restart_tenths);
            // The two frames in between are the replica's own, and it
            // runs no clock of its own over them.
            let held = replica.drawable_state().restart_tenths;
            replica.tick_presentation(PHYSICS_FIXED_DT);
            replica.tick_presentation(PHYSICS_FIXED_DT);
            assert_eq!(replica.drawable_state().restart_tenths, held, "the replica ran the countdown itself");
        }
        assert!(seen.windows(2).all(|w| w[1] <= w[0]), "the countdown went back up: {seen:?}");
        assert!(seen.last() < seen.first(), "the countdown never moved: {seen:?}");
        assert!(game.outcome() != crate::simulation::Outcome::Playing, "the round restarted mid-test");
    }

    #[test]
    fn a_late_joiner_sees_the_same_picture() {
        let mut game = authoritative(PROPS_MAP, 0xF406, 6);
        for frame in 1..=600 {
            if frame == 120 {
                game.debug_detonate(map::cell_to_world(20, 10)).expect("the barrel cluster");
            }
            if frame == 300 {
                game.debug_kill(1).expect("enemy in slot 1");
            }
            step(&mut game, frame);
        }
        let replica = welcome_through_the_codec(&game);
        let expected = game.drawable_state();
        assert_eq!(replica.drawable_state(), expected);
        assert!(expected.tanks.iter().any(|t| t.wreck), "the round should be busy: a wreck");
        assert!(expected.tiles.len() < authoritative(PROPS_MAP, 0xF406, 6).drawable_state().tiles.len(), "and tiles gone");
        assert_eq!(replica.frame(), game.frame());
        assert!(replica.world.query::<&crate::ai::Ai>().iter().count() == 0, "a replica's enemies have no Ai");
    }

    #[test]
    fn encoding_the_same_round_twice_gives_identical_bytes() {
        let mut game = authoritative(DEFAULT_MAP, 0xB0B5, 6);
        for frame in 1..=240 {
            step(&mut game, frame);
        }
        let a = encode(&Msg::Snapshot(enc::snapshot(&game, [1; MAX_SEATS])));
        let b = encode(&Msg::Snapshot(enc::snapshot(&game, [1; MAX_SEATS])));
        assert_eq!(a, b);
        let welcome = |g: &Game| enc::welcome(g, 0, roster(g), "{}".into(), [1; MAX_SEATS]).unwrap();
        assert_eq!(encode(&Msg::Welcome(welcome(&game))), encode(&Msg::Welcome(welcome(&game))));
        println!("default map, frame 240: snapshot {} B, welcome {} B", a.len(), encode(&Msg::Welcome(welcome(&game))).len());
    }

    #[test]
    fn a_welcome_from_another_protocol_is_refused() {
        let game = authoritative(DEFAULT_MAP, 1, 2);
        let mut w = enc::welcome(&game, 0, roster(&game), "{}".into(), [0; MAX_SEATS]).unwrap();
        w.protocol += 1;
        assert!(welcome(&w).is_err());
        w.protocol -= 1;
        w.map_toml = "not a map".into();
        assert!(welcome(&w).is_err());
    }

    /// An open waves round, the shape a room server runs when the map
    /// rolls its enemies in: one wave of one, no growth.
    fn waves_round() -> Game {
        let mut game = Game::default();
        game.seed_override = Some(0x5A1E);
        game.player_row_override = Some(3);
        game.level_overrides.mission = Some(Mission::Destroy);
        game.level_overrides.spawn = Some(SpawnKind::Waves);
        game.level_overrides.waves = Some(1);
        game.level_overrides.wave_size = Some(1);
        game.level_overrides.wave_growth = Some(0);
        game.map = MapFile::from_toml_str("version = 1\ntanks = 1\ncells.\"20,11\" = { kind = \"start\" }\n")
            .expect("the inline map parses");
        let (width, height) = game.map.field_size();
        game.init(width, height);
        game
    }

    /// `Tank::alpha` of the tank in `slot`, `None` once it is gone.
    fn wreck_alpha(game: &Game, slot: usize) -> Option<f32> {
        game.world.query::<&Tank>().iter().find(|t| t.owner_slot() == slot).map(|t| t.alpha())
    }

    /// A wave round's wreck fades out on the replica without the server
    /// sending its `despawn_timer`: the replica arms the timer the first
    /// frame it sees the wreck and runs it down, and taking the hull off
    /// the field stays the server's (`Event::WreckRemoved` plus a snapshot
    /// that no longer lists it).
    #[test]
    fn a_replicas_wreck_fades_out_on_its_own() {
        let mut game = waves_round();
        let mut slot = None;
        for frame in 1..=900 {
            step(&mut game, frame);
            if let Some(&crate::simulation::Event::TankEntered { slot: s }) =
                game.events().iter().find(|e| matches!(e, crate::simulation::Event::TankEntered { .. }))
            {
                slot = Some(s);
                break;
            }
        }
        let slot = slot.expect("a wave tank rolls in");
        game.debug_kill(slot).expect("the tank that rolled in");
        step(&mut game, 1);
        assert!(game.drawable_state().tanks.iter().any(|t| t.wreck), "the round has a wreck");

        let mut replica = welcome_through_the_codec(&game);
        assert_eq!(wreck_alpha(&replica, slot), Some(1.0), "a wreck starts at full opacity");
        let seconds = tuning().wave_wreck_despawn_seconds;
        let ticks = |s: f32| (s / PHYSICS_FIXED_DT).round() as u32;
        for _ in 0..ticks(seconds - 0.5) {
            replica.tick_presentation(PHYSICS_FIXED_DT);
        }
        let fading = wreck_alpha(&replica, slot).expect("the replica does not remove its own wrecks");
        assert!(fading > 0.0 && fading < 1.0, "half a second from removal it is fading: {fading}");
        for _ in 0..ticks(0.6) {
            replica.tick_presentation(PHYSICS_FIXED_DT);
        }
        assert_eq!(wreck_alpha(&replica, slot), Some(0.0), "faded out, and still on the field");
    }

    /// A ground fire that goes out darkens the replica's ground, spends
    /// its oil cell and drops its charred plank - driven by the snapshot
    /// that stops listing the cell, since burning out is the only way a
    /// fire ever leaves the list.
    #[test]
    fn a_replicas_fire_burns_out_when_the_snapshot_drops_it() {
        let mut game = authoritative(PROPS_MAP, 0xC0FFEE, 2);
        game.debug_detonate(map::cell_to_world(17, 6)).expect("the oil drum at (17,6)");
        for frame in 1..=30 {
            step(&mut game, frame);
        }
        let lit: Vec<(i32, i32)> = game.fires.iter().map(|f| f.cell).collect();
        assert!(!lit.is_empty(), "the drum leaves a burning pool");
        let mut replica = welcome_through_the_codec(&game);
        assert_eq!(replica.fires.len(), game.fires.len(), "the replica has the same pool");
        let decals = replica.decals.len();

        let mut burnt = false;
        for frame in 31..=1800 {
            step(&mut game, frame);
            if frame % 3 == 0 {
                snapshot(&mut replica, &enc::snapshot(&game, [0; MAX_SEATS]));
            } else {
                replica.tick_presentation(PHYSICS_FIXED_DT);
            }
            if lit.iter().all(|cell| !game.fires.iter().any(|f| f.cell == *cell)) {
                snapshot(&mut replica, &enc::snapshot(&game, [0; MAX_SEATS]));
                burnt = true;
                break;
            }
        }
        assert!(burnt, "the pool never burned out");
        assert!(replica.fires.iter().all(|f| !lit.contains(&f.cell)), "the cells are out on the replica too");
        assert!(replica.decals.len() > decals, "a burnt-out cell leaves a charred plank");
        assert_eq!(replica.oil_cells, game.oil_cells, "and the oil it burned is spent on both sides");
    }

    /// A tile the snapshot says is burning flickers on the replica, and
    /// only flickers: the charring that kills it is the server's, and the
    /// tile stays on the field until an `ObstacleDestroyed` takes it away,
    /// however long the replica runs.
    #[test]
    fn a_burning_tile_flickers_on_the_replica_without_charring_out() {
        const WOOD_MAP: &str =
            "version = 1\ntanks = 0\ncells.\"5,5\" = { kind = \"start\" }\ncells.\"10,8\" = { kind = \"wall\", material = \"wood\" }\n";
        let cell = (10, 8);
        let game = authoritative(WOOD_MAP, 1, 0);
        // Light the wall the way a fire beside it does.
        for obstacle in game.world.query::<&mut Obstacle>().iter() {
            if obstacle.cell() == cell {
                obstacle.health = 0.0;
                obstacle.burning = true;
            }
        }
        let mut replica = welcome_through_the_codec(&game);
        let tile = |g: &Game| {
            g.world.query::<&Obstacle>().iter().find(|o| o.cell() == cell).map(|o| (o.burning, o.burn_frame, o.burn_elapsed))
        };
        assert_eq!(tile(&replica), Some((true, 0, 0.0)), "the replica has the burning wall");
        let ticks = ((tuning().wood_burn_frame_seconds + PHYSICS_FIXED_DT) / PHYSICS_FIXED_DT).ceil() as u32;
        for _ in 0..ticks {
            replica.tick_presentation(PHYSICS_FIXED_DT);
        }
        assert_eq!(tile(&replica), Some((true, 1, 0.0)), "the flicker advanced, the charring did not");
        // Well past `wood_burn_seconds`, and the tile is still standing.
        for _ in 0..600 {
            replica.tick_presentation(PHYSICS_FIXED_DT);
        }
        assert!(tile(&replica).is_some(), "the replica never chars a tile out on its own");
    }

    /// `DrumLaunched` puts the drum in the air on the replica, which flies
    /// it from an age `tick_presentation` advances and drops it when it
    /// lands - the blast there is the server's own `Blast`.
    #[test]
    fn a_launched_drum_flies_on_the_replica() {
        let mut game = authoritative(PROPS_MAP, 0xC0FFEE, 2);
        let mut replica = welcome_through_the_codec(&game);
        // A cascade can launch more than one drum on the same frame.
        let mut launched: Vec<(f32, f32, f32, f32)> = Vec::new();
        for frame in 1..=900 {
            if frame == 120 {
                game.debug_detonate(map::cell_to_world(17, 6)).expect("the oil drum at (17,6)");
            }
            step(&mut game, frame);
            snapshot(&mut replica, &enc::snapshot(&game, [0; MAX_SEATS]));
            launched.extend(game.events().iter().filter_map(|e| match *e {
                crate::simulation::Event::DrumLaunched { x, y, to_x, to_y } => Some((x, y, to_x, to_y)),
                _ => None,
            }));
            if !launched.is_empty() {
                break;
            }
        }
        let (x, y, to_x, to_y) = *launched.first().expect("the trail reaches a fuel drum and launches it");
        assert_eq!(replica.flying_drums.len(), launched.len(), "the replica put every launched drum in the air");
        let drum = replica.flying_drums[0];
        // The launch travels in quarter pixels.
        assert!(drum.from.distance_to(Position::new(x, y)) <= 0.5, "from {:?} for {x},{y}", drum.from);
        assert!(drum.to.distance_to(Position::new(to_x, to_y)) <= 0.5, "to {:?} for {to_x},{to_y}", drum.to);

        let start = drum.draw_pos();
        replica.tick_presentation(PHYSICS_FIXED_DT);
        assert!(replica.flying_drums[0].draw_pos().distance_to(start) > 0.0, "the arc advances");
        let ticks = (tuning().debris_flight_seconds / PHYSICS_FIXED_DT).ceil() as u32 + 2;
        for _ in 0..ticks {
            replica.tick_presentation(PHYSICS_FIXED_DT);
        }
        assert!(replica.flying_drums.is_empty(), "and it lands");
    }

    #[test]
    fn cell_indices_round_trip() {
        for cols in [1u16, 34, 40, 200] {
            for (col, row) in [(0, 0), (3, 7), (33, 16)] {
                let i = cell_index(cols, (col.min(cols as i32 - 1), row));
                assert_eq!(cell_from_index(cols, i), (col.min(cols as i32 - 1), row));
            }
        }
    }
}
