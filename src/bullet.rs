//! Minigun bullets: individually-simulated physical projectiles fired in
//! rapid bursts by a tank holding `pickup::PickupKind::Minigun` ammo (see
//! `tank::Tank::minigun_ammo`/`minigun_burst`, `simulation.rs`'s
//! `fire_bullet`/burst-ticking). Mirrors `shell::Shell`'s shape (position,
//! velocity, rotation, owner, state, timer, shadow_offset, physics body) -
//! same "individually-simulated moving projectile" concept, just a
//! different weapon and a much shorter state machine (see `BulletState`):
//! bullets fire far too rapidly in succession (MINIGUN_BULLET_DELAY_SECONDS)
//! for an elaborate multi-frame wind-up/impact sequence to read well at
//! burst cadence. No `variant`/chassis-colour row unlike `Shell` - every
//! bullet shares one small piece of art regardless of shooter chassis (see
//! tools/spritegen/gen_bullets.py). No `bounces_left` either - bullets never
//! ricochet (see `simulation.rs`'s bullet hit-resolution, which skips the
//! shell loop's ricochet branch entirely).

use crate::tuning::tuning;
use sola_raylib::prelude::*;

use crate::shell::Owner;
use crate::tank::Tank;
use crate::{
    MINIGUN_BULLET_SCALE,
    MINIGUN_BULLET_TEXTURE_SIZE,
    Position,
};

/// A minigun bullet's lifecycle - deliberately compact next to `ShellState`'s
/// seven columns: muzzle -> flying -> impact and nothing else.
#[derive(Clone, Copy, PartialEq)]
pub enum BulletState {
    Muzzle, // col 0 - tiny spark as the bullet clears the barrel
    Flying, // col 1 - tracer in the air
    Hit,    // col 2 - small impact spark
}

impl BulletState {
    /// Column of this state in minigun_bullets.png.
    fn col(self) -> i32 {
        match self {
            BulletState::Muzzle => 0,
            BulletState::Flying => 1,
            BulletState::Hit => 2,
        }
    }

    /// How long this state is shown (seconds). Flying is time-unbounded
    /// (moves until it physically hits something), same convention as
    /// `ShellState::duration`.
    fn duration(self) -> f32 {
        match self {
            BulletState::Muzzle => 0.025,
            BulletState::Flying => f32::INFINITY,
            BulletState::Hit => 0.08,
        }
    }
}

/// One minigun round - see this module's doc comment for how it compares to
/// `shell::Shell`.
pub struct Bullet {
    pub state: BulletState,
    pub position: Position,
    /// Direction of travel while flying (pixels per second).
    pub velocity: Vector2,
    /// Facing angle in degrees (matches the tank's rotation when fired, plus
    /// this bullet's own misfire/spread skew).
    pub rotation: f32,
    /// Time elapsed in the current state.
    pub timer: f32,
    /// Set once the bullet has finished its last state and can be removed.
    pub done: bool,
    /// Who fired this bullet; see `shell::Owner`.
    pub owner: Owner,
    /// The firing tank's `row` (0..TANK_VARIANTS), copied at spawn - used to
    /// scale damage by chassis class (TANK_CHASSIS_DAMAGE_FACTOR_BY_ROW),
    /// same as `Shell::shooter_row`.
    pub shooter_row: i32,
    /// This bullet's drop-shadow distance (px), rolled once at fire time -
    /// same role as `Shell::shadow_offset`.
    pub shadow_offset: f32,
    /// Same role as `Shell::prev_position` - the start of this frame's
    /// swept hit segment, written by the simulation.
    pub prev_position: Position,
    /// Obstacle tiles this projectile already rolled a pass-over on (a
    /// sandbag it sailed over) - skipped by every later hit sweep, since a
    /// segment ending inside a tile would otherwise re-roll it next frame.
    pub passed_over: Vec<hecs::Entity>,
}

