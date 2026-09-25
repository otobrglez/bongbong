//! The ripple effects' state: a `Shockwave` per ripple in flight, kept by
//! the round (`Game::shocks`, `Game::muzzle_flashes`). The shader that
//! resolves them is `render::shockwave::RippleFx`.

use crate::Position;

/// A ripple effect in flight: a radial-distortion ring expanding from
/// `center`, `time` seconds after it started. Shared shape for both ripple
/// effects in the game: the full-screen kill shockwave (`Game::shock`, at
/// most one at a time) and the small, split-second muzzle-flash heat haze
/// (`Game::muzzle_flashes`, one per shot fired). See `Game::render`.
pub struct Shockwave {
    /// Hit point in world/screen pixels (the game has no camera transform, so
    /// world space and screen space are the same thing).
    pub center: Position,
    /// Seconds since the ripple was triggered.
    pub time: f32,
    /// How hard this one hits, as a multiple of `shockwave_strength` and
    /// `camera_shake_magnitude`. 1.0 is a tank dying; a fence collapsing
    /// has no business shaking the screen as hard as that.
    pub strength: f32,
}

impl Shockwave {
    /// A ripple at full strength.
    pub fn new(center: Position) -> Self {
        Shockwave { center, time: 0.0, strength: 1.0 }
    }

    /// A ripple scaled against a tank kill, which is the 1.0 reference.
    pub fn scaled(center: Position, strength: f32) -> Self {
        Shockwave { center, time: 0.0, strength }
    }

    /// Punch left in it: strength faded by how much of its life is gone.
    /// `Game::finish_frame` evicts by this rather than by age, so a barrel
    /// cascade's little fuse pops cannot shove out the tank explosion that
    /// set them off.
    pub fn remaining(&self) -> f32 {
        let left = 1.0 - (self.time / crate::tuning::tuning().shockwave_duration).clamp(0.0, 1.0);
        self.strength * left
    }
}
