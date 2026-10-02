//! The pacing director (docs/large-maps-follow-camera.md section 12,
//! docs/large-maps-patterns.md "Pacing director"): Left 4 Dead's cycle of
//! build-up, peak and relax, adapted to the waves of a field map. Each seat
//! carries an **intensity** from 0 to 1, read from the round alone: the
//! damage its tank takes - and the damage the players' frog takes - jolts
//! it up (`director_hurt_full` of a pool is the whole way), the enemies
//! standing inside its sight box hold it up (`director_crowd_full` of them
//! hold it at the top), and with neither it falls back to them over
//! `director_fall_seconds`. The team stands at its most pressed seat, and
//! that is what paces the breather before the next wave:
//!
//! - **Peak** (`director_peak`): the breather stands still, so the next
//!   wave is held while the team is fighting the last one - a wave that
//!   times out mid-fight waits for the fight.
//! - **Relax**: a peak owes the team `director_relax_seconds` of rest once
//!   it passes, and no breather ends before that is paid, so the breather
//!   after a hard fight stretches - a peak in the middle of one pushes it
//!   back, and one that comes after the team has already rested in the
//!   wave's tail does not.
//! - **Calm** (`director_calm`): with no peak to recover from and nothing
//!   happening, the breather runs `director_calm_rate` times as fast.
//! - **Bounds**: a breather lasts at least `director_breather_min_seconds`
//!   and never more than `director_breather_max_seconds`, holds included,
//!   so the round goes on whatever the team does.
//!
//! An arena keeps the plain breather (`wave_gap_seconds`) and replays as it
//! always has, and so does every map with `director_enabled` off. No RNG,
//! no tie to break: the intensities are sums and maxima of the round's own
//! numbers. A replica needs nothing new: the breather's time left is what
//! `RoundState::next_wave` already carries, so its `WAVE N` banner stays up
//! for as long as the room's, holds included.

use crate::ai::{Ai, in_sight_box};
use crate::frog::Frog;
use crate::tank::Tank;
use crate::tuning::{Tuning, tuning};
use crate::{MAX_DAMAGE, MAX_SEATS, Position};

use super::{Game, with_tank};

/// What a seat on the field shows the director this tick
/// (`Game::wave_phase`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct SeatReading {
    /// Its tank's damage and shield (`Tank::damage`, `Tank::shield_hp`):
    /// what it lost since the last tick is the jolt.
    pub(super) damage: f32,
    pub(super) shield: f32,
    /// Live enemies inside its sight box (`ai::in_sight_box`).
    pub(super) crowd: usize,
}

/// The director's memory for one round: reset by `Game::init`.
#[derive(Clone, Debug, Default)]
pub(crate) struct Director {
    /// Each seat's intensity, 0 to 1, by seat index.
    intensity: [f32; MAX_SEATS],
    /// Each seat's damage and shield at the last tick it stood on the
    /// field: what this tick's loss is measured from. Cleared while it is
    /// off the field, so a seat back from a wreck starts from its fresh
    /// tank rather than a jolt.
    last: [Option<(f32, f32)>; MAX_SEATS],
    /// Whether each seat stood on the field at the last tick, which is
    /// what the team's intensity is the most of.
    on_field: [bool; MAX_SEATS],
    /// The players' frog's health at the last tick, while it lives.
    frog_health: Option<f32>,
    /// Seconds of rest the last peak still owes the team.
    relax: f32,
    /// Seconds the current breather has run, holds included.
    breather: f32,
}

