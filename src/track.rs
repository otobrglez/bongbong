use crate::canvas::{Canvas, Sheet};
use crate::tuning::tuning;
use crate::math::{Color, Rectangle, Vec2};

use crate::{Position, TRACK_TEXTURE_SIZE};

/// A single tread mark pressed into the ground where a tank drove. Marks are laid
/// down along a tank's path and slowly fade out as they age.
pub struct Track {
    /// Center position on screen (pixels).
    pub position: Position,
    /// Facing angle in degrees, matching the tank's heading when it was laid.
    pub rotation: f32,
    /// How much to scale the 32x32 sprite (matches the tank that laid it,
    /// including its per-chassis weight - see TRACK_WEIGHT_SCALE_BY_ROW).
    pub scale: f32,
    /// This mark's fresh (age 0) opacity - TRACK_MAX_OPACITY scaled by the
    /// laying tank's chassis weight (see TRACK_WEIGHT_OPACITY_BY_ROW), so a
    /// heavier tank presses a darker mark, not just a bigger one.
    pub max_opacity: f32,
    /// Seconds since the mark was laid; drives the fade-out.
    pub age: f32,
    /// Burnt into the ground by a tank dying on top of it: darker, and it
    /// never fades. The kill site stays legible for the rest of the round,
    /// after the wreck itself has been cleared away by a wave.
    pub scorched: bool,
    /// Laid by a hull that waded out of water within
    /// `water_wet_track_seconds`: darker, and it fades over that time
    /// rather than `track_lifetime`.
    pub wet: bool,
}

impl Track {
    /// Advance the mark's age. Returns true once it has fully faded and can be
    /// dropped.
    pub fn tick(&mut self, dt: f32) -> bool {
        self.age += dt;
        !self.scorched && self.age >= self.lifetime()
    }

    fn lifetime(&self) -> f32 {
        if self.wet { tuning().water_wet_track_seconds.max(0.05) } else { tuning().track_lifetime }
    }

    /// Remaining opacity, fading linearly from `max_opacity` (fresh) to 0.0
    /// (gone) so marks stay faint even when brand new.
    fn opacity(&self) -> f32 {
        if self.scorched {
            return (self.max_opacity * tuning().wreck_track_darken).clamp(0.0, 1.0);
        }
        let darken = if self.wet { tuning().water_wet_track_darken } else { 1.0 };
        ((1.0 - self.age / self.lifetime()).clamp(0.0, 1.0) * self.max_opacity * darken).clamp(0.0, 1.0)
    }
}

/// Draw a track mark, centered on its position, rotated to the tank's heading and
/// faded according to its age.
pub fn draw_track(c: &mut impl Canvas, track: &Track) {
    let src = Rectangle::new(0.0, 0.0, TRACK_TEXTURE_SIZE, TRACK_TEXTURE_SIZE);
    let size = TRACK_TEXTURE_SIZE * track.scale;

    let dest = Rectangle::new(track.position.x, track.position.y, size, size);
    let origin = Vec2::new(size / 2.0, size / 2.0);

    // Fade the whole sprite by scaling its alpha with the mark's remaining life.
    let tint = Color::WHITE.alpha(track.opacity());
    c.blit(Sheet::Tracks, src, dest, origin, track.rotation, tint);
}
