//! The local twin: the same scripts through a local two-seat `Game`,
//! stepped the way the window steps a local round, recorded frame by
//! frame the way the online clients are. This is what "feels local"
//! means, measured rather than assumed.
//!
//! The twin runs on virtual time - frame `k` is drawn at exactly `k / fps`
//! seconds - so it is the frame rate's ideal: a local round on a display
//! that never misses a frame. The online clients draw on a real frame
//! loop, and whatever that loop's own jitter costs them is reported
//! beside the rest (`Metrics::frame_interval_ms`); the metrics that could
//! mistake it for the network's cost measure in frames or against each
//! frame's own `dt`.

use std::time::Instant;

use bongbong::level::{LevelOverrides, Mission};
use bongbong::map::MapFile;
use bongbong::simulation::{Game, Input, Outcome, PlayerCount};
use bongbong::tuning;
use bongbong::{PHYSICS_FIXED_DT, SIM_MAX_STEPS_PER_FRAME};

use crate::client::aim_of;
use crate::sample::{self, FrameSample};
use crate::script::Scenario;

/// `app::StepClock`, which is behind the window's `render` feature: real
/// frame time paid out as whole fixed steps, at most
/// `SIM_MAX_STEPS_PER_FRAME` a frame with the rest dropped.
#[derive(Default)]
pub struct StepClock {
    owed: f32,
}

impl StepClock {
    pub fn advance(&mut self, dt: f32) -> u32 {
        let cap = SIM_MAX_STEPS_PER_FRAME as f32 * PHYSICS_FIXED_DT;
        self.owed = (self.owed + dt.max(0.0)).min(cap);
        let mut steps = 0;
        while self.owed >= PHYSICS_FIXED_DT && steps < SIM_MAX_STEPS_PER_FRAME {
            self.owed -= PHYSICS_FIXED_DT;
            steps += 1;
        }
        if steps == SIM_MAX_STEPS_PER_FRAME {
            self.owed = 0.0;
        }
        steps
    }
}

/// The twin's setup: the round a room of two would start on the same
/// terms.
#[derive(Clone, Debug)]
pub struct TwinPlan {
    pub map: MapFile,
    pub mission: Mission,
    pub seed: u64,
    pub scenario: Scenario,
    pub fps: f64,
    pub seconds: f64,
    /// The most taps each script makes (`Scenario::script`).
    pub tap_limit: Option<u32>,
}

/// A local two-seat round, the way the room builds one: the map, the
/// mission, both seats and the pinned seed, under the tuning patch a room
/// of two plays with.
pub fn build(plan: &TwinPlan) -> Game {
    let patch = bongbong_server::room::tuning_patch(2);
    if tuning::submit_json(&patch).is_ok() {
        tuning::apply_pending();
    }
    let mut game = Game::default();
    game.map = plan.map.clone();
    game.level_overrides = LevelOverrides { mission: Some(plan.mission), ..LevelOverrides::default() };
    game.players = PlayerCount::TWO;
    game.seed_override = Some(plan.seed);
    let (w, h) = game.map.field_size();
    game.init(w, h);
    game
}

/// Play the twin and record every frame. One record serves both seats'
/// views: in a local round the host and the guest look at the same world.
pub fn run(plan: &TwinPlan) -> Vec<FrameSample> {
    let mut game = build(plan);
    let (w, h) = game.map.field_size();
    let aim = aim_of(&game);
    let mut scripts = [plan.scenario.script(0, aim, plan.tap_limit), plan.scenario.script(1, aim, plan.tap_limit)];
    let mut clock = StepClock::default();
    let mut carried = Input::default();
    let dt = (1.0 / plan.fps) as f32;
    let frames = (plan.seconds * plan.fps).round() as usize;
    let mut out = Vec::with_capacity(frames);
    for k in 0..frames {
        let t = k as f64 / plan.fps;
        let intents = [scripts[0].intent(t), scripts[1].intent(t)];
        let input = Input::two(intents[0], intents[1]).or_presses(carried);
        let mut s = FrameSample { t_ms: t * 1000.0, script_s: t, ..FrameSample::default() };
        sample::read_intent(0, &intents[0], &mut s);
        sample::read_intent(1, &intents[1], &mut s);
        let steps = clock.advance(dt);
        let cpu = Instant::now();
        for i in 0..steps {
            let step_input = if i == 0 { input } else { input.held_only() };
            game.update(step_input, PHYSICS_FIXED_DT, w, h);
            sample::read_events(game.events(), &mut s.events);
        }
        s.cpu_us = cpu.elapsed().as_secs_f64() * 1e6;
        carried = if steps == 0 { input } else { Input::default() };
        sample::read_picture(&game, None, &mut s);
        out.push(s);
        // The recording stops where an online client's does: the round is
        // decided, and what follows is the end screen and a new round.
        if game.outcome() != Outcome::Playing {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_step_clock_pays_one_step_a_frame_at_sixty() {
        let mut clock = StepClock::default();
        let steps: Vec<u32> = (0..120).map(|_| clock.advance(1.0 / 60.0)).collect();
        assert!(steps.iter().all(|s| *s == 1), "{steps:?}");
    }

    #[test]
    fn the_step_clock_alternates_at_one_twenty_and_caps_a_stall() {
        let mut clock = StepClock::default();
        let total: u32 = (0..240).map(|_| clock.advance(1.0 / 120.0)).sum();
        assert!((119..=120).contains(&total), "{total}");
        assert_eq!(clock.advance(1.0), SIM_MAX_STEPS_PER_FRAME);
        assert_eq!(clock.advance(0.0), 0, "the rest of the stall is dropped");
    }
}
