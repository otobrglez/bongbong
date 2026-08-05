use sola_raylib::prelude::*;

use crate::{
    MAX_DAMAGE, MAX_SHELLS, Position, TANK_ACCEL, TANK_FRICTION, TANK_MAX_REVERSE,
    TANK_MAX_SPEED, TANK_REVERSE_ACCEL, TANK_TEXTURE_SIZE, WRECK_BURN_SECONDS,
};

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
    /// Signed speed along the hull's heading (px/s): positive drives forward
    /// in the facing direction, negative reverses. Carries momentum.
    pub velocity: f32,
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
}

impl Default for Tank {
    fn default() -> Self {
        Self {
            row: 0,
            col: 0,
            position: Position::default(),
            rotation: 0.0,
            scale: 2.0, // 3.0,
            velocity: 0.0,
            damage: 0.0,
            shells_ammo: MAX_SHELLS,
            recharge_timer: 0.0,
            ram_cooldown: 0.0,
            wreck_timer: 0.0,
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

    /// True if this tank's square footprint overlaps another's.
    pub fn overlaps(&self, other: &Tank) -> bool {
        let half = (self.size() + other.size()) * 0.5;
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

    /// Unit vector pointing along the hull's heading (rotation 0 == up / -Y).
    pub fn heading(&self) -> Vector2 {
        let rot = self.rotation.to_radians();
        Vector2::new(rot.sin(), -rot.cos())
    }

    /// Rotate the hull in place by `deg` degrees, keeping rotation in [0, 360).
    pub fn rotate(&mut self, deg: f32) {
        self.rotation = (self.rotation + deg).rem_euclid(360.0);
    }

    /// Apply throttle/coasting to the signed velocity, given the driver's intent:
    /// `throttle` is +1 (forward), -1 (reverse), or 0 (coast). Real tanks build up
    /// and shed speed gradually, so we accelerate toward the throttle direction and
    /// let friction bleed velocity back to zero when coasting.
    pub fn drive(&mut self, throttle: f32, dt: f32) {
        if throttle > 0.0 {
            self.velocity = (self.velocity + TANK_ACCEL * dt).min(TANK_MAX_SPEED);
        } else if throttle < 0.0 {
            self.velocity = (self.velocity - TANK_REVERSE_ACCEL * dt).max(-TANK_MAX_REVERSE);
        } else {
            // Coast: friction pulls the speed toward zero without overshooting.
            let drop = TANK_FRICTION * dt;
            if self.velocity > 0.0 {
                self.velocity = (self.velocity - drop).max(0.0);
            } else if self.velocity < 0.0 {
                self.velocity = (self.velocity + drop).min(0.0);
            }
        }
    }

    /// Advance the tank along its heading by the current velocity for `dt` seconds.
    pub fn advance(&mut self, dt: f32) {
        let dir = self.heading();
        self.position.x += dir.x * self.velocity * dt;
        self.position.y += dir.y * self.velocity * dt;
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
