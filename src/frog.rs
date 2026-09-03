//! ToxicFrog: the player's protect-objective. No independent AI/decision-
//! making of its own (unlike `ai::Ai`, it never chooses a target or plans a
//! route) - just two reflexes, both driven from `simulation.rs`, the one
//! place that already has the world/physics access either needs: it tries
//! to hop away (`Frog::start_hop`, landing chosen by
//! `simulation::frog_hop_target`) whenever a shell hits it and survives -
//! a real animated leap, `position` interpolated from `hop_start` to
//! `hop_end` over FROG_HOP_SECONDS by `tick`, not a teleport, with
//! `Game::update` keeping the physics body in lockstep every frame so it
//! stays collidable throughout - and it bites (`Frog::start_attack`)
//! whichever tank - either side - is nearest once one gets within
//! `Frog::attack_range`. If its `health` reaches zero the round ends in a
//! loss (`Game::update`), the same severity as the player's own tank being
//! destroyed.

use crate::tuning::tuning;
use rapier2d::prelude::RigidBodyHandle;
use sola_raylib::prelude::*;

use crate::{
    FROG_ATTACK_FPS,
    FROG_ATTACK_FRAMES,
    FROG_ATTACK_SECONDS,
    FROG_EXPLOSION_FPS,
    FROG_EXPLOSION_FRAMES,
    FROG_HOP_FPS,
    FROG_HOP_FRAMES,
    FROG_HOP_SECONDS,
    FROG_HURT_FPS,
    FROG_HURT_FRAMES,
    FROG_HURT_SECONDS,
    FROG_IDLE_FPS,
    FROG_IDLE_FRAMES,
    FROG_SCALE,
    FROG_TEXTURE_SIZE,
    Position,
};

pub struct Frog {
    pub position: Position,
    pub health: f32,
    pub max_health: f32,
    /// This frog's colour: an index into `FROG_VARIANT_DIRS`, rolled once at
    /// spawn and fixed for the round. Purely cosmetic - every variant shares
    /// identical layout/frame counts/timing (see docs/FROG_SPEC.md), so
    /// nothing gameplay-relevant reads this - only `game.rs::render` does,
    /// to pick which of `Textures::frog_variants`' texture sets to draw.
    pub variant: i32,
    /// This frog's rapier fixed-body collider (see
    /// `physics::Physics::spawn_static`) - the same "blocks tank movement
    /// and doubles as the shell-hit target" role `Obstacle::body` plays,
    /// just for a living thing instead of terrain.
    pub body: RigidBodyHandle,
    /// Seconds remaining showing the Hurt flicker after the most recent
    /// hit - set by `damage`, ticked down by `tick`. Purely a one-shot
    /// reaction animation; drives *only* which sprite frame is drawn.
    pub hurt_timer: f32,
    /// Seconds remaining to show this frog's overhead health bar
    /// (`Game::render`) - same "only visible for a few seconds after a
    /// hit, then fades" convention as `Tank::hit_flash_timer`/`mark_hit`,
    /// reusing the same HEALTH_BAR_OVERHEAD_SECONDS/
    /// HEALTH_BAR_OVERHEAD_FADE_SECONDS constants. Kept separate from
    /// `hurt_timer` since the two run on different durations (the Hurt
    /// sprite flicker is much shorter than the bar's visible window) and
    /// answer different questions - "which sprite frame" vs. "is the bar
    /// showing".
    pub hit_flash_timer: f32,
    /// Seconds remaining in an in-flight hop - set to FROG_HOP_SECONDS by
    /// `start_hop`, ticked down by `tick`. While this is positive,
    /// `position` is being actively interpolated from `hop_start` to
    /// `hop_end` (see `tick`) in step with the Hop clip, so the two always
    /// finish together; never gates whether a hop *can* start, that's
    /// `hop_cooldown`.
    pub hop_timer: f32,
    /// Where the current (or most recent) hop began - `position`'s value
    /// the instant `start_hop` was called. Only meaningful while
    /// `hop_timer > 0.0`.
    pub hop_start: Position,
    /// Where the current (or most recent) hop is carrying `position` to -
    /// set by `start_hop` from the landing spot `simulation::frog_hop_target`
    /// found. Only meaningful while `hop_timer > 0.0`.
    pub hop_end: Position,
    /// Seconds remaining before another hop can trigger - set to
    /// FROG_HOP_COOLDOWN_SECONDS by `start_hop`, so a rapid volley of hits
    /// can't chain-hop the frog every single frame one lands.
    pub hop_cooldown: f32,
    /// Seconds remaining showing the Attack clip after the most recent
    /// bite - set by `start_attack`, ticked down by `tick`.
    pub attack_timer: f32,
    /// Seconds remaining before the frog can bite again - set to
    /// FROG_ATTACK_COOLDOWN_SECONDS by `start_attack`, pacing damage output
    /// against a tank that lingers in range the same way a tank's own
    /// `fire_cooldown` paces its shots.
    pub attack_cooldown: f32,
    /// Seconds elapsed since `health` first reached zero - `None` while
    /// still alive. Set once by `damage` and never cleared; drives the
    /// Explosion sequence (see `anim`), which holds on its last frame
    /// forever rather than looping or disappearing, once
    /// FROG_EXPLOSION_FRAMES/FROG_EXPLOSION_FPS worth of it has played.
    pub death_elapsed: Option<f32>,
}

