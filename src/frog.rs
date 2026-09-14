//! ToxicFrog: a side's objective (`Side`) - the player's frog is what a
//! Protect/Hunt round defends, the enemy frog what a Hunt round attacks
//! (docs/maps-to-levels.md). No independent AI/decision-making of its own
//! (unlike `ai::Ai`, it never chooses a target or plans a route) - just two
//! reflexes, both driven from `simulation.rs`, the one place that already
//! has the world/physics access either needs: it tries to hop away
//! (`Frog::start_hop`, landing chosen by `combat::frog_hop_target`, which
//! shortens and angles the leap until it finds ground the frog's hull
//! actually fits on rather than giving up near terrain) whenever a shell
//! hits it and survives, or whenever a tank comes inside
//! `Frog::avoid_range` - a real animated leap, `position` interpolated
//! from `hop_start` to `hop_end` over FROG_HOP_SECONDS by `tick`, not a
//! teleport, with `Game::update` keeping the physics body in lockstep
//! every frame so it stays collidable throughout - and it bites
//! (`Frog::start_attack`) the nearest tank of the *other* side
//! (`Side::bites`) once one gets within `Frog::attack_range`. It hops away
//! from any tank, its own side included; it only ever bites a hostile one,
//! so a frog is never a hazard to the side whose objective it is. Any shot
//! damages any frog. The player's frog reaching zero `health`
//! ends the round in a loss, the enemy frog's in a win
//! (`Game::check_round_end`).

use crate::shell::Owner;
use crate::tank::{HealthRamp, RingStyle, draw_ground_ring_at, with_opacity};
use crate::tuning::tuning;
use rapier2d::prelude::RigidBodyHandle;
use serde::{Deserialize, Serialize};
use sola_raylib::prelude::*;

use crate::{
    FROG_ATTACK_FPS,
    FROG_ATTACK_FRAMES,
    FROG_ATTACK_SECONDS,
    FROG_EXPLOSION_FPS,
    FROG_EXPLOSION_FRAMES,
    FROG_FACING_DEADBAND_PX,
    FROG_HOP_FPS,
    FROG_HOP_FRAMES,
    FROG_HOP_SECONDS,
    FROG_HURT_FPS,
    FROG_HURT_FRAMES,
    FROG_HURT_SECONDS,
    FROG_IDLE_FPS,
    FROG_IDLE_FRAMES,
    FROG_SCALE,
    FROG_SPRITE_BODY_CENTER_X,
    FROG_TEXTURE_SIZE,
    Position,
};

/// Whose objective a frog is. Tells the two frogs of a Hunt round apart
/// (round-end rule, ground-ring colour, hunter targeting); the frog itself
/// behaves identically on either side.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Side {
    Player,
    Enemy,
}

impl Side {
    pub fn name(self) -> &'static str {
        match self {
            Side::Player => "player",
            Side::Enemy => "enemy",
        }
    }

    /// Whether a frog on this side bites `owner`'s tank: only ever the
    /// other side's. Your own frog is an objective to defend, not a hazard
    /// to park away from, and an enemy frog must not chew through the pack
    /// guarding it. The hop is deliberately *not* gated on this (see
    /// `Game::frog_reflexes`): the frog stays skittish of every tank.
    pub fn bites(self, owner: Owner) -> bool {
        match self {
            Side::Player => !owner.is_player(),
            Side::Enemy => owner.is_player(),
        }
    }
}

/// Which way a frog's art faces. The pack is authored facing right (see
/// FROG_SPRITE_BODY_CENTER_X), so `Left` is the mirrored draw and there is
/// no second set of sprites - `mirror` is the one place that flip lives.
///
/// Kept on `Frog` rather than derived per frame so it *persists*: a frog
/// that hopped west goes on facing west while it idles there, instead of
/// snapping back to the authored orientation the moment it lands.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Facing {
    Right,
    Left,
}

