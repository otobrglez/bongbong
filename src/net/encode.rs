//! A live `Game` onto the wire (docs/online-coop-prd.md §4.2): `snapshot`
//! fills every family the PRD's table lists as travelling, `welcome` wraps
//! one in the round's parameters. Reads the round through
//! `simulation::replica`'s accessors and the entities' own quantised
//! readings (`Tank::hull_points`, `Frog::clip_phase`,
//! `Obstacle::lean_strength`), so what goes on the wire is exactly what
//! `Game::drawable_state` compares. Pure: no RNG, nothing written back.
//!
//! What a snapshot cannot say, the events do: a tile's death is an
//! `ObstacleDestroyed` (the tile is gone from the world the same frame,
//! so it appears here once more as a `DESTROYED` entry cut from that
//! event), a shot's turn is a `Ricochet`. `Snapshot::events` holds the
//! events of the frame the snapshot was cut on; a server sending every
//! third tick keeps `wire_events` of the ticks between and prepends them.

use crate::net::MAX_SEATS;
use crate::net::PROTOCOL_VERSION;
use crate::net::events::WireEvent;
use crate::net::wire::{
    BonusPickup, FireState, FrogState, RoundState, Seat, ShotKind, ShotState, Snapshot, TankState, TileState, Welcome,
    dir_index, frog_flags, quantise_heading, quantise_health, quantise_pos, quantise_seconds, quantise_velocity, tank_flags, tile_flags,
};
use crate::bullet::Bullet;
use crate::frog::Frog;
use crate::map;
use crate::obstacle::Obstacle;
use crate::pickup::Pickup;
use crate::plasma::Plasma;
use crate::shell::Shell;
use crate::simulation::replica::plasma_variant_index;
use crate::simulation::{Event, Game};
use crate::tank::{Dir, Tank};
use crate::{OBSTACLE_GRID_SIZE, Position};

/// The map's width in cells, the stride of every `row * cols + col` cell
/// index on the wire.
pub fn field_cols(game: &Game) -> u16 {
    let (width, _) = game.map.field_size();
    (width / OBSTACLE_GRID_SIZE).round().max(1.0) as u16
}

/// A grid cell as its wire index; a cell off the map saturates.
pub fn cell_index(cols: u16, cell: (i32, i32)) -> u16 {
    let (col, row) = (cell.0.max(0) as u32, cell.1.max(0) as u32);
    (row * cols as u32 + col).min(u16::MAX as u32) as u16
}

/// Inverse of `cell_index`.
pub fn cell_from_index(cols: u16, index: u16) -> (i32, i32) {
    let cols = cols.max(1) as i32;
    ((index as i32) % cols, (index as i32) / cols)
}

/// The wire form of the events in `events`, the AI's trace left out.
pub fn wire_events(events: &[Event]) -> Vec<WireEvent> {
    events.iter().filter_map(WireEvent::from_event).collect()
}

/// The complete state of `game` at its current frame. `server_ms` is left
/// at 0: the clock is the server's, which stamps it before sending.
pub fn snapshot(game: &Game, acked: [u32; MAX_SEATS]) -> Snapshot {
    let cols = field_cols(game);
    let mut s = Snapshot {
        tick: game.frame().min(u32::MAX as u64) as u32,
        server_ms: 0,
        acked,
        tanks: tanks(game),
        shots: shots(game),
        frogs: frogs(game),
        pickups: 0,
        bonus_pickups: Vec::new(),
        tiles: tiles(game, cols),
        fires: game.fires.iter().map(|f| FireState { cell: cell_index(cols, f.cell), left: quantise_seconds(f.left) }).collect(),
        round: round(game),
        events: wire_events(game.events()),
    };
    let (mask, bonus) = pickups(game, cols);
    s.pickups = mask;
    s.bonus_pickups = bonus;
    s.normalise();
    s
}