impl Director {
    /// One tick: every seat on the field (`seats[i]`, `None` for a seat
    /// that is empty, a wreck or in a gate lane) and the players' frog's
    /// health and pool while it lives. Returns the team's intensity, the
    /// highest of its seats on the field (0 with none).
    pub(super) fn observe(&mut self, seats: &[Option<SeatReading>; MAX_SEATS], frog: Option<(f32, f32)>, dt: f32, t: &Tuning) -> f32 {
        let full = t.director_hurt_full.max(f32::EPSILON);
        // The frog's loss jolts every seat: it is the whole team's.
        let frog_jolt = match (frog, self.frog_health) {
            (Some((health, pool)), Some(before)) if pool > 0.0 => (before - health).max(0.0) / pool / full,
            _ => 0.0,
        };
        self.frog_health = frog.map(|(health, _)| health);
        let mut team = 0.0f32;
        for (i, reading) in seats.iter().enumerate() {
            let Some(r) = reading else {
                self.last[i] = None;
                self.on_field[i] = false;
                self.intensity[i] = (self.intensity[i] - dt / t.director_fall_seconds.max(f32::EPSILON)).max(0.0);
                continue;
            };
            let lost = self.last[i].map_or(0.0, |(damage, shield)| (r.damage - damage).max(0.0) + (shield - r.shield).max(0.0));
            self.last[i] = Some((r.damage, r.shield));
            self.on_field[i] = true;
            let jolt = lost / MAX_DAMAGE / full + frog_jolt;
            let crowd = (r.crowd as f32 / t.director_crowd_full.max(f32::EPSILON)).min(1.0);
            self.intensity[i] = settle(self.intensity[i], jolt, crowd, dt, t.director_fall_seconds);
            team = team.max(self.intensity[i]);
        }
        if team >= t.director_peak {
            self.relax = t.director_relax_seconds;
        } else {
            self.relax = (self.relax - dt).max(0.0);
        }
        team
    }

    /// The breather a wave that was just cleared or timed out leaves before
    /// the next: `wave_gap_seconds`, or the rest a recent peak still owes
    /// the team where that is longer, within the director's bounds.
    pub(super) fn breather_length(&mut self, t: &Tuning) -> f32 {
        self.breather = 0.0;
        t.wave_gap_seconds.max(self.relax).clamp(t.director_breather_min_seconds, t.director_breather_max_seconds.max(t.director_breather_min_seconds))
    }

    /// The breather's time left after one tick with the team at `team`:
    /// held at a peak (and never under the rest the peak owes), down at
    /// the calm rate while nothing is happening and nothing is owed, down
    /// at real time otherwise - never under what is left of
    /// `director_breather_min_seconds`, and 0, the next wave, once it has
    /// run `director_breather_max_seconds`.
    pub(super) fn pace_breather(&mut self, left: f32, team: f32, dt: f32, t: &Tuning) -> f32 {
        self.breather += dt;
        if self.breather >= t.director_breather_max_seconds {
            return 0.0;
        }
        if team >= t.director_peak {
            return left.max(self.relax);
        }
        let rate = if self.relax <= 0.0 && team <= t.director_calm { t.director_calm_rate } else { 1.0 };
        (left - dt * rate).max(self.relax).max(t.director_breather_min_seconds - self.breather).max(0.0)
    }

    /// Seat `seat`'s intensity, 0 to 1 (0 for a seat past `MAX_SEATS`).
    pub(crate) fn intensity(&self, seat: usize) -> f32 {
        self.intensity.get(seat).copied().unwrap_or(0.0)
    }

    /// The team's intensity as the last tick left it: the most pressed
    /// seat on the field.
    pub(crate) fn team(&self) -> f32 {
        (0..MAX_SEATS).filter(|&i| self.on_field[i]).map(|i| self.intensity[i]).fold(0.0, f32::max)
    }

    /// Seconds of rest the last peak still owes the team.
    pub(crate) fn relax(&self) -> f32 {
        self.relax
    }
}

