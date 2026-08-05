use sola_raylib::prelude::*;

use crate::{
    MAX_DAMAGE, MAX_SHELLS, Position, TANK_HULL_FRACTION, TANK_SPEED, TANK_TEXTURE_SIZE,
    WRECK_BURN_SECONDS,
};

/// The four movement/facing directions. rotation 0 == up, clockwise positive,
/// matching the sprite orientation and shell-spawn math.
#[derive(Clone, Copy, PartialEq)]
pub enum Dir {
    Up,
    Down,
    Left,
    Right,
}

impl Dir {
    /// Hull rotation in degrees for this direction.
    pub fn rotation(self) -> f32 {
        match self {
            Dir::Up => 0.0,
            Dir::Right => 90.0,
            Dir::Down => 180.0,
            Dir::Left => 270.0,
        }
    }

    /// Unit movement vector (screen space: +x right, +y down).
    pub fn vec(self) -> Vector2 {
        match self {
            Dir::Up => Vector2::new(0.0, -1.0),
            Dir::Down => Vector2::new(0.0, 1.0),
            Dir::Left => Vector2::new(-1.0, 0.0),
            Dir::Right => Vector2::new(1.0, 0.0),
        }
    }

    /// Cardinal direction from `from` toward `to`, choosing the dominant axis.
    pub fn toward(from: Position, to: Position) -> Dir {
        let dx = to.x - from.x;
        let dy = to.y - from.y;
        if dx.abs() >= dy.abs() {
            if dx >= 0.0 { Dir::Right } else { Dir::Left }
        } else if dy >= 0.0 {
            Dir::Down
        } else {
            Dir::Up
        }
    }
}

pub struct Tank {
    /// Which sprite in the atlas to draw.
    pub row: i32,
    pub col: i32,
    /// Center position on screen (pixels).
    pub position: Position,
    /// Facing angle in degrees.
    pub rotation: f32,
    /// How much to scale the 32x32 sprite when drawn.
    pub scale: f32,
    /// Movement speed in pixels per second (player and enemies differ).
    pub speed: f32,
    /// Accumulated damage, 0 (pristine) .. MAX_DAMAGE (destroyed wreck).
    pub damage: f32,
    /// Remaining shells this tank can fire before it must recharge.
    pub shells_ammo: i32,
    /// Seconds accumulated toward recharging the next shell.
    pub recharge_timer: f32,
    /// Seconds remaining before this tank can take ramming damage again.
    pub ram_cooldown: f32,
    /// Seconds spent as a wreck. Once it exceeds WRECK_BURN_SECONDS the fire
    /// dies out and the tank becomes a static charred "dead" hulk.
    pub wreck_timer: f32,
    /// Distance travelled (pixels) since the last track mark was dropped.
    pub track_accum: f32,
    /// Intended velocity this frame (pixels per second): the movement direction
    /// times speed, or zero when not moving. Set by `control` and read by the AI's
    /// predictive collision avoidance.
    pub velocity: Vector2,
}

impl Default for Tank {
    fn default() -> Self {
        Self {
            row: 0,
            col: 0,
            position: Position::default(),
            rotation: 0.0,
            scale: 2.0, // 3.0,
            speed: TANK_SPEED,
            damage: 0.0,
            shells_ammo: MAX_SHELLS,
            recharge_timer: 0.0,
            ram_cooldown: 0.0,
            wreck_timer: 0.0,
            track_accum: 0.0,
            velocity: Vector2::new(0.0, 0.0),
        }
    }
}

impl Tank {
    /// Side length of this tank on screen (square sprite).
    pub fn size(&self) -> f32 {
        TANK_TEXTURE_SIZE * self.scale
    }

    /// True once the tank has taken maximum damage (a burning wreck).
    pub fn is_wreck(&self) -> bool {
        self.damage >= MAX_DAMAGE
    }

    /// True once a wreck has finished burning and settled into a dead hulk.
    pub fn is_dead(&self) -> bool {
        self.is_wreck() && self.wreck_timer >= WRECK_BURN_SECONDS
    }

    /// Small phase offset (seconds) derived from screen position so that several
    /// burning tanks don't animate their smoke/fire in perfect lockstep.
    pub fn anim_phase(&self) -> f32 {
        (self.position.x + self.position.y) * 0.01
    }

