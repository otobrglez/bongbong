use crate::tuning::tuning;
use serde::{Deserialize, Serialize};
use rand::RngExt;
use rand::rngs::SmallRng;
use crate::math::Vec2;

use crate::bt::{Node, Status, action, condition, selector, sequence};
use crate::obstacle::Material;
use crate::pathfind::{Grid, RouteAhead};
use crate::pickup::PickupKind;
use crate::tank::{ActiveWeapon, Dir, Tank};
use crate::{
    MAX_DAMAGE,
    OBSTACLE_GRID_SIZE,
    Position,
};

/// A read-only snapshot of one tank's motion for collision prediction. The game
/// builds a slice of these (all live tanks: the players first, then the
/// enemies) each frame and hands it to every enemy's `think`, so an enemy
/// can predict closest approach to the others without borrowing the
/// mutable tank list.
#[derive(Clone, Copy)]
pub struct Mover {
    pub position: Position,
    pub velocity: Vec2,
    /// Collision radius - see `Tank::avoidance_radius` (the true
    /// bounding-circle radius of the tank's real, per-row physics footprint
    /// at its current facing, not a flat approximation).
    pub radius: f32,
    /// A human player's tank - never a friendly to an enemy.
    pub is_player: bool,
}

/// What a driver (player or AI) wants to do this frame. The physics layer turns
/// this into a facing/step + firing, so player input and AI decisions flow
/// through the exact same code path. Movement is 4-direction only.
#[derive(Default, Clone, Copy, Debug)]
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
    /// How much to ease off the throttle this frame, 0 (full speed) to 1
    /// (stopped) - `Game::drive_tank` scales the commanded speed by
    /// `speed_scale()`.
    ///
    /// Stored inverted on purpose. This struct derives `Default`, so a
    /// `throttle` field would default to 0.0 and every intent that forgot to
    /// set it would be a *stopped* tank; storing the deviation from normal
    /// means the default is "drive normally" and a forgotten field is
    /// harmless. Read it through `speed_scale()` rather than inverting by
    /// hand at the use site.
    ///
    /// Nothing in `ai.rs` ever sets this: it is the one lever
    /// `simulation::command` has over a tank's movement
    /// (docs/enemy-command-and-control-prd.md), and the player always leaves
    /// it at 0.
    pub slow: f32,
    /// The lamp key held (docs/volcano.md): a player's seat sets a lantern
    /// down on the press, while it has any left. Held, like `fire`, so the
    /// simulation finds the edge and an online packet that repeats a held
    /// key never drops two. The AI never sets it.
    pub lamp: bool,
}

impl Intent {
    /// Fraction of commanded top speed to actually drive at, 0..=1.
    pub fn speed_scale(&self) -> f32 {
        1.0 - self.slow.clamp(0.0, 1.0)
    }
}

/// What stands directly ahead of a tank in one direction - the tile a shot
/// fired that way would hit within breach reach (see `Ai::think`'s
/// `walls_ahead` and `Brain::wants_breach`).
#[derive(Clone, Copy, Debug)]
pub struct WallAhead {
    pub material: Material,
    /// Wood already alight: solid, but shooting it does nothing.
    pub burning: bool,
}

/// A latched decision to shoot through the tile in `dir` (see
/// `Brain::wants_breach`); `timer` is the seconds left before giving up.
#[derive(Clone, Copy)]
struct Breach {
    dir: Dir,
    timer: f32,
}

/// A tower that hurt this tank (`Ai::notify_tower_hit`): where it stands,
/// the seconds the tank holds it against the tower, and whether this frame
/// sees it (`Ai::set_grudge_sight`, from `enemy_phase`).
#[derive(Clone, Copy)]
struct Grudge {
    at: Position,
    timer: f32,
    in_sight: bool,
}

/// A guard's beat (see `Role::Guard`): the annulus `keep_off..=radius`
/// around `anchor`, its own frog. The inner radius keeps the guard out of
/// the frog's hop range - the frog is a solid body that hops away from any
/// tank, its guard included, so crowding it walks it off its post. It does
/// not bite its own side (`frog::Side::bites`).
#[derive(Clone, Copy, Debug)]
struct Leash {
    anchor: Position,
    radius: f32,
    keep_off: f32,
}

impl Leash {
    fn contains(&self, p: Position) -> bool {
        let d = p.distance_to(self.anchor);
        d >= self.keep_off && d <= self.radius
    }
}

/// What an enemy is for this round - rolled once at spawn by `Game::init`
/// from the mission's hunter share (docs/maps-to-levels.md "AI roles").
/// `think`'s `target`/`frog_target` parameters are what the simulation
/// resolves from the role each frame; the tree reads the role itself only
/// for the two role-specific tiers (`build`'s snipe and guard).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    /// Fights the player: the engagement ring is around the player.
    #[default]
    Player,
    /// Drives at and shoots the player's frog through a second ring around
    /// it; shoots the player only when already lined up within attack
    /// range. Behaves as `Player` once that frog is dead.
    Hunter,
    /// Stays within `guard_leash_px` of its own frog: engages the player
    /// like `Player` while the player is inside that leash, otherwise
    /// wanders inside it and heads back whenever it finds itself outside.
    Guard,
}

impl Role {
    pub fn name(self) -> &'static str {
        match self {
            Role::Player => "player",
            Role::Hunter => "hunter",
            Role::Guard => "guard",
        }
    }
}

/// Persistent per-enemy memory that survives across frames. Kept separate from
/// the transient perception so the behavior tree can read state and remember
/// decisions (committed heading, timers) between ticks.
pub struct Ai {
    /// What this enemy is for - see `Role`. Set at spawn, never changes.
    pub role: Role,
    /// A training dummy (docs/training-stage.md): it never fires at a
    /// seat (`Brain::may_fire_at_seat`), so with the hunter's role it goes
    /// only for the frog. False for every other tank.
    pub frog_only: bool,
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
    /// How long this tank has been holding back for something directly in
    /// front of it (`crowded_ahead`). Bounded by `enemy_yield_seconds` -
    /// see that knob for why a brake without a ceiling deadlocks.
    yield_timer: f32,
    /// Seconds until a hunter may take another opportunistic shot at the
    /// player - set by `act_snipe` when it fires, so one snipe never
    /// becomes a standing duel that forgets the frog.
    snipe_cooldown: f32,
    /// True while backing off to recharge ammo. Latched between
    /// ENEMY_AMMO_LOW and ENEMY_AMMO_RESUME (see `wants_retreat`) so the tank
    /// doesn't flicker into and out of Attack every frame near either
    /// threshold.
    retreating: bool,
    /// The *previous* tick's `move_dir` - `None` when this tank was
    /// deliberately holding position (aiming, waiting out a retreat)
    /// rather than trying to move. Set at the end of `think`; read at the
    /// start of the next `think` to judge how much of the real velocity
    /// was progress in the direction that was asked for. See
    /// `stuck_timer`.
    last_move_dir: Option<Dir>,
    /// Where this tank stood at the previous tick - the baseline the next
    /// tick's displacement is measured from. `None` before the first tick.
    last_position: Option<Position>,
    /// The hull's displacement per second since the previous think - how
    /// it is really moving, whatever it was told, which is what its slide
    /// through a turn is reckoned from (`lane_turn`, `walks_into_wall`).
    /// Zero before there is a displacement to judge.
    motion: Vec2,
    /// Displacement per second along the commanded heading, smoothed over
    /// `stuck_progress_window_seconds` (an exponential average). `None`
    /// while the tank is deliberately holding position or has no
    /// displacement to judge yet. The smoothing is what makes the stuck
    /// clock immune to a single-frame twitch: two tanks pressed together
    /// creep a fraction of a pixel a frame and occasionally get shoved a
    /// pixel or two by the contact solver, and an unsmoothed check reset
    /// on every such frame, so the pair sat jammed for good.
    progress_avg: Option<f32>,
    /// Seconds this tank has been asked to move (see `last_move_dir`)
    /// while `progress_avg` stays under `stuck_speed_eps` - ticked in
    /// `think` from the tank's own displacement, never from a physics
    /// velocity: the contact solver hands a tank pushing against another
    /// body a velocity along its heading every frame without it going
    /// anywhere, which is exactly the case this clock exists to catch.
    /// Progress is the displacement's component along the commanded
    /// heading, not its magnitude: a tank wedged in a jam of other tanks
    /// can be carried sideways at near full speed while getting nowhere it
    /// was told to go, and a speed check reads that as "moving fine".
    /// Once this crosses `stuck_escape_seconds`, `steer` forces an escape
    /// and resets it to zero. Catches everything the obstacle-ahead
    /// override in `steer` can't: a bad commitment call it didn't foresee,
    /// a jam against other tanks, or a layout with no path around an
    /// obstacle cluster at all.
    stuck_timer: f32,
    /// Seconds remaining since this tank last took a hit - see
    /// `notify_hit`/`ENEMY_HIT_ALERT_SECONDS`. `build`'s Chase condition
    /// treats this as equivalent to having the player in view range, so a
    /// tank that gets shot from outside its normal awareness range still
    /// fights back instead of obliviously continuing to patrol/wander.
    hit_alert_timer: f32,
    /// True while `wander`'s last waypoint resample came up completely
    /// empty-handed - every candidate it rolled was unreachable, so the
    /// waypoint it settled on is a known-unreachable fallback. While set,
    /// the "current waypoint turned out unreachable, resample right now"
    /// fast path stays suppressed (the normal ENEMY_RETARGET_SECONDS
    /// cadence still applies): re-rolling would almost certainly come up
    /// empty again, and doing that every frame hands the tank a brand-new
    /// random heading per tick - the same spin-in-place failure
    /// `Grid::boxed_in`'s guard exists for, just in a *multi-cell*
    /// reachability pocket where `boxed_in` (a single-cell check) stays
    /// false. Found via the probe harness's `spin` anomaly sweep: tanks
    /// lapping a tight box at full speed, one fresh unreachable waypoint
    /// per frame.
    wander_pocketed: bool,
    /// The behaviour-tree action the last `think` settled on (see
    /// `bt::Node::tick_traced`) and the intent it produced - inspection
    /// only, nothing in `think` reads them back.
    last_action: Option<&'static str>,
    last_intent: Intent,
    /// The seat the last `think`'s trigger pull was aimed at: set when it
    /// fires at the seat it fights (the attack tier, a hunter's snipe),
    /// `None` when it fired at a frog, a tower or a tile or did not fire.
    /// Inspection only, like `last_action` - the probe's `offbox-fire`
    /// check reads it (`TankSnapshot::shot_at_seat`).
    shot_at_seat: Option<u8>,
    /// Seconds running that the tank has commanded movement straight into
    /// a destructible tile (`walls_ahead` in its `last_move_dir`) - the
    /// trigger for a breach. Velocity-independent on purpose: a tank
    /// scraping sideways along a wall it keeps driving at counts.
    wall_ahead_timer: f32,
    /// The breach in progress, if any - see `Brain::wants_breach`.
    breach: Option<Breach>,
    /// The tower this tank fires back at, if one hurt it lately - see
    /// `Brain::grudge_shot`.
    grudge: Option<Grudge>,
    /// Stuck escapes fired this round (see `stuck_timer`) - a counter
    /// rather than a flag so tooling can see an escape that fired and
    /// reset within one frame.
    escapes: u32,
    /// Which seat this tank is fighting: the target of its `Role::Player`
    /// behaviour and the ring it competes on. Always 0 in a single-player
    /// round; with more seats `enemy_phase` retargets it to the nearest
    /// live, visible one past `enemy_target_switch_margin_px`.
    target_player: u8,
    /// What bounds this tank on a field map - its own alert, its home,
    /// its call to the fight, whether anything has woken it. Kept by
    /// `simulation::field` and never touched on an arena, where it stays
    /// at its default and every reader below sees exactly the arena's
    /// tank.
    pub(crate) field: FieldMind,
}

/// The memory a tank carries only on a field map
/// (docs/large-maps-follow-camera.md section 12, `simulation::field`):
/// what replaces the arena's one shared alert, and what decides how often
/// the tank thinks. Plain values the simulation writes between thinks; the
/// tree reads `home` (`Brain::home_leash`) and the alert it is handed.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct FieldMind {
    /// This tank's own alert: where a seat was last seen, by itself or by
    /// a neighbour down the chain (`enemy_alert_chain_px`), and the
    /// seconds it still holds (`enemy_alert_hold_seconds` when fresh).
    pub(crate) alert: Option<Position>,
    pub(crate) alert_timer: f32,
    /// Where the tank first stood on the field - its spawn cell, or its
    /// gate's inside point - the anchor of its leash (`enemy_leash_px`).
    pub(crate) home: Option<Position>,
    /// A wave tank sent to the fight: it routes at the seat it fights
    /// until it first comes within sight range of a seat or takes a hit.
    pub(crate) called: bool,
    /// It came onto the field with a wave - through a gate, or into the
    /// band a map with no gate falls back to - so it may be a straggler
    /// the round rolls in again through a nearer gate
    /// (`Game::reroll_stragglers`).
    pub(crate) wave: bool,
    /// Seconds a wave tank has gone without a live seat, or the players'
    /// frog, within its sight: counted from its arrival and back to zero
    /// whenever one is (`field::mind`). How long it has been lost to the
    /// fight.
    pub(crate) lost: f32,
    /// Something has reached it - an alert, a hit, a call, a seat within
    /// `enemy_far_px`, a frog to hunt. A far tank that was never woken
    /// does not think at all; once woken it stays so.
    pub(crate) awake: bool,
    /// Seconds since its last think while far thinking skips ticks: the
    /// `dt` its next think covers.
    pub(crate) think_debt: f32,
}

/// Read-only view of an `Ai`'s memory for tooling (`Ai::snapshot`).
#[derive(Clone, Copy, Debug, Serialize)]
pub struct AiSnapshot {
    /// `Role::name`.
    pub role: &'static str,
    pub waypoint_x: f32,
    pub waypoint_y: f32,
    pub committed_dir: Option<&'static str>,
    pub dir_hold: f32,
    pub dodge_dir: Option<&'static str>,
    pub dodge_timer: f32,
    pub yield_timer: f32,
    pub snipe_cooldown: f32,
    pub retreating: bool,
    pub stuck_timer: f32,
    pub hit_alert_timer: f32,
    pub aim_settle: f32,
    pub fire_timer: f32,
    pub wander_pocketed: bool,
    pub last_move_dir: Option<&'static str>,
    /// Smoothed displacement per second along the commanded heading
    /// (`Ai::progress_avg`); `None` while holding position.
    pub progress_px_s: Option<f32>,
    pub last_action: Option<&'static str>,
    pub wall_ahead_timer: f32,
    /// Direction of the breach in progress.
    pub breaching: Option<&'static str>,
    pub intent_move: Option<&'static str>,
    pub intent_face: Option<&'static str>,
    pub intent_fire: bool,
    pub retarget_timer: f32,
    /// Seconds left before the breach in progress is given up.
    pub breach_timer: Option<f32>,
    pub escapes: u32,
    /// `Ai::target_player`.
    pub target_player: u8,
    /// The seat the last tick's trigger pull was aimed at (`Ai::shot_at_seat`).
    pub shot_at_seat: Option<u8>,
    /// Field maps (`FieldMind`): the tank's own alert point, its home, its
    /// call to the fight and whether it is awake. `None`/false on an arena.
    pub field_alert: Option<(f32, f32)>,
    pub home: Option<(f32, f32)>,
    pub called: bool,
    pub awake: bool,
}

impl Default for Ai {
    fn default() -> Self {
        Self {
            role: Role::Player,
            frog_only: false,
            waypoint: Position::default(),
            retarget_timer: 0.0,
            fire_timer: tuning().enemy_fire_interval,
            committed_dir: None,
            dir_hold: 0.0,
            aim_settle: 0.0,
            dodge_dir: None,
            dodge_timer: 0.0,
            yield_timer: 0.0,
            snipe_cooldown: 0.0,
            retreating: false,
            last_move_dir: None,
            last_position: None,
            motion: Vec2::new(0.0, 0.0),
            progress_avg: None,
            stuck_timer: 0.0,
            hit_alert_timer: 0.0,
            wander_pocketed: false,
            last_action: None,
            last_intent: Intent::default(),
            shot_at_seat: None,
            wall_ahead_timer: 0.0,
            breach: None,
            grudge: None,
            escapes: 0,
            target_player: 0,
            field: FieldMind::default(),
        }
    }
}

