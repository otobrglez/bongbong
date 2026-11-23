//! What a seat feels of its own fight (docs/effects.md "Being hit"): the
//! shake of its own view, the pulse round that view's edge and the tick of
//! its health readout when it is hurt, and the hold on the blow that
//! wrecks an enemy.
//!
//! Presentation only, owned by `app.rs` beside `fx`: it reads the round
//! after every step - each local seat's hull damage and its count of
//! wrecks (`RoundStats::by_seat`), so every cause of damage shows, a blast
//! and a fire as well as a shot - and never writes it. A replica is read
//! the same way, so a room's seat feels its hits too. The hold is the one
//! thing that reaches back toward the round, and only as the share of real
//! time `app.rs` pays the step clock (`time_scale`), the way the round's
//! last blow is held (`Session::time_scale`): offline only, never under a
//! dev-server lockstep, and nothing the simulation can see.
//!
//! A blow that came from somewhere - a shot, a blast, a slug - shoves the
//! seat's own view away from where it came from instead of shaking it
//! (`Feedback::shove`): the follow camera takes the shove through the
//! kick's layer (`follow::SeatTank::shove`), an arena's picture is shifted
//! by it with the shake (`shockwave::camera_shake_with`). A hurt with no
//! direction - fire, lava, ooze, a strike landing on the hull itself -
//! shakes as ever.
//!
//! Reduced motion (`motion.rs`) takes the shake, the shove, the pulse and
//! the hold away and leaves the tick, which then only lights the number.

use crate::math::{Color, Vec2};
use crate::shockwave::Shockwave;
use crate::simulation::{Game, Outcome};
use crate::tuning::Tuning;
use crate::{MAX_DAMAGE, MAX_SEATS, Position};

/// Whose view of the world a picture is: every seat's - the one view of
/// an arena or of a couch standing together - or one half of a couch's
/// split screen, which is the own view of the seat it follows and of
/// nobody else.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewOf {
    Every,
    /// The first half of a split: every seat's but the one the second
    /// half follows.
    Except(usize),
    /// The second half of a split: its seat's alone.
    Only(usize),
}

impl ViewOf {
    /// Whether this is `seat`'s own view.
    pub fn has(self, seat: usize) -> bool {
        match self {
            ViewOf::Every => true,
            ViewOf::Except(other) => seat != other,
            ViewOf::Only(only) => seat == only,
        }
    }
}

/// One blow a local seat took.
struct Hurt {
    seat: usize,
    /// How hard, 0 to 1: the damage against `hurt_full_damage`. Its
    /// shake is two fifths of `hurt_shock` at nothing and all of it at 1,
    /// since the shake lands on whole 2 px blocks and a lighter one would
    /// round away.
    strength: f32,
    /// Seconds since, on the round's clock.
    age: f32,
    /// The shake of the seat's own view, centred on the hull.
    shock: Shockwave,
    /// The shove of the seat's own view instead, world px at the blow:
    /// away from where it came from (`shove_of`). `None` for a blow with
    /// no direction, which shakes.
    push: Option<Vec2>,
}

/// The pulse round a view's edge as a frame draws it (`pulse_look`).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Pulse {
    /// How many 2 px blocks deep the band is.
    pub depth: i32,
    /// Its colour: a step of the fire ramp's reds, at an opacity in
    /// eighths.
    pub color: Color,
    /// The share of the band's dithered blocks still drawn, 1 to 0.5.
    pub cover: f32,
}

impl Pulse {
    /// How much of the row `d` blocks in from the edge is drawn, before
    /// `cover`: the outer two fifths of the band whole, the rest thinning
    /// inward.
    fn row(&self, d: i32) -> f32 {
        let inward = d as f32 / self.depth.max(1) as f32;
        ((1.0 - inward) / 0.6).min(1.0)
    }

    /// Whether the block `d` blocks in from the edge at block coordinates
    /// (`bx`, `by`) is drawn: its row's share, through the Bayer pattern.
    pub fn block(&self, d: i32, bx: i32, by: i32) -> bool {
        (0..self.depth).contains(&d) && crate::pyro::bayer(bx, by) < self.row(d) * self.cover
    }

    /// Whether every block of the row `d` blocks in is drawn.
    pub fn whole_row(&self, d: i32) -> bool {
        (0..self.depth).contains(&d) && self.row(d) * self.cover >= 1.0
    }
}

