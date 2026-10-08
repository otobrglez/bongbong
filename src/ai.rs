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
    /// The map cell an enemy's rod reticle is to walk to (`rod::Steer::aim`,
    /// docs/rod-from-god.md): the AI's stick for it. AI-only, like
    /// `fire_aim_offset`; never on the wire, and a seat's is always `None`.
    pub aim_cell: Option<(i32, i32)>,
    /// Let a charge in progress go without firing it (`SpecialUse::Drop`):
    /// the collect pass lapses it. AI-only, like `aim_cell`.
    pub drop_charge: bool,
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

/// What `enemy_phase` measured for the special weapon a tank carries, for
/// that weapon's rule (`special_rule`, docs/sonic-hammer.md "The AI hook
/// for special weapons"): one variant per special whose use needs more
/// than the tree's own perception, every seat in it already held to the
/// sight-box rule. `None` for every other tank.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum SpecialSense {
    #[default]
    None,
    Hammer(HammerSense),
    Emp(EmpSense),
    Gauss(GaussSense),
    Fpv(FpvSense),
    Rod(RodSense),
    Well(WellSense),
}

/// What an enemy carrying the gravity well is handed this frame
/// (`Game::well_senses`, docs/gravity-well.md "What it is handed"): every
/// seat in its plan already held to the sight-box rule and to what the
/// tank knows.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WellSense {
    /// What a launch now would be for, `None` when nothing qualifies.
    pub plan: Option<WellPlan>,
    /// Its orb in flight: how far it has flown and whether, where it
    /// stands now, anchoring would drag more allies than seats.
    pub orb: Option<(f32, bool)>,
}

/// A well tank's launch (`WellSense::plan`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct WellPlan {
    /// The facing to launch along and how far along it to anchor (px from
    /// the gun line's muzzle).
    pub face: Dir,
    pub anchor_px: f32,
    /// The seat the well is used on - the lowest seat it counts - what
    /// `Ai::shot_at_seat` records at the launch.
    pub at_seat: Option<u8>,
    /// "clump", "trouble", "guard" or "shield": the trace's word.
    pub why: &'static str,
}

/// A gravity well's pull on an enemy this frame (`Game::pull_senses`,
/// docs/gravity-well.md "The `pull` tier"), set before it thinks: handed
/// to every enemy whose centre lies within a forming or pulling well's
/// reach, or within `enemy_danger_clear_px` past it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PullSense {
    /// The centre of the well whose pull is strongest on it (the lower id
    /// on a tie) and the field's pull there - at the pull's start for a
    /// forming well.
    pub core: Position,
    pub pull: Vec2,
    /// It is heavy (`tank_mass_factor` at or over `well_ai_heavy_mass`) and
    /// its tracks hold broadside where it stands (`well::holds_broadside`).
    pub brace: bool,
    /// Its centre stands inside a reach, not only within the clear margin.
    pub inside: bool,
}

/// What an enemy carrying the rod from god is handed this frame
/// (`Game::rod_senses`, docs/rod-from-god.md "What it is handed"): every
/// seat in its pick already held to the sight-box rule and to what the
/// tank knows.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RodSense {
    /// What it would call now; `None` when nothing qualifies.
    pub pick: Option<RodPick>,
    /// Its centre stands inside a call's danger: a reticle it holds is
    /// dropped and the dodge takes it out.
    pub under_call: bool,
    /// The nearest seat it knows of from inside that seat's sight box: the
    /// one it keeps its distance from (`rod_rule`'s stand-off). A tank that
    /// carries calls fires no shells, so one parked beside a seat - in the
    /// circle a call on it would crush - does nothing at all.
    pub keep_from: Option<Position>,
    /// A seat it would call on but for its own hull in the circle: what
    /// backs it off even from where it holds (`rod_rule`).
    pub self_blocks: bool,
}

/// A rod tank's target (`RodSense::pick`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct RodPick {
    /// The cell its reticle goes to and the call lands on.
    pub cell: (i32, i32),
    /// The seat the call is used on, for a seat's pick: what
    /// `Ai::shot_at_seat` records at the release.
    pub at_seat: Option<u8>,
    /// "camper", "lead", "tower" or "frog": the trace's word.
    pub why: &'static str,
}

/// What an enemy carrying the FPV swarm measured this frame
/// (`Game::fpv_senses`, docs/fpv-swarm.md "AI"): every seat in it already
/// held to the sight-box rule - it stands inside that seat's box - and to
/// what the tank knows (within its sight under the sky, not hidden from it
/// in tall grass).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct FpvSense {
    /// The seat a drone launched now would go for, not under a tree's
    /// crown, nearest first (ties to the lower seat), and where it stands.
    pub at_seat: Option<(u8, Position)>,
    /// The nearest seat it knows of that *is* under a tree's crown, and the
    /// centre of that tree: a drone dived into the crown breaks the cover.
    pub canopy: Option<(u8, Position)>,
    /// A hunter's quarry - the players' frog, alive, within its sight, not
    /// under a crown.
    pub quarry: Option<Position>,
    /// The seat it would launch at has a line of sight to it.
    pub exposed: bool,
    /// Where to launch from instead while exposed: a spot nearby that the
    /// seat cannot see, still inside its box and out of its face
    /// (`fpv_cover_spot`, latched on the `Ai`).
    pub cover: Option<Position>,
    /// A seat it knows of is within `fpv_ai_min_range_px` and sees it: the
    /// point it backs off to.
    pub back_off: Option<Position>,
    /// Its own drones in the air (`Tank::fpv_out`).
    pub in_air: u8,
}

/// A seat's drone coming at this tank (`Game::air_threats`,
/// docs/fpv-swarm.md "Reacting to a seat's swarm") - locked on it, or
/// going for a point its blast would reach it at - set on the `Ai` before it
/// thinks: what its `air` tier answers.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AirThreat {
    /// The drone's ground point, its height over it, and its seconds to
    /// arrive at its speed.
    pub drone: Position,
    pub height: f32,
    pub eta: f32,
    /// This tank stands inside the sight box of the seat that sent it: it
    /// may fire at the drone.
    pub may_shoot: bool,
    /// Its hull is under a tree's crown already: it stands.
    pub covered: bool,
    /// The nearest tree within `fpv_ai_tree_px` it can get under: the open
    /// cell nearest that tree it can drive to, and the tree's centre, which
    /// it drives at from there until its hull is under the crown.
    pub tree: Option<(Position, Position)>,
}

/// What a gauss rail's slug from where a tank stands would go through, each
/// way it could face (`Game::gauss_senses`, docs/gauss-rail.md "AI"): its
/// own stop rule (iron stops it - an enemy never overcharges).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GaussSense {
    /// One per facing, `Dir::index` order.
    pub lanes: [GaussLane; 4],
}

/// One way a gauss rail could face (`GaussSense::lanes`). A seat counts
/// only where the slug would go through it, this tank stands in its sight
/// box, it is within this tank's sight under the sky (`Game::sight_on`)
/// and it is not hidden from it (concealed and not hit-alerted) - cover in
/// between does not hide it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GaussLane {
    /// The nearest seat along the lane that counts, and how far along it
    /// stands: the seat the slug is used on (`Ai::shot_at_seat`).
    pub at_seat: Option<(u8, f32)>,
    /// Seats that count.
    pub seats: u8,
    /// Standing player towers it would go through.
    pub towers: u8,
    /// The players' frog it would go through, and how far along - read
    /// only for a hunter, whose quarry it is.
    pub quarry: Option<f32>,
    /// The nearest live fellow enemy, standing enemy tower or the enemies'
    /// own frog it would go through, and how far along: nothing fires this
    /// way at a target beyond it.
    pub friend: Option<f32>,
    /// `at_seat` stands inside its sight box and this tank's sight by
    /// `gauss_ai_box_margin_px` as well: a charge starts only on such a
    /// lane, so a tank at the box's edge, where a pixel of drift takes the
    /// seat out of it, does not charge only to lose it at full.
    pub settled: bool,
}

impl GaussLane {
    /// What the lane is worth: two a seat, two the quarry, one a player
    /// tower.
    pub fn score(&self) -> i32 {
        2 * self.seats as i32 + if self.quarry.is_some() { 2 } else { 0 } + self.towers as i32
    }

    /// Whether a slug down this lane is worth firing: worth at least two,
    /// with a seat or the quarry to fire at, and no friend before it. A
    /// friend beyond the target is the slug's to go on through, as a shell
    /// that misses goes on: holding fire for it left two rail tanks either
    /// side of a seat each waiting on the other.
    pub fn counts(&self) -> bool {
        self.score() >= 2 && self.target_along().is_some_and(|t| self.friend.is_none_or(|f| t < f))
    }

    /// How far along its first target stands: the seat, else the quarry.
    pub fn target_along(&self) -> Option<f32> {
        self.at_seat.map(|(_, d)| d).or(self.quarry)
    }

    /// The arm's name, for the trace.
    fn why(&self) -> &'static str {
        if self.seats >= 2 {
            "rail-two"
        } else if self.seats == 1 && self.towers > 0 {
            "rail-tower"
        } else if self.seats == 1 {
            "rail"
        } else {
            "rail-quarry"
        }
    }
}

/// What a pulse of an EMP tank's own would get it where it stands
/// (`Game::emp_senses`, docs/emp-burst.md "AI"). A seat counts only from
/// inside its sight box and not hidden from this tank; `off_box` flags one
/// in reach that would see the tank from outside it, which holds the
/// pulse whole.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct EmpSense {
    /// What a pulse from here is worth: each seat in reach by
    /// `emp::seat_value`, each standing, online player tower in reach
    /// `emp_ai_tower_value`.
    pub value: i32,
    /// The seat the pulse is used on: the most valuable in reach, ties to
    /// the lower seat. What `Ai::shot_at_seat` records.
    pub at_seat: Option<u8>,
    /// A seat in reach would see this tank from outside its sight box.
    pub off_box: bool,
    /// A fellow enemy - live, not disabled, not this one - whose hull is,
    /// or by the time a pulse decided now lands will be, within
    /// `emp_radius_px + emp_ai_friend_margin_px`.
    pub friends: bool,
    /// A standing, online enemy tower within reach.
    pub friendly_tower: bool,
    /// The seat this tank fights, valued alone whatever the range, and
    /// whether a fellow enemy's hull stands within `emp_radius_px` of it:
    /// what the approach weighs.
    pub target_value: i32,
    pub target_crowded: bool,
    /// This tank is one of the `emp_ai_closers` EMP tanks nearest the seat
    /// it fights (ties on slot): it may close in.
    pub closer: bool,
}

/// A place an enemy keeps out of this tick, because a weapon could go off
/// on it there (docs/emp-burst.md "Reacting to the EMP"): built once a frame
/// by `enemy_phase` - empty unless a weapon makes one, so a round without
/// decides exactly what it did - and kept out of by the `dodge` tier and by
/// every point the tree steers at (`Brain::out_of_danger`). The latch and
/// the steering read only `depth` and `exit`, so a weapon adds a shape (the
/// rail's lane, the rod's circle) with its own arm of those two.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Danger {
    pub shape: DangerShape,
    /// The owner slot of the tank whose weapon it is: that tank never shies
    /// from it. `None` for one nobody owns, which every enemy keeps out of.
    pub owner: Option<usize>,
    /// How deep its outer band runs (px) in which a tank only stops, going
    /// no deeper, rather than backing out: the berth round a danger that is
    /// over in a moment (an ally's crackle), where turning round and back
    /// would only spin the tank. 0 backs out from the edge.
    pub slack: f32,
}

/// A danger's ground (`Danger`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum DangerShape {
    /// Everything within `radius` of `at`: an armed EMP's reach, an ally's
    /// crackle.
    Disc { at: Position, radius: f32 },
    /// A charging gauss rail's lane (docs/gauss-rail.md): everything within
    /// `half_width` of the line from `from` (its gun line's muzzle) along
    /// `dir`, `length` px out - where its slug would go through a hull.
    /// `at` is the charging hull.
    Lane { at: Position, from: Position, dir: Dir, length: f32, half_width: f32 },
}

impl Danger {
    /// How far `p` stands inside it, px (negative outside).
    pub fn depth(&self, p: Position) -> f32 {
        match self.shape {
            DangerShape::Disc { at, radius } => radius - p.distance_to(at),
            DangerShape::Lane { from, dir, length, half_width, .. } => {
                let (along, across) = lane_offsets(from, dir, p);
                if along < 0.0 {
                    along
                } else if along > length {
                    length - along
                } else {
                    half_width - across.abs()
                }
            }
        }
    }

    /// Whether it is a charging rail's lane.
    pub fn is_lane(&self) -> bool {
        matches!(self.shape, DangerShape::Lane { .. })
    }

