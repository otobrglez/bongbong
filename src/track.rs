use sola_raylib::prelude::*;

use crate::{Position, TRACK_LIFETIME, TRACK_MAX_OPACITY, TRACK_TEXTURE_SIZE};

/// A single tread mark pressed into the ground where a tank drove. Marks are laid
/// down along a tank's path and slowly fade out as they age.
pub struct Track {
    /// Center position on screen (pixels).
    pub position: Position,
    /// Facing angle in degrees, matching the tank's heading when it was laid.
    pub rotation: f32,
    /// How much to scale the 32x32 sprite (matches the tank that laid it).
    pub scale: f32,
    /// Seconds since the mark was laid; drives the fade-out.
    pub age: f32,
}

impl Track {
    /// Advance the mark's age. Returns true once it has fully faded and can be
    /// dropped.
    pub fn tick(&mut self, dt: f32) -> bool {
        self.age += dt;
        self.age >= TRACK_LIFETIME
    }

    /// Remaining opacity, fading linearly from TRACK_MAX_OPACITY (fresh) to 0.0
    /// (gone) so marks stay faint even when brand new.
    fn opacity(&self) -> f32 {
        (1.0 - self.age / TRACK_LIFETIME).clamp(0.0, 1.0) * TRACK_MAX_OPACITY
    }
}

/// Draw a track mark, centered on its position, rotated to the tank's heading and
/// faded according to its age.
pub fn draw_track(d: &mut impl RaylibDraw, texture: &Texture2D, track: &Track) {
    let src = Rectangle::new(0.0, 0.0, TRACK_TEXTURE_SIZE, TRACK_TEXTURE_SIZE);
    let size = TRACK_TEXTURE_SIZE * track.scale;

    let dest = Rectangle::new(track.position.x, track.position.y, size, size);
    let origin = Vector2::new(size / 2.0, size / 2.0);

    // Fade the whole sprite by scaling its alpha with the mark's remaining life.
    let tint = Color::WHITE.alpha(track.opacity());
    d.draw_texture_pro(texture, src, dest, origin, track.rotation, tint);
}