/// The pulse `strength` (0 to 1) makes `age` seconds after the blow: a
/// band of blocks round the edge, deeper for a harder blow, stepping down
/// the fire ramp's reds as it ages; over the first half of its life its
/// dithered blocks thin to half, then the ones kept fade in eighths
/// (docs/effects.md, rules 1 to 3). `None` once it is over.
pub fn pulse_look(strength: f32, age: f32, t: &Tuning) -> Option<Pulse> {
    let life = t.hurt_pulse_seconds;
    if life <= 0.0 || age >= life || strength <= 0.0 {
        return None;
    }
    let depth = (t.hurt_pulse_blocks as f32 * (0.3 + 0.7 * strength.min(1.0))).round() as i32;
    if depth <= 0 {
        return None;
    }
    let p = (age / life).clamp(0.0, 1.0);
    let step = if p < 0.2 {
        3
    } else if p < 0.55 {
        2
    } else {
        1
    };
    let cover = 1.0 - p.min(0.5);
    let eighths = if p <= 0.5 { 8.0 } else { (8.0 * (1.0 - (p - 0.5) / 0.5)).ceil() };
    Some(Pulse { depth, color: crate::pyro::alpha(crate::pyro::FIRE[step], eighths / 8.0), cover })
}

/// The health readout's tick `age` seconds after its seat was hurt: how
/// lit the number is, 3 down to 1 in steps, `None` once over.
pub fn tick_look(age: f32, t: &Tuning) -> Option<u8> {
    let life = t.hurt_tick_seconds;
    if life <= 0.0 || age >= life {
        return None;
    }
    Some(3 - ((age / life) * 3.0) as u8)
}

/// What `Feedback` reads of one simulated frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Reading {
    /// `Game::frame` and `Game::round_seed`: which frame of which round.
    pub frame: u64,
    pub round: u64,
    /// The round is still undecided.
    pub playing: bool,
    /// Each seat's hull damage and where it stands; `None` for a seat
    /// with no tank.
    pub hulls: [Option<(f32, Position)>; MAX_SEATS],
    /// The enemies each seat has wrecked (`RoundStats::by_seat`).
    pub kills: [u32; MAX_SEATS],
    /// The way the blows that landed on each seat's hull this frame drove
    /// it, a unit vector from where they struck through the hull
    /// (`blow_of`); `None` where nothing struck it from anywhere.
    pub blows: [Option<Vec2>; MAX_SEATS],
}

impl Reading {
    /// The frame `game` is at.
    pub fn of(game: &Game) -> Reading {
        let mut hulls = [None; MAX_SEATS];
        for (seat, hull) in hulls.iter_mut().enumerate() {
            *hull = game.seat(seat).and_then(|e| game.world.get::<&crate::tank::Tank>(e).ok().map(|tank| (tank.damage, tank.position)));
        }
        let mut blows = [None; MAX_SEATS];
        for (seat, blow) in blows.iter_mut().enumerate() {
            if let Some((_, hull)) = hulls[seat] {
                *blow = blow_of(hull, game.events(), seat);
            }
        }
        Reading { frame: game.frame(), round: game.round_seed(), playing: game.outcome() == Outcome::Playing, hulls, kills: game.round_stats().by_seat, blows }
    }
}

/// The way this frame's blows on `seat`'s hull at `hull` drove it: from
/// each `Event::Hit` that landed damage on it to the hull, weighted by the
/// damage, as a unit vector. A blow struck on the hull's own point (a
/// tesla's arc) has no way, and contacts report no damage in their
/// events, so fire, lava and ooze have none either; `None` when nothing
/// gives one, or the blows cancel out.
pub fn blow_of(hull: Position, events: &[crate::simulation::Event], seat: usize) -> Option<Vec2> {
    use crate::simulation::{Event, HitTarget};
    let mut way = Vec2::new(0.0, 0.0);
    for event in events {
        if let Event::Hit { target: HitTarget::Player { player }, damage, x, y, .. } = *event {
            let off = Vec2::new(hull.x - x, hull.y - y);
            let len = off.length();
            if player as usize == seat && damage > 0.0 && len >= 1.0 && len.is_finite() {
                way = way + off * (damage / len);
            }
        }
    }
    let len = way.length();
    (len > 1e-3 && len.is_finite()).then(|| way * (1.0 / len))
}