    /// True if `point` lies within the tank's square footprint.
    pub fn contains(&self, point: Position) -> bool {
        let half = self.size() * 0.5;
        (point.x - self.position.x).abs() <= half && (point.y - self.position.y).abs() <= half
    }

    /// Collision footprint side length: the visible hull, not the full sprite
    /// tile, so tanks can close the gap left by the sprite's transparent padding.
    pub fn hull_size(&self) -> f32 {
        self.size() * TANK_HULL_FRACTION
    }

    /// True if this tank's hull footprint overlaps another's.
    pub fn overlaps(&self, other: &Tank) -> bool {
        let half = (self.hull_size() + other.hull_size()) * 0.5;
        (self.position.x - other.position.x).abs() < half
            && (self.position.y - other.position.y).abs() < half
    }

    /// Recharge ammo over time toward MAX_SHELLS, one shell per interval.
    pub fn tick_recharge(&mut self, dt: f32) {
        if self.shells_ammo >= MAX_SHELLS {
            self.recharge_timer = 0.0;
            return;
        }
        self.recharge_timer += dt;
        while self.recharge_timer >= crate::SHELL_RECHARGE_SECONDS && self.shells_ammo < MAX_SHELLS {
            self.recharge_timer -= crate::SHELL_RECHARGE_SECONDS;
            self.shells_ammo += 1;
        }
    }

    /// Age a wreck so its fire burns for WRECK_BURN_SECONDS before going out. The
    /// timer only runs once the tank is a wreck; a live tank keeps it at zero.
    pub fn tick_wreck(&mut self, dt: f32) {
        if self.is_wreck() {
            // Cap the timer so it doesn't grow unbounded once the fire is out.
            self.wreck_timer = (self.wreck_timer + dt).min(WRECK_BURN_SECONDS);
        }
    }

    /// Drive the tank for one frame. `move_dir` faces the hull that way and steps
    /// at `speed` (classic 4-direction, no momentum). `face` turns the hull in
    /// place without moving (used when an AI stops to aim). `move_dir` takes
    /// precedence. Shared by the player and the AI so both move identically.
    pub fn control(&mut self, move_dir: Option<Dir>, face: Option<Dir>, dt: f32) {
        if let Some(dir) = move_dir {
            self.rotation = dir.rotation();
            let step = dir.vec();
            self.velocity = Vector2::new(step.x * self.speed, step.y * self.speed);
            self.position.x += step.x * self.speed * dt;
            self.position.y += step.y * self.speed * dt;
        } else {
            self.velocity = Vector2::new(0.0, 0.0);
            if let Some(dir) = face {
                self.rotation = dir.rotation();
            }
        }
    }

    /// Keep the tank inside the battlefield (the `width` x `height` screen) by
    /// clamping its center so the visible hull never crosses an edge.
    pub fn clamp_to_field(&mut self, width: f32, height: f32) {
        let half = self.hull_size() * 0.5;
        self.position.x = self.position.x.clamp(half, width - half);
        self.position.y = self.position.y.clamp(half, height - half);
    }
}

/// Source rectangle for the tank at (row, col) inside the atlas.
fn source_rec(row: i32, col: i32) -> Rectangle {
    Rectangle::new(
        col as f32 * TANK_TEXTURE_SIZE,
        row as f32 * TANK_TEXTURE_SIZE,
        TANK_TEXTURE_SIZE,
        TANK_TEXTURE_SIZE,
    )
}

/// Draw a single tank sprite from the atlas at its center position, scaled and rotated.
pub fn draw_tank(d: &mut impl RaylibDraw, texture: &Texture2D, tank: &Tank) {
    let src = source_rec(tank.row, tank.col);
    let size = tank.size();

    // dest is placed at the tank's position; origin is half the size so the
    // sprite is centered on `position` and rotates around its own middle.
    let dest = Rectangle::new(tank.position.x, tank.position.y, size, size);
    let origin = Vector2::new(size / 2.0, size / 2.0);

    d.draw_texture_pro(texture, src, dest, origin, tank.rotation, Color::WHITE);
}
