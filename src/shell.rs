use rapier2d::prelude::RigidBodyHandle;
use sola_raylib::prelude::*;

use crate::tank::Tank;
use crate::{
    Position, SHADOW_DIR_X, SHADOW_DIR_Y, SHELL_SCALE, SHELL_SHADOW_OPACITY, SHELL_SPEED,
    SHELL_TEXTURE_SIZE, TANK_MUZZLE_FORWARD_OFFSET_BY_ROW,
};

/// A shell's lifecycle. Each variant maps to a column in shells.png (see
/// SHELL_VARIANTS for the row dimension) and carries its own on-screen
/// duration in seconds (Flying lasts until it hits a tank, obstacle, or
/// battlefield wall - see `Shell::update`).
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

    /// How long this state is shown (seconds). Flying is time-unbounded
    /// (moves until it physically hits something - see `Shell::update`), so
    /// it has no fixed duration here.
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

/// Who fired a shell, so collision can charge the right side's damage.
/// (Physics collision groups - see `physics::owner_group` - are what
/// actually stop a shell from hitting the tank that fired it; `Owner` here
/// is purely for damage/kill attribution.) `Enemy` carries that enemy's
/// index into `Game::enemies`, which is stable for the round (the list is
/// only rebuilt on restart) - the same index used to derive that tank's
/// collision-group slot (`Game::enemy_owner_slot`).
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
    /// The firing tank's `row` (0..TANK_VARIANTS), copied at spawn - by the
    /// time this shell resolves a hit, the shooter tank itself is out of
    /// scope, so the shell carries its own shooter's chassis identity
    /// forward. Used to scale damage by chassis class (see
    /// TANK_CHASSIS_DAMAGE_FACTOR_BY_ROW and its use in `Game::update`'s
    /// hit-resolution).
    pub shooter_row: i32,
    /// This shell's rapier sensor body, spawned alongside it (see
    /// `Game::update`/`physics::Physics::spawn_shell`) and kept in sync with
    /// `position` every frame so hit detection can query real intersections
    /// against a tank's `hit_sensor` instead of a hand-rolled point-in-square
    /// check. Removed from the physics world when the shell is dropped.
    pub body: Option<RigidBodyHandle>,
    /// This shell's drop-shadow distance (px), rolled once at fire time from
    /// `SHELL_SHADOW_OFFSET_MIN..MAX` (see `Game::update`, right after
    /// `Shell::spawn`) and fixed for its whole flight - different shells land
    /// on different rolls, so they read as flying at different heights
    /// instead of every shot looking identical. `0.0` here (like `body:
    /// None` above) is a placeholder `spawn` itself never uses, since it has
    /// no `rng` to roll from; the real value is set right after construction.
    pub shadow_offset: f32,
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
        // Start at the turret/barrel tip, not the tank's own center - see
        // TANK_MUZZLE_FORWARD_OFFSET_BY_ROW for how that distance was
        // measured per tank archetype from the sprite sheet's own published
        // bounding boxes. (Self-hits are prevented separately, by physics
        // collision groups excluding the shooter's own hit sensor - see
        // `physics::Physics::spawn_shell` - not by this offset, so this is
        // free to match the actual sprite art rather than needing to clear
        // the tank's hit sensor.)
        let muzzle = TANK_MUZZLE_FORWARD_OFFSET_BY_ROW[tank.row as usize] * tank.scale;
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
            shooter_row: tank.row,
            body: None,
            shadow_offset: 0.0,
        }
    }

    /// Advance the shell: move it while flying, and step through its timed states.
    /// Flying shells detonate via a real physics hit against a tank,
    /// obstacle, or battlefield wall (see `simulation::find_shell_target`) -
    /// this only moves the shell, it never detonates it. Walls sit exactly
    /// at the screen edge (see `battlefield::spawn_walls`), so a shell that
    /// would otherwise fly off the battlefield always hits one first.
    pub fn update(&mut self, dt: f32) {
        self.timer += dt;

        if self.state == ShellState::Flying {
            self.position.x += self.velocity.x * dt;
            self.position.y += self.velocity.y * dt;
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

/// Draw this shell's drop shadow: same sprite/rotation, offset further than a
/// tank's shadow so the gap between shell and shadow reads as height - see
/// docs/sprite-shadows-design.md. The offset itself is `shell.shadow_offset`
/// (rolled once per shell at fire time, see the field doc on `Shell`), not a
/// flat constant, so different shells appear to fly at different heights.
/// Caller (`Game::render`) only calls this while `shell.state ==
/// ShellState::Flying`; the fire/impact frames are stationary blast sprites,
/// not airborne objects, so they get no shadow.
pub fn draw_shell_shadow(d: &mut impl RaylibDraw, texture: &Texture2D, shell: &Shell) {
    let src = source_rec(shell.variant, shell.state.col());
    let size = SHELL_TEXTURE_SIZE * SHELL_SCALE;

    let dest = Rectangle::new(
        shell.position.x + SHADOW_DIR_X * shell.shadow_offset,
        shell.position.y + SHADOW_DIR_Y * shell.shadow_offset,
        size,
        size,
    );
    let origin = Vector2::new(size / 2.0, size / 2.0);
    let shadow = Color::new(0, 0, 0, (255.0 * SHELL_SHADOW_OPACITY) as u8);

    d.draw_texture_pro(texture, src, dest, origin, shell.rotation, shadow);
}
