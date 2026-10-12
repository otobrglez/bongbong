//! Air support (docs/air-support.md), headless: the call a seat's crate
//! makes, the aircraft that answers it, its flight and the geometry of its
//! attack. A crate (`PickupKind::AirHeli`, `PickupKind::AirJet`) puts an
//! `AirCall` on `Game::air_calls`; when its count runs out an `Aircraft`
//! flies in from the field's edge, attacks the enemies round the seat and
//! leaves. The world half is `simulation/air_support.rs`; the sprite is
//! `render/air_support.rs`.
//!
//! The rules every reader keeps:
//!
//! 1. no RNG: the edge it comes in from is hashed from the call's id, the
//!    targets are sorted by distance and then slot, and everything else is
//!    the round clock and the fixed step;
//! 2. a call and an aircraft belong to a seat, and only enemies are hurt
//!    (`side_blast`, `Owner::same_side`);
//! 3. one sortie of a seat at a time: a call made while another of the seat
//!    is counting or in the air waits (`AirCall::arrive_at` `None`) and its
//!    count starts when that one is gone;
//! 4. walks go by id.

use serde::{Deserialize, Serialize};

use crate::math::Vec2;
use crate::pickup::PickupKind;
use crate::tuning::Tuning;
use crate::Position;

/// Which aircraft answers a call. The variant order is the wire's encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Sortie {
    /// An Apache-like gunship: hovers at a standoff and puts salvos of
    /// rockets into the nearest enemies. An air target.
    Heli,
    /// An A-10-like attack jet: one straight run, a stripe of cannon fire and
    /// two bombs through the enemies' middle. Too fast to hit.
    Jet,
}

impl Sortie {
    /// The sortie a crate of `kind` calls, `None` for every other crate.
    pub fn of(kind: PickupKind) -> Option<Sortie> {
        match kind {
            PickupKind::AirHeli => Some(Sortie::Heli),
            PickupKind::AirJet => Some(Sortie::Jet),
            _ => None,
        }
    }

    /// The crate that calls it: its glyph is the HUD's and the minimap's.
    pub fn pickup(self) -> PickupKind {
        match self {
            Sortie::Heli => PickupKind::AirHeli,
            Sortie::Jet => PickupKind::AirJet,
        }
    }

    /// Lower-case name for tooling and the probe.
    pub fn name(self) -> &'static str {
        match self {
            Sortie::Heli => "heli",
            Sortie::Jet => "jet",
        }
    }

    /// Its wire code (`net::wire::AirState::sortie`).
    pub fn code(self) -> u8 {
        match self {
            Sortie::Heli => 0,
            Sortie::Jet => 1,
        }
    }

    /// Inverse of `code`; an unknown code is the heli.
    pub fn from_code(code: u8) -> Sortie {
        if code == 1 { Sortie::Jet } else { Sortie::Heli }
    }

    /// How high it flies (px): what it is drawn lifted by and its shadow's
    /// offset.
    pub fn height(self, t: &Tuning) -> f32 {
        match self {
            Sortie::Heli => t.air_heli_height_px,
            Sortie::Jet => t.air_jet_height_px,
        }
    }

    /// How fast it flies (px/s) while it is not hovering.
    pub fn speed(self, t: &Tuning) -> f32 {
        match self {
            Sortie::Heli => t.air_heli_speed,
            Sortie::Jet => t.air_jet_speed,
        }
    }
}

/// A seat's call for air support (`Game::air_calls`), from the crate until
/// its aircraft arrives.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AirCall {
    /// From the round's projectile counter (`Game::take_shot_id`): its wire
    /// key, its walk order, and the aircraft's id when it arrives.
    pub id: u32,
    pub seat: u8,
    pub sortie: Sortie,
    /// When it arrives on the round clock; `None` while it waits for the
    /// seat's sortie before it (rule 3).
    pub arrive_at: Option<f32>,
}

impl AirCall {
    /// Seconds before it arrives at round time `now` (never under 0), `None`
    /// while it waits.
    pub fn left(&self, now: f32) -> Option<f32> {
        self.arrive_at.map(|at| (at - now).max(0.0))
    }
}

/// Where an aircraft is in its sortie. The variant order is the wire's
/// encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AirStage {
    /// Flying in from the edge.
    Inbound,
    /// The heli hovering and firing; the jet on its gun run.
    Attack,
    /// Leaving off the edge.
    Egress,
}

/// `net::wire::AirState::stage` for a call waiting behind another.
pub const AIR_WAITING: u8 = 0;

/// `net::wire::AirState::stage` for a call counting down.
pub const AIR_COUNTING: u8 = 1;

