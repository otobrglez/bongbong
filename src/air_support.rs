//! Air support (docs/air-support.md), headless: the call a seat's crate
//! makes, the aircraft that answers it, and what the world shows it and
//! takes from it. A crate (`PickupKind::AirHeli`, `PickupKind::AirJet`) puts
//! an `AirCall` on `Game::air_calls`; when its count runs out an `Aircraft`
//! flies in from the field's edge. How each one flies and fights is its own
//! file - `air_support/heli.rs` the helicopter, `air_support/jet.rs` the
//! jet - behind one door, `Aircraft::step`: the aircraft, an `AirView` of
//! the world and the fixed step in, `AirAct`s out. The world half is
//! `simulation/air_support.rs`; the sprite is `render/air_support.rs`.
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
//! 4. walks go by id;
//! 5. an aircraft reads the world only through its `AirView` and changes it
//!    only through its `AirAct`s.

use serde::{Deserialize, Serialize};

use crate::math::Vec2;
use crate::pickup::PickupKind;
use crate::tuning::Tuning;
use crate::Position;

mod heli;
mod jet;

pub use heli::HeliState;
pub use jet::JetState;

/// Which aircraft answers a call. The variant order is the wire's encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Sortie {
    /// An Apache-like gunship (`air_support/heli.rs`). An air target.
    Heli,
    /// An A-10-like attack jet (`air_support/jet.rs`). Never an air target.
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

    /// How fast it flies (px/s) at full speed.
    pub fn speed(self, t: &Tuning) -> f32 {
        match self {
            Sortie::Heli => t.air_heli_speed,
            Sortie::Jet => t.air_jet_speed,
        }
    }

    /// The ammunition it arrives with (`Aircraft::ammo`): the helicopter's
    /// salvos, the jet's passes.
    pub fn ammo(self, t: &Tuning) -> u8 {
        match self {
            Sortie::Heli => t.air_heli_salvos.clamp(0, 255) as u8,
            Sortie::Jet => t.air_jet_passes.clamp(0, 255) as u8,
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
    /// On a target: the heli firing at one, the jet on an attack run.
    Attack,
    /// Leaving off the edge.
    Egress,
    /// The heli holding station by its seat with nothing to shoot at.
    Escort,
    /// The jet between attack runs, or circling its seat.
    Loiter,
}

/// `net::wire::AirState::stage` for a call waiting behind another.
pub const AIR_WAITING: u8 = 0;

/// `net::wire::AirState::stage` for a call counting down.
pub const AIR_COUNTING: u8 = 1;

impl AirStage {
    /// Every stage, in wire order.
    pub const ALL: [AirStage; 5] = [AirStage::Inbound, AirStage::Attack, AirStage::Egress, AirStage::Escort, AirStage::Loiter];

    /// Its wire code (`net::wire::AirState::stage`), past the calls' two.
    pub fn code(self) -> u8 {
        match self {
            AirStage::Inbound => 2,
            AirStage::Attack => 3,
            AirStage::Egress => 4,
            AirStage::Escort => 5,
            AirStage::Loiter => 6,
        }
    }

    /// Inverse of `code`, `None` for a call's codes.
    pub fn from_code(code: u8) -> Option<AirStage> {
        AirStage::ALL.into_iter().find(|s| s.code() == code)
    }

    /// Lower-case name for tooling.
    pub fn name(self) -> &'static str {
        match self {
            AirStage::Inbound => "inbound",
            AirStage::Attack => "attack",
            AirStage::Egress => "egress",
            AirStage::Escort => "escort",
            AirStage::Loiter => "loiter",
        }
    }
}

/// Which target an `AirMark` is: the same while it lives, so an aircraft
/// can keep after one from step to step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum MarkKey {
    /// An enemy tank, by its owner slot.
    Tank(usize),
    /// An enemy tower, by its map cell.
    Tower((i32, i32)),
}

