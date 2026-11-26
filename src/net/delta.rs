//! Delta encoding of a `Snapshot` against the previous one. The baseline
//! is the last snapshot sent: a WebSocket loses nothing, so the server
//! keeps one `Snapshot` per room and the client one per socket, and both
//! step them with `apply_delta`. `Welcome` carries the first full one.
//!
//! Per keyed family (`tanks`, `shots`, `missiles`, `grenades`, `drones`, `bongs`, `tiles`, `fires`, `crates`, `grass_flat`) a delta
//! carries three lists: entries that are new or changed (in full),
//! entries that only moved by less than 32 px on each axis (`Moved`: the
//! key and two `i8` quarter-pixel steps, the common case for every hull
//! and projectile between two snapshots 50 ms apart), and the keys that
//! disappeared. Unchanged entries cost nothing. The towers' four families
//! (`towers`, then `globs`, `ooze` and `coats` in a nested `OozeDelta`)
//! travel the same way inside one optional `TowerDelta`, absent when none
//! of them changed. The scalar families
//! (`pickups`, `bonus_pickups`, `round`) travel whole when they differ and
//! as `None` when not; `events` are always sent whole, since they are what
//! happened since the previous snapshot rather than state.
//!
//! Everything is keyed through `BTreeMap`s, so the output is sorted by key
//! whatever order the inputs came in, and `apply_delta` returns a
//! normalised `Snapshot` (`Snapshot::normalise`).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::net::MAX_SEATS;
use crate::net::events::WireEvent;
use crate::net::wire::{
    AirState,
    BonusPickup, CoatState, CraterState, GlobState, GrassFlatState, OozeState, RubbleState, CrateState, DroneState, FireState, BongState, GrenadeState, LampState, LooseState, MissileState, RoundState, ShotState, Snapshot, TankState, TowerState, WellDrumState,
    ZoneState, TileState, side_code,
};

/// An entry that kept every field but its position, which moved by
/// (`dx`, `dy`) quarter pixels since the previous snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Moved {
    /// The family's key: a tank's or shot's id, a bong's `side_code`.
    pub key: u16,
    pub dx: i8,
    pub dy: i8,
}

/// The difference between two consecutive snapshots.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct SnapshotDelta {
    pub tick: u32,
    pub server_ms: u32,
    pub acked: [u32; MAX_SEATS],
    pub mailbox: [u8; MAX_SEATS],
    /// New or changed tanks, in full.
    pub tanks: Vec<TankState>,
    pub tanks_moved: Vec<Moved>,
    pub tanks_gone: Vec<u16>,
    /// New or changed shots, in full.
    pub shots: Vec<ShotState>,
    pub shots_moved: Vec<Moved>,
    pub shots_gone: Vec<u16>,
    /// New or changed missiles, in full.
    pub missiles: Vec<MissileState>,
    pub missiles_moved: Vec<Moved>,
    pub missiles_gone: Vec<u16>,
    /// New or changed grenades, in full.
    pub grenades: Vec<GrenadeState>,
    pub grenades_moved: Vec<Moved>,
    pub grenades_gone: Vec<u16>,
    /// New or changed drones, in full.
    pub drones: Vec<DroneState>,
    pub drones_moved: Vec<Moved>,
    pub drones_gone: Vec<u16>,
    /// New or changed bongs, in full.
    pub bongs: Vec<BongState>,
    pub bongs_moved: Vec<Moved>,
    /// Keys are `side_code`s.
    pub bongs_gone: Vec<u16>,
    pub pickups: Option<u64>,
    pub bonus_pickups: Option<Vec<BonusPickup>>,
    /// New or changed tiles, in full.
    pub tiles: Vec<TileState>,
    pub tiles_gone: Vec<u16>,
    /// New or changed fires, in full.
    pub fires: Vec<FireState>,
    pub fires_gone: Vec<u16>,
    /// New lanterns, in full.
    pub lamps: Vec<LampState>,
    pub lamps_gone: Vec<u16>,
    /// New or changed crates, in full.
    pub crates: Vec<CrateState>,
    pub crates_gone: Vec<u16>,
    /// New zones, in full (a zone never changes; it only comes and goes).
    pub zones: Vec<ZoneState>,
    pub zones_gone: Vec<u16>,
    /// New craters, in full.
    pub craters: Vec<CraterState>,
    pub craters_gone: Vec<u16>,
    /// New or deeper rubble, in full.
    pub rubble: Vec<RubbleState>,
    pub rubble_gone: Vec<u16>,
    /// New or changed held drums, in full.
    pub well_drums: Vec<WellDrumState>,
    pub well_drums_gone: Vec<u16>,
    pub loose: Vec<LooseState>,
    pub loose_gone: Vec<u16>,
    /// New or changed air support, in full.
    pub air: Vec<AirState>,
    pub air_moved: Vec<Moved>,
    pub air_gone: Vec<u16>,
    /// The volcanoes' shifts, whole, when they changed.
    pub volcano_shifts: Option<Vec<i32>>,
    /// The smoke cells, whole, when they changed.
    pub smoke_cells: Option<Vec<u16>>,
    /// Newly flattened (or flattened again) grass, in full.
    pub grass_flat: Vec<GrassFlatState>,
    pub grass_flat_gone: Vec<u16>,
    /// The towers' families, when any of them changed.
    pub towers: Option<TowerDelta>,
    pub round: Option<RoundState>,
    pub events: Vec<WireEvent>,
}

/// The defence towers' families (docs/defence-towers-prd.md §13): the
/// weapons, and behind them the bio slush's globs, ooze and coats. Kept
/// behind one `Option` so a delta in which none of them changed - every
/// delta on a map without towers, and most on one with them - pays one
/// byte for all four rather than two per family; and the bio slush's three
/// behind a second, so a tick on which only a turret turned or a charge
/// rose - most of the ticks a tower is in a fight - pays one for those.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct TowerDelta {
    /// New or changed towers, in full.
    pub towers: Vec<TowerState>,
    pub towers_gone: Vec<u16>,
    /// The bio slush's families, when any of them changed.
    pub ooze: Option<OozeDelta>,
}

/// The bio slush's families: the globs in the air, the ooze on the ground
/// and the coats on the tanks.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct OozeDelta {
    /// New globs, in full (a glob never changes; it only comes and goes).
    pub globs: Vec<GlobState>,
    pub globs_gone: Vec<u16>,
    /// New or splashed-again ooze, in full.
    pub ooze: Vec<OozeState>,
    pub ooze_gone: Vec<u16>,
    /// New or changed coats, in full.
    pub coats: Vec<CoatState>,
    pub coats_gone: Vec<u16>,
}