impl Facing {
    pub fn name(self) -> &'static str {
        match self {
            Facing::Right => "right",
            Facing::Left => "left",
        }
    }

    /// The facing `dx` px of horizontal travel implies, or `None` when that
    /// is too close to vertical to mean anything (FROG_FACING_DEADBAND_PX).
    pub fn from_dx(dx: f32) -> Option<Facing> {
        if dx.abs() < FROG_FACING_DEADBAND_PX {
            None
        } else if dx < 0.0 {
            Some(Facing::Left)
        } else {
            Some(Facing::Right)
        }
    }
}

/// How `facing` is drawn: the sign to multiply the source rectangle's width
/// by - raylib's mirror idiom, as in `blast::draw_blast` - and the screen-px
/// offset that keeps the mirrored body over the same patch of ground.
///
/// The offset is not cosmetic slack: the body sits 2.5 design px left of the
/// cell centre it would be mirrored about (FROG_SPRITE_BODY_CENTER_X), so
/// without it a frog jumps 10 screen px sideways every time its facing
/// flips. `Right` is the identity, so a right-facing frog draws exactly as
/// it always has.
pub fn mirror(facing: Facing) -> (f32, f32) {
    match facing {
        Facing::Right => (1.0, 0.0),
        Facing::Left => (-1.0, -2.0 * (cell_center_x() - FROG_SPRITE_BODY_CENTER_X) * FROG_SCALE),
    }
}

/// The x a mirrored source rectangle reflects about: the centre of the
/// cell's pixel columns, 0..=47, not its width.
fn cell_center_x() -> f32 {
    (FROG_TEXTURE_SIZE - 1.0) / 2.0
}

pub struct Frog {
    pub side: Side,
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
    /// Which way this frog is drawn (see `Facing`). Set by `start_hop` from
    /// where the leap lands and by `start_attack` from where the victim is,
    /// and kept until one of those changes it. Purely a draw-time choice -
    /// nothing in `simulation/` reads it.
    pub facing: Facing,
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
    /// `Obstacle::size` - what sizes its ground ring (`draw_frog_ring`)
    /// exactly as a tank's sizes the tank's.
    pub fn size(&self) -> f32 {
        FROG_TEXTURE_SIZE * FROG_SCALE
    }

    pub fn is_dead(&self) -> bool {
        self.death_elapsed.is_some()
    }

    /// Remaining health as a fraction of `max_health`, 1 to 0 - what the
    /// frog's ring gauge reads.
    pub fn health_fraction(&self) -> f32 {
        if self.max_health > 0.0 { (self.health / self.max_health).clamp(0.0, 1.0) } else { 0.0 }
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
        if self.health <= 0.0 {
            self.death_elapsed = Some(0.0);
        }
    }

    /// Restore health, clamped at `max_health`. A no-op once dead, the same
    /// convention `damage` follows: a frog health pack
    /// (`pickup::PickupKind::FrogHealth`) cannot revive one, and there is no
    /// case where it would matter - the player's frog dying ends the round
    /// on the same frame it happens.
    pub fn heal(&mut self, amount: f32) {
        if self.is_dead() {
            return;
        }
        self.health = (self.health + amount).min(self.max_health);
    }

    /// Alive and undamaged. This is the one state in which a frog health
    /// pack is left on the ground rather than collected, so it is the
    /// whole of that pickup's collect rule (`simulation::pickup_phase`): a
    /// *dead* frog is deliberately not "full", and a pack driven over
    /// after one dies is consumed for nothing like any other side with no
    /// frog to heal.
    pub fn at_full_health(&self) -> bool {
        !self.is_dead() && self.health >= self.max_health
    }

    /// Turn to face `x`, unless it is so nearly straight above or below
    /// that the direction means nothing - then the current facing stands
    /// rather than flipping on the sign of a near-zero number.
    fn face_toward(&mut self, x: f32) {
        if let Some(facing) = Facing::from_dx(x - self.position.x) {
            self.facing = facing;
        }
    }

