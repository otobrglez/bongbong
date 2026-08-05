use sola_raylib::prelude::*;

use crate::tank::Tank;
use crate::{Position, SHELL_SCALE, SHELL_SPEED, SHELL_TEXTURE_SIZE, TANK_TEXTURE_SIZE};

/// A shell's lifecycle. Each variant maps to a column in shells.png and carries
/// its own on-screen duration in seconds (Flying lasts until it leaves the screen).
#[derive(Clone, Copy, PartialEq)]
pub enum ShellState {
    Explosion, // col 0 - muzzle blast as the shell exits the tank
    Fired,     // col 1 - shell just left the barrel
    Flying,    // col 2 - projectile in the air
    Bang0,     // col 3 - impact burst
    Bang1,     // col 4 - impact dissipating
}

impl ShellState {
    /// Column of this state in the shells sprite sheet.
    fn col(self) -> i32 {
        match self {
            ShellState::Explosion => 0,
            ShellState::Fired => 1,
            ShellState::Flying => 2,
            ShellState::Bang0 => 3,
            ShellState::Bang1 => 4,
        }
    }

    /// How long this state is shown (seconds). Flying is time-unbounded (moves
    /// until it hits a screen edge), so it has no fixed duration here.
    fn duration(self) -> f32 {
        match self {
            ShellState::Explosion => 0.06,
            ShellState::Fired => 0.06,
            ShellState::Flying => f32::INFINITY,
            ShellState::Bang0 => 0.08,
            ShellState::Bang1 => 0.12,
        }
    }
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
    /// True if the player fired this shell (hits enemies); false for enemy fire
    /// (hits the player). Lets one collision routine serve both sides.
    pub from_player: bool,
}

impl Shell {
    /// Create a shell at the tank's muzzle, travelling in the direction the tank
    /// faces. `from_player` tags which side fired it for collision purposes.
    /// `aim_offset` (degrees) skews the travel direction off the barrel so a shot
    /// can miss; pass 0.0 for a clean shot straight ahead.
    pub fn spawn(tank: &Tank, from_player: bool, aim_offset: f32) -> Shell {
        let rot = (tank.rotation + aim_offset).to_radians();
        // rotation 0 == facing up (-Y); +90 == right, etc. matches the tank movement.
        let dir = Vector2::new(rot.sin(), -rot.cos());
        // Start a little ahead of the tank center so the shell exits the barrel.
        let muzzle = TANK_TEXTURE_SIZE * tank.scale * 0.5;
        Shell {
            state: ShellState::Explosion,
            position: Position::new(
                tank.position.x + dir.x * muzzle,
                tank.position.y + dir.y * muzzle,
            ),
            velocity: Vector2::new(dir.x * SHELL_SPEED, dir.y * SHELL_SPEED),
            rotation: tank.rotation + aim_offset,
            timer: 0.0,
            done: false,
            from_player,
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
                ShellState::Explosion => ShellState::Fired,
                ShellState::Fired => ShellState::Flying,
                ShellState::Flying => ShellState::Flying, // handled above
                ShellState::Bang0 => ShellState::Bang1,
                ShellState::Bang1 => {
                    self.done = true;
                    ShellState::Bang1
                }
            };
        }
    }

    /// Switch a flying shell into its impact (bang) animation at the current spot.
    pub fn detonate(&mut self) {
        self.state = ShellState::Bang0;
        self.timer = 0.0;
    }
}

/// Source rectangle for a shell frame (indexed by column) in shells.png.
fn source_rec(col: i32) -> Rectangle {
    Rectangle::new(
        col as f32 * SHELL_TEXTURE_SIZE,
        0.0,
        SHELL_TEXTURE_SIZE,
        SHELL_TEXTURE_SIZE,
    )
}

/// Draw a shell using its current state's frame, centered and rotated to face travel.
pub fn draw_shell(d: &mut impl RaylibDraw, texture: &Texture2D, shell: &Shell) {
    let src = source_rec(shell.state.col());
    let size = SHELL_TEXTURE_SIZE * SHELL_SCALE;

    let dest = Rectangle::new(shell.position.x, shell.position.y, size, size);
    let origin = Vector2::new(size / 2.0, size / 2.0);

    d.draw_texture_pro(texture, src, dest, origin, shell.rotation, Color::WHITE);
}
