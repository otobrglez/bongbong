//! The bong: a side's objective (`Side`) - the player's bong is what a
//! Protect/Hunt round defends, the enemy bong what a Hunt round attacks
//! (docs/maps-to-levels.md). No independent AI/decision-making of its own
//! (unlike `ai::Ai`, it never chooses a target or plans a route) - just two
//! reflexes, both driven from `simulation.rs`, the one place that already
//! has the world/physics access either needs: it tries to hop away
//! (`Bong::start_hop`, landing chosen by `combat::bong_hop_target`, which
//! shortens and angles the leap until it finds ground the bong's hull
//! actually fits on rather than giving up near terrain) whenever a shell
//! hits it and survives, or whenever a tank comes inside
//! `Bong::avoid_range` - a real animated leap, `position` interpolated
//! from `hop_start` to `hop_end` over BONG_HOP_SECONDS by `tick`, not a
//! teleport, with `Game::update` keeping the physics body in lockstep
//! every frame so it stays collidable throughout - and it bites
//! (`Bong::start_attack`) the nearest tank of the *other* side
//! (`Side::bites`) once one gets within `Bong::attack_range`. It hops away
//! from any tank, its own side included; it only ever bites a hostile one,
//! so a bong is never a hazard to the side whose objective it is. Any shot
//! damages any bong. The player's bong reaching zero `health`
//! ends the round in a loss, the enemy bong's in a win
//! (`Game::check_round_end`).

use crate::math;
use crate::canvas::{Canvas, Sheet};
use crate::shell::Owner;
use crate::tank::{HealthRamp, RingStyle, draw_ground_ring_at, with_opacity};
use crate::tuning::tuning;
use rapier2d::prelude::RigidBodyHandle;
use serde::{Deserialize, Serialize};
use crate::math::{Color, Rectangle, Vec2};

use crate::{
    BONG_ATTACK_FPS,
    BONG_ATTACK_FRAMES,
    BONG_ATTACK_SECONDS,
    BONG_EXPLOSION_FPS,
    BONG_EXPLOSION_FRAMES,
    BONG_FACING_DEADBAND_PX,
    BONG_HOP_FPS,
    BONG_HOP_FRAMES,
    BONG_HOP_SECONDS,
    BONG_HURT_FPS,
    BONG_HURT_FRAMES,
    BONG_HURT_SECONDS,
    BONG_IDLE_FPS,
    BONG_IDLE_FRAMES,
    BONG_SCALE,
    BONG_SPRITE_BODY_CENTER_X,
    BONG_TEXTURE_SIZE,
    Position,
};

/// Which character a bong is (docs/BONG_SPEC.md): Spike, Oggle and Gloop
/// stand for the player, Puff, Capi, Blip and Fuzz for the enemy. A map
/// names its two (`MapFile::bong_kind`); the kind is how the bong looks and
/// nothing that plays reads it.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BongKind {
    Spike,
    Oggle,
    Gloop,
    Puff,
    Capi,
    Blip,
    Fuzz,
}

impl BongKind {
    /// Every kind, the player's three first.
    pub const ALL: [BongKind; 7] = [
        BongKind::Spike,
        BongKind::Oggle,
        BongKind::Gloop,
        BongKind::Puff,
        BongKind::Capi,
        BongKind::Blip,
        BongKind::Fuzz,
    ];

    /// The kinds a side's bong may be, its default first.
    pub fn of(side: Side) -> &'static [BongKind] {
        match side {
            Side::Player => &BongKind::ALL[..3],
            Side::Enemy => &[BongKind::Blip, BongKind::Puff, BongKind::Capi, BongKind::Fuzz],
        }
    }

    /// The side this kind stands for.
    pub fn side(self) -> Side {
        if BongKind::of(Side::Player).contains(&self) { Side::Player } else { Side::Enemy }
    }

    /// A side's bong when its map names none: Bong Spike and Bong Blip.
    pub fn default_for(side: Side) -> BongKind {
        BongKind::of(side)[0]
    }

    /// The kind's name in a map file and its sheet folder,
    /// `static/bongs/<name>/`.
    pub fn name(self) -> &'static str {
        match self {
            BongKind::Spike => "spike",
            BongKind::Oggle => "oggle",
            BongKind::Gloop => "gloop",
            BongKind::Puff => "puff",
            BongKind::Capi => "capi",
            BongKind::Blip => "blip",
            BongKind::Fuzz => "fuzz",
        }
    }

    /// What a player reads: "Bong Spike".
    pub fn title(self) -> crate::text::Key {
        use crate::text::keys;
        match self {
            BongKind::Spike => keys::BONG_NAME_SPIKE,
            BongKind::Oggle => keys::BONG_NAME_OGGLE,
            BongKind::Gloop => keys::BONG_NAME_GLOOP,
            BongKind::Puff => keys::BONG_NAME_PUFF,
            BongKind::Capi => keys::BONG_NAME_CAPI,
            BongKind::Blip => keys::BONG_NAME_BLIP,
            BongKind::Fuzz => keys::BONG_NAME_FUZZ,
        }
    }

    /// The kind's place in `ALL`.
    pub fn index(self) -> usize {
        BongKind::ALL.iter().position(|&k| k == self).unwrap_or(0)
    }

    /// The kind called `name` (`name`'s inverse).
    pub fn parse(name: &str) -> Option<BongKind> {
        BongKind::ALL.into_iter().find(|k| k.name() == name)
    }
}