impl TowerDelta {
    /// The difference in the towers' families from `prev` to `next`, or
    /// `None` when there is none.
    fn between(prev: &Snapshot, next: &Snapshot) -> Option<TowerDelta> {
        let (towers, towers_gone) = diff_keyed(&prev.towers, &next.towers);
        let (globs, globs_gone) = diff_keyed(&prev.globs, &next.globs);
        let (ooze, ooze_gone) = diff_keyed(&prev.ooze, &next.ooze);
        let (coats, coats_gone) = diff_keyed(&prev.coats, &next.coats);
        let bio = OozeDelta { globs, globs_gone, ooze, ooze_gone, coats, coats_gone };
        let d = TowerDelta { towers, towers_gone, ooze: (bio != OozeDelta::default()).then_some(bio) };
        (d != TowerDelta::default()).then_some(d)
    }
}

/// An entry of a keyed family.
trait Keyed: Clone + PartialEq {
    fn key(&self) -> u16;
}

/// A keyed entry with a position that may travel as a `Moved`.
trait Positioned: Keyed {
    fn xy(&self) -> (i16, i16);

    fn with_xy(&self, x: i16, y: i16) -> Self;
}

impl Keyed for TankState {
    fn key(&self) -> u16 {
        self.id
    }
}

impl Positioned for TankState {
    fn xy(&self) -> (i16, i16) {
        (self.x, self.y)
    }

    fn with_xy(&self, x: i16, y: i16) -> Self {
        TankState { x, y, ..*self }
    }
}

impl Keyed for ShotState {
    fn key(&self) -> u16 {
        self.id
    }
}

impl Positioned for ShotState {
    fn xy(&self) -> (i16, i16) {
        (self.x, self.y)
    }

    fn with_xy(&self, x: i16, y: i16) -> Self {
        ShotState { x, y, ..*self }
    }
}

impl Keyed for MissileState {
    fn key(&self) -> u16 {
        self.id
    }
}

impl Positioned for MissileState {
    fn xy(&self) -> (i16, i16) {
        (self.x, self.y)
    }

    fn with_xy(&self, x: i16, y: i16) -> Self {
        MissileState { x, y, ..*self }
    }
}

impl Keyed for GrenadeState {
    fn key(&self) -> u16 {
        self.id
    }
}

impl Positioned for GrenadeState {
    fn xy(&self) -> (i16, i16) {
        (self.x, self.y)
    }

    fn with_xy(&self, x: i16, y: i16) -> Self {
        GrenadeState { x, y, ..*self }
    }
}

impl Keyed for DroneState {
    fn key(&self) -> u16 {
        self.id
    }
}

impl Positioned for DroneState {
    fn xy(&self) -> (i16, i16) {
        (self.x, self.y)
    }

    fn with_xy(&self, x: i16, y: i16) -> Self {
        DroneState { x, y, ..*self }
    }
}

impl Keyed for AirState {
    fn key(&self) -> u16 {
        self.id
    }
}

impl Positioned for AirState {
    fn xy(&self) -> (i16, i16) {
        (self.x, self.y)
    }

    fn with_xy(&self, x: i16, y: i16) -> Self {
        AirState { x, y, ..*self }
    }
}

impl Keyed for BongState {
    fn key(&self) -> u16 {
        side_code(self.side) as u16
    }
}

impl Positioned for BongState {
    fn xy(&self) -> (i16, i16) {
        (self.x, self.y)
    }

    fn with_xy(&self, x: i16, y: i16) -> Self {
        BongState { x, y, ..*self }
    }
}

impl Keyed for TileState {
    fn key(&self) -> u16 {
        self.cell
    }
}

impl Keyed for FireState {
    fn key(&self) -> u16 {
        self.cell
    }
}

impl Keyed for LampState {
    fn key(&self) -> u16 {
        self.id
    }
}

impl Keyed for CrateState {
    fn key(&self) -> u16 {
        self.cell
    }
}

impl Keyed for ZoneState {
    fn key(&self) -> u16 {
        self.id
    }
}

impl Keyed for WellDrumState {
    fn key(&self) -> u16 {
        self.id
    }
}

impl Keyed for LooseState {
    fn key(&self) -> u16 {
        self.home
    }
}

impl Keyed for CraterState {
    fn key(&self) -> u16 {
        self.cell
    }
}

impl Keyed for GrassFlatState {
    fn key(&self) -> u16 {
        self.cell
    }
}

impl Keyed for RubbleState {
    fn key(&self) -> u16 {
        self.cell
    }
}

impl Keyed for TowerState {
    fn key(&self) -> u16 {
        self.cell
    }
}

impl Keyed for GlobState {
    fn key(&self) -> u16 {
        self.id
    }
}

impl Keyed for OozeState {
    fn key(&self) -> u16 {
        self.cell
    }
}

impl Keyed for CoatState {
    fn key(&self) -> u16 {
        self.id
    }
}

fn by_key<T: Keyed>(entries: &[T]) -> BTreeMap<u16, &T> {
    entries.iter().map(|e| (e.key(), e)).collect()
}

/// New or changed entries and the keys that went away.
fn diff_keyed<T: Keyed>(prev: &[T], next: &[T]) -> (Vec<T>, Vec<u16>) {
    let prev = by_key(prev);
    let next = by_key(next);
    let changed = next.values().filter(|n| prev.get(&n.key()) != Some(n)).map(|n| (*n).clone()).collect();
    let gone = prev.keys().filter(|k| !next.contains_key(k)).copied().collect();
    (changed, gone)
}

/// `diff_keyed` with the entries that only moved a short way split off.
fn diff_positioned<T: Positioned>(prev: &[T], next: &[T]) -> (Vec<T>, Vec<Moved>, Vec<u16>) {
    let prev = by_key(prev);
    let next = by_key(next);
    let mut changed = Vec::new();
    let mut moved = Vec::new();
    for (key, n) in &next {
        match prev.get(key) {
            Some(p) if *p == *n => {}
            Some(p) => {
                let (px, py) = p.xy();
                let (nx, ny) = n.xy();
                let (dx, dy) = (nx as i32 - px as i32, ny as i32 - py as i32);
                let short = |d: i32| i8::try_from(d).ok();
                match (short(dx), short(dy)) {
                    (Some(dx), Some(dy)) if p.with_xy(nx, ny) == **n => moved.push(Moved { key: *key, dx, dy }),
                    _ => changed.push((*n).clone()),
                }
            }
            None => changed.push((*n).clone()),
        }
    }
    let gone = prev.keys().filter(|k| !next.contains_key(k)).copied().collect();
    (changed, moved, gone)
}