/// How far a blow of `strength` (0 to 1) shoves its seat's view, world px:
/// `camera_hurt_shove` times the shake it stands in for - its length
/// (`camera_shake_magnitude` times `hurt_shock`, two fifths of it at
/// nothing and all of it at 1) under `screen_fx_intensity` - never past the
/// shake's cap (`camera_shake_magnitude` times `camera_shake_max_stack`).
pub fn shove_length(strength: f32, t: &Tuning) -> f32 {
    let magnitude = (t.camera_shake_magnitude * t.screen_fx_intensity).max(0.0);
    let length = t.camera_hurt_shove.max(0.0) * magnitude * t.hurt_shock.max(0.0) * (0.4 + 0.6 * strength.clamp(0.0, 1.0));
    length.min(magnitude * t.camera_shake_max_stack)
}

/// What is left `age` seconds after a blow of a shove that began as
/// `push`: all of it at the blow, falling to about a third every
/// `camera_hurt_shove_seconds`; nothing once under a hundredth of a pixel.
pub fn shove_left(push: Vec2, age: f32, t: &Tuning) -> Vec2 {
    let seconds = t.camera_hurt_shove_seconds;
    let left = if age <= 0.0 { push } else if seconds > 0.0 { push * crate::math::exp(-age / seconds) } else { Vec2::new(0.0, 0.0) };
    if left.length() < 0.01 { Vec2::new(0.0, 0.0) } else { left }
}

/// What the local seats feel (the module docs).
#[derive(Default)]
pub struct Feedback {
    /// The frame last read.
    last: Option<Reading>,
    /// The blows still showing, oldest first.
    hurts: Vec<Hurt>,
    /// Real seconds the hold on a kill still has to run, and real seconds
    /// since the last one began.
    hold: f32,
    since_hold: Option<f32>,
}

impl Feedback {
    /// Read the simulated frame `game` is at, once (`read`).
    pub fn observe_events(&mut self, game: &Game, seats: &[u8], t: &Tuning, hold: bool) {
        if self.last.is_some_and(|last| (last.frame, last.round) == (game.frame(), game.round_seed())) {
            return;
        }
        self.read(Reading::of(game), seats, t, hold);
    }

    /// Take a frame in: what each of `seats` (the seats this screen
    /// plays) lost of its hull since the frame read before, and whether
    /// one of them wrecked an enemy - which holds the round where `hold`
    /// allows it (an offline round at full motion) and the round is still
    /// undecided, its last blow having a hold of its own. Another round,
    /// or a frame from before the last, starts over with nothing showing.
    pub fn read(&mut self, now: Reading, seats: &[u8], t: &Tuning, hold: bool) {
        let before = self.last.replace(now).filter(|last| last.round == now.round && last.frame < now.frame);
        let Some(before) = before else {
            self.hurts.clear();
            self.hold = 0.0;
            return;
        };
        for seat in seats.iter().map(|&seat| seat as usize).filter(|&seat| seat < MAX_SEATS) {
            if let (Some((was, _)), Some((damage, at))) = (before.hulls[seat], now.hulls[seat]) {
                let lost = damage.min(MAX_DAMAGE) - was.min(MAX_DAMAGE);
                if lost > 0.0 && lost >= t.hurt_min_damage {
                    let strength = (lost / t.hurt_full_damage.max(1.0)).clamp(0.0, 1.0);
                    self.hurts.retain(|h| h.seat != seat || h.strength > strength);
                    let push = now.blows[seat].filter(|_| t.camera_hurt_shove > 0.0).map(|way| way * shove_length(strength, t));
                    self.hurts.push(Hurt { seat, strength, age: 0.0, shock: Shockwave::scaled(at, t.hurt_shock * (0.4 + 0.6 * strength)), push });
                }
            }
            let killed = now.kills[seat] > before.kills[seat];
            if killed && hold && now.playing && t.kill_hitstop_seconds > 0.0 && self.since_hold.is_none_or(|since| since >= t.kill_hitstop_gap_seconds) {
                self.hold = t.kill_hitstop_seconds;
                self.since_hold = Some(0.0);
            }
        }
    }