/// Whose objective a bong is. Tells the two bongs of a Hunt round apart
/// (round-end rule, ground-ring colour, hunter targeting); the bong itself
/// behaves identically on either side.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug, Serialize, Deserialize)]
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

    /// Whether a bong on this side bites `owner`'s tank: only ever the
    /// other side's. Your own bong is an objective to defend, not a hazard
    /// to park away from, and an enemy bong must not chew through the pack
    /// guarding it. The hop is deliberately *not* gated on this (see
    /// `Game::bong_reflexes`): the bong stays skittish of every tank.
    pub fn bites(self, owner: Owner) -> bool {
        match self {
            Side::Player => !owner.is_player(),
            Side::Enemy => owner.is_player(),
        }
    }
}

/// Which way a bong's art faces. The pack is authored facing right (see
/// BONG_SPRITE_BODY_CENTER_X), so `Left` is the mirrored draw and there is
/// no second set of sprites - `mirror` is the one place that flip lives.
///
/// Kept on `Bong` rather than derived per frame so it *persists*: a bong
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
    /// is too close to vertical to mean anything (BONG_FACING_DEADBAND_PX).
    pub fn from_dx(dx: f32) -> Option<Facing> {
        if dx.abs() < BONG_FACING_DEADBAND_PX {
            None
        } else if dx < 0.0 {
            Some(Facing::Left)
        } else {
            Some(Facing::Right)
        }
    }
}

/// How `facing` is drawn: the sign to multiply the source rectangle's width
/// by - raylib's mirror idiom, as in `blast::draw_scorch` - and the screen-px
/// offset that keeps the mirrored body over the same patch of ground.
///
/// The offset is the distance between the body's centre
/// (BONG_SPRITE_BODY_CENTER_X) and the cell's, doubled and scaled: zero for
/// the generated bongs, which are centred on the cell, and what keeps any
/// off-centre art on the same ground when it turns. `Right` is the identity.
pub fn mirror(facing: Facing) -> (f32, f32) {
    match facing {
        Facing::Right => (1.0, 0.0),
        Facing::Left => (-1.0, -2.0 * (cell_center_x() - BONG_SPRITE_BODY_CENTER_X) * BONG_SCALE),
    }
}

/// The x a mirrored source rectangle reflects about: the centre of the
/// cell's pixel columns, 0..=47, not its width.
fn cell_center_x() -> f32 {
    (BONG_TEXTURE_SIZE - 1.0) / 2.0
}

