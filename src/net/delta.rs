//! Delta encoding of a `Snapshot` against the previous one. The baseline
//! is the last snapshot sent: a WebSocket loses nothing, so the server
//! keeps one `Snapshot` per room and the client one per socket, and both
//! step them with `apply_delta`. `Welcome` carries the first full one.
//!
//! Per keyed family (`tanks`, `shots`, `missiles`, `frogs`, `tiles`, `fires`) a delta
//! carries three lists: entries that are new or changed (in full),
//! entries that only moved by less than 32 px on each axis (`Moved`: the
//! key and two `i8` quarter-pixel steps, the common case for every hull
//! and projectile between two snapshots 50 ms apart), and the keys that
//! disappeared. Unchanged entries cost nothing. The scalar families
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
    BonusPickup, FireState, FrogState, MissileState, RoundState, ShotState, Snapshot, TankState, TileState, side_code,
};

/// An entry that kept every field but its position, which moved by
/// (`dx`, `dy`) quarter pixels since the previous snapshot.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Moved {
    /// The family's key: a tank's or shot's id, a frog's `side_code`.
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
    /// New or changed frogs, in full.
    pub frogs: Vec<FrogState>,
    pub frogs_moved: Vec<Moved>,
    /// Keys are `side_code`s.
    pub frogs_gone: Vec<u16>,
    pub pickups: Option<u64>,
    pub bonus_pickups: Option<Vec<BonusPickup>>,
    /// New or changed tiles, in full.
    pub tiles: Vec<TileState>,
    pub tiles_gone: Vec<u16>,
    /// New or changed fires, in full.
    pub fires: Vec<FireState>,
    pub fires_gone: Vec<u16>,
    pub round: Option<RoundState>,
    pub events: Vec<WireEvent>,
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

impl Keyed for FrogState {
    fn key(&self) -> u16 {
        side_code(self.side) as u16
    }
}

impl Positioned for FrogState {
    fn xy(&self) -> (i16, i16) {
        (self.x, self.y)
    }

    fn with_xy(&self, x: i16, y: i16) -> Self {
        FrogState { x, y, ..*self }
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
    let (frogs, frogs_moved, frogs_gone) = diff_positioned(&prev.frogs, &next.frogs);
    let (tiles, tiles_gone) = diff_keyed(&prev.tiles, &next.tiles);
    let (fires, fires_gone) = diff_keyed(&prev.fires, &next.fires);
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
        frogs,
        frogs_moved,
        frogs_gone,
        pickups: (next.pickups != prev.pickups).then_some(next.pickups),
        bonus_pickups: (bonus_pickups != prev_bonus).then_some(bonus_pickups),
        tiles,
        tiles_gone,
        fires,
        fires_gone,
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
    Snapshot {
        tick: delta.tick,
        server_ms: delta.server_ms,
        acked: delta.acked,
        mailbox: delta.mailbox,
        tanks: apply_positioned(&prev.tanks, &delta.tanks, &delta.tanks_moved, &delta.tanks_gone),
        shots: apply_positioned(&prev.shots, &delta.shots, &delta.shots_moved, &delta.shots_gone),
        missiles: apply_positioned(&prev.missiles, &delta.missiles, &delta.missiles_moved, &delta.missiles_gone),
        frogs: apply_positioned(&prev.frogs, &delta.frogs, &delta.frogs_moved, &delta.frogs_gone),
        pickups: delta.pickups.unwrap_or(prev.pickups),
        bonus_pickups,
        tiles: apply_keyed(&prev.tiles, &delta.tiles, &delta.tiles_gone),
        fires: apply_keyed(&prev.fires, &delta.fires, &delta.fires_gone),
        round: delta.round.unwrap_or(prev.round),
        events: delta.events.clone(),
    }
}

#[cfg(test)]
mod tests {
    use rand::rngs::SmallRng;
    use rand::{RngExt, SeedableRng};

    use super::*;
    use crate::frog::Side;
    use crate::net::codec::{Msg, encode};
    use crate::net::wire::{RoundOutcome, ShotKind, WeaponKind, quantise_pos, quantise_velocity};
    use crate::pickup::PickupKind;

    fn random_tank(rng: &mut SmallRng, id: u16) -> TankState {
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
        }
    }

    fn random_shot(rng: &mut SmallRng, id: u16) -> ShotState {
        ShotState {
            id,
            kind: [ShotKind::Shell, ShotKind::Bullet, ShotKind::Plasma][rng.random_range(0..3)],
            x: rng.random_range(-4000..4000),
            y: rng.random_range(-4000..4000),
            heading: rng.random(),
            state: rng.random_range(0..8),
            variant: rng.random_range(0..18),
        }
    }

