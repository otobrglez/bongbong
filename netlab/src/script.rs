//! The scripted players: what each seat does, as a function of the time
//! since its round began, so an online client and the local twin are fed
//! the same inputs at the same moments.
//!
//! A script is stateful only in its taps: a tap is one frame of trigger on
//! the first frame at or past each tap time, so a slow frame never loses
//! one and a fast frame never doubles one.

use bongbong::ai::Intent;
use bongbong::tank::Dir;

/// What the run plays.
#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Scenario {
    /// The host drives a rectangle, the guest stands still: own-input
    /// latency, and the pacing and lag of a remote hull.
    Drive,
    /// The host faces the enemies and taps fire while strafing slowly: the
    /// shot ledger and incoming fire.
    Shoot,
    /// Both seats drive rectangles out of phase and tap fire.
    Duel,
}

impl Scenario {
    pub fn name(self) -> &'static str {
        match self {
            Scenario::Drive => "drive",
            Scenario::Shoot => "shoot",
            Scenario::Duel => "duel",
        }
    }

    /// The enemies the scenario is played against when the command line
    /// names no count.
    pub fn default_enemies(self) -> usize {
        match self {
            Scenario::Drive => 0,
            Scenario::Shoot => 4,
            Scenario::Duel => 2,
        }
    }

    /// The script seat `seat` (0 the host, 1 the guest) runs. `aim` is the
    /// direction from the host toward the enemies when the round began,
    /// read off the round itself.
    pub fn script(self, seat: usize, aim: Dir) -> Script {
        match (self, seat) {
            (Scenario::Drive, 0) => Script::rectangle(0.0, None),
            (Scenario::Shoot, 0) => Script::Shoot { aim, taps: Taps::new(SHOOT_TAP_SECONDS, 0.2) },
            (Scenario::Duel, 0) => Script::rectangle(0.0, Some(Taps::new(DUEL_TAP_SECONDS, 0.0))),
            (Scenario::Duel, _) => Script::rectangle(RECTANGLE_SECONDS / 2.0, Some(Taps::new(DUEL_TAP_SECONDS, DUEL_TAP_SECONDS / 2.0))),
            _ => Script::Idle,
        }
    }
}

/// The rectangle's legs: right, down, left, up, in seconds.
pub const RECTANGLE: [(Dir, f64); 4] = [(Dir::Right, 1.2), (Dir::Down, 0.8), (Dir::Left, 1.2), (Dir::Up, 0.8)];

/// One lap of the rectangle.
pub const RECTANGLE_SECONDS: f64 = 4.0;

/// How often the shooter taps the trigger.
pub const SHOOT_TAP_SECONDS: f64 = 0.4;

/// How often each duellist taps.
pub const DUEL_TAP_SECONDS: f64 = 0.6;

/// The shooter's cycle: a short strafe step, then facing the enemies.
pub const STRAFE_CYCLE_SECONDS: f64 = 2.0;

/// How long each strafe step drives.
pub const STRAFE_STEP_SECONDS: f64 = 0.3;

/// Trigger taps every `period` seconds from `offset`.
#[derive(Clone, Copy, Debug)]
pub struct Taps {
    period: f64,
    offset: f64,
    /// The last tap index fired.
    last: Option<i64>,
}

impl Taps {
    pub fn new(period: f64, offset: f64) -> Taps {
        Taps { period, offset, last: None }
    }

    /// Whether this frame, at script time `t`, carries a tap.
    fn fire(&mut self, t: f64) -> bool {
        if t < self.offset {
            return false;
        }
        let index = ((t - self.offset) / self.period).floor() as i64;
        if self.last.is_some_and(|l| l >= index) {
            return false;
        }
        self.last = Some(index);
        true
    }
}

/// One seat's script.
#[derive(Clone, Copy, Debug)]
pub enum Script {
    /// Hands off the controls.
    Idle,
    /// The rectangle, started `phase` seconds into its lap, with optional
    /// taps.
    Rectangle { phase: f64, taps: Option<Taps> },
    /// Strafe a step along the axis across `aim`, face `aim`, tap.
    Shoot { aim: Dir, taps: Taps },
}

impl Script {
    pub fn rectangle(phase: f64, taps: Option<Taps>) -> Script {
        Script::Rectangle { phase, taps }
    }

    /// The seat's intent on a frame at script time `t` seconds.
    pub fn intent(&mut self, t: f64) -> Intent {
        match self {
            Script::Idle => Intent::default(),
            Script::Rectangle { phase, taps } => {
                let fire = taps.as_mut().is_some_and(|taps| taps.fire(t));
                Intent { move_dir: Some(rectangle_leg(t + *phase)), fire, ..Intent::default() }
            }
            Script::Shoot { aim, taps } => {
                let fire = taps.fire(t);
                let cycle = (t / STRAFE_CYCLE_SECONDS).floor() as i64;
                let into = t - cycle as f64 * STRAFE_CYCLE_SECONDS;
                if into < STRAFE_STEP_SECONDS {
                    let (a, b) = across(*aim);
                    let step = if cycle.rem_euclid(2) == 0 { a } else { b };
                    Intent { move_dir: Some(step), fire, ..Intent::default() }
                } else {
                    Intent { face: Some(*aim), fire, ..Intent::default() }
                }
            }
        }
    }
}

/// The rectangle's leg at lap time `t`.
pub fn rectangle_leg(t: f64) -> Dir {
    let mut into = t.rem_euclid(RECTANGLE_SECONDS);
    for (dir, seconds) in RECTANGLE {
        if into < seconds {
            return dir;
        }
        into -= seconds;
    }
    RECTANGLE[0].0
}

/// The two directions across `dir`.
fn across(dir: Dir) -> (Dir, Dir) {
    match dir {
        Dir::Up | Dir::Down => (Dir::Right, Dir::Left),
        Dir::Left | Dir::Right => (Dir::Down, Dir::Up),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_rectangle_legs_follow_the_lap() {
        assert_eq!(rectangle_leg(0.0), Dir::Right);
        assert_eq!(rectangle_leg(1.19), Dir::Right);
        assert_eq!(rectangle_leg(1.2), Dir::Down);
        assert_eq!(rectangle_leg(2.1), Dir::Left);
        assert_eq!(rectangle_leg(3.5), Dir::Up);
        assert_eq!(rectangle_leg(4.0), Dir::Right);
    }

    #[test]
    fn a_tap_is_one_frame_per_period_whatever_the_frame_rate() {
        for fps in [30.0, 60.0, 144.0] {
            let mut taps = Taps::new(0.4, 0.2);
            let fired = (0..(10.0 * fps) as usize).filter(|i| taps.fire(*i as f64 / fps)).count();
            assert_eq!(fired, 25, "at {fps} fps");
        }
    }
}
