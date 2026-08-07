use rand::RngExt;
use rand::rngs::ThreadRng;
use sola_raylib::prelude::Vector2;

use crate::bt::{Node, Status, action, condition, selector, sequence};
use crate::pathfind::Grid;
use crate::tank::{Dir, Tank};
use crate::{
    AI_DIR_HOLD_SECONDS, AI_DIR_SWITCH_MARGIN_PX, AVOID_DODGE_SECONDS, AVOID_LOOKAHEAD,
    AVOID_MARGIN, AVOID_MIN_SPEED, ENEMY_AIM_SETTLE, ENEMY_AMMO_LOW, ENEMY_AMMO_RESUME,
    ENEMY_ATTACK_RANGE, ENEMY_FIRE_ALIGN_PX, ENEMY_FIRE_INTERVAL, ENEMY_FIRE_INTERVAL_AGGRESSIVE,
    ENEMY_FLEE_DAMAGE, ENEMY_FRIENDLY_FIRE_HOLD_CHANCE, ENEMY_MISFIRE_ANGLE_MAX,
    ENEMY_MISFIRE_ANGLE_MIN, ENEMY_MISFIRE_CHANCE_MAX, ENEMY_MISFIRE_RANGE,
    ENEMY_RETARGET_SECONDS, ENEMY_RETREAT_RANGE, ENEMY_VIEW_RANGE, MAX_DAMAGE, MAX_SHELLS, Position,
};

/// A read-only snapshot of one tank's motion for collision prediction. The game
/// builds a slice of these (all live tanks: player + enemies) each frame and hands
/// it to every enemy's `think`, so an enemy can predict closest approach to the
/// others without borrowing the mutable tank list.
#[derive(Clone, Copy)]
pub struct Mover {
    pub position: Position,
    pub velocity: Vector2,
    /// Collision radius - see `Tank::avoidance_radius` (the true
    /// bounding-circle radius of the tank's real, per-row physics footprint
    /// at its current facing, not a flat approximation).
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
    /// Active sidestep from predictive collision avoidance: the perpendicular
    /// direction to dodge and how many seconds remain on it. Latched so a dodge
    /// commits for a short window instead of being re-decided every frame.
    dodge_dir: Option<Dir>,
    dodge_timer: f32,
    /// True while backing off to recharge ammo. Latched between
    /// ENEMY_AMMO_LOW and ENEMY_AMMO_RESUME (see `wants_retreat`) so the tank
    /// doesn't flicker into and out of Attack every frame near either
    /// threshold.
    retreating: bool,
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
            dodge_dir: None,
            dodge_timer: 0.0,
            retreating: false,
        }
    }
}

