//! Zones (docs/rod-from-god.md "Zones"): a world-placed area with an owner, a
//! centre and an end on the round clock - something a weapon left standing
//! on the field that the enemies keep out of, the router prices, a seat is
//! warned of when it is off its screen and the wire carries whole. A rod's
//! call is one; a gravity well (docs/gravity-well.md) is the other.
//!
//! The rules every reader keeps:
//!
//! 1. `Game::zones` holds them sorted by id - the walk order - and only the
//!    weapon that made one removes it, when it ends.
//! 2. The AI reads `Zone::danger` (a `Danger` like any other, owned by
//!    nobody: the dodge and the edge hold need nothing of their own, and
//!    `out_of_danger` takes a nobody's disc's exit on the tank's own side)
//!    and the router `Zone::route` (the disc and its cost). A kind the AI
//!    meets some other way - a well, which an enemy drives across rather
//!    than backs out of - has no danger.
//! 3. A zone's radius is its kind's knob, not its own state: the wire
//!    carries the kind, the centre and the end.
//! 4. A round with none - every round without the rod or the well - reads
//!    nothing.

use crate::ai::{Danger, DangerShape};
use crate::rod::RodCall;
use crate::shell::Owner;
use crate::tuning::Tuning;
use crate::well::{WellStage, WellZone};
use crate::Position;

/// The wire's tag for a rod's call (`net::wire::ZoneState::kind`).
pub const ZONE_ROD: u8 = 0;

/// The wire's tag for a gravity well.
pub const ZONE_WELL: u8 = 1;

/// The ids a client's own zones drawn ahead of the room's take
/// (`Game::set_provisional_zones`): past every id the wire's `u16` can name,
/// so one never collides with the room's.
pub const PROVISIONAL_ZONE_BASE: u32 = 0x0001_0000;

/// A world-placed area with a lifetime (`Game::zones`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Zone {
    /// From the round's projectile counter (`Game::take_shot_id`): its wire
    /// key and its walk order.
    pub id: u32,
    pub kind: ZoneKind,
    /// Whose it is: the kill credit, and who a seat's screen is not warned
    /// of (its own).
    pub owner: Owner,
    pub centre: Position,
    /// When it ends, on the round clock (`Game::time`).
    pub until: f32,
}

/// What a zone is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ZoneKind {
    /// A rod's call: lands at `until`.
    Rod(RodCall),
    /// A gravity well: `until` is the end of its stage - its pull's start
    /// while forming, its collapse while pulling.
    Well(WellZone),
}

impl Zone {
    /// Seconds left at round time `now`.
    pub fn left(&self, now: f32) -> f32 {
        (self.until - now).max(0.0)
    }

    /// The area it acts on (px): a rod's circle, a well's reach.
    pub fn radius(&self, t: &Tuning) -> f32 {
        match self.kind {
            ZoneKind::Rod(_) => t.rod_kill_radius_px,
            ZoneKind::Well(_) => t.well_radius_px,
        }
    }

    /// The disc an enemy keeps its centre out of while it stands: a rod's
    /// circle grown by the widest hull's half (`battlefield::
    /// max_tank_clearance_half_extent`) and `rod_ai_berth_px`, so a hull at
    /// its edge stands clear of the circle.
    pub fn danger_radius(&self, t: &Tuning) -> f32 {
        match self.kind {
            ZoneKind::Rod(_) => t.rod_kill_radius_px + crate::battlefield::max_tank_clearance_half_extent() + t.rod_ai_berth_px,
            ZoneKind::Well(_) => t.well_radius_px,
        }
    }

    /// What an enemy keeps out of while it stands, owned by nobody - a
    /// rod's caller dies in its circle as surely as anyone. A well is none:
    /// the dodge's way out of a disc is radial, the wrong way out of a pull
    /// (the AI's `pull` tier drives across it instead).
    pub fn danger(&self, t: &Tuning) -> Option<Danger> {
        match self.kind {
            ZoneKind::Rod(_) => Some(Danger { shape: DangerShape::Disc { at: self.centre, radius: self.danger_radius(t) }, owner: None, slack: 0.0 }),
            ZoneKind::Well(_) => None,
        }
    }

    /// Whether `p` stands inside its danger - none for a well.
    pub fn holds(&self, p: Position, t: &Tuning) -> bool {
        self.danger(t).is_some() && p.distance_to(self.centre) <= self.danger_radius(t)
    }

    /// The radius of the disc whose nav cells the router surcharges while it
    /// stands, and the surcharge (`rod_ai_circle_cost`); `None` for none.
    pub fn route(&self, t: &Tuning) -> Option<(f32, u32)> {
        match self.kind {
            ZoneKind::Rod(_) => (t.rod_ai_circle_cost > 0).then(|| (self.danger_radius(t), t.rod_ai_circle_cost as u32)),
            ZoneKind::Well(_) => (t.well_ai_route_cost > 0).then(|| (t.well_radius_px, t.well_ai_route_cost as u32)),
        }
    }

    /// The wire's tag for its kind.
    pub fn wire_kind(&self) -> u8 {
        match self.kind {
            ZoneKind::Rod(_) => ZONE_ROD,
            ZoneKind::Well(_) => ZONE_WELL,
        }
    }

    /// The wire's stage byte: a well's stage (0 forming, 1 pulling), 0 for
    /// a call.
    pub fn wire_stage(&self) -> u8 {
        match self.kind {
            ZoneKind::Well(w) if w.stage == WellStage::Pulling => 1,
            _ => 0,
        }
    }

    /// The rod's call, if it is one.
    pub fn rod(&self) -> Option<RodCall> {
        match self.kind {
            ZoneKind::Rod(call) => Some(call),
            ZoneKind::Well(_) => None,
        }
    }

    /// The well, if it is one.
    pub fn well(&self) -> Option<WellZone> {
        match self.kind {
            ZoneKind::Well(w) => Some(w),
            ZoneKind::Rod(_) => None,
        }
    }

    /// A client's own, drawn ahead of the room's copy.
    pub fn provisional(&self) -> bool {
        self.id >= PROVISIONAL_ZONE_BASE
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rod_at(at: Position) -> Zone {
        Zone { id: 3, kind: ZoneKind::Rod(RodCall { cell: crate::map::world_to_cell(at), seat: None }), owner: Owner::Enemy(4), centre: at, until: 10.0 }
    }

    #[test]
    fn a_rod_zones_danger_covers_its_circle_and_a_hull_beside_it() {
        let t = Tuning::DEFAULT;
        let z = rod_at(Position::new(320.0, 320.0));
        let d = z.danger(&t).expect("a danger");
        assert_eq!(d.owner, None);
        let edge = Position::new(320.0 + t.rod_kill_radius_px + crate::battlefield::max_tank_clearance_half_extent(), 320.0);
        assert!(d.depth(edge) > 0.0, "a hull centred there would reach into the circle");
        assert!(z.holds(edge, &t));
        assert!(!z.holds(Position::new(320.0 + z.danger_radius(&t) + 1.0, 320.0), &t));
    }

    #[test]
    fn left_counts_down_on_the_round_clock() {
        let z = rod_at(Position::new(64.0, 64.0));
        assert_eq!(z.left(6.0), 4.0);
        assert_eq!(z.left(12.0), 0.0);
        assert!(!z.provisional());
    }
}