fn apply_keyed<T: Keyed>(prev: &[T], changed: &[T], gone: &[u16]) -> Vec<T> {
    let mut map: BTreeMap<u16, T> = prev.iter().map(|e| (e.key(), e.clone())).collect();
    for key in gone {
        map.remove(key);
    }
    for entry in changed {
        map.insert(entry.key(), entry.clone());
    }
    map.into_values().collect()
}

fn apply_positioned<T: Positioned>(prev: &[T], changed: &[T], moved: &[Moved], gone: &[u16]) -> Vec<T> {
    let mut map: BTreeMap<u16, T> = prev.iter().map(|e| (e.key(), e.clone())).collect();
    for key in gone {
        map.remove(key);
    }
    for m in moved {
        if let Some(entry) = map.get_mut(&m.key) {
            let (x, y) = entry.xy();
            *entry = entry.with_xy(x.wrapping_add(m.dx as i16), y.wrapping_add(m.dy as i16));
        }
    }
    for entry in changed {
        map.insert(entry.key(), entry.clone());
    }
    map.into_values().collect()
}

/// The delta that takes `prev` to `next`.
pub fn delta(prev: &Snapshot, next: &Snapshot) -> SnapshotDelta {
    let (tanks, tanks_moved, tanks_gone) = diff_positioned(&prev.tanks, &next.tanks);
    let (shots, shots_moved, shots_gone) = diff_positioned(&prev.shots, &next.shots);
    let (missiles, missiles_moved, missiles_gone) = diff_positioned(&prev.missiles, &next.missiles);
    let (grenades, grenades_moved, grenades_gone) = diff_positioned(&prev.grenades, &next.grenades);
    let (drones, drones_moved, drones_gone) = diff_positioned(&prev.drones, &next.drones);
    let (bongs, bongs_moved, bongs_gone) = diff_positioned(&prev.bongs, &next.bongs);
    let (tiles, tiles_gone) = diff_keyed(&prev.tiles, &next.tiles);
    let (fires, fires_gone) = diff_keyed(&prev.fires, &next.fires);
    let (lamps, lamps_gone) = diff_keyed(&prev.lamps, &next.lamps);
    let (crates, crates_gone) = diff_keyed(&prev.crates, &next.crates);
    let (zones, zones_gone) = diff_keyed(&prev.zones, &next.zones);
    let (craters, craters_gone) = diff_keyed(&prev.craters, &next.craters);
    let (rubble, rubble_gone) = diff_keyed(&prev.rubble, &next.rubble);
    let (well_drums, well_drums_gone) = diff_keyed(&prev.well_drums, &next.well_drums);
    let (loose, loose_gone) = diff_keyed(&prev.loose, &next.loose);
    let (air, air_moved, air_gone) = diff_positioned(&prev.air, &next.air);
    let (grass_flat, grass_flat_gone) = diff_keyed(&prev.grass_flat, &next.grass_flat);
    let mut bonus_pickups = next.bonus_pickups.clone();
    bonus_pickups.sort();
    bonus_pickups.dedup();
    let mut prev_bonus = prev.bonus_pickups.clone();
    prev_bonus.sort();
    prev_bonus.dedup();
    SnapshotDelta {
        tick: next.tick,
        server_ms: next.server_ms,
        acked: next.acked,
        mailbox: next.mailbox,
        tanks,
        tanks_moved,
        tanks_gone,
        shots,
        shots_moved,
        shots_gone,
        missiles,
        missiles_moved,
        missiles_gone,
        grenades,
        grenades_moved,
        grenades_gone,
        drones,
        drones_moved,
        drones_gone,
        bongs,
        bongs_moved,
        bongs_gone,
        pickups: (next.pickups != prev.pickups).then_some(next.pickups),
        bonus_pickups: (bonus_pickups != prev_bonus).then_some(bonus_pickups),
        tiles,
        tiles_gone,
        fires,
        fires_gone,
        lamps,
        lamps_gone,
        crates,
        crates_gone,
        zones,
        zones_gone,
        craters,
        craters_gone,
        rubble,
        rubble_gone,
        well_drums,
        well_drums_gone,
        loose,
        loose_gone,
        air,
        air_moved,
        air_gone,
        volcano_shifts: (next.volcano_shifts != prev.volcano_shifts).then(|| next.volcano_shifts.clone()),
        smoke_cells: (next.smoke_cells != prev.smoke_cells).then(|| next.smoke_cells.clone()),
        grass_flat,
        grass_flat_gone,
        towers: TowerDelta::between(prev, next),
        round: (next.round != prev.round).then_some(next.round),
        events: next.events.clone(),
    }
}

/// `prev` stepped by `delta`: the snapshot the delta was cut from, in
/// normalised order. A `Moved` whose key `prev` lacks is ignored, which
/// only happens when the delta was not cut against this baseline.
pub fn apply_delta(prev: &Snapshot, delta: &SnapshotDelta) -> Snapshot {
    let mut bonus_pickups = match &delta.bonus_pickups {
        Some(b) => b.clone(),
        None => prev.bonus_pickups.clone(),
    };
    bonus_pickups.sort();
    bonus_pickups.dedup();
    let (none, no_ooze) = (TowerDelta::default(), OozeDelta::default());
    let towers = delta.towers.as_ref().unwrap_or(&none);
    let bio = towers.ooze.as_ref().unwrap_or(&no_ooze);
    Snapshot {
        tick: delta.tick,
        server_ms: delta.server_ms,
        acked: delta.acked,
        mailbox: delta.mailbox,
        tanks: apply_positioned(&prev.tanks, &delta.tanks, &delta.tanks_moved, &delta.tanks_gone),
        shots: apply_positioned(&prev.shots, &delta.shots, &delta.shots_moved, &delta.shots_gone),
        missiles: apply_positioned(&prev.missiles, &delta.missiles, &delta.missiles_moved, &delta.missiles_gone),
        grenades: apply_positioned(&prev.grenades, &delta.grenades, &delta.grenades_moved, &delta.grenades_gone),
        drones: apply_positioned(&prev.drones, &delta.drones, &delta.drones_moved, &delta.drones_gone),
        bongs: apply_positioned(&prev.bongs, &delta.bongs, &delta.bongs_moved, &delta.bongs_gone),
        pickups: delta.pickups.unwrap_or(prev.pickups),
        bonus_pickups,
        tiles: apply_keyed(&prev.tiles, &delta.tiles, &delta.tiles_gone),
        fires: apply_keyed(&prev.fires, &delta.fires, &delta.fires_gone),
        lamps: apply_keyed(&prev.lamps, &delta.lamps, &delta.lamps_gone),
        crates: apply_keyed(&prev.crates, &delta.crates, &delta.crates_gone),
        zones: apply_keyed(&prev.zones, &delta.zones, &delta.zones_gone),
        craters: apply_keyed(&prev.craters, &delta.craters, &delta.craters_gone),
        rubble: apply_keyed(&prev.rubble, &delta.rubble, &delta.rubble_gone),
        well_drums: apply_keyed(&prev.well_drums, &delta.well_drums, &delta.well_drums_gone),
        loose: apply_keyed(&prev.loose, &delta.loose, &delta.loose_gone),
        air: apply_positioned(&prev.air, &delta.air, &delta.air_moved, &delta.air_gone),
        volcano_shifts: delta.volcano_shifts.clone().unwrap_or_else(|| prev.volcano_shifts.clone()),
        smoke_cells: delta.smoke_cells.clone().unwrap_or_else(|| prev.smoke_cells.clone()),
        grass_flat: apply_keyed(&prev.grass_flat, &delta.grass_flat, &delta.grass_flat_gone),
        towers: apply_keyed(&prev.towers, &towers.towers, &towers.towers_gone),
        globs: apply_keyed(&prev.globs, &bio.globs, &bio.globs_gone),
        ooze: apply_keyed(&prev.ooze, &bio.ooze, &bio.ooze_gone),
        coats: apply_keyed(&prev.coats, &bio.coats, &bio.coats_gone),
        round: delta.round.unwrap_or(prev.round),
        events: delta.events.clone(),
    }
}

