//! The metrics: one code path over `FrameSample`s, whether they came off
//! an online client's replica or off the local twin.
//!
//! Every metric reads what was *drawn* - the positions the picture held
//! on each frame, on the process clock - because that is what a player
//! sees. The online clients and the twin differ only in where the frames
//! came from, so the gap between the two rows of a report is the network's
//! cost and nothing else.

use serde::Serialize;

use bongbong::tank::Dir;
use bongbong::{TANK_HULL_BBOX_BY_ROW, TANK_TEXTURE_SIZE, TANK_TURRET_BBOX_BY_ROW};

use crate::sample::{EventSample, FrameSample};
use crate::script::{RECTANGLE, RECTANGLE_SECONDS, Scenario};

/// A distribution's summary. Every field is `None` with no samples.
#[derive(Clone, Copy, Debug, Default, Serialize, serde::Deserialize, PartialEq)]
pub struct Stat {
    pub n: usize,
    pub p50: Option<f64>,
    pub p95: Option<f64>,
    pub p99: Option<f64>,
    pub max: Option<f64>,
    pub mean: Option<f64>,
}

impl Stat {
    pub fn of(values: &[f64]) -> Stat {
        let mut v: Vec<f64> = values.iter().copied().filter(|x| x.is_finite()).collect();
        if v.is_empty() {
            return Stat::default();
        }
        v.sort_by(f64::total_cmp);
        let pick = |q: f64| v[((v.len() - 1) as f64 * q).round() as usize];
        Stat {
            n: v.len(),
            p50: Some(pick(0.50)),
            p95: Some(pick(0.95)),
            p99: Some(pick(0.99)),
            max: v.last().copied(),
            mean: Some(v.iter().sum::<f64>() / v.len() as f64),
        }
    }
}

/// How far the drawn own hull has to move along the new direction before
/// the change counts as answered: past the wire's quarter-pixel rounding
/// and any settling, well under anything a player can see.
pub const MOVE_ANSWERED_PX: f32 = 0.5;

/// How long after a change the answer is looked for.
pub const ANSWER_WINDOW_MS: f64 = 1000.0;

/// Frames this soon after a turn are not a constant-velocity stretch: the
/// hull is still swinging round and getting up to speed.
pub const TURN_SETTLE_S: f64 = 0.15;

/// A stretch slower than this is a hull standing against something, and
/// its pacing says nothing about the link.
pub const MIN_STRETCH_SPEED: f64 = 5.0;

/// How far back the remote-lag search looks on the host's own trajectory.
pub const LAG_WINDOW_MS: f64 = 1500.0;

/// How near the host's own drawn path the guest's drawn hull has to be to
/// count as a point on it.
pub const LAG_MATCH_PX: f32 = 1.5;

/// A backward step is past the wire's quarter-pixel rounding.
pub const BACKWARD_PX: f32 = 0.25;

/// How far a shot can move between two frames and still be the same shot.
pub const SHOT_TRACK_PX: f32 = 24.0;

/// How long after a press its `Fired` may arrive.
pub const FIRED_WINDOW_MS: f64 = 1000.0;

/// How long after its `Fired` a press may still be the one a hit answers.
pub const HIT_WINDOW_MS: f64 = 1500.0;

/// How near in ticks the guest's `Fired` has to be to the host's to be
/// the same shot.
pub const FIRED_TICK_MATCH: u64 = 6;

/// The own hull's input latency: from each frame the scripted direction
/// changes to the first frame the drawn own hull has moved
/// `MOVE_ANSWERED_PX` along the new direction, as milliseconds and as
/// frames.
pub fn own_input_latency(frames: &[FrameSample], seat: usize) -> Vec<(f64, usize)> {
    let mut out = Vec::new();
    for i in 1..frames.len() {
        let (prev, cur) = (&frames[i - 1], &frames[i]);
        let Some(dir) = cur.move_dir() else { continue };
        if prev.move_dir() == Some(dir) {
            continue;
        }
        let Some(base) = prev.tank(seat) else { continue };
        let v = dir.vec();
        for (k, later) in frames[i..].iter().enumerate() {
            if later.t_ms - cur.t_ms > ANSWER_WINDOW_MS || later.move_dir() != Some(dir) {
                break;
            }
            let Some(p) = later.tank(seat) else { break };
            if (p.x - base.x) * v.x + (p.y - base.y) * v.y > MOVE_ANSWERED_PX {
                out.push((later.t_ms - cur.t_ms, k));
                break;
            }
        }
    }
    out
}