impl Bullet {
    /// Create a bullet at the tank's muzzle, travelling in the direction the
    /// tank faces. Same muzzle math as `Shell::spawn`, minus a
    /// `lateral_offset` param - every bullet fires dead-center regardless of
    /// the mount overlay's three drawn barrels, since that overlay is purely
    /// cosmetic (cycling actual spawn points among three barrels would add
    /// real complexity for no gameplay payoff). `aim_offset` bundles this
    /// bullet's shared burst misfire skew plus its own fresh spread jitter -
    /// see `simulation::fire_bullet`, which computes that sum before calling
    /// this.
    pub fn spawn(tank: &Tank, owner: Owner, aim_offset: f32) -> Bullet {
        let rot = (tank.rotation + aim_offset).to_radians();
        let dir = Vector2::new(rot.sin(), -rot.cos());
        let muzzle = tuning().tank_muzzle_forward_offset[tank.row as usize] * tank.scale;
        let position = Position::new(
            tank.position.x + dir.x * muzzle,
            tank.position.y + dir.y * muzzle,
        );
        Bullet {
            state: BulletState::Muzzle,
            position,
            velocity: Vector2::new(dir.x * tuning().minigun_bullet_speed, dir.y * tuning().minigun_bullet_speed),
            rotation: tank.rotation + aim_offset,
            timer: 0.0,
            done: false,
            owner,
            shooter_row: tank.row,
            shadow_offset: 0.0,
            prev_position: position,
            passed_over: Vec::new(),
        }
    }

    /// Advance the bullet: move it while flying, and step through its timed
    /// states. Mirrors `Shell::update` exactly, just over the 3-state
    /// machine above.
    pub fn update(&mut self, dt: f32) {
        self.timer += dt;

        if self.state == BulletState::Flying {
            self.position.x += self.velocity.x * dt;
            self.position.y += self.velocity.y * dt;
            return;
        }

        if self.timer >= self.state.duration() {
            self.timer = 0.0;
            self.state = match self.state {
                BulletState::Muzzle => BulletState::Flying,
                BulletState::Flying => BulletState::Flying, // handled above
                BulletState::Hit => {
                    self.done = true;
                    BulletState::Hit
                }
            };
        }
    }

    /// Switch a flying bullet into its impact (hit) animation at the current spot.
    pub fn detonate(&mut self) {
        self.state = BulletState::Hit;
        self.timer = 0.0;
    }
}

/// Source rectangle for a bullet frame (state column) in minigun_bullets.png.
fn source_rec(col: i32) -> Rectangle {
    Rectangle::new(
        col as f32 * MINIGUN_BULLET_TEXTURE_SIZE,
        0.0,
        MINIGUN_BULLET_TEXTURE_SIZE,
        MINIGUN_BULLET_TEXTURE_SIZE,
    )
}

/// Draw a bullet using its current state's frame, centered and rotated to
/// face travel.
pub fn draw_bullet(d: &mut impl RaylibDraw, texture: &Texture2D, bullet: &Bullet) {
    let src = source_rec(bullet.state.col());
    let size = MINIGUN_BULLET_TEXTURE_SIZE * MINIGUN_BULLET_SCALE;

    let dest = Rectangle::new(bullet.position.x, bullet.position.y, size, size);
    let origin = Vector2::new(size / 2.0, size / 2.0);

    d.draw_texture_pro(texture, src, dest, origin, bullet.rotation, Color::WHITE);
}

/// Draw this bullet's drop shadow - same convention as `draw_shell_shadow`.
/// Caller (`Game::render`) only calls this while `bullet.state ==
/// BulletState::Flying`.
pub fn draw_bullet_shadow(d: &mut impl RaylibDraw, texture: &Texture2D, bullet: &Bullet) {
    let src = source_rec(bullet.state.col());
    let size = MINIGUN_BULLET_TEXTURE_SIZE * MINIGUN_BULLET_SCALE;

    let dest = Rectangle::new(
        bullet.position.x + tuning().shadow_dir_x * bullet.shadow_offset,
        bullet.position.y + tuning().shadow_dir_y * bullet.shadow_offset,
        size,
        size,
    );
    let origin = Vector2::new(size / 2.0, size / 2.0);
    let shadow = Color::new(0, 0, 0, (255.0 * tuning().minigun_bullet_shadow_opacity) as u8);

    d.draw_texture_pro(texture, src, dest, origin, bullet.rotation, shadow);
}
