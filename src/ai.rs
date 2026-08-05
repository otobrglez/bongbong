use rand::RngExt;
use rand::rngs::ThreadRng;
use sola_raylib::prelude::Vector2;

use crate::bt::{Node, Status, action, condition, selector, sequence};
use crate::tank::{Dir, Tank};
use crate::{
    AI_DIR_HOLD_SECONDS, AI_DIR_SWITCH_MARGIN_PX, AVOID_DODGE_SECONDS, AVOID_LOOKAHEAD,
    AVOID_MARGIN, AVOID_MIN_SPEED, ENEMY_AIM_SETTLE, ENEMY_ATTACK_RANGE, ENEMY_FIRE_ALIGN_PX,
    ENEMY_FIRE_INTERVAL, ENEMY_FLEE_DAMAGE, ENEMY_MISFIRE_ANGLE_MAX, ENEMY_MISFIRE_ANGLE_MIN,
    ENEMY_MISFIRE_CHANCE_MAX, ENEMY_MISFIRE_RANGE, ENEMY_RETARGET_SECONDS, ENEMY_VIEW_RANGE,
    Position,
};

/// A read-only snapshot of one tank's motion for collision prediction. The game
/// builds a slice of these (all live tanks: player + enemies) each frame and hands
/// it to every enemy's `think`, so an enemy can predict closest approach to the
/// others without borrowing the mutable tank list.
#[derive(Clone, Copy)]
pub struct Mover {
    pub position: Position,
    pub velocity: Vector2,
    /// Collision radius (half the hull footprint).
    pub radius: f32,
}

/// What a driver (player or AI) wants to do this frame. The physics layer turns
/// this into a facing/step + firing, so player input and AI decisions flow
/// through the exact same code path. Movement is 4-direction only.
#[derive(Default, Clone, Copy)]
pub struct Intent {
    /// Direction to face and move this frame, or None to stay put.
    pub move_dir: Option<Dir>,
    /// Direction to face without moving (e.g. while aiming). Ignored if move_dir
    /// is set. None leaves the hull as-is.
    pub face: Option<Dir>,
    /// True on the frame the tank wants to fire a shell.
    pub fire: bool,
    /// Extra angle (degrees) to add to the shell's heading when firing, so a shot
    /// can be thrown off-aim. Zero means fire straight down the barrel. Used by the
    /// enemy AI to model point-blank misfires.
    pub fire_aim_offset: f32,
}

/// Persistent per-enemy memory that survives across frames. Kept separate from
/// the transient perception so the behavior tree can read state and remember
/// decisions (committed heading, timers) between ticks.
pub struct Ai {
    /// Roaming target used while patrolling.
    waypoint: Position,
    /// Seconds until we pick a fresh patrol waypoint (avoids per-frame jitter).
    retarget_timer: f32,
    /// Seconds until this tank may fire again.
    fire_timer: f32,
    /// The cardinal heading the tank has committed to, and how long it's been
    /// held. Direction commitment (hold time + switch margin) is what kills the
    /// frame-to-frame jitter near 45-degree diagonals.
    committed_dir: Option<Dir>,
    dir_hold: f32,
    /// How long the tank has been lined up on the player's firing axis. It must
    /// stay aligned for ENEMY_AIM_SETTLE before it actually shoots.
    aim_settle: f32,
    /// While sliding along a wall, the direction chosen to follow it. Held until
    /// the tank leaves the wall or that direction is itself blocked, so it doesn't
    /// flip back and forth against the edge every frame (the cycling bug).
    wall_follow: Option<Dir>,
    /// Active sidestep from predictive collision avoidance: the perpendicular
    /// direction to dodge and how many seconds remain on it. Latched so a dodge
    /// commits for a short window instead of being re-decided every frame.
    dodge_dir: Option<Dir>,
    dodge_timer: f32,
}

impl Default for Ai {
    fn default() -> Self {
        Self {
            waypoint: Position::default(),
            retarget_timer: 0.0,
            fire_timer: ENEMY_FIRE_INTERVAL,
            committed_dir: None,
            dir_hold: 0.0,
            aim_settle: 0.0,
            wall_follow: None,
            dodge_dir: None,
            dodge_timer: 0.0,
        }
    }
}