/// One thing an aircraft may attack (`Game::air_marks`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AirMark {
    pub key: MarkKey,
    pub at: Position,
    /// Its velocity over the ground (px/s): zero for a tower.
    pub velocity: Vec2,
}

/// What the world shows an aircraft for one step (rule 5).
#[derive(Clone, Copy, Debug)]
pub struct AirView<'a> {
    /// Where its seat's tank stands, a wreck included; `None` once the seat
    /// has no tank at all.
    pub seat: Option<Position>,
    /// That tank's velocity over the ground (px/s): what its body does,
    /// zero for a wreck or no seat.
    pub seat_velocity: Vec2,
    /// Whether that tank is a wreck.
    pub seat_wrecked: bool,
    /// What it may attack this step: the enemy tanks live on the field and
    /// the enemy towers still standing inside the seat's sight box
    /// (`ai::in_sight_box`), nearest the seat first. Empty with no seat.
    pub marks: &'a [AirMark],
    /// The field's size.
    pub field: (f32, f32),
    /// False on the end screen: nothing fires, and an aircraft goes home.
    pub live: bool,
}

/// What an aircraft does to the world in a step (rule 5); the world half
/// carries it out (`Game::air_support_phase`), and drops it on the end
/// screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum AirAct {
    /// The heli's salvo: `air_heli_salvo_rockets` seeker missiles from
    /// `from` (where it is drawn) at `at`.
    Salvo { from: Position, at: Position },
    /// One of the jet's cannon bursts, or (`bomb`) one of its bombs, landing
    /// at `at`, the jet flying `dir`.
    Strike { at: Position, dir: Vec2, bomb: bool },
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
    /// Its velocity over the ground (px/s).
    pub velocity: Vec2,
    /// How hard it is turning: -1 full left to 1 full right, the share of
    /// its turn rate in use. What its sprite banks by.
    pub bank: f32,
    /// Where it came in.
    pub entry: Position,
    /// Seconds in its current stage (`enter` restarts it).
    pub stage_time: f32,
    /// Ammunition left: the helicopter's salvos, the jet's passes
    /// (`Sortie::ammo`). The HUD's box shows it.
    pub ammo: u8,
    /// Bullets that struck it (the heli; `air_heli_hits` bring it down).
    pub hits: u32,
    /// The helicopter's own state; unread on a jet.
    pub heli: HeliState,
    /// The jet's own state; unread on a helicopter.
    pub jet: JetState,
}

impl Aircraft {
    /// The aircraft answering call `id` of `seat`, coming in at `entry`
    /// facing `toward`, at full speed with all its ammunition.
    pub fn arrive(id: u32, seat: u8, sortie: Sortie, entry: Position, toward: Position, t: &Tuning) -> Aircraft {
        let heading = unit(toward - entry).unwrap_or(Vec2::new(0.0, 1.0));
        Aircraft {
            id,
            seat,
            sortie,
            stage: AirStage::Inbound,
            ground: entry,
            height: sortie.height(t),
            heading,
            velocity: heading * sortie.speed(t),
            bank: 0.0,
            entry,
            stage_time: 0.0,
            ammo: sortie.ammo(t),
            hits: 0,
            heli: HeliState::default(),
            jet: JetState::default(),
        }
    }