    /// The points `clear` px outside it nearest `p`, best first: the
    /// nearest, then ones turned further and further from it, for when the
    /// nearest lies off the field or in a wall (`Brain::way_out` takes the
    /// first it can reach). `from` is the tank asking and `facing` its
    /// facing, read only where `p` gives no way.
    pub fn exits(&self, p: Position, clear: f32, from: Position, facing: Dir) -> [Position; 9] {
        match self.shape {
            // Along the line from its middle through `p` - through `from`
            // when `p` stands on the middle (a seat steered at is the middle
            // of its own danger), along `facing` when both do - then that
            // line turned 30, 60, 90 and 120 degrees each way, never back
            // through the middle.
            DangerShape::Disc { at, radius } => {
                const TURNS: [(f32, f32); 9] = [
                    (1.0, 0.0),
                    (0.866_025_4, 0.5),
                    (0.866_025_4, -0.5),
                    (0.5, 0.866_025_4),
                    (0.5, -0.866_025_4),
                    (0.0, 1.0),
                    (0.0, -1.0),
                    (-0.5, 0.866_025_4),
                    (-0.5, -0.866_025_4),
                ];
                let unit = |q: Position| {
                    let away = Vec2::new(q.x - at.x, q.y - at.y);
                    let len = away.length();
                    (len > 1.0).then(|| Vec2::new(away.x / len, away.y / len))
                };
                let d = unit(p).or_else(|| unit(from)).unwrap_or_else(|| facing.vec());
                let reach = radius + clear;
                TURNS.map(|(c, s)| Position::new(at.x + (d.x * c - d.y * s) * reach, at.y + (d.x * s + d.y * c) * reach))
            }
            // Straight out of a side: the one `facing` points across to,
            // so a tank crossing the lane goes on over rather than turning
            // back (a reversal and back is a spin); for a tank facing along
            // it, the near one - `p`'s side, else the side `from` stands
            // on, else the one clockwise of the lane. At `p`'s distance
            // along it, then a cell and two either way along it, then the
            // far side the same.
            DangerShape::Lane { from: muzzle, dir, length, half_width, .. } => {
                let (along, across) = lane_offsets(muzzle, dir, p);
                let n = Vec2::new(-dir.vec().y, dir.vec().x);
                let f = facing.vec();
                let turn = f.x * n.x + f.y * n.y;
                let side = if turn.abs() > 0.5 {
                    turn.signum()
                } else if across.abs() > 0.5 {
                    across.signum()
                } else {
                    let (_, a) = lane_offsets(muzzle, dir, from);
                    if a.abs() > 0.5 { a.signum() } else { 1.0 }
                };
                let at = along.clamp(0.0, length);
                let cell = OBSTACLE_GRID_SIZE;
                let reach = half_width + clear;
                let point = |s: f32, k: f32| {
                    let a = (at + k * cell).clamp(0.0, length);
                    Position::new(muzzle.x + dir.vec().x * a + n.x * s * reach, muzzle.y + dir.vec().y * a + n.y * s * reach)
                };
                [
                    point(side, 0.0),
                    point(side, 1.0),
                    point(side, -1.0),
                    point(side, 2.0),
                    point(side, -2.0),
                    point(-side, 0.0),
                    point(-side, 1.0),
                    point(-side, -1.0),
                    point(-side, 2.0),
                ]
            }
        }
    }

    /// The points `clear` px outside it on the axes through its middle, in
    /// `Dir::ALL` order: where a tank fighting what stands at the middle
    /// can wait lined up on it.
    pub fn posts(&self, clear: f32) -> [Position; 4] {
        match self.shape {
            DangerShape::Disc { at, radius } => {
                Dir::ALL.map(|d| Position::new(at.x + d.vec().x * (radius + clear), at.y + d.vec().y * (radius + clear)))
            }
            // Beside the charging hull and behind it - never ahead, which is
            // the lane.
            DangerShape::Lane { at, dir, half_width, .. } => Dir::ALL.map(|d| {
                let d = if d == dir { dir.opposite() } else { d };
                Position::new(at.x + d.vec().x * (half_width + clear), at.y + d.vec().y * (half_width + clear))
            }),
        }
    }

    /// The point it is drawn round: a disc's middle, a lane's charging
    /// hull.
    pub fn middle(&self) -> Position {
        match self.shape {
            DangerShape::Disc { at, .. } | DangerShape::Lane { at, .. } => at,
        }
    }

    /// The nearest point `clear` px outside it from `p` (`exits`' first).
    pub fn exit(&self, p: Position, clear: f32, from: Position, facing: Dir) -> Position {
        self.exits(p, clear, from, facing)[0]
    }
}

/// What a sonic hammer's blast each way would do (`Game::hammer_senses`).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HammerSense {
    /// One per facing, `Dir::index` order.
    pub aims: [HammerAim; 4],
    /// Where this tank closes in to, one of the `sonic_ai_closers` hammer
    /// tanks nearest the seat it fights: a spot square on the seat from
    /// which a blast reaches it, with no fellow enemy in the way
    /// (`simulation::sonic::closer_spots`). `None` for every other tank.
    pub spot: Option<Position>,
}

/// A blast one way (`HammerSense::aims`). A seat is named only if this
/// tank stands inside its sight box, and - but for `flush` - only if the
/// seat is not hidden from it.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct HammerAim {
    /// A seat the knock would slide into trouble.
    pub trouble: Option<u8>,
    /// A seat a drum in the cone would be thrown onto.
    pub drum: Option<u8>,
    /// The hidden seat whose grass round its alert point the wave would
    /// flatten.
    pub flush: Option<u8>,
    /// A seat in the cone within `sonic_ai_breaker_px`.
    pub breaker: Option<u8>,
    /// A hunter's quarry in the cone, not stunned.
    pub frog: bool,
    /// A live fellow enemy in the cone: nothing fires this way.
    pub friend: bool,
}

/// How far `p` stands along a lane from `from` along `dir`, and across it
/// (positive on the side clockwise of `dir`).
fn lane_offsets(from: Position, dir: Dir, p: Position) -> (f32, f32) {
    let d = dir.vec();
    let rel = Vec2::new(p.x - from.x, p.y - from.y);
    (rel.x * d.x + rel.y * d.y, rel.x * -d.y + rel.y * d.x)
}