impl Ai {
    /// A fresh memory for an enemy spawning with `role`.
    pub fn with_role(role: Role) -> Self {
        Ai { role, ..Ai::default() }
    }

    /// Decide this enemy's intent for the frame by ticking the behavior tree.
    /// `rng` is threaded in so patrol wandering is varied; timers advance by `dt`.
    /// `movers` is a snapshot of every live tank (player + all enemies) used for
    /// predictive collision avoidance; `my_index` is this enemy's slot within it,
    /// so it can skip itself. The order matches how the game builds the slice.
    /// `grid` is this frame's obstacle occupancy grid (see `pathfind::Grid`),
    /// used by `steer` to route around static obstacles. Progress toward a
    /// commanded heading is judged from `me.position` against the position
    /// seen at the previous tick (`last_position`) - what the tank actually
    /// displaced, never a physics velocity - see `stuck_timer`.
    /// `alert` is this frame's shared "last known player position" (see
    /// `simulation.rs`'s `Game::alert_position`) - `Some` while any enemy on
    /// the field currently has the player within sight (`Game::enemy_sight`),
    /// or did within the last `ENEMY_ALERT_HOLD_SECONDS`, so an enemy that can't
    /// personally see the player can still converge on where the group last
    /// saw them instead of wandering randomly - see `act_patrol`. On a field
    /// map it is this tank's own alert instead, or its call to the fight
    /// (`simulation::field`): an alert there reaches only the enemies
    /// chained to the one that saw.
    /// `engage_target` is this tank's assigned point on the shared
    /// engagement ring around the player (see `simulation.rs::Game::update`'s
    /// `engage_targets`, and `ENGAGE_RING_RADIUS`'s doc comment) - `Some`
    /// whenever two or more enemies are simultaneously within sight, so
    /// `act_chase`/`act_attack` can steer at a point
    /// that's spread out from the other engaged enemies instead of the
    /// player's exact position, which is what used to send a whole group
    /// at the same spot and pile them up. `None` when this tank is the only
    /// one engaged (or the player is dead) - nothing to spread out from, so
    /// those fall back to the raw player position.
    /// `pickups` is every currently-live health/ammo pickup on the
    /// battlefield this frame (kind + position) - see `Brain::nearest_pickup`,
    /// used by `act_flee`/`act_retreat` so a hurting or ammo-starved tank
    /// heads for a pickup instead of just running blind.
    /// `line_of_sight` is whether this tank's straight line to `target` is
    /// currently unobstructed by terrain (any obstacle, or a frog other than
    /// the target itself) - computed in `simulation` (see
    /// `hits::Terrain::line_of_sight`) against the real, un-margined
    /// obstacle geometry a shot would actually resolve against,
    /// deliberately *not* `pathfind::Grid`: that grid's clearance margin is
    /// sized for a tank's own hull to route through gaps, not for a
    /// shell's, so reusing it here rejected plenty of geometrically clear
    /// shots on denser maps and left engaged enemies unable to ever find a
    /// firing solution. Gates `act_attack`'s fire decision so an enemy
    /// aligned through a wall it can't destroy (or with a frog in the way)
    /// doesn't just fire into it forever. For a hunter it is the line of
    /// *fire* to the frog instead (`hits::Terrain::line_of_fire_to_frog`):
    /// only iron or another frog blocks it, so a walled-in frog is shot at
    /// through its destructible walls until they fall. `player_line_of_sight` is the
    /// same test toward the player, for a hunter's opportunistic shot
    /// (`build`'s snipe tier); identical to `line_of_sight` whenever the
    /// player *is* the target.
    /// `target` is the point this tank fights - the player's position, or
    /// for a `Role::Hunter` the player's frog while it lives - the thing
    /// `act_chase`/`act_attack` close on and aim at. `frog_target` is the
    /// frog this role is anchored to, while it lives: a hunter's quarry
    /// (the player's frog, so `Some` means `target` is that frog; `None`
    /// once it is dead, and the hunter fights the player like everyone
    /// else) or a guard's own frog (the leash anchor). `None` for
    /// `Role::Player`.
    /// `walls_ahead` is, per `Dir` (indexed by `Dir::index`), the tile a
    /// shot fired that way would hit within breach reach
    /// (`Terrain::obstacle_ahead`), so a tank wedged against a brick can
    /// decide to shoot it down - see `Brain::wants_breach`.
    /// `sight` is how far this tank sees (px): `enemy_view_range` under the
    /// round's sky (`Game::enemy_sight`, docs/weather.md) - the range it
    /// chases from, and a cap on the range it attacks and snipes from.
    #[allow(clippy::too_many_arguments)] // perception is passed by value, not bundled
    pub fn think(
        &mut self,
        me: &Tank,
        player: &Tank,
        target: Position,
        frog_target: Option<Position>,
        width: f32,
        height: f32,
        dt: f32,
        movers: &[Mover],
        my_index: usize,
        grid: &Grid,
        rng: &mut SmallRng,
        alert: Option<Position>,
        engage_target: Option<Position>,
        pickups: &[(PickupKind, Position)],
        line_of_sight: bool,
        player_line_of_sight: bool,
        target_concealed: bool,
        walls_ahead: [Option<WallAhead>; 4],
        sight: f32,
    ) -> Intent {
        self.fire_timer = (self.fire_timer - dt).max(0.0);
        self.retarget_timer = (self.retarget_timer - dt).max(0.0);
        self.hit_alert_timer = (self.hit_alert_timer - dt).max(0.0);
        self.dir_hold += dt;
        self.dodge_timer = (self.dodge_timer - dt).max(0.0);
        self.snipe_cooldown = (self.snipe_cooldown - dt).max(0.0);
        self.shot_at_seat = None;
        if let Some(grudge) = &mut self.grudge {
            grudge.timer -= dt;
            if grudge.timer <= 0.0 {
                self.grudge = None;
            }
        }
        if self.dodge_timer <= 0.0 {
            self.dodge_dir = None;
        }
        // Was asked to move last tick and made no headway in that
        // direction: another dt of stuck evidence. Only the displacement
        // component along the commanded heading counts - being shoved
        // sideways or backwards by other tanks is not progress - and it is
        // smoothed over `stuck_progress_window_seconds` so one twitchy
        // frame cannot clear the clock. Wasn't asked to move resets it -
        // deliberately holding position to aim/wait isn't stuck. See
        // `steer_toward`'s escape.
        let moved = self.last_position.map(|p| Position::new(me.position.x - p.x, me.position.y - p.y));
        self.last_position = Some(me.position);
        self.motion = moved.map_or(Vec2::new(0.0, 0.0), |m| Vec2::new(m.x / dt.max(f32::EPSILON), m.y / dt.max(f32::EPSILON)));
        match (self.last_move_dir, moved) {
            (Some(dir), Some(moved)) => {
                let progress = (moved.x * dir.vec().x + moved.y * dir.vec().y) / dt.max(f32::EPSILON);
                let k = (dt / tuning().stuck_progress_window_seconds).clamp(0.0, 1.0);
                let avg = match self.progress_avg {
                    Some(avg) => avg + (progress - avg) * k,
                    None => progress,
                };
                self.progress_avg = Some(avg);
                if avg < tuning().stuck_speed_eps {
                    self.stuck_timer += dt;
                } else {
                    self.stuck_timer = 0.0;
                }
            }
            // Commanded, but no baseline to measure against yet (first
            // tick): no evidence either way.
            (Some(_), None) => {}
            (None, _) => {
                self.stuck_timer = 0.0;
                self.progress_avg = None;
            }
        }
        // Breach evidence: still commanding movement into a tile that a
        // shell could remove. Iron never counts.
        let into_wall = self
            .last_move_dir
            .and_then(|d| walls_ahead[d.index()])
            .is_some_and(|w| !w.material.is_permanent());
        if into_wall {
            self.wall_ahead_timer += dt;
        } else {
            self.wall_ahead_timer = 0.0;
        }
        if let Some(breach) = &mut self.breach {
            breach.timer -= dt;
        }

        let mut bb = Brain {
            me,
            player,
            target,
            frog_target,
            width,
            height,
            dt,
            movers,
            my_index,
            grid,
            rng,
            ai: self,
            intent: Intent::default(),
            alert,
            engage_target,
            pickups,
            line_of_sight,
            player_line_of_sight,
            target_concealed,
            walls_ahead,
            sight,
        };
        let mut last_action = None;
        build().tick_traced(&mut bb, &mut last_action);
        let mut intent = bb.intent;

        // Personal space, applied to whatever the tree decided: pull up
        // short of the tank in front instead of driving through it. One
        // place rather than inside each behaviour, because every one of
        // them - attack, chase, patrol, wander, hunter, guard - converges
        // on the same few places and slams the same way.
        //
        // This is deliberately a *brake*, not another sidestep.
        // `avoid_collisions` already dodges paths that are going to cross,
        // and it explicitly stops caring once two tanks are inside
        // `avoid_margin` of each other - which is exactly the regime that
        // produces the slam. Stopping is also what a driver would do.
        //
        // `face` is kept so a tank holding station still aims and shoots;
        // only the driving stops.
        //
        // The timer keeps counting for as long as the way ahead stays
        // blocked and resets only when it clears, so a tank holds *once*
        // for `enemy_yield_seconds` and then drives on. Decaying it while
        // still blocked instead - the obvious first cut - makes the tank
        // brake, release, fall back under the ceiling and brake again: a
        // permanent half-speed shuffle rather than a yield, which left two
        // enemies jammed corner to corner in a corridor taking far longer
        // to work free (`mechanics_tests`).
        if let Some(dir) = intent.move_dir {
            if crowded_ahead(me.position, dir, movers, my_index) {
                self.yield_timer += dt;
                if self.yield_timer <= tuning().enemy_yield_seconds {
                    intent.move_dir = None;
                    intent.face = intent.face.or(Some(dir));
                }
            } else {
                self.yield_timer = 0.0;
            }
        } else {
            self.yield_timer = 0.0;
        }

        let intent = intent;
        self.last_move_dir = intent.move_dir;
        self.last_action = last_action;
        self.last_intent = intent;
        intent
    }

    /// This tank's AI memory as plain values - for the dev server's
    /// snapshot and overlays.
    pub fn snapshot(&self) -> AiSnapshot {
        AiSnapshot {
            role: self.role.name(),
            waypoint_x: self.waypoint.x,
            waypoint_y: self.waypoint.y,
            committed_dir: self.committed_dir.map(Dir::name),
            dir_hold: self.dir_hold,
            dodge_dir: self.dodge_dir.map(Dir::name),
            dodge_timer: self.dodge_timer,
            yield_timer: self.yield_timer,
            snipe_cooldown: self.snipe_cooldown,
            retreating: self.retreating,
            stuck_timer: self.stuck_timer,
            hit_alert_timer: self.hit_alert_timer,
            aim_settle: self.aim_settle,
            fire_timer: self.fire_timer,
            wander_pocketed: self.wander_pocketed,
            last_move_dir: self.last_move_dir.map(Dir::name),
            progress_px_s: self.progress_avg,
            last_action: self.last_action,
            wall_ahead_timer: self.wall_ahead_timer,
            breaching: self.breach.map(|b| b.dir.name()),
            intent_move: self.last_intent.move_dir.map(Dir::name),
            intent_face: self.last_intent.face.map(Dir::name),
            intent_fire: self.last_intent.fire,
            retarget_timer: self.retarget_timer,
            breach_timer: self.breach.map(|b| b.timer),
            escapes: self.escapes,
            target_player: self.target_player,
            shot_at_seat: self.shot_at_seat,
            field_alert: self.field.alert.map(|p| (p.x, p.y)),
            home: self.field.home.map(|p| (p.x, p.y)),
            called: self.field.called,
            awake: self.field.awake,
        }
    }

    /// The intent the last `think` produced - what a far tank keeps
    /// driving between thinks (`simulation::field`).
    pub(crate) fn last_intent(&self) -> Intent {
        self.last_intent
    }

    /// Which human player this tank is fighting - see the field.
    pub fn target_player(&self) -> u8 {
        self.target_player
    }

    /// The seat the last tick's trigger pull was aimed at - see the field.
    pub fn shot_at_seat(&self) -> Option<u8> {
        self.shot_at_seat
    }

    /// Point this tank at the other player (`enemy_phase`'s retarget pass).
    pub(crate) fn set_target_player(&mut self, player: u8) {
        self.target_player = player;
    }

    /// Called from `simulation.rs`'s shell-hit resolution whenever a shell
    /// damages this tank without killing it - see `ENEMY_HIT_ALERT_SECONDS`
    /// for why this exists. Refreshes (rather than adds to) the timer, same
    /// "how long since last contact" convention as `ENEMY_ALERT_HOLD_SECONDS`,
    /// so a tank taking sustained fire just stays alert continuously instead
    /// of the timer stacking up.
    pub fn notify_hit(&mut self) {
        self.hit_alert_timer = tuning().enemy_hit_alert_seconds;
    }

    /// The hit came from the tower standing at `at`: hold it against that
    /// tower for `enemy_tower_grudge_seconds`, and fire back whenever it is
    /// lined up and in sight (docs/defence-towers-prd.md section 9).
    pub(crate) fn notify_tower_hit(&mut self, at: Position) {
        self.notify_hit();
        self.grudge = Some(Grudge { at, timer: tuning().enemy_tower_grudge_seconds, in_sight: false });
    }

    /// Where the tower this tank holds a grudge against stands, if any.
    pub(crate) fn grudge_target(&self) -> Option<Position> {
        self.grudge.map(|g| g.at)
    }

    /// This frame's view of the grudge tower: `Some(sight)` while it
    /// stands, `None` once it is gone, which drops the grudge.
    pub(crate) fn set_grudge_sight(&mut self, sight: Option<bool>) {
        match (sight, &mut self.grudge) {
            (Some(in_sight), Some(grudge)) => grudge.in_sight = in_sight,
            _ => self.grudge = None,
        }
    }

    /// The tank was just moved through a portal (`Game::portal_phase`).
    /// Everything `think` measured at the old position is void: the
    /// heading commitment pointed at the entrance, the stuck clock's
    /// baseline is a screen away, the waypoint and any breach belong to
    /// the room it left. Clearing them makes the next tick re-plan from
    /// where it stands. Alertness, retreat state, the fire timer, the
    /// escape count and the target player are about the fight, not the
    /// place, and stay.
    pub(crate) fn on_teleported(&mut self) {
        self.retarget_timer = 0.0;
        self.committed_dir = None;
        self.dir_hold = 0.0;
        self.dodge_dir = None;
        self.dodge_timer = 0.0;
        self.yield_timer = 0.0;
        self.last_move_dir = None;
        self.last_position = None;
        self.motion = Vec2::new(0.0, 0.0);
        self.progress_avg = None;
        self.stuck_timer = 0.0;
        self.wander_pocketed = false;
        self.wall_ahead_timer = 0.0;
        self.breach = None;
    }