#[cfg(test)]
mod tests {
    use crate::RoundRng;
    use rand::{RngExt, SeedableRng};

    use super::*;
    use crate::bong::Side;
    use crate::net::codec::{Msg, encode};
    use crate::net::wire::{RoundOutcome, ShotKind, WeaponKind, quantise_pos, quantise_velocity};
    use crate::pickup::PickupKind;

    fn random_tank(rng: &mut RoundRng, id: u16) -> TankState {
        TankState {
            id,
            row: rng.random_range(0..12),
            x: rng.random_range(-4000..4000),
            y: rng.random_range(-4000..4000),
            vx: rng.random_range(-100..100),
            vy: rng.random_range(-100..100),
            dir: rng.random_range(0..4),
            hp: rng.random_range(0..=100),
            shield: rng.random_range(0..=50),
            flags: rng.random(),
            weapon: WeaponKind::ALL[rng.random_range(0..WeaponKind::ALL.len())],
            ammo: rng.random(),
            tell: rng.random_range(0..8),
            skid: rng.random_range(0..12),
            disabled: rng.random_range(0..30),
            offline: rng.random_range(0..30),
            shells: rng.random_range(0..12),
            charge: if rng.random_range(0..4) == 0 { rng.random_range(1..200) } else { 0 },
            reticle: if rng.random_range(0..6) == 0 { rng.random_range(1..600) } else { 0 },
            track: if rng.random_range(0..4) == 0 { rng.random_range(-120..120) } else { 0 },
            target: if rng.random_range(0..4) == 0 { rng.random_range(1..12) } else { 0 },
        }
    }

    fn random_air(rng: &mut RoundRng, id: u16) -> AirState {
        AirState {
            id,
            seat: rng.random_range(0..8),
            sortie: rng.random_range(0..2),
            stage: rng.random_range(0..7),
            arrive: rng.random_range(0..200_000),
            x: rng.random(),
            y: rng.random(),
            height: rng.random_range(0..400),
            heading: rng.random(),
            bank: rng.random_range(-127..=127),
            ammo: rng.random_range(0..9),
        }
    }

    fn random_shot(rng: &mut RoundRng, id: u16) -> ShotState {
        ShotState {
            id,
            kind: [ShotKind::Shell, ShotKind::Bullet, ShotKind::Plasma, ShotKind::Orb][rng.random_range(0..4)],
            x: rng.random_range(-4000..4000),
            y: rng.random_range(-4000..4000),
            heading: rng.random(),
            state: rng.random_range(0..8),
            variant: rng.random_range(0..18), owner: 255,
        }
    }

    fn random_bong(rng: &mut RoundRng, side: Side) -> BongState {
        BongState {
            side,
            x: rng.random_range(0..4000),
            y: rng.random_range(0..2000),
            hp: rng.random_range(0..=40),
            state: rng.random_range(0..16),
            phase: rng.random(),
            hop_x: rng.random_range(0..4000),
            hop_y: rng.random_range(0..2000),
        }
    }

    fn random_tower(rng: &mut RoundRng, cell: u16) -> TowerState {
        TowerState { cell, heading: rng.random(), charge: rng.random_range(0..=32), flags: rng.random_range(0..2) }
    }

    fn random_glob(rng: &mut RoundRng, id: u16) -> GlobState {
        GlobState { id, x: rng.random(), y: rng.random(), to_x: rng.random(), to_y: rng.random(), tick: rng.random_range(0..200_000) }
    }

    fn random_ooze(rng: &mut RoundRng, cell: u16) -> OozeState {
        OozeState { cell, until: rng.random_range(0..200_000), total: [360, 720][rng.random_range(0..2)] }
    }

    fn random_keys(rng: &mut RoundRng, max: usize, range: u16) -> Vec<u16> {
        let n = rng.random_range(0..=max);
        let mut keys: Vec<u16> = (0..n).map(|_| rng.random_range(0..range)).collect();
        keys.sort();
        keys.dedup();
        keys
    }

    fn random_events(rng: &mut RoundRng) -> Vec<WireEvent> {
        (0..rng.random_range(0..4))
            .map(|_| match rng.random_range(0..3) {
                0 => WireEvent::Fired { slot: rng.random_range(0..40), weapon: WeaponKind::Shell, input_tick: 0 },
                1 => WireEvent::Wreck { slot: rng.random_range(0..40), x: rng.random(), y: rng.random() },
                _ => WireEvent::CookOff { x: rng.random(), y: rng.random() },
            })
            .collect()
    }