impl AirStage {
    /// Its wire code (`net::wire::AirState::stage`), past the calls' two.
    pub fn code(self) -> u8 {
        match self {
            AirStage::Inbound => 2,
            AirStage::Attack => 3,
            AirStage::Egress => 4,
        }
    }

    /// Inverse of `code`, `None` for a call's codes.
    pub fn from_code(code: u8) -> Option<AirStage> {
        match code {
            2 => Some(AirStage::Inbound),
            3 => Some(AirStage::Attack),
            4 => Some(AirStage::Egress),
            _ => None,
        }
    }
}

/// An aircraft on a sortie: a hecs component spawned alone, like a drone.
/// It has no physics body and the nav grid never sees it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Aircraft {
    /// Its call's id.
    pub id: u32,
    pub seat: u8,
    pub sortie: Sortie,
    pub stage: AirStage,
    /// The point on the ground under it.
    pub ground: Position,
    /// How high over that point it flies.
    pub height: f32,
    /// The way it faces (unit).
    pub heading: Vec2,
    /// Where it came in: the heli leaves the way it came.
    pub entry: Position,
    /// The heli's hover point; the jet's aim point, the middle of its run.
    pub mark: Position,
    /// Seconds in its current stage.
    pub stage_time: f32,
    /// The jet: px flown since its entry, which its gun and bombs are timed
    /// on.
    pub flown: f32,
    /// What it has fired: the heli's salvos, the jet's cannon bursts (the
    /// bombs count as one more).
    pub fired: u8,
    /// Whether it found enemies to attack when it arrived; an aircraft that
    /// found none passes over the seat and leaves without firing.
    pub armed: bool,
    /// Bullets that struck it (the heli; `air_heli_hits` bring it down).
    pub hits: u32,
}

impl Aircraft {
    /// The aircraft answering call `id` of `seat`, coming in at `entry` on
    /// its way to `mark`.
    pub fn arrive(id: u32, seat: u8, sortie: Sortie, entry: Position, mark: Position, armed: bool, t: &Tuning) -> Aircraft {
        Aircraft {
            id,
            seat,
            sortie,
            stage: AirStage::Inbound,
            ground: entry,
            height: sortie.height(t),
            heading: unit(mark - entry).unwrap_or(Vec2::new(0.0, 1.0)),
            entry,
            mark,
            stage_time: 0.0,
            flown: 0.0,
            fired: 0,
            armed,
            hits: 0,
        }
    }

    /// Where it is drawn: the ground point lifted by the height.
    pub fn drawn(&self) -> Position {
        Position::new(self.ground.x, self.ground.y - self.height)
    }

    /// The jet's run, unit, from where it came in towards its mark.
    pub fn run(&self) -> Vec2 {
        unit(self.mark - self.entry).unwrap_or(self.heading)
    }

    /// Seat as owner (`shell::Owner::Player`).
    pub fn owner(&self) -> crate::shell::Owner {
        crate::shell::Owner::Player(self.seat)
    }

    /// As an air target (`air.rs`): only the heli is one.
    pub fn as_target(&self, t: &Tuning) -> Option<crate::air::AirTarget> {
        (self.sortie == Sortie::Heli).then(|| crate::air::AirTarget {
            key: crate::air::AirKey::Aircraft(self.id),
            owner: self.owner(),
            ground: self.ground,
            height: self.height,
            velocity: if self.stage == AirStage::Attack { Vec2::zero() } else { self.heading * t.air_heli_speed },
            half: t.air_heli_hit_half_px,
        })
    }

    /// Fly one fixed step (no RNG, no world): the heli in to its hover
    /// point, then out back past its entry; the jet straight along its run.
    /// The stage change into `Attack` is the heli reaching its hover point;
    /// the rest (firing, leaving) is the world half's to decide.
    pub fn advance(&mut self, dt: f32, t: &Tuning) {
        self.stage_time += dt;
        let speed = self.sortie.speed(t);
        match (self.sortie, self.stage) {
            (Sortie::Heli, AirStage::Inbound) => {
                let to = self.mark - self.ground;
                let step = speed * dt;
                if to.length() <= step {
                    self.ground = self.mark;
                    self.stage = AirStage::Attack;
                    self.stage_time = 0.0;
                } else if let Some(dir) = unit(to) {
                    self.ground = self.ground + dir * step;
                    self.heading = dir;
                }
            }
            (Sortie::Heli, AirStage::Attack) => {}
            (Sortie::Heli, AirStage::Egress) => {
                let away = unit(self.entry - self.mark).unwrap_or(Vec2::new(0.0, -1.0));
                self.ground = self.ground + away * speed * dt;
                self.heading = away;
            }
            (Sortie::Jet, _) => {
                let run = self.run();
                self.ground = self.ground + run * speed * dt;
                self.heading = run;
                self.flown += speed * dt;
            }
        }
    }