/// Everything a joining client needs: the round's parameters, the holes
/// fire and shot have made in the map, and a full `snapshot`. The roster's
/// chassis are what the server pinned each seat's `player_row_override`
/// to, so the replica's `init` draws the same rolls. Fails only if the map
/// does not serialise.
pub fn welcome(
    game: &Game,
    seat: u8,
    roster: Vec<Seat>,
    tuning_json: String,
    acked: [u32; MAX_SEATS],
) -> Result<Welcome, String> {
    let cols = field_cols(game);
    let mut oil_cells: Vec<u16> = game.oil_cells.iter().map(|&c| cell_index(cols, c)).collect();
    oil_cells.sort_unstable();
    oil_cells.dedup();
    let standing: std::collections::BTreeSet<u16> = game
        .world
        .query::<&Obstacle>()
        .iter()
        .filter(|o| !o.destroyed)
        .map(|o| cell_index(cols, o.cell()))
        .collect();
    let dead_cells: Vec<u16> = game
        .map
        .iter_cells()
        .filter(|(_, _, o)| o.is_solid())
        .map(|(col, row, _)| cell_index(cols, (col, row)))
        .filter(|c| !standing.contains(c))
        .collect();
    Ok(Welcome {
        protocol: PROTOCOL_VERSION,
        sim_version: env!("CARGO_PKG_VERSION").to_string(),
        seat,
        roster,
        map_toml: game.map.to_toml_string()?,
        seed: game.round_seed(),
        tuning_json,
        overrides: game.level_overrides.into(),
        enemy_count: game.enemy_count_override.map(|n| n.min(u16::MAX as usize) as u16),
        oil_cells,
        dead_cells,
        snapshot: snapshot(game, acked),
    })
}

fn tanks(game: &Game) -> Vec<TankState> {
    game.world
        .query::<&Tank>()
        .iter()
        .map(|t| {
            let velocity = game.body_velocity(t);
            let mut flags = 0;
            for (on, bit) in [
                (t.is_wreck(), tank_flags::WRECK),
                (t.is_shielded(), tank_flags::SHIELD),
                (t.speed_boost_timer > 0.0, tank_flags::BOOST),
                (t.burn_timer > 0.0, tank_flags::BURNING),
                (t.hit_flash_timer > 0.0, tank_flags::HIT),
                (t.flame_held, tank_flags::FLAME),
            ] {
                if on {
                    flags |= bit;
                }
            }
            TankState {
                id: t.owner_slot().min(u16::MAX as usize) as u16,
                row: t.row.clamp(0, u8::MAX as i32) as u8,
                x: quantise_pos(t.position.x),
                y: quantise_pos(t.position.y),
                vx: quantise_velocity(velocity.x),
                vy: quantise_velocity(velocity.y),
                dir: dir_index(Dir::from_rotation(t.rotation).unwrap_or(Dir::Up)),
                hp: t.hull_points(),
                shield: t.shield_points(),
                flags,
                weapon: t.active_weapon().into(),
                ammo: t.active_ammo(),
            }
        })
        .collect()
}

fn shot(id: u32, kind: ShotKind, position: Position, rotation: f32, state: i32, variant: i32) -> ShotState {
    ShotState {
        id: (id & 0xFFFF) as u16,
        kind,
        x: quantise_pos(position.x),
        y: quantise_pos(position.y),
        heading: quantise_heading(rotation),
        state: state.clamp(0, u8::MAX as i32) as u8,
        variant: variant.clamp(0, u8::MAX as i32) as u8,
    }
}

fn shots(game: &Game) -> Vec<ShotState> {
    let mut out = Vec::new();
    for s in game.world.query::<&Shell>().iter() {
        out.push(shot(s.id, ShotKind::Shell, s.position, s.rotation, s.state.col(), s.variant));
    }
    for b in game.world.query::<&Bullet>().iter() {
        out.push(shot(b.id, ShotKind::Bullet, b.position, b.rotation, b.state.col(), 0));
    }
    for p in game.world.query::<&Plasma>().iter() {
        out.push(shot(p.id, ShotKind::Plasma, p.position, p.rotation, p.state.col(), plasma_variant_index(p.variant)));
    }
    out
}

