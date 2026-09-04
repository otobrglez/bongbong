use crate::tuning::tuning;
use sola_raylib::prelude::*;

use crate::tank::Tank;
use crate::{
    Position,
    SHELL_SCALE,
    SHELL_TEXTURE_SIZE,
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

/// Who fired a projectile: damage/kill attribution, same-side checks, and
/// shooter self-exclusion in the hit test. `Enemy(n)` is the nth enemy
/// spawned this round (`Tank::owner_slot - 1`, see `Tank::owner`).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Owner {
    Player,
    Enemy(usize),
}

impl Owner {
    /// The owner slot this maps to (`Tank::owner_slot`): 0 for the player,
    /// `n + 1` for `Enemy(n)`.
    pub fn slot(self) -> usize {
        match self {
            Owner::Player => 0,
            Owner::Enemy(n) => n + 1,
        }
    }

    /// True if both owners fight on the same side - every enemy counts as
    /// friendly to every other enemy, never to the player.
    pub fn same_side(self, other: Owner) -> bool {
        matches!((self, other), (Owner::Player, Owner::Player) | (Owner::Enemy(_), Owner::Enemy(_)))
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

    /// This shell's drop-shadow distance (px), rolled once at fire time from
    /// `SHELL_SHADOW_OFFSET_MIN..MAX` (see `Game::update`, right after
    /// `Shell::spawn`) and fixed for its whole flight - different shells land
    /// on different rolls, so they read as flying at different heights
    /// instead of every shot looking identical. `0.0` here (like `body:
    /// None` above) is a placeholder `spawn` itself never uses, since it has
    /// no `rng` to roll from; the real value is set right after construction.
    pub shadow_offset: f32,
    /// Where this shell was at the start of the current frame's physics
    /// steps - written by the simulation before it advances projectiles,
    /// never by `update`. `prev_position..position` is the segment the
    /// swept hit test checks, so a shell can't tunnel through a thin
    /// target however far one frame carries it.
    pub prev_position: Position,
    /// How many more times this shell may ricochet off an indestructible
    /// Iron obstacle before detonating on one instead. Battlefield walls
    /// and every other target detonate it on first contact regardless.
    pub bounces_left: u32,
    /// Obstacle tiles this projectile already rolled a pass-over on (a
    /// sandbag it sailed over) - skipped by every later hit sweep, since a
    /// segment ending inside a tile would otherwise re-roll it next frame.
    pub passed_over: Vec<hecs::Entity>,
}

impl Shell {
    /// Create a shell at the tank's muzzle, travelling in the direction the tank
    /// faces. `owner` tags which tank fired it for collision purposes.
    /// `aim_offset` (degrees) skews the travel direction off the barrel so a shot
    /// can miss; pass 0.0 for a clean shot straight ahead. `lateral_offset`
    /// (tile px, pre-scale) shifts the spawn point sideways from the tank's
    /// centerline - nonzero only for a twin-barrel chassis's two independent
    /// shells (see TANK_BARREL_LATERAL_OFFSET_BY_ROW), zero for every
    /// single-barrel shot, which fires from dead center. The travel
    /// direction itself is unaffected by this - both barrels of a twin
    /// chassis fire parallel, not converging/diverging.
    pub fn spawn(tank: &Tank, owner: Owner, aim_offset: f32, lateral_offset: f32) -> Shell {
        let rot = (tank.rotation + aim_offset).to_radians();
        // rotation 0 == facing up (-Y); +90 == right, etc. matches the tank movement.
        let dir = Vector2::new(rot.sin(), -rot.cos());
        // Start at the turret/barrel tip, not the tank's own center - see
        // TANK_MUZZLE_FORWARD_OFFSET_BY_ROW for how that distance was
        // measured per tank archetype from the sprite sheet's own published
        // bounding boxes. (Self-hits are prevented by the hit test skipping
        // the shooter's own boxes - see `simulation::hits::Terrain::sweep` -
        // not by this offset, so it's free to match the sprite art.)
        let muzzle = tuning().tank_muzzle_forward_offset[tank.row as usize] * tank.scale;
        // Perpendicular to `dir`, using the *tank's own* rotation (not
        // rotation + aim_offset) - a misfire's aim skew shouldn't also swing
        // which side of the hull a barrel sits on. Rotating (sin,-cos) by
        // +90 degrees gives (cos, sin): at rotation 0 (facing up) this is
        // (1, 0), i.e. screen +x - the tank's right-hand side - matching
        // TANK_BARREL_LATERAL_OFFSET_BY_ROW's "positive = right barrel"
        // convention.
        let hull_rot = tank.rotation.to_radians();
        let lateral = Vector2::new(hull_rot.cos(), hull_rot.sin()) * (lateral_offset * tank.scale);
        let position = Position::new(
            tank.position.x + dir.x * muzzle + lateral.x,
            tank.position.y + dir.y * muzzle + lateral.y,
        );
        Shell {
            state: ShellState::Fire0,
            position,
            velocity: Vector2::new(dir.x * tuning().shell_speed, dir.y * tuning().shell_speed),
            rotation: tank.rotation + aim_offset,
            timer: 0.0,
            done: false,
            owner,
            variant: tank.shell_variant,
            shooter_row: tank.row,
            shadow_offset: 0.0,
            prev_position: position,
            bounces_left: tuning().shell_ricochet_bounces,
            passed_over: Vec::new(),
        }
    }

    /// Advance the shell: move it while flying, and step through its timed states.
    /// Flying shells are detonated by the simulation's swept hit test
    /// (`simulation::hits::Terrain::sweep`), never here. Walls sit exactly
    /// at the screen edge (see `battlefield::wall_rects`), so a shell that
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
        shell.position.x + tuning().shadow_dir_x * shell.shadow_offset,
        shell.position.y + tuning().shadow_dir_y * shell.shadow_offset,
        size,
        size,
    );
    let origin = Vector2::new(size / 2.0, size / 2.0);
    let shadow = Color::new(0, 0, 0, (255.0 * tuning().shell_shadow_opacity) as u8);

    d.draw_texture_pro(texture, src, dest, origin, shell.rotation, shadow);
}
