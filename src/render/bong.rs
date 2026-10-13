//! The bongs' filmstrips as textures (`bong.rs` owns the bong and its
//! `Canvas` draw).

use sola_raylib::prelude::*;

use crate::bong::{BongAnim, BongKind};
use crate::canvas::Sheet;

/// One character's full set of five clips (see docs/BONG_SPEC.md) -
/// `app.rs` loads one of these per `BongKind`, in `BongKind::ALL` order,
/// and keeps the whole set alive for the game's lifetime;
/// `game::Textures::bongs` then resolves `canvas::Sheet::Bong { kind, clip }`
/// through `clip`.
pub struct BongTextures {
    pub idle: Texture2D,
    pub hurt: Texture2D,
    pub hop: Texture2D,
    pub attack: Texture2D,
    pub explosion: Texture2D,
}

impl BongTextures {
    /// Load `kind`'s five filmstrips from `static/bongs/<kind>/`.
    pub fn load(rl: &mut RaylibHandle, thread: &RaylibThread, kind: BongKind) -> BongTextures {
        let mut clip = |clip: BongAnim| {
            let path = Sheet::Bong { kind, clip }.path();
            rl.load_texture(thread, &path).unwrap_or_else(|e| panic!("failed loading {path}: {e}"))
        };
        BongTextures {
            idle: clip(BongAnim::Idle),
            hurt: clip(BongAnim::Hurt),
            hop: clip(BongAnim::Hop),
            attack: clip(BongAnim::Attack),
            explosion: clip(BongAnim::Explosion),
        }
    }

    /// The filmstrip for one clip.
    pub fn clip(&self, clip: BongAnim) -> &Texture2D {
        match clip {
            BongAnim::Idle => &self.idle,
            BongAnim::Hurt => &self.hurt,
            BongAnim::Hop => &self.hop,
            BongAnim::Attack => &self.attack,
            BongAnim::Explosion => &self.explosion,
        }
    }
}