    /// Where it is drawn: the ground point lifted by the height.
    pub fn drawn(&self) -> Position {
        Position::new(self.ground.x, self.ground.y - self.height)
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
            velocity: self.velocity,
            half: t.air_heli_hit_half_px,
        })
    }

    /// Move to `stage`, its clock (`stage_time`) restarted.
    pub fn enter(&mut self, stage: AirStage) {
        self.stage = stage;
        self.stage_time = 0.0;
    }

    /// Fly and fight one fixed step (no RNG, rule 5): what it sees is
    /// `view`, what it does to the world goes on `acts`.
    pub fn step(&mut self, view: &AirView, dt: f32, t: &Tuning, acts: &mut Vec<AirAct>) {
        self.stage_time += dt;
        match self.sortie {
            Sortie::Heli => heli::step(self, view, dt, t, acts),
            Sortie::Jet => jet::step(self, view, dt, t, acts),
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

/// `heading` (unit) turned toward `want` by at most `max` radians: the new
/// unit heading, and the angle turned - positive clockwise on the y-down
/// screen, which is a turn to the right. A `want` of no length turns
/// nothing.
pub fn turn_toward(heading: Vec2, want: Vec2, max: f32) -> (Vec2, f32) {
    if want.length() < 1e-3 {
        return (heading, 0.0);
    }
    let have = crate::math::atan2(heading.y, heading.x);
    let mut delta = crate::math::atan2(want.y, want.x) - have;
    while delta > std::f32::consts::PI {
        delta -= std::f32::consts::TAU;
    }
    while delta < -std::f32::consts::PI {
        delta += std::f32::consts::TAU;
    }
    let turned = delta.clamp(-max.max(0.0), max.max(0.0));
    let (sin, cos) = crate::math::sin_cos(have + turned);
    (Vec2::new(cos, sin), turned)
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

/// The edge the jet comes in from (`entry_point`'s numbering): the axis a
/// hash of call `id` (`entry_edge`), the side the one farther from what it
/// is coming for (`toward`) on a field of size `field`, so it has at least
/// half the field to line up in.
pub fn jet_edge(id: u32, toward: Position, field: (f32, f32)) -> u8 {
    if entry_edge(id) & 1 == 0 {
        if toward.y > field.1 * 0.5 { 0 } else { 2 }
    } else if toward.x > field.0 * 0.5 {
        3
    } else {
        1
    }
}

/// The aircraft sheet's cell, px: 44 design pixels at 2 px a block, ten
/// per cent over a tank's 80 px sprite cell (docs/AIRCRAFT_SPEC.md).
pub const AIRCRAFT_CELL: f32 = 88.0;

/// The sheet's columns: the heli's airframe, its rotor's two frames, the
/// jet.
const COL_HELI: f32 = 0.0;
const COL_ROTOR: f32 = 1.0;
const COL_JET: f32 = 3.0;

/// How far ahead of the heli's centre its mast stands, px
/// (`gen_aircraft.py`'s `MAST_Y`, doubled).
pub const HELI_MAST_PX: f32 = 12.0;

/// The rotor's frames per second: its two frames alternate this fast.
const ROTOR_HZ: f32 = 30.0;

/// The rotor drawn over the airframe this opaque: a blur, not a blade.
const ROTOR_ALPHA: f32 = 0.55;

/// The sheet's art block, px: what a banked sprite's edges move in.
const ART_BLOCK_PX: f32 = 2.0;

/// How far in each of the jet's wing tips is drawn at a full bank, px: its
/// 88 px cell foreshortened to 64 across the wings, 73 per cent.
const JET_BANK_TIP_PX: f32 = 12.0;

/// How far the jet's shadow slides toward the inside of its turn at a full
/// bank, px.
const JET_BANK_SHADOW_PX: f32 = 6.0;

fn cell(col: f32) -> crate::math::Rectangle {
    crate::math::Rectangle::new(col * AIRCRAFT_CELL, 0.0, AIRCRAFT_CELL, AIRCRAFT_CELL)
}

impl Aircraft {
    /// The degrees its sprite turns (0 faces up).
    pub fn rotation(&self) -> f32 {
        crate::math::atan2(self.heading.x, -self.heading.y).to_degrees()
    }

    /// The way its right wing points (unit): its heading turned a quarter
    /// clockwise on the y-down screen, the inside of a right turn.
    pub fn right(&self) -> Vec2 {
        Vec2::new(-self.heading.y, self.heading.x)
    }

    /// How wide its sprite cell is drawn across the wings, px
    /// (`banked_span`): the jet's narrows with its bank, the helicopter's
    /// never does.
    pub fn span(&self) -> f32 {
        banked_span(self.bank, self.sortie.bank_tip_px())
    }

    /// How far its shadow slides along `right` for its bank, px
    /// (`bank_slide`): toward the inside of the jet's turn, nothing for the
    /// helicopter.
    pub fn shadow_slide(&self) -> f32 {
        match self.sortie {
            Sortie::Heli => 0.0,
            Sortie::Jet => bank_slide(self.bank, JET_BANK_SHADOW_PX),
        }
    }
}

impl Sortie {
    /// How far in each wing tip of its sprite is drawn at a full bank, px:
    /// the helicopter is drawn level whatever its bank.
    fn bank_tip_px(self) -> f32 {
        match self {
            Sortie::Heli => 0.0,
            Sortie::Jet => JET_BANK_TIP_PX,
        }
    }
}

/// `bank` as the share of a full one, 0 to 1 whichever way it rolls; a
/// bank that is no number is level.
fn bank_share(bank: f32) -> f32 {
    if bank.is_nan() { 0.0 } else { bank.abs().min(1.0) }
}

/// How wide an aircraft's sprite cell is drawn across its wings at `bank`
/// (-1 full left to 1 full right), px. A top-down sprite banks by its span
/// foreshortening: each wing tip comes in up to `tip_px` at a full bank,
/// in whole art blocks, so the cell narrows 4 px a step about its centre
/// and a level aircraft is drawn at the cell's own `AIRCRAFT_CELL`.
pub fn banked_span(bank: f32, tip_px: f32) -> f32 {
    let inset = (bank_share(bank) * tip_px / ART_BLOCK_PX).round() * ART_BLOCK_PX;
    AIRCRAFT_CELL - 2.0 * inset
}

/// How far a shadow slides toward the inside of a turn at `bank`, px, in
/// whole art blocks: up to `full_px` to the right at 1, to the left at -1.
pub fn bank_slide(bank: f32, full_px: f32) -> f32 {
    let blocks = (bank_share(bank) * full_px / ART_BLOCK_PX).round() * ART_BLOCK_PX;
    if bank < 0.0 { -blocks } else { blocks }
}

/// Where a sprite cell centred on `at` and drawn `span` px across its
/// wings (`banked_span`) lands: the blit's destination and the origin its
/// rotation turns about - the cell's centre, so the nose-to-tail length
/// and the centreline stay where a level one has them.
pub fn cell_dest(at: Position, span: f32) -> (crate::math::Rectangle, Vec2) {
    let dest = crate::math::Rectangle::new(at.x, at.y, span, AIRCRAFT_CELL);
    (dest, Vec2::new(span * 0.5, AIRCRAFT_CELL * 0.5))
}

/// One aircraft-sheet cell centred on `at`, `span` px across the wings,
/// turned `rotation` degrees.
fn blit_cell(c: &mut impl crate::canvas::Canvas, col: f32, at: Position, span: f32, rotation: f32, tint: crate::math::Color) {
    let (dest, origin) = cell_dest(at, span);
    c.blit(crate::canvas::Sheet::Aircraft, cell(col), dest, origin, rotation, tint);
}

/// Every aircraft's shadow on the ground under it (docs/air-support.md
/// "Drawing"): its silhouette tinted black at the tanks' shadow opacity,
/// on its ground point nudged along the light - the rotor's disc too. A
/// banked jet's is foreshortened as the jet is (`Aircraft::span`) and
/// slides toward the inside of the turn (`Aircraft::shadow_slide`).
/// Drawn after the floor, under the tanks. `aircraft` in id order.
pub fn paint_shadows(c: &mut impl crate::canvas::Canvas, aircraft: &[Aircraft], time: f32) {
    let t = crate::tuning::tuning();
    let nudge = Vec2::new(t.shadow_dir_x, t.shadow_dir_y) * 6.0;
    let shadow = crate::math::Color::new(0, 0, 0, (255.0 * t.tank_shadow_opacity * 0.8) as u8);
    for a in aircraft {
        let slide = a.shadow_slide();
        let at = if slide == 0.0 { a.ground + nudge } else { a.ground + nudge + a.right() * slide };
        let rot = a.rotation();
        let span = a.span();
        match a.sortie {
            Sortie::Heli => {
                blit_cell(c, COL_HELI, at, span, rot, shadow);
                let disc = crate::math::Color::new(0, 0, 0, (shadow.a as f32 * ROTOR_ALPHA) as u8);
                blit_cell(c, COL_ROTOR + rotor_frame(time), at + a.heading * HELI_MAST_PX, AIRCRAFT_CELL, rot, disc);
            }
            Sortie::Jet => blit_cell(c, COL_JET, at, span, rot, shadow),
        }
    }
}

/// Every aircraft in the air, over everything that stands - trees and
/// drones too: lifted by its height over its shadow, the heli's rotor
/// turning over it, the jet banked by its turn (`Aircraft::span`).
/// `aircraft` in id order.
pub fn paint_aircraft(c: &mut impl crate::canvas::Canvas, aircraft: &[Aircraft], time: f32) {
    let white = crate::math::Color::WHITE;
    for a in aircraft {
        let at = a.drawn();
        let rot = a.rotation();
        let span = a.span();
        match a.sortie {
            Sortie::Heli => {
                blit_cell(c, COL_HELI, at, span, rot, white);
                let blur = crate::math::Color::new(255, 255, 255, (255.0 * ROTOR_ALPHA) as u8);
                blit_cell(c, COL_ROTOR + rotor_frame(time), at + a.heading * HELI_MAST_PX, AIRCRAFT_CELL, rot, blur);
            }
            Sortie::Jet => blit_cell(c, COL_JET, at, span, rot, white),
        }
    }
}

/// Which of the rotor's two frames shows at round time `time`: 0 or 1.
fn rotor_frame(time: f32) -> f32 {
    ((time * ROTOR_HZ) as i64 % 2) as f32
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
    fn a_stage_names_its_code_and_back() {
        for stage in AirStage::ALL {
            assert_eq!(AirStage::from_code(stage.code()), Some(stage));
        }
        assert_eq!(AirStage::from_code(AIR_WAITING), None);
        assert_eq!(AirStage::from_code(AIR_COUNTING), None);
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
    fn the_jet_comes_in_from_the_far_side_of_what_it_comes_for() {
        let field = (1000.0, 600.0);
        for id in 1..64 {
            for toward in [Position::new(100.0, 100.0), Position::new(900.0, 500.0)] {
                let entry = entry_point(jet_edge(id, toward, field), toward, field, 96.0);
                let run = (toward - entry).length();
                assert!(run >= 300.0 + 96.0 - 1e-3, "id {id}: a run of {run}");
            }
        }
    }

    #[test]
    fn a_turn_is_held_to_its_rate_and_signed_right_positive() {
        let east = Vec2::new(1.0, 0.0);
        let south = Vec2::new(0.0, 1.0);
        // South is clockwise of east on the y-down screen: a right turn.
        let (h, turned) = turn_toward(east, south, 0.1);
        assert!((turned - 0.1).abs() < 1e-6, "{turned}");
        assert!(h.y > 0.0 && (h.length() - 1.0).abs() < 1e-5);
        let (h, turned) = turn_toward(east, Vec2::new(0.0, -1.0), 0.1);
        assert!((turned + 0.1).abs() < 1e-6 && h.y < 0.0);
        // Inside the rate it lands on the wanted heading.
        let (h, turned) = turn_toward(east, south, 4.0);
        assert!((turned - std::f32::consts::FRAC_PI_2).abs() < 1e-5);
        assert!((h - south).length() < 1e-5);
        assert_eq!(turn_toward(east, Vec2::zero(), 1.0), (east, 0.0));
    }

    #[test]
    fn an_aircraft_arrives_at_full_speed_with_its_ammunition() {
        let t = t();
        let a = Aircraft::arrive(1, 0, Sortie::Jet, Position::new(-50.0, 300.0), Position::new(500.0, 300.0), &t);
        assert_eq!(a.heading, Vec2::new(1.0, 0.0));
        assert_eq!(a.velocity, Vec2::new(t.air_jet_speed, 0.0));
        assert_eq!(a.ammo as i32, t.air_jet_passes);
        assert_eq!(Aircraft::arrive(2, 0, Sortie::Heli, Position::new(0.0, 0.0), Position::new(0.0, 9.0), &t).ammo as i32, t.air_heli_salvos);
    }

    #[test]
    fn only_the_heli_is_an_air_target() {
        let t = t();
        let heli = Aircraft::arrive(3, 1, Sortie::Heli, Position::new(0.0, 0.0), Position::new(10.0, 10.0), &t);
        let jet = Aircraft::arrive(4, 1, Sortie::Jet, Position::new(0.0, 0.0), Position::new(10.0, 10.0), &t);
        assert_eq!(heli.as_target(&t).map(|a| a.key), Some(crate::air::AirKey::Aircraft(3)));
        assert!(jet.as_target(&t).is_none());
    }

    #[test]
    fn it_is_gone_only_on_its_way_out_past_the_margin() {
        let t = t();
        let field = (400.0, 400.0);
        let mut a = Aircraft::arrive(5, 0, Sortie::Jet, Position::new(-t.air_edge_margin_px - 50.0, 200.0), Position::new(200.0, 200.0), &t);
        assert!(!a.gone(field, &t), "coming in, it is outside the field and not gone");
        a.enter(AirStage::Egress);
        assert!(a.gone(field, &t));
        a.ground = Position::new(200.0, 200.0);
        assert!(!a.gone(field, &t));
    }

    /// A canvas that keeps each blit: its source column, destination,
    /// origin, rotation and tint.
    #[derive(Default)]
    struct Blits(Vec<(f32, crate::math::Rectangle, Vec2, f32, crate::math::Color)>);

    impl crate::canvas::Canvas for Blits {
        fn blit(&mut self, _: crate::canvas::Sheet, src: crate::math::Rectangle, dest: crate::math::Rectangle, origin: Vec2, rotation: f32, tint: crate::math::Color) {
            self.0.push((src.x / AIRCRAFT_CELL, dest, origin, rotation, tint));
        }
        fn fill_rect(&mut self, _: i32, _: i32, _: i32, _: i32, _: crate::math::Color) {}
        fn gradient_v(&mut self, _: i32, _: i32, _: i32, _: i32, _: crate::math::Color, _: crate::math::Color) {}
        fn gradient_h(&mut self, _: i32, _: i32, _: i32, _: i32, _: crate::math::Color, _: crate::math::Color) {}
        fn disc(&mut self, _: Position, _: f32, _: crate::math::Color) {}
        fn ring(&mut self, _: Position, _: f32, _: f32, _: f32, _: f32, _: i32, _: crate::math::Color) {}
        fn blocks(&mut self, _: &crate::canvas::BlockImage) {}
        fn blocks_part(&mut self, _: &crate::canvas::BlockImage, _: (usize, usize, usize, usize), _: (i32, i32)) {}
    }

    /// A jet over (400, 300) flying `heading` at `bank`.
    fn jet(heading: Vec2, bank: f32) -> Aircraft {
        let at = Position::new(400.0, 300.0);
        let mut a = Aircraft::arrive(1, 0, Sortie::Jet, at, at + heading, &t());
        a.bank = bank;
        a
    }

    #[test]
    fn a_banked_span_narrows_in_whole_blocks_a_wing_tip() {
        assert_eq!(banked_span(0.0, JET_BANK_TIP_PX), AIRCRAFT_CELL, "level: the cell's own width");
        assert_eq!(banked_span(1.0, JET_BANK_TIP_PX), 64.0, "a full bank: 12 px in a tip");
        let mut last = AIRCRAFT_CELL;
        for step in 0..=200 {
            let bank = step as f32 / 100.0;
            let span = banked_span(bank, JET_BANK_TIP_PX);
            assert_eq!(span, banked_span(-bank, JET_BANK_TIP_PX), "{bank}: left as right");
            assert_eq!((AIRCRAFT_CELL - span) % (2.0 * ART_BLOCK_PX), 0.0, "{bank}: a whole block a tip");
            assert!(span <= last && span >= 64.0, "{bank}: {span} after {last}");
            last = span;
        }
        assert_eq!(last, 64.0, "past a full bank it holds");
        assert_eq!(banked_span(f32::NAN, JET_BANK_TIP_PX), AIRCRAFT_CELL);
        assert_eq!(banked_span(1.0, 0.0), AIRCRAFT_CELL, "no tip to bring in");
    }

    #[test]
    fn a_shadow_slides_to_the_inside_of_the_turn_in_whole_blocks() {
        assert_eq!(bank_slide(0.0, JET_BANK_SHADOW_PX), 0.0);
        assert_eq!(bank_slide(1.0, JET_BANK_SHADOW_PX), 6.0);
        assert_eq!(bank_slide(-1.0, JET_BANK_SHADOW_PX), -6.0);
        assert_eq!(bank_slide(-3.0, JET_BANK_SHADOW_PX), -6.0);
        assert_eq!(bank_slide(f32::NAN, JET_BANK_SHADOW_PX), 0.0);
        for step in -100..=100 {
            let slide = bank_slide(step as f32 / 100.0, JET_BANK_SHADOW_PX);
            assert_eq!(slide % ART_BLOCK_PX, 0.0);
            assert!(slide * step as f32 >= 0.0, "{step}: {slide} is the wrong way");
        }
    }

    #[test]
    fn a_cell_narrows_about_its_centre() {
        let at = Position::new(400.0, 300.0);
        let (dest, origin) = cell_dest(at, AIRCRAFT_CELL);
        assert_eq!(dest, crate::math::Rectangle::new(400.0, 300.0, 88.0, 88.0));
        assert_eq!(origin, Vec2::new(44.0, 44.0));
        let (dest, origin) = cell_dest(at, 64.0);
        assert_eq!(dest, crate::math::Rectangle::new(400.0, 300.0, 64.0, 88.0), "its length is kept");
        assert_eq!(origin, Vec2::new(32.0, 44.0), "the centreline stays on the aircraft");
    }

    #[test]
    fn a_level_aircraft_is_drawn_at_its_cell_and_a_banked_jet_foreshortened() {
        let up = Vec2::new(0.0, -1.0);
        let level = jet(up, 0.0);
        let mut c = Blits::default();
        paint_aircraft(&mut c, &[level], 0.0);
        let half = AIRCRAFT_CELL * 0.5;
        let drawn = level.drawn();
        assert_eq!(
            c.0,
            vec![(COL_JET, crate::math::Rectangle::new(drawn.x, drawn.y, AIRCRAFT_CELL, AIRCRAFT_CELL), Vec2::new(half, half), level.rotation(), crate::math::Color::WHITE)],
            "level: the plain cell about its centre"
        );
        for bank in [-1.0, -0.5, 0.5, 1.0] {
            let banked = jet(up, bank);
            let mut c = Blits::default();
            paint_aircraft(&mut c, &[banked], 0.0);
            let (_, dest, origin, rotation, _) = c.0[0];
            assert_eq!((dest.x, dest.y, dest.height), (drawn.x, drawn.y, AIRCRAFT_CELL), "{bank}: where a level one is");
            assert_eq!(dest.width, banked_span(bank, JET_BANK_TIP_PX));
            assert!(dest.width < AIRCRAFT_CELL, "{bank}: its span is foreshortened");
            assert_eq!(origin, Vec2::new(dest.width * 0.5, half));
            assert_eq!(rotation, level.rotation());
        }
        // The helicopter is drawn level whatever its bank, its rotor a
        // full disc.
        let mut heli = Aircraft::arrive(2, 0, Sortie::Heli, Position::new(400.0, 300.0), Position::new(400.0, 200.0), &t());
        let mut flat = Blits::default();
        paint_aircraft(&mut flat, &[heli], 0.0);
        heli.bank = 1.0;
        let mut rolled = Blits::default();
        paint_aircraft(&mut rolled, &[heli], 0.0);
        assert_eq!(flat.0, rolled.0);
        assert!(flat.0.iter().all(|(_, dest, ..)| dest.width == AIRCRAFT_CELL));
    }

    #[test]
    fn a_banked_jets_shadow_narrows_and_slides_to_the_inside_of_its_turn() {
        // Flying up the screen: its right is +x. Flying right: its right
        // is down the screen.
        for (heading, right) in [(Vec2::new(0.0, -1.0), Vec2::new(1.0, 0.0)), (Vec2::new(1.0, 0.0), Vec2::new(0.0, 1.0))] {
            let level = jet(heading, 0.0);
            assert_eq!(level.right(), right);
            let mut flat = Blits::default();
            paint_shadows(&mut flat, &[level], 0.0);
            let (_, at, origin, ..) = flat.0[0];
            assert_eq!((at.width, origin), (AIRCRAFT_CELL, Vec2::new(44.0, 44.0)), "a level shadow is the plain cell");
            for bank in [-1.0f32, 1.0] {
                let mut c = Blits::default();
                paint_shadows(&mut c, &[jet(heading, bank)], 0.0);
                let (_, dest, ..) = c.0[0];
                assert_eq!(dest.width, 64.0, "{bank}: the shadow is the jet's silhouette");
                let slid = Vec2::new(dest.x - at.x, dest.y - at.y);
                assert!((slid - right * (bank * JET_BANK_SHADOW_PX)).length() < 1e-3, "{heading:?} at {bank}: slid {slid:?}");
            }
        }
    }

    /// The rasteriser's view of a bank: a jet flying up the screen covers
    /// 88 columns level and 64 at a full bank, about the same centre, and
    /// as many rows either way.
    #[test]
    fn a_banked_jet_covers_fewer_columns_on_the_cpu_canvas() {
        use crate::canvas::{CpuCanvas, Pixels, Sheet};
        use crate::math::Color;
        let cover = |bank: f32| {
            let mut canvas = CpuCanvas::blank(800, 600);
            canvas.insert_sheet(Sheet::Aircraft, Pixels::filled(4 * AIRCRAFT_CELL as usize, AIRCRAFT_CELL as usize, Color::BLACK));
            let a = jet(Vec2::new(0.0, -1.0), bank);
            paint_aircraft(&mut canvas, &[a], 0.0);
            let dark = |x: usize, y: usize| canvas.pixel(x, y) == Color::BLACK;
            let cols: Vec<usize> = (0..800).filter(|&x| (0..600).any(|y| dark(x, y))).collect();
            let rows = (0..600).filter(|&y| (0..800).any(|x| dark(x, y))).count();
            (cols[0], cols.len(), rows)
        };
        let drawn_x = jet(Vec2::new(0.0, -1.0), 0.0).drawn().x as usize;
        assert_eq!(cover(0.0), (drawn_x - 44, 88, 88));
        assert_eq!(cover(1.0), (drawn_x - 32, 64, 88));
        assert_eq!(cover(-1.0), cover(1.0));
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