    fn random_frog(rng: &mut SmallRng, side: Side) -> FrogState {
        FrogState {
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

    fn random_keys(rng: &mut SmallRng, max: usize, range: u16) -> Vec<u16> {
        let n = rng.random_range(0..=max);
        let mut keys: Vec<u16> = (0..n).map(|_| rng.random_range(0..range)).collect();
        keys.sort();
        keys.dedup();
        keys
    }

    fn random_events(rng: &mut SmallRng) -> Vec<WireEvent> {
        (0..rng.random_range(0..4))
            .map(|_| match rng.random_range(0..3) {
                0 => WireEvent::Fired { slot: rng.random_range(0..40), weapon: WeaponKind::Shell },
                1 => WireEvent::Wreck { slot: rng.random_range(0..40), x: rng.random(), y: rng.random() },
                _ => WireEvent::CookOff { x: rng.random(), y: rng.random() },
            })
            .collect()
    }

    fn random_snapshot(rng: &mut SmallRng) -> Snapshot {
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
            })
            .collect();
        let mut frogs = Vec::new();
        for side in [Side::Player, Side::Enemy] {
            if rng.random::<bool>() {
                frogs.push(random_frog(rng, side));
            }
        }
        let bonus_pickups = random_keys(rng, 3, 600)
            .into_iter()
            .map(|cell| BonusPickup { cell, kind: if rng.random() { PickupKind::Shield } else { PickupKind::FrogHealth } })
            .collect();
        let tiles = random_keys(rng, 20, 600)
            .into_iter()
            .map(|cell| TileState { cell, hp: rng.random(), flags: rng.random(), faces: rng.random_range(0..16) })
            .collect();
        let fires = random_keys(rng, 10, 600).into_iter().map(|cell| FireState { cell, left: rng.random() }).collect();
        let mut s = Snapshot {
            tick: rng.random_range(0..200_000),
            server_ms: rng.random(),
            acked,
            mailbox,
            tanks,
            shots,
            missiles,
            frogs,
            pickups: rng.random(),
            bonus_pickups,
            tiles,
            fires,
            round: RoundState {
                wave: rng.random_range(0..10),
                alive: rng.random_range(0..40),
                pending: rng.random_range(0..40),
                intro: rng.random_range(0..30),
                next_wave: rng.random_range(0..30),
                restart: rng.random_range(0..30),
                outcome: [RoundOutcome::Playing, RoundOutcome::Won, RoundOutcome::Lost][rng.random_range(0..3)],
            },
            events: random_events(rng),
        };
        s.normalise();
        s
    }

    /// `prev` one interval later: most entries nudged a short way, a few
    /// changed outright, some removed, some added.
    fn mutate(rng: &mut SmallRng, prev: &Snapshot) -> Snapshot {
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
        for f in &mut next.frogs {
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
            next.tiles.push(TileState { cell, hp: rng.random(), flags: 0, faces: 0 });
        }
        for cell in random_keys(rng, 2, 600) {
            next.fires.push(FireState { cell, left: rng.random() });
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
        next.events = random_events(rng);
        // Later pushes of an existing key must win, the way the server's
        // fresh state would; normalise keeps the first, so drop the older.
        next.tanks.reverse();
        next.shots.reverse();
        next.tiles.reverse();
        next.fires.reverse();
        next.normalise();
        next
    }

    #[test]
    fn applying_a_delta_reproduces_the_next_snapshot() {
        for seed in 0..300u64 {
            let mut rng = SmallRng::seed_from_u64(seed);
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
        let mut rng = SmallRng::seed_from_u64(42);
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
        let mut rng = SmallRng::seed_from_u64(7);
        let a = random_snapshot(&mut rng);
        let mut b = a.clone();
        b.events.clear();
        let d = delta(&a, &b);
        assert!(d.tanks.is_empty() && d.tanks_moved.is_empty() && d.tanks_gone.is_empty());
        assert!(d.shots.is_empty() && d.tiles.is_empty() && d.fires.is_empty());
        assert_eq!(d.pickups, None);
        assert_eq!(d.round, None);
        assert_eq!(d.bonus_pickups, None);
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

    /// The snapshot the PRD sizes: eight tanks and twenty-four shots on
    /// the default field, both frogs, nothing damaged yet.
    fn busy_snapshot() -> Snapshot {
        let mut rng = SmallRng::seed_from_u64(0xB0B5);
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
                variant: (id % 18) as u8,
            })
            .collect();
        let frogs = vec![
            FrogState { side: Side::Player, x: quantise_pos(544.0), y: quantise_pos(272.0), hp: 40, state: 0, phase: 0, hop_x: 0, hop_y: 0 },
            FrogState { side: Side::Enemy, x: quantise_pos(96.0), y: quantise_pos(96.0), hp: 40, state: 0, phase: 0, hop_x: 0, hop_y: 0 },
        ];
        let mut s = Snapshot {
            tick: 3600,
            server_ms: 60_000,
            tanks,
            shots,
            frogs,
            pickups: 0b1011,
            round: RoundState { wave: 2, alive: 7, pending: 4, intro: 0, next_wave: 0, restart: 0, outcome: RoundOutcome::Playing },
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
            c.shots.push(ShotState { id: 2000 + id, kind: ShotKind::Shell, x: 400, y: 800, heading: 64, state: 0, variant: 3 });
        }
        c.normalise();
        c.events = vec![
            WireEvent::Hit { target: crate::net::events::WireHitTarget::Enemy { slot: 1 }, damage: 12.0, killed: false, x: 400, y: 800 },
            WireEvent::Fired { slot: 0, weapon: WeaponKind::Shell },
        ];
        let busy = encode(&Msg::Delta(delta(&b, &c))).len();
        let idle = encode(&Msg::Delta(delta(&a, &a))).len();
        println!("snapshot sizes: full {full} B, delta moving {moving} B, delta busy {busy} B, delta idle {idle} B");
        // Every delta carries `acked` and `mailbox` whole: eight
        // varints and eight bytes, the header the idle bound is.
        assert!(full <= 420, "full snapshot {full} B");
        assert!(moving <= 210, "moving delta {moving} B");
        assert!(busy <= 270, "busy delta {busy} B");
        assert!(idle <= 48, "idle delta {idle} B");
    }
}