impl Ai {
    /// Decide this enemy's intent for the frame by ticking the behavior tree.
    /// `rng` is threaded in so patrol wandering is varied; timers advance by `dt`.
    /// `movers` is a snapshot of every live tank (player + all enemies) used for
    /// predictive collision avoidance; `my_index` is this enemy's slot within it,
    /// so it can skip itself. The order matches how the game builds the slice.
    #[allow(clippy::too_many_arguments)] // perception is passed by value, not bundled
    pub fn think(
        &mut self,
        me: &Tank,
        player: &Tank,
        width: f32,
        height: f32,
        dt: f32,
        movers: &[Mover],
        my_index: usize,
        rng: &mut ThreadRng,
    ) -> Intent {
        self.fire_timer = (self.fire_timer - dt).max(0.0);
        self.retarget_timer = (self.retarget_timer - dt).max(0.0);
        self.dir_hold += dt;
        self.dodge_timer = (self.dodge_timer - dt).max(0.0);
        if self.dodge_timer <= 0.0 {
            self.dodge_dir = None;
        }

        let mut bb = Brain {
            me,
            player,
            width,
            height,
            dt,
            movers,
            my_index,
            rng,
            ai: self,
            intent: Intent::default(),
        };
        build().tick(&mut bb);
        bb.intent
    }

    /// Choose a heading toward `target`, but resist flipping: keep the committed
    /// heading unless it's been held long enough AND the freshly-computed heading
    /// beats it by a clear margin along the off-axis. This is the jitter fix.
    ///
    /// `bounds` is the battlefield (width, height) and `half` the tank's clamp
    /// margin, used to deflect the heading along a wall instead of driving into it
    /// (otherwise the tank would pin itself against the edge and get stuck).
    /// `ctx` carries the motion snapshot for predictive collision avoidance.
    fn steer(&mut self, from: Position, target: Position, bounds: (f32, f32), half: f32, ctx: AvoidCtx) -> Dir {
        let fresh = Dir::toward(from, target);
        let dir = match self.committed_dir {
            None => {
                self.commit(fresh);
                fresh
            }
            Some(committed) if self.dir_hold < AI_DIR_HOLD_SECONDS => {
                // Not held long enough yet: stick with the current heading.
                committed
            }
            Some(committed) => {
                // How far off each axis is the target? Only switch if the fresh
                // heading is meaningfully better (reduces the perpendicular error).
                let dx = (target.x - from.x).abs();
                let dy = (target.y - from.y).abs();
                let committed_off = if committed.is_horizontal() { dy } else { dx };
                let fresh_off = if fresh.is_horizontal() { dy } else { dx };
                if fresh != committed && committed_off - fresh_off > AI_DIR_SWITCH_MARGIN_PX {
                    fresh
                } else {
                    committed
                }
            }
        };

        // Sidestep an imminent collision, then deflect along any wall the chosen
        // heading drives into, then commit the final heading so it holds.
        let dir = self.avoid_collisions(dir, from, bounds, half, ctx);
        let dir = self.deflect_from_walls(dir, from, target, bounds, half);
        self.commit(dir);
        dir
    }

