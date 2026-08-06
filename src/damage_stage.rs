use sola_raylib::prelude::*;

use crate::tank::Tank;
use crate::{DAMAGE_TEXTURE_SIZE, DEAD_FRAME, MAX_DAMAGE};

/// A stage of visible damage. Each stage owns a list of overlay frame columns
/// in damage.png that it cycles through at `fps` to animate.
#[derive(Clone, Copy)]
pub struct DamageStage {
    frames: &'static [i32],
    fps: f32,
}

impl DamageStage {
    /// Pick a damage stage from a tank's accumulated damage, or None when the
    /// tank is still pristine (no overlay).
    pub fn from_damage(damage: f32) -> Option<DamageStage> {
        // Static stages use a single frame; a 0 fps would divide-by-zero in the
        // frame math but a single-frame list never indexes past 0 anyway.
        if damage <= 0.0 {
            None
        } else if damage <= 15.0 {
            Some(DamageStage {
                frames: &[0],
                fps: 0.0,
            }) // dusty
        } else if damage <= 30.0 {
            Some(DamageStage {
                frames: &[1],
                fps: 0.0,
            }) // gray
        } else if damage <= 45.0 {
            Some(DamageStage {
                frames: &[2, 3],
                fps: 5.0,
            }) // small smoke
        } else if damage <= 60.0 {
            Some(DamageStage {
                frames: &[4, 5],
                fps: 5.0,
            }) // more smoke
        } else if damage <= 75.0 {
            Some(DamageStage {
                frames: &[6, 7],
                fps: 8.0,
            }) // small fire
        } else if damage < MAX_DAMAGE {
            Some(DamageStage {
                frames: &[8, 9],
                fps: 10.0,
            }) // large fire
        } else {
            Some(DamageStage {
                frames: &[10, 11, 12],
                fps: 8.0,
            }) // wrecked
        }
    }

    /// The overlay column to show at time `t` (seconds). A per-tank phase offset
    /// can be folded into `t` so multiple burning tanks don't animate in lockstep.
    pub fn frame_at(&self, t: f32) -> i32 {
        let i = (t.max(0.0) * self.fps) as usize % self.frames.len();
        self.frames[i]
    }
}

/// Source rectangle for a damage overlay frame (indexed by column) in damage.png.
fn source_rec(col: i32) -> Rectangle {
    Rectangle::new(
        col as f32 * DAMAGE_TEXTURE_SIZE,
        0.0,
        DAMAGE_TEXTURE_SIZE,
        DAMAGE_TEXTURE_SIZE,
    )
}

/// Draw the tank's current damage overlay on top of it, if any. The overlay is
/// axis-aligned (smoke/fire rise upward regardless of the tank's facing) and
/// animates by cycling its stage's frames over `time`, with a per-tank phase
/// offset so multiple burning tanks don't animate in lockstep.
pub fn draw_damage(d: &mut impl RaylibDraw, texture: &Texture2D, tank: &Tank, time: f32) {
    // A burnt-out wreck stops animating and shows the static charred hulk.
    let col = if tank.is_dead() {
        DEAD_FRAME
    } else {
        let Some(stage) = DamageStage::from_damage(tank.damage) else {
            return;
        };
        stage.frame_at(time + tank.anim_phase())
    };
    let src = source_rec(col);
    let size = tank.size();
    let dest = Rectangle::new(tank.position.x, tank.position.y, size, size);
    let origin = Vector2::new(size / 2.0, size / 2.0);
    d.draw_texture_pro(texture, src, dest, origin, 0.0, Color::WHITE);
}