impl Ai {
    /// Decide this enemy's intent for the frame by ticking the behavior tree.
    /// `rng` is threaded in so patrol wandering is varied; timers advance by `dt`.
    /// `movers` is a snapshot of every live tank (player + all enemies) used for
    /// predictive collision avoidance; `my_index` is this enemy's slot within it,
    /// so it can skip itself. The order matches how the game builds the slice.
    /// `grid` is this frame's obstacle occupancy grid (see `pathfind::Grid`),
    /// used by `steer` to route around static obstacles.
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
        grid: &Grid,
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
            grid,
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
    /// `bounds` is the battlefield (width, height) and `half` the tank's collision
    /// radius, used by predictive collision avoidance to avoid dodging straight
    /// into a wall (see `avoid_collisions`) - actually driving into a wall is the
    /// physics engine's job now (see docs/physics-engine-design.md), not steering's.
    /// `ctx` carries the motion snapshot for predictive collision avoidance.
    /// `grid` routes the naive straight-line heading around static obstacles
    /// (see `pathfind::Grid::next_step`) before any of the commitment/dodge
    /// logic below ever sees it - obstacles are just another reason the
    /// "fresh" heading might differ from a straight line at the player/
    /// waypoint, same as a jittery target would.
    fn steer(
        &mut self,
        from: Position,
        target: Position,
        bounds: (f32, f32),
        half: f32,
        ctx: AvoidCtx,
        grid: &Grid,
    ) -> Dir {
        let routed = grid.next_step(from, target).unwrap_or(target);
        let fresh = Dir::toward(from, routed);
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
                // How far off each axis is the (routed) aim point? Only switch
                // if the fresh heading is meaningfully better (reduces the
                // perpendicular error). Uses `routed`, not `target`, so this
                // reads as "how good is `fresh` at reaching where it's
                // actually walking toward this step" - comparing it against
                // the original, possibly-far-away `target` would judge a
                // pathfinding detour by the wrong yardstick.
                let dx = (routed.x - from.x).abs();
                let dy = (routed.y - from.y).abs();
                let committed_off = if committed.is_horizontal() { dy } else { dx };
                let fresh_off = if fresh.is_horizontal() { dy } else { dx };
                if fresh != committed && committed_off - fresh_off > AI_DIR_SWITCH_MARGIN_PX {
                    fresh
                } else {
                    committed
                }
            }
        };

        // Sidestep an imminent collision, then commit the final heading so it
        // holds. Driving into a wall is no longer steering's problem - the
        // physics engine's wall colliders stop/slide the tank for real.
        let dir = self.avoid_collisions(dir, from, bounds, half, ctx);
        self.commit(dir);
        dir
    }

    /// Predictively sidestep a likely collision. Given the `desired` heading, look
    /// ahead along it and estimate the closest approach to every other tank; if a
    /// hit looks likely within AVOID_LOOKAHEAD, latch a perpendicular dodge for
    /// AVOID_DODGE_SECONDS and return it. When the dodge expires, normal steering
    /// resumes and pulls the tank back on course — the "away then back" motion.
    fn avoid_collisions(
        &mut self,
        desired: Dir,
        from: Position,
        bounds: (f32, f32),
        half: f32,
        ctx: AvoidCtx,
    ) -> Dir {
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
        let choice = [primary, secondary]
            .into_iter()
            .find(|&d| !heads_into_wall(d, from, bounds, half) && !self.dir_collides(d, from, ctx));
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

    /// Whether this tank should be backing off to recharge instead of
    /// fighting, given its current ammo. Hysteresis between ENEMY_AMMO_LOW
    /// and ENEMY_AMMO_RESUME: crossing the low mark latches retreat on,
    /// crossing the (higher) resume mark latches it back off, and anywhere
    /// in between just keeps whatever was already decided.
    fn wants_retreat(&mut self, ammo: i32) -> bool {
        if ammo <= ENEMY_AMMO_LOW {
            self.retreating = true;
        } else if ammo >= ENEMY_AMMO_RESUME {
            self.retreating = false;
        }
        self.retreating
    }

    /// True while backing off to recharge ammo (see `wants_retreat`). Read
    /// by the inspect-mode debug overlay (`game.rs`); not used by any
    /// gameplay decision outside this module.
    pub fn is_retreating(&self) -> bool {
        self.retreating
    }

    /// Seconds until this tank may fire again (zero or negative means
    /// ready). Read by the inspect-mode debug overlay (`game.rs`).
    pub fn fire_cooldown(&self) -> f32 {
        self.fire_timer
    }

    fn commit(&mut self, dir: Dir) {
        if self.committed_dir != Some(dir) {
            self.committed_dir = Some(dir);
            self.dir_hold = 0.0;
        }
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
    /// This tank's collision radius - see `Tank::avoidance_radius`.
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
    /// This frame's obstacle occupancy grid, for routing around static
    /// obstacles - see `Ai::steer`.
    grid: &'a Grid,
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

    /// Seconds to wait before firing again, scaled by how well-stocked this
    /// tank is: a full-ammo, full-health tank re-fires at
    /// ENEMY_FIRE_INTERVAL_AGGRESSIVE, and it eases back toward the baseline
    /// ENEMY_FIRE_INTERVAL as either ammo or health drops - whichever
    /// resource is scarcer sets the pace (min, not average), since being
    /// low on either alone is reason enough to ease off. Attack is only
    /// reached above both ENEMY_AMMO_LOW and ENEMY_FLEE_DAMAGE (lower on
    /// either and the retreat/flee branches take over instead), so this
    /// never actually hits the slow end in practice - it's the "more
    /// resources, more aggressive" half; wants_retreat/act_flee are the
    /// "running low, fall back" half.
    fn fire_interval(&self) -> f32 {
        let ammo_frac = (self.me.shells_ammo as f32 / MAX_SHELLS as f32).clamp(0.0, 1.0);
        let health_frac = (1.0 - self.me.damage / MAX_DAMAGE).clamp(0.0, 1.0);
        let aggression = ammo_frac.min(health_frac);
        ENEMY_FIRE_INTERVAL - (ENEMY_FIRE_INTERVAL - ENEMY_FIRE_INTERVAL_AGGRESSIVE) * aggression
    }

    /// Steer toward `target`, routing around static obstacles and
    /// sidestepping predicted collisions. Wraps `Ai::steer` with this tank's
    /// bounds and collision radius (`Tank::avoidance_radius` - a safe
    /// over-approximation of the tank's real, per-row physics collider, so
    /// this never assumes a tank is smaller than it actually is), the motion
    /// snapshot for avoidance, and this frame's obstacle grid.
    fn steer(&mut self, target: Position) -> Dir {
        let radius = self.me.avoidance_radius();
        let ctx = AvoidCtx {
            movers: self.movers,
            my_index: self.my_index,
            radius,
            speed: self.me.effective_speed(),
        };
        self.ai.steer(
            self.me.position,
            target,
            (self.width, self.height),
            radius,
            ctx,
            self.grid,
        )
    }

    /// Perpendicular offset of the player from the firing axis toward them, and
    /// whether the player is actually in front (positive along the fire dir).
    fn aim_alignment(&self) -> (Dir, f32, bool) {
        let dir = Dir::toward(self.me.position, self.player.position);
        let (off_axis, forward) = axis_offsets(self.me.position, self.player.position, dir);
        (dir, off_axis, forward > 0.0)
    }

    /// True if another enemy sits roughly on `fire_dir`'s line, closer than
    /// `max_forward` (normally the distance to the player) - i.e. firing
    /// straight down that axis right now would hit a teammate before the
    /// shot ever reached its intended target. Checked against `movers`
    /// (skipping slot 0, the player, and this tank's own slot) since that's
    /// all the perception `Brain` is handed - see `Ai::think`'s doc comment.
    /// Shells can hit any tank except whoever fired them (see
    /// `Game::update`), so this is what `act_attack` uses to mostly (not
    /// always - see `ENEMY_FRIENDLY_FIRE_HOLD_CHANCE`) hold fire rather than
    /// shoot through a friendly.
    fn friendly_blocks_shot(&self, fire_dir: Dir, max_forward: f32) -> bool {
        self.movers.iter().enumerate().any(|(i, mover)| {
            if i == 0 || i == self.my_index {
                return false;
            }
            let (off_axis, forward) = axis_offsets(self.me.position, mover.position, fire_dir);
            forward > 0.0 && forward < max_forward && off_axis <= ENEMY_FIRE_ALIGN_PX
        })
    }
}

/// Perpendicular and forward distance of `to` from `from` along the cardinal
/// axis `dir` points along - shared by aim alignment (target: the player) and
/// friendly-fire avoidance (target: another enemy), so both read the same way.
fn axis_offsets(from: Position, to: Position, dir: Dir) -> (f32, f32) {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    match dir {
        Dir::Up => (dx.abs(), -dy),
        Dir::Down => (dx.abs(), dy),
        Dir::Left => (dy.abs(), -dx),
        Dir::Right => (dy.abs(), dx),
    }
}

/// Build the enemy behavior tree. Priority (Selector) order, highest first:
///   1. Dead? do nothing.
///   2. Flee when badly hurt.
///   3. Retreat to recharge when ammo is low.
///   4. Attack when in range (aim, settle, fire; else close in).
///   5. Chase when the player is visible.
///   6. Patrol otherwise.
///
/// The tree is rebuilt each tick (cheap: a handful of enum nodes) for clarity.
fn build<'a>() -> Node<Brain<'a>> {
    selector(vec![
        // 1. Wrecks are inert.
        sequence(vec![
            condition(|b: &mut Brain| b.me.is_wreck()),
            action(|_b: &mut Brain| Status::Success),
        ]),
        // 2. Flee when badly damaged and the player is still a threat.
        // Takes priority over the ammo-based retreat below: survival first.
        sequence(vec![
            condition(|b: &mut Brain| b.me.damage >= ENEMY_FLEE_DAMAGE && b.player_alive()),
            action(act_flee),
        ]),
        // 3. Low on shells: back off and hold fire until recharged.
        sequence(vec![
            condition(|b: &mut Brain| {
                b.player_alive() && b.ai.wants_retreat(b.me.shells_ammo)
            }),
            action(act_retreat),
        ]),
        // 4. Attack when the player is alive and within attack range.
        sequence(vec![
            condition(|b: &mut Brain| b.player_alive() && b.dist_to_player() <= ENEMY_ATTACK_RANGE),
            action(act_attack),
        ]),
        // 5. Chase when the player is alive and within view range.
        sequence(vec![
            condition(|b: &mut Brain| b.player_alive() && b.dist_to_player() <= ENEMY_VIEW_RANGE),
            action(act_chase),
        ]),
        // 6. Fallback: patrol.
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

/// Back off to recharge ammo. Like `act_flee`, but only until clear of
/// ENEMY_RETREAT_RANGE (breathing room outside attack range) rather than
/// running all the way off - once there, it holds position, faces the
/// player, and just waits out the recharge instead of camping the map edge.
/// Never fires: `b.intent.fire` starts false each frame and this leaf
/// doesn't set it.
fn act_retreat(b: &mut Brain) -> Status {
    b.reset_aim();
    if b.dist_to_player() >= ENEMY_RETREAT_RANGE {
        b.intent.face = Some(Dir::toward(b.me.position, b.player.position));
        return Status::Success;
    }
    let away_point = Position::new(
        2.0 * b.me.position.x - b.player.position.x,
        2.0 * b.me.position.y - b.player.position.y,
    );
    let dir = b.steer(away_point);
    b.intent.move_dir = Some(dir);
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
            let blocked = b.friendly_blocks_shot(fire_dir, b.dist_to_player());
            let hold_fire =
                blocked && b.rng.random_range(0.0..1.0) < ENEMY_FRIENDLY_FIRE_HOLD_CHANCE;
            // Whether it fires or holds, this firing opportunity is spent -
            // otherwise a held shot would just re-roll every frame at ~60Hz
            // and fire almost immediately anyway, defeating the hold chance.
            // The interval itself scales with ammo: fuller magazine, faster
            // follow-up shot (see Brain::fire_interval).
            b.ai.fire_timer = b.fire_interval();
            if !hold_fire {
                b.intent.fire = true;
                b.intent.fire_aim_offset = b.roll_misfire();
            }
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
        let mag = self
            .rng
            .random_range(ENEMY_MISFIRE_ANGLE_MIN..ENEMY_MISFIRE_ANGLE_MAX);
        if self.rng.random_range(0.0..1.0) < 0.5 {
            -mag
        } else {
            mag
        }
    }
}