pub struct Bong {
    pub side: Side,
    pub position: Position,
    pub health: f32,
    pub max_health: f32,
    /// Which character this bong is, the map's choice for its side
    /// (`MapFile::bong_kind`). Purely cosmetic - every kind shares the
    /// same clips, frame counts and timing (docs/BONG_SPEC.md) - so only
    /// drawing reads it.
    pub kind: BongKind,
    /// This bong's rapier fixed-body collider (see
    /// `physics::Physics::spawn_static`) - the same "blocks tank movement
    /// and doubles as the shell-hit target" role `Obstacle::body` plays,
    /// just for a living thing instead of terrain.
    pub body: RigidBodyHandle,
    /// Seconds remaining showing the Hurt flicker after the most recent
    /// hit - set by `damage`, ticked down by `tick`. Purely a one-shot
    /// reaction animation; drives *only* which sprite frame is drawn.
    pub hurt_timer: f32,
    /// Seconds remaining in an in-flight hop - set to BONG_HOP_SECONDS by
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
    /// set by `start_hop` from the landing spot `simulation::bong_hop_target`
    /// found. Only meaningful while `hop_timer > 0.0`.
    pub hop_end: Position,
    /// Seconds remaining before another hop can trigger - set to
    /// BONG_HOP_COOLDOWN_SECONDS by `start_hop`, so a rapid volley of hits
    /// can't chain-hop the bong every single frame one lands.
    pub hop_cooldown: f32,
    /// Seconds remaining showing the Attack clip after the most recent
    /// bite - set by `start_attack`, ticked down by `tick`.
    pub attack_timer: f32,
    /// Seconds remaining before the bong can bite again - set to
    /// BONG_ATTACK_COOLDOWN_SECONDS by `start_attack`, pacing damage output
    /// against a tank that lingers in range the same way a tank's own
    /// `fire_cooldown` paces its shots.
    pub attack_cooldown: f32,
    /// Which way this bong is drawn (see `Facing`). Set by `start_hop` from
    /// where the leap lands and by `start_attack` from where the victim is,
    /// and kept until one of those changes it. Purely a draw-time choice -
    /// nothing in `simulation/` reads it.
    pub facing: Facing,
    /// Seconds elapsed since `health` first reached zero - `None` while
    /// still alive. Set once by `damage` and never cleared; drives the
    /// Explosion sequence (see `anim`), which holds on its last frame
    /// forever rather than looping or disappearing, once
    /// BONG_EXPLOSION_FRAMES/BONG_EXPLOSION_FPS worth of it has played.
    pub death_elapsed: Option<f32>,
    /// Seconds left stunned by a sonic hammer's wave (docs/sonic-hammer.md):
    /// while positive the bong neither hops (`can_hop`) nor bites
    /// (`can_attack`). Set by `stun`, ticked down by `tick`.
    pub stun_timer: f32,
    /// In a gravity well's pull this tick (docs/gravity-well.md): pinned,
    /// it slides toward the core and does not hop.
    pub pulled: bool,
}

/// Which of the five filmstrips (see docs/BONG_SPEC.md) `anim` picked for
/// this frame, plus which frame within it. Also the clip half of
/// `canvas::Sheet::Bong`, hence the ordering derives.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum BongAnim {
    Idle,
    Hurt,
    Hop,
    Attack,
    Explosion,
}

impl Bong {
    /// Side length of this bong's sprite on screen, matching `Tank::size`/
    /// `Obstacle::size` - what sizes its ground ring (`draw_bong_ring`)
    /// exactly as a tank's sizes the tank's.
    pub fn size(&self) -> f32 {
        BONG_TEXTURE_SIZE * BONG_SCALE
    }

    pub fn is_dead(&self) -> bool {
        self.death_elapsed.is_some()
    }

    /// Remaining health as a fraction of `max_health`, 1 to 0 - what the
    /// bong's ring gauge reads.
    pub fn health_fraction(&self) -> f32 {
        if self.max_health > 0.0 { (self.health / self.max_health).clamp(0.0, 1.0) } else { 0.0 }
    }

    /// Remaining health in whole points, rounded to the nearest: 0 only
    /// once dead, at least 1 while alive, at most 255. What travels on the
    /// wire (`net::encode`) and what the picture compares.
    pub fn health_points(&self) -> u8 {
        if self.is_dead() {
            return 0;
        }
        self.health.round().clamp(1.0, 255.0) as u8
    }

    /// How far (px) a single hop covers - see BONG_HOP_DISTANCE_FACTOR's
    /// comment for why this is a factor of `size()` rather than a flat
    /// constant.
    pub fn hop_distance(&self) -> f32 {
        self.size() * tuning().bong_hop_distance_factor
    }

    /// How close (px, center to center) a tank has to get before the bong
    /// bites it - see BONG_HOP_DISTANCE_FACTOR's comment.
    pub fn attack_range(&self) -> f32 {
        self.size() * tuning().bong_attack_range_factor
    }

    /// How close (px, center to center) a tank has to get before the bong
    /// tries to hop away from it - see BONG_AVOID_RANGE_FACTOR's comment.
    pub fn avoid_range(&self) -> f32 {
        self.size() * tuning().bong_avoid_range_factor
    }

    /// Whether a hop can trigger right now - alive, and not still cooling
    /// down from the last one. Doesn't know or care whether a *landing
    /// spot* is actually available; that's `simulation::bong_hop_target`'s
    /// job, checked separately since it needs world/obstacle data this
    /// type deliberately has no access to.
    pub fn can_hop(&self) -> bool {
        !self.is_dead() && self.hop_cooldown <= 0.0 && !self.is_stunned() && !self.pulled
    }