impl Game {
    /// The director's tick (`Director::observe`) on this round as it
    /// stands: every seat on the field with its tank's damage, shield and
    /// the live enemies inside its sight box, and the players' frog while
    /// it lives. The team's intensity comes back.
    pub(super) fn observe_pressure(&mut self, dt: f32) -> f32 {
        let enemies: Vec<Position> =
            self.world.query::<&Tank>().with::<&Ai>().iter().filter(|t| !t.is_wreck()).map(|t| t.position).collect();
        let mut seats = [None; MAX_SEATS];
        for (reading, seat) in seats.iter_mut().zip(self.seats_on_field()) {
            *reading = seat.and_then(|e| {
                with_tank(&self.world, e, |t| {
                    (!t.is_wreck()).then(|| SeatReading {
                        damage: t.damage,
                        shield: t.shield_hp,
                        crowd: enemies.iter().filter(|&&p| in_sight_box(t.position, p)).count(),
                    })
                })
            });
        }
        let frog = self.frog.and_then(|e| self.world.get::<&Frog>(e).ok().and_then(|fr| (!fr.is_dead()).then_some((fr.health, fr.max_health))));
        self.director.observe(&seats, frog, dt, &tuning())
    }

    /// The pacing director's reading of the round, for the dev server's
    /// `status` and the tests: each seat's intensity, the team's, and the
    /// rest a peak still owes it. All zero where nothing paces anything:
    /// an arena, a band round, and a replica, whose room does the pacing.
    pub fn pacing(&self) -> Pacing {
        Pacing {
            seats: (0..self.players.count()).map(|i| self.director.intensity(i)).collect(),
            team: self.director.team(),
            relax: self.director.relax(),
        }
    }
}

/// `Game::pacing`: the director's reading of the round.
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct Pacing {
    /// Each seat's intensity, 0 to 1, by seat index.
    pub seats: Vec<f32>,
    /// The team's: its most pressed seat on the field.
    pub team: f32,
    /// Seconds of rest the last peak still owes the team.
    pub relax: f32,
}

