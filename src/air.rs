//! Air targets (docs/fpv-swarm.md "Air targets"): something in the air that
//! can be struck there. Until the FPV swarm everything a shot could hit
//! stood on the ground, and everything in the air - missiles, flying drums,
//! lava bombs, globs, grenades aloft - was struck by nothing. The drones are
//! the first air targets; a later weapon that puts something in the air
//! makes it one by adding a key.
//!
//! What reads them (`Game::air_targets`, a list sorted by key): the bullets'
//! hit test (`Terrain::sweep_rewound` with `strikes_air`), lag
//! compensation's history (`HitBoxFrame::air`), the towers (the tesla's arc,
//! the gun tower's pick), the EMP's ring, the hammer's wave, the rail's
//! slug, the drawn world a client's own bullets are swept against
//! (`PresentWorld`), and the gravity well (BB-42). What changes one:
//! `Game::strike_air` alone.
//!
//! The rules every reader keeps:
//!
//! 1. only a shot that strikes the air hits one - bullets; shells, plasma and
//!    beams pass under;
//! 2. a side never strikes its own side's air targets - but the EMP's ring,
//!    which is blind, and the rail's slug, which goes through everything in
//!    its lane;
//! 3. a tower or an enemy decides to engage a seat's air target only from
//!    inside that seat's sight box - the box it may engage the seat itself
//!    from - and an enemy's from anywhere;
//! 4. nothing on the ground collides with one: walls, tiles, tanks and frogs
//!    pass under it, and its own landing is its owner's business;
//! 5. walks go by key.

use serde::{Deserialize, Serialize};

use crate::Position;
use crate::math::Vec2;
use crate::shell::Owner;

/// Which air target: the walk order, and for a drone its wire key.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum AirKey {
    /// An FPV drone, by its per-round id (`fpv::Drone::id`).
    Drone(u32),
}

/// What struck an air target (`Event::DroneDowned::by`). The variant order
/// is the wire's encoding (`net::events::WireEvent::DroneDowned` carries
/// it as it is).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum AirStrike {
    /// A bullet - a minigun's, either side's, or a gun tower's.
    Bullet,
    /// A tesla coil's arc.
    Tesla,
    /// An EMP's ring.
    Emp,
    /// A sonic hammer's wave.
    Sonic,
    /// A gauss rail's slug.
    Rail,
    /// A rod from god's impact.
    Rod,
    /// A gravity well's core, which swallows it.
    Well,
}

impl AirStrike {
    /// Every cause, in wire order.
    pub const ALL: [AirStrike; 7] = [AirStrike::Bullet, AirStrike::Tesla, AirStrike::Emp, AirStrike::Sonic, AirStrike::Rail, AirStrike::Rod, AirStrike::Well];

    /// Inverse of `name`.
    pub fn parse(name: &str) -> Option<AirStrike> {
        AirStrike::ALL.into_iter().find(|s| s.name() == name)
    }

    /// Lower-case name for tooling and the probe.
    pub fn name(self) -> &'static str {
        match self {
            AirStrike::Bullet => "bullet",
            AirStrike::Tesla => "tesla",
            AirStrike::Emp => "emp",
            AirStrike::Sonic => "sonic",
            AirStrike::Rail => "rail",
            AirStrike::Rod => "rod",
            AirStrike::Well => "well",
        }
    }
}

/// One thing in the air that can be struck there.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AirTarget {
    pub key: AirKey,
    /// Whose it is: who may strike it (`Owner::same_side`).
    pub owner: Owner,
    /// The point on the ground under it.
    pub ground: Position,
    /// Its height over that point (px).
    pub height: f32,
    /// Its ground velocity (px/s): what a gun tower leads it by.
    pub velocity: Vec2,
    /// Half its body's width (px).
    pub half: f32,
}

impl AirTarget {
    /// The box a shot strikes it in: a column from its shadow up to its
    /// body, `half` either side and `half` past each end. A shot crossing
    /// the drone, its shadow or the line between them strikes it - which is
    /// what the picture shows, a drone drawn lifted by its height over its
    /// shadow.
    pub fn strike_box(&self) -> (Position, Position) {
        strike_box(self.ground, self.height, self.half)
    }

    /// Where it is drawn: the ground point lifted by the height.
    pub fn drawn(&self) -> Position {
        Position::new(self.ground.x, self.ground.y - self.height)
    }
}

/// `AirTarget::strike_box` for a body of half width `half` at `height` over
/// `ground`: centre and half extents.
pub fn strike_box(ground: Position, height: f32, half: f32) -> (Position, Position) {
    let h = height.max(0.0);
    (Position::new(ground.x, ground.y - h * 0.5), Position::new(half, h * 0.5 + half))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn target(height: f32) -> AirTarget {
        AirTarget {
            key: AirKey::Drone(1),
            owner: Owner::Player(0),
            ground: Position::new(100.0, 200.0),
            height,
            velocity: Vec2::zero(),
            half: 6.0,
        }
    }

    #[test]
    fn the_strike_box_is_the_column() {
        let (c, h) = target(36.0).strike_box();
        assert_eq!((c.x, c.y), (100.0, 182.0));
        assert_eq!((h.x, h.y), (6.0, 24.0));
        // From the shadow's foot to the body's top.
        assert_eq!(c.y + h.y, 206.0);
        assert_eq!(c.y - h.y, 158.0);
    }

    #[test]
    fn drawn_is_lifted_by_the_height() {
        let d = target(36.0).drawn();
        assert_eq!((d.x, d.y), (100.0, 164.0));
        let (c, h) = target(0.0).strike_box();
        assert_eq!((c.y, h.y), (200.0, 6.0), "on the ground it is a square round its point");
    }
}