/// A point on the host's own drawn trajectory.
#[derive(Clone, Copy, Debug)]
struct Point {
    t: f64,
    x: f32,
    y: f32,
}

fn trajectory(frames: &[FrameSample], slot: usize) -> Vec<Point> {
    frames.iter().filter_map(|f| f.tank(slot).map(|t| Point { t: f.t_ms, x: t.x, y: t.y })).collect()
}

/// When the host itself drew its hull at `(x, y)`, searching its own
/// trajectory back from `t` over `LAG_WINDOW_MS`: the nearest point on
/// any moving segment within `LAG_MATCH_PX`, the latest of equals.
fn when_drawn(path: &[Point], t: f64, x: f32, y: f32) -> Option<f64> {
    let lo = path.partition_point(|p| p.t < t - LAG_WINDOW_MS);
    let hi = path.partition_point(|p| p.t <= t + 50.0);
    let mut best: Option<(f32, f64)> = None;
    for k in lo.max(1)..hi {
        let (a, b) = (path[k - 1], path[k]);
        let (dx, dy) = (b.x - a.x, b.y - a.y);
        let len2 = dx * dx + dy * dy;
        if len2 < 1e-4 {
            continue;
        }
        let u = (((x - a.x) * dx + (y - a.y) * dy) / len2).clamp(0.0, 1.0);
        let (px, py) = (a.x + u * dx, a.y + u * dy);
        let d = ((x - px).powi(2) + (y - py).powi(2)).sqrt();
        if d > LAG_MATCH_PX {
            continue;
        }
        let at = a.t + u as f64 * (b.t - a.t);
        if best.is_none_or(|(bd, bt)| d < bd - 0.01 || (d <= bd + 0.01 && at > bt)) {
            best = Some((d, at));
        }
    }
    best.map(|(_, at)| at)
}

/// Remote motion lag: for each observer frame where the remote hull moved,
/// how long ago the host itself drew its hull there. Returns the lag per
/// observer frame index (None where there was no match).
pub fn remote_lag(observer: &[FrameSample], host: &[FrameSample], slot: usize) -> Vec<Option<f64>> {
    let path = trajectory(host, slot);
    let mut out = vec![None; observer.len()];
    for i in 1..observer.len() {
        let (Some(a), Some(b)) = (observer[i - 1].tank(slot), observer[i].tank(slot)) else { continue };
        if (b.x - a.x).abs() + (b.y - a.y).abs() < 0.25 {
            continue;
        }
        out[i] = when_drawn(&path, observer[i].t_ms, b.x, b.y).map(|at| observer[i].t_ms - at);
    }
    out
}

/// The pacing of a remote hull's drawn motion over its straight legs.
#[derive(Clone, Copy, Debug, Default, Serialize, serde::Deserialize, PartialEq)]
pub struct Pacing {
    /// Frames on a constant-velocity stretch.
    pub frames: usize,
    /// Moved under a quarter of the stretch's per-frame distance.
    pub stall_pct: f64,
    /// Moved over one and three quarters of it.
    pub jump_pct: f64,
    /// Moved against the direction of travel.
    pub backward_pct: f64,
    /// Coefficient of variation of the per-frame distance, relative to
    /// the stretch's.
    pub cv: f64,
}