    /// Whether it has flown off the field (`field` its size) far enough to
    /// be gone: past the margin it came in from, on its way out.
    pub fn gone(&self, field: (f32, f32), t: &Tuning) -> bool {
        if self.stage != AirStage::Egress {
            return false;
        }
        let m = t.air_edge_margin_px + 1.0;
        self.ground.x < -m || self.ground.y < -m || self.ground.x > field.0 + m || self.ground.y > field.1 + m
    }
}

/// The unit vector along `v`, `None` for a zero one.
pub fn unit(v: Vec2) -> Option<Vec2> {
    let len = v.length();
    (len > 1e-3).then(|| v / len)
}

/// Which edge call `id` comes in from: 0 top, 1 right, 2 bottom, 3 left, a
/// hash of the id (rule 1).
pub fn entry_edge(id: u32) -> u8 {
    (id.wrapping_mul(0x9E37_79B1) >> 30) as u8
}

/// Where an aircraft coming at `toward` from edge `edge` (`entry_edge`)
/// enters: on that edge in line with `toward`, `margin` px outside the
/// field.
pub fn entry_point(edge: u8, toward: Position, field: (f32, f32), margin: f32) -> Position {
    let (w, h) = field;
    match edge & 3 {
        0 => Position::new(toward.x.clamp(0.0, w), -margin),
        1 => Position::new(w + margin, toward.y.clamp(0.0, h)),
        2 => Position::new(toward.x.clamp(0.0, w), h + margin),
        _ => Position::new(-margin, toward.y.clamp(0.0, h)),
    }
}

/// The heli's hover point over `target`: `standoff` px back towards where it
/// came in, held `inset` px inside the field.
pub fn hover_point(entry: Position, target: Position, standoff: f32, field: (f32, f32), inset: f32) -> Position {
    let back = unit(entry - target).unwrap_or(Vec2::new(0.0, -1.0));
    let p = target + back * standoff;
    Position::new(p.x.clamp(inset, (field.0 - inset).max(inset)), p.y.clamp(inset, (field.1 - inset).max(inset)))
}

/// The jet's aim point: the middle of the nearest `n` targets (`targets`
/// already sorted nearest first), `None` for none.
pub fn run_mark(targets: &[Position], n: usize) -> Option<Position> {
    let near = &targets[..targets.len().min(n.max(1))];
    if near.is_empty() {
        return None;
    }
    let sum = near.iter().fold(Vec2::zero(), |acc, &p| acc + p);
    Some(sum / near.len() as f32)
}

/// The jet's cannon bursts: where burst `i` of `n` lands along a run from
/// `entry` through `mark` - a stripe `len` px long ending on the mark - and
/// how far the jet has flown when it fires it, `ahead` px short of the
/// impact.
pub fn gun_burst(entry: Position, mark: Position, i: u8, n: u8, len: f32, ahead: f32) -> (Position, f32) {
    let run = unit(mark - entry).unwrap_or(Vec2::new(0.0, 1.0));
    let to_mark = (mark - entry).length();
    let step = if n > 1 { len / (n - 1) as f32 } else { 0.0 };
    let along = to_mark - len + step * i as f32;
    (entry + run * along, (along - ahead).max(0.0))
}