    /// Predictively sidestep a likely collision. Given the `desired` heading, look
    /// ahead along it and estimate the closest approach to every other tank; if a
    /// hit looks likely within AVOID_LOOKAHEAD, latch a perpendicular dodge for
    /// AVOID_DODGE_SECONDS and return it. When the dodge expires, normal steering
    /// resumes and pulls the tank back on course — the "away then back" motion.
    fn avoid_collisions(&mut self, desired: Dir, from: Position, bounds: (f32, f32), half: f32, ctx: AvoidCtx) -> Dir {
        // A dodge already in progress holds until its timer runs out (ticked in
        // `think`), as long as it isn't driving into a wall.
        if let Some(dodge) = self.dodge_dir {
            if self.dodge_timer > 0.0 && !heads_into_wall(dodge, from, bounds, half) {
                return dodge;
            }
            self.dodge_dir = None;
        }

        // Too slow to meaningfully predict our own path: don't dodge.
        if ctx.speed < AVOID_MIN_SPEED {
            return desired;
        }

        let step = desired.vec();
        let my_vel = Vector2::new(step.x * ctx.speed, step.y * ctx.speed);

        // Find the soonest predicted collision among the other movers.
        let mut soonest: Option<(f32, Vector2)> = None; // (time, relative position)
        for (i, other) in ctx.movers.iter().enumerate() {
            if i == ctx.my_index {
                continue;
            }
            let p = Vector2::new(other.position.x - from.x, other.position.y - from.y);
            let v = Vector2::new(my_vel.x - other.velocity.x, my_vel.y - other.velocity.y);
            let vv = v.x * v.x + v.y * v.y;
            if vv <= f32::EPSILON {
                continue; // no relative motion
            }
            // Already overlapping is the ram system's job, not ours.
            let sep_now = (p.x * p.x + p.y * p.y).sqrt();
            let reach = ctx.radius + other.radius + AVOID_MARGIN;
            if sep_now < reach {
                continue;
            }
            // Moving apart? (relative velocity points away) then no approach.
            let pv = p.x * v.x + p.y * v.y;
            if pv >= 0.0 {
                continue;
            }
            let t = (-pv / vv).clamp(0.0, AVOID_LOOKAHEAD);
            let cx = p.x + v.x * t;
            let cy = p.y + v.y * t;
            let closest = (cx * cx + cy * cy).sqrt();
            if closest < reach && soonest.is_none_or(|(bt, _)| t < bt) {
                soonest = Some((t, p));
            }
        }

        let Some((_, rel)) = soonest else {
            return desired;
        };

        // Pick the dodge side: turn away from where the obstacle sits relative to
        // our heading, using the 2D cross product's sign. On a near-tie, break it
        // deterministically by our index so two tanks don't mirror into each other.
        let cross = step.x * rel.y - step.y * rel.x;
        let turn_left = if cross.abs() < 1.0 {
            ctx.my_index.is_multiple_of(2)
        } else {
            cross > 0.0
        };
        let primary = perpendicular(desired, turn_left);
        let secondary = perpendicular(desired, !turn_left);

        // Prefer a dodge side that is neither walled nor itself about to collide.
        let choice = [primary, secondary].into_iter().find(|&d| {
            !heads_into_wall(d, from, bounds, half) && !self.dir_collides(d, from, ctx)
        });
        // Fall back to any non-walled side; if both are walls, abandon the dodge.
        let choice = choice.or_else(|| {
            [primary, secondary]
                .into_iter()
                .find(|&d| !heads_into_wall(d, from, bounds, half))
        });

        match choice {
            Some(dir) => {
                self.dodge_dir = Some(dir);
                self.dodge_timer = AVOID_DODGE_SECONDS;
                dir
            }
            None => desired,
        }
    }

    /// True if heading `dir` from `from` at full speed would come dangerously close
    /// to another mover within the lookahead — used to reject a dodge side that
    /// merely trades one collision for another.
    fn dir_collides(&self, dir: Dir, from: Position, ctx: AvoidCtx) -> bool {
        let step = dir.vec();
        let my_vel = Vector2::new(step.x * ctx.speed, step.y * ctx.speed);
        for (i, other) in ctx.movers.iter().enumerate() {
            if i == ctx.my_index {
                continue;
            }
            let p = Vector2::new(other.position.x - from.x, other.position.y - from.y);
            let v = Vector2::new(my_vel.x - other.velocity.x, my_vel.y - other.velocity.y);
            let vv = v.x * v.x + v.y * v.y;
            if vv <= f32::EPSILON {
                continue;
            }
            let pv = p.x * v.x + p.y * v.y;
            if pv >= 0.0 {
                continue;
            }
            let t = (-pv / vv).clamp(0.0, AVOID_LOOKAHEAD);
            let cx = p.x + v.x * t;
            let cy = p.y + v.y * t;
            let closest = (cx * cx + cy * cy).sqrt();
            if closest < ctx.radius + other.radius + AVOID_MARGIN {
                return true;
            }
        }
        false
    }