    fn random_snapshot(rng: &mut RoundRng) -> Snapshot {
        let mut acked = [0u32; MAX_SEATS];
        for a in &mut acked {
            *a = rng.random_range(0..100_000);
        }
        let mut mailbox = [0u8; MAX_SEATS];
        for m in &mut mailbox {
            *m = rng.random_range(0..8) | if rng.random_ratio(1, 5) { crate::net::mailbox::STARVED_BIT } else { 0 };
        }
        let tanks = random_keys(rng, 12, 40).into_iter().map(|id| random_tank(rng, id)).collect();
        let shots = random_keys(rng, 30, 400).into_iter().map(|id| random_shot(rng, id)).collect();
        let missiles: Vec<MissileState> = random_keys(rng, 8, 400)
            .into_iter()
            .map(|id| MissileState {
                id,
                x: rng.random(),
                y: rng.random(),
                height: rng.random_range(0..2000),
                facing: rng.random(),
                heading: rng.random(),
                tube: rng.random_range(0..4),
                dead: rng.random(),
            })
            .collect();
        let grenades: Vec<GrenadeState> = random_keys(rng, 6, 400)
            .into_iter()
            .map(|id| GrenadeState { id, x: rng.random(), y: rng.random(), height: rng.random_range(0..400), fuse: rng.random_range(0..3000) })
            .collect();
        let drones: Vec<DroneState> = random_keys(rng, 6, 400)
            .into_iter()
            .map(|id| DroneState {
                id,
                x: rng.random(),
                y: rng.random(),
                height: rng.random_range(0..200),
                heading: rng.random(),
                stage: rng.random_range(0..4),
                halo: rng.random_range(0..6),
                owner: rng.random_range(0..40),
                lock: rng.random(),
            })
            .collect();
        let mut bongs = Vec::new();
        for side in [Side::Player, Side::Enemy] {
            if rng.random::<bool>() {
                bongs.push(random_bong(rng, side));
            }
        }
        let bonus_pickups = random_keys(rng, 3, 600)
            .into_iter()
            .map(|cell| BonusPickup { cell, kind: if rng.random() { PickupKind::Shield } else { PickupKind::BongHealth } })
            .collect();
        let tiles = random_keys(rng, 20, 600)
            .into_iter()
            .map(|cell| TileState { cell, hp: rng.random(), flags: rng.random(), faces: rng.random_range(0..16), chunks: rng.random(), cage: rng.random_range(0..3) })
            .collect();
        let fires = random_keys(rng, 10, 600).into_iter().map(|cell| FireState { cell, left: rng.random(), lava: rng.random() }).collect();
        let lamps = random_keys(rng, 4, 200)
            .into_iter()
            .map(|id| LampState { id, x: rng.random(), y: rng.random(), seat: rng.random_range(0..8) })
            .collect();
        let crates = random_keys(rng, 4, 600)
            .into_iter()
            .map(|cell| CrateState { cell, hp: rng.random(), flags: rng.random_range(0..4), left: rng.random(), dx: rng.random_range(-400..400), dy: rng.random_range(-400..400) })
            .collect();
        let zones = random_keys(rng, 3, 400)
            .into_iter()
            .map(|id| ZoneState { id, kind: rng.random_range(0..2), x: rng.random(), y: rng.random(), until: rng.random_range(0..200_000), owner: rng.random_range(0..40), cell: rng.random_range(0..600), stage: rng.random_range(0..2) })
            .collect();
        let craters = random_keys(rng, 3, 600).into_iter().map(|cell| CraterState { cell, tick: rng.random_range(0..200_000) }).collect();
        let rubble = random_keys(rng, 4, 600).into_iter().map(|cell| RubbleState { cell, units: rng.random_range(1..64) }).collect();
        let well_drums = random_keys(rng, 3, 400)
            .into_iter()
            .map(|id| WellDrumState { id, well: rng.random_range(0..400), cell: rng.random_range(0..600), drum: rng.random_range(0..2), fused: rng.random(), tick: rng.random_range(0..200_000) })
            .collect();
        let loose = random_keys(rng, 4, 600).into_iter().map(|home| LooseState { home, x: rng.random(), y: rng.random() }).collect();
        let air = random_keys(rng, 3, 400).into_iter().map(|id| random_air(rng, id)).collect();
        let volcano_shifts = if rng.random() { vec![rng.random_range(-4000..4000)] } else { Vec::new() };
        let smoke_cells = random_keys(rng, 3, 600);
        let grass_flat = random_keys(rng, 6, 600).into_iter().map(|cell| GrassFlatState { cell, until: rng.random_range(0..200_000) }).collect();
        let towers = random_keys(rng, 8, 600).into_iter().map(|cell| random_tower(rng, cell)).collect();
        let globs = random_keys(rng, 3, 400).into_iter().map(|id| random_glob(rng, id)).collect();
        let ooze = random_keys(rng, 9, 600).into_iter().map(|cell| random_ooze(rng, cell)).collect();
        let coats = random_keys(rng, 3, 40).into_iter().map(|id| CoatState { id, left: rng.random_range(1..=40) }).collect();
        let mut s = Snapshot {
            tick: rng.random_range(0..200_000),
            server_ms: rng.random(),
            acked,
            mailbox,
            tanks,
            shots,
            missiles,
            grenades,
            drones,
            bongs,
            pickups: rng.random(),
            bonus_pickups,
            tiles,
            fires,
            lamps,
            crates,
            zones,
            craters,
            rubble,
            well_drums,
            loose,
            air,
            volcano_shifts,
            smoke_cells,
            grass_flat,
            towers,
            globs,
            ooze,
            coats,
            round: RoundState {
                wave: rng.random_range(0..10),
                alive: rng.random_range(0..40),
                pending: rng.random_range(0..40),
                intro: rng.random_range(0..30),
                next_wave: rng.random_range(0..30),
                restart: rng.random_range(0..30),
                outcome: [RoundOutcome::Playing, RoundOutcome::Won, RoundOutcome::Lost][rng.random_range(0..3)],
                lamps_out: rng.random_range(0..100),
            },
            events: random_events(rng),
        };
        s.normalise();
        s
    }