/// The leg of a rectangle script `t` seconds in: its index in the lap
/// count, its direction, and how far into and before the end of it `t` is.
fn rectangle_leg_at(t: f64, phase: f64) -> (i64, Dir, f64, f64) {
    let lap_t = t + phase;
    let lap = (lap_t / RECTANGLE_SECONDS).floor() as i64;
    let mut into = lap_t - lap as f64 * RECTANGLE_SECONDS;
    for (i, (dir, seconds)) in RECTANGLE.iter().enumerate() {
        if into < *seconds {
            return (lap * 4 + i as i64, *dir, into, seconds - into);
        }
        into -= seconds;
    }
    (lap * 4 + 3, RECTANGLE[3].0, 0.0, 0.0)
}

/// The pacing of the host's hull as `observer` drew it. `host_script_s`
/// maps an observer frame to the host's script time at the moment the
/// host drew what the observer shows: the frame's own script time for the
/// twin, the matched host time for an online guest.
pub fn remote_pacing(observer: &[FrameSample], slot: usize, host_script_s: &[Option<f64>], phase: f64) -> Pacing {
    // One stretch per rectangle leg: its frames' (distance along, dt).
    let mut stretches: std::collections::BTreeMap<i64, Vec<(f64, f64)>> = std::collections::BTreeMap::new();
    for i in 1..observer.len() {
        let (Some(sa), Some(sb)) = (host_script_s[i - 1], host_script_s[i]) else { continue };
        let (leg_a, _, into_a, _) = rectangle_leg_at(sa, phase);
        let (leg_b, dir, _, left_b) = rectangle_leg_at(sb, phase);
        if leg_a != leg_b || into_a < TURN_SETTLE_S || left_b <= 0.0 {
            continue;
        }
        let (Some(a), Some(b)) = (observer[i - 1].tank(slot), observer[i].tank(slot)) else { continue };
        if a.wreck || b.wreck {
            continue;
        }
        let v = dir.vec();
        let d = ((b.x - a.x) * v.x + (b.y - a.y) * v.y) as f64;
        let dt = (observer[i].t_ms - observer[i - 1].t_ms) / 1000.0;
        if dt <= 0.0 {
            continue;
        }
        stretches.entry(leg_b).or_default().push((d, dt));
    }
    let (mut n, mut stall, mut jump, mut back) = (0usize, 0usize, 0usize, 0usize);
    let mut ratios = Vec::new();
    for steps in stretches.values() {
        let (sum_d, sum_t): (f64, f64) = steps.iter().fold((0.0, 0.0), |(d, t), s| (d + s.0, t + s.1));
        let speed = sum_d / sum_t;
        if !(speed >= MIN_STRETCH_SPEED) {
            continue;
        }
        for &(d, dt) in steps {
            let e = speed * dt;
            n += 1;
            stall += (d < 0.25 * e) as usize;
            jump += (d > 1.75 * e) as usize;
            back += (d < -(BACKWARD_PX as f64)) as usize;
            ratios.push(d / e);
        }
    }
    if n == 0 {
        return Pacing::default();
    }
    let mean = ratios.iter().sum::<f64>() / n as f64;
    let var = ratios.iter().map(|r| (r - mean).powi(2)).sum::<f64>() / n as f64;
    let pct = |k: usize| 100.0 * k as f64 / n as f64;
    Pacing { frames: n, stall_pct: pct(stall), jump_pct: pct(jump), backward_pct: pct(back), cv: if mean.abs() > 1e-9 { var.sqrt() / mean } else { 0.0 } }
}

/// The host's presses and what became of them.
#[derive(Clone, Copy, Debug, Default, Serialize, serde::Deserialize, PartialEq)]
pub struct ShotLedger {
    pub presses: usize,
    /// Press to the first frame the own shot is drawn.
    pub drawn_ms: Stat,
    /// Press to the frame the room's `Fired` for it is handed over.
    pub fired_ms: Stat,
    /// On the frame a provisional shot is retired, how far its last drawn
    /// position is from the room's copy of it drawn that frame (max per
    /// press).
    pub handoff_gap_px: Stat,
    /// Press to the first enemy hit handed over for it (only where the
    /// host is the one seat shooting).
    pub hit_ms: Stat,
    /// Press to the guest's `Fired` for it.
    pub guest_fired_ms: Stat,
    /// Presses the room never answered with a `Fired`.
    pub unanswered: usize,
}

