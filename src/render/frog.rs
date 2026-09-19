//! The frog's filmstrips as textures (`frog.rs` owns the frog and its
//! `Canvas` draw).

use sola_raylib::prelude::*;

use crate::frog::FrogAnim;

/// One colour variant's full set of five clips (see docs/FROG_SPEC.md) -
/// `app.rs` loads one of these per `FROG_VARIANT_DIRS` entry and keeps the
/// whole set alive for the game's lifetime; `game::Textures::frog_variants`
/// then resolves `canvas::Sheet::Frog { variant, clip }` through `clip`.
pub struct FrogVariantTextures {
    pub idle: Texture2D,
    pub hurt: Texture2D,
    pub hop: Texture2D,
    pub attack: Texture2D,
    pub explosion: Texture2D,
}

impl FrogVariantTextures {
    /// The filmstrip for one clip.
    pub fn clip(&self, clip: FrogAnim) -> &Texture2D {
        match clip {
            FrogAnim::Idle => &self.idle,
            FrogAnim::Hurt => &self.hurt,
            FrogAnim::Hop => &self.hop,
            FrogAnim::Attack => &self.attack,
            FrogAnim::Explosion => &self.explosion,
        }
    }
}