    /// `prev` one interval later: most entries nudged a short way, a few
    /// changed outright, some removed, some added.
    fn mutate(rng: &mut RoundRng, prev: &Snapshot) -> Snapshot {
        let mut next = prev.clone();
        next.tick += 3;
        next.server_ms = next.server_ms.wrapping_add(50);
        if rng.random_ratio(1, 3) {
            next.acked[rng.random_range(0..MAX_SEATS)] += 3;
        }
        if rng.random_ratio(1, 3) {
            next.mailbox[rng.random_range(0..MAX_SEATS)] = rng.random_range(0..8);
        }
        for t in &mut next.tanks {
            match rng.random_range(0..10) {
                0 => *t = random_tank(rng, t.id),
                1 => t.hp = t.hp.saturating_sub(7),
                2 => {}
                _ => {
                    t.x = t.x.wrapping_add(rng.random_range(-128..=127));
                    t.y = t.y.wrapping_add(rng.random_range(-128..=127));
                }
            }
        }
        for s in &mut next.shots {
            match rng.random_range(0..10) {
                0 => s.state += 1,
                1 => {
                    s.x = s.x.wrapping_add(rng.random_range(-2000..2000));
                }
                _ => {
                    s.x = s.x.wrapping_add(rng.random_range(-100..=100));
                    s.y = s.y.wrapping_add(rng.random_range(-100..=100));
                }
            }
        }
        for f in &mut next.bongs {
            if rng.random_ratio(1, 4) {
                f.x = f.x.wrapping_add(rng.random_range(-20..=20));
            }
        }
        next.tanks.retain(|_| !rng.random_ratio(1, 10));
        next.shots.retain(|_| !rng.random_ratio(1, 5));
        next.tiles.retain(|_| !rng.random_ratio(1, 8));
        next.fires.retain(|_| !rng.random_ratio(1, 4));
        for id in random_keys(rng, 2, 40) {
            next.tanks.push(random_tank(rng, id));
        }
        for id in random_keys(rng, 4, 400) {
            next.shots.push(random_shot(rng, id));
        }
        for cell in random_keys(rng, 3, 600) {
            next.tiles.push(TileState { cell, hp: rng.random(), flags: 0, faces: 0, chunks: 0, cage: 0 });
        }
        for cell in random_keys(rng, 2, 600) {
            next.fires.push(FireState { cell, left: rng.random(), lava: rng.random() });
        }
        next.lamps.retain(|_| !rng.random_ratio(1, 6));
        for id in random_keys(rng, 1, 200) {
            next.lamps.push(LampState { id, x: rng.random(), y: rng.random(), seat: rng.random_range(0..8) });
        }
        next.crates.retain(|_| !rng.random_ratio(1, 3));
        for cell in random_keys(rng, 1, 600) {
            next.crates.push(CrateState { cell, hp: rng.random(), flags: rng.random_range(0..4), left: rng.random(), dx: rng.random_range(-400..400), dy: rng.random_range(-400..400) });
        }
        if rng.random_ratio(1, 5) {
            next.pickups ^= 1 << rng.random_range(0..64);
        }
        if rng.random_ratio(1, 5) {
            next.bonus_pickups.push(BonusPickup { cell: rng.random_range(0..600), kind: PickupKind::Shield });
        }
        if rng.random_ratio(1, 5) {
            next.round.alive = next.round.alive.wrapping_add(1);
        }
        next.zones.retain(|_| !rng.random_ratio(1, 3));
        for id in random_keys(rng, 1, 400) {
            next.zones.push(ZoneState { id, kind: 0, x: rng.random(), y: rng.random(), until: rng.random_range(0..200_000), owner: 3, cell: rng.random_range(0..600), stage: 0 });
        }
        // A gravity well's zone, its stage turning, and the drums it holds.
        for zone in next.zones.iter_mut() {
            if rng.random_ratio(1, 4) {
                zone.kind = crate::zone::ZONE_WELL;
                zone.stage = rng.random_range(0..2);
            }
        }
        // Air support: a call starting its count, an aircraft moving a
        // little or a lot, rolling into a bank, spending its ammunition,
        // one gone, one new.
        next.air.retain(|_| !rng.random_ratio(1, 4));
        for a in next.air.iter_mut() {
            if rng.random_ratio(1, 2) {
                a.x = a.x.wrapping_add(rng.random_range(-40..40));
                a.y = a.y.wrapping_add(rng.random_range(-40..40));
            } else if rng.random_ratio(1, 3) {
                a.stage = rng.random_range(0..7);
                a.arrive = rng.random_range(0..200_000);
            }
            if rng.random_ratio(1, 3) {
                a.bank = rng.random_range(-127..=127);
            }
            if rng.random_ratio(1, 4) {
                a.ammo = a.ammo.saturating_sub(1);
            }
        }
        for id in random_keys(rng, 1, 400) {
            next.air.push(random_air(rng, id));
        }
        next.well_drums.retain(|_| !rng.random_ratio(1, 3));
        for id in random_keys(rng, 1, 400) {
            next.well_drums.push(WellDrumState { id, well: rng.random(), cell: rng.random_range(0..600), drum: rng.random_range(0..2), fused: rng.random(), tick: rng.random_range(0..200_000) });
        }
        // A drum shoved on, one back on its own cell, one shoved off it.
        for d in next.loose.iter_mut() {
            if rng.random_ratio(1, 2) {
                d.x = d.x.wrapping_add(rng.random_range(-40..40));
            }
        }
        next.loose.retain(|_| !rng.random_ratio(1, 4));
        for home in random_keys(rng, 1, 600) {
            next.loose.push(LooseState { home, x: rng.random(), y: rng.random() });
        }
        for cell in random_keys(rng, 1, 600) {
            next.craters.push(CraterState { cell, tick: next.tick });
        }
        for cell in random_keys(rng, 1, 600) {
            next.rubble.retain(|r| r.cell != cell);
            next.rubble.push(RubbleState { cell, units: rng.random_range(1..64) });
        }
        if rng.random_ratio(1, 6) {
            next.volcano_shifts = vec![rng.random_range(-4000..4000)];
        }
        if rng.random_ratio(1, 4) {
            next.smoke_cells = random_keys(rng, 3, 600);
        }
        // Grass standing back up, and grass flattened (again).
        next.grass_flat.retain(|_| !rng.random_ratio(1, 4));
        for cell in random_keys(rng, 2, 600) {
            next.grass_flat.push(GrassFlatState { cell, until: next.tick + rng.random_range(1..600) });
        }
        // Towers turning, charging and catching fire, one gone with its
        // tile; a glob landed and one lobbed; ooze dried, splashed again
        // and new; coats running down, washed off and new.
        for tower in next.towers.iter_mut() {
            match rng.random_range(0..4) {
                0 => tower.heading = tower.heading.wrapping_add(rng.random_range(1..6)),
                1 => tower.charge = rng.random_range(0..=32),
                2 if rng.random_ratio(1, 4) => tower.flags ^= crate::net::wire::tower_flags::BURNING,
                _ => {}
            }
        }
        next.towers.retain(|_| !rng.random_ratio(1, 10));
        next.globs.retain(|_| !rng.random_ratio(1, 3));
        for id in random_keys(rng, 1, 400) {
            next.globs.push(random_glob(rng, id));
        }
        next.ooze.retain(|_| !rng.random_ratio(1, 4));
        for o in next.ooze.iter_mut() {
            if rng.random_ratio(1, 4) {
                o.until = next.tick + rng.random_range(1..720);
            }
        }
        for cell in random_keys(rng, 3, 600) {
            next.ooze.push(random_ooze(rng, cell));
        }
        for c in next.coats.iter_mut() {
            c.left = c.left.saturating_sub(rng.random_range(0..2)).max(1);
        }
        next.coats.retain(|_| !rng.random_ratio(1, 4));
        for id in random_keys(rng, 1, 40) {
            next.coats.push(CoatState { id, left: 40 });
        }
        next.events = random_events(rng);
        // Later pushes of an existing key must win, the way the server's
        // fresh state would; normalise keeps the first, so drop the older.
        next.globs.reverse();
        next.ooze.reverse();
        next.coats.reverse();
        next.tanks.reverse();
        next.shots.reverse();
        next.tiles.reverse();
        next.fires.reverse();
        next.lamps.reverse();
        next.crates.reverse();
        next.well_drums.reverse();
        next.loose.reverse();
        next.grass_flat.reverse();
        next.normalise();
        next
    }