    /// Stunned for `seconds`, or as long as it already was, if longer.
    pub fn stun(&mut self, seconds: f32) {
        if !self.is_dead() {
            self.stun_timer = self.stun_timer.max(seconds);
        }
    }

    /// Whether a stun still holds it.
    pub fn is_stunned(&self) -> bool {
        self.stun_timer > 0.0
    }

    /// Whether the bong can bite right now - alive, and not still cooling
    /// down from the last bite.
    pub fn can_attack(&self) -> bool {
        !self.is_dead() && self.attack_cooldown <= 0.0 && !self.is_stunned()
    }

    /// Apply shell damage. A no-op once already dead, so callers don't need
    /// to gate on `is_dead()` themselves (same convention as
    /// `Obstacle::damage`).
    pub fn damage(&mut self, amount: f32) {
        if self.is_dead() {
            return;
        }
        self.health = (self.health - amount).max(0.0);
        self.hurt_timer = BONG_HURT_SECONDS;
        if self.health <= 0.0 {
            self.death_elapsed = Some(0.0);
        }
    }

    /// Restore health, clamped at `max_health`. A no-op once dead, the same
    /// convention `damage` follows: a bong health pack
    /// (`pickup::PickupKind::BongHealth`) cannot revive one, and there is no
    /// case where it would matter - the player's bong dying ends the round
    /// on the same frame it happens.
    pub fn heal(&mut self, amount: f32) {
        if self.is_dead() {
            return;
        }
        self.health = (self.health + amount).min(self.max_health);
    }

    /// Alive and undamaged. This is the one state in which a bong health
    /// pack is left on the ground rather than collected, so it is the
    /// whole of that pickup's collect rule (`simulation::pickup_phase`): a
    /// *dead* bong is deliberately not "full", and a pack driven over
    /// after one dies is consumed for nothing like any other side with no
    /// bong to heal.
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
    /// found valid (see `simulation::bong_hop_target`) - `position` doesn't
    /// jump to `target` here, `tick` carries it there smoothly over the
    /// next BONG_HOP_SECONDS, in step with the Hop clip/cooldown this also
    /// starts.
    pub fn start_hop(&mut self, target: Position) {
        // Where the leap *lands*, not the direction it fled: the landing
        // search angles a hop up to a quarter turn off the away direction
        // (`combat::bong_hop_target`), and the sprite has to match what the
        // bong visibly does.
        self.face_toward(target.x);
        self.hop_start = self.position;
        self.hop_end = target;
        self.hop_timer = BONG_HOP_SECONDS;
        self.hop_cooldown = tuning().bong_hop_cooldown_seconds;
    }

    /// Register that the bong just bit a tank - starts the cosmetic Attack
    /// clip/cooldown. The caller (`Game::update`) is responsible for
    /// actually applying damage to the target; this only tracks the bong's
    /// own reaction/pacing state.
    pub fn start_attack(&mut self, victim: Position) {
        // The attack clip strikes toward the side it faces, so a
        // bite has to turn the bong or the strike misses the tank.
        self.face_toward(victim.x);
        self.attack_timer = BONG_ATTACK_SECONDS;
        self.attack_cooldown = tuning().bong_attack_cooldown_seconds;
    }

