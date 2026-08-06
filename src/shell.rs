use rapier2d::prelude::RigidBodyHandle;
use sola_raylib::prelude::*;

use crate::tank::Tank;
use crate::{Position, SHELL_SCALE, SHELL_SPEED, SHELL_TEXTURE_SIZE, TANK_TEXTURE_SIZE};

/// A shell's lifecycle. Each variant maps to a column in shells.png (see
/// SHELL_VARIANTS for the row dimension) and carries its own on-screen
/// duration in seconds (Flying lasts until it leaves the screen).
#[derive(Clone, Copy, PartialEq)]
pub enum ShellState {
    Fire0,  // col 0 - big muzzle blast as the shell exits the tank
    Fire1,  // col 1 - shell just left the barrel, small trailing flash
    Fire2,  // col 2 - flash finishing as the shell pulls away
    Flying, // col 3 - projectile in the air
    Hit0,   // col 4 - impact burst starting
    Hit1,   // col 5 - impact burst expanding
    Hit2,   // col 6 - impact burst dissipating (smoke + embers)
}

impl ShellState {
    /// Column of this state in the shells sprite sheet.
    fn col(self) -> i32 {
        match self {
            ShellState::Fire0 => 0,
            ShellState::Fire1 => 1,
            ShellState::Fire2 => 2,
            ShellState::Flying => 3,
            ShellState::Hit0 => 4,
            ShellState::Hit1 => 5,
            ShellState::Hit2 => 6,
        }
    }

    /// How long this state is shown (seconds). Flying is time-unbounded (moves
    /// until it hits a screen edge), so it has no fixed duration here.
    fn duration(self) -> f32 {
        match self {
            ShellState::Fire0 => 0.06,
            ShellState::Fire1 => 0.06,
            ShellState::Fire2 => 0.05,
            ShellState::Flying => f32::INFINITY,
            ShellState::Hit0 => 0.08,
            ShellState::Hit1 => 0.1,
            ShellState::Hit2 => 0.14,
        }
    }
}

/// Who fired a shell, so collision can charge the right side's damage and a
/// shell can never hit the very tank that fired it. `Enemy` carries that
/// enemy's index into `Game::enemies`, which is stable for the round (the
/// list is only rebuilt on restart).
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Owner {
    Player,
    Enemy(usize),
}

pub struct Shell {
    pub state: ShellState,
    pub position: Position,
    /// Direction of travel while flying (pixels per second).
    pub velocity: Vector2,
    /// Facing angle in degrees (matches the tank's rotation when fired).
    pub rotation: f32,
    /// Time elapsed in the current state.
    pub timer: f32,
    /// Set once the shell has finished its last state and can be removed.
    pub done: bool,
    /// Who fired this shell; see `Owner`.
    pub owner: Owner,
    /// Row in shells.png (0..SHELL_VARIANTS), copied from the firing tank's
    /// `shell_variant` at spawn. Fixed for this shell's whole lifetime, same
    /// as every other shell that tank fires.
    pub variant: i32,
    /// This shell's rapier sensor body, spawned alongside it (see
    /// `Game::update`/`physics::Physics::spawn_shell`) and kept in sync with
    /// `position` every frame so hit detection can query real intersections
    /// against a tank's `hit_sensor` instead of a hand-rolled point-in-square
    /// check. Removed from the physics world when the shell is dropped.
    pub body: Option<RigidBodyHandle>,
}

impl Shell {
    /// Create a shell at the tank's muzzle, travelling in the direction the tank
    /// faces. `owner` tags which tank fired it for collision purposes.
    /// `aim_offset` (degrees) skews the travel direction off the barrel so a shot
    /// can miss; pass 0.0 for a clean shot straight ahead.
    pub fn spawn(tank: &Tank, owner: Owner, aim_offset: f32) -> Shell {
        let rot = (tank.rotation + aim_offset).to_radians();
        // rotation 0 == facing up (-Y); +90 == right, etc. matches the tank movement.
        let dir = Vector2::new(rot.sin(), -rot.cos());
        // Start a little ahead of the tank center so the shell exits the barrel.
        // (Right on the tank's own hit-box boundary, in fact - see the owner
        // exclusion in Game::update, which is what keeps a tank from instantly
        // shooting itself.)
        let muzzle = TANK_TEXTURE_SIZE * tank.scale * 0.5;
        Shell {
            state: ShellState::Fire0,
            position: Position::new(
                tank.position.x + dir.x * muzzle,
                tank.position.y + dir.y * muzzle,
            ),
            velocity: Vector2::new(dir.x * SHELL_SPEED, dir.y * SHELL_SPEED),
            rotation: tank.rotation + aim_offset,
            timer: 0.0,
            done: false,
            owner,
            variant: tank.shell_variant,
            body: None,
        }
    }

    /// Advance the shell: move it while flying, and step through its timed states.
    pub fn update(&mut self, dt: f32, width: f32, height: f32) {
        self.timer += dt;

        if self.state == ShellState::Flying {
            self.position.x += self.velocity.x * dt;
            self.position.y += self.velocity.y * dt;

            // Impact when the shell leaves the screen -> start the bang.
            if self.position.x < 0.0
                || self.position.x > width
                || self.position.y < 0.0
                || self.position.y > height
            {
                self.detonate();
            }
            return;
        }

        // Non-flying states advance once their duration elapses.
        if self.timer >= self.state.duration() {
            self.timer = 0.0;
            self.state = match self.state {
                ShellState::Fire0 => ShellState::Fire1,
                ShellState::Fire1 => ShellState::Fire2,
                ShellState::Fire2 => ShellState::Flying,
                ShellState::Flying => ShellState::Flying, // handled above
                ShellState::Hit0 => ShellState::Hit1,
                ShellState::Hit1 => ShellState::Hit2,
                ShellState::Hit2 => {
                    self.done = true;
                    ShellState::Hit2
                }
            };
        }
    }

    /// Switch a flying shell into its impact (hit) animation at the current spot.
    pub fn detonate(&mut self) {
        self.state = ShellState::Hit0;
        self.timer = 0.0;
    }
}

/// Source rectangle for a shell frame (variant row, state column) in shells.png.
fn source_rec(variant: i32, col: i32) -> Rectangle {
    Rectangle::new(
        col as f32 * SHELL_TEXTURE_SIZE,
        variant as f32 * SHELL_TEXTURE_SIZE,
        SHELL_TEXTURE_SIZE,
        SHELL_TEXTURE_SIZE,
    )
}

/// Draw a shell using its current state's frame (from its variant row),
/// centered and rotated to face travel.
pub fn draw_shell(d: &mut impl RaylibDraw, texture: &Texture2D, shell: &Shell) {
    let src = source_rec(shell.variant, shell.state.col());
    let size = SHELL_TEXTURE_SIZE * SHELL_SCALE;

    let dest = Rectangle::new(shell.position.x, shell.position.y, size, size);
    let origin = Vector2::new(size / 2.0, size / 2.0);

    d.draw_texture_pro(texture, src, dest, origin, shell.rotation, Color::WHITE);
}