    #[test]
    fn applying_a_delta_reproduces_the_next_snapshot() {
        for seed in 0..300u64 {
            let mut rng = RoundRng::seed_from_u64(seed);
            let a = random_snapshot(&mut rng);
            let b = mutate(&mut rng, &a);
            let d = delta(&a, &b);
            assert_eq!(apply_delta(&a, &d), b, "seed {seed}: mutated pair");
            let c = random_snapshot(&mut rng);
            let d = delta(&a, &c);
            assert_eq!(apply_delta(&a, &d), c, "seed {seed}: unrelated pair");
            let d = delta(&b, &b);
            assert_eq!(apply_delta(&b, &d), b, "seed {seed}: identity");
        }
    }

    #[test]
    fn a_delta_survives_the_wire() {
        let mut rng = RoundRng::seed_from_u64(42);
        let a = random_snapshot(&mut rng);
        let b = mutate(&mut rng, &a);
        let d = delta(&a, &b);
        let bytes = postcard::to_stdvec(&d).unwrap();
        let back: SnapshotDelta = postcard::from_bytes(&bytes).unwrap();
        assert_eq!(back, d);
        assert_eq!(apply_delta(&a, &back), b);
    }

    #[test]
    fn an_identical_snapshot_costs_only_the_header() {
        let mut rng = RoundRng::seed_from_u64(7);
        let a = random_snapshot(&mut rng);
        let mut b = a.clone();
        b.events.clear();
        let d = delta(&a, &b);
        assert!(d.tanks.is_empty() && d.tanks_moved.is_empty() && d.tanks_gone.is_empty());
        assert!(d.shots.is_empty() && d.tiles.is_empty() && d.fires.is_empty());
        assert_eq!(d.pickups, None);
        assert_eq!(d.round, None);
        assert_eq!(d.bonus_pickups, None);
        assert_eq!(d.towers, None);
    }

    /// Twenty towers standing idle cost a delta one byte for all of the
    /// towers' families; one turning travels alone, without the ooze's
    /// lists, and a glob, a puddle and a coat each go once, when they come.
    #[test]
    fn idle_towers_cost_a_byte_and_a_turning_one_travels_alone() {
        let towers: Vec<TowerState> = (0..20u16).map(|i| TowerState { cell: 100 + i * 37, heading: 64, charge: 32, flags: 0 }).collect();
        let a = Snapshot { towers, ..Default::default() };
        let idle = delta(&a, &a);
        assert_eq!(idle.towers, None);
        let bare = encode(&Msg::Delta(delta(&Snapshot::default(), &Snapshot::default()))).len();
        assert_eq!(encode(&Msg::Delta(idle)).len(), bare, "idle towers cost what no towers do");
        let mut b = a.clone();
        b.towers[7].heading = 66;
        b.towers[3].charge = 5;
        let d = delta(&a, &b);
        let t = d.towers.as_ref().expect("two towers changed");
        assert_eq!(t.towers, vec![b.towers[3], b.towers[7]]);
        assert!(t.towers_gone.is_empty() && t.ooze.is_none(), "a turret's turn carries none of the ooze's lists");
        assert_eq!(apply_delta(&a, &d), b);
        // A lob: its glob, then a splash's puddles and the coat it put on.
        let mut c = b.clone();
        c.globs.push(GlobState { id: 9, x: 400, y: 400, to_x: 900, to_y: 600, tick: 100 });
        let d = delta(&b, &c);
        let t = d.towers.as_ref().expect("a lob");
        assert!(t.towers.is_empty());
        assert_eq!(t.ooze.as_ref().map(|o| o.globs.len()), Some(1));
        let mut e = c.clone();
        e.globs.clear();
        e.ooze = vec![OozeState { cell: 300, until: 460, total: 360 }, OozeState { cell: 301, until: 460, total: 360 }];
        e.coats = vec![CoatState { id: 0, left: 40 }];
        let d = delta(&c, &e);
        let o = d.towers.as_ref().and_then(|t| t.ooze.as_ref()).expect("a splash");
        assert_eq!((o.globs_gone.clone(), o.ooze.len(), o.coats.len()), (vec![9], 2, 1));
        let back: SnapshotDelta = postcard::from_bytes(&postcard::to_stdvec(&d).unwrap()).unwrap();
        assert_eq!(apply_delta(&c, &back), e);
        // The puddles drying on their own clock change nothing.
        let mut f = e.clone();
        f.tick += 60;
        assert_eq!(delta(&e, &f).towers, None);
    }

    #[test]
    fn a_short_move_travels_as_three_bytes_and_a_long_one_in_full() {
        let a = Snapshot { tanks: vec![TankState { id: 4, x: 100, y: 100, ..Default::default() }], ..Default::default() };
        let mut b = a.clone();
        b.tanks[0].x += 127;
        b.tanks[0].y -= 128;
        let d = delta(&a, &b);
        assert_eq!(d.tanks_moved, vec![Moved { key: 4, dx: 127, dy: -128 }]);
        assert!(d.tanks.is_empty());
        assert_eq!(apply_delta(&a, &d), b);
        let mut c = a.clone();
        c.tanks[0].x += 128;
        let d = delta(&a, &c);
        assert!(d.tanks_moved.is_empty());
        assert_eq!(d.tanks, c.tanks);
        let mut e = b.clone();
        e.tanks[0].hp = 3;
        let d = delta(&a, &e);
        assert!(d.tanks_moved.is_empty(), "a moved tank that also changed goes in full");
        assert_eq!(apply_delta(&a, &d), e);
    }