    fn commit(&mut self, dir: Dir) {
        if self.committed_dir != Some(dir) {
            self.committed_dir = Some(dir);
            self.dir_hold = 0.0;
        }
    }

    /// If `dir` would push a tank at `from` into a wall it's already up against,
    /// swap to a perpendicular heading that runs along the edge (so the tank
    /// slides past the corner instead of stalling). `bounds` is the field size and
    /// `half` the clamp margin (how close the center gets to an edge).
    ///
    /// The slide direction is latched in `wall_follow` and held until the tank
    /// leaves the wall or that direction is itself blocked. Without this latch the
    /// tank re-decides which way to slide every frame from the target's position,
    /// and flips to the opposite way the instant it passes the target's level,
    /// then flips back — cycling forever against the edge.
    fn deflect_from_walls(
        &mut self,
        dir: Dir,
        from: Position,
        target: Position,
        bounds: (f32, f32),
        half: f32,
    ) -> Dir {
        let (width, height) = bounds;
        let skin = half + 1.0;
        let at_left = from.x <= skin;
        let at_right = from.x >= width - skin;
        let at_top = from.y <= skin;
        let at_bottom = from.y >= height - skin;

        // Would the heading drive further into a wall we're already against?
        if !heads_into_wall(dir, from, bounds, half) {
            // Free of the wall: forget any slide direction so the next wall we hit
            // picks a fresh one.
            self.wall_follow = None;
            return dir;
        }

        // A latched slide direction stays in force as long as it isn't itself
        // driving into a wall now — this is what stops the back-and-forth cycling.
        if let Some(follow) = self.wall_follow
            && !heads_into_wall(follow, from, bounds, half)
        {
            return follow;
        }

        // Pick a new slide direction along the perpendicular axis, heading toward
        // the target and avoiding a second wall if this is a corner.
        let follow = if dir.is_horizontal() {
            // Hit a vertical wall (left/right): move along Y toward the target.
            let want_down = target.y >= from.y;
            if want_down && !at_bottom {
                Dir::Down
            } else if !want_down && !at_top {
                Dir::Up
            } else if !at_bottom {
                Dir::Down
            } else {
                Dir::Up
            }
        } else {
            // Hit a horizontal wall (top/bottom): move along X toward the target.
            let want_right = target.x >= from.x;
            if want_right && !at_right {
                Dir::Right
            } else if !want_right && !at_left {
                Dir::Left
            } else if !at_right {
                Dir::Right
            } else {
                Dir::Left
            }
        };
        self.wall_follow = Some(follow);
        follow
    }
}

impl Dir {
    /// True for Left/Right (movement along the x axis).
    pub fn is_horizontal(self) -> bool {
        matches!(self, Dir::Left | Dir::Right)
    }
}

/// The perpendicular of `dir`, turning left (counter-clockwise) or right. Used to
/// pick a sidestep heading for collision avoidance.
fn perpendicular(dir: Dir, left: bool) -> Dir {
    match (dir, left) {
        (Dir::Up, true) => Dir::Left,
        (Dir::Up, false) => Dir::Right,
        (Dir::Down, true) => Dir::Right,
        (Dir::Down, false) => Dir::Left,
        (Dir::Left, true) => Dir::Down,
        (Dir::Left, false) => Dir::Up,
        (Dir::Right, true) => Dir::Up,
        (Dir::Right, false) => Dir::Down,
    }
}

/// True if heading `dir` from `from` would drive further into a battlefield edge
/// the tank is already pressed against (within a 1px skin over the clamp margin).
fn heads_into_wall(dir: Dir, from: Position, bounds: (f32, f32), half: f32) -> bool {
    let (width, height) = bounds;
    let skin = half + 1.0;
    match dir {
        Dir::Left => from.x <= skin,
        Dir::Right => from.x >= width - skin,
        Dir::Up => from.y <= skin,
        Dir::Down => from.y >= height - skin,
    }
}