    /// Age it: the blows on the time the round on screen ran (`round_dt`,
    /// none while it stands still), the hold on real time.
    pub fn tick(&mut self, round_dt: f32, real_dt: f32, t: &Tuning) {
        let life = t.hurt_pulse_seconds.max(t.hurt_tick_seconds).max(t.camera_shake_duration).max(t.camera_hurt_shove_seconds * 4.0);
        self.hurts.retain_mut(|h| {
            h.age += round_dt;
            h.shock.time += round_dt;
            h.age < life
        });
        self.hold = (self.hold - real_dt.max(0.0)).max(0.0);
        if let Some(since) = &mut self.since_hold {
            *since += real_dt.max(0.0);
        }
    }

    /// The share of real time the round runs at: none while a kill is
    /// held, all of it otherwise.
    pub fn time_scale(&self) -> f32 {
        if self.hold > 0.0 { 0.0 } else { 1.0 }
    }

    /// The shakes of the blows the seats of `view` took that have no
    /// shove in their place, for `shockwave::camera_shake_of`.
    pub fn shakes(&self, view: ViewOf) -> impl Iterator<Item = &Shockwave> {
        self.hurts.iter().filter(move |h| view.has(h.seat) && h.push.is_none()).map(|h| &h.shock)
    }

    /// The shove the blows the seats of `view` took ask of it now, world
    /// px: the view moves this way (`shove_left` of each), the sum no
    /// longer than one shove can be at its hardest (`shove_length` of 1).
    pub fn shove(&self, view: ViewOf, t: &Tuning) -> Vec2 {
        let sum = self.hurts.iter().filter(|h| view.has(h.seat)).filter_map(|h| h.push.map(|push| shove_left(push, h.age, t))).fold(Vec2::new(0.0, 0.0), |a, b| a + b);
        let (len, most) = (sum.length(), shove_length(1.0, t));
        if len > most && len > 0.0 { sum * (most / len) } else { sum }
    }

    /// The pulse round the edge of `view`: its hardest-hit seat's.
    pub fn pulse(&self, view: ViewOf, t: &Tuning) -> Option<Pulse> {
        self.hurts
            .iter()
            .filter(|h| view.has(h.seat))
            .filter_map(|h| pulse_look(h.strength, h.age, t))
            .max_by(|a, b| (a.depth as f32 * a.cover).total_cmp(&(b.depth as f32 * b.cover)))
    }