    /// Choose a heading toward `target` - or, if pathfinding can't reach
    /// `target` at all, toward a local fallback waypoint instead (see
    /// `wander`), so this always returns a real heading. `margin` is how
    /// far from the battlefield edge a fallback waypoint may land (see
    /// `wander`) - unused when `target` is directly reachable.
    ///
    /// `next_step` returns `None` for two very different reasons: no route
    /// exists at all, or `from`/`target` already share a grid cell (see its
    /// own `start == goal` check) - i.e. "arrived, nothing left to route"
    /// rather than "unreachable". At PATHFIND_CELL_SIZE=48px that second
    /// case fires constantly during ordinary close-range maneuvering (found
    /// via the probe harness: treating every `None` as unreachable made the
    /// fallback below trigger on almost every `steer` call once an enemy
    /// was near attack range, not just against a genuinely sealed
    /// obstruction) - `same_cell` is what tells the two apart.
    ///
    /// Genuinely no route existing is most commonly the player holed up
    /// inside their own sealed fortress (see
    /// `battlefield::spawn_player_fortress`'s GLYPH_O: a closed ring with no
    /// door by design - the player is meant to shoot their own way out, not
    /// be walked in on; even a shot-open tile is usually still narrower
    /// than a tank's own pathfinding clearance margin, so the fortress
    /// reads as permanently sealed to the AI in practice, not just at round
    /// start) - and since the fortress sits at the map's center for the
    /// *entire* round, not just at spawn, this isn't a rare edge case: it's
    /// the common state whenever the player stops moving somewhere
    /// pathfinding can't reach. Two earlier fixes tried here: falling back
    /// to the raw unreachable `target` (the tank commits to ramming
    /// whichever wall tile sits on that line, forever, since every future
    /// tick recomputes the same straight-line heading at the same
    /// unreachable point), and orbiting perpendicular to the obstruction in
    /// place (did get it firing again - aim alignment is computed
    /// independently of movement, see `Brain::aim_alignment`, so an
    /// orbiting tank still gets lucky alignment sometimes - but the orbit
    /// had no real destination, so over a long enough unreachable stretch,
    /// which a sealed fortress guarantees, it regularly walked itself into
    /// the battlefield boundary instead and got stuck there; a later
    /// attempt made it hold position entirely instead of orbiting, which
    /// fixed *that* but meant every enemy with no path to a stationary
    /// player just froze solid, indefinitely, the moment the player
    /// stopped moving - trading one visible "stuck" complaint for another).
    /// `wander` is the fix that stuck: reuse patrol's own bounded,
    /// already-proven-safe waypoint system instead of inventing a new
    /// movement heuristic, so the tank still gets the lucky-alignment
    /// upside without either failure mode.
    #[allow(clippy::too_many_arguments)] // perception is passed by value, not bundled
    fn steer(
        &mut self,
        from: Position,
        target: Position,
        bounds: (f32, f32),
        half: f32,
        margin: f32,
        ctx: AvoidCtx,
        grid: &Grid,
        rng: &mut SmallRng,
    ) -> Dir {
        let heading = self.route_heading();
        let route = if ctx.on_portal_cooldown { grid.route_ahead_walking(from, target) } else { grid.route_ahead(from, target, heading) };
        if route.is_none() && !grid.same_cell(from, target) {
            return self.wander(from, bounds, half, margin, None, ctx, grid, rng);
        }
        self.steer_toward(from, route, target, bounds, half, ctx, grid)
    }

    /// Wander toward a roaming local waypoint - `act_patrol`'s own
    /// top-level behavior (via `Brain::wander`) when the player's out of
    /// view entirely, and also `steer`'s fallback whenever the real target
    /// it was asked for has no path to it at all (see `steer`'s doc
    /// comment for why that's common, not rare). A tank that can't get to
    /// where it actually wants to be might as well patrol nearby instead of
    /// freezing solid until the target happens to wander somewhere
    /// reachable again.
    ///
    /// Resamples `waypoint` when it's reached, when `ENEMY_RETARGET_SECONDS`
    /// elapses, or the moment the *current* waypoint itself turns out
    /// unreachable (checked fresh every call - a round's static obstacle
    /// layout only ever gains reachable area as things get destroyed, never
    /// loses it, so a bad pick won't fix itself without a resample). That
    /// last check is the one thing this adds beyond plain patrol's original
    /// behavior: patrol's own waypoints were always low-stakes (only picked
    /// once the player's out of view range entirely), but a bad pick here
    /// would otherwise stall an actively-engaging tank for up to the full
    /// retarget interval right as the player's watching - a fresh pick
    /// avoids that without needing its own bounded-retry loop, since
    /// picking again next frame (not gated behind the timer, unlike the
    /// "reached"/"timer expired" cases) converges within a handful of
    /// frames given how much of the battlefield is normally open ground.
    ///
    /// `leash` confines the waypoint to a guard's beat around its frog (see
    /// `Leash`): candidates are sampled from the beat's bounding box
    /// (clipped to the margin) and kept only inside the annulus, and a
    /// waypoint that falls outside it - left over from an unleashed wander,
    /// or overtaken by a frog hop - is resampled at once.
    #[allow(clippy::too_many_arguments)] // perception is passed by value, not bundled
    fn wander(
        &mut self,
        from: Position,
        bounds: (f32, f32),
        half: f32,
        margin: f32,
        leash: Option<Leash>,
        ctx: AvoidCtx,
        grid: &Grid,
        rng: &mut SmallRng,
    ) -> Dir {
        let (width, height) = bounds;
        // A label read, not a search: a candidate is a point nobody is
        // routing to yet, so the grid's flood fill answers it for free.
        // The labels join rooms through the portal hub, so a tank on its
        // portal cooldown (which may not route through the hub) asks the
        // walking search instead - a few searches per resample, only
        // while the cooldown runs.
        let reachable = |wp: Position| {
            if ctx.on_portal_cooldown {
                grid.same_cell(from, wp) || grid.next_step_walking(from, wp).is_some()
            } else {
                grid.connected(from, wp)
            }
        };
        // The sampling box: the whole margin-inset battlefield, or the
        // beat's bounding box clipped to it (never empty - a beat pressed
        // against the edge still yields a sliver).
        let (x_range, y_range) = match leash {
            None => (margin..(width - margin), margin..(height - margin)),
            Some(Leash { anchor, radius, .. }) => {
                let x0 = (anchor.x - radius).max(margin);
                let y0 = (anchor.y - radius).max(margin);
                let x1 = (anchor.x + radius).min(width - margin).max(x0 + 1.0);
                let y1 = (anchor.y + radius).min(height - margin).max(y0 + 1.0);
                (x0..x1, y0..y1)
            }
        };
        let inside_leash = |p: Position| leash.is_none_or(|l| l.contains(p));
        // Only resample early over unreachability if a *different* waypoint
        // could plausibly do better. When `from` itself is boxed in (see
        // `Grid::boxed_in`), every candidate fails identically no matter
        // how many times this re-rolls - without this guard, that meant a
        // brand new random waypoint (pointing some new random direction)
        // every single frame, which is what "the tank span in place" was:
        // not a bug in the direction-commitment logic, a fresh target
        // defeating it every tick. Boxed-in tanks still get a fresh
        // roll on the normal ENEMY_RETARGET_SECONDS cadence (below, via
        // `retarget_timer`) in case circumstances change (an obstacle
        // burns away), just not every frame.
        let stuck_here = grid.boxed_in(from);
        // `wander_pocketed` suppresses the third, unreachability-triggered
        // resample the same way `stuck_here` does, for the same reason at a
        // different scale - see the field's own doc comment.
        if self.retarget_timer <= 0.0
            || from.distance_to(self.waypoint) < margin
            || !inside_leash(self.waypoint)
            || (!stuck_here && !self.wander_pocketed && !reachable(self.waypoint))
        {
            // Roll WANDER_SPREAD_CANDIDATES points and keep whichever is
            // both reachable and farthest from every other live tank
            // (`ctx.movers`, skipping this tank's own slot), rather than
            // committing to the first random point - see
            // WANDER_SPREAD_CANDIDATES's own doc comment for why plain
            // uniform sampling wasn't enough: independent wandering
            // enemies kept landing in the same small pathfinding-reachable
            // pocket with no awareness of each other, reading as tanks
            // clumping together (found via the probe harness: several
            // enemies parked within a ~100px box for seconds, nowhere near
            // the player). Falls back to any random point, reachable or
            // not, only if every single candidate this pass failed
            // reachability - matches the old single-roll behavior in that
            // rare worst case, still self-correcting next frame via the
            // `!reachable` check above.
            let mut best: Option<(Position, f32)> = None;
            for _ in 0..tuning().wander_spread_candidates {
                let candidate = Position::new(
                    rng.random_range(x_range.clone()),
                    rng.random_range(y_range.clone()),
                );
                if !inside_leash(candidate) || !reachable(candidate) {
                    continue;
                }
                let spread = ctx
                    .movers
                    .iter()
                    .enumerate()
                    .filter(|&(i, _)| i != ctx.my_index)
                    .map(|(_, m)| candidate.distance_to(m.position))
                    .fold(f32::INFINITY, f32::min);
                if best.is_none_or(|(_, best_spread)| spread > best_spread) {
                    best = Some((candidate, spread));
                }
            }
            // Latch (or clear) the pocket state off this pass's outcome:
            // empty-handed means the fallback waypoint below is known
            // unreachable, and re-rolling before the normal retarget
            // cadence would just spin the tank - see `wander_pocketed`.
            self.wander_pocketed = best.is_none();
            self.waypoint = best.map(|(candidate, _)| candidate).unwrap_or_else(|| {
                Position::new(rng.random_range(x_range.clone()), rng.random_range(y_range.clone()))
            });
            self.retarget_timer = tuning().enemy_retarget_seconds;
        }
        let heading = self.route_heading();
        let route =
            if ctx.on_portal_cooldown { grid.route_ahead_walking(from, self.waypoint) } else { grid.route_ahead(from, self.waypoint, heading) };
        self.steer_toward(from, route, self.waypoint, bounds, half, ctx, grid)
    }

    /// Whether this tank reads its route as lanes (`lane_turn`): on a field
    /// map - the one place a tank keeps a home (`FieldMind::home`, set the
    /// first tick it stands there) - while `ai_lane_turns` is on. An arena
    /// steers by the switch margin alone, as it always has.
    fn lanes(&self) -> bool {
        self.field.home.is_some() && tuning().ai_lane_turns
    }

    /// The heading a route is straightened along (`Grid::route_ahead`):
    /// the one held, while the route is read as lanes.
    fn route_heading(&self) -> Option<Position> {
        self.committed_dir.filter(|_| self.lanes()).map(Dir::vec)
    }

    /// Shared point-convergence core for both `steer` (chasing/fleeing/etc.
    /// a real target) and `wander` (patrolling a fallback waypoint) once
    /// the target is known-reachable (or "same cell", i.e. arrived): the
    /// route read as lanes (`lane_turn`), commitment/hold-margin hysteresis
    /// for everything else, the obstacle-ahead override, the stuck-escape
    /// safety net, and predictive collision dodging. `route` is
    /// `grid.route_ahead(from, target)`, already computed by the caller (it
    /// needed the result anyway, to decide whether to route here or to
    /// `wander` in `target`'s place).
    #[allow(clippy::too_many_arguments)] // perception is passed by value, not bundled
    fn steer_toward(
        &mut self,
        from: Position,
        route: Option<RouteAhead>,
        target: Position,
        bounds: (f32, f32),
        half: f32,
        ctx: AvoidCtx,
        grid: &Grid,
    ) -> Dir {
        let routed = route.map_or(target, |r| r.first());
        let fresh = Dir::toward(from, routed);
        // The route as lanes, against the heading held: `Some` while it
        // walks whole cells from the hull's own (see `lane_turn`).
        let lane = match (self.committed_dir, route) {
            (Some(heading), Some(route)) if self.lanes() => self.lane_turn(from, heading, &route, ctx, grid),
            _ => None,
        };
        // Continuing on the committed heading would walk into a cell the
        // grid already knows is blocked. That's a hard geometric fact, not
        // the wobbling-live-target case the hold/margin gate below exists
        // to filter out, so it doesn't need AI_DIR_HOLD_SECONDS's full wait
        // or AI_DIR_SWITCH_MARGIN_PX's off-axis-improvement bar - but it
        // still needs *some* dwell time (AI_OBSTACLE_OVERRIDE_HOLD_SECONDS,
        // much shorter): the coarse grid's routed direction can itself
        // wobble by a cell frame-to-frame near a corner, and reacting to
        // every single such wobble with an instant switch reintroduces the
        // very jitter commitment exists to prevent, just obstacle-triggered
        // instead of diagonal-target-triggered (see that constant's own
        // comment - found via the probe harness's `--rounds` sweep).
        let obstacle_ahead =
            self.dir_hold >= tuning().ai_obstacle_override_hold_seconds && self.walks_into_wall(from, ctx, grid);

        // A due turn waits out the same hold as any other switch, so a
        // route that may step either way holds each leg that long rather
        // than turning on every cell.
        let turn = match lane {
            Some(Lane::Turn(turn)) if self.dir_hold >= tuning().ai_dir_hold_seconds => Some(turn),
            _ => None,
        };

        let dir = if self.stuck_timer >= tuning().stuck_escape_seconds {
            // Asked to move for STUCK_ESCAPE_SECONDS running and made no
            // headway that way (see `think`'s progress tracking) - force a
            // hard reset instead of letting a bad commitment call wedge the
            // tank forever. This is the reactive safety net for a target
            // that's technically reachable but still jamming in practice (a
            // tight, contested squeeze between other tanks, say) - outright
            // unreachability is `steer`'s job, one level up, to hand off to
            // `wander` before this function ever sees it. Turn away from
            // whatever heading has actually been failing (`committed_dir`)
            // rather than retrying the same pathfind toward the same
            // target: a perpendicular first (side chosen by index parity so
            // two jammed tanks don't mirror each other), the other
            // perpendicular if that one drives into a wall or blocked cell,
            // and straight back out as the last resort - a tank pinned in a
            // corner has nowhere else to go.
            self.stuck_timer = 0.0;
            self.escapes += 1;
            let failing = self.committed_dir.unwrap_or(fresh);
            let left = ctx.my_index.is_multiple_of(2);
            let blocked =
                |d: Dir| heads_into_wall(d, from, bounds, half) || grid.blocked_ahead(from, d.vec());
            [perpendicular(failing, left), perpendicular(failing, !left), opposite(failing)]
                .into_iter()
                .find(|&d| !blocked(d))
                .unwrap_or_else(|| perpendicular(failing, left))
        } else {
            match (self.committed_dir, turn) {
                (None, _) => {
                    self.commit(fresh);
                    fresh
                }
                (Some(_), _) if obstacle_ahead => fresh,
                (Some(_), Some(turn)) => turn,
                // On its lane, short of its turn or with none in sight: on.
                (Some(committed), None) if lane.is_some() => committed,
                (Some(committed), None) if self.dir_hold < tuning().ai_dir_hold_seconds => {
                    // Not held long enough yet: stick with the current heading.
                    committed
                }
                (Some(committed), None) => {
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
                    // Deliberately blind to a straight reversal: Left and
                    // Right leave the same perpendicular error, so a routed
                    // point directly *behind* the tank never wins here and
                    // the tank keeps going until the obstacle override or
                    // the stuck escape intervenes. Letting a routed point
                    // behind the tank flip it was tried and rejected:
                    // `act_chase` never stops at its slot, so a tank
                    // overshoots by a whole hold period, flips, overshoots
                    // the other way, and reads as jitter (a 30-round
                    // default-map sweep went from jitter=6 to jitter=30).
                    if fresh != committed && committed_off - fresh_off > tuning().ai_dir_switch_margin_px {
                        fresh
                    } else {
                        committed
                    }
                }
            }
        };

        // Sidestep an imminent collision, then commit the final heading so it
        // holds. Driving into a wall is no longer steering's problem - the
        // physics engine's wall colliders stop/slide the tank for real.
        let dir = self.avoid_collisions(dir, from, bounds, half, ctx, grid);
        self.commit(dir);
        dir
    }

    /// Whether driving on along the heading held walks into a cell the
    /// grid knows is blocked (`steer_toward`'s obstacle-ahead override).
    /// On a field map it is judged from where the slide across that
    /// heading will leave the hull rather than from its centre: a lane
    /// turn comes the slide's length before its turning (`lane_turn`), so
    /// for a moment after it the centre still stands in the lane the hull
    /// is leaving, whose cell ahead can be the very wall the turn was
    /// timed to clear; judged from the centre, a hull entering a one-lane
    /// passage would be thrown back and forth across its mouth.
    fn walks_into_wall(&self, from: Position, ctx: AvoidCtx, grid: &Grid) -> bool {
        self.committed_dir.is_some_and(|heading| {
            let at = if self.lanes() {
                // The motion across the heading, and the signed slide it
                // carries the hull on for.
                let ahead = heading.vec();
                let across = Vec2::new(self.motion.x * ahead.y.abs(), self.motion.y * ahead.x.abs());
                let slide = |v: f32| v * v.abs() / (2.0 * ctx.grip.max(1.0));
                Position::new(from.x + slide(across.x), from.y + slide(across.y))
            } else {
                from
            };
            grid.blocked_ahead(at, heading.vec())
        })
    }