/// Per-frame context for predictive collision avoidance, passed down to `steer`.
#[derive(Clone, Copy)]
struct AvoidCtx<'a> {
    /// Snapshot of every live tank's motion (player + all enemies).
    movers: &'a [Mover],
    /// This tank's slot in `movers`, so it can skip itself.
    my_index: usize,
    /// This tank's collision radius (half the hull footprint).
    radius: f32,
    /// This tank's movement speed (px/s).
    speed: f32,
}

/// The behavior-tree blackboard: transient per-frame perception plus references
/// to the enemy's persistent memory and the output intent the tree fills in.
struct Brain<'a> {
    me: &'a Tank,
    player: &'a Tank,
    width: f32,
    height: f32,
    dt: f32,
    /// Motion snapshot of all live tanks, for predictive collision avoidance.
    movers: &'a [Mover],
    /// This tank's slot within `movers`.
    my_index: usize,
    rng: &'a mut ThreadRng,
    ai: &'a mut Ai,
    intent: Intent,
}

impl Brain<'_> {
    fn dist_to_player(&self) -> f32 {
        self.me.position.distance_to(self.player.position)
    }

    fn player_alive(&self) -> bool {
        !self.player.is_wreck()
    }

    /// Steer toward `target`, deflecting along the battlefield edges so the tank
    /// never pins itself against a wall and sidestepping predicted collisions.
    /// Wraps `Ai::steer` with this tank's bounds and clamp margin (which mirror
    /// `Tank::clamp_to_field`) plus the motion snapshot for avoidance.
    fn steer(&mut self, target: Position) -> Dir {
        let half = self.me.hull_size() * 0.5;
        let ctx = AvoidCtx {
            movers: self.movers,
            my_index: self.my_index,
            radius: half,
            speed: self.me.speed,
        };
        self.ai
            .steer(self.me.position, target, (self.width, self.height), half, ctx)
    }

    /// Perpendicular offset of the player from the firing axis toward them, and
    /// whether the player is actually in front (positive along the fire dir).
    fn aim_alignment(&self) -> (Dir, f32, bool) {
        let dir = Dir::toward(self.me.position, self.player.position);
        let dx = self.player.position.x - self.me.position.x;
        let dy = self.player.position.y - self.me.position.y;
        let (off_axis, forward) = match dir {
            Dir::Up => (dx.abs(), -dy),
            Dir::Down => (dx.abs(), dy),
            Dir::Left => (dy.abs(), -dx),
            Dir::Right => (dy.abs(), dx),
        };
        (dir, off_axis, forward > 0.0)
    }
}

/// Build the enemy behavior tree. Priority (Selector) order, highest first:
///   1. Dead? do nothing.
///   2. Flee when badly hurt.
///   3. Attack when in range (aim, settle, fire; else close in).
///   4. Chase when the player is visible.
///   5. Patrol otherwise.
/// The tree is rebuilt each tick (cheap: a handful of enum nodes) for clarity.
fn build<'a>() -> Node<Brain<'a>> {
    selector(vec![
        // 1. Wrecks are inert.
        sequence(vec![
            condition(|b: &mut Brain| b.me.is_wreck()),
            action(|_b: &mut Brain| Status::Success),
        ]),
        // 2. Flee when badly damaged and the player is still a threat.
        sequence(vec![
            condition(|b: &mut Brain| b.me.damage >= ENEMY_FLEE_DAMAGE && b.player_alive()),
            action(act_flee),
        ]),
        // 3. Attack when the player is alive and within attack range.
        sequence(vec![
            condition(|b: &mut Brain| b.player_alive() && b.dist_to_player() <= ENEMY_ATTACK_RANGE),
            action(act_attack),
        ]),
        // 4. Chase when the player is alive and within view range.
        sequence(vec![
            condition(|b: &mut Brain| b.player_alive() && b.dist_to_player() <= ENEMY_VIEW_RANGE),
            action(act_chase),
        ]),
        // 5. Fallback: patrol.
        action(act_patrol),
    ])
}