/// One seat's intensity after a tick: `before`, jolted up by `jolt`
/// (capped at 1), then held up by `crowd` - raised to it at once - or
/// falling toward it at one whole intensity per `fall_seconds`.
fn settle(before: f32, jolt: f32, crowd: f32, dt: f32, fall_seconds: f32) -> f32 {
    let jolted = (before + jolt).min(1.0);
    if crowd >= jolted { crowd } else { (jolted - dt / fall_seconds.max(f32::EPSILON)).max(crowd) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DT: f32 = 1.0 / 60.0;

    fn alone(reading: SeatReading) -> [Option<SeatReading>; MAX_SEATS] {
        let mut seats = [None; MAX_SEATS];
        seats[0] = Some(reading);
        seats
    }

    fn quiet() -> SeatReading {
        SeatReading { damage: 0.0, shield: 0.0, crowd: 0 }
    }

    #[test]
    fn damage_jolts_a_seat_and_it_falls_back_over_the_fall_time() {
        let t = Tuning::DEFAULT;
        let mut d = Director::default();
        d.observe(&alone(quiet()), None, DT, &t);
        let hurt = MAX_DAMAGE * t.director_hurt_full;
        let team = d.observe(&alone(SeatReading { damage: hurt, ..quiet() }), None, DT, &t);
        // The top, less the one tick's fall that follows the jolt.
        let one_tick = DT / t.director_fall_seconds;
        assert!((team - (1.0 - one_tick)).abs() < 1e-5, "the whole of `director_hurt_full` in one tick is the top: {team}");
        // Then nothing: one whole intensity per `director_fall_seconds`.
        let ticks = (t.director_fall_seconds * 0.5 / DT).round() as usize;
        for _ in 0..ticks {
            d.observe(&alone(SeatReading { damage: hurt, ..quiet() }), None, DT, &t);
        }
        assert!((d.team() - 0.5).abs() < 0.01, "half way down after half the fall: {}", d.team());
    }

    #[test]
    fn enemies_in_the_box_hold_a_seat_up_and_the_frogs_loss_is_everyones() {
        let t = Tuning::DEFAULT;
        let mut d = Director::default();
        let crowd = t.director_crowd_full.ceil() as usize;
        for _ in 0..600 {
            d.observe(&alone(SeatReading { crowd, ..quiet() }), None, DT, &t);
        }
        assert_eq!(d.team(), 1.0, "a full crowd holds the seat at the top, however long");
        let mut two = [None; MAX_SEATS];
        two[0] = Some(quiet());
        two[1] = Some(quiet());
        let mut d = Director::default();
        d.observe(&two, Some((40.0, 40.0)), DT, &t);
        let lost = 40.0 * t.director_hurt_full * 0.5;
        d.observe(&two, Some((40.0 - lost, 40.0)), DT, &t);
        let half = 0.5 - DT / t.director_fall_seconds;
        assert!((d.intensity(0) - half).abs() < 1e-4 && (d.intensity(1) - half).abs() < 1e-4, "{} {}", d.intensity(0), d.intensity(1));
    }

    #[test]
    fn a_seat_back_from_a_wreck_starts_from_its_fresh_tank() {
        let t = Tuning::DEFAULT;
        let mut d = Director::default();
        d.observe(&alone(SeatReading { damage: 60.0, ..quiet() }), None, DT, &t);
        // A wreck, then a fresh tank: no reading in between, no jolt after.
        d.observe(&[None; MAX_SEATS], None, DT, &t);
        let team = d.observe(&alone(quiet()), None, DT, &t);
        assert_eq!(team, 0.0);
    }

    #[test]
    fn a_peak_holds_the_breather_then_owes_the_team_its_rest() {
        let t = Tuning::DEFAULT;
        let mut d = Director::default();
        let crowd = t.director_crowd_full.ceil() as usize;
        d.observe(&alone(SeatReading { crowd, ..quiet() }), None, DT, &t);
        let mut left = d.breather_length(&t);
        assert_eq!(left, t.wave_gap_seconds.max(t.director_relax_seconds), "a peak's rest is owed from the start");
        for _ in 0..600 {
            let team = d.observe(&alone(SeatReading { crowd, ..quiet() }), None, DT, &t);
            left = d.pace_breather(left, team, DT, &t);
        }
        assert_eq!(left, t.director_relax_seconds, "held while the team is at its peak");
        // The fight ends: the intensity falls under the peak, and from
        // there the breather runs out no sooner than the rest it owes.
        let off_peak = (1.0 - t.director_peak) * t.director_fall_seconds;
        let mut ticks = 0;
        while left > 0.0 {
            let team = d.observe(&alone(quiet()), None, DT, &t);
            left = d.pace_breather(left, team, DT, &t);
            ticks += 1;
        }
        let seconds = ticks as f32 * DT;
        let owed = off_peak + t.director_relax_seconds;
        assert!((seconds - owed).abs() <= 3.0 * DT, "rested {seconds:.2}s, owed {owed:.2}s");
    }

    #[test]
    fn a_calm_breather_runs_fast_but_never_under_its_floor_nor_over_its_ceiling() {
        let t = Tuning::DEFAULT;
        let mut d = Director::default();
        let mut left = d.breather_length(&t);
        assert_eq!(left, t.wave_gap_seconds.clamp(t.director_breather_min_seconds, t.director_breather_max_seconds));
        let mut ticks = 0;
        while left > 0.0 {
            let team = d.observe(&alone(quiet()), None, DT, &t);
            left = d.pace_breather(left, team, DT, &t);
            ticks += 1;
        }
        let seconds = ticks as f32 * DT;
        let calm = (t.wave_gap_seconds / t.director_calm_rate).max(t.director_breather_min_seconds);
        assert!((seconds - calm).abs() <= 2.0 * DT, "a calm breather is {calm:.2}s, ran {seconds:.2}s");
        // Held at a peak for good: the ceiling calls the wave anyway.
        let crowd = t.director_crowd_full.ceil() as usize;
        let mut d = Director::default();
        let mut left = d.breather_length(&t);
        let mut ticks = 0;
        while left > 0.0 {
            let team = d.observe(&alone(SeatReading { crowd, ..quiet() }), None, DT, &t);
            left = d.pace_breather(left, team, DT, &t);
            ticks += 1;
        }
        assert!((ticks as f32 * DT - t.director_breather_max_seconds).abs() <= 2.0 * DT, "ran {:.2}s", ticks as f32 * DT);
    }
}