/// Which of the five filmstrips (see docs/FROG_SPEC.md) `anim` picked for
/// this frame, plus which frame within it.
pub enum FrogAnim {
    Idle,
    Hurt,
    Hop,
    Attack,
    Explosion,
}

impl Frog {
    /// Side length of this frog's sprite on screen, matching `Tank::size`/
    /// `Obstacle::size` - used to place its overhead health bar
    /// (`Game::render`) the same "half the sprite height, plus a gap" way.
    pub fn size(&self) -> f32 {
        FROG_TEXTURE_SIZE * FROG_SCALE
    }

    pub fn is_dead(&self) -> bool {
        self.death_elapsed.is_some()
    }

    /// How far (px) a single hop covers - see FROG_HOP_DISTANCE_FACTOR's
    /// comment for why this is a factor of `size()` rather than a flat
    /// constant.
    pub fn hop_distance(&self) -> f32 {
        self.size() * tuning().frog_hop_distance_factor
    }

    /// How close (px, center to center) a tank has to get before the frog
    /// bites it - see FROG_HOP_DISTANCE_FACTOR's comment.
    pub fn attack_range(&self) -> f32 {
        self.size() * tuning().frog_attack_range_factor
    }

    /// How close (px, center to center) a tank has to get before the frog
    /// tries to hop away from it - see FROG_AVOID_RANGE_FACTOR's comment.
    pub fn avoid_range(&self) -> f32 {
        self.size() * tuning().frog_avoid_range_factor
    }

    /// Whether a hop can trigger right now - alive, and not still cooling
    /// down from the last one. Doesn't know or care whether a *landing
    /// spot* is actually available; that's `simulation::frog_hop_target`'s
    /// job, checked separately since it needs world/obstacle data this
    /// type deliberately has no access to.
    pub fn can_hop(&self) -> bool {
        !self.is_dead() && self.hop_cooldown <= 0.0
    }

    /// Whether the frog can bite right now - alive, and not still cooling
    /// down from the last bite.
    pub fn can_attack(&self) -> bool {
        !self.is_dead() && self.attack_cooldown <= 0.0
    }

    /// Apply shell damage. A no-op once already dead, so callers don't need
    /// to gate on `is_dead()` themselves (same convention as
    /// `Obstacle::damage`).
    pub fn damage(&mut self, amount: f32) {
        if self.is_dead() {
            return;
        }
        self.health = (self.health - amount).max(0.0);
        self.hurt_timer = FROG_HURT_SECONDS;
        self.hit_flash_timer = tuning().health_bar_overhead_seconds;
        if self.health <= 0.0 {
            self.death_elapsed = Some(0.0);
        }
    }