    /// Begin an animated hop toward a landing spot that's already been
    /// found valid (see `simulation::frog_hop_target`) - `position` doesn't
    /// jump to `target` here, `tick` carries it there smoothly over the
    /// next FROG_HOP_SECONDS, in step with the Hop clip/cooldown this also
    /// starts.
    pub fn start_hop(&mut self, target: Position) {
        // Where the leap *lands*, not the direction it fled: the landing
        // search angles a hop up to a quarter turn off the away direction
        // (`combat::frog_hop_target`), and the sprite has to match what the
        // frog visibly does.
        self.face_toward(target.x);
        self.hop_start = self.position;
        self.hop_end = target;
        self.hop_timer = FROG_HOP_SECONDS;
        self.hop_cooldown = tuning().frog_hop_cooldown_seconds;
    }

    /// Register that the frog just bit a tank - starts the cosmetic Attack
    /// clip/cooldown. The caller (`Game::update`) is responsible for
    /// actually applying damage to the target; this only tracks the frog's
    /// own reaction/pacing state.
    pub fn start_attack(&mut self, victim: Position) {
        // The attack clip lashes its tongue toward the side it faces, so a
        // bite has to turn the frog or the tongue misses the tank.
        self.face_toward(victim.x);
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

/// Draw the frog's side marker as its health gauge: the shared ground ring
/// (`tank::draw_ground_ring_at`, the player tank's own ring in the same
/// size class) under the sprite, its filled arc the remaining health - in
/// the white ramp for the player's frog, the all-red ramp for the enemy's -
/// and the rest of the circle the side's colour dimmed to
/// `health_ring_base_opacity`, so the ring stays a full marker whose side
/// reads at any health. On while the frog lives; gone once it is dead - the
/// explosion crater has no side. Call before `draw_frog`.
pub fn draw_frog_ring(d: &mut impl RaylibDraw, frog: &Frog, time: f32) {
    if frog.is_dead() {
        return;
    }
    let ramp = match frog.side {
        Side::Player => HealthRamp::White,
        Side::Enemy => HealthRamp::Red,
    };
    let base = with_opacity(ramp.base(), tuning().player_ring_opacity * tuning().health_ring_base_opacity);
    let style = RingStyle::Gauge { frac: frog.health_fraction(), ramp, base };
    draw_ground_ring_at(d, frog.position, frog.size(), 0.0, time, style, 1.0);
}

/// Draw the frog: whichever of `textures`' five clips `Frog::anim` picks
/// for this frame, centered at its position. Never rotates (it's not a
/// tank), so this skips the rotation param `draw_tank` needs - same as
/// `draw_obstacle`. It does *mirror*: the art is authored facing right, and
/// `mirror` turns `Frog::facing` into the source rectangle's width sign
/// plus the destination offset that keeps the body over the same ground.
pub fn draw_frog(d: &mut impl RaylibDraw, textures: &FrogTextures, frog: &Frog, t: f32) {
    let (anim, frame) = frog.anim(t);
    let texture = match anim {
        FrogAnim::Idle => textures.idle,
        FrogAnim::Hurt => textures.hurt,
        FrogAnim::Hop => textures.hop,
        FrogAnim::Attack => textures.attack,
        FrogAnim::Explosion => textures.explosion,
    };
    let (flip, offset) = mirror(frog.facing);
    let src = Rectangle::new(
        frame as f32 * FROG_TEXTURE_SIZE,
        0.0,
        FROG_TEXTURE_SIZE * flip,
        FROG_TEXTURE_SIZE,
    );
    let size = frog.size();
    let dest = Rectangle::new(frog.position.x + offset, frog.position.y, size, size);
    let origin = Vector2::new(size / 2.0, size / 2.0);
    d.draw_texture_pro(texture, src, dest, origin, 0.0, Color::WHITE);
}

#[cfg(test)]
mod facing_tests {
    use super::*;

    fn frog_at(x: f32) -> Frog {
        Frog {
            side: Side::Player,
            position: Position::new(x, 300.0),
            health: 100.0,
            max_health: 100.0,
            variant: 0,
            body: RigidBodyHandle::invalid(),
            hurt_timer: 0.0,
            hop_timer: 0.0,
            hop_start: Position::new(x, 300.0),
            hop_end: Position::new(x, 300.0),
            hop_cooldown: 0.0,
            attack_timer: 0.0,
            attack_cooldown: 0.0,
            facing: Facing::Right,
            death_elapsed: None,
        }
    }

    /// Where the frog's *body* lands on screen for a given facing: the
    /// drawn cell's left edge plus the body's own column, mirrored or not.
    /// The whole point of `mirror`'s offset is that this does not move.
    fn body_center_on_screen(frog: &Frog) -> f32 {
        let (flip, offset) = mirror(frog.facing);
        let col = if flip < 0.0 {
            (FROG_TEXTURE_SIZE - 1.0) - FROG_SPRITE_BODY_CENTER_X
        } else {
            FROG_SPRITE_BODY_CENTER_X
        };
        frog.position.x + offset + (col - cell_center_x()) * FROG_SCALE
    }

    #[test]
    fn a_hop_faces_the_way_it_lands() {
        let mut frog = frog_at(500.0);
        frog.start_hop(Position::new(400.0, 300.0));
        assert_eq!(frog.facing, Facing::Left, "a hop west faces west");

        let mut frog = frog_at(500.0);
        frog.facing = Facing::Left;
        frog.start_hop(Position::new(600.0, 300.0));
        assert_eq!(frog.facing, Facing::Right, "a hop east faces east");
    }

    /// The hop fan reaches a quarter turn either way, so a dead-vertical
    /// leap is a real case - and its sign is float noise, not a direction.
    #[test]
    fn a_vertical_hop_keeps_the_facing_it_had() {
        for facing in [Facing::Left, Facing::Right] {
            let mut frog = frog_at(500.0);
            frog.facing = facing;
            frog.start_hop(Position::new(500.0 + FROG_FACING_DEADBAND_PX * 0.5, 380.0));
            assert_eq!(frog.facing, facing, "a near-vertical hop turned the frog");
        }
    }

    #[test]
    fn a_bite_faces_the_tank_it_bites() {
        let mut frog = frog_at(500.0);
        frog.start_attack(Position::new(440.0, 300.0));
        assert_eq!(frog.facing, Facing::Left, "the tongue has to lash at the tank");
        assert!(frog.attack_timer > 0.0, "the clip still starts");

        frog.start_attack(Position::new(560.0, 300.0));
        assert_eq!(frog.facing, Facing::Right);
    }

    /// `Right` is the identity, so every existing frog draws exactly as it
    /// always has.
    #[test]
    fn facing_right_draws_unchanged() {
        assert_eq!(mirror(Facing::Right), (1.0, 0.0));
    }

    /// The offset is load-bearing, not slack: the body sits off the cell's
    /// centre, so a bare width flip would slide the frog sideways every
    /// time it turned around.
    #[test]
    fn mirroring_keeps_the_body_on_the_same_ground() {
        let (flip, offset) = mirror(Facing::Left);
        assert!(flip < 0.0, "Left is the mirrored draw");
        assert!(offset != 0.0, "a bare flip would move the body off its own position");

        let mut right = frog_at(500.0);
        right.facing = Facing::Right;
        let mut left = frog_at(500.0);
        left.facing = Facing::Left;
        let (a, b) = (body_center_on_screen(&right), body_center_on_screen(&left));
        assert!((a - b).abs() < 0.001, "the body moved {} px when the frog turned around", (a - b).abs());
    }
}