// --- Leaf actions. Each fills in `b.intent` and returns Success. ---

/// Drive away from the player along a committed cardinal heading.
fn act_flee(b: &mut Brain) -> Status {
    // Steer toward a point behind us (mirror of the player across our position),
    // so commitment/hysteresis applies just like chasing.
    let away_point = Position::new(
        2.0 * b.me.position.x - b.player.position.x,
        2.0 * b.me.position.y - b.player.position.y,
    );
    let dir = b.steer(away_point);
    b.intent.move_dir = Some(dir);
    b.reset_aim();
    Status::Success
}

/// Hold near the player and shoot when lined up on a cardinal axis. The tank
/// only fires after staying aligned for ENEMY_AIM_SETTLE, and stops to aim.
fn act_attack(b: &mut Brain) -> Status {
    let (fire_dir, off_axis, in_front) = b.aim_alignment();
    let aligned = off_axis <= ENEMY_FIRE_ALIGN_PX && in_front;

    if aligned {
        // Line up: face the fire direction and hold position while settling.
        b.ai.aim_settle += b.dt;
        b.intent.face = Some(fire_dir);
        // Keep the committed heading in sync so leaving Attack doesn't snap.
        b.ai.commit(fire_dir);

        if b.ai.aim_settle >= ENEMY_AIM_SETTLE && b.ai.fire_timer <= 0.0 {
            b.intent.fire = true;
            b.intent.fire_aim_offset = b.roll_misfire();
            b.ai.fire_timer = ENEMY_FIRE_INTERVAL;
        }
    } else {
        // Not lined up: reposition toward the player (with commitment).
        b.reset_aim();
        let dir = b.steer(b.player.position);
        b.intent.move_dir = Some(dir);
    }
    Status::Success
}

/// Close in on the player along a committed cardinal heading.
fn act_chase(b: &mut Brain) -> Status {
    let dir = b.steer(b.player.position);
    b.intent.move_dir = Some(dir);
    b.reset_aim();
    Status::Success
}

/// Wander toward a roaming waypoint, refreshed periodically or on arrival.
fn act_patrol(b: &mut Brain) -> Status {
    if b.ai.retarget_timer <= 0.0 || b.me.position.distance_to(b.ai.waypoint) < b.me.size() {
        let margin = b.me.size();
        b.ai.waypoint = Position::new(
            b.rng.random_range(margin..(b.width - margin)),
            b.rng.random_range(margin..(b.height - margin)),
        );
        b.ai.retarget_timer = ENEMY_RETARGET_SECONDS;
    }
    let dir = b.steer(b.ai.waypoint);
    b.intent.move_dir = Some(dir);
    b.reset_aim();
    Status::Success
}

impl Brain<'_> {
    /// Reset the aim-settle timer whenever the tank isn't holding a firing line.
    fn reset_aim(&mut self) {
        self.ai.aim_settle = 0.0;
    }

    /// Decide whether this shot misfires because the player is dangerously close,
    /// returning the angular deflection (degrees, signed) to add to the shot. The
    /// closer the player, the likelier the miss; zero means a clean shot. Beyond
    /// ENEMY_MISFIRE_RANGE the enemy always fires straight.
    fn roll_misfire(&mut self) -> f32 {
        let dist = self.dist_to_player();
        if dist >= ENEMY_MISFIRE_RANGE {
            return 0.0;
        }
        // Chance ramps from 0 at the range edge up to _CHANCE_MAX point-blank.
        let closeness = 1.0 - dist / ENEMY_MISFIRE_RANGE;
        let chance = closeness * ENEMY_MISFIRE_CHANCE_MAX;
        if self.rng.random_range(0.0..1.0) >= chance {
            return 0.0;
        }
        // Misfire: deflect by a random magnitude to either side.
        let mag = self.rng.random_range(ENEMY_MISFIRE_ANGLE_MIN..ENEMY_MISFIRE_ANGLE_MAX);
        if self.rng.random_range(0.0..1.0) < 0.5 { -mag } else { mag }
    }
}