fn frogs(game: &Game) -> Vec<FrogState> {
    game.world
        .query::<&Frog>()
        .iter()
        .map(|f| {
            let hopping = f.hop_timer > 0.0;
            let mut state = 0;
            for (on, bit) in [
                (f.is_dead(), frog_flags::DEAD),
                (hopping, frog_flags::HOPPING),
                (f.hurt_timer > 0.0, frog_flags::HURT),
                (f.attack_timer > 0.0, frog_flags::BITING),
            ] {
                if on {
                    state |= bit;
                }
            }
            FrogState {
                side: f.side,
                x: quantise_pos(f.position.x),
                y: quantise_pos(f.position.y),
                hp: f.health_points(),
                state,
                phase: f.clip_phase(),
                hop_x: if hopping { quantise_pos(f.hop_end.x) } else { 0 },
                hop_y: if hopping { quantise_pos(f.hop_end.y) } else { 0 },
            }
        })
        .collect()
}

/// The slot bitmask (bit `i` for the `i`th map slot with its pickup on
/// the field; slots past 63 cannot travel) and the pickups on no slot.
fn pickups(game: &Game, cols: u16) -> (u64, Vec<BonusPickup>) {
    let slots = game.pickup_slots();
    let mut mask = 0u64;
    let mut bonus = Vec::new();
    for p in game.world.query::<&Pickup>().iter() {
        let slot = slots.iter().position(|&(pos, _)| pos.distance_to(p.position) <= 0.5);
        match slot {
            Some(i) if i < 64 => mask |= 1 << i,
            _ => bonus.push(BonusPickup { cell: cell_index(cols, map::world_to_cell(p.position)), kind: p.kind }),
        }
    }
    (mask, bonus)
}

/// Every live tile that differs from its fresh state, then a `DESTROYED`
/// entry for each tile the frame's events say died.
fn tiles(game: &Game, cols: u16) -> Vec<TileState> {
    let movers: Vec<Position> =
        game.world.query::<&Tank>().iter().filter(|t| !t.is_wreck()).map(|t| t.position).collect();
    let mut out: Vec<TileState> = game
        .world
        .query::<&Obstacle>()
        .iter()
        .filter(|o| !o.destroyed)
        // Health in the whole points it travels in: a tile a fraction under
        // full draws as full, and the replica keeps it at full.
        .filter(|o| {
            tile_points(o.health) != tile_points(o.max_health)
                || o.burning
                || o.fuse.is_some()
                || o.scorched != 0
                || o.ram_timer > 0.0
        })
        .map(|o| {
            let mut flags = 0;
            if o.burning {
                flags |= tile_flags::BURNING;
            }
            if o.fuse.is_some() {
                flags |= tile_flags::FUSED;
            }
            let strength = o.lean_strength();
            if strength > 0 {
                // The side the push comes from: the nearest hull, as the
                // tree's lean is drawn from.
                let pusher = movers
                    .iter()
                    .min_by(|a, b| a.distance_to(o.position).total_cmp(&b.distance_to(o.position)))
                    .map_or(0, |m| dir_index(Dir::toward(*m, o.position)));
                flags |= ((strength << 2) | pusher) << tile_flags::LEAN_SHIFT;
            }
            TileState {
                cell: cell_index(cols, o.cell()),
                hp: tile_points(o.health),
                flags,
                faces: o.scorched,
            }
        })
        .collect();
    for event in game.events() {
        if let Event::ObstacleDestroyed { x, y, .. } = *event {
            let cell = cell_index(cols, map::world_to_cell(Position::new(x, y)));
            out.push(TileState { cell, hp: 0, flags: tile_flags::DESTROYED, faces: 0 });
        }
    }
    out
}

/// A tile's health in whole points, `quantise_health` by another name so
/// the change test above reads the same as the value sent.
fn tile_points(health: f32) -> u8 {
    quantise_health(health)
}

fn round(game: &Game) -> RoundState {
    let wave = game.wave_status();
    let byte = |n: usize| n.min(u8::MAX as usize) as u8;
    RoundState {
        wave: wave.map_or(0, |w| w.index.min(u8::MAX as u32) as u8),
        alive: wave.map_or(0, |w| byte(w.alive)),
        pending: wave.map_or(0, |w| byte(w.pending)),
        intro: quantise_seconds(game.intro_timer),
        outcome: game.outcome().into(),
    }
}