/// The jet's two bombs: where each lands, `spacing` px apart astride the
/// mark along the run, and how far the jet has flown when it lets them go,
/// `lead` px short of the mark.
pub fn bombs(entry: Position, mark: Position, spacing: f32, lead: f32) -> ([Position; 2], f32) {
    let run = unit(mark - entry).unwrap_or(Vec2::new(0.0, 1.0));
    let half = run * (spacing * 0.5);
    ([mark - half, mark + half], ((mark - entry).length() - lead).max(0.0))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn t() -> Tuning {
        Tuning::default()
    }

    #[test]
    fn a_crate_names_its_sortie_and_back() {
        for s in [Sortie::Heli, Sortie::Jet] {
            assert_eq!(Sortie::of(s.pickup()), Some(s));
        }
        assert_eq!(Sortie::of(PickupKind::Health), None);
    }

    #[test]
    fn the_entry_edge_is_a_hash_of_the_id_and_spans_every_edge() {
        let edges: std::collections::BTreeSet<u8> = (1..64).map(entry_edge).collect();
        assert_eq!(edges.len(), 4);
        assert_eq!(entry_edge(7), entry_edge(7));
    }

    #[test]
    fn entry_points_sit_outside_the_field_in_line_with_the_target() {
        let field = (1000.0, 600.0);
        let at = Position::new(300.0, 200.0);
        assert_eq!(entry_point(0, at, field, 50.0), Position::new(300.0, -50.0));
        assert_eq!(entry_point(1, at, field, 50.0), Position::new(1050.0, 200.0));
        assert_eq!(entry_point(2, at, field, 50.0), Position::new(300.0, 650.0));
        assert_eq!(entry_point(3, at, field, 50.0), Position::new(-50.0, 200.0));
    }

    #[test]
    fn the_heli_hovers_short_of_its_target_towards_its_entry() {
        let p = hover_point(Position::new(300.0, -50.0), Position::new(300.0, 400.0), 128.0, (1000.0, 600.0), 32.0);
        assert_eq!(p, Position::new(300.0, 272.0));
    }

    #[test]
    fn the_run_mark_is_the_middle_of_the_nearest() {
        let ts = [Position::new(0.0, 0.0), Position::new(100.0, 0.0), Position::new(50.0, 90.0), Position::new(900.0, 900.0)];
        assert_eq!(run_mark(&ts, 3), Some(Position::new(50.0, 30.0)));
        assert_eq!(run_mark(&[], 3), None);
    }

    #[test]
    fn the_gun_stripe_ends_on_the_mark_and_fires_ahead_of_it() {
        let (entry, mark) = (Position::new(100.0, -50.0), Position::new(100.0, 450.0));
        let (first, at0) = gun_burst(entry, mark, 0, 5, 200.0, 120.0);
        let (last, at4) = gun_burst(entry, mark, 4, 5, 200.0, 120.0);
        assert_eq!(first, Position::new(100.0, 250.0));
        assert_eq!(last, mark);
        assert_eq!((at0, at4), (180.0, 380.0));
        let ([a, b], at) = bombs(entry, mark, 48.0, 60.0);
        assert_eq!((a, b, at), (Position::new(100.0, 426.0), Position::new(100.0, 474.0), 440.0));
    }

    #[test]
    fn the_heli_flies_in_hovers_and_leaves_the_way_it_came() {
        let t = t();
        let entry = Position::new(200.0, -t.air_edge_margin_px);
        let mut a = Aircraft::arrive(1, 0, Sortie::Heli, entry, Position::new(200.0, 300.0), true, &t);
        let mut steps = 0;
        while a.stage == AirStage::Inbound && steps < 10_000 {
            a.advance(1.0 / 60.0, &t);
            steps += 1;
        }
        assert_eq!(a.stage, AirStage::Attack);
        assert_eq!(a.ground, a.mark);
        a.advance(1.0, &t);
        assert_eq!(a.ground, a.mark, "it hovers");
        a.stage = AirStage::Egress;
        let field = (400.0, 400.0);
        while !a.gone(field, &t) && steps < 20_000 {
            a.advance(1.0 / 60.0, &t);
            steps += 1;
        }
        assert!(a.gone(field, &t));
        assert!(a.ground.y < 0.0, "out the top it came in by");
    }

    #[test]
    fn the_jet_flies_straight_and_counts_its_distance() {
        let t = t();
        let mut a = Aircraft::arrive(2, 0, Sortie::Jet, Position::new(-50.0, 300.0), Position::new(500.0, 300.0), true, &t);
        a.advance(0.5, &t);
        assert_eq!(a.ground.y, 300.0);
        assert!((a.flown - t.air_jet_speed * 0.5).abs() < 1e-3);
        assert!((a.ground.x - (-50.0 + t.air_jet_speed * 0.5)).abs() < 1e-3);
    }

    #[test]
    fn only_the_heli_is_an_air_target() {
        let t = t();
        let heli = Aircraft::arrive(3, 1, Sortie::Heli, Position::new(0.0, 0.0), Position::new(10.0, 10.0), true, &t);
        let jet = Aircraft::arrive(4, 1, Sortie::Jet, Position::new(0.0, 0.0), Position::new(10.0, 10.0), true, &t);
        assert_eq!(heli.as_target(&t).map(|a| a.key), Some(crate::air::AirKey::Aircraft(3)));
        assert!(jet.as_target(&t).is_none());
    }

    #[test]
    fn a_waiting_call_has_no_count() {
        let c = AirCall { id: 1, seat: 0, sortie: Sortie::Jet, arrive_at: None };
        assert_eq!(c.left(3.0), None);
        let c = AirCall { arrive_at: Some(15.0), ..c };
        assert_eq!(c.left(3.0), Some(12.0));
        assert_eq!(c.left(20.0), Some(0.0));
    }
}