/// A drawn own shot followed frame to frame.
#[derive(Clone, Copy, Debug)]
struct Track {
    kind: u8,
    provisional: bool,
    x: f32,
    y: f32,
    press: Option<usize>,
    seen: bool,
}

/// The host's shot ledger. `guest` is the other seat's frames (the twin's
/// own again), `hits` whether enemy hits can be put down to the host.
pub fn shot_ledger(host: &[FrameSample], guest: &[FrameSample], seat: usize, hits: bool) -> ShotLedger {
    let presses: Vec<f64> = host.iter().filter(|f| f.fire).map(|f| f.t_ms).collect();
    if presses.is_empty() {
        return ShotLedger::default();
    }
    let any_provisional = host.iter().any(|f| f.shots.iter().any(|s| s.provisional));
    let mut drawn: Vec<Option<f64>> = vec![None; presses.len()];
    let mut gap: Vec<Option<f64>> = vec![None; presses.len()];
    let mut tracks: Vec<Track> = Vec::new();
    let mut press_i = 0usize;
    for f in host {
        while press_i < presses.len() && presses[press_i] <= f.t_ms {
            press_i += 1;
        }
        for t in &mut tracks {
            t.seen = false;
        }
        let own: Vec<_> = f.shots.iter().filter(|s| s.provisional || s.owner == Some(seat)).collect();
        let mut fresh = Vec::new();
        for s in &own {
            let near = tracks
                .iter_mut()
                .filter(|t| !t.seen && t.kind == s.kind && t.provisional == s.provisional)
                .map(|t| {
                    let d = ((t.x - s.x).powi(2) + (t.y - s.y).powi(2)).sqrt();
                    (d, t)
                })
                .filter(|(d, _)| *d <= SHOT_TRACK_PX)
                .min_by(|a, b| a.0.total_cmp(&b.0));
            match near {
                Some((_, t)) => {
                    t.x = s.x;
                    t.y = s.y;
                    t.seen = true;
                }
                None => fresh.push(**s),
            }
        }
        // A provisional that is gone this frame was retired: how far is
        // the room's copy of it?
        for t in tracks.iter().filter(|t| !t.seen && t.provisional) {
            let Some(p) = t.press else { continue };
            let nearest = own
                .iter()
                .filter(|s| !s.provisional && s.kind == t.kind)
                .map(|s| (((s.x - t.x).powi(2) + (s.y - t.y).powi(2)).sqrt()) as f64)
                .min_by(f64::total_cmp);
            if let Some(d) = nearest {
                gap[p] = Some(gap[p].map_or(d, |g: f64| g.max(d)));
            }
        }
        tracks.retain(|t| t.seen);
        for s in fresh {
            let counts = s.provisional || !any_provisional;
            let press = if counts {
                // The newest press before this frame still without a shot:
                // a press that drew nothing (the round over for this seat,
                // a weapon with no projectile) is left unanswered rather
                // than handed the next press's shot.
                (0..press_i).rev().find(|&i| drawn[i].is_none() && f.t_ms - presses[i] <= FIRED_WINDOW_MS)
            } else {
                None
            };
            if let Some(i) = press {
                drawn[i] = Some(f.t_ms - presses[i]);
            }
            tracks.push(Track { kind: s.kind, provisional: s.provisional, x: s.x, y: s.y, press, seen: true });
        }
    }

    // The room's `Fired` for each press, oldest first, each used once.
    let fired_at: Vec<(f64, u64)> = events_of(host, |e| matches!(e, EventSample::Fired { slot } if *slot == seat));
    let mut fired: Vec<Option<(f64, u64)>> = vec![None; presses.len()];
    let mut next = 0usize;
    for (i, &p) in presses.iter().enumerate() {
        while next < fired_at.len() && fired_at[next].0 < p {
            next += 1;
        }
        if next < fired_at.len() && fired_at[next].0 - p <= FIRED_WINDOW_MS {
            fired[i] = Some(fired_at[next]);
            next += 1;
        }
    }

    let mut hit_ms = Vec::new();
    if hits {
        let hit_at: Vec<(f64, u64)> = events_of(host, |e| matches!(e, EventSample::HitEnemy { .. }));
        let mut answered = vec![false; presses.len()];
        for (t, _) in hit_at {
            let press = (0..presses.len()).find(|&i| {
                !answered[i] && fired[i].is_some_and(|(ft, _)| ft <= t && t - ft <= HIT_WINDOW_MS)
            });
            if let Some(i) = press {
                answered[i] = true;
                hit_ms.push(t - presses[i]);
            }
        }
    }

    let guest_fired: Vec<(f64, u64)> = events_of(guest, |e| matches!(e, EventSample::Fired { slot } if *slot == seat));
    let mut used = vec![false; guest_fired.len()];
    let mut guest_ms = Vec::new();
    for (i, f) in fired.iter().enumerate() {
        let Some((_, tick)) = f else { continue };
        if let Some(j) = (0..guest_fired.len()).find(|&j| !used[j] && guest_fired[j].1.abs_diff(*tick) <= FIRED_TICK_MATCH) {
            used[j] = true;
            guest_ms.push(guest_fired[j].0 - presses[i]);
        }
    }

    ShotLedger {
        presses: presses.len(),
        drawn_ms: Stat::of(&drawn.iter().flatten().copied().collect::<Vec<_>>()),
        fired_ms: Stat::of(&fired.iter().zip(&presses).filter_map(|(f, p)| f.map(|(t, _)| t - p)).collect::<Vec<_>>()),
        handoff_gap_px: Stat::of(&gap.iter().flatten().copied().collect::<Vec<_>>()),
        hit_ms: Stat::of(&hit_ms),
        guest_fired_ms: Stat::of(&guest_ms),
        unanswered: fired.iter().filter(|f| f.is_none()).count(),
    }
}