    /// Advance this bong's per-frame animation/cooldown timers, including
    /// carrying `position` along an in-flight hop (see `hop_timer`'s own
    /// comment). Called every frame regardless of round outcome (see
    /// `Game::update`'s "round is over" branch, which still ticks a
    /// wrecked Tank's fire/a burning Obstacle's char loop) so a fresh
    /// Explosion - or a hop already in flight when the round ends - keeps
    /// playing through the end-of-round restart countdown instead of
    /// freezing mid-animation. `Game::update` is responsible for copying
    /// the resulting `position` into this bong's physics body every frame
    /// this runs, the same way it reads tank positions back *from*
    /// physics - here the data flows the other way, since a hop is
    /// authored in game code, not by rapier's own integration.
    pub fn tick(&mut self, dt: f32) {
        self.hurt_timer = (self.hurt_timer - dt).max(0.0);
        self.stun_timer = (self.stun_timer - dt).max(0.0);
        if self.hop_timer > 0.0 {
            self.hop_timer = (self.hop_timer - dt).max(0.0);
            let frac = (1.0 - self.hop_timer / BONG_HOP_SECONDS).clamp(0.0, 1.0);
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
    /// `t` (only used for the looping Idle). Priority order: a death
    /// always wins; otherwise a fresh hop (it visually *is* the reaction to
    /// being shot, superseding the plain Hurt flicker) beats a fresh bite,
    /// which beats a fresh hit that didn't trigger a hop, which beats
    /// idling.
    pub fn anim(&self, t: f32) -> (BongAnim, i32) {
        if let Some(elapsed) = self.death_elapsed {
            let frame = (elapsed * BONG_EXPLOSION_FPS) as i32;
            return (BongAnim::Explosion, frame.min(BONG_EXPLOSION_FRAMES - 1));
        }
        if self.hop_timer > 0.0 {
            let elapsed = (BONG_HOP_SECONDS - self.hop_timer).max(0.0);
            let frame = (elapsed * BONG_HOP_FPS) as i32;
            return (BongAnim::Hop, frame.clamp(0, BONG_HOP_FRAMES - 1));
        }
        // Stunned, or pinned in a gravity well's pull: the hurt clip held
        // on its first frame.
        if self.is_stunned() || self.pulled {
            return (BongAnim::Hurt, 0);
        }
        if self.attack_timer > 0.0 {
            let elapsed = (BONG_ATTACK_SECONDS - self.attack_timer).max(0.0);
            let frame = (elapsed * BONG_ATTACK_FPS) as i32;
            return (BongAnim::Attack, frame.clamp(0, BONG_ATTACK_FRAMES - 1));
        }
        if self.hurt_timer > 0.0 {
            let elapsed = (BONG_HURT_SECONDS - self.hurt_timer).max(0.0);
            let frame = (elapsed * BONG_HURT_FPS) as i32;
            return (BongAnim::Hurt, frame.clamp(0, BONG_HURT_FRAMES - 1));
        }
        let frame = (t.max(0.0) * BONG_IDLE_FPS) as i32 % BONG_IDLE_FRAMES;
        (BongAnim::Idle, frame)
    }
}


/// The marks a stunned bong wears (docs/sonic-hammer.md): three pale blocks
/// with a shade under each, circling over its head on the clock `t`.
/// Empty while it is not stunned or is dead.
pub fn stun_marks(bong: &Bong, t: f32) -> Vec<crate::pyro::Shape> {
    let mut out = Vec::new();
    if !bong.is_stunned() || bong.is_dead() {
        return out;
    }
    let head = Position::new(bong.position.x, bong.position.y - bong.size() * 0.32);
    for i in 0..3 {
        let a = t * 6.0 + i as f32 * std::f32::consts::TAU / 3.0;
        let at = Position::new(head.x + math::cos(a) * 10.0, head.y + math::sin(a) * 3.0);
        out.push(crate::pyro::Shape::Mark { pos: Position::new(at.x, at.y + 2.0), size: 2, color: crate::sonic::STONE[1] });
        out.push(crate::pyro::Shape::Mark { pos: at, size: 2, color: crate::sonic::STONE[3] });
    }
    out
}

/// Draw the bong's side marker as its health gauge: the shared ground ring
/// (`tank::draw_ground_ring_at`, the player tank's own ring in the same
/// size class) under the sprite, its filled arc the remaining health - in
/// the white ramp for the player's bong, the all-red ramp for the enemy's -
/// and the rest of the circle the side's colour dimmed to
/// `health_ring_base_opacity`, so the ring stays a full marker whose side
/// reads at any health. On while the bong lives; gone once it is dead - the
/// explosion crater has no side. Call before `draw_bong`.
pub fn draw_bong_ring(c: &mut impl Canvas, bong: &Bong, time: f32) {
    if bong.is_dead() {
        return;
    }
    let ramp = match bong.side {
        Side::Player => HealthRamp::White,
        Side::Enemy => HealthRamp::Red,
    };
    let base = with_opacity(ramp.base(), tuning().player_ring_opacity * tuning().health_ring_base_opacity);
    let style = RingStyle::Gauge { frac: bong.health_fraction(), ramp, base };
    draw_ground_ring_at(c, bong.position, bong.size(), 0.0, time, style, 1.0);
}

/// Draw the bong: whichever of its variant's five clips `Bong::anim` picks
/// for this frame (`Sheet::Bong`), centered at its position. Never rotates (it's not a
/// tank), so this skips the rotation param `draw_tank` needs - same as
/// `draw_obstacle`. It does *mirror*: the art is authored facing right, and
/// `mirror` turns `Bong::facing` into the source rectangle's width sign
/// plus the destination offset that keeps the body over the same ground.
pub fn draw_bong(c: &mut impl Canvas, bong: &Bong, t: f32) {
    let (anim, frame) = bong.anim(t);
    let sheet = Sheet::Bong { kind: bong.kind, clip: anim };
    let (flip, offset) = mirror(bong.facing);
    let src = Rectangle::new(
        frame as f32 * BONG_TEXTURE_SIZE,
        0.0,
        BONG_TEXTURE_SIZE * flip,
        BONG_TEXTURE_SIZE,
    );
    let size = bong.size();
    let dest = Rectangle::new(bong.position.x + offset, bong.position.y, size, size);
    let origin = Vec2::new(size / 2.0, size / 2.0);
    c.blit(sheet, src, dest, origin, 0.0, Color::WHITE);
}

#[cfg(test)]
mod facing_tests {
    use super::*;

    fn bong_at(x: f32) -> Bong {
        Bong {
            side: Side::Player,
            position: Position::new(x, 300.0),
            health: 100.0,
            max_health: 100.0,
            kind: BongKind::Spike,
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
            stun_timer: 0.0,
            pulled: false,
        }
    }

    /// Where the bong's *body* lands on screen for a given facing: the
    /// drawn cell's left edge plus the body's own column, mirrored or not.
    /// The whole point of `mirror`'s offset is that this does not move.
    fn body_center_on_screen(bong: &Bong) -> f32 {
        let (flip, offset) = mirror(bong.facing);
        let col = if flip < 0.0 {
            (BONG_TEXTURE_SIZE - 1.0) - BONG_SPRITE_BODY_CENTER_X
        } else {
            BONG_SPRITE_BODY_CENTER_X
        };
        bong.position.x + offset + (col - cell_center_x()) * BONG_SCALE
    }

    #[test]
    fn a_hop_faces_the_way_it_lands() {
        let mut bong = bong_at(500.0);
        bong.start_hop(Position::new(400.0, 300.0));
        assert_eq!(bong.facing, Facing::Left, "a hop west faces west");

        let mut bong = bong_at(500.0);
        bong.facing = Facing::Left;
        bong.start_hop(Position::new(600.0, 300.0));
        assert_eq!(bong.facing, Facing::Right, "a hop east faces east");
    }

    /// The hop fan reaches a quarter turn either way, so a dead-vertical
    /// leap is a real case - and its sign is float noise, not a direction.
    #[test]
    fn a_vertical_hop_keeps_the_facing_it_had() {
        for facing in [Facing::Left, Facing::Right] {
            let mut bong = bong_at(500.0);
            bong.facing = facing;
            bong.start_hop(Position::new(500.0 + BONG_FACING_DEADBAND_PX * 0.5, 380.0));
            assert_eq!(bong.facing, facing, "a near-vertical hop turned the bong");
        }
    }

    #[test]
    fn a_bite_faces_the_tank_it_bites() {
        let mut bong = bong_at(500.0);
        bong.start_attack(Position::new(440.0, 300.0));
        assert_eq!(bong.facing, Facing::Left, "the strike has to reach the tank");
        assert!(bong.attack_timer > 0.0, "the clip still starts");

        bong.start_attack(Position::new(560.0, 300.0));
        assert_eq!(bong.facing, Facing::Right);
    }

    /// `Right` is the identity, so every existing bong draws exactly as it
    /// always has.
    #[test]
    fn facing_right_draws_unchanged() {
        assert_eq!(mirror(Facing::Right), (1.0, 0.0));
    }

    /// Turning around never slides the bong sideways: the body is centred on
    /// the cell (BONG_SPRITE_BODY_CENTER_X), so the flip alone keeps it on
    /// its ground and the offset is zero.
    #[test]
    fn mirroring_keeps_the_body_on_the_same_ground() {
        let (flip, offset) = mirror(Facing::Left);
        assert!(flip < 0.0, "Left is the mirrored draw");
        assert_eq!(offset, 0.0, "a centred body needs no offset");

        let mut right = bong_at(500.0);
        right.facing = Facing::Right;
        let mut left = bong_at(500.0);
        left.facing = Facing::Left;
        let (a, b) = (body_center_on_screen(&right), body_center_on_screen(&left));
        assert!((a - b).abs() < 0.001, "the body moved {} px when the bong turned around", (a - b).abs());
    }
}