    /// The tick of `seat`'s health readout (`tick_look`), `None` when it
    /// has not just been hurt.
    pub fn tick_of(&self, seat: usize, t: &Tuning) -> Option<u8> {
        self.hurts.iter().filter(|h| h.seat == seat).filter_map(|h| tick_look(h.age, t)).max()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Frame `frame` of round 7: seats 0 and 1 with these damages, seat 0
    /// with `kills` wrecks.
    fn reading(frame: u64, damage: [f32; 2], kills: u32) -> Reading {
        let mut hulls = [None; MAX_SEATS];
        hulls[0] = Some((damage[0], Position::new(100.0, 100.0)));
        hulls[1] = Some((damage[1], Position::new(900.0, 400.0)));
        let mut by_seat = [0; MAX_SEATS];
        by_seat[0] = kills;
        Reading { frame, round: 7, playing: true, hulls, kills: by_seat, blows: [None; MAX_SEATS] }
    }

    /// A blow shows on the seat that took it and on no other: its own
    /// view shakes and pulses, its readout ticks, and the other half of a
    /// split screen stands still. A harder blow shakes harder.
    #[test]
    fn a_hurt_seat_feels_it_on_its_own_view_alone() {
        let t = Tuning::default();
        let mut fb = Feedback::default();
        fb.read(reading(1, [0.0, 0.0], 0), &[0, 1], &t, true);
        fb.read(reading(2, [0.0, 15.0], 0), &[0, 1], &t, true);

        assert_eq!(fb.shakes(ViewOf::Only(1)).count(), 1, "the hurt seat's half shakes");
        assert_eq!(fb.shakes(ViewOf::Except(1)).count(), 0, "the other half does not");
        assert_eq!(fb.shakes(ViewOf::Every).count(), 1, "a shared view is the hurt seat's too");
        assert!(fb.pulse(ViewOf::Only(1), &t).is_some() && fb.pulse(ViewOf::Except(1), &t).is_none());
        assert!(fb.tick_of(1, &t).is_some() && fb.tick_of(0, &t).is_none());
        let light = fb.shakes(ViewOf::Every).next().expect("a shake");
        assert!((light.strength - t.hurt_shock * 0.7).abs() < 1e-4, "half of `hurt_full_damage` is half way up from the floor");
        assert_eq!((light.center.x, light.center.y), (900.0, 400.0), "centred on the hull");

        fb.read(reading(3, [0.0, 75.0], 0), &[0, 1], &t, true);
        let hard = fb.shakes(ViewOf::Every).next().expect("a shake").strength;
        assert_eq!(fb.shakes(ViewOf::Every).count(), 1, "the harder blow takes the lighter one's place");
        assert!((hard - t.hurt_shock).abs() < 1e-4, "and is capped at the whole of `hurt_shock`");
    }

    /// A blow that came from somewhere shoves the hurt seat's own view
    /// away from it instead of shaking it, harder for a harder blow, never
    /// past the shake's cap, fading on the round's time; one from nowhere
    /// shakes as ever, and with `camera_hurt_shove` at 0 every blow does.
    #[test]
    fn a_blow_from_somewhere_shoves_and_one_from_nowhere_shakes() {
        let t = Tuning::default();
        let mut fb = Feedback::default();
        fb.read(reading(1, [0.0, 0.0], 0), &[0, 1], &t, true);
        // Seat 1 struck from its left: its view goes right.
        let mut struck = reading(2, [0.0, 15.0], 0);
        struck.blows[1] = Some(Vec2::new(1.0, 0.0));
        fb.read(struck, &[0, 1], &t, true);
        assert_eq!(fb.shakes(ViewOf::Every).count(), 0, "the shove stands in for the shake");
        let shove = fb.shove(ViewOf::Only(1), &t);
        assert!((shove.x - shove_length(0.5, &t)).abs() < 1e-5 && shove.y == 0.0, "{shove:?}");
        assert!((shove_length(0.5, &t) - 1.5 * 3.0 * 0.6 * 0.7).abs() < 1e-5, "one and a half times the shake's length");
        assert_eq!(fb.shove(ViewOf::Except(1), &t), Vec2::new(0.0, 0.0), "the other half of a split feels nothing");
        let cap = t.camera_shake_magnitude * t.camera_shake_max_stack;
        assert!(shove_length(1.0, &t) <= cap && shove_length(1.0, &Tuning { camera_hurt_shove: 2.5, hurt_shock: 3.0, ..t }) == cap);
        // It fades on the round's time and is gone with the blow.
        fb.tick(t.camera_hurt_shove_seconds, 0.0, &t);
        let later = fb.shove(ViewOf::Only(1), &t);
        assert!((later.x - shove.x * crate::math::exp(-1.0)).abs() < 1e-4, "{later:?}");
        fb.tick(1.0, 0.0, &t);
        assert_eq!(fb.shove(ViewOf::Every, &t), Vec2::new(0.0, 0.0));
        // From nowhere - fire, lava, an arc on the hull - it shakes.
        fb.read(reading(3, [0.0, 15.0], 0), &[0, 1], &t, true);
        fb.read(reading(4, [0.0, 40.0], 0), &[0, 1], &t, true);
        assert_eq!((fb.shakes(ViewOf::Every).count(), fb.shove(ViewOf::Every, &t)), (1, Vec2::new(0.0, 0.0)));
        // With no shove asked for, a blow from somewhere shakes too.
        let off = Tuning { camera_hurt_shove: 0.0, ..t };
        let mut fb = Feedback::default();
        fb.read(reading(1, [0.0, 0.0], 0), &[0], &off, true);
        let mut struck = reading(2, [15.0, 0.0], 0);
        struck.blows[0] = Some(Vec2::new(0.0, -1.0));
        fb.read(struck, &[0], &off, true);
        assert_eq!((fb.shakes(ViewOf::Every).count(), fb.shove(ViewOf::Every, &off)), (1, Vec2::new(0.0, 0.0)));
    }

    /// `blow_of`: the way this frame's hits drove a hull, from where they
    /// struck through it, weighted by damage; a hit on the hull's own point,
    /// one that landed nothing, another seat's and blows that cancel out
    /// give none.
    #[test]
    fn a_blow_is_read_from_where_it_struck() {
        use crate::simulation::{Event, HitCause, HitTarget};
        let hull = Position::new(500.0, 300.0);
        let hit = |player: u8, damage: f32, x: f32, y: f32| Event::Hit { target: HitTarget::Player { player }, damage, killed: false, x, y, cause: HitCause::Shell, by: None };
        assert_eq!(blow_of(hull, &[hit(0, 20.0, 488.0, 300.0)], 0), Some(Vec2::new(1.0, 0.0)), "struck on its left, driven right");
        assert_eq!(blow_of(hull, &[hit(0, 20.0, 500.0, 330.0)], 0), Some(Vec2::new(0.0, -1.0)));
        let both = blow_of(hull, &[hit(0, 30.0, 488.0, 300.0), hit(0, 10.0, 500.0, 312.0)], 0).expect("a way");
        assert!(both.x > 0.9 && both.y < 0.0 && (both.length() - 1.0).abs() < 1e-5, "{both:?}");
        assert_eq!(blow_of(hull, &[hit(0, 20.0, 500.0, 300.0)], 0), None, "on the hull's own point");
        assert_eq!(blow_of(hull, &[hit(0, 0.0, 488.0, 300.0)], 0), None, "nothing landed");
        assert_eq!(blow_of(hull, &[hit(1, 20.0, 488.0, 300.0)], 0), None, "another seat's");
        assert_eq!(blow_of(hull, &[hit(0, 20.0, 488.0, 300.0), hit(0, 20.0, 512.0, 300.0)], 0), None, "cancelled out");
    }

    /// A trickle under `hurt_min_damage`, a repair, a seat this screen
    /// does not play, another round and the first frame read show nothing.
    #[test]
    fn a_trickle_a_repair_and_another_screens_seat_show_nothing() {
        let t = Tuning::default();
        let mut fb = Feedback::default();
        fb.read(reading(5, [50.0, 0.0], 0), &[0], &t, true);
        assert_eq!(fb.shakes(ViewOf::Every).count(), 0, "the first frame read has nothing to compare");
        fb.read(reading(6, [50.0 + t.hurt_min_damage * 0.5, 40.0], 0), &[0], &t, true);
        assert_eq!(fb.shakes(ViewOf::Every).count(), 0, "a trickle, and seat 1 is another screen's");
        fb.read(reading(7, [10.0, 40.0], 0), &[0], &t, true);
        assert_eq!(fb.shakes(ViewOf::Every).count(), 0, "a repair");
        fb.read(Reading { round: 8, ..reading(1, [90.0, 40.0], 0) }, &[0], &t, true);
        assert_eq!(fb.shakes(ViewOf::Every).count(), 0, "another round starts over");
    }

    /// The blows age on the round's time and the hold on real time: with
    /// the round standing still a pulse stays as it is, and it is over by
    /// its row's seconds once the round runs.
    #[test]
    fn blows_age_with_the_round_and_end() {
        let t = Tuning::default();
        let mut fb = Feedback::default();
        fb.read(reading(1, [0.0, 0.0], 0), &[0], &t, true);
        fb.read(reading(2, [30.0, 0.0], 0), &[0], &t, true);
        let first = fb.pulse(ViewOf::Every, &t).expect("a pulse");
        fb.tick(0.0, 1.0, &t);
        assert_eq!(fb.pulse(ViewOf::Every, &t), Some(first), "a round that stands still holds its pulse");
        fb.tick(t.hurt_pulse_seconds * 0.75, 0.0, &t);
        let late = fb.pulse(ViewOf::Every, &t).expect("still pulsing");
        assert!(late.cover < first.cover && late.color.a < first.color.a, "thinner and fainter late in its life");
        fb.tick(1.0, 0.0, &t);
        assert!(fb.pulse(ViewOf::Every, &t).is_none() && fb.tick_of(0, &t).is_none() && fb.shakes(ViewOf::Every).count() == 0);
    }

    /// The pulse is whole blocks of the fire ramp's reds at an opacity in
    /// eighths, deeper for a harder blow, and gone at its end.
    #[test]
    fn the_pulse_is_ramp_steps_in_eighths() {
        let t = Tuning::default();
        let mut seen = std::collections::BTreeSet::new();
        for i in 0..40 {
            let age = t.hurt_pulse_seconds * i as f32 / 40.0;
            let p = pulse_look(1.0, age, &t).expect("alive before its end");
            let step = crate::pyro::FIRE.iter().position(|c| (c.r, c.g, c.b) == (p.color.r, p.color.g, p.color.b)).expect("a fire step");
            assert!((1..=3).contains(&step), "one of the ramp's reds");
            assert!((1..=8).any(|k| crate::pyro::alpha(crate::pyro::FIRE[step], k as f32 / 8.0).a == p.color.a), "opacity in eighths");
            assert!((0.5..=1.0).contains(&p.cover));
            seen.insert(step);
        }
        assert_eq!(seen.len(), 3, "it steps down through all three");
        assert!(pulse_look(1.0, t.hurt_pulse_seconds, &t).is_none());
        let (soft, hard) = (pulse_look(0.2, 0.0, &t).expect("soft"), pulse_look(1.0, 0.0, &t).expect("hard"));
        assert!(soft.depth < hard.depth && hard.depth == t.hurt_pulse_blocks);
        assert!(hard.whole_row(0) && hard.block(0, 3, 7) && !hard.block(hard.depth, 0, 0));
        assert!(!hard.whole_row(hard.depth - 1), "the inner rows are dithered");
    }

    /// A local seat's wreck holds the round for `kill_hitstop_seconds` of
    /// real time - where the caller allows it, on an undecided round, and
    /// no sooner than the gap after the last hold.
    #[test]
    fn a_local_seats_kill_holds_the_round_offline_only() {
        let t = Tuning::default();
        let mut fb = Feedback::default();
        fb.read(reading(1, [0.0, 0.0], 0), &[0], &t, false);
        fb.read(reading(2, [0.0, 0.0], 1), &[0], &t, false);
        assert_eq!(fb.time_scale(), 1.0, "an online round, or reduced motion: no hold");

        fb.read(reading(3, [0.0, 0.0], 2), &[0], &t, true);
        assert_eq!(fb.time_scale(), 0.0, "held");
        fb.tick(0.0, t.kill_hitstop_seconds * 0.5, &t);
        assert_eq!(fb.time_scale(), 0.0, "still held half way");
        fb.tick(0.0, t.kill_hitstop_seconds, &t);
        assert_eq!(fb.time_scale(), 1.0, "and let go on real time, the round's own standing still");

        fb.read(reading(4, [0.0, 0.0], 3), &[0], &t, true);
        assert_eq!(fb.time_scale(), 1.0, "a second kill inside the gap does not stutter");
        fb.tick(0.0, t.kill_hitstop_gap_seconds, &t);
        fb.read(reading(5, [0.0, 0.0], 4), &[0], &t, true);
        assert_eq!(fb.time_scale(), 0.0, "past the gap it holds again");
        fb.tick(0.0, 1.0, &t);

        fb.read(Reading { playing: false, ..reading(6, [0.0, 0.0], 5) }, &[0], &t, true);
        assert_eq!(fb.time_scale(), 1.0, "the blow that decided the round has its own hold");
        fb.read(reading(7, [0.0, 0.0], 5), &[1], &t, true);
        fb.read(reading(8, [0.0, 0.0], 6), &[1], &t, true);
        assert_eq!(fb.time_scale(), 1.0, "a kill by a seat this screen does not play");
    }

    /// `Reading::of` reads a real round: a seat's damage shows the frame
    /// after it lands, and a frame is read once.
    #[test]
    fn a_round_is_read_seat_by_seat() {
        let (w, h) = (crate::DEFAULT_SCREEN_WIDTH as f32, crate::DEFAULT_SCREEN_HEIGHT as f32);
        let t = Tuning::default();
        let mut game = Game::default();
        game.enemy_count_override = Some(1);
        game.seed_override = Some(11);
        game.map = crate::map::MapFile::from_toml_str(include_str!("../maps/default.toml")).expect("the default map parses");
        game.init(w, h);
        let mut fb = Feedback::default();
        game.update(crate::simulation::Input::default(), crate::PHYSICS_FIXED_DT, w, h);
        fb.observe_events(&game, &[0], &t, true);
        let seat = game.seat(0).expect("a seat");
        game.world.get::<&mut crate::tank::Tank>(seat).expect("the seat's tank").damage += 20.0;
        fb.observe_events(&game, &[0], &t, true);
        assert_eq!(fb.shakes(ViewOf::Every).count(), 0, "the same frame is read once");
        game.update(crate::simulation::Input::default(), crate::PHYSICS_FIXED_DT, w, h);
        fb.observe_events(&game, &[0], &t, true);
        assert_eq!(fb.shakes(ViewOf::Every).count(), 1);
        assert!(fb.tick_of(0, &t).is_some());
    }
}