    /// The route read as lanes - the rows and columns of nav cells a hull
    /// drives along - against `heading`, the heading the hull holds.
    ///
    /// A hull does not turn on the spot: whatever it is told, its tracks
    /// only scrub off the speed along its old heading at their grip
    /// (`tank_turn_grip_force` over its mass), so it slides on for
    /// `v^2 / 2a` after it turns - four fifths of a cell for an assault at
    /// an enemy's pace, more than two for the heaviest hull at its fastest.
    /// Judged by the switch margin alone, a turn waits until the hull is
    /// well into the turning, and the slide then carries it past the new
    /// lane's centre line to its far edge, closer to the next row's centre
    /// than `ai_dir_switch_margin_px`: no step into that row can ever beat
    /// the margin again, and a hull whose route turns that way drives past
    /// every turning, to and fro, for the rest of the round
    /// (docs/large-maps-follow-camera.md section 12). Here the turn comes
    /// where the slide ends on the centre line of the cell the route turns
    /// in - the hull's rest point, its position plus the slide, reaching
    /// that line on the think nearest the crossing - so a hull drives its
    /// lanes on their centre lines and the hull's place across its lane
    /// plays no part in when it turns.
    ///
    /// `Lane::Turn` when the route turns across `heading` and that moment has
    /// come; `Lane::Hold` while the route runs on along `heading` as far as
    /// it reads, turns further on, or turns where the hull comes too late for
    /// its slide to end within half a cell of the line (the route from
    /// further on decides then - never so strictly that the cell it turns in
    /// offers the hull no think at all); `None` when the route is no walk
    /// along lanes from here and the margin decides - its first step is
    /// straight back (blind to a reversal, as ever) or out through a portal,
    /// or it was searched rather than read from a flow field
    /// (`RouteAhead::shared`) and the margin can still turn the hull onto it:
    /// a search's path is one of many as cheap toward a target of the tank's
    /// own that moves, an engagement slot most of all, and read as lanes it
    /// crowds a level's corridors (docs/large-maps-follow-camera.md
    /// section 12). Where the margin never can (`margin_never_turns`: the
    /// hull rides the edge of its lane on the side the route turns to, or so
    /// near it that the margin's window is narrower than the ground the hull
    /// covers between two thinks), a searched route's turn is a lane turn
    /// too - that is the hull this is for, wherever its route comes from. A
    /// turn taken before the hull is in the cell it turns in sweeps the cells
    /// beside the ones it still crosses, so it waits for that cell while any
    /// of them is blocked: the nav grid keeps a hull's centre clear of the
    /// walls there.
    fn lane_turn(&self, from: Position, heading: Dir, route: &RouteAhead, ctx: AvoidCtx, grid: &Grid) -> Option<Lane> {
        let along = |p: Vec2| p.x * heading.vec().x + p.y * heading.vec().y;
        let v = along(self.motion);
        // What the hull covers along its heading until the next think.
        let stride = v.abs() * ctx.dt;
        if !route.shared() && !margin_never_turns(from, heading, route, stride) {
            return None;
        }
        let mut at = route.start();
        for (i, &next) in route.cells().iter().enumerate() {
            let step = cell_step(at, next)?;
            if step == heading {
                at = next;
                continue;
            }
            if step == opposite(heading) {
                return if i == 0 { None } else { Some(Lane::Hold) };
            }
            let slide = v * v.abs() / (2.0 * ctx.grip.max(1.0));
            let (rest, line) = (along(from) + slide, along(route.centre(at)));
            // The think nearest the crossing: half of what the hull covers
            // until the next one either side of the line.
            if rest + 0.5 * stride < line {
                return Some(Lane::Hold);
            }
            // Too late to land in the turn's lane - the slide would end
            // more than half a cell past its line - and the turn is the
            // route's from further on. Never so tight that the cell it
            // turns in offers no think at all: a slide longer than a cell
            // reaches past the line from the moment the hull comes in.
            let half = route.cell_size() * 0.5;
            if rest > line + half.max(slide - half + stride) {
                return Some(Lane::Hold);
            }
            let side = |c: (usize, usize)| {
                let (dc, dr) = (step.vec().x as isize, step.vec().y as isize);
                let (col, row) = (c.0 as isize + dc, c.1 as isize + dr);
                col < 0 || row < 0 || grid.is_blocked(col as usize, row as usize)
            };
            let mut crossing = std::iter::once(route.start()).chain(route.cells()[..i].iter().copied()).take_while(|&c| c != at);
            if crossing.any(side) {
                return Some(Lane::Hold);
            }
            return Some(Lane::Turn(step));
        }
        Some(Lane::Hold)
    }