/// Every event `pick` accepts, with the frame it was handed over on and
/// that frame's tick.
fn events_of(frames: &[FrameSample], pick: impl Fn(&EventSample) -> bool) -> Vec<(f64, u64)> {
    frames.iter().flat_map(|f| f.events.iter().filter(|e| pick(e)).map(move |_| (f.t_ms, f.tick))).collect()
}

/// Hits on the host, and how far from the drawn hull the shot that hit was
/// drawn when it hit.
#[derive(Clone, Copy, Debug, Default, Serialize, serde::Deserialize, PartialEq)]
pub struct IncomingFire {
    pub hits: usize,
    /// The gap between the drawn hull and the enemy shot that hit it: the
    /// shot drawn nearest the impact on the frame before, carried on along
    /// its own drawn motion to the hit frame, measured to the hull's hit
    /// boxes as the hit test builds them (hull and turret, grown by the
    /// shell's own half extent). In a local round the shot is inside them
    /// on the frame it hits, so this is nought; a shot drawn in the past
    /// against a hull drawn in the present hits "from afar".
    pub from_afar_px: Stat,
    /// Hits with no enemy shot drawn at all on the frame before (a beam,
    /// a ram, a blast - or a shot the picture never showed).
    pub unseen: usize,
}

/// A hull's two hit boxes, as `Tank::hull_bbox_world` and
/// `Tank::turret_bbox_world` build them: (centre offset x, y, half x, y)
/// from the hull's centre, turned to its facing.
pub fn hit_boxes(row: i32, dir: u8) -> [(f32, f32, f32, f32); 2] {
    let row = (row.max(0) as usize).min(TANK_HULL_BBOX_BY_ROW.len() - 1);
    let dir = Dir::ALL.get(dir as usize).copied().unwrap_or(Dir::Up);
    let along_x = matches!(dir, Dir::Left | Dir::Right);
    let (w, h) = TANK_HULL_BBOX_BY_ROW[row];
    let (hw, hh) = (w * 0.5 * TANK_SCALE, h * 0.5 * TANK_SCALE);
    let hull = if along_x { (0.0, 0.0, hh, hw) } else { (0.0, 0.0, hw, hh) };
    let (x0, y0, x1, y1) = TANK_TURRET_BBOX_BY_ROW[row];
    let half = TANK_TEXTURE_SIZE * 0.5;
    let (cx, cy) = ((x0 + x1 + 1.0) * 0.5 - half, (y0 + y1 + 1.0) * 0.5 - half);
    let (tw, th) = ((x1 - x0 + 1.0) * 0.5, (y1 - y0 + 1.0) * 0.5);
    let (ox, oy, tw, th) = match dir {
        Dir::Up => (cx, cy, tw, th),
        Dir::Right => (-cy, cx, th, tw),
        Dir::Down => (-cx, -cy, tw, th),
        Dir::Left => (cy, -cx, th, tw),
    };
    [hull, (ox * TANK_SCALE, oy * TANK_SCALE, tw * TANK_SCALE, th * TANK_SCALE)]
}