    /// Begin an animated hop toward a landing spot that's already been
    /// found valid (see `simulation::frog_hop_target`) - `position` doesn't
    /// jump to `target` here, `tick` carries it there smoothly over the
    /// next FROG_HOP_SECONDS, in step with the Hop clip/cooldown this also
    /// starts.
    pub fn start_hop(&mut self, target: Position) {
        self.hop_start = self.position;
        self.hop_end = target;
        self.hop_timer = FROG_HOP_SECONDS;
        self.hop_cooldown = tuning().frog_hop_cooldown_seconds;
    }

    /// Register that the frog just bit a tank - starts the cosmetic Attack
    /// clip/cooldown. The caller (`Game::update`) is responsible for
    /// actually applying damage to the target; this only tracks the frog's
    /// own reaction/pacing state.
    pub fn start_attack(&mut self) {
        self.attack_timer = FROG_ATTACK_SECONDS;
        self.attack_cooldown = tuning().frog_attack_cooldown_seconds;
    }

    /// Advance this frog's per-frame animation/cooldown timers, including
    /// carrying `position` along an in-flight hop (see `hop_timer`'s own
    /// comment). Called every frame regardless of round outcome (see
    /// `Game::update`'s "round is over" branch, which still ticks a
    /// wrecked Tank's fire/a burning Obstacle's char loop) so a fresh
    /// Explosion - or a hop already in flight when the round ends - keeps
    /// playing through the end-of-round restart countdown instead of
    /// freezing mid-animation. `Game::update` is responsible for copying
    /// the resulting `position` into this frog's physics body every frame
    /// this runs, the same way it reads tank positions back *from*
    /// physics - here the data flows the other way, since a hop is
    /// authored in game code, not by rapier's own integration.
    pub fn tick(&mut self, dt: f32) {
        self.hurt_timer = (self.hurt_timer - dt).max(0.0);
        self.hit_flash_timer = (self.hit_flash_timer - dt).max(0.0);
        if self.hop_timer > 0.0 {
            self.hop_timer = (self.hop_timer - dt).max(0.0);
            let frac = (1.0 - self.hop_timer / FROG_HOP_SECONDS).clamp(0.0, 1.0);
            self.position = Position::new(
                self.hop_start.x + (self.hop_end.x - self.hop_start.x) * frac,
                self.hop_start.y + (self.hop_end.y - self.hop_start.y) * frac,
            );
        }
        self.hop_cooldown = (self.hop_cooldown - dt).max(0.0);
        self.attack_timer = (self.attack_timer - dt).max(0.0);
        self.attack_cooldown = (self.attack_cooldown - dt).max(0.0);
        if let Some(elapsed) = &mut self.death_elapsed {
            *elapsed += dt;
        }
    }

    /// Which animation + frame to show right now, given the global clock
    /// `t` (only used for the looping Idle - same idea as
    /// `damage_stage::DamageStage::frame_at`). Priority order: a death
    /// always wins; otherwise a fresh hop (it visually *is* the reaction to
    /// being shot, superseding the plain Hurt flicker) beats a fresh bite,
    /// which beats a fresh hit that didn't trigger a hop, which beats
    /// idling.
    pub fn anim(&self, t: f32) -> (FrogAnim, i32) {
        if let Some(elapsed) = self.death_elapsed {
            let frame = (elapsed * FROG_EXPLOSION_FPS) as i32;
            return (FrogAnim::Explosion, frame.min(FROG_EXPLOSION_FRAMES - 1));
        }
        if self.hop_timer > 0.0 {
            let elapsed = (FROG_HOP_SECONDS - self.hop_timer).max(0.0);
            let frame = (elapsed * FROG_HOP_FPS) as i32;
            return (FrogAnim::Hop, frame.clamp(0, FROG_HOP_FRAMES - 1));
        }
        if self.attack_timer > 0.0 {
            let elapsed = (FROG_ATTACK_SECONDS - self.attack_timer).max(0.0);
            let frame = (elapsed * FROG_ATTACK_FPS) as i32;
            return (FrogAnim::Attack, frame.clamp(0, FROG_ATTACK_FRAMES - 1));
        }
        if self.hurt_timer > 0.0 {
            let elapsed = (FROG_HURT_SECONDS - self.hurt_timer).max(0.0);
            let frame = (elapsed * FROG_HURT_FPS) as i32;
            return (FrogAnim::Hurt, frame.clamp(0, FROG_HURT_FRAMES - 1));
        }
        let frame = (t.max(0.0) * FROG_IDLE_FPS) as i32 % FROG_IDLE_FRAMES;
        (FrogAnim::Idle, frame)
    }
}