    /// Predictively sidestep a likely collision. Given the `desired` heading, look
    /// ahead along it and estimate the closest approach to every other tank; if a
    /// hit looks likely within AVOID_LOOKAHEAD, latch a perpendicular dodge for
    /// AVOID_DODGE_SECONDS and return it. When the dodge expires, normal steering
    /// resumes and pulls the tank back on course — the "away then back" motion.
    ///
    /// `grid` supplements the plain geometric `heads_into_wall` check with a
    /// grid-cell-accurate one (`Grid::blocked_ahead`) for both the battlefield
    /// boundary and static obstacles - without it, a dodge pick could walk
    /// toward the boundary in the narrow gap between the two checks'
    /// thresholds (`heads_into_wall`'s is a flat pixel skin from `from`'s
    /// exact position; the grid's is "which cell is this"), which
    /// `steer_toward`'s own obstacle-ahead override would then immediately
    /// veto next tick - `avoid_collisions` picks the same doomed dodge
    /// again the tick after that (the same nearby mover is usually still
    /// there), and so on: found as a sustained heading flip-flop with the
    /// tank pinned in one spot near a battlefield corner, via the probe
    /// harness's per-commit trace.
    fn avoid_collisions(
        &mut self,
        desired: Dir,
        from: Position,
        bounds: (f32, f32),
        half: f32,
        ctx: AvoidCtx,
        grid: &Grid,
    ) -> Dir {
        let blocked = |d: Dir| heads_into_wall(d, from, bounds, half) || grid.blocked_ahead(from, d.vec());
        // A dodge already in progress holds until its timer runs out (ticked in
        // `think`), as long as it isn't driving into a wall or obstacle.
        if let Some(dodge) = self.dodge_dir {
            if self.dodge_timer > 0.0 && !blocked(dodge) {
                return dodge;
            }
            self.dodge_dir = None;
        }

        // Too slow to meaningfully predict our own path: don't dodge.
        if ctx.speed < tuning().avoid_min_speed {
            return desired;
        }

        let step = desired.vec();
        let my_vel = Vec2::new(step.x * ctx.speed, step.y * ctx.speed);

        // Find the soonest predicted collision among the other movers.
        let mut soonest: Option<(f32, Vec2)> = None; // (time, relative position)
        for (i, other) in ctx.movers.iter().enumerate() {
            if i == ctx.my_index {
                continue;
            }
            let p = Vec2::new(other.position.x - from.x, other.position.y - from.y);
            let v = Vec2::new(my_vel.x - other.velocity.x, my_vel.y - other.velocity.y);
            let vv = v.x * v.x + v.y * v.y;
            if vv <= f32::EPSILON {
                continue; // no relative motion
            }
            // Already overlapping is the ram system's job, not ours.
            let sep_now = (p.x * p.x + p.y * p.y).sqrt();
            let reach = ctx.radius + other.radius + tuning().avoid_margin;
            if sep_now < reach {
                continue;
            }
            // Moving apart? (relative velocity points away) then no approach.
            let pv = p.x * v.x + p.y * v.y;
            if pv >= 0.0 {
                continue;
            }
            let t = (-pv / vv).clamp(0.0, tuning().avoid_lookahead);
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

        // Prefer a dodge side that is neither walled/obstructed nor itself
        // about to collide.
        let choice = [primary, secondary]
            .into_iter()
            .find(|&d| !blocked(d) && !self.dir_collides(d, from, ctx));
        // Fall back to any non-walled/obstructed side; if both fail, abandon the dodge.
        let choice = choice.or_else(|| [primary, secondary].into_iter().find(|&d| !blocked(d)));

        match choice {
            Some(dir) => {
                self.dodge_dir = Some(dir);
                self.dodge_timer = tuning().avoid_dodge_seconds;
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
        let my_vel = Vec2::new(step.x * ctx.speed, step.y * ctx.speed);
        for (i, other) in ctx.movers.iter().enumerate() {
            if i == ctx.my_index {
                continue;
            }
            let p = Vec2::new(other.position.x - from.x, other.position.y - from.y);
            let v = Vec2::new(my_vel.x - other.velocity.x, my_vel.y - other.velocity.y);
            let vv = v.x * v.x + v.y * v.y;
            if vv <= f32::EPSILON {
                continue;
            }
            let pv = p.x * v.x + p.y * v.y;
            if pv >= 0.0 {
                continue;
            }
            let t = (-pv / vv).clamp(0.0, tuning().avoid_lookahead);
            let cx = p.x + v.x * t;
            let cy = p.y + v.y * t;
            let closest = (cx * cx + cy * cy).sqrt();
            if closest < ctx.radius + other.radius + tuning().avoid_margin {
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
        if ammo <= tuning().enemy_ammo_low {
            self.retreating = true;
        } else if ammo >= tuning().enemy_ammo_resume {
            self.retreating = false;
        }
        self.retreating
    }

    /// True while backing off to recharge ammo (see `wants_retreat`). Read
    /// by the `stats` debug overlay (`game.rs::draw_tank_stats`) and by `Game::update`'s
    /// engagement-slot assignment, which excludes a retreating tank from the
    /// engaged set - it isn't attacking, so it shouldn't consume a slot.
    pub fn is_retreating(&self) -> bool {
        self.retreating
    }

    /// True while this tank is still reacting to a recent hit (see
    /// `notify_hit`/`ENEMY_HIT_ALERT_SECONDS`) - used by `Game::update`'s
    /// engagement-slot assignment so a hit-alerted tank outside normal view
    /// range is still routed through the same spread-slot system instead of
    /// falling back to the raw player position.
    pub fn is_hit_alerted(&self) -> bool {
        self.hit_alert_timer > 0.0
    }

    /// Seconds until this tank may fire again (zero or negative means
    /// ready). Read by the `stats` debug overlay (`game.rs::draw_tank_stats`).
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
pub(crate) fn perpendicular(dir: Dir, left: bool) -> Dir {
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

/// The reverse of `dir` - the heading a stuck tank backs out along once
/// both perpendiculars are blocked.
pub(crate) fn opposite(dir: Dir) -> Dir {
    match dir {
        Dir::Up => Dir::Down,
        Dir::Down => Dir::Up,
        Dir::Left => Dir::Right,
        Dir::Right => Dir::Left,
    }
}

/// What the route read as lanes tells a hull (`Ai::lane_turn`).
#[derive(Clone, Copy, Debug, PartialEq)]
enum Lane {
    /// Keep to the heading: the route runs on along it, or turns further on.
    Hold,
    /// Turn this way now: the hull's slide through the turn ends on the
    /// centre line of the cell the route turns in.
    Turn(Dir),
}

/// Whether `steer_toward`'s switch margin can never turn a hull at `from`,
/// heading `heading` and covering `stride` px along it between two thinks,
/// onto the first step of `route`, a step across the heading. The margin
/// turns a hull on a think where its error across the heading, to the
/// step's cell, beats its error along it by `ai_dir_switch_margin_px`, and
/// driving on never changes the error across. A hull nearer the step's
/// centre line than the margin - riding the edge of its own lane on that
/// side - never turns; one a little farther turns only on a think whose
/// error along is under what its error across beats the margin by, a
/// window twice that wide about the turning's centre, and a window
/// narrower than `stride` can fall between two thinks - on a lane a hull
/// drives to and fro at one pace, pass after pass (`Ai::lane_turn`).
fn margin_never_turns(from: Position, heading: Dir, route: &RouteAhead, stride: f32) -> bool {
    let Some(&next) = route.cells().first() else { return false };
    match cell_step(route.start(), next) {
        Some(step) if step != heading && step != opposite(heading) => {
            let across = |p: Vec2| p.x * step.vec().x + p.y * step.vec().y;
            across(route.centre(next)) - across(from) - tuning().ai_dir_switch_margin_px <= 0.5 * stride
        }
        _ => false,
    }
}

/// The heading of one route step between two side-by-side cells, `None`
/// for any other pair (a portal's exit).
fn cell_step(from: (usize, usize), to: (usize, usize)) -> Option<Dir> {
    match (to.0 as isize - from.0 as isize, to.1 as isize - from.1 as isize) {
        (1, 0) => Some(Dir::Right),
        (-1, 0) => Some(Dir::Left),
        (0, 1) => Some(Dir::Down),
        (0, -1) => Some(Dir::Up),
        _ => None,
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
    /// How hard its tracks scrub off sideways speed (px/s^2):
    /// `tank_turn_grip_force` over its mass - what sets its slide through
    /// a turn (`Ai::lane_turn`).
    grip: f32,
    /// The seconds this think covers, about the time to the next one.
    dt: f32,
    /// This tank's portal cooldown is running (`Tank::portal_cooldown`),
    /// so it routes on foot only (`Grid::next_step_walking`): a route
    /// through the hub would walk it back onto the portal it just came out
    /// of, where the cooldown stands still, and it would circle the
    /// footprint until it left and came back. With no walking route it
    /// wanders instead, and takes the portal once the cooldown is out.
    on_portal_cooldown: bool,
}

/// The behavior-tree blackboard: transient per-frame perception plus references
/// to the enemy's persistent memory and the output intent the tree fills in.
struct Brain<'a> {
    me: &'a Tank,
    player: &'a Tank,
    /// What this tank fights - see `think`'s `target` parameter.
    target: Position,
    /// The frog this role is anchored to - see `think`'s `frog_target`.
    frog_target: Option<Position>,
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
    rng: &'a mut SmallRng,
    ai: &'a mut Ai,
    intent: Intent,
    /// Last known player position shared across every enemy this round,
    /// while any one of them currently has the player within
    /// its sight - see `think`'s `alert` parameter and
    /// `act_patrol`. `None` when no enemy has spotted the player recently.
    alert: Option<Position>,
    /// This tank's assigned spot on the shared engagement ring around the
    /// player - see `think`'s `engage_target` parameter. `None` when this
    /// tank is the only one currently engaged (or the player's dead), in
    /// which case `engage_point` falls back to the raw player position.
    engage_target: Option<Position>,
    /// Every currently-live pickup on the battlefield - see `think`'s
    /// `pickups` parameter and `nearest_pickup`.
    pickups: &'a [(PickupKind, Position)],
    /// Whether this tank's straight line to `target` is currently clear
    /// of terrain - see `think`'s `line_of_sight` parameter.
    line_of_sight: bool,
    /// The same test toward the player - see `think`'s
    /// `player_line_of_sight` parameter.
    player_line_of_sight: bool,
    /// Whether the target is hidden from *this* tank by tall grass
    /// (`Terrain::conceals` plus `grass_reveal_range`). Separate from
    /// `line_of_sight` on purpose: that one also counts walls, and a tank
    /// that stopped chasing whenever a wall came between it and the player
    /// would never path around anything. This one only ever means "lost in
    /// cover", so it can gate the chase tier without touching navigation.
    target_concealed: bool,
    /// The tile directly ahead in each direction - see `think`'s
    /// `walls_ahead` parameter.
    walls_ahead: [Option<WallAhead>; 4],
    /// How far this tank sees - see `think`'s `sight` parameter.
    sight: f32,
}

impl<'a> Brain<'a> {
    /// This tank as `Ai::steer` and `Ai::wander` see it: the motion
    /// snapshot, its collision radius (`Tank::avoidance_radius` - a safe
    /// over-approximation of the tank's real, per-row physics collider),
    /// its top speed and its tracks' grip, this think's span and its
    /// portal cooldown.
    fn avoid_ctx(&self) -> AvoidCtx<'a> {
        AvoidCtx {
            movers: self.movers,
            my_index: self.my_index,
            radius: self.me.avoidance_radius(),
            speed: self.me.effective_speed(),
            grip: tuning().tank_turn_grip_force / self.me.mass(),
            dt: self.dt,
            on_portal_cooldown: self.me.portal_cooldown > 0.0,
        }
    }
}

impl Brain<'_> {
    /// `enemy_attack_range`, never past what this tank sees: in a fog
    /// thick enough to hide a target inside it, the attack waits until the
    /// target shows.
    fn attack_range(&self) -> f32 {
        tuning().enemy_attack_range.min(self.sight)
    }

    fn dist_to_player(&self) -> f32 {
        self.me.position.distance_to(self.player.position)
    }

    fn dist_to_target(&self) -> f32 {
        self.me.position.distance_to(self.target)
    }

    /// Where to steer when closing in on/repositioning around the target -
    /// this tank's engagement-ring slot if it has one, otherwise the
    /// target's exact position. Used by `act_chase` and `act_attack`'s
    /// reposition branch; firing/aim (`aim_alignment`) always targets the
    /// real target regardless, so spreading out changes where a tank walks,
    /// never what it shoots at.
    fn engage_point(&self) -> Position {
        self.engage_target.unwrap_or(self.target)
    }

    fn player_alive(&self) -> bool {
        !self.player.is_wreck()
    }

    /// Whether `target` is still worth fighting: a hunter's frog while it
    /// lives (`frog_target` is `Some`), otherwise the player.
    fn target_alive(&self) -> bool {
        self.hunting_frog() || self.player_alive()
    }

    /// True while this tank is a hunter whose quarry is alive - i.e. while
    /// `target` is the player's frog rather than the player.
    fn hunting_frog(&self) -> bool {
        self.ai.role == Role::Hunter && self.frog_target.is_some()
    }

    /// The guard's beat around its own frog for a guard whose frog is
    /// alive, `None` for every other role. The outer radius is
    /// `guard_leash_px` less a hull, so a waypoint on the rim never carries
    /// the hull over the line; the inner one `guard_keep_off_px`, capped
    /// so the annulus never closes.
    fn leash(&self) -> Option<Leash> {
        if self.ai.role != Role::Guard {
            return None;
        }
        let anchor = self.frog_target?;
        let radius = (tuning().guard_leash_px - self.me.size()).max(self.me.size());
        let keep_off = tuning().guard_keep_off_px.min(radius * 0.75);
        Some(Leash { anchor, radius, keep_off })
    }

    /// The leash around this tank's home on a field map
    /// (`FieldMind::home`, `enemy_leash_px` less a hull, so a waypoint on
    /// the rim never carries the hull over the line), `None` on an arena,
    /// where no home is ever kept. It binds a tank with nothing to fight:
    /// `act_patrol` without an alert, and the pickups it detours for.
    fn home_leash(&self) -> Option<Leash> {
        let anchor = self.ai.field.home?;
        let radius = (tuning().enemy_leash_px - self.me.size()).max(self.me.size());
        Some(Leash { anchor, radius, keep_off: 0.0 })
    }

    /// The nearest live pickup of `kind` worth a detour: any on an arena,
    /// only those inside the home leash on a field map
    /// (`home_leash`), so a tank with nothing to fight never crosses the
    /// map for one.
    fn seek(&self, kind: PickupKind) -> Option<Position> {
        match self.home_leash() {
            None => self.nearest_pickup(kind),
            Some(leash) => self
                .pickups
                .iter()
                .filter(|&&(k, at)| k == kind && at.distance_to(leash.anchor) <= leash.radius)
                .map(|&(_, at)| at)
                .min_by(|&a, &b| self.me.position.distance_to(a).total_cmp(&self.me.position.distance_to(b))),
        }
    }

    /// Whether a guard should hold its beat rather than fight: the player
    /// is dead or outside `guard_leash_px` of its frog.
    fn guard_holds(&self) -> bool {
        self.leash().is_some_and(|l| {
            !self.player_alive() || self.player.position.distance_to(l.anchor) > tuning().guard_leash_px
        })
    }

    /// Whether a hunter can take the opportunistic shot at the player this
    /// tick: the player is alive, within attack range, in clear sight,
    /// already lined up on one of this tank's firing axes - so no
    /// repositioning away from the frog is ever spent on it - with this
    /// tank inside the player's sight box (`may_fire_at_seat`), and the
    /// last snipe's `hunter_snipe_cooldown_seconds` have passed.
    fn can_snipe_player(&self) -> bool {
        if !self.hunting_frog() || !self.player_alive() || !self.player_line_of_sight || self.ai.snipe_cooldown > 0.0 {
            return false;
        }
        if self.dist_to_player() > self.attack_range() || !self.may_fire_at_seat() {
            return false;
        }
        let (_, off_axis, in_front) = self.aim_alignment_at(self.player.position);
        off_axis <= tuning().enemy_fire_align_px && in_front
    }

    /// Whether this tank stands where it may fire at the seat it fights
    /// (`player`): inside that seat's sight box (`in_sight_box`), the
    /// rule every enemy shot at a seat obeys, so nobody is shot from
    /// beyond the edge of their own screen
    /// (docs/large-maps-follow-camera.md section 5). Sideways the box
    /// reaches past `enemy_attack_range`, so a shot lined up on a row is
    /// never held back by it; up and down it is shorter, and a tank lined
    /// up on a column closes in before it fires.
    ///
    /// A training dummy (`Ai::frog_only`) never may.
    fn may_fire_at_seat(&self) -> bool {
        !self.ai.frog_only && in_sight_box(self.player.position, self.me.position)
    }

    /// This tank's own position to the nearest currently-live pickup of
    /// `kind`, if any exist right now - used by `act_flee`/`act_retreat` so
    /// a hurting or ammo-starved tank heads for a pickup instead of just
    /// running blind. `None` when no pickup of that kind is on the field
    /// this frame (already collected and still respawning - see
    /// PICKUP_RESPAWN_SECONDS), in which case those callers fall back to
    /// their old player-relative behavior.
    fn nearest_pickup(&self, kind: PickupKind) -> Option<Position> {
        self.pickups
            .iter()
            .filter(|(k, _)| *k == kind)
            .map(|&(_, pos)| pos)
            .min_by(|&a, &b| {
                self.me
                    .position
                    .distance_to(a)
                    .total_cmp(&self.me.position.distance_to(b))
            })
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
        // A held laser charge or minigun ammo costs no shells, so either one
        // counts as full ammo confidence for pacing purposes - same
        // reasoning as tier 3's retreat-skip above (`build`), just for
        // firing cadence instead of whether to retreat at all.
        let ammo_frac = if self.me.active_weapon() != ActiveWeapon::Shell {
            1.0
        } else {
            (self.me.shells_ammo as f32 / tuning().max_shells as f32).clamp(0.0, 1.0)
        };
        let health_frac = (1.0 - self.me.damage / MAX_DAMAGE).clamp(0.0, 1.0);
        let aggression = ammo_frac.min(health_frac);
        tuning().enemy_fire_interval - (tuning().enemy_fire_interval - tuning().enemy_fire_interval_aggressive) * aggression
    }

    /// Steer toward `target`, routing around static obstacles and
    /// sidestepping predicted collisions - or toward a fallback waypoint if
    /// `target` can't be reached at all (see `Ai::steer`), so this always
    /// returns a real heading, never "give up and hold". Wraps `Ai::steer`
    /// with this tank's bounds and collision radius (`Tank::avoidance_radius`
    /// - a safe over-approximation of the tank's real, per-row physics
    /// collider, so this never assumes a tank is smaller than it actually
    /// is), `Tank::size()` as the fallback waypoint's clearance margin from
    /// the battlefield edge (same margin `act_patrol` always sampled
    /// within), the motion snapshot for avoidance, and this frame's
    /// obstacle grid.
    fn steer(&mut self, target: Position) -> Dir {
        let ctx = self.avoid_ctx();
        let radius = ctx.radius;
        self.ai.steer(
            self.me.position,
            target,
            (self.width, self.height),
            radius,
            self.me.size(),
            ctx,
            self.grid,
            self.rng,
        )
    }

    /// Wander toward a roaming local waypoint with no particular target in
    /// mind - `act_patrol`'s own top-level behavior. Thin wrapper around
    /// `Ai::wander`, same shape as `steer` above.
    fn wander(&mut self) -> Dir {
        let ctx = self.avoid_ctx();
        let radius = ctx.radius;
        self.ai.wander(
            self.me.position,
            (self.width, self.height),
            radius,
            self.me.size(),
            None,
            ctx,
            self.grid,
            self.rng,
        )
    }

    /// `wander` confined to `leash` - a guard's beat.
    fn wander_within(&mut self, leash: Leash) -> Dir {
        let ctx = self.avoid_ctx();
        let radius = ctx.radius;
        self.ai.wander(
            self.me.position,
            (self.width, self.height),
            radius,
            self.me.size(),
            Some(leash),
            ctx,
            self.grid,
            self.rng,
        )
    }

    /// Perpendicular offset of the target from the firing axis toward it,
    /// and whether the target is actually in front (positive along the
    /// fire dir).
    fn aim_alignment(&self) -> (Dir, f32, bool) {
        self.aim_alignment_at(self.target)
    }

    /// `aim_alignment` toward an arbitrary point.
    fn aim_alignment_at(&self, at: Position) -> (Dir, f32, bool) {
        let dir = Dir::toward(self.me.position, at);
        let (off_axis, forward) = axis_offsets(self.me.position, at, dir);
        (dir, off_axis, forward > 0.0)
    }

    /// A shot back at the tower this tank holds a grudge against: the
    /// fire direction and range while the tower is in sight, within its
    /// own reach of the tank and lined up on an axis - the tower is a whole
    /// cell, so the alignment allows half of one on top of
    /// `enemy_fire_align_px`.
    fn grudge_shot(&self) -> Option<(Dir, f32)> {
        let grudge = self.ai.grudge?;
        if !grudge.in_sight {
            return None;
        }
        let t = tuning();
        let (dir, off_axis, in_front) = self.aim_alignment_at(grudge.at);
        let range = self.me.position.distance_to(grudge.at);
        let reach = t.enemy_attack_range.max(t.gun_tower_range).max(t.bio_range);
        (in_front && off_axis <= t.enemy_fire_align_px + OBSTACLE_GRID_SIZE * 0.5 && range <= reach).then_some((dir, range))
    }

    /// Hold position facing `fire_dir` and, once the aim has settled for
    /// `enemy_aim_settle` and the fire timer allows, shoot at the point
    /// `range` px ahead - the aligned half of `act_attack`, shared with the
    /// hunter's snipe and the grudge shot. Holds fire (mostly) when a
    /// teammate is in the way. `at_seat` is the seat the shot is aimed at,
    /// `None` for a frog or a tower; the caller has already held the tank
    /// to that seat's sight box (`may_fire_at_seat`), and a shot taken
    /// records it in `Ai::shot_at_seat`.
    fn hold_and_fire(&mut self, fire_dir: Dir, range: f32, at_seat: Option<u8>) {
        self.ai.aim_settle += self.dt;
        self.intent.face = Some(fire_dir);
        // Keep the committed heading in sync so leaving Attack doesn't snap.
        self.ai.commit(fire_dir);

        if self.ai.aim_settle >= tuning().enemy_aim_settle && self.ai.fire_timer <= 0.0 {
            let blocked = self.friendly_blocks_shot(fire_dir, range);
            let hold_fire =
                blocked && self.rng.random_range(0.0..1.0) < tuning().enemy_friendly_fire_hold_chance;
            // Whether it fires or holds, this firing opportunity is spent -
            // otherwise a held shot would just re-roll every frame at ~60Hz
            // and fire almost immediately anyway, defeating the hold chance.
            // The interval itself scales with ammo: fuller magazine, faster
            // follow-up shot (see Brain::fire_interval).
            self.ai.fire_timer = self.fire_interval();
            if !hold_fire {
                self.intent.fire = true;
                self.intent.fire_aim_offset = self.roll_misfire(range);
                self.ai.shot_at_seat = at_seat;
            }
        }
    }


    /// Whether to shoot through the tile ahead instead of steering around
    /// it. Latched into `Ai::breach` once the tank has spent
    /// `enemy_breach_after_seconds` driving into a destructible tile, and
    /// kept while that tile still stands, the give-up timer runs, and the
    /// tank can afford it: damage at or under `enemy_breach_max_damage`
    /// and, on shells, `enemy_breach_min_shells` in the rack (a special
    /// weapon spends no shells, so it always qualifies). Iron is never
    /// breached. The moment the tile is gone the latch clears and normal
    /// steering finds the fresh gap.
    fn wants_breach(&mut self) -> bool {
        let t = tuning();
        let can_afford = self.me.damage <= t.enemy_breach_max_damage
            && (self.me.active_weapon() != ActiveWeapon::Shell || self.me.shells_ammo >= t.enemy_breach_min_shells);
        let walls = self.walls_ahead;
        let wall_in = |dir: Dir| walls[dir.index()].filter(|w| !w.material.is_permanent());
        if let Some(breach) = self.ai.breach {
            if can_afford && breach.timer > 0.0 && wall_in(breach.dir).is_some() {
                return true;
            }
            self.ai.breach = None;
            return false;
        }
        if !can_afford || self.ai.wall_ahead_timer < t.enemy_breach_after_seconds {
            return false;
        }
        let Some(dir) = self.ai.last_move_dir else { return false };
        if wall_in(dir).is_none() {
            return false;
        }
        self.ai.breach = Some(Breach { dir, timer: t.enemy_breach_give_up_seconds });
        self.ai.wall_ahead_timer = 0.0;
        true
    }

    /// True if another enemy sits roughly on `fire_dir`'s line, closer than
    /// `max_forward` (normally the distance to the target) - i.e. firing
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
            if mover.is_player || i == self.my_index {
                return false;
            }
            let (off_axis, forward) = axis_offsets(self.me.position, mover.position, fire_dir);
            forward > 0.0 && forward < max_forward && off_axis <= tuning().enemy_fire_align_px
        })
    }
}

/// Perpendicular and forward distance of `to` from `from` along the cardinal
/// axis `dir` points along - shared by aim alignment (target: the player) and
/// friendly-fire avoidance (target: another enemy), so both read the same way.
/// Is something close enough *directly in front* that driving on would
/// slam into it?
///
/// Measured hull surface to hull surface (`Mover::radius` is the real
/// per-row footprint at the tank's current facing), against
/// `enemy_separation_px`.
///
/// Three conditions, and each one is load-bearing:
///
/// - **Ahead**, by the sign of the dot product with the heading. A tank
///   beside or behind is not in the way, and braking for one would have a
///   pair that is merely passing each other stop dead.
/// - **In the lane**, by perpendicular distance: only something the hull
///   would actually meet counts, not a tank sliding past a hull's width to
///   the side.
/// - **Inside the gap**, surface to surface, so a `titan` keeps the same
///   clear air as a `scout` rather than the same centre distance.
///
/// The player is in `movers` too and is treated no differently - the ask
/// was for tanks to stop short of the player as well, and an enemy that
/// noses up to the hull and holds reads far better than one that grinds
/// into it.
fn crowded_ahead(from: Position, dir: Dir, movers: &[Mover], my_index: usize) -> bool {
    let gap_wanted = tuning().enemy_separation_px;
    let Some(me) = movers.get(my_index) else { return false };
    let step = dir.vec();
    movers.iter().enumerate().any(|(i, other)| {
        if i == my_index {
            return false;
        }
        let (dx, dy) = (other.position.x - from.x, other.position.y - from.y);
        if dx * step.x + dy * step.y <= 0.0 {
            return false;
        }
        // `step` is a unit cardinal, so the 2D cross product is the
        // perpendicular distance outright.
        if (dx * step.y - dy * step.x).abs() > me.radius + other.radius {
            return false;
        }
        (dx * dx + dy * dy).sqrt() - me.radius - other.radius <= gap_wanted
    })
}

pub(crate) fn axis_offsets(from: Position, to: Position, dir: Dir) -> (f32, f32) {
    let dx = to.x - from.x;
    let dy = to.y - from.y;
    match dir {
        Dir::Up => (dx.abs(), -dy),
        Dir::Down => (dx.abs(), dy),
        Dir::Left => (dy.abs(), -dx),
        Dir::Right => (dy.abs(), dx),
    }
}

/// Whether `at` stands inside the sight box of the seat whose tank is at
/// `seat`: no further than the box's half extents
/// (`Tuning::sight_box_half_px`) from it on either axis, centre to centre,
/// the edge included. The one test every enemy fire decision at a seat
/// takes - a tank's attack and snipe (`Brain::may_fire_at_seat`), a seeker
/// missile's lock (`Game::guide_missiles`), an enemy tower's pick
/// (`simulation::towers`) - and the one the probe's `offbox-fire` check
/// holds them to (docs/large-maps-follow-camera.md section 5).
pub fn in_sight_box(seat: Position, at: Position) -> bool {
    in_sight_box_of(tuning().sight_box_half_px(), seat, at)
}

/// `in_sight_box` with the box's half extents read once by the caller
/// (`Tuning::sight_box_half_px`), for a loop over many points.
pub fn in_sight_box_of((half_w, half_h): (f32, f32), seat: Position, at: Position) -> bool {
    (at.x - seat.x).abs() <= half_w && (at.y - seat.y).abs() <= half_h
}

/// Build the enemy behavior tree. Priority (Selector) order, highest first:
///   1. Dead? do nothing.
///   2. Flee when badly hurt.
///   3. Retreat to recharge when ammo is low.
///   3.4. A tower that hurt it lined up in sight: fire back.
///   3.5. A hunter lined up on the player in range and inside the
///      player's sight box: snipe (hold, fire).
///   3.6. A guard whose player is outside the leash: hold the beat.
///   4. Attack when the target is in range (aim, settle, fire - a seat
///      only from inside its sight box; else close in).
///   5. Chase when the target is visible.
///   6. Patrol otherwise.
///
/// "Target" is the player for `Role::Player`/`Role::Guard` and the
/// player's frog for a `Role::Hunter` while it lives (`think`'s `target`).
/// The tree is rebuilt each tick (cheap: a handful of enum nodes) for clarity.
fn build<'a>() -> Node<Brain<'a>> {
    selector(vec![
        // 1. Wrecks are inert.
        sequence(vec![
            condition(|b: &mut Brain| b.me.is_wreck()),
            action("wreck", |_b: &mut Brain| Status::Success),
        ]),
        // 2. Flee when badly damaged and the player is still a threat.
        // Takes priority over the ammo-based retreat below: survival first.
        sequence(vec![
            condition(|b: &mut Brain| b.me.damage >= tuning().enemy_flee_damage && b.player_alive()),
            action("flee", act_flee),
        ]),
        // 2.5. Wedged against a tile a shell can remove: stop and shoot it
        // down rather than scrape along it - only while healthy and
        // stocked, see `Brain::wants_breach`. Above retreat/attack so a
        // tank that got stuck on its way to either finishes the job.
        sequence(vec![
            condition(|b: &mut Brain| b.wants_breach()),
            action("breach", act_breach),
        ]),
        // 3. Low on shells: back off and hold fire until recharged - unless
        // the special weapon carried still has ammo, since firing that costs
        // no shells at all and there's nothing to recharge by retreating
        // from. Deliberately short-circuits before `wants_retreat` so its
        // ammo hysteresis doesn't even latch on while a special covers for
        // it; once the special runs dry (`active_weapon()` falls back to
        // Shell), the next tick evaluates fresh against whatever
        // `shells_ammo` actually is by then.
        sequence(vec![
            condition(|b: &mut Brain| {
                b.player_alive()
                    && b.me.active_weapon() == ActiveWeapon::Shell
                    && b.ai.wants_retreat(b.me.shells_ammo)
            }),
            action("retreat", act_retreat),
        ]),
        // 3.4. A tower hurt this tank and stands lined up in sight: shoot
        // it back (`Ai::notify_tower_hit`). Never a detour - the routing
        // steers around a tower's reach, so this is a shot taken where one
        // offers itself. Below retreat: a tank low on shells keeps them.
        sequence(vec![
            condition(|b: &mut Brain| b.grudge_shot().is_some()),
            action("grudge", |b: &mut Brain| {
                let Some((dir, range)) = b.grudge_shot() else { return Status::Failure };
                b.hold_and_fire(dir, range, None);
                Status::Success
            }),
        ]),
        // 3.5. A hunter that happens to be lined up on the player within
        // attack range, inside the player's sight box, shoots the player
        // this tick instead of the frog - an opportunity taken, never a
        // detour (see `Brain::can_snipe_player`).
        sequence(vec![
            condition(|b: &mut Brain| b.can_snipe_player()),
            action("snipe", act_snipe),
        ]),
        // 3.6. A guard whose player is nowhere near its frog stays on its
        // beat instead of chasing - see `Role::Guard`.
        sequence(vec![
            condition(|b: &mut Brain| b.guard_holds()),
            action("guard", act_guard),
        ]),
        // 4. Attack when the target is alive and within attack range.
        sequence(vec![
            condition(|b: &mut Brain| {
                b.target_alive()
                    && b.dist_to_target() <= b.attack_range()
                    // Concealment has to break this tier as well as the
                    // chase below it, or a lost player is still walked at:
                    // `act_attack`'s unaligned branch repositions toward the
                    // target, so a tank that cannot shoot still closes until
                    // it is near enough to see through the grass. A hunter
                    // is aiming at the frog, not the player, so it is exempt.
                    && (b.hunting_frog() || !b.target_concealed || b.ai.hit_alert_timer > 0.0)
            }),
            action("attack", act_attack),
        ]),
        // 5. Chase when the target is alive and either within view range or
        // this tank has recently taken a hit - see `Ai::notify_hit`/
        // `ENEMY_HIT_ALERT_SECONDS`'s own doc comment: a shot landing from
        // outside normal awareness range shouldn't just be shrugged off. A
        // hunter's frog is an objective, not a sighting: it is chased from
        // anywhere on the map.
        sequence(vec![
            condition(|b: &mut Brain| {
                b.target_alive()
                    && (b.hunting_frog()
                        // Concealment breaks the chase, which is what makes
                        // hiding mean anything: without it the tier below
                        // (patrol, which follows the shared alert) is never
                        // reached and every enemy inside view range walks
                        // straight to a player it cannot see. A tank that
                        // took a hit keeps coming regardless - it knows
                        // something is there.
                        || (b.dist_to_target() <= b.sight && !b.target_concealed)
                        || b.ai.hit_alert_timer > 0.0)
            }),
            action("chase", act_chase),
        ]),
        // 5.5. Opportunistically go collect a live Laser pickup while firing
        // shells - reached only once nothing higher-priority (fleeing,
        // retreating, attacking, chasing) already claimed this tank, so it
        // never interrupts a fight, just fills idle patrol time with a
        // purposeful detour instead. A tank carries one special weapon and
        // a crate replaces it (`Tank::take_weapon`), so this tier and the
        // weapon tiers below are each gated on carrying none
        // (`Tank::wants_pickup`); which detour is worth taking *first* is
        // expressed by their tier order (laser, then plasma, then missiles,
        // then minigun - strongest first). See `act_seek_laser`. On a field map only a
        // pickup inside the tank's home leash is worth the detour
        // (`Brain::seek`).
        sequence(vec![
            condition(|b: &mut Brain| {
                b.me.wants_pickup(PickupKind::Laser) && b.seek(PickupKind::Laser).is_some()
            }),
            action("seek_laser", act_seek_laser),
        ]),
        // 5.6. Same idea for a live Plasma pickup (see tier 5.5's comment).
        sequence(vec![
            condition(|b: &mut Brain| {
                b.me.wants_pickup(PickupKind::Plasma)
                    && b.seek(PickupKind::Plasma).is_some()
            }),
            action("seek_plasma", act_seek_plasma),
        ]),
        // 5.65. The seeker-missile pod, between plasma and the minigun: a
        // volley finds its own target, so it is worth a detour.
        sequence(vec![
            condition(|b: &mut Brain| {
                b.me.wants_pickup(PickupKind::Missiles)
                    && b.seek(PickupKind::Missiles).is_some()
            }),
            action("seek_missiles", act_seek_missiles),
        ]),
        // 5.7. Same idea for a live Minigun pickup, last of the weapon
        // tiers (see tier 5.5's comment on ordering).
        sequence(vec![
            condition(|b: &mut Brain| {
                b.me.wants_pickup(PickupKind::Minigun)
                    && b.seek(PickupKind::Minigun).is_some()
            }),
            action("seek_minigun", act_seek_minigun),
        ]),
        // 5.8. Same idea for a live SpeedUp pickup, reached whenever this
        // tank isn't currently boosted - unlike the weapon tiers above, this
        // isn't gated on `active_weapon` (a stat buff, not a weapon), just
        // "not already benefiting from one".
        sequence(vec![
            condition(|b: &mut Brain| {
                b.me.wants_pickup(PickupKind::SpeedUp)
                    && b.seek(PickupKind::SpeedUp).is_some()
            }),
            action("seek_speedup", act_seek_speedup),
        ]),
        // 5.9. And for a live rainbow shield, whenever this tank isn't
        // already shielded - a full heal plus invulnerability is worth the
        // detour at any health.
        sequence(vec![
            condition(|b: &mut Brain| {
                b.me.wants_pickup(PickupKind::Shield) && b.seek(PickupKind::Shield).is_some()
            }),
            action("seek_shield", act_seek_shield),
        ]),
        // 6. Fallback: patrol.
        action("patrol", act_patrol),
    ])
}

// --- Leaf actions. Each fills in `b.intent` and returns Success. ---

/// Drive away from the player along a committed cardinal heading - or, if a
/// Health pickup is currently on the field, straight for that instead (see
/// `Brain::nearest_pickup`): a hurt tank actively trying to patch itself up
/// reads as far more purposeful than blindly running, and it's usually
/// heading away from the fight anyway since pickups respawn near the
/// battlefield's corners. Falls back to the old blind-flee behavior once no
/// Health pickup exists (already collected, still respawning).
fn act_flee(b: &mut Brain) -> Status {
    b.reset_aim();
    if let Some(target) = b.nearest_pickup(PickupKind::Health) {
        b.intent.move_dir = Some(b.steer(target));
        return Status::Success;
    }
    // Steer toward a point behind us (mirror of the player across our position),
    // so commitment/hysteresis applies just like chasing.
    let away_point = Position::new(
        2.0 * b.me.position.x - b.player.position.x,
        2.0 * b.me.position.y - b.player.position.y,
    );
    b.intent.move_dir = Some(b.steer(away_point));
    Status::Success
}

/// Back off to recharge ammo - or, if an Ammo pickup is currently on the
/// field, head straight for that instead (see `Brain::nearest_pickup` and
/// `act_flee`'s doc comment for the same reasoning: an active pickup run
/// beats blindly backing away). Without one, falls back to the old
/// behavior: retreat only until clear of ENEMY_RETREAT_RANGE (breathing
/// room outside attack range) rather than running all the way off - once
/// there, hold position, face the player, and just wait out the passive
/// recharge instead of camping the map edge. Never fires: `b.intent.fire`
/// starts false each frame and this leaf doesn't set it.
fn act_retreat(b: &mut Brain) -> Status {
    b.reset_aim();
    if let Some(target) = b.nearest_pickup(PickupKind::Ammo) {
        b.intent.move_dir = Some(b.steer(target));
        return Status::Success;
    }
    if b.dist_to_player() >= tuning().enemy_retreat_range() {
        b.intent.face = Some(Dir::toward(b.me.position, b.player.position));
        return Status::Success;
    }
    let away_point = Position::new(
        2.0 * b.me.position.x - b.player.position.x,
        2.0 * b.me.position.y - b.player.position.y,
    );
    b.intent.move_dir = Some(b.steer(away_point));
    Status::Success
}

/// Head for the nearest live Laser pickup - see the behavior tree's tier 5.5
/// (`build`) for when this is actually reached. Failure (rather than a blind
/// fallback like `act_flee`/`act_retreat` have) is deliberate: the tree's
/// own condition already guarantees a pickup exists whenever this runs, so
/// `None` here would mean it was collected the same frame another enemy
/// reached it first - falling through to patrol is the right response, not
/// wandering toward a spot that's no longer there.
fn act_seek_laser(b: &mut Brain) -> Status {
    b.reset_aim();
    let Some(target) = b.seek(PickupKind::Laser) else {
        return Status::Failure;
    };
    b.intent.move_dir = Some(b.steer(target));
    Status::Success
}

/// Same idea as `act_seek_laser`, for a Plasma pickup instead - see tier
/// 5.6 (`build`) for when this is actually reached.
fn act_seek_plasma(b: &mut Brain) -> Status {
    b.reset_aim();
    let Some(target) = b.seek(PickupKind::Plasma) else {
        return Status::Failure;
    };
    b.intent.move_dir = Some(b.steer(target));
    Status::Success
}

/// Same idea as `act_seek_laser`, for a Missiles pickup instead - see tier
/// 5.65 (`build`) for when this is actually reached.
fn act_seek_missiles(b: &mut Brain) -> Status {
    b.reset_aim();
    let Some(target) = b.seek(PickupKind::Missiles) else {
        return Status::Failure;
    };
    b.intent.move_dir = Some(b.steer(target));
    Status::Success
}

/// Same idea as `act_seek_laser`, for a Minigun pickup instead - see tier
/// 5.7 (`build`) for when this is actually reached.
fn act_seek_minigun(b: &mut Brain) -> Status {
    b.reset_aim();
    let Some(target) = b.seek(PickupKind::Minigun) else {
        return Status::Failure;
    };
    b.intent.move_dir = Some(b.steer(target));
    Status::Success
}

/// Same idea as `act_seek_laser`, for a SpeedUp pickup instead - see tier
/// 5.8 (`build`) for when this is actually reached.
fn act_seek_speedup(b: &mut Brain) -> Status {
    b.reset_aim();
    let Some(target) = b.seek(PickupKind::SpeedUp) else {
        return Status::Failure;
    };
    b.intent.move_dir = Some(b.steer(target));
    Status::Success
}

/// Same idea as `act_seek_speedup`, for a rainbow shield pickup - see tier
/// 5.9 (`build`) for when this is actually reached.
fn act_seek_shield(b: &mut Brain) -> Status {
    b.reset_aim();
    let Some(target) = b.seek(PickupKind::Shield) else {
        return Status::Failure;
    };
    b.intent.move_dir = Some(b.steer(target));
    Status::Success
}

/// Hold near the target and shoot when lined up on a cardinal axis. The tank
/// only fires after staying aligned for ENEMY_AIM_SETTLE, and stops to aim.
/// A seat is only fired at from inside its sight box
/// (`Brain::may_fire_at_seat`): lined up on a seat's column further out
/// than the box reaches, the tank keeps closing in.
fn act_attack(b: &mut Brain) -> Status {
    let (fire_dir, off_axis, in_front) = b.aim_alignment();
    let at_seat = !b.hunting_frog();
    // A geometrically on-axis spot the AI can't actually shoot from (a wall
    // in the way) is treated the same as not being aligned at all, so the
    // tank repositions instead of settling in and holding a shot it can
    // never take - see `think`'s `line_of_sight` parameter doc comment.
    // So is a spot outside the seat's sight box: the reposition below
    // steers at the tank's engagement slot - a firing slot stands inside
    // the box, a reserve beyond attack range where nothing fires - or at
    // the seat itself, so a tank closes into the box to fire rather than
    // holding outside it on a shot the rule never lets it take.
    let aligned = off_axis <= tuning().enemy_fire_align_px
        && in_front
        && b.line_of_sight
        && (!at_seat || b.may_fire_at_seat());

    if aligned {
        // Line up: face the fire direction and hold position while settling.
        let range = b.dist_to_target();
        let seat = at_seat.then_some(b.ai.target_player);
        b.hold_and_fire(fire_dir, range, seat);
    } else {
        // Not lined up: reposition toward this tank's engagement-ring slot
        // (with commitment) rather than the target's exact position, so a
        // group of attackers spreads out instead of piling onto the same
        // point - see `Brain::engage_point`.
        b.reset_aim();
        b.intent.move_dir = Some(b.steer(b.engage_point()));
    }
    Status::Success
}

/// A hunter's opportunistic shot at the player: already lined up (see
/// `Brain::can_snipe_player`), so just hold and fire down that axis.
fn act_snipe(b: &mut Brain) -> Status {
    let (fire_dir, _, _) = b.aim_alignment_at(b.player.position);
    let range = b.dist_to_player();
    let seat = b.ai.target_player;
    b.hold_and_fire(fire_dir, range, Some(seat));
    if b.intent.fire {
        b.ai.snipe_cooldown = tuning().hunter_snipe_cooldown_seconds;
    }
    Status::Success
}

/// A guard's beat while the player is away from its frog (see
/// `Brain::guard_holds`): head back to the beat when outside it, otherwise
/// wander within it.
fn act_guard(b: &mut Brain) -> Status {
    let Some(leash) = b.leash() else { return Status::Failure };
    b.reset_aim();
    let me = b.me.position;
    if me.distance_to(leash.anchor) > leash.radius {
        // Aim for the beat's inner rim on the line back to the frog, not
        // the frog itself: it is a solid body in a blocked nav cell, so a
        // route *to* it never exists. A committed heading still pointing
        // away from home is dropped so the turn happens now - the
        // hold/margin gate is blind to a straight reversal by design, and
        // out here the only right answer is to go back.
        let away = Vec2::new(me.x - leash.anchor.x, me.y - leash.anchor.y);
        let len = (away.x * away.x + away.y * away.y).sqrt().max(1.0);
        let rim = Position::new(leash.anchor.x + away.x / len * leash.keep_off, leash.anchor.y + away.y / len * leash.keep_off);
        if b.ai.committed_dir.is_some_and(|d| d.vec().x * away.x + d.vec().y * away.y > 0.0) {
            b.ai.committed_dir = None;
        }
        b.intent.move_dir = Some(b.steer(rim));
    } else {
        b.intent.move_dir = Some(b.wander_within(leash));
    }
    Status::Success
}

/// Hold position facing the tile ahead and shoot it down (see
/// `Brain::wants_breach`). A burning wood tile is waited out, not shot:
/// damage is a no-op until it chars away. Paced by `fire_timer` at
/// `enemy_breach_fire_interval`, and held while a teammate is in front.
fn act_breach(b: &mut Brain) -> Status {
    let Some(breach) = b.ai.breach else { return Status::Failure };
    b.reset_aim();
    b.intent.face = Some(breach.dir);
    b.ai.commit(breach.dir);
    let burning = b.walls_ahead[breach.dir.index()].is_some_and(|w| w.burning);
    let reach = b.me.hull_size() * 0.5 + tuning().enemy_breach_reach_px;
    if !burning && b.ai.fire_timer <= 0.0 && !b.friendly_blocks_shot(breach.dir, reach) {
        b.ai.fire_timer = tuning().enemy_breach_fire_interval;
        b.intent.fire = true;
    }
    Status::Success
}

/// Close in on the target along a committed cardinal heading - toward this
/// tank's engagement-ring slot, not the target's exact position, so a group
/// of chasers spreads out instead of converging on the same point. See
/// `Brain::engage_point`.
fn act_chase(b: &mut Brain) -> Status {
    b.intent.move_dir = Some(b.steer(b.engage_point()));
    b.reset_aim();
    Status::Success
}

/// Wander toward a roaming waypoint (see `Ai::wander`) - unless the group
/// has a shared `alert` (see `Brain::alert`), in which case head straight
/// for it instead of picking a random point. This is what makes the *whole*
/// map converge on a sighting rather than just whichever single enemy
/// happened to be close enough to personally see the player. Steers at
/// `engage_target` instead of the raw alert point on the rare tick this
/// tank already has one despite still being outside view range (it took a
/// hit and got pulled into slot assignment early - see `Game::update`'s
/// `hit_alerted`); a merely alert-following tank with no personal sighting
/// yet has no slot and just heads for the raw point, which is deliberate -
/// see `Game::update`'s engagement-slot doc comment for why broadening slot
/// assignment to every alerted tank, not just hit ones, made clustering
/// worse instead of better (a still-distant pack funnels toward its
/// eventual axis slots through the same bottleneck for its whole transit).
///
/// On a field map the alert is the tank's own (`simulation::field`: it
/// reached the tank down a chain of neighbours, or it is the call that
/// sends a wave tank to the fight), and a tank without one is leashed to
/// its home (`Brain::home_leash`): past the leash it turns back, inside it
/// it wanders within it.
fn act_patrol(b: &mut Brain) -> Status {
    if let Some(target) = b.alert {
        b.intent.move_dir = Some(b.steer(b.engage_target.unwrap_or(target)));
    } else if let Some(leash) = b.home_leash() {
        let me = b.me.position;
        if me.distance_to(leash.anchor) > leash.radius {
            // Home is where the tank stood, an open cell, so it is routed
            // to directly. A committed heading still pointing away is
            // dropped so the turn happens now: the hold/margin gate is
            // blind to a straight reversal by design (`act_guard` does the
            // same on its beat).
            let away = Vec2::new(me.x - leash.anchor.x, me.y - leash.anchor.y);
            if b.ai.committed_dir.is_some_and(|d| d.vec().x * away.x + d.vec().y * away.y > 0.0) {
                b.ai.committed_dir = None;
            }
            b.intent.move_dir = Some(b.steer(leash.anchor));
        } else {
            b.intent.move_dir = Some(b.wander_within(leash));
        }
    } else {
        b.intent.move_dir = Some(b.wander());
    }
    b.reset_aim();
    Status::Success
}

impl Brain<'_> {
    /// Reset the aim-settle timer whenever the tank isn't holding a firing line.
    fn reset_aim(&mut self) {
        self.ai.aim_settle = 0.0;
    }

    /// Decide whether this shot misfires because what it is aimed at is
    /// dangerously close (`dist` px away), returning the angular deflection
    /// (degrees, signed) to add to the shot. The closer, the likelier the
    /// miss; zero means a clean shot. Beyond ENEMY_MISFIRE_RANGE the enemy
    /// always fires straight.
    fn roll_misfire(&mut self, dist: f32) -> f32 {
        if dist >= tuning().enemy_misfire_range {
            return 0.0;
        }
        // Chance ramps from 0 at the range edge up to _CHANCE_MAX point-blank.
        let closeness = 1.0 - dist / tuning().enemy_misfire_range;
        let chance = closeness * tuning().enemy_misfire_chance_max;
        if self.rng.random_range(0.0..1.0) >= chance {
            return 0.0;
        }
        // Misfire: deflect by a random magnitude to either side.
        let mag = self
            .rng
            .random_range(tuning().enemy_misfire_angle_min..tuning().enemy_misfire_angle_max);
        if self.rng.random_range(0.0..1.0) < 0.5 {
            -mag
        } else {
            mag
        }
    }
}

#[cfg(test)]
mod role_tests {
    use super::*;
    use rand::SeedableRng;