/// The draw scale every tank has (`Tank::scale`).
pub const TANK_SCALE: f32 = 2.0;

pub fn incoming_fire(host: &[FrameSample], seat: usize, seats: usize) -> IncomingFire {
    let mut out = IncomingFire::default();
    let mut dist = Vec::new();
    let pad = bongbong::tuning::tuning().shell_hit_half_extent;
    for i in 1..host.len() {
        for e in &host[i].events {
            let EventSample::HitPlayer { player, x, y } = *e else { continue };
            if player as usize != seat {
                continue;
            }
            out.hits += 1;
            let Some(hull) = host[i].tank(seat) else { continue };
            let enemy = host[i - 1].shots.iter().filter(|s| s.owner.is_some_and(|o| o >= seats));
            let Some(shot) = enemy.min_by(|a, b| {
                let da = (a.x - x).powi(2) + (a.y - y).powi(2);
                let db = (b.x - x).powi(2) + (b.y - y).powi(2);
                da.total_cmp(&db)
            }) else {
                out.unseen += 1;
                continue;
            };
            // Where the shot stands on the hit frame, carried on at the
            // speed it was drawn moving between the two frames before.
            let (mut sx, mut sy) = (shot.x, shot.y);
            if i >= 2
                && let Some(before) = host[i - 2].shots.iter().find(|s| s.id == shot.id && s.kind == shot.kind)
            {
                let span = (host[i - 1].t_ms - host[i - 2].t_ms) as f32;
                if span > 0.0 {
                    let ahead = (host[i].t_ms - host[i - 1].t_ms) as f32 / span;
                    sx += (shot.x - before.x) * ahead;
                    sy += (shot.y - before.y) * ahead;
                }
            }
            let gap = hit_boxes(hull.row, hull.dir)
                .iter()
                .map(|&(ox, oy, hx, hy)| {
                    let dx = ((sx - hull.x - ox).abs() - hx - pad).max(0.0);
                    let dy = ((sy - hull.y - oy).abs() - hy - pad).max(0.0);
                    (dx * dx + dy * dy).sqrt()
                })
                .fold(f32::INFINITY, f32::min);
            dist.push(gap as f64);
        }
    }
    out.from_afar_px = Stat::of(&dist);
    out
}

/// Everything measured of one view of the round.
#[derive(Clone, Debug, Default, Serialize, serde::Deserialize, PartialEq)]
pub struct Metrics {
    pub frames: usize,
    pub own_input_ms: Stat,
    /// The same latencies in frames: what the verdict reads, since a frame
    /// that came late in real time is not a frame of latency.
    pub own_input_frames: Stat,
    pub remote_pacing: Option<Pacing>,
    pub remote_lag_ms: Stat,
    pub shots: ShotLedger,
    pub incoming: IncomingFire,
    pub frame_cpu_us: Stat,
    /// Seconds between frames as drawn: how steady the frame loop was.
    pub frame_interval_ms: Stat,
}