/// What a special's rule asks of the tank this tick (`special_rule`).
#[derive(Clone, Copy, Debug, PartialEq)]
enum SpecialUse {
    /// Face `face` and pull the trigger (the simulation runs the weapon's
    /// tell first); `at_seat` is the seat it is used on, `why` the arm.
    Fire { face: Dir, at_seat: Option<u8>, why: &'static str },
    /// Hold still facing `face`; `why` for the trace.
    Hold { face: Dir, why: &'static str },
    /// Have the tank's ring of `radius` px cleared of its own side before
    /// it fires (`Ai::clearing`: a danger its allies keep out of, and
    /// `simulation::command`'s `clear_rings` with the commander on), the
    /// tank fighting on meanwhile.
    Clear { radius: f32 },
    /// Drive to `to`: to bring a short-range weapon to bear, or to a spot
    /// a weapon is used from. `why` for the trace.
    Approach { to: Position, why: &'static str },
    /// Hold the trigger of a charge weapon facing `face` (the charge-and-hold
    /// pattern, docs/gauss-rail.md): a press starts a charge, holding keeps
    /// it, and the tank stands its ground meanwhile. `aim` is the cell a
    /// rod's reticle walks to (`Intent::aim_cell`); `None` for the rail.
    Charge { face: Dir, aim: Option<(i32, i32)>, why: &'static str },
    /// Let a charge in progress go without firing it (`Intent::drop_charge`):
    /// a rod tank whose target no longer holds, any charging tank a call
    /// stands over - where the tree goes on below, to the dodge.
    Drop { why: &'static str },
    /// Let a charge weapon's trigger go facing `face`: the simulation fires
    /// the charge if it is ready. `at_seat` is the seat it is used on.
    Release { face: Dir, at_seat: Option<u8>, why: &'static str },
    /// Send something after `want` (`fpv::AirWant`, the FPV swarm): applied
    /// as `Fire` along the facing it has, the want handed to the launch
    /// (`Ai::air_want`). `at_seat` is the seat it is used on.
    Launch { want: crate::fpv::AirWant, at_seat: Option<u8>, why: &'static str },
    /// Launch a gravity well's orb facing `face` (docs/gravity-well.md): as
    /// `Fire`, with the distance its rule anchors it at remembered
    /// (`Ai::well_anchor_px`).
    Orb { face: Dir, anchor_px: f32, at_seat: Option<u8>, why: &'static str },
    /// Press the trigger to anchor the orb in flight, facing `face`: an
    /// edge that touches neither the fire timer nor `shot_at_seat`.
    Anchor { face: Dir },
}

/// The BB-36 weapons' crates an enemy detours for, in the order it wants
/// them (`build`'s `seek_special` tier, after the minigun's).
pub const SEEK_SPECIALS: [PickupKind; 6] =
    [PickupKind::SonicHammer, PickupKind::Emp, PickupKind::GaussRail, PickupKind::FpvSwarm, PickupKind::RodFromGod, PickupKind::GravityWell];

/// Whether the tree's generic tiers - attack, snipe, grudge, breach - may
/// pull the trigger on `weapon`: false for a special whose own rule owns
/// it (`special_rule`).
pub fn generic_fire(weapon: ActiveWeapon) -> bool {
    !matches!(
        weapon,
        ActiveWeapon::SonicHammer | ActiveWeapon::Emp | ActiveWeapon::GaussRail | ActiveWeapon::FpvSwarm | ActiveWeapon::RodFromGod | ActiveWeapon::GravityWell
    )
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
    /// The arm of its special's rule the last `special` tier ran
    /// (`SpecialUse::Fire`'s `why`, "hold", "approach"), `None` when it ran
    /// none. Inspection only, like `last_action`.
    special_why: Option<&'static str>,
    /// Where a rod tank is moving to in its stand-off (`rod_rule`), held
    /// until it gets there so a band shared with a moving ally is not a
    /// choice made again every tick; dropped the tick after it stands off
    /// no more.
    rod_spot: Option<Position>,
    /// Seconds a rod tank stands where it is in its stand-off rather than
    /// make for a spot (`rod_rule`): set when a move to one stopped
    /// against a tank or a wall, so a narrow place is not ground against.
    rod_wait: f32,
    /// It held its stand-off last tick: it backs off only from a cell
    /// nearer, so a hull sliding on after it stopped - wet ground, ice - is
    /// not sent back out again (`rod_rule`).
    rod_held: bool,
    /// The nearest it has come to `rod_spot` since it chose it (px): a
    /// move that has drifted a cell further than that is making no
    /// headway, and is given up as a blocked one is.
    rod_spot_best: f32,
    /// Its brain is off (an EMP, docs/emp-burst.md): `enemy_phase` coasts it
    /// and does not call `think`; the first tick it is back, `reboot`.
    pub(crate) down: bool,
    /// The ring its EMP rule asked to have cleared of its own side this tick
    /// (`SpecialUse::Clear`) - a danger its allies keep out of on the next
    /// (`Game::emp_dangers`), and the commander's to clear when it is on -,
    /// `None` the rest of the time.
    clearing: Option<f32>,
    /// Backing out of a danger (`Danger`, the `dodge` tier): latched until
    /// the tank stands `enemy_danger_clear_px` outside it, so the edge is
    /// never a place to jitter.
    dodging: bool,
    /// The way out it is backing out to while `dodging`: one of the
    /// danger's `Danger::exits` as it stood when chosen, kept while it
    /// stays out of every danger and the tank can still walk to it, and
    /// chosen again only when not. The exits turn with the tank's bearing
    /// from the danger's middle, so one chosen afresh every tick slides as
    /// the tank moves - in and out of reach at a shore or a wall's margin,
    /// off the field's edge - and the tank would turn back and forth
    /// between it and the next one round.
    dodge_exit: Option<Position>,
    /// Waiting at the spot outside a danger its steering target was moved
    /// to (`act_attack`) this tick: a hold of its own choosing.
    kept_out: bool,
    /// Seconds its EMP rule has held the pulse for allies in its ring
    /// (`SpecialUse::Clear`), running while they stay in it and back to 0
    /// once none is: past `emp_ai_clear_patience_seconds` it stops asking.
    clear_waited: f32,
    /// What its FPV rule's launch this tick asked a drone to lock
    /// (`SpecialUse::Launch`): handed to the tank before its trigger
    /// reaches the simulation (`Tank::fpv_want`), `None` the rest of the
    /// time.
    pub(crate) air_want: Option<crate::fpv::AirWant>,
    /// The cover spot its FPV rule is making for and the seconds since it
    /// was chosen (`Game::fpv_senses` keeps it, searching again only when
    /// it no longer hides it or `fpv_ai_cover_seconds` have passed).
    pub(crate) cover_spot: Option<(Position, f32)>,
    /// Seconds its FPV rule has spent driving to a place to launch from -
    /// making for cover or backing off (`SpecialUse::Approach` "to cover",
    /// "back off") - with no launch: past `fpv_ai_cover_seconds` it launches
    /// from where it stands. Back to 0 on a launch and whenever it stands in
    /// cover from the seat it would launch at (`Game::fpv_senses`).
    pub(crate) place_waited: f32,
    /// A seat's drone locked on it this frame (`Game::air_threats`), set
    /// before it thinks; what the `air` tier answers.
    pub(crate) air_threat: Option<AirThreat>,
    /// The arm of the `air` tier it ran this tick ("flak", "tree",
    /// "canopy", "break"), `None` when it ran none. Inspection only.
    air_why: Option<&'static str>,
    /// A gravity well's pull on it this frame (`Game::pull_senses`), set
    /// before it thinks; what the `pull` tier answers.
    pub(crate) pull: Option<PullSense>,
    /// The way it escapes a pull along (`act_pull`), latched until it
    /// stands `enemy_danger_clear_px` outside every reach.
    pub(crate) pull_escape: Option<Dir>,
    /// The arm of the `pull` tier it ran this tick ("brace", "across"),
    /// `None` when it ran none. Inspection only.
    pull_why: Option<&'static str>,
    /// How far its orb flies before its rule anchors it (px from the
    /// muzzle): its plan's, set at the launch (`SpecialUse::Orb`).
    well_anchor_px: f32,
    /// The plan its well's senses handed it this tick (`WellSense::plan`).
    /// Inspection only.
    well_plan: Option<WellPlan>,
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
    /// The special's rule's last arm (`Ai::special_why`).
    pub special: Option<&'static str>,
    /// The `air` tier's last arm (`Ai::air_why`).
    pub air: Option<&'static str>,
    /// The arm of the `pull` tier it ran this tick (docs/gravity-well.md).
    pub pull: Option<&'static str>,
    /// Its well's plan this tick: the arm, the facing and the anchor's
    /// distance from the muzzle (`WellSense::plan`).
    pub well: Option<(&'static str, &'static str, f32)>,
    /// Its brain is off (`Ai::down`).
    pub down: bool,
    /// Backing out of a danger (`Ai::dodging`).
    pub dodging: bool,
    /// Waiting outside a danger (`Ai::kept_out`).
    pub kept_out: bool,
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
            special_why: None,
            rod_spot: None,
            rod_wait: 0.0,
            rod_held: false,
            rod_spot_best: 0.0,
            target_player: 0,
            field: FieldMind::default(),
            down: false,
            clearing: None,
            dodging: false,
            dodge_exit: None,
            kept_out: false,
            clear_waited: 0.0,
            air_want: None,
            cover_spot: None,
            place_waited: 0.0,
            air_threat: None,
            air_why: None,
            pull: None,
            pull_escape: None,
            pull_why: None,
            well_anchor_px: 0.0,
            well_plan: None,
        }
    }
}

impl Ai {
    /// Sets the trigger's pacing timer, for a test that starts a tank mid
    /// interval.
    #[cfg(test)]
    pub(crate) fn set_fire_timer(&mut self, seconds: f32) {
        self.fire_timer = seconds;
    }

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
    /// `sense` is what the simulation measured for the special this tank
    /// carries (`SpecialSense`), for that weapon's rule (`special_rule`).
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
        sense: &SpecialSense,
        dangers: &[Danger],
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
        // Knocked off its tracks (`Tank::skid`): the slide it did not ask
        // for is evidence of nothing, so the stuck and breach clocks stand
        // still until it ends (docs/sonic-hammer.md).
        let skidding = me.skid > 0.0;
        match (self.last_move_dir, moved) {
            _ if skidding => {}
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
        if !skidding {
            if into_wall {
                self.wall_ahead_timer += dt;
            } else {
                self.wall_ahead_timer = 0.0;
            }
        }
        if let Some(breach) = &mut self.breach {
            breach.timer -= dt;
        }

        if self.special_why != Some("stand-off") {
            self.rod_spot = None;
            self.rod_held = false;
        }
        self.rod_wait = (self.rod_wait - dt).max(0.0);
        self.special_why = None;
        self.clearing = None;
        self.kept_out = false;
        self.air_want = None;
        self.air_why = None;
        self.pull_why = None;
        self.well_plan = match sense {
            SpecialSense::Well(s) => s.plan,
            _ => None,
        };
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
            sense,
            dangers,
        };
        let mut last_action = None;
        build().tick_traced(&mut bb, &mut last_action);
        let mut intent = bb.intent;
        // The wait on allies in its ring ends with them.
        if !matches!(sense, SpecialSense::Emp(s) if s.friends) {
            self.clear_waited = 0.0;
        }

        // A charging rail's lane (docs/gauss-rail.md "Reacting to a rail"):
        // a step into one this tank does not own, from outside every one,
        // is not taken - it waits at the edge, facing as it is, rather than
        // walk in for the dodge to push it back out again, until the
        // charge ends. The dodge's own way out is never held.
        if let Some(dir) = intent.move_dir
            && last_action != Some("dodge")
            && enters_lane(me, dir, dangers)
        {
            intent.move_dir = None;
            self.kept_out = true;
        }

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
            special: self.special_why,
            air: self.air_why,
            pull: self.pull_why,
            well: self.well_plan.map(|p| (p.why, p.face.name(), p.anchor_px)),
            down: self.down,
            dodging: self.dodging,
            kept_out: self.kept_out,
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

    /// The brain is back after an EMP (docs/emp-burst.md "An enemy struck:
    /// brain off"): what the coast made a lie of is cleared - the stuck
    /// clock's baseline, the heading commitment, the dodge, the yield, the
    /// breach evidence and a latched breach, the aim - and what is about
    /// the fight (role, alerts, target, fire timer, retreat, escapes) is
    /// kept. Its clocks were frozen meanwhile, not paid back.
    pub(crate) fn reboot(&mut self) {
        self.down = false;
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
        self.wall_ahead_timer = 0.0;
        self.breach = None;
        self.aim_settle = 0.0;
        self.dodging = false;
        self.dodge_exit = None;
    }

    /// The ring this tank's EMP rule asked to have cleared this tick, if
    /// any (`SpecialUse::Clear`): what the collect pass hands the commander
    /// and the next frame's dangers read.
    pub(crate) fn clearing(&self) -> Option<f32> {
        self.clearing
    }

    /// Whether it is backing out of a danger (`dodging`, the latch).
    pub(crate) fn dodging(&self) -> bool {
        self.dodging
    }

    /// Whether its `pull` tier ran this tick: bracing or driving across a
    /// gravity well's pull.
    pub(crate) fn pulled(&self) -> bool {
        self.pull_why.is_some()
    }

    /// Whether it braced broadside in a pull this tick (the `pull` tier's
    /// "brace").
    pub fn bracing(&self) -> bool {
        self.pull_why == Some("brace")
    }

    /// Whether it held still for its orb in flight this tick (the well's
    /// rule's `Hold "orb"` or its anchor press).
    pub fn anchoring(&self) -> bool {
        matches!(self.special_why, Some("orb") | Some("anchor"))
    }

    /// Whether it waited outside a danger this tick (`kept_out`).
    /// The behaviour tree's last leaf and its special's arm, for the
    /// probe's trace (`TankSnapshot::action`).
    pub fn action(&self) -> (Option<&'static str>, Option<&'static str>) {
        (self.last_action, self.special_why.or(self.pull_why))
    }

    pub(crate) fn kept_out(&self) -> bool {
        self.kept_out
    }

    /// Whether it stood this tick on purpose for the FPV swarm
    /// (docs/fpv-swarm.md "AI"): watching its own drone work, or under a
    /// tree's crown while a seat's drone comes at it.
    pub(crate) fn air_hold(&self) -> bool {
        self.special_why == Some("watch") || self.air_why == Some("canopy")
    }

    /// Whether it kept its distance from a seat this tick on purpose, the
    /// rod from god's stand-off (docs/rod-from-god.md "AI"): a tank that
    /// carries calls holds a band out of the circle a call on that seat
    /// would crush.
    pub(crate) fn rod_hold(&self) -> bool {
        self.special_why == Some("stand-off")
    }

    /// Whether its fire timer has run out: a rod tank still waiting on it
    /// takes no target from the tanks after it (`Game::rod_senses`).
    pub(crate) fn fire_ready(&self) -> bool {
        self.fire_timer <= 0.0
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

    /// Whether its last think kept a guard's beat (`Role::Guard`'s tier):
    /// the seat it would fight is far from the frog it guards.
    pub(crate) fn holds_beat(&self) -> bool {
        self.last_action == Some("guard")
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
    /// What the simulation measured for its special - see `think`'s
    /// `sense` parameter.
    sense: &'a SpecialSense,
    /// The places it keeps out of this tick - see `think`'s `dangers`
    /// parameter.
    dangers: &'a [Danger],
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
        self.out_of_danger(self.engage_target.unwrap_or(self.target))
    }

    /// The danger this tank stands inside and does not own, the deepest
    /// (ties to the earlier), while `dodging` is not latched; latched, the
    /// first it is not yet `enemy_danger_clear_px` clear of. `None` when it
    /// stands clear of every one.
    fn danger_here(&self) -> Option<Danger> {
        let me = self.me.position;
        let clear = tuning().enemy_danger_clear_px;
        let mine = self.me.owner_slot();
        let others = self.dangers.iter().filter(|d| d.owner != Some(mine));
        if self.ai.dodging {
            return others.copied().find(|d| d.depth(me) > -clear);
        }
        others.fold(None, |best: Option<Danger>, d| {
            let depth = d.depth(me);
            if depth > 0.0 && best.is_none_or(|b| depth > b.depth(me)) { Some(*d) } else { best }
        })
    }

    /// `p`, or - where it lies inside a danger this tank does not own - its
    /// way out of the first that holds it (`Danger::exit`, the clear margin
    /// past the edge): what the tree steers at, so a tank kept out of a
    /// danger is never sent back into it. Aim and fire read the real point.
    fn out_of_danger(&self, p: Position) -> Position {
        let mine = self.me.owner_slot();
        let facing = Dir::from_rotation(self.me.rotation).unwrap_or(Dir::Up);
        match self.dangers.iter().find(|d| d.owner != Some(mine) && d.depth(p) > 0.0) {
            Some(d) => self.way_out(d, p, tuning().enemy_danger_clear_px + OBSTACLE_GRID_SIZE, facing),
            None => p,
        }
    }

    /// The first of `danger`'s exits from `p` - from where this tank stands,
    /// for a rod's call it stands outside - this tank can drive to - on
    /// the field, in a usable cell, joined to where it stands - else the
    /// nearest. A disc round a seat by the field's edge has its nearest
    /// exit off the field, and steering at that wanders.
    fn way_out(&self, danger: &Danger, p: Position, clear: f32, facing: Dir) -> Position {
        let reachable = |q: &Position| self.can_reach(*q);
        // A seat steered at stands at the middle of its own danger: wait
        // lined up on it, inside its sight box where a post is (a shot is
        // fired from there), then the nearest.
        if p.distance_to(danger.middle()) < 1.0 {
            let me = self.me.position;
            let mut posts = danger.posts(clear);
            posts.sort_by(|a, b| {
                let key = |q: &Position| (!in_sight_box(p, *q), me.distance_to(*q));
                key(a).partial_cmp(&key(b)).unwrap_or(std::cmp::Ordering::Equal)
            });
            if let Some(post) = posts.iter().find(|q| reachable(q)) {
                return *post;
            }
        }
        // Out of a rod's call (a danger nobody owns), from where this tank
        // stands outside it: the nearest of the circle's edge to it. The
        // side of the call's middle a seat stands on in its cell is a few
        // pixels' difference, and a chaser steered there would drive round
        // the circle; it waits on its own side for the impact.
        let zone = danger.owner.is_none() && matches!(danger.shape, DangerShape::Disc { .. }) && danger.depth(self.me.position) <= 0.0;
        let exits = danger.exits(if zone { self.me.position } else { p }, clear, self.me.position, facing);
        exits.iter().copied().find(reachable).unwrap_or(exits[0])
    }

    /// Whether this tank can drive to `q`: on the field, in a usable cell,
    /// joined to where it stands (`way_out`'s test).
    fn can_reach(&self, q: Position) -> bool {
        q.x > 0.0 && q.y > 0.0 && q.x < self.width && q.y < self.height && self.grid.usable(q) && self.grid.connected(self.me.position, q)
    }

    /// Whether it stands inside a gravity well's pull it does not brace
    /// against (docs/gravity-well.md): its special's use yields to the
    /// `pull` tier.
    fn yields_to_pull(&self) -> bool {
        self.ai.pull.is_some_and(|p| p.inside && !p.brace)
    }

    /// Whether this tank's centre stands inside a call's circle: a danger
    /// nobody owns (`Zone::danger`, docs/rod-from-god.md).
    fn in_call(&self) -> bool {
        self.dangers.iter().any(|d| d.owner.is_none() && d.depth(self.me.position) > 0.0)
    }

    /// Whether `p` lies inside a danger this tank does not own.
    fn in_danger(&self, p: Position) -> bool {
        let mine = self.me.owner_slot();
        self.dangers.iter().any(|d| d.owner != Some(mine) && d.depth(p) > 0.0)
    }

    /// Where an EMP closer closes in on `around` to: on the line from it
    /// out to `slot` (this tank's engagement slot, else its own position),
    /// `radius` out - or as near that as an open cell allows, half a cell
    /// at a time back out toward the slot. The tank's own place when it
    /// already stands nearer than `radius`. The pulse is a disc, so any
    /// side of the seat serves, and one closer (`emp_ai_closers`) has no
    /// other to keep out of the way of.
    fn close_spot(&self, around: Position, slot: Option<Position>, radius: f32) -> Position {
        let toward = slot.unwrap_or(self.me.position);
        let away = Vec2::new(toward.x - around.x, toward.y - around.y);
        let len = away.length();
        if len <= radius {
            return toward;
        }
        let dir = Vec2::new(away.x / len, away.y / len);
        let mut r = radius;
        while r < len {
            let at = Position::new(around.x + dir.x * r, around.y + dir.y * r);
            if self.grid.usable(at) {
                return at;
            }
            r += OBSTACLE_GRID_SIZE * 0.5;
        }
        toward
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
    /// A pickup inside a danger (`Danger`) is not worth it.
    fn seek(&self, kind: PickupKind) -> Option<Position> {
        let leash = self.home_leash();
        self.pickups
            .iter()
            .filter(|&&(k, at)| k == kind && leash.is_none_or(|l| at.distance_to(l.anchor) <= l.radius) && !self.in_danger(at))
            .map(|&(_, at)| at)
            .min_by(|&a, &b| self.me.position.distance_to(a).total_cmp(&self.me.position.distance_to(b)))
    }

    /// The nearest BB-36 weapon crate worth the detour: the first kind of
    /// `SEEK_SPECIALS` this tank would take (`Tank::wants_pickup`) with one
    /// in reach (`seek`).
    fn seek_special(&self) -> Option<Position> {
        SEEK_SPECIALS.into_iter().filter(|&k| self.me.wants_pickup(k)).find_map(|k| self.seek(k))
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
        // A weapon its own rule fires is no sniper's: holding for a shot
        // it never takes would only park the tank.
        if !generic_fire(self.me.active_weapon()) {
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
    /// their old player-relative behavior. A pickup inside a danger
    /// (`Danger`) is none: the dodge tier would only drive the tank back
    /// out of it, its heading committed away, to come round again.
    fn nearest_pickup(&self, kind: PickupKind) -> Option<Position> {
        self.pickups
            .iter()
            .filter(|(k, at)| *k == kind && !self.in_danger(*at))
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
    /// A special's pacing (`special_fire_interval`, up to several seconds)
    /// does not carry over to the weapon it falls back on once it is spent:
    /// the timer it left is cut to the longest a generic shot waits
    /// (`enemy_fire_interval`), or a tank that launched its last well would
    /// stand lined up on its shells for the rest of the well's interval.
    /// A timer only a generic shot or a breach set is never longer, so this
    /// changes nothing for a tank that never carried a special.
    fn cap_fire_timer(&mut self) {
        let longest = tuning().enemy_fire_interval.max(tuning().enemy_fire_interval_aggressive);
        self.ai.fire_timer = self.ai.fire_timer.min(longest);
    }

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

    /// Steer at a danger's way out (`act_dodge`) as `steer` steers - the
    /// heading held, avoidance, the stuck escape - but on foot, and with no
    /// wander when it has no route (that draws on the round's RNG): it
    /// heads straight for the point instead.
    fn steer_out(&mut self, target: Position) -> Dir {
        let ctx = self.avoid_ctx();
        let from = self.me.position;
        let route = self.grid.route_ahead_walking(from, target);
        self.ai.steer_toward(from, route, target, (self.width, self.height), ctx.radius, ctx, self.grid)
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
        // A weapon its own rule fires has no shot to send back: holding
        // for one would only park the tank in the tower's reach.
        if !grudge.in_sight || !generic_fire(self.me.active_weapon()) {
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
        // A special whose rule owns the trigger is never fired here: the
        // tank lines up and holds, no more, and spends neither its fire
        // timer nor a roll on it.
        if !generic_fire(self.me.active_weapon()) {
            return;
        }
        self.cap_fire_timer();

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
        // A special whose own rule owns the trigger breaches by that rule,
        // if at all - the sonic hammer breaks glass and nothing else.
        if !generic_fire(self.me.active_weapon()) {
            self.ai.breach = None;
            return false;
        }
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

/// Whether `me`, outside every charging rail's lane it does not own, would
/// step into one driving along `dir`: the point a hull's radius and
/// `enemy_danger_clear_px` ahead lies inside it.
fn enters_lane(me: &Tank, dir: Dir, dangers: &[Danger]) -> bool {
    let mine = me.owner_slot();
    let lanes = || dangers.iter().filter(|d| d.is_lane() && d.owner != Some(mine));
    if lanes().any(|d| d.depth(me.position) > 0.0) {
        return false;
    }
    let reach = me.avoidance_radius() + tuning().enemy_danger_clear_px;
    let ahead = Position::new(me.position.x + dir.vec().x * reach, me.position.y + dir.vec().y * reach);
    lanes().any(|d| d.depth(ahead) > 0.0)
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
        // 1.45. A seat's drone locked on it (`AirThreat`): flak, a tree's
        // crown, or a break across its line (`act_air`). Not under a tell
        // or a charge in progress, which commit, as everywhere; nor inside a
        // danger, which it backs out of first (1.6) - a drone's burst is a
        // scratch beside what a danger holds.
        sequence(vec![
            condition(|b: &mut Brain| b.ai.air_threat.is_some() && b.me.windup().is_none() && b.danger_here().is_none() && !b.yields_to_pull()),
            action("air", act_air),
        ]),
        // 1.5. The special carried has a use of its own this tick
        // (`special_rule`): a tell holding, a blast to fire, a close-range
        // weapon to bring to bear. Above flee: a hurt tank shoving away the
        // seat in its face is the use (the rule offers a healthy tank alone
        // the approach).
        sequence(vec![
            condition(|b: &mut Brain| special_rule(b).is_some()),
            action("special", act_special),
        ]),
        // 1.6. Inside a danger (`Danger`: a seat's armed EMP, an ally's
        // crackle): back out past its edge - latched until clear of it by
        // `enemy_danger_clear_px`, so the edge is not a place to jitter.
        // Below the special tier, so an EMP tank's own approach still walks
        // in on a seat armed with one; above flee, since a hurt tank is no
        // safer inside.
        sequence(vec![
            condition(|b: &mut Brain| {
                let here = b.danger_here().is_some();
                b.ai.dodging = here;
                if !here {
                    b.ai.dodge_exit = None;
                }
                here
            }),
            action("dodge", act_dodge),
        ]),
        // 1.65. In a gravity well's pull (docs/gravity-well.md "The `pull`
        // tier"): a heavy chassis whose tracks hold braces broadside, any
        // other drives across the pull on its own side of the core -
        // latched until clear of every reach by `enemy_danger_clear_px`.
        // Not under a tell, which commits.
        sequence(vec![
            condition(|b: &mut Brain| {
                if b.ai.pull.is_none() {
                    b.ai.pull_escape = None;
                }
                b.me.tell.is_none() && b.ai.pull.is_some_and(|p| p.inside || b.ai.pull_escape.is_some())
            }),
            action("pull", act_pull),
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
        // 5.75. The BB-36 weapons' crates (`SEEK_SPECIALS`), in the order
        // the AI wants them, on the weapon tiers' rule.
        sequence(vec![
            condition(|b: &mut Brain| b.seek_special().is_some()),
            action("seek_special", act_seek_special),
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

/// Head for the nearest BB-36 weapon crate worth the detour - see tier
/// 5.75 (`build`).
fn act_seek_special(b: &mut Brain) -> Status {
    b.reset_aim();
    let Some(target) = b.seek_special() else {
        return Status::Failure;
    };
    b.intent.move_dir = Some(b.steer(target));
    Status::Success
}

/// What the special `b`'s tank carries asks of it this tick
/// (docs/sonic-hammer.md "The AI hook for special weapons"): first a
/// wind-up in progress (`Tank::windup`), which belongs to its weapon's
/// rule (`windup_rule`); then one arm per weapon with a rule. `None` for no
/// use this tick - the tree goes on.
fn special_rule(b: &Brain) -> Option<SpecialUse> {
    if let Some(windup) = b.me.windup() {
        // A charge is let go in a pull it does not brace against
        // (docs/gravity-well.md); a tell, half a second long, commits.
        if b.me.charge.is_some() && b.yields_to_pull() {
            return Some(SpecialUse::Drop { why: "pull" });
        }
        return windup_rule(b, windup);
    }
    // A call stands over it (docs/rod-from-god.md "Reacting to a call"): no
    // special is used from inside the circle; the dodge takes it out. Nor
    // from inside a pull it does not brace against: the `pull` tier takes
    // it out.
    if b.in_call() || b.yields_to_pull() {
        return None;
    }
    match (b.me.active_weapon(), b.sense) {
        (ActiveWeapon::SonicHammer, SpecialSense::Hammer(sense)) => hammer_rule(b, sense),
        (ActiveWeapon::Emp, SpecialSense::Emp(sense)) => emp_rule(b, sense),
        (ActiveWeapon::GaussRail, SpecialSense::Gauss(sense)) => gauss_rule(b, sense),
        (ActiveWeapon::FpvSwarm, SpecialSense::Fpv(sense)) => fpv_rule(b, sense),
        (ActiveWeapon::RodFromGod, SpecialSense::Rod(sense)) => rod_rule(b, sense),
        (ActiveWeapon::GravityWell, SpecialSense::Well(sense)) => well_rule(b, sense),
        _ => None,
    }
}

/// The gravity well's rule (docs/gravity-well.md "The rule"): its orb in
/// flight owns the trigger - a press the tick it has flown its plan's
/// distance and anchoring there drags no more allies than seats, the
/// trigger released before -; a training dummy never launches, nor does a
/// tank still cooling; a plan launches.
fn well_rule(b: &Brain, sense: &WellSense) -> Option<SpecialUse> {
    let facing = Dir::from_rotation(b.me.rotation).unwrap_or(Dir::Up);
    if b.me.orb.is_some() {
        return Some(match sense.orb {
            Some((flown, drags)) if flown >= b.ai.well_anchor_px && !drags => SpecialUse::Anchor { face: facing },
            _ => SpecialUse::Hold { face: facing, why: "orb" },
        });
    }
    if b.ai.frog_only || b.me.fire_cooldown > 0.0 || b.ai.fire_timer > 0.0 {
        return None;
    }
    let plan = sense.plan?;
    Some(SpecialUse::Orb { face: plan.face, anchor_px: plan.anchor_px, at_seat: plan.at_seat, why: plan.why })
}

/// The `pull` tier (docs/gravity-well.md): a heavy chassis whose tracks
/// hold turns broadside - the cardinal across the pull nearer its bearing
/// to the seat it fights - and stands, firing on a seat lined up along
/// that facing as the attack tier fires; any other drives across the pull
/// on its own side of the core (`well::escape_dir`), the way latched while
/// it stays open (`Ai::pull_escape`). No RNG but the attack's own shot.
fn act_pull(b: &mut Brain) -> Status {
    let Some(pull) = b.ai.pull else { return Status::Failure };
    b.reset_aim();
    let me = b.me.position;
    let facing = Dir::from_rotation(b.me.rotation).unwrap_or(Dir::Up);
    if pull.brace && pull.inside {
        let face = crate::well::brace_dir(me, pull.core, Some(b.player.position), facing);
        b.ai.pull_why = Some("brace");
        b.ai.pull_escape = None;
        let (fire_dir, off_axis, in_front) = b.aim_alignment();
        let at_seat = !b.hunting_frog();
        if fire_dir == face && off_axis <= tuning().enemy_fire_align_px && in_front && b.line_of_sight && (!at_seat || b.may_fire_at_seat()) {
            let range = b.dist_to_target();
            b.hold_and_fire(face, range, at_seat.then_some(b.ai.target_player));
        } else {
            b.intent.face = Some(face);
            b.ai.commit(face);
        }
        return Status::Success;
    }
    let open = |d: Dir| b.walls_ahead[d.index()].is_none() && !b.grid.blocked_ahead(me, d.vec());
    let way = b.ai.pull_escape.filter(|&d| open(d)).or_else(|| crate::well::escape_dir(me, pull.core, open));
    let Some(way) = way else {
        b.ai.pull_escape = None;
        return Status::Failure;
    };
    b.ai.pull_escape = Some(way);
    b.ai.pull_why = Some("across");
    b.ai.commit(way);
    b.intent.move_dir = Some(way);
    Status::Success
}

/// The rod from god's rule with no reticle up (docs/rod-from-god.md "The
/// rule"): a training dummy never calls, nor does a rod still reloading or
/// a tank whose fire timer runs; a pick starts the reticle toward its cell.
/// Draws no RNG.
fn rod_rule(b: &Brain, sense: &RodSense) -> Option<SpecialUse> {
    if b.ai.frog_only {
        return None;
    }
    let cooling = b.me.fire_cooldown > 0.0 || b.ai.fire_timer > 0.0;
    if let Some(pick) = sense.pick.filter(|_| !cooling) {
        let face = Dir::from_rotation(b.me.rotation).unwrap_or(Dir::Up);
        return Some(SpecialUse::Charge { face, aim: Some(pick.cell), why: pick.why });
    }
    // Its distance from the seat it knows of (`RodSense::keep_from`), while
    // it is healthy enough not to flee: backed off out of the circle a call
    // on that seat would crush, held a band past it facing the seat, and
    // left to the tree further out - which brings it back in, never closer
    // than the band.
    let seat = sense.keep_from.filter(|_| b.me.damage < tuning().enemy_flee_damage)?;
    let reach = rod_stand_off_px();
    let d = b.me.position.distance_to(seat);
    // A move that stopped against a tank or a wall: it stands where it is
    // a while rather than grind (`Ai::rod_wait`). Its facing is kept - the
    // reticle aims, not the hull.
    let facing = Dir::from_rotation(b.me.rotation).unwrap_or(Dir::Up);
    if b.ai.rod_wait > 0.0 && d <= reach + tuning().rod_ai_band_px && !b.in_danger(b.me.position) {
        return Some(SpecialUse::Hold { face: facing, why: "stand-off" });
    }
    // On its way to a spot it chose: on until it is there, a tank stands in
    // its way or the way there is no longer open from where it is.
    let me = b.me.position;
    if let Some(to) = b.ai.rod_spot.filter(|&q| {
        me.distance_to(q) > OBSTACLE_GRID_SIZE * 0.5 && q.distance_to(seat) >= reach && way_open(b, me, q) && !crowded_ahead(me, Dir::toward(me, q), b.movers, b.my_index)
    }) {
        return Some(SpecialUse::Approach { to, why: "stand-off" });
    }
    // Backed off from inside `reach` - from a cell nearer while it holds,
    // unless its own hull is what keeps it from calling.
    // With nowhere open to back off to it stands where it is, keeping its
    // facing - the attack tier would only turn it about.
    let inner = if b.ai.rod_held && !sense.self_blocks { reach - OBSTACLE_GRID_SIZE } else { reach };
    if d < inner {
        return match rod_stand_off(b, seat, reach + OBSTACLE_GRID_SIZE) {
            Some(to) => Some(SpecialUse::Approach { to, why: "stand-off" }),
            None => (!b.in_danger(b.me.position)).then_some(SpecialUse::Hold { face: facing, why: "stand-off" }),
        };
    }
    if d <= reach + tuning().rod_ai_band_px && !b.in_danger(b.me.position) {
        // A spot an ally crowds is no place to stand: one driving to its
        // own slot would grind against a hull that never gives way. It
        // moves round the seat to a free spot of the band instead.
        if rod_crowded(b, b.me.position)
            && let Some(to) = rod_free_spot(b, seat, reach + OBSTACLE_GRID_SIZE)
        {
            return Some(SpecialUse::Approach { to, why: "stand-off" });
        }
        return Some(SpecialUse::Hold { face: facing, why: "stand-off" });
    }
    None
}

/// Whether another tank stands within `enemy_separation_px` of a hull at
/// `p`, hull to hull.
fn rod_crowded(b: &Brain, p: Position) -> bool {
    let gap = tuning().enemy_separation_px;
    let Some(me) = b.movers.get(b.my_index) else { return false };
    b.movers.iter().enumerate().any(|(i, other)| i != b.my_index && p.distance_to(other.position) - me.radius - other.radius <= gap)
}

/// The nearest of the eight spots `reach` out round `seat` - its row, its
/// column and the diagonals - that this tank can make for (`rod_spot_open`).
fn rod_free_spot(b: &Brain, seat: Position, reach: f32) -> Option<Position> {
    let d = std::f32::consts::FRAC_1_SQRT_2;
    let dirs = [(0.0, -1.0), (1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (d, -d), (d, d), (-d, d), (-d, -d)];
    let me = b.me.position;
    let mut spots: Vec<Position> = dirs.iter().map(|&(x, y)| seat + Vec2::new(x, y) * reach).collect();
    spots.sort_by(|a, c| me.distance_to(*a).total_cmp(&me.distance_to(*c)));
    spots.into_iter().find(|&q| rod_spot_open(b, q, seat, reach))
}

/// How far a rod tank keeps from the seat it knows of: a call on that
/// seat's cell must not hold the tank itself - the circle, the friend
/// margin, the widest hull's half - and a cell more.
fn rod_stand_off_px() -> f32 {
    let t = tuning();
    t.rod_kill_radius_px + t.rod_ai_friend_margin_px + crate::battlefield::max_tank_clearance_half_extent() + OBSTACLE_GRID_SIZE
}

/// Where a rod tank too near `seat` backs off to: `reach` out from the
/// seat along the line through the tank, else on the seat's row or column
/// - the nearest of those it can make for (`rod_spot_open`). `None` where
/// it can make for none.
fn rod_stand_off(b: &Brain, seat: Position, reach: f32) -> Option<Position> {
    let me = b.me.position;
    let away = me - seat;
    let len = away.length();
    let mut spots: Vec<Position> = Vec::new();
    if len > 1.0 {
        spots.push(seat + away * (reach / len));
    }
    let mut axes: Vec<Position> = Dir::ALL.iter().map(|d| seat + d.vec() * reach).collect();
    axes.sort_by(|a, c| me.distance_to(*a).total_cmp(&me.distance_to(*c)));
    spots.extend(axes);
    spots.into_iter().find(|&q| rod_spot_open(b, q, seat, reach))
}

/// Whether a rod tank can make for `q` round `seat`: on the field and
/// joined to where it stands by a way a four-way hull drives straight - a
/// leg along a row and one along a column, clear of every danger
/// (`way_open`: no route round a block, which in a maze can lead anywhere,
/// nor across a call) -, no tank crowding it there, none in its way as it
/// sets off - two backing off past each other on one line would only push
/// - and on its own side of the seat: the line stays `reach` less a cell
/// clear of the seat (or no nearer than the tank is, backing off), so it
/// never drives round the seat to the far side.
fn rod_spot_open(b: &Brain, q: Position, seat: Position, reach: f32) -> bool {
    let me = b.me.position;
    let along = q - me;
    let len2 = along.length_sqr().max(1e-6);
    let k = ((seat - me).dot(along) / len2).clamp(0.0, 1.0);
    let nearest = me + along * k;
    // Never nearer the seat than it already is, inside the band.
    let own_side = nearest.distance_to(seat) >= (reach - OBSTACLE_GRID_SIZE).min(me.distance_to(seat) - 1.0);
    own_side && b.can_reach(q) && way_open(b, me, q) && !rod_crowded(b, q) && !crowded_ahead(me, Dir::toward(me, q), b.movers, b.my_index)
}

/// Whether a hull at `from` can drive to `to` along a row then a column,
/// or a column then a row, every point half a cell apart in a usable nav
/// cell outside every danger this tank does not own.
fn way_open(b: &Brain, from: Position, to: Position) -> bool {
    let leg = |a: Position, z: Position| {
        let steps = (a.distance_to(z) / (OBSTACLE_GRID_SIZE * 0.5)).ceil() as usize;
        (1..=steps).all(|i| {
            let p = a + (z - a) * (i as f32 / steps as f32);
            b.grid.usable(p) && !b.in_danger(p)
        })
    };
    let (across, down) = (Position::new(to.x, from.y), Position::new(from.x, to.y));
    (leg(from, across) && leg(across, to)) || (leg(from, down) && leg(down, to))
}

/// A rod tank's rule while its reticle is up (`windup_rule`): standing
/// under a call, or with no pick left, it drops the reticle; short of the
/// pick's cell it walks the reticle on; on it, rested
/// `rod_ai_aim_hold_seconds` with the charge full, it lets go - the call.
/// Never `None`.
fn rod_aim_rule(b: &Brain, face: Dir) -> SpecialUse {
    let sense = match b.sense {
        SpecialSense::Rod(sense) => *sense,
        _ => RodSense::default(),
    };
    if sense.under_call {
        return SpecialUse::Drop { why: "rod-under-call" };
    }
    let Some(pick) = sense.pick else { return SpecialUse::Drop { why: "rod-lost" } };
    let full = b.me.charge.is_some_and(|c| c.stage() == crate::tank::ChargeStage::Full);
    match b.me.reticle {
        Some(r) if full && r.cell == pick.cell && r.rest >= tuning().rod_ai_aim_hold_seconds => SpecialUse::Release { face, at_seat: pick.at_seat, why: pick.why },
        _ => SpecialUse::Charge { face, aim: Some(pick.cell), why: "rod-aim" },
    }
}

/// The FPV swarm's rule (docs/fpv-swarm.md "AI"), in priority order: a
/// training dummy never launches; a healthy tank a seat it knows sees from
/// inside `fpv_ai_min_range_px` backs off; with none of its drones in the
/// air, one exposed to the seat it would launch at moves to cover first -
/// the two for at most `fpv_ai_cover_seconds` together (`Ai::place_waited`),
/// a place it cannot get to being none -, then it launches - at that seat, else into the crown over a seat hiding
/// under a tree, else (a hunter) at the players' frog (`FpvSense::quarry`,
/// set for a hunter alone) - one at a time, the
/// fire timer (`fpv_enemy_gap_seconds`) spacing them; with one in the air
/// it holds where it stands while it works. Draws no RNG.
fn fpv_rule(b: &Brain, sense: &FpvSense) -> Option<SpecialUse> {
    let t = tuning();
    if b.ai.frog_only {
        return None;
    }
    let free = b.me.damage < t.enemy_flee_damage && !b.guard_holds();
    let patient = b.ai.place_waited < t.fpv_ai_cover_seconds;
    if let Some(spot) = sense.back_off
        && free
        && patient
        && b.can_reach(spot)
    {
        return Some(SpecialUse::Approach { to: spot, why: "back off" });
    }
    // A hunter with drones fights the seat like everyone else
    // (`generic_fire` is false), and sends them at its quarry when no seat
    // is to be had.
    let quarry = sense.quarry;
    if sense.in_air == 0 {
        if let Some(spot) = sense.cover
            && sense.exposed
            && free
            && patient
            && b.me.position.distance_to(spot) > OBSTACLE_GRID_SIZE * 0.5
        {
            return Some(SpecialUse::Approach { to: spot, why: "to cover" });
        }
        if let Some((seat, _)) = sense.at_seat {
            let why = if sense.exposed { "open" } else { "cover" };
            return Some(SpecialUse::Launch { want: crate::fpv::AirWant::Seat(seat), at_seat: Some(seat), why });
        }
        if let Some((seat, crown)) = sense.canopy {
            return Some(SpecialUse::Launch { want: crate::fpv::AirWant::Point(crown), at_seat: Some(seat), why: "canopy" });
        }
        if quarry.is_some() {
            return Some(SpecialUse::Launch { want: crate::fpv::AirWant::Frog, at_seat: None, why: "frog" });
        }
        return None;
    }
    let watching = sense.at_seat.or(sense.canopy).map(|(_, p)| p).or(quarry);
    match watching {
        Some(at) if free => Some(SpecialUse::Hold { face: Dir::toward(b.me.position, at), why: "watch" }),
        _ => None,
    }
}

/// React to a seat's drone locked on this tank (`AirThreat`,
/// docs/fpv-swarm.md "Reacting to a seat's swarm"), in order: with an
/// online minigun and the seat's box round it, face the drone, stand and
/// fire while it is in reach and on the facing's line; else make for the
/// nearest spot under a tree's crown and stand there; else, once the drone
/// is within `fpv_ai_break_px`, drive across its line - the clockwise
/// perpendicular first, then the other, whichever is open. Draws no RNG.
fn act_air(b: &mut Brain) -> Status {
    let Some(threat) = b.ai.air_threat else { return Status::Failure };
    let t = tuning();
    let me = b.me.position;
    b.reset_aim();
    let face = Dir::toward(me, threat.drone);
    // A training dummy never fires toward a seat, and flak at a drone is
    // fire toward the seat that sent it (`Brain::may_fire_at_seat`).
    if b.me.active_weapon() == ActiveWeapon::Minigun && threat.may_shoot && !b.ai.frog_only {
        b.intent.face = Some(face);
        b.ai.commit(face);
        b.intent.move_dir = None;
        // The bullets strike the drone's column, from its shadow up to its
        // body (`air::AirTarget::strike_box`): on the line is the gun line
        // crossing that column, or passing within the tolerance of it.
        let to = threat.drone - me;
        let top = Position::new(to.x, to.y - threat.height);
        let line = face.vec();
        let along = |p: Position| p.x * line.x + p.y * line.y;
        let side = |p: Position| p.x * line.y - p.y * line.x;
        let (low, high) = (side(to), side(top));
        let across = if low * high <= 0.0 { 0.0 } else { low.abs().min(high.abs()) };
        let ahead = along(to).max(along(top)) > 0.0;
        b.intent.fire = ahead && to.length() <= t.fpv_ai_flak_range_px && across <= t.fpv_ai_flak_align_px;
        b.ai.air_why = Some("flak");
        return Status::Success;
    }
    if threat.covered {
        b.intent.move_dir = None;
        b.ai.air_why = Some("canopy");
        return Status::Success;
    }
    if let Some((spot, tree)) = threat.tree {
        let dir = if me.distance_to(spot) <= OBSTACLE_GRID_SIZE * 0.5 {
            Dir::toward(me, tree)
        } else {
            // A heading held from the fight that does not lead toward the
            // tree gives way, as `act_dodge`'s does: steering keeps a held
            // heading through a quarter turn.
            let to = spot - me;
            let toward = |d: Dir| d.vec().x * to.x + d.vec().y * to.y > 0.0;
            if b.ai.committed_dir.is_none_or(|d| !toward(d)) {
                b.ai.commit(Dir::toward(me, spot));
            }
            b.steer_out(spot)
        };
        b.intent.move_dir = Some(dir);
        b.ai.air_why = Some("tree");
        return Status::Success;
    }
    if me.distance_to(threat.drone) <= t.fpv_ai_break_px {
        let clockwise = match face {
            Dir::Up => [Dir::Right, Dir::Left],
            Dir::Right => [Dir::Down, Dir::Up],
            Dir::Down => [Dir::Left, Dir::Right],
            Dir::Left => [Dir::Up, Dir::Down],
        };
        let open = clockwise.into_iter().find(|d| b.walls_ahead[d.index()].is_none() && !b.grid.blocked_ahead(me, d.vec()));
        if let Some(dir) = open {
            b.intent.move_dir = Some(dir);
            b.intent.face = Some(dir);
            b.ai.air_why = Some("break");
            return Status::Success;
        }
    }
    Status::Failure
}

/// The gauss rail's rule with no charge running (docs/gauss-rail.md "AI"):
/// a training dummy never charges, nor does a rail still cooling (its
/// reload, a vent); otherwise the best lane it could face that counts with
/// its seat settled in the box (`GaussLane::settled`) - two seats, then a
/// seat and a player tower, then a seat (or a hunter's quarry), ties to the
/// facing it has and then `Dir::ALL` order - starts a charge. Draws no
/// RNG.
fn gauss_rule(b: &Brain, sense: &GaussSense) -> Option<SpecialUse> {
    if b.ai.frog_only || b.me.fire_cooldown > 0.0 {
        return None;
    }
    let facing = Dir::from_rotation(b.me.rotation).unwrap_or(Dir::Up);
    let order = std::iter::once(facing).chain(Dir::ALL.into_iter().filter(|&d| d != facing));
    let mut best: Option<(Dir, GaussLane)> = None;
    for d in order {
        let lane = sense.lanes[d.index()];
        let settled = lane.settled || lane.at_seat.is_none();
        if lane.counts() && settled && best.is_none_or(|(_, b)| lane.score() > b.score()) {
            best = Some((d, lane));
        }
    }
    let (face, lane) = best?;
    Some(SpecialUse::Charge { face, aim: None, why: lane.why() })
}

/// A charging gauss rail's rule (`windup_rule`): charging, it holds the
/// trigger and stands its ground; full, it lets go if the lane still
/// holds a seat that counts (or a hunter's quarry), else it holds and
/// waits for one to step back in; overcharged it holds on to the vent -
/// it never releases overcharged, so iron stays the cover that holds
/// against it. Never `None` while the charge runs: every other tier
/// leaves the trigger up, which would be a release.
fn gauss_charge_rule(b: &Brain, face: Dir, sense: Option<&GaussSense>) -> SpecialUse {
    let lane = sense.map(|s| s.lanes[face.index()]);
    match b.me.charge.map(|c| c.stage()) {
        Some(crate::tank::ChargeStage::Full) => match lane.filter(|l| l.counts() && l.target_along().is_some()) {
            Some(lane) => SpecialUse::Release { face, at_seat: lane.at_seat.map(|(s, _)| s), why: lane.why() },
            None => SpecialUse::Charge { face, aim: None, why: "rail-wait" },
        },
        Some(crate::tank::ChargeStage::Overcharged) => SpecialUse::Charge { face, aim: None, why: "rail-vent" },
        _ => SpecialUse::Charge { face, aim: None, why: "rail-charge" },
    }
}

/// The EMP burst's rule (docs/emp-burst.md "AI"), in priority order: a
/// training dummy never pulses, and a seat in reach that would see it from
/// outside its sight box holds the pulse whole; a pulse worth
/// `emp_ai_fire_value` goes off - unless one of its own side's towers is in
/// reach, or an ally is (then it has the ring cleared, `Clear`: its allies
/// keep out of it, and with the commander on they are nudged out too - for
/// `emp_ai_clear_patience_seconds`, after which it asks no more until its
/// ring is clear); then a closer
/// whose seat is worth `emp_ai_approach_value` on its own, not crowded by
/// an ally and not hidden from it, closes in to its spot of the seat's ring
/// drawn in to well inside the ring's reach and waits there facing it. A fire arm that
/// matched while the fire timer runs holds (`act_special`). Draws no RNG.
fn emp_rule(b: &Brain, sense: &EmpSense) -> Option<SpecialUse> {
    let t = tuning();
    if b.ai.frog_only || sense.off_box {
        return None;
    }
    let facing = Dir::from_rotation(b.me.rotation).unwrap_or(Dir::Up);
    if sense.value >= t.emp_ai_fire_value {
        if sense.friendly_tower {
            return None;
        }
        if sense.friends {
            let waiting = b.ai.clear_waited < t.emp_ai_clear_patience_seconds;
            return waiting.then_some(SpecialUse::Clear { radius: t.emp_radius_px + t.emp_ai_friend_margin_px });
        }
        return Some(SpecialUse::Fire { face: facing, at_seat: sense.at_seat, why: "pulse" });
    }
    let free = b.me.damage < t.enemy_flee_damage && !b.guard_holds() && !b.hunting_frog();
    // A seat hidden from it in tall grass is none to go after, unless it
    // was just shot by it (the attack tier's rule).
    let seen = !b.target_concealed || b.ai.hit_alert_timer > 0.0;
    if sense.closer
        && free
        && seen
        && !sense.target_crowded
        && sense.target_value >= t.emp_ai_approach_value
        && b.player_alive()
        && b.player_line_of_sight
        && b.dist_to_player() <= b.attack_range()
    {
        let seat = b.player.position;
        let spot = b.close_spot(seat, b.engage_target, t.emp_radius_px * t.emp_ai_close_share);
        if b.me.position.distance_to(spot) <= OBSTACLE_GRID_SIZE * 0.5 {
            return Some(SpecialUse::Hold { face: Dir::toward(b.me.position, seat), why: "close" });
        }
        return Some(SpecialUse::Approach { to: spot, why: "approach" });
    }
    None
}

/// Back out of the danger this tank stands inside (`Brain::danger_here`):
/// walk for its way out a cell past the clear margin - the exit it chose
/// (`Ai::dodge_exit`) while it stays out of every danger and the tank can
/// still walk there, else the first of `Danger::exits` that does (the
/// first it can walk to, failing that; with none, the tree goes on) - or,
/// in a passing danger's slack band, stop.
/// On foot, never through a portal: a route that hops one hands out the
/// far portal as its first step from the near one's edge, which a tank
/// that cannot hop yet would turn back and forth on. Steering drives it
/// (`Brain::steer_out`), so it sidesteps a tank in its way and its stuck
/// escape runs; the one turn steering never takes, round from a heading
/// that leads away from the way out (`steer_toward`), is put on its heading
/// here, a dodge being most often that.
fn act_dodge(b: &mut Brain) -> Status {
    let Some(danger) = b.danger_here() else { return Status::Failure };
    b.reset_aim();
    if danger.slack > 0.0 && danger.depth(b.me.position) <= danger.slack {
        // In its slack band (or out past it, latched): wait for it to
        // pass, going no deeper.
        b.intent.move_dir = None;
        return Status::Success;
    }
    let facing = Dir::from_rotation(b.me.rotation).unwrap_or(Dir::Up);
    let me = b.me.position;
    let exits = danger.exits(me, tuning().enemy_danger_clear_px + OBSTACLE_GRID_SIZE, me, facing);
    // The first step of the walk to `q`, if it can walk there.
    let walk = |b: &Brain, q: Position| {
        if !b.can_reach(q) {
            None
        } else if b.grid.same_cell(me, q) {
            Some(q)
        } else {
            b.grid.next_step_walking(me, q)
        }
    };
    let open = |b: &Brain, q: Position| if b.in_danger(q) { None } else { walk(b, q).map(|step| (q, step)) };
    let held = b.ai.dodge_exit.and_then(|q| open(b, q));
    let pick = held.or_else(|| exits.iter().find_map(|&q| open(b, q))).or_else(|| exits.iter().find_map(|&q| walk(b, q).map(|step| (q, step))));
    b.ai.dodge_exit = pick.map(|(q, _)| q);
    // Walled in with no way out on foot: the rest of the tree has the tick.
    let Some((out, step)) = pick else { return Status::Failure };
    // A heading held that leads away from the way out - the reversal
    // steering never takes - gives way to the route's first step.
    let way = Dir::toward(me, step);
    let to_out = Vec2::new(out.x - me.x, out.y - me.y);
    let len = to_out.length().max(1.0);
    let leads_away = |d: Dir| (d.vec().x * to_out.x + d.vec().y * to_out.y) / len < -0.5;
    if b.ai.committed_dir.is_none_or(leads_away) {
        b.ai.commit(way);
    }
    b.intent.move_dir = Some(b.steer_out(out));
    Status::Success
}

/// What a wind-up asks of its tank, by its weapon: a tell holds the tank
/// facing the way it goes off; a charge (`Tank::charge`) is its weapon's
/// to hold or let go - the rail's `gauss_charge_rule` - and a charge
/// weapon adds its arm here.
fn windup_rule(b: &Brain, windup: crate::tank::Windup) -> Option<SpecialUse> {
    // A charge is let go under a call (docs/rod-from-god.md "Reacting to a
    // call"); a tell, half a second long, commits.
    if b.me.charge.is_some() && b.in_call() {
        return Some(SpecialUse::Drop { why: "under-call" });
    }
    match b.me.charge.map(|c| c.weapon) {
        Some(ActiveWeapon::RodFromGod) => Some(rod_aim_rule(b, windup.facing)),
        Some(ActiveWeapon::GaussRail) => {
            let sense = match b.sense {
                SpecialSense::Gauss(sense) => Some(sense),
                _ => None,
            };
            Some(gauss_charge_rule(b, windup.facing, sense))
        }
        _ => Some(SpecialUse::Hold { face: windup.facing, why: "hold" }),
    }
}

/// The sonic hammer's rule (docs/sonic-hammer.md "AI"), in priority order:
/// shout down glass in its way at once; then, each way it could face - its
/// own facing first, then `Dir::ALL`, never a way a fellow enemy stands -
/// shove a seat into trouble, throw a drum onto one, flush the grass round
/// a hidden seat's alert, break a seat in its face, pin a hunter's frog;
/// then a closer closes in to its spot (`HammerSense::spot`) and waits
/// there facing the seat.
/// A fire arm that matched while the fire timer runs holds facing it
/// (`act_special`). Draws no RNG.
fn hammer_rule(b: &Brain, sense: &HammerSense) -> Option<SpecialUse> {
    let t = tuning();
    // Glass in the way it is driving: down at once.
    if let Some(dir) = b.ai.last_move_dir
        && b.walls_ahead[dir.index()].is_some_and(|w| w.material.breaks_by_sound())
        && b.ai.wall_ahead_timer >= t.sonic_ai_glass_after_seconds
    {
        return Some(SpecialUse::Fire { face: dir, at_seat: None, why: "glass" });
    }
    let facing = Dir::from_rotation(b.me.rotation).unwrap_or(Dir::Up);
    let order = std::iter::once(facing).chain(Dir::ALL.into_iter().filter(|&d| d != facing));
    let aims: Vec<(Dir, HammerAim)> = order.map(|d| (d, sense.aims[d.index()])).filter(|(_, a)| !a.friend).collect();
    type Arm = fn(&HammerAim) -> Option<Option<u8>>;
    let arms: [(&'static str, Arm); 5] = [
        ("trouble", |a| a.trouble.map(Some)),
        ("drum", |a| a.drum.map(Some)),
        ("flush", |a| a.flush.map(Some)),
        ("breaker", |a| a.breaker.map(Some)),
        ("frog", |a| a.frog.then_some(None)),
    ];
    for (why, arm) in arms {
        if let Some((face, at_seat)) = aims.iter().find_map(|(d, a)| arm(a).map(|seat| (*d, seat))) {
            return Some(SpecialUse::Fire { face, at_seat, why });
        }
    }
    // With nothing to shout at, a closer (`HammerSense::spot`), healthy
    // and free to roam, closes in on the seat it fights - it has no shot
    // at range - to its spot, square on the seat at the breaker's
    // distance, and waits there facing it. The others hold their slots of
    // the ring by the tree's attack tier, which never fires the hammer.
    let free = b.me.damage < t.enemy_flee_damage && !b.guard_holds();
    // A seat carrying an armed EMP (a `Danger` round it) is not closed in
    // on: the hammer's reach is inside the EMP's.
    if let Some(spot) = sense.spot
        && free
        && b.player_alive()
        && b.player_line_of_sight
        && b.dist_to_player() <= b.attack_range()
        && !b.in_danger(b.player.position)
    {
        if b.me.position.distance_to(spot) <= OBSTACLE_GRID_SIZE * 0.5 {
            return Some(SpecialUse::Hold { face: Dir::toward(b.me.position, b.player.position), why: "close" });
        }
        return Some(SpecialUse::Approach { to: spot, why: "approach" });
    }
    None
}

/// Apply what the special's rule asked (`special_rule`): fire - face,
/// commit and, with the fire timer out, pull the trigger, recording the
/// seat it is used on and resetting the timer to the weapon's own interval;
/// hold; close in; or ask for its ring to be cleared, which only records
/// the ring (`Ai::clearing`) and leaves the tick to the tiers below - a
/// tank that held still for it would stand there for as long as an ally
/// did not leave, and two clearing each other would never move.
fn act_special(b: &mut Brain) -> Status {
    let Some(use_) = special_rule(b) else { return Status::Failure };
    b.reset_aim();
    match use_ {
        SpecialUse::Fire { face, at_seat, why } => {
            b.intent.face = Some(face);
            b.ai.commit(face);
            b.ai.special_why = Some(why);
            if b.ai.fire_timer <= 0.0 && b.me.fire_cooldown <= 0.0 {
                b.intent.fire = true;
                b.ai.fire_timer = special_fire_interval(b.me.active_weapon());
                b.ai.shot_at_seat = at_seat;
            }
        }
        SpecialUse::Hold { face, why } => {
            b.intent.face = Some(face);
            b.ai.commit(face);
            b.ai.special_why = Some(why);
            b.ai.rod_spot = None;
            b.ai.rod_held = why == "stand-off";
        }
        SpecialUse::Clear { radius } => {
            b.ai.special_why = Some("clear");
            b.ai.clearing = Some(radius);
            b.ai.clear_waited += b.dt;
            return Status::Failure;
        }
        SpecialUse::Launch { want, at_seat, why } => {
            let face = Dir::from_rotation(b.me.rotation).unwrap_or(Dir::Up);
            b.intent.face = Some(face);
            b.ai.special_why = Some(why);
            if b.ai.fire_timer <= 0.0 && b.me.fire_cooldown <= 0.0 {
                b.intent.fire = true;
                b.ai.fire_timer = special_fire_interval(b.me.active_weapon());
                b.ai.shot_at_seat = at_seat;
                b.ai.air_want = Some(want);
                b.ai.place_waited = 0.0;
            }
        }
        SpecialUse::Approach { to, why } => {
            // A stand-off's move that has stopped against a tank or a wall
            // (`rod_ai_give_up_seconds` of no headway), or whose way leads
            // away from its spot - the router going round something, which
            // ends in a loop: steered off by more than half a cell, or a
            // cell further than it has been -, is given up for
            // `rod_ai_wait_seconds`: it stands.
            let away = b.me.position.distance_to(to);
            let dir = b.steer(to);
            let lost = why == "stand-off"
                && ((to - b.me.position).dot(dir.vec()) < -OBSTACLE_GRID_SIZE * 0.5
                    || (b.ai.rod_spot == Some(to) && away > b.ai.rod_spot_best + OBSTACLE_GRID_SIZE));
            if why == "stand-off" && (b.ai.stuck_timer >= tuning().rod_ai_give_up_seconds || lost) {
                b.ai.special_why = Some(why);
                b.ai.rod_spot = None;
                b.ai.rod_wait = tuning().rod_ai_wait_seconds;
                return Status::Success;
            }
            b.intent.move_dir = Some(dir);
            b.ai.special_why = Some(why);
            if why == "stand-off" {
                b.ai.rod_spot_best = if b.ai.rod_spot == Some(to) { b.ai.rod_spot_best.min(away) } else { away };
                b.ai.rod_spot = Some(to);
                b.ai.rod_held = false;
            }
            if why == "to cover" || why == "back off" {
                b.ai.place_waited += b.dt;
            }
        }
        SpecialUse::Charge { face, aim, why } => {
            b.intent.face = Some(face);
            b.ai.commit(face);
            b.ai.special_why = Some(why);
            b.intent.aim_cell = aim;
            // A charge running is held whatever the timer says; a new one
            // waits for it (and for the weapon's own cooldown).
            let running = b.me.charge.is_some();
            if running || (b.ai.fire_timer <= 0.0 && b.me.fire_cooldown <= 0.0) {
                if !running {
                    b.ai.fire_timer = special_fire_interval(b.me.active_weapon());
                }
                b.intent.fire = true;
            }
        }
        SpecialUse::Release { face, at_seat, why } => {
            b.intent.face = Some(face);
            b.ai.commit(face);
            b.ai.special_why = Some(why);
            b.intent.fire = false;
            b.ai.shot_at_seat = at_seat;
        }
        SpecialUse::Drop { why } => {
            b.ai.special_why = Some(why);
            b.intent.fire = false;
            b.intent.drop_charge = true;
            // Under a call or in a pull the tree goes on, to the dodge or
            // the `pull` tier; a target lost only ends the tick standing as
            // it was.
            if b.in_call() || b.yields_to_pull() {
                return Status::Failure;
            }
        }
        SpecialUse::Orb { face, anchor_px, at_seat, why } => {
            b.intent.face = Some(face);
            b.ai.commit(face);
            b.ai.special_why = Some(why);
            if b.ai.fire_timer <= 0.0 && b.me.fire_cooldown <= 0.0 {
                b.intent.fire = true;
                b.ai.fire_timer = special_fire_interval(b.me.active_weapon());
                b.ai.shot_at_seat = at_seat;
                b.ai.well_anchor_px = anchor_px;
            }
        }
        SpecialUse::Anchor { face } => {
            b.intent.face = Some(face);
            b.ai.commit(face);
            b.ai.special_why = Some("anchor");
            b.intent.fire = true;
        }
    }
    Status::Success
}

/// Seconds an enemy waits between two decisions to use `weapon` by its
/// rule.
fn special_fire_interval(weapon: ActiveWeapon) -> f32 {
    match weapon {
        ActiveWeapon::SonicHammer => tuning().sonic_ai_fire_interval,
        ActiveWeapon::Emp => tuning().emp_ai_fire_interval,
        ActiveWeapon::GaussRail => tuning().gauss_ai_fire_interval,
        ActiveWeapon::FpvSwarm => tuning().fpv_enemy_gap_seconds,
        ActiveWeapon::RodFromGod => tuning().rod_ai_fire_interval,
        ActiveWeapon::GravityWell => tuning().well_ai_fire_interval,
        _ => tuning().enemy_fire_interval,
    }
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
        let raw = b.engage_target.unwrap_or(b.target);
        let to = b.engage_point();
        if to != raw && b.me.position.distance_to(to) <= OBSTACLE_GRID_SIZE * 0.5 {
            // Kept out of a danger at the spot it was moved to: wait there
            // facing the fight rather than driving on past it, which
            // steering's commitment would (`steer_toward`), into the very
            // danger.
            b.ai.kept_out = true;
            b.intent.move_dir = None;
            b.intent.face = Some(Dir::toward(b.me.position, raw));
        } else {
            b.intent.move_dir = Some(b.steer(to));
        }
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
    b.cap_fire_timer();
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
        b.intent.move_dir = Some(b.steer(b.out_of_danger(b.engage_target.unwrap_or(target))));
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
            &SpecialSense::None,
            &[],
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
                &SpecialSense::None,
                &[],
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
            &SpecialSense::None,
            &[],
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

/// The sonic hammer's rule (docs/sonic-hammer.md "AI"), one arm at a time
/// on an open field: what `enemy_phase` measured is handed in as a
/// `HammerSense`, so the rule is tested without a world.
#[cfg(test)]
mod hammer_tests {
    use super::*;
    use rand::SeedableRng;

    const ME: Position = Position::new(640.0, 360.0);
    const SEAT: Position = Position::new(200.0, 360.0);

    /// An enemy facing up with the hammer, its fire timer out.
    fn hammer_tank() -> Tank {
        let mut me = Tank { owner: crate::shell::Owner::Enemy(2), ..Tank::default() };
        me.position = ME;
        me.rotation = 0.0;
        me.sonic_ammo = 3;
        me
    }

    fn think(ai: &mut Ai, me: &Tank, sense: SpecialSense, walls: [Option<WallAhead>; 4], seat: Position) -> Intent {
        let mut player = Tank::default();
        player.position = seat;
        let grid = Grid::build(1280.0, 720.0, 48.0, 0.0, std::iter::empty());
        let movers = [
            Mover { position: seat, velocity: Vec2::new(0.0, 0.0), radius: 20.0, is_player: true },
            Mover { position: me.position, velocity: Vec2::new(0.0, 0.0), radius: 20.0, is_player: false },
        ];
        let mut rng = SmallRng::seed_from_u64(7);
        ai.think(me, &player, seat, None, 1280.0, 720.0, 1.0 / 60.0, &movers, 1, &grid, &mut rng, None, None, &[], true, true, false, walls, tuning().enemy_view_range, &sense, &[])
    }

    /// A fresh memory with its fire timer out.
    fn ready() -> Ai {
        Ai { fire_timer: 0.0, ..Ai::with_role(Role::Player) }
    }

    fn sense_left(aim: HammerAim) -> SpecialSense {
        let mut aims = [HammerAim::default(); 4];
        aims[Dir::Left.index()] = aim;
        SpecialSense::Hammer(HammerSense { aims, spot: None })
    }

    #[test]
    fn every_arm_fires_the_way_it_names() {
        let arms: [(&str, HammerAim); 5] = [
            ("trouble", HammerAim { trouble: Some(0), ..HammerAim::default() }),
            ("drum", HammerAim { drum: Some(0), ..HammerAim::default() }),
            ("flush", HammerAim { flush: Some(0), ..HammerAim::default() }),
            ("breaker", HammerAim { breaker: Some(0), ..HammerAim::default() }),
            ("frog", HammerAim { frog: true, ..HammerAim::default() }),
        ];
        for (why, aim) in arms {
            let mut ai = ready();
            let intent = think(&mut ai, &hammer_tank(), sense_left(aim), [None; 4], SEAT);
            assert!(intent.fire, "{why}: it pulls the trigger");
            assert_eq!(intent.face, Some(Dir::Left), "{why}");
            assert_eq!(ai.special_why, Some(why));
            assert_eq!(ai.fire_timer, tuning().sonic_ai_fire_interval, "{why}: on the weapon's own interval");
            // While the timer runs it holds facing the same way, trigger off.
            let again = think(&mut ai, &hammer_tank(), sense_left(aim), [None; 4], SEAT);
            assert!(!again.fire && again.face == Some(Dir::Left), "{why}: {again:?}");
        }
    }

    #[test]
    fn the_arms_go_in_order_and_its_own_facing_first() {
        let mut aims = [HammerAim::default(); 4];
        aims[Dir::Left.index()] = HammerAim { trouble: Some(0), ..HammerAim::default() };
        aims[Dir::Up.index()] = HammerAim { breaker: Some(0), ..HammerAim::default() };
        let mut ai = ready();
        let intent = think(&mut ai, &hammer_tank(), SpecialSense::Hammer(HammerSense { aims, spot: None }), [None; 4], SEAT);
        assert_eq!((intent.face, ai.special_why), (Some(Dir::Left), Some("trouble")), "trouble outranks a breaker");
        let mut aims = [HammerAim::default(); 4];
        aims[Dir::Left.index()] = HammerAim { breaker: Some(0), ..HammerAim::default() };
        aims[Dir::Up.index()] = HammerAim { breaker: Some(0), ..HammerAim::default() };
        let mut ai = ready();
        let intent = think(&mut ai, &hammer_tank(), SpecialSense::Hammer(HammerSense { aims, spot: None }), [None; 4], SEAT);
        assert_eq!(intent.face, Some(Dir::Up), "the same arm both ways: the way it already faces");
    }

    #[test]
    fn it_never_fires_where_a_fellow_enemy_stands() {
        let mut ai = ready();
        let aim = HammerAim { trouble: Some(0), breaker: Some(0), friend: true, ..HammerAim::default() };
        for _ in 0..120 {
            let intent = think(&mut ai, &hammer_tank(), sense_left(aim), [None; 4], SEAT);
            assert!(!intent.fire, "{intent:?}");
        }
    }

    /// The generic tiers never pull the trigger on a weapon its rule owns:
    /// lined up on a seat in range with nothing in the sense, it does not
    /// fire, where the same tank on shells does.
    #[test]
    fn the_generic_tiers_never_fire_the_hammer() {
        let seat = Position::new(ME.x - 200.0, ME.y);
        let (mut hammer_fired, mut shell_fired) = (false, false);
        let (mut a, mut b) = (ready(), ready());
        let mut shells = hammer_tank();
        shells.sonic_ammo = 0;
        shells.rotation = 270.0;
        let mut hammer = hammer_tank();
        hammer.rotation = 270.0;
        let none = SpecialSense::Hammer(HammerSense::default());
        for _ in 0..240 {
            hammer_fired |= think(&mut a, &hammer, none, [None; 4], seat).fire;
            shell_fired |= think(&mut b, &shells, SpecialSense::None, [None; 4], seat).fire;
        }
        assert!(shell_fired, "the shells tank fires");
        assert!(!hammer_fired, "the hammer tank does not");
    }

    /// With nothing to shout at, a tank given a spot closes in on it and
    /// waits there facing the seat; one given none leaves it to the tree.
    #[test]
    fn a_closer_closes_in_to_its_spot_and_waits_there() {
        let seat = Position::new(ME.x - 200.0, ME.y);
        let mut ai = ready();
        think(&mut ai, &hammer_tank(), SpecialSense::Hammer(HammerSense::default()), [None; 4], seat);
        assert_eq!(ai.special_why, None, "no spot: the tree's");
        let spot = Position::new(seat.x + tuning().sonic_ai_breaker_px - 16.0, seat.y);
        let given = SpecialSense::Hammer(HammerSense { spot: Some(spot), ..HammerSense::default() });
        let mut ai = ready();
        let intent = think(&mut ai, &hammer_tank(), given, [None; 4], seat);
        assert_eq!((ai.special_why, intent.move_dir), (Some("approach"), Some(Dir::Left)));
        // At its spot: it holds facing the seat.
        let mut there = hammer_tank();
        there.position = Position::new(spot.x + 6.0, spot.y);
        let mut ai = ready();
        let intent = think(&mut ai, &there, given, [None; 4], seat);
        assert_eq!((ai.special_why, intent.move_dir, intent.face), (Some("close"), None, Some(Dir::Left)));
    }

    #[test]
    fn a_tell_holds_the_tank_facing_its_way() {
        let mut me = hammer_tank();
        me.tell = Some(crate::tank::Tell { weapon: ActiveWeapon::SonicHammer, left: 0.3, total: 0.55, facing: Dir::Right });
        let mut ai = ready();
        let intent = think(&mut ai, &me, sense_left(HammerAim { trouble: Some(0), ..HammerAim::default() }), [None; 4], SEAT);
        assert_eq!((intent.face, intent.move_dir, intent.fire), (Some(Dir::Right), None, false));
        assert_eq!(ai.special_why, Some("hold"));
    }

    #[test]
    fn glass_in_its_way_is_shouted_down() {
        let mut ai = ready();
        // Driving at the seat to the west, a pane in the way.
        ai.last_move_dir = Some(Dir::Left);
        let mut walls = [None; 4];
        walls[Dir::Left.index()] = Some(WallAhead { material: Material::Glass, burning: false });
        let mut fired = false;
        for _ in 0..30 {
            let intent = think(&mut ai, &hammer_tank(), SpecialSense::Hammer(HammerSense::default()), walls, SEAT);
            if intent.fire {
                assert_eq!((intent.face, ai.special_why), (Some(Dir::Left), Some("glass")));
                fired = true;
                break;
            }
        }
        assert!(fired, "it shouts the pane down");
    }
}

/// The EMP's rule and the dangers (docs/emp-burst.md "AI"), on an open
/// field: what `enemy_phase` measured is handed in as an `EmpSense` and a
/// list of `Danger`s, so the rule is tested without a world.
#[cfg(test)]
mod emp_rule_tests {
    use super::*;
    use rand::SeedableRng;

    const ME: Position = Position::new(640.0, 360.0);

    /// An enemy facing up with the EMP, its fire timer out.
    fn emp_tank() -> Tank {
        let mut me = Tank { owner: crate::shell::Owner::Enemy(2), ..Tank::default() };
        me.position = ME;
        me.rotation = 0.0;
        me.emp_charges = 3;
        me
    }

    fn think(ai: &mut Ai, me: &Tank, sense: SpecialSense, dangers: &[Danger], seat: Position) -> Intent {
        think_hidden(ai, me, sense, dangers, seat, false)
    }

    /// `think`, with the seat hidden from the tank in tall grass or not.
    fn think_hidden(ai: &mut Ai, me: &Tank, sense: SpecialSense, dangers: &[Danger], seat: Position, hidden: bool) -> Intent {
        let mut player = Tank::default();
        player.position = seat;
        let grid = Grid::build(1280.0, 720.0, 48.0, 0.0, std::iter::empty());
        let movers = [
            Mover { position: seat, velocity: Vec2::new(0.0, 0.0), radius: 20.0, is_player: true },
            Mover { position: me.position, velocity: Vec2::new(0.0, 0.0), radius: 20.0, is_player: false },
        ];
        let mut rng = SmallRng::seed_from_u64(7);
        ai.think(me, &player, seat, None, 1280.0, 720.0, 1.0 / 60.0, &movers, 1, &grid, &mut rng, None, None, &[], true, true, hidden, [None; 4], tuning().enemy_view_range, &sense, dangers)
    }

    fn ready() -> Ai {
        Ai { fire_timer: 0.0, ..Ai::with_role(Role::Player) }
    }

    fn sense(value: i32) -> SpecialSense {
        SpecialSense::Emp(EmpSense { value, at_seat: (value > 0).then_some(0), ..EmpSense::default() })
    }

    /// A seat in its ring is worth a pulse at the default threshold: it
    /// pulls the trigger facing the way it stands, on the weapon's own
    /// interval, and names the seat.
    #[test]
    fn a_seat_in_the_ring_is_pulsed() {
        let seat = Position::new(ME.x - 100.0, ME.y);
        let mut ai = ready();
        let intent = think(&mut ai, &emp_tank(), sense(tuning().emp_ai_fire_value), &[], seat);
        assert!(intent.fire, "{intent:?}");
        assert_eq!((intent.face, ai.special_why), (Some(Dir::Up), Some("pulse")));
        assert_eq!(ai.fire_timer, tuning().emp_ai_fire_interval);
        assert_eq!(ai.shot_at_seat(), Some(0));
    }

    /// Never from outside the seat's sight box, beside its own side's
    /// tower, or into an ally with the commander off; never a dummy.
    #[test]
    fn it_holds_off_box_beside_its_tower_into_an_ally_and_as_a_dummy() {
        let seat = Position::new(ME.x - 100.0, ME.y);
        let v = tuning().emp_ai_fire_value;
        let cases = [
            ("off box", EmpSense { value: v, at_seat: Some(0), off_box: true, ..EmpSense::default() }, false),
            ("tower", EmpSense { value: v, at_seat: Some(0), friendly_tower: true, ..EmpSense::default() }, false),
            ("ally", EmpSense { value: v, at_seat: Some(0), friends: true, ..EmpSense::default() }, false),
            ("dummy", EmpSense { value: v, at_seat: Some(0), ..EmpSense::default() }, true),
        ];
        for (why, s, dummy) in cases {
            let mut ai = ready();
            ai.frog_only = dummy;
            for _ in 0..60 {
                let intent = think(&mut ai, &emp_tank(), SpecialSense::Emp(s), &[], seat);
                assert!(!intent.fire, "{why}: {intent:?}");
            }
        }
    }

    /// The generic tiers never pull the trigger on the EMP: lined up on a
    /// seat in range with nothing in its ring, it does not fire, where the
    /// same tank on shells does.
    #[test]
    fn the_generic_tiers_never_fire_the_emp() {
        let seat = Position::new(ME.x - 260.0, ME.y);
        let (mut emp_fired, mut shell_fired) = (false, false);
        let (mut a, mut b) = (ready(), ready());
        let mut shells = emp_tank();
        shells.emp_charges = 0;
        shells.rotation = 270.0;
        let mut emp = emp_tank();
        emp.rotation = 270.0;
        for _ in 0..240 {
            emp_fired |= think(&mut a, &emp, sense(0), &[], seat).fire;
            shell_fired |= think(&mut b, &shells, SpecialSense::None, &[], seat).fire;
        }
        assert!(shell_fired, "the shells tank fires");
        assert!(!emp_fired, "the EMP tank does not");
    }

    /// A closer goes looking only for a seat worth it - the approach
    /// threshold - that is not crowded by its own side and not hidden from
    /// it, unless it just shot the closer; a tank that is not one leaves it
    /// to the tree.
    #[test]
    fn a_closer_approaches_only_a_seat_worth_it() {
        let seat = Position::new(ME.x - 300.0, ME.y);
        let approach = tuning().emp_ai_approach_value;
        let closer = |target_value| SpecialSense::Emp(EmpSense { target_value, closer: true, ..EmpSense::default() });
        let mut ai = ready();
        think(&mut ai, &emp_tank(), closer(approach - 1), &[], seat);
        assert_eq!(ai.special_why, None, "a bare seat by day: the tree's");
        let mut ai = ready();
        let intent = think(&mut ai, &emp_tank(), closer(approach), &[], seat);
        assert_eq!((ai.special_why, intent.move_dir), (Some("approach"), Some(Dir::Left)));
        let crowded = SpecialSense::Emp(EmpSense { target_value: approach, closer: true, target_crowded: true, ..EmpSense::default() });
        let mut ai = ready();
        think(&mut ai, &emp_tank(), crowded, &[], seat);
        assert_eq!(ai.special_why, None, "a seat already crowded by its own side");
        let mut ai = ready();
        think_hidden(&mut ai, &emp_tank(), closer(approach), &[], seat, true);
        assert_eq!(ai.special_why, None, "a seat hidden from it in the grass");
        let mut ai = Ai { hit_alert_timer: 1.0, ..ready() };
        think_hidden(&mut ai, &emp_tank(), closer(approach), &[], seat, true);
        assert_eq!(ai.special_why, Some("approach"), "hidden, but it was just shot by it");
    }

    /// Inside a danger that is not its own a tank backs out - away from its
    /// middle, whichever way it was heading - and stays latched until it
    /// is clear; its own danger it ignores; one nobody owns everybody
    /// keeps out of.
    #[test]
    fn a_tank_backs_out_of_a_danger_not_its_own() {
        let seat = Position::new(ME.x - 400.0, ME.y);
        let disc = |owner| Danger { shape: DangerShape::Disc { at: Position::new(ME.x - 60.0, ME.y), radius: 200.0 }, owner, slack: 0.0 };
        for (owner, out) in [(Some(0), true), (Some(2), false), (None, true)] {
            let mut ai = ready();
            ai.committed_dir = Some(Dir::Left);
            let intent = think(&mut ai, &emp_tank(), SpecialSense::None, &[disc(owner)], seat);
            if out {
                assert_eq!((ai.last_action, intent.move_dir), (Some("dodge"), Some(Dir::Right)), "{owner:?}");
                assert!(ai.dodging);
            } else {
                assert_ne!(ai.last_action, Some("dodge"), "its own: {owner:?}");
            }
        }
        // Latched: just outside the edge it is still backing out.
        let mut ai = ready();
        ai.dodging = true;
        let mut me = emp_tank();
        me.position = Position::new(ME.x - 60.0 + 200.0 + tuning().enemy_danger_clear_px * 0.5, ME.y);
        let intent = think(&mut ai, &me, SpecialSense::None, &[disc(Some(0))], seat);
        assert_eq!((ai.last_action, intent.move_dir), (Some("dodge"), Some(Dir::Right)));
    }

    /// A tank backing out keeps the exit it chose while it can reach it,
    /// though an exit nearer the line out comes back into reach - so one
    /// on a shore's edge, flickering in and out of reach as the tank
    /// moves, does not turn it back and forth on the spot - and chooses
    /// again only once the one it holds is out of reach.
    #[test]
    fn a_tank_backing_out_keeps_the_exit_it_chose() {
        let seat = Position::new(ME.x - 400.0, ME.y);
        let disc = |radius| Danger { shape: DangerShape::Disc { at: Position::new(ME.x - 60.0, ME.y), radius }, owner: Some(0), slack: 0.0 };
        // Straight out is to the right; the exit turned a quarter round,
        // below, is held.
        let clear = tuning().enemy_danger_clear_px + OBSTACLE_GRID_SIZE;
        let middle = Position::new(ME.x - 60.0, ME.y);
        let mut ai = ready();
        let intent = think(&mut ai, &emp_tank(), SpecialSense::None, &[disc(200.0)], seat);
        let straight = Position::new(middle.x + 200.0 + clear, ME.y);
        assert_eq!((intent.move_dir, ai.dodge_exit), (Some(Dir::Right), Some(straight)), "the nearest first");
        let below = Position::new(middle.x, ME.y + 200.0 + clear);
        let mut ai = ready();
        ai.dodge_exit = Some(below);
        let intent = think(&mut ai, &emp_tank(), SpecialSense::None, &[disc(200.0)], seat);
        assert_eq!((intent.move_dir, ai.dodge_exit), (Some(Dir::Down), Some(below)), "the one it holds");
        // Held past the field's bottom edge: out of reach, chosen again.
        let mut ai = ready();
        ai.dodge_exit = Some(Position::new(middle.x, ME.y + 400.0 + clear));
        let intent = think(&mut ai, &emp_tank(), SpecialSense::None, &[disc(400.0)], seat);
        assert_eq!((intent.move_dir, ai.dodge_exit), (Some(Dir::Right), Some(Position::new(middle.x + 400.0 + clear, ME.y))), "out of reach: the first it can reach");
        // Held inside the danger: chosen again too.
        let mut ai = ready();
        ai.dodge_exit = Some(Position::new(ME.x, ME.y + 100.0));
        think(&mut ai, &emp_tank(), SpecialSense::None, &[disc(200.0)], seat);
        assert_eq!(ai.dodge_exit, Some(straight), "inside the danger: the first out of it");
        // Clear of every danger, it lets go.
        let mut ai = ready();
        ai.dodge_exit = Some(below);
        think(&mut ai, &emp_tank(), SpecialSense::None, &[], seat);
        assert_eq!((ai.dodging, ai.dodge_exit), (false, None));
    }

    /// In the slack band of a danger that is over in a moment (an ally's
    /// crackle) a tank stops rather than turning round; deeper, it backs
    /// out.
    #[test]
    fn a_tank_in_a_crackles_slack_band_stops_and_deeper_backs_out() {
        let seat = Position::new(ME.x - 400.0, ME.y);
        let crackle = Danger { shape: DangerShape::Disc { at: Position::new(ME.x - 190.0, ME.y), radius: 208.0 }, owner: Some(5), slack: 48.0 };
        let mut ai = ready();
        ai.committed_dir = Some(Dir::Left);
        let intent = think(&mut ai, &emp_tank(), SpecialSense::None, &[crackle], seat);
        assert_eq!((ai.last_action, intent.move_dir), (Some("dodge"), None), "18 px in: it stops");
        let deep = Danger { shape: DangerShape::Disc { at: Position::new(ME.x - 100.0, ME.y), radius: 208.0 }, ..crackle };
        let mut ai = ready();
        ai.committed_dir = Some(Dir::Left);
        let intent = think(&mut ai, &emp_tank(), SpecialSense::None, &[deep], seat);
        assert_eq!((ai.last_action, intent.move_dir), (Some("dodge"), Some(Dir::Right)), "108 px in: it backs out");
    }

    /// A disc's exits: straight out first, turned further each way after,
    /// and from the tank's side when the point is the middle; its posts on
    /// the axes.
    #[test]
    fn a_discs_exits_and_posts() {
        let d = Danger { shape: DangerShape::Disc { at: Position::new(0.0, 0.0), radius: 100.0 }, owner: None, slack: 0.0 };
        let exits = d.exits(Position::new(50.0, 0.0), 10.0, Position::new(0.0, 0.0), Dir::Up);
        assert!(exits[0].distance_to(Position::new(110.0, 0.0)) < 1e-3);
        assert!(exits.iter().all(|e| (e.length() - 110.0).abs() < 1e-3 && e.x > -60.0), "never back through the middle: {exits:?}");
        let from_middle = d.exits(Position::new(0.0, 0.0), 10.0, Position::new(0.0, 40.0), Dir::Up);
        assert!(from_middle[0].distance_to(Position::new(0.0, 110.0)) < 1e-3, "out on the tank's side");
        assert_eq!(d.posts(10.0), Dir::ALL.map(|dir| Position::new(dir.vec().x * 110.0, dir.vec().y * 110.0)));
    }
}