    /// One open-field `think` tick for an enemy of `role` at `me`, the
    /// player at `player`, fighting `target` (see `Ai::think`), with a
    /// clear line of sight everywhere.
    fn tick(ai: &mut Ai, me: Position, player: Position, target: Position, frog_target: Option<Position>) -> Intent {
        let mut me_tank = Tank::default();
        me_tank.position = me;
        let mut player_tank = Tank::default();
        player_tank.position = player;
        let grid = Grid::build(1280.0, 720.0, 48.0, 0.0, std::iter::empty());
        let movers = [
            Mover { position: player, velocity: Vec2::new(0.0, 0.0), radius: 20.0, is_player: true },
            Mover { position: me, velocity: Vec2::new(0.0, 0.0), radius: 20.0, is_player: false },
        ];
        let mut rng = SmallRng::seed_from_u64(7);
        ai.think(
            &me_tank,
            &player_tank,
            target,
            frog_target,
            1280.0,
            720.0,
            1.0 / 60.0,
            &movers,
            1,
            &grid,
            &mut rng,
            None,
            None,
            &[],
            true,
            true,
            false,
            [None; 4],
            tuning().enemy_view_range,
        )
    }

    /// A tank with nothing to fight detours for a pickup it wants only
    /// inside its home leash on a field map (`Brain::seek`): a laser 400 px
    /// from home is sought, one 900 px away is left alone for the patrol,
    /// and on an arena - no home kept - the far one is sought too.
    #[test]
    fn a_leashed_tank_seeks_only_the_pickups_inside_its_leash() {
        let (width, height) = (3200.0, 1280.0);
        let home = Position::new(1600.0, 640.0);
        let player_at = Position::new(200.0, 640.0);
        assert!(home.distance_to(player_at) > tuning().enemy_view_range, "nothing to fight in sight");
        let leash = tuning().enemy_leash_px;
        let (near, far) = (Position::new(home.x + 400.0, home.y), Position::new(home.x + 900.0, home.y));
        assert!(near.distance_to(home) < leash - 32.0 && far.distance_to(home) > leash, "the case this is about");
        let act = |home_kept: bool, pickup: Position| {
            let mut ai = Ai::with_role(Role::Player);
            if home_kept {
                ai.field.home = Some(home);
            }
            let mut me = Tank::default();
            me.position = home;
            let mut player = Tank::default();
            player.position = player_at;
            assert!(me.wants_pickup(PickupKind::Laser));
            let grid = Grid::build(width, height, 48.0, 0.0, std::iter::empty());
            let movers = [
                Mover { position: player_at, velocity: Vec2::new(0.0, 0.0), radius: 20.0, is_player: true },
                Mover { position: home, velocity: Vec2::new(0.0, 0.0), radius: 20.0, is_player: false },
            ];
            let mut rng = SmallRng::seed_from_u64(7);
            ai.think(
                &me,
                &player,
                player_at,
                None,
                width,
                height,
                1.0 / 60.0,
                &movers,
                1,
                &grid,
                &mut rng,
                None,
                None,
                &[(PickupKind::Laser, pickup)],
                true,
                true,
                false,
                [None; 4],
                tuning().enemy_view_range,
            );
            ai.snapshot().last_action
        };
        assert_eq!(act(true, near), Some("seek_laser"), "inside the leash");
        assert_ne!(act(true, far), Some("seek_laser"), "past the leash");
        assert_eq!(act(false, far), Some("seek_laser"), "an arena keeps no home");
    }