    /// An aircraft flying level and straight costs a move; one that rolled
    /// into a bank or spent ammunition on the way goes in full, and either
    /// way the next snapshot is rebuilt whole.
    #[test]
    fn an_aircraft_that_banks_or_spends_ammunition_travels_in_full() {
        let craft = AirState { id: 12, seat: 1, sortie: 1, stage: 6, arrive: 0, x: 400, y: 900, height: 380, heading: 64, bank: 0, ammo: 3 };
        let a = Snapshot { air: vec![craft], ..Default::default() };
        let mut b = a.clone();
        b.air[0].x += 20;
        let d = delta(&a, &b);
        assert_eq!(d.air_moved, vec![Moved { key: 12, dx: 20, dy: 0 }]);
        assert!(d.air.is_empty());
        assert_eq!(apply_delta(&a, &d), b);
        for change in [AirState { bank: -90, ..b.air[0] }, AirState { ammo: 2, ..b.air[0] }] {
            let mut c = b.clone();
            c.air[0] = change;
            let d = delta(&a, &c);
            assert!(d.air_moved.is_empty(), "{change:?}");
            assert_eq!(d.air, c.air);
            assert_eq!(apply_delta(&a, &d), c);
            let back: SnapshotDelta = postcard::from_bytes(&postcard::to_stdvec(&d).unwrap()).unwrap();
            assert_eq!(apply_delta(&a, &back), c);
        }
    }

    /// The snapshot the PRD sizes: eight tanks and twenty-four shots on
    /// the default field, both bongs, nothing damaged yet.
    fn busy_snapshot() -> Snapshot {
        let mut rng = RoundRng::seed_from_u64(0xB0B5);
        let tanks = (0..8u16)
            .map(|id| TankState {
                id,
                row: (id % 12) as u8,
                x: quantise_pos(rng.random_range(64.0..1024.0)),
                y: quantise_pos(rng.random_range(64.0..480.0)),
                vx: quantise_velocity(rng.random_range(-210.0..210.0)),
                vy: quantise_velocity(rng.random_range(-210.0..210.0)),
                dir: rng.random_range(0..4),
                hp: rng.random_range(30..=100),
                shield: 0,
                flags: 0,
                weapon: WeaponKind::Shell,
                ammo: rng.random_range(5..30),
                tell: 0,
                skid: 0,
                disabled: 0,
                offline: 0,
                shells: 10,
                charge: 0,
                reticle: 0,
                track: 0,
                target: 0,
            })
            .collect();
        let shots = (0..24u16)
            .map(|id| ShotState {
                id: 1000 + id,
                kind: ShotKind::Shell,
                x: quantise_pos(rng.random_range(32.0..1056.0)),
                y: quantise_pos(rng.random_range(32.0..512.0)),
                heading: rng.random(),
                state: 3,
                variant: (id % 18) as u8, owner: 255,
            })
            .collect();
        let bongs = vec![
            BongState { side: Side::Player, x: quantise_pos(544.0), y: quantise_pos(272.0), hp: 40, state: 0, phase: 0, hop_x: 0, hop_y: 0 },
            BongState { side: Side::Enemy, x: quantise_pos(96.0), y: quantise_pos(96.0), hp: 40, state: 0, phase: 0, hop_x: 0, hop_y: 0 },
        ];
        let mut s = Snapshot {
            tick: 3600,
            server_ms: 60_000,
            tanks,
            shots,
            bongs,
            pickups: 0b1011,
            round: RoundState { wave: 2, alive: 7, pending: 4, intro: 0, next_wave: 0, restart: 0, outcome: RoundOutcome::Playing, lamps_out: 0 },
            ..Default::default()
        };
        s.normalise();
        s
    }

    /// The measured sizes of the module docs. Loose ceilings guard against
    /// an accidental widening; the exact numbers print under
    /// `--nocapture`.
    #[test]
    fn sizes_of_the_prd_snapshot() {
        let a = busy_snapshot();
        let full = encode(&Msg::Snapshot(a.clone())).len();
        // One interval later: every hull moved 10 px at the default speed
        // and every shell 25 px, nothing else changed.
        let mut b = a.clone();
        b.tick += 3;
        b.server_ms += 50;
        for t in &mut b.tanks {
            t.x += 42;
        }
        for s in &mut b.shots {
            s.y -= 100;
        }
        let moving = encode(&Msg::Delta(delta(&a, &b))).len();
        // A busier interval on top: two hulls hit, a shield spent, four
        // shells landed and four new ones fired, one event each.
        let mut c = b.clone();
        c.tick += 3;
        c.tanks[1].hp -= 12;
        c.tanks[1].flags = crate::net::wire::tank_flags::HIT;
        c.tanks[5].shield = 20;
        c.tanks[5].flags = crate::net::wire::tank_flags::SHIELD;
        for t in &mut c.tanks {
            t.x += 42;
        }
        c.shots.drain(0..4);
        for s in &mut c.shots {
            s.y -= 100;
        }
        for id in 0..4u16 {
            c.shots.push(ShotState { id: 2000 + id, kind: ShotKind::Shell, x: 400, y: 800, heading: 64, state: 0, variant: 3, owner: 255 });
        }
        c.normalise();
        c.events = vec![
            WireEvent::Hit { target: crate::net::events::WireHitTarget::Enemy { slot: 1 }, damage: 12.0, killed: false, x: 400, y: 800, cause: crate::simulation::HitCause::Shell, by: None },
            WireEvent::Fired { slot: 0, weapon: WeaponKind::Shell, input_tick: 0 },
        ];
        let busy = encode(&Msg::Delta(delta(&b, &c))).len();
        let idle = encode(&Msg::Delta(delta(&a, &a))).len();
        println!("snapshot sizes: full {full} B, delta moving {moving} B, delta busy {busy} B, delta idle {idle} B");
        // Every delta carries `acked` and `mailbox` whole: eight
        // varints and eight bytes, the header the idle bound is, and a
        // length byte for each keyed family's lists, the grenades' and the
        // drones' three included. A new shot costs one more byte for its owner
        // (protocol 8); every tank two for its tell and its skid
        // (protocol 16), and three for an EMP's two outages and its
        // magazine, the round one for the lamps (protocol 17); every tank
        // one for its charge (protocol 18); the drones' family three
        // (protocol 19); every tank one for its reticle and the zones', the
        // craters' and the volcanoes' shifts' lists one each, the delta two
        // for the zones and two for the craters and one for the shifts
        // (protocol 20); the held drums' list one, the delta two for them
        // (protocol 21); the rubble's list one, the delta two for it
        // (protocol 23); the smoke cells' list one, the delta one for
        // them (protocol 29); air support's list one, the delta three for it
        // (protocol 30); the flattened grass's list one, the delta two for
        // it (protocol 31); the loose drums' list one, the delta two for
        // them (protocol 37); the towers', the globs', the ooze's and the
        // coats' lists one each, the delta one for all four while none of
        // them changed (protocol 41); every tank two for its minigun
        // turret's swing and target (protocol 44).
        assert!(full <= 524, "full snapshot {full} B");
        assert!(moving <= 218, "moving delta {moving} B");
        assert!(busy <= 284, "busy delta {busy} B");
        assert!(idle <= 70, "idle delta {idle} B");
    }
}