/// The inputs the metrics need beyond the frames.
pub struct View<'a> {
    pub host: &'a [FrameSample],
    pub guest: &'a [FrameSample],
    /// The host's script start on the process clock, to turn a matched
    /// host time into host script time.
    pub host_t0_ms: f64,
    pub host_seat: usize,
    pub guest_seat: usize,
    pub scenario: Scenario,
}

/// Compute every frame-based metric of a view. The twin is a view whose
/// host and guest are the same frames.
pub fn measure(view: &View) -> Metrics {
    let host_seat = view.host_seat;
    let mut own = own_input_latency(view.host, host_seat);
    // The guest's own hull answers its own script too (duel).
    if view.scenario == Scenario::Duel {
        own.extend(own_input_latency(view.guest, view.guest_seat));
    }
    // The host's hull is followed only where its script lays a path that
    // does not fold back on itself within the lag window: the
    // rectangles. The shooter's strafe steps retrace the same few pixels,
    // and a match there would be any of them.
    let rectangle = matches!(view.scenario, Scenario::Drive | Scenario::Duel);
    let lag = if rectangle { remote_lag(view.guest, view.host, host_seat) } else { vec![None; view.guest.len()] };
    let lags: Vec<f64> = lag.iter().flatten().copied().collect();
    let remote_pacing = match view.scenario {
        Scenario::Drive | Scenario::Duel => {
            let median = Stat::of(&lags).p50.unwrap_or(0.0);
            let host_script: Vec<Option<f64>> = view
                .guest
                .iter()
                .zip(&lag)
                .map(|(f, l)| {
                    let at = f.t_ms - l.unwrap_or(median);
                    let s = (at - view.host_t0_ms) / 1000.0;
                    (s >= 0.0).then_some(s)
                })
                .collect();
            Some(remote_pacing(view.guest, host_seat, &host_script, 0.0))
        }
        Scenario::Shoot => None,
    };
    let seats = 2;
    let intervals: Vec<f64> = view.host.windows(2).map(|w| w[1].t_ms - w[0].t_ms).collect();
    Metrics {
        frames: view.host.len(),
        own_input_ms: Stat::of(&own.iter().map(|o| o.0).collect::<Vec<_>>()),
        own_input_frames: Stat::of(&own.iter().map(|o| o.1 as f64).collect::<Vec<_>>()),
        remote_pacing,
        remote_lag_ms: Stat::of(&lags),
        shots: shot_ledger(view.host, view.guest, host_seat, view.scenario == Scenario::Shoot),
        incoming: incoming_fire(view.host, host_seat, seats),
        frame_cpu_us: Stat::of(&view.host.iter().map(|f| f.cpu_us).collect::<Vec<_>>()),
        frame_interval_ms: Stat::of(&intervals),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::{ShotSample, TankSample, dir_index};

    fn frame(t_ms: f64, dir: Option<Dir>, x: f32, y: f32) -> FrameSample {
        FrameSample {
            t_ms,
            move_dir: dir.map(dir_index),
            tanks: vec![TankSample { slot: 0, x, y, row: 0, dir: 3, wreck: false, player: true }],
            ..FrameSample::default()
        }
    }

    #[test]
    fn stats_pick_percentiles_and_skip_nothing_finite() {
        let s = Stat::of(&(1..=100).map(|v| v as f64).collect::<Vec<_>>());
        assert_eq!(s.n, 100);
        assert_eq!(s.p50, Some(51.0));
        assert_eq!(s.max, Some(100.0));
        assert_eq!(Stat::of(&[]), Stat::default());
    }

    #[test]
    fn own_input_latency_counts_frames_until_the_hull_moves_the_new_way() {
        // Standing still, then told to go right at frame 1; the hull moves
        // on frame 3.
        let frames = vec![
            frame(0.0, None, 100.0, 100.0),
            frame(16.0, Some(Dir::Right), 100.0, 100.0),
            frame(32.0, Some(Dir::Right), 100.2, 100.0),
            frame(48.0, Some(Dir::Right), 102.0, 100.0),
        ];
        assert_eq!(own_input_latency(&frames, 0), vec![(32.0, 2)]);
    }

    #[test]
    fn a_remote_hull_drawn_where_the_host_was_a_while_ago_has_that_lag() {
        // The host moves right 2 px a frame; the guest draws it 5 frames
        // (80 ms) behind.
        let host: Vec<_> = (0..40).map(|i| frame(i as f64 * 16.0, Some(Dir::Right), 100.0 + 2.0 * i as f32, 50.0)).collect();
        let guest: Vec<_> = (0..40)
            .map(|i| frame(i as f64 * 16.0, None, 100.0 + 2.0 * (i as f32 - 5.0).max(0.0), 50.0))
            .collect();
        let lag = remote_lag(&guest, &host, 0);
        let lags: Vec<f64> = lag.iter().flatten().copied().collect();
        assert!(!lags.is_empty());
        assert!(lags.iter().all(|l| (l - 80.0).abs() < 0.5), "{lags:?}");
    }

    #[test]
    fn pacing_flags_stalls_and_jumps() {
        // Straight right at 2 px a frame, with one frame frozen and the
        // next catching up.
        let mut xs: Vec<f32> = (0..60).map(|i| 2.0 * i as f32).collect();
        xs[30] = xs[29];
        let frames: Vec<_> = xs.iter().enumerate().map(|(i, &x)| frame(i as f64 * 16.0, None, x, 0.0)).collect();
        // Host script time: 0.2 s in, all on the first (right) leg.
        let script: Vec<Option<f64>> = (0..60).map(|i| Some(0.2 + i as f64 * 0.016)).collect();
        let p = remote_pacing(&frames, 0, &script, 0.0);
        assert!(p.frames > 50);
        assert!(p.stall_pct > 0.0 && p.jump_pct > 0.0);
        assert_eq!(p.backward_pct, 0.0);
        let smooth: Vec<_> = (0..60).map(|i| frame(i as f64 * 16.0, None, 2.0 * i as f32, 0.0)).collect();
        let q = remote_pacing(&smooth, 0, &script, 0.0);
        assert_eq!((q.stall_pct, q.jump_pct), (0.0, 0.0));
        assert!(q.cv < 1e-6);
    }

    #[test]
    fn the_ledger_times_a_press_to_its_shot_and_its_fired() {
        let shot = |id, x, provisional| ShotSample { id, kind: 0, x, y: 0.0, owner: Some(0), provisional, flying: true };
        let mut frames: Vec<FrameSample> = (0..10).map(|i| frame(i as f64 * 16.0, None, 0.0, 0.0)).collect();
        frames[2].fire = true;
        frames[3].shots = vec![shot(0xF000, 10.0, true)];
        frames[4].shots = vec![shot(0xF000, 18.0, true)];
        frames[5].shots = vec![shot(3, 21.0, false)];
        frames[5].events = vec![EventSample::Fired { slot: 0 }];
        frames[5].tick = 40;
        let mut guest = frames.clone();
        guest[5].events.clear();
        guest[7].events = vec![EventSample::Fired { slot: 0 }];
        guest[7].tick = 40;
        let l = shot_ledger(&frames, &guest, 0, false);
        assert_eq!(l.presses, 1);
        assert_eq!(l.drawn_ms.p50, Some(16.0));
        assert_eq!(l.fired_ms.p50, Some(48.0));
        assert_eq!(l.handoff_gap_px.p50, Some(3.0));
        assert_eq!(l.guest_fired_ms.p50, Some(80.0));
        assert_eq!(l.unanswered, 0);
    }
}