    #[test]
    fn a_hunter_closes_on_its_frog_and_reverts_to_the_player_without_one() {
        let mut ai = Ai::with_role(Role::Hunter);
        assert_eq!(ai.snapshot().role, "hunter");
        let me = Position::new(600.0, 300.0);
        let player = Position::new(200.0, 600.0);
        let frog = Position::new(1100.0, 300.0);
        // Quarry alive: the frog is the target, so the tank heads east
        // toward it rather than south-west toward the player.
        let intent = tick(&mut ai, me, player, frog, Some(frog));
        assert_eq!(intent.move_dir, Some(Dir::Right), "{:?}", ai.snapshot().last_action);
        assert_eq!(ai.snapshot().last_action, Some("chase"));
        // Quarry dead: the simulation hands it the player as target and
        // no frog, and it chases the player like any other enemy.
        let mut ai = Ai::with_role(Role::Hunter);
        let intent = tick(&mut ai, me, player, player, None);
        assert!(
            matches!(intent.move_dir, Some(Dir::Left | Dir::Down)),
            "heads south-west toward the player, got {:?}",
            intent.move_dir.map(Dir::name)
        );
        assert_eq!(ai.snapshot().last_action, Some("chase"));
    }

    #[test]
    fn a_hunter_lined_up_on_the_player_in_range_snipes_instead() {
        let mut ai = Ai::with_role(Role::Hunter);
        let me = Position::new(600.0, 300.0);
        let frog = Position::new(1100.0, 300.0);
        // The player sits straight below, well inside attack range.
        let player = Position::new(600.0, 300.0 + tuning().enemy_attack_range * 0.5);
        // Snipe holds and settles its aim until the shot goes off.
        let settle = (tuning().enemy_aim_settle * 60.0) as u32 + 2;
        let mut fired = false;
        for _ in 0..settle + 60 {
            let intent = tick(&mut ai, me, player, frog, Some(frog));
            assert_eq!(ai.snapshot().last_action, Some("snipe"));
            assert_eq!(intent.move_dir, None, "a snipe holds position");
            assert_eq!(intent.face, Some(Dir::Down));
            if intent.fire {
                fired = true;
                break;
            }
        }
        assert!(fired, "never fired at the player");
        // One shot is the opportunity: still lined up, the hunter goes
        // back to the frog until the snipe cooldown has run out.
        assert!(ai.snapshot().snipe_cooldown > 0.0, "the shot starts the cooldown");
        tick(&mut ai, me, player, frog, Some(frog));
        assert_eq!(ai.snapshot().last_action, Some("chase"), "back to the frog after the shot");
        let cooldown = (tuning().hunter_snipe_cooldown_seconds * 60.0) as u32 + 2;
        for _ in 0..cooldown {
            tick(&mut ai, me, player, frog, Some(frog));
        }
        assert_eq!(ai.snapshot().last_action, Some("snipe"), "may snipe again once the cooldown is over");
        // Off-axis, the same player in range is ignored for the frog.
        let mut ai = Ai::with_role(Role::Hunter);
        let player = Position::new(700.0, 450.0);
        tick(&mut ai, me, player, frog, Some(frog));
        assert_eq!(ai.snapshot().last_action, Some("chase"));
    }

    /// A snipe is a shot at a seat, so it is taken only from inside the
    /// player's sight box: lined up and in attack range but further out on
    /// the player's column than the box reaches, the hunter goes on after
    /// the frog.
    #[test]
    fn a_hunter_snipes_only_from_inside_the_players_sight_box() {
        let (_, half_h) = tuning().sight_box_half_px();
        let me = Position::new(600.0, 300.0);
        let frog = Position::new(1100.0, 300.0);
        let action_with_player_below = |below: f32| {
            let mut ai = Ai::with_role(Role::Hunter);
            tick(&mut ai, me, Position::new(me.x, me.y + below), frog, Some(frog));
            ai.snapshot().last_action
        };
        assert!(half_h + 30.0 < tuning().enemy_attack_range, "the defaults this is about");
        assert_eq!(action_with_player_below(half_h + 30.0), Some("chase"), "outside the box: back to the frog");
        assert_eq!(action_with_player_below(half_h - 30.0), Some("snipe"));
    }

    #[test]
    fn a_guard_holds_its_beat_until_the_player_comes_close() {
        let frog = Position::new(1100.0, 300.0);
        let leash = tuning().guard_leash_px;
        // Player far from the frog, guard outside the leash: head home.
        let mut ai = Ai::with_role(Role::Guard);
        let intent = tick(&mut ai, Position::new(600.0, 300.0), Position::new(200.0, 600.0), Position::new(200.0, 600.0), Some(frog));
        assert_eq!(ai.snapshot().last_action, Some("guard"));
        assert_eq!(intent.move_dir, Some(Dir::Right));
        // Guard inside the leash: wanders, but only to waypoints on its
        // beat - inside the leash and clear of the frog itself.
        let mut ai = Ai::with_role(Role::Guard);
        let keep_off = tuning().guard_keep_off_px;
        for _ in 0..600 {
            let intent = tick(&mut ai, Position::new(1000.0, 300.0), Position::new(200.0, 600.0), Position::new(200.0, 600.0), Some(frog));
            assert_eq!(ai.snapshot().last_action, Some("guard"));
            assert!(intent.move_dir.is_some());
            let s = ai.snapshot();
            let wp = Position::new(s.waypoint_x, s.waypoint_y);
            let d = wp.distance_to(frog);
            assert!(d <= leash && d >= keep_off.min(leash * 0.5), "waypoint {wp:?} is {d} px from the frog");
        }
        // Player inside the leash (and in the guard's view): fights like
        // everyone else.
        let mut ai = Ai::with_role(Role::Guard);
        let player = Position::new(frog.x - leash * 0.5, frog.y);
        tick(&mut ai, Position::new(400.0, 300.0), player, player, Some(frog));
        assert_eq!(ai.snapshot().last_action, Some("chase"));
    }
}

#[cfg(test)]
mod lane_tests {
    use super::*;
    use crate::PATHFIND_CELL_SIZE as CELL;

    /// An assault's grip (px/s^2) and an enemy's pace (px/s): a slide of
    /// 160^2 / 1000 = 25.6 px through a turn.
    const GRIP: f32 = 500.0;
    const PACE: f32 = 160.0;

    fn at(col: usize, row: usize) -> Position {
        Position::new((col as f32 + 0.5) * CELL, (row as f32 + 0.5) * CELL)
    }

    /// A 40 x 30 grid, `walls` blocked, with a field toward cell (10, 20):
    /// a hull heading east along row 5 west of column 10 reads its route
    /// on east to column 10 and then south.
    fn grid(walls: &[(usize, usize)]) -> Grid {
        let obstacles: Vec<(Position, f32)> = walls.iter().map(|&(c, r)| (at(c, r), CELL / 2.0)).collect();
        let mut grid = Grid::build(40.0 * CELL, 30.0 * CELL, CELL, 0.0, obstacles.into_iter());
        grid.add_field(at(10, 20));
        grid
    }

    /// What the lanes tell a hull at `from` heading east at `pace`, routed
    /// to cell (10, 20).
    fn read(grid: &Grid, from: Position, pace: f32) -> Option<Lane> {
        read_to(grid, from, pace, at(10, 20))
    }

    /// `read`, routed to `to`.
    fn read_to(grid: &Grid, from: Position, pace: f32, to: Position) -> Option<Lane> {
        let ai = Ai { committed_dir: Some(Dir::Right), motion: Vec2::new(pace, 0.0), ..Ai::default() };
        let ctx = AvoidCtx { movers: &[], my_index: 0, radius: 20.0, speed: PACE, grip: GRIP, dt: 1.0 / 60.0, on_portal_cooldown: false };
        let route = grid.route_ahead(from, to, Some(Dir::Right.vec())).expect("routes");
        ai.lane_turn(from, Dir::Right, &route, ctx, grid)
    }