/// The `static/toxic_frog/<dir>/` colour variants (see docs/FROG_SPEC.md and
/// `static/toxic_frog/SOURCE.md`), in `Frog::variant` index order - the
/// single source of truth for both "which directory" and "how many
/// variants exist" (`main.rs` loads one `FrogVariantTextures` per entry;
/// `Game::init` rolls `Frog::variant` via `rng.random_range(0..FROG_VARIANT_DIRS.len())`).
/// All six are pixel-layout-identical (same frame counts/timing/cell size),
/// just a different colour third-party art asset - see `SOURCE.md` for the
/// pack-folder each one came from.
pub const FROG_VARIANT_DIRS: [&str; 6] = [
    "purple_white",
    "blue_blue",
    "blue_brown",
    "green_blue",
    "green_brown",
    "purple_blue",
];

/// The five animation filmstrips `draw_frog` picks from (see
/// docs/FROG_SPEC.md), bundled into one param the same way `game::Textures`
/// bundles the rest of the game's atlases - so `draw_frog`'s signature
/// doesn't grow every time another clip gets wired in.
pub struct FrogTextures<'a> {
    pub idle: &'a Texture2D,
    pub hurt: &'a Texture2D,
    pub hop: &'a Texture2D,
    pub attack: &'a Texture2D,
    pub explosion: &'a Texture2D,
}

/// One colour variant's full set of five clips, owned rather than borrowed
/// (unlike `FrogTextures`) - `main.rs` loads one of these per
/// `FROG_VARIANT_DIRS` entry and keeps the whole set alive for the game's
/// lifetime; `game::Textures::frog_variants` then hands `render` a slice of
/// them to index into by `Frog::variant` each frame.
pub struct FrogVariantTextures {
    pub idle: Texture2D,
    pub hurt: Texture2D,
    pub hop: Texture2D,
    pub attack: Texture2D,
    pub explosion: Texture2D,
}

impl FrogVariantTextures {
    /// Borrow this variant's five clips as a `FrogTextures` for `draw_frog`.
    pub fn as_frog_textures(&self) -> FrogTextures<'_> {
        FrogTextures {
            idle: &self.idle,
            hurt: &self.hurt,
            hop: &self.hop,
            attack: &self.attack,
            explosion: &self.explosion,
        }
    }
}

/// Draw the frog: whichever of `textures`' five clips `Frog::anim` picks
/// for this frame, centered at its position. Never rotates (it's not a
/// tank), so this skips the rotation param `draw_tank` needs - same as
/// `draw_obstacle`.
pub fn draw_frog(d: &mut impl RaylibDraw, textures: &FrogTextures, frog: &Frog, t: f32) {
    let (anim, frame) = frog.anim(t);
    let texture = match anim {
        FrogAnim::Idle => textures.idle,
        FrogAnim::Hurt => textures.hurt,
        FrogAnim::Hop => textures.hop,
        FrogAnim::Attack => textures.attack,
        FrogAnim::Explosion => textures.explosion,
    };
    let src = Rectangle::new(
        frame as f32 * FROG_TEXTURE_SIZE,
        0.0,
        FROG_TEXTURE_SIZE,
        FROG_TEXTURE_SIZE,
    );
    let size = frog.size();
    let dest = Rectangle::new(frog.position.x, frog.position.y, size, size);
    let origin = Vector2::new(size / 2.0, size / 2.0);
    d.draw_texture_pro(texture, src, dest, origin, 0.0, Color::WHITE);
}