    /// The turn comes where the slide ends on the turn's centre line: a
    /// slide short of column 10's centre holds on, one that reaches it
    /// turns, a cell before the turning - and where the hull rides across
    /// its lane makes no difference at all.
    #[test]
    fn a_turn_comes_where_the_slide_ends_on_the_turns_centre_line() {
        let grid = grid(&[]);
        let line = at(10, 5).x;
        let slide = PACE * PACE / (2.0 * GRIP);
        for across in [0.0, 15.5, -15.5] {
            let y = at(10, 5).y + across;
            assert_eq!(read(&grid, Position::new(line - slide - 4.0, y), PACE), Some(Lane::Hold), "{across} px across");
            assert_eq!(read(&grid, Position::new(line - slide, y), PACE), Some(Lane::Turn(Dir::Down)), "{across} px across");
        }
        // Standing still it turns on the line itself.
        assert_eq!(read(&grid, Position::new(line - 2.0, at(10, 5).y), 0.0), Some(Lane::Hold));
        assert_eq!(read(&grid, Position::new(line, at(10, 5).y), 0.0), Some(Lane::Turn(Dir::Down)));
        // Far from the turning: on along the lane.
        assert_eq!(read(&grid, at(3, 5), PACE), Some(Lane::Hold));
    }

    /// A turn not taken early - the corner it would have cut was shut, or
    /// the turn came into view too late - is taken in the cell it turns in
    /// while the slide still ends within half a cell of the line, and left
    /// to the route from further on past that; a slide longer than a cell
    /// still gets the first think inside.
    #[test]
    fn a_turn_too_late_to_land_in_its_lane_is_left_to_the_route_on() {
        // Column 9 shut below row 5: the corner a turn from it would cut.
        let shut = grid(&[(9, 6)]);
        let (line, y) = (at(10, 5).x, at(10, 5).y);
        let slide = PACE * PACE / (2.0 * GRIP);
        assert_eq!(read(&shut, Position::new(line - slide, y), PACE), Some(Lane::Hold), "no cutting the shut corner");
        assert_eq!(read(&shut, Position::new(line - 15.0, y), PACE), Some(Lane::Turn(Dir::Down)), "in its cell, slide ending 10.6 px past the line");
        assert_eq!(read(&shut, Position::new(line - 9.0, y), PACE), Some(Lane::Hold), "slide ending 16.6 px past the line");
        // Half as fast again, a 57.6 px slide: past the line from the moment
        // it comes in, and it turns there all the same.
        let fast = 1.5 * PACE;
        assert_eq!(read(&shut, Position::new(line - 15.0, y), fast), Some(Lane::Turn(Dir::Down)));
        assert_eq!(read(&shut, Position::new(line - 10.0, y), fast), Some(Lane::Hold));
    }

    /// A searched route - toward a target of the tank's own, with no field
    /// to read - is the margin's, as every arena's route is, while the
    /// margin can turn the hull onto it. Riding the edge of its lane on the
    /// side the route turns to, nearer the next lane's centre line than
    /// `ai_dir_switch_margin_px`, the margin never could, and the turn is
    /// a lane turn: taken where the slide still ends within half a cell of
    /// the turning's centre line, held past that.
    #[test]
    fn a_searched_route_is_left_to_the_margin_but_on_the_edge_of_its_lane() {
        let plain = Grid::build(40.0 * CELL, 30.0 * CELL, CELL, 0.0, std::iter::empty());
        let slide = PACE * PACE / (2.0 * GRIP);
        assert_eq!(read(&plain, Position::new(at(10, 5).x - slide, at(10, 5).y), PACE), None);
        // Down column 6 is the search's one way from it to (6, 20), and the
        // turn shows only once the hull is in column 6.
        let (line, to) = (at(6, 5).x, at(6, 20));
        let x = line + 12.0 - slide;
        assert_eq!(read_to(&plain, Position::new(x, at(6, 5).y), PACE, to), None, "on its lane's centre line");
        let edge = at(6, 5).y + 15.5;
        assert!(at(6, 6).y - edge <= tuning().ai_dir_switch_margin_px, "the case this is about");
        assert_eq!(read_to(&plain, Position::new(x, edge), PACE, to), Some(Lane::Turn(Dir::Down)), "slide ending 12 px past the line");
        assert_eq!(read_to(&plain, Position::new(line + 18.0 - slide, edge), PACE, to), Some(Lane::Hold), "18 px past it");
    }

    /// Just after an early turn north into a one-lane gap at column 18, the
    /// hull's centre still stands in column 19, below a wall, while its
    /// slide west carries it under the gap: on a field map the wall ahead
    /// is judged where the slide ends, so the turn is not thrown back
    /// across the gap's mouth; an arena still judges the centre.
    #[test]
    fn a_wall_ahead_is_judged_where_the_slide_across_ends() {
        let grid = grid(&[(19, 4), (17, 4)]);
        let from = Position::new(611.6, 188.1);
        let sliding = Ai { committed_dir: Some(Dir::Up), motion: Vec2::new(-129.0, -101.0), ..Ai::default() };
        let ctx = AvoidCtx { movers: &[], my_index: 0, radius: 20.0, speed: PACE, grip: GRIP, dt: 1.0 / 60.0, on_portal_cooldown: false };
        let field = Ai { field: FieldMind { home: Some(at(0, 0)), ..FieldMind::default() }, ..sliding };
        assert!(!field.walks_into_wall(from, ctx, &grid), "a 16.6 px slide ends in column 18, under the gap");
        assert!(sliding.walks_into_wall(from, ctx, &grid), "an arena judges the centre, under the wall");
        // Sliding no further than the centre's own column, the wall counts.
        let slow = Ai { motion: Vec2::new(-60.0, -101.0), ..field };
        assert!(slow.walks_into_wall(from, ctx, &grid), "a 3.6 px slide stays in column 19");
    }

    /// A route whose first step is straight back is no walk along the
    /// lanes from here: the margin decides, blind to a reversal as ever.
    #[test]
    fn a_route_straight_back_is_left_to_the_margin() {
        // Row 6 shut east of column 10: past the turning, the only way is back.
        let walls: Vec<(usize, usize)> = (11..=20).map(|c| (c, 6)).collect();
        assert_eq!(read(&grid(&walls), at(14, 5), PACE), None);
    }
}

#[cfg(test)]
mod stuck_tests {
    use super::*;
    use rand::SeedableRng;

    const DT: f32 = 1.0 / 60.0;

    /// One `think` tick for an enemy standing at `me` with the player far
    /// off its firing axes (so the tree wants to move, not hold and aim).
    fn tick(ai: &mut Ai, me_pos: Position) -> Intent {
        let mut me = Tank::default();
        me.position = me_pos;
        let mut player = Tank::default();
        player.position = Position::new(200.0, 600.0);
        let grid = Grid::build(1280.0, 720.0, 48.0, 0.0, std::iter::empty());
        let movers = [
            Mover { position: player.position, velocity: Vec2::new(0.0, 0.0), radius: 20.0, is_player: true },
            Mover { position: me.position, velocity: Vec2::new(0.0, 0.0), radius: 20.0, is_player: false },
        ];
        let mut rng = SmallRng::seed_from_u64(7);
        ai.think(
            &me,
            &player,
            player.position,
            None,
            1280.0,
            720.0,
            DT,
            &movers,
            1,
            &grid,
            &mut rng,
            None,
            None,
            &[],
            false,
            false,
            false,
            [None; 4],
            tuning().enemy_view_range,
        )
    }

    /// Move the tank at `velocity` (px/s) for `frames` ticks while it is
    /// on record as having been told to drive `commanded` every tick -
    /// what physics did versus what the AI asked for.
    fn drive(ai: &mut Ai, pos: &mut Position, commanded: Dir, velocity: Position, frames: u32) -> Intent {
        let mut intent = Intent::default();
        for _ in 0..frames {
            pos.x += velocity.x * DT;
            pos.y += velocity.y * DT;
            ai.last_move_dir = Some(commanded);
            ai.committed_dir = Some(commanded);
            intent = tick(ai, *pos);
        }
        intent
    }

    /// A tank that has stood on `pos` for one tick, so the next tick has a
    /// displacement to judge.
    fn settled() -> (Ai, Position) {
        let mut ai = Ai::default();
        let pos = Position::new(600.0, 300.0);
        ai.last_move_dir = None;
        tick(&mut ai, pos);
        (ai, pos)
    }

    #[test]
    fn only_sustained_progress_along_the_commanded_heading_resets_the_stuck_clock() {
        let (mut ai, mut pos) = settled();
        // Told to go Down, carried East at speed by a jam: no progress.
        drive(&mut ai, &mut pos, Dir::Down, Position::new(100.0, 0.0), 12);
        assert!(ai.stuck_timer > 0.15, "sideways drift counted as movement: {}", ai.stuck_timer);
        // Actually driving Down clears it within a few frames.
        drive(&mut ai, &mut pos, Dir::Down, Position::new(0.0, 100.0), 6);
        assert_eq!(ai.stuck_timer, 0.0);
        // Shoved backwards is no better than standing still.
        drive(&mut ai, &mut pos, Dir::Down, Position::new(0.0, -100.0), 12);
        assert!(ai.stuck_timer > 0.15);
        // Deliberately holding position is not stuck, whatever physics says.
        ai.last_move_dir = None;
        tick(&mut ai, pos);
        assert_eq!(ai.stuck_timer, 0.0);
        assert_eq!(ai.snapshot().progress_px_s, None);
    }

    #[test]
    fn a_single_frame_twitch_does_not_clear_the_stuck_clock() {
        let (mut ai, mut pos) = settled();
        // Pressed against another body: a fraction of a pixel a frame.
        drive(&mut ai, &mut pos, Dir::Down, Position::new(0.0, 2.0), 20);
        let before = ai.stuck_timer;
        assert!(before > 0.3, "creeping at 2 px/s counted as progress: {before}");
        // The contact solver shoves it two pixels in one frame (120 px/s
        // for that frame alone), then it is pinned again.
        drive(&mut ai, &mut pos, Dir::Down, Position::new(0.0, 120.0), 1);
        assert!(ai.stuck_timer > before, "one fast frame cleared the clock");
        drive(&mut ai, &mut pos, Dir::Down, Position::new(0.0, 2.0), 20);
        assert!(ai.stuck_timer > before + 0.3);
    }

    #[test]
    fn a_tank_carried_sideways_long_enough_escapes_perpendicular() {
        let (mut ai, mut pos) = settled();
        let budget = (tuning().stuck_escape_seconds * 60.0) as u32 + 3;
        let mut escaped = None;
        for frame in 1..=budget {
            let intent = drive(&mut ai, &mut pos, Dir::Down, Position::new(100.0, 0.0), 1);
            let dir = intent.move_dir.expect("still trying to move");
            if dir != Dir::Down {
                escaped = Some((frame, dir));
                break;
            }
        }
        let (frame, dir) = escaped.expect("never escaped the failing heading");
        assert!(dir.is_horizontal(), "escape should turn off the failing axis, got {}", dir.rotation());
        assert!(
            frame as f32 / 60.0 >= tuning().stuck_escape_seconds,
            "escaped after {frame} frames, before stuck_escape_seconds elapsed"
        );
        assert_eq!(ai.stuck_timer, 0.0, "the escape resets the clock");
        assert_eq!(ai.escapes, 1);
    }
}

#[cfg(test)]
mod intent_tests {
    use super::*;

    /// The throttle is stored as a *deviation* (`slow`) precisely so that a
    /// default intent drives normally. If this ever fails, every driver that
    /// did not explicitly set the field has become a stopped tank.
    #[test]
    fn a_default_intent_drives_at_full_speed() {
        assert_eq!(Intent::default().slow, 0.0);
        assert_eq!(Intent::default().speed_scale(), 1.0);
    }

    /// `Game::drive_tank` multiplies the commanded target by `speed_scale()`.
    /// At 1.0 that multiply is *exactly* value-preserving under IEEE-754, so
    /// adding the throttle cannot perturb a single bit of an untouched
    /// round - which is what lets the C2 rollout's byte-identical checkpoints
    /// treat this step as free (docs/enemy-command-and-control-prd.md §4).
    #[test]
    fn scaling_by_an_untouched_throttle_is_bit_exact() {
        let scale = Intent::default().speed_scale();
        for v in [0.0f32, 1.0, -1.0, 210.0, -173.456, f32::MIN_POSITIVE, f32::MAX] {
            assert_eq!((v * scale).to_bits(), v.to_bits(), "{v} changed bits when scaled by {scale}");
        }
    }

    #[test]
    fn slow_clamps_rather_than_reversing_or_overdriving() {
        let faster = Intent { slow: -1.0, ..Intent::default() };
        let stopped = Intent { slow: 1.0, ..Intent::default() };
        let absurd = Intent { slow: 5.0, ..Intent::default() };
        assert_eq!(faster.speed_scale(), 1.0, "a negative slow must not overdrive");
        assert_eq!(stopped.speed_scale(), 0.0);
        assert_eq!(absurd.speed_scale(), 0.0, "an out-of-range slow must not reverse the tank");
    }
}

#[cfg(test)]
mod separation_tests {
    use crate::map::MapFile;
    use crate::simulation::{Event, Game, Input};

    const W: f32 = 1280.0;
    const H: f32 = 720.0;

    /// Enemies keep a car's length rather than driving through each other
    /// and through the player (`crowded_ahead`, `enemy_separation_px`).
    ///
    /// A bare map with the player parked in the open, because the shipped
    /// map's tall grass gates ram damage on concealment and so never
    /// exercises the player case at all.
    ///
    /// The ceilings were re-measured when the nav grid's cell size dropped
    /// to the map grid's (see `PATHFIND_CELL_SIZE`). Over 16 seeds x 1200
    /// frames, `enemy_separation_px` 0 (brake off) vs 12 (on):
    ///
    /// ```text
    ///                     rams into player   enemy-vs-enemy rams
    ///   48px grid  off           12                  79
    ///   48px grid  on             6                  46
    ///   32px grid  off           27                  37
    ///   32px grid  on            18                  60
    /// ```
    ///
    /// The brake still works - it is the only reason the on row is below
    /// the off row - but both columns moved, in opposite directions, and
    /// neither is the brake's doing: at the coarser cell size enemies
    /// converging on one target bunched into *each other* instead of
    /// arriving (79 enemy-vs-enemy rams with nothing braking them), and
    /// routing that actually reaches the target trades those for arrivals.
    /// Enemy-vs-enemy rams then rise again with the brake on because
    /// braked tanks hold station in contact rather than shoving past.
    ///
    /// So these numbers bound the brake, not the feel: 18 rams on a player
    /// who never moves is a tuning question (`enemy_separation_px` is not
    /// the lever - sweeping it 12 -> 64 moves the player column by less
    /// than its seed-to-seed spread), and it belongs to whoever tunes
    /// aggression, not to this test.
    #[test]
    fn enemies_pull_up_short_instead_of_ramming() {
        let map = "version = 1\ntanks = 5\ncells.\"2,2\" = { kind = \"frog\" }\ncells.\"20,11\" = { kind = \"start\" }\n";
        let (mut pair, mut into_player) = (0, 0);
        for seed in 0..4u64 {
            let mut game = Game::default();
            game.enemy_count_override = Some(5);
            game.seed_override = Some(77 + seed);
            game.map = MapFile::from_toml_str(map).expect("test map parses");
            game.init(W, H);
            for _ in 0..1200 {
                game.update(Input::default(), 1.0 / 60.0, W, H);
                for e in game.events() {
                    if let Event::Ram { slot, other_slot, .. } = e {
                        if *slot == 0 || *other_slot == 0 {
                            into_player += 1
                        } else {
                            pair += 1
                        }
                    }
                }
            }
        }
        // Measured 3 and 22 on these four seeds; the headroom is one
        // seed's worth of spread, so a real regression still trips this.
        assert!(
            into_player <= 5,
            "enemies rammed the parked player {into_player} times; measured 3 here and 27 over 16 seeds \
             with the brake disabled, and the point of the brake is that they stop short"
        );
        assert!(pair <= 30, "enemies rammed each other {pair} times; measured 22 here");
    }
}
