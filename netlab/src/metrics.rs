//! The metrics: one code path over `FrameSample`s, whether they came off
//! an online client's replica or off the local twin.
//!
//! Every metric reads what was *drawn* - the positions the picture held
//! on each frame, on the process clock - because that is what a player
//! sees. The online clients and the twin differ only in where the frames
//! came from, so the gap between the two rows of a report is the network's
//! cost and nothing else.

use serde::Serialize;

use bongbong::simulation::present::segment_box;
use bongbong::tank::Dir;
use bongbong::{TANK_HULL_BBOX_BY_ROW, TANK_TEXTURE_SIZE, TANK_TURRET_BBOX_BY_ROW};

use crate::sample::{EventSample, FrameSample, TankSample};
use crate::script::{RECTANGLE, RECTANGLE_SECONDS, Scenario};
use crate::shots;

/// A distribution's summary. Every field is `None` with no samples.
#[derive(Clone, Copy, Debug, Default, Serialize, serde::Deserialize, PartialEq)]
pub struct Stat {
    pub n: usize,
    pub min: Option<f64>,
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
            min: v.first().copied(),
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

/// How long after a press its `Fired` may arrive.
pub const FIRED_WINDOW_MS: f64 = 1000.0;

/// How long after its `Fired` a press may still be the one a hit answers.
pub const HIT_WINDOW_MS: f64 = 1500.0;

/// How near in ticks the guest's `Fired` has to be to the host's to be
/// the same shot, where the two are paired by order.
pub const FIRED_TICK_MATCH: u64 = 6;

/// How near the drawn own hull a shot first drawn counts as leaving its
/// muzzle: a hull's length.
pub const LAUNCH_RADIUS_PX: f32 = 64.0;

/// How long after an own shot leaves the picture another own shot
/// appearing away from the muzzle is taken as the same shot drawn again:
/// past the client's hold on an unpaired room copy
/// (`net::predict::UNPAIRED_HOLD_SECONDS`) by a few frames.
pub const HANDOFF_WINDOW_MS: f64 = 250.0;

/// The most a shot that left the picture in flight is carried on to where
/// it would have been when the next one appeared: a few frames, not a
/// guess at a flight nobody saw.
pub const HANDOFF_CARRY_MS: f64 = 50.0;

/// How long before a hit is handed over the picture may have stopped the
/// shot that made it: a round trip and the picture's delay, generously.
pub const STRIKE_WINDOW_MS: f64 = 1000.0;

/// How near the room's impact point the drawn path of the shot that made
/// it passes: shots fly straight, so the point is on the path the picture
/// drew, give or take the wire's rounding and a hit box.
pub const PATH_MATCH_PX: f32 = 12.0;

/// How far past a drawn path's end the room's impact point may lie and
/// still be on it: a shot the picture stopped short, at a wall or at the
/// hull's near face.
pub const PATH_EXTEND_PX: f32 = 64.0;

/// A shot the picture stopped this near the drawn hull's hit boxes struck
/// it.
pub const STRIKE_PX: f32 = 1.0;

/// How far past a frame's flight a shot taken off the picture may have
/// met the drawn hull: a client drawing incoming fire in the present
/// carries a shot further than a frame's flight while it catches up to its
/// lead (`net::round::CATCH_UP_MS`).
pub const STRIKE_REACH_PX: f32 = 48.0;

/// The own hull's input latency: from each frame `seat`'s scripted
/// direction changes to the first frame the drawn hull of `seat` has moved
/// `MOVE_ANSWERED_PX` along the new direction, as milliseconds and as
/// frames.
pub fn own_input_latency(frames: &[FrameSample], seat: usize) -> Vec<(f64, usize)> {
    let mut out = Vec::new();
    for i in 1..frames.len() {
        let (prev, cur) = (&frames[i - 1], &frames[i]);
        let Some(dir) = cur.move_dir(seat) else { continue };
        if prev.move_dir(seat) == Some(dir) {
            continue;
        }
        let Some(base) = prev.tank(seat) else { continue };
        let v = dir.vec();
        for (k, later) in frames[i..].iter().enumerate() {
            if later.t_ms - cur.t_ms > ANSWER_WINDOW_MS || later.move_dir(seat) != Some(dir) {
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
#[derive(Clone, Debug, Default, Serialize, serde::Deserialize, PartialEq)]
pub struct ShotLedger {
    pub presses: usize,
    /// Press to the first frame its shot is drawn leaving the muzzle: the
    /// provisional where the client draws one, the room's copy where it
    /// does not.
    pub drawn_ms: Stat,
    /// Press to the frame the room's `Fired` for it is handed over.
    pub fired_ms: Stat,
    /// How each press was paired with its `Fired`: `input_tick` - the
    /// press's own intent tick, which the room's `Fired` names, both read
    /// off the wire by the tap - or `order`, oldest first, where there is
    /// no tap (`--remote`, the twin).
    pub fired_pairing: String,
    /// Every time an own shot left the picture and an own shot appeared
    /// away from the muzzle within `HANDOFF_WINDOW_MS`: how far the shot
    /// jumped between the two - a provisional swapped for the room's copy,
    /// a room copy shown again after the provisional that stood for it
    /// was gone, a provisional snapped to the room's impact. Nothing is
    /// swapped on the present timeline, so its count is the finding there.
    pub handoff_gap_px: Stat,
    /// Room copies of the host's shots that appeared while the provisional
    /// standing for the same shot was still drawn - on its path, behind it:
    /// one shot drawn twice, the second a round trip late.
    pub drawn_twice: usize,
    /// Distinct room copies of the host's own shots the picture drew, on a
    /// client that draws its own shots from the press (`None` where it
    /// draws none): each one is a shot drawn twice, or drawn only a round
    /// trip late.
    pub room_copies_shown: Option<usize>,
    /// Press to the first enemy hit handed over for it (only where the
    /// host is the one seat shooting).
    pub hit_ms: Stat,
    /// Press to the guest's `Fired` for it.
    pub guest_fired_ms: Stat,
    /// Presses the room never answered with a `Fired`.
    pub unanswered: usize,
}

/// The ticks of the snapshots that carried each host press's `Fired`, read
/// off the wire by the tap: per press in press order, to the host and to
/// the guest. What pairs a press with its `Fired` exactly
/// (`ShotLedger::fired_pairing`).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct WireFired {
    pub host: Vec<Option<u32>>,
    pub guest: Vec<Option<u32>>,
}

/// When each press of `seat` was made: the frames whose script tapped.
pub fn presses(frames: &[FrameSample], seat: usize) -> Vec<f64> {
    frames.iter().filter(|f| f.fire(seat)).map(|f| f.t_ms).collect()
}

/// The first frame at or after `not_before` that draws tick `tick` or
/// later - the frame the interpolator handed that snapshot's events over.
fn frame_reaching(frames: &[FrameSample], tick: u32, not_before: f64) -> Option<(f64, u64)> {
    frames.iter().find(|f| f.t_ms >= not_before && f.tick >= tick as u64).map(|f| (f.t_ms, f.tick))
}

/// The gap between a point and a drawn hull's hit boxes, grown by the
/// shot's half extent: nought on or inside them.
fn gap_to_hull(hull: &TankSample, x: f32, y: f32, half: f32) -> f32 {
    hit_boxes(hull.row, hull.dir)
        .iter()
        .map(|&(ox, oy, hx, hy)| {
            let dx = ((x - hull.x - ox).abs() - hx - half).max(0.0);
            let dy = ((y - hull.y - oy).abs() - hy - half).max(0.0);
            (dx * dx + dy * dy).sqrt()
        })
        .fold(f32::INFINITY, f32::min)
}

/// Whether the room copy `room` first appeared beside a provisional drawn
/// on the same frame whose path it lies on, behind it or level with it: the
/// same shot, drawn twice. A room copy of an older shot lies ahead of the
/// newer provisionals on the same line, so it is not taken for one of
/// them.
fn beside_its_provisional(own: &[shots::Track], room: &shots::Track) -> bool {
    let first = room.first();
    own.iter().filter(|p| p.provisional).any(|p| {
        let Some(now) = p.points.iter().find(|q| q.frame == first.frame) else { return false };
        if p.path_distance(first.x, first.y, 0.0) > PATH_MATCH_PX {
            return false;
        }
        let (dx, dy) = (p.last().x - p.first().x, p.last().y - p.first().y);
        let len = (dx * dx + dy * dy).sqrt();
        len < 1e-3 || ((now.x - first.x) * dx + (now.y - first.y) * dy) / len >= -PATH_MATCH_PX
    })
}

/// The host's shot ledger. `guest` is the other seat's frames (the twin's
/// own again), `hits` whether enemy hits can be put down to the host,
/// `wire` the tap's reading of which snapshot carried each press's
/// `Fired`.
pub fn shot_ledger(host: &[FrameSample], guest: &[FrameSample], seat: usize, hits: bool, wire: Option<&WireFired>) -> ShotLedger {
    let presses = presses(host, seat);
    if presses.is_empty() {
        return ShotLedger::default();
    }
    let own = shots::follow(host, |s| s.seat == Some(seat as u8));
    // A client that draws its own shots from the press: its provisionals
    // are the launches, and a room copy of one is a shot drawn twice.
    let predicted = own.iter().any(|t| t.provisional);

    // Launches: a track that begins at the muzzle, answering the newest
    // press at or before it if that press has no shot yet. A press that
    // drew nothing (refused, the round over for this seat) is left
    // unanswered rather than handed the next press's shot.
    let mut drawn: Vec<Option<f64>> = vec![None; presses.len()];
    let mut launched = vec![false; own.len()];
    for (k, t) in own.iter().enumerate() {
        if t.provisional != predicted {
            continue;
        }
        let first = t.first();
        let f = &host[first.frame];
        let Some(hull) = f.tank(seat) else { continue };
        if ((first.x - hull.x).powi(2) + (first.y - hull.y).powi(2)).sqrt() > LAUNCH_RADIUS_PX {
            continue;
        }
        let Some(i) = presses.iter().rposition(|&p| p <= f.t_ms) else { continue };
        if drawn[i].is_none() && f.t_ms - presses[i] <= FIRED_WINDOW_MS {
            drawn[i] = Some(f.t_ms - presses[i]);
            launched[k] = true;
        }
    }

    // Every other appearance of an own shot: a room copy drawn beside the
    // provisional that stands for the same shot is that shot drawn twice;
    // anything else is a hand-off from the own shot of its kind that left
    // the picture most recently before it.
    let mut handed = vec![false; own.len()];
    let mut gaps = Vec::new();
    let mut drawn_twice = 0;
    for (k, t) in own.iter().enumerate() {
        if launched[k] {
            continue;
        }
        if predicted && !t.provisional && beside_its_provisional(&own, t) {
            drawn_twice += 1;
            continue;
        }
        let first = t.first();
        let at = host[first.frame].t_ms;
        let before = own
            .iter()
            .enumerate()
            .filter(|&(j, e)| {
                j != k && !handed[j] && e.kind == t.kind && e.last().frame < first.frame && at - host[e.last().frame].t_ms <= HANDOFF_WINDOW_MS
            })
            .map(|(j, e)| {
                let (x, y) = e.carried(host, at, HANDOFF_CARRY_MS);
                (j, e.last().frame, ((x - first.x).powi(2) + (y - first.y).powi(2)).sqrt())
            })
            .max_by(|a, b| a.1.cmp(&b.1).then(b.2.total_cmp(&a.2)));
        if let Some((j, _, gap)) = before {
            handed[j] = true;
            gaps.push(gap as f64);
        }
    }
    let room_copies_shown = predicted.then(|| {
        own.iter().filter_map(|t| t.room_id).collect::<std::collections::BTreeSet<_>>().len()
    });

    // The room's `Fired` for each press: exactly, by the snapshot the tap
    // saw carry it, or oldest first within a window.
    let (fired, guest_ms, pairing): (Vec<Option<(f64, u64)>>, Vec<f64>, &str) = match wire {
        Some(w) => {
            let fired: Vec<Option<(f64, u64)>> = presses
                .iter()
                .enumerate()
                .map(|(i, &p)| w.host.get(i).copied().flatten().and_then(|tick| frame_reaching(host, tick, p)))
                .collect();
            let guest_ms = presses
                .iter()
                .enumerate()
                .filter_map(|(i, &p)| w.guest.get(i).copied().flatten().and_then(|tick| frame_reaching(guest, tick, p)).map(|(t, _)| t - p))
                .collect();
            (fired, guest_ms, "input_tick")
        }
        None => {
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
            (fired, guest_ms, "order")
        }
    };

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

    ShotLedger {
        presses: presses.len(),
        drawn_ms: Stat::of(&drawn.iter().flatten().copied().collect::<Vec<_>>()),
        fired_ms: Stat::of(&fired.iter().zip(&presses).filter_map(|(f, p)| f.map(|(t, _)| t - p)).collect::<Vec<_>>()),
        fired_pairing: pairing.to_string(),
        handoff_gap_px: Stat::of(&gaps),
        drawn_twice,
        room_copies_shown,
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

/// Hits on a seat, and where the picture had the shot that made each.
///
/// Read off the picture's own shots: every shot not the seat's, followed
/// by the room's id, and where the picture stopped it - its impact frames,
/// or, taken off in flight, the point its path met the drawn hull (a
/// client drawing incoming fire in the present takes a shot that meets its
/// drawn hull off the picture there and then, `net::round`; a shot that
/// vanished anywhere else was cleared by the room and is no strike). A hit
/// is put down to a shot the picture stopped in the `STRIKE_WINDOW_MS`
/// before the hit was handed over: struck on a path through the room's
/// impact point, or burst at that point; of several, the one stopped
/// nearest the hull.
#[derive(Clone, Copy, Debug, Default, Serialize, serde::Deserialize, PartialEq)]
pub struct IncomingFire {
    pub hits: usize,
    /// Where the picture stopped the shot that hit, from the drawn hull's
    /// hit boxes as the hit test builds them (hull and turret, grown by
    /// the shot's own half extent), on the frame it stopped. Locally the
    /// shot stops inside them on the frame it hits, so this is nought; a
    /// shot the picture drew passing the hull, or bursting where the hull
    /// was a moment ago, hits "from afar".
    pub from_afar_px: Stat,
    /// From the frame the picture stopped that shot to the frame the
    /// room's hit was handed over: nought locally; online, how long a
    /// strike stands on screen before its damage lands.
    pub strike_to_hit_ms: Stat,
    /// Hits no drawn shot accounts for: a beam, a ram, a blast - or a shot
    /// the picture never showed striking or bursting.
    pub unseen: usize,
    /// Shots the picture stopped against the seat's drawn hull that no
    /// hit ever followed: a strike drawn for a shot the room judged a
    /// miss.
    pub phantom_strikes: usize,
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

/// Where the picture stopped a shot that was taken off in flight, if that
/// was a strike on `seat`'s drawn hull: the point its line of flight
/// enters the hull's hit boxes (grown by its half extent), at most a
/// frame's flight and `STRIKE_REACH_PX` past where it was last drawn - a
/// client drawing incoming fire in the present takes a shot off the
/// picture where its path this frame meets the hull, and draws the impact
/// there. `None` for a shot that vanished anywhere else.
fn strike_point(track: &shots::Track, stop: &shots::Stop, frames: &[FrameSample], seat: usize) -> Option<(f32, f32)> {
    use bongbong::Position;
    let hull = frames[stop.frame].tank(seat)?;
    let (dx, dy) = track.heading()?;
    let last = track.last();
    let dt = (frames[stop.frame].t_ms - frames[last.frame].t_ms).max(0.0) as f32 / 1000.0;
    let reach = shots::speed(track.kind) * dt * shots::FOLLOW_FLIGHT + STRIKE_REACH_PX;
    let (p0, p1) = (Position::new(last.x, last.y), Position::new(last.x + dx * reach, last.y + dy * reach));
    let half = shots::half_extent(track.kind);
    let t = hit_boxes(hull.row, hull.dir)
        .iter()
        .filter_map(|&(ox, oy, hx, hy)| {
            segment_box(p0, p1, Position::new(hull.x + ox, hull.y + oy), Position::new(hx + half, hy + half))
        })
        .min_by(f32::total_cmp)?;
    Some((p0.x + (p1.x - p0.x) * t, p0.y + (p1.y - p0.y) * t))
}

/// Hits on `seat` over `frames`, and where the picture had the shots that
/// made them (`IncomingFire`).
pub fn incoming_fire(frames: &[FrameSample], seat: usize) -> IncomingFire {
    let mut out = IncomingFire::default();
    let tracks = shots::follow(frames, |s| s.seat != Some(seat as u8) && !s.provisional);
    // Where the picture stopped each shot against this seat: a burst
    // wherever it burst, a shot taken off in flight only where it struck
    // the drawn hull - one that vanished anywhere else was the room
    // clearing a shot that hit nothing here, and the picture never showed
    // it hitting.
    let stops: Vec<Option<shots::Stop>> = tracks
        .iter()
        .map(|t| {
            let mut stop = t.stop(frames)?;
            if stop.vanished {
                (stop.x, stop.y) = strike_point(t, &stop, frames, seat)?;
            }
            Some(stop)
        })
        .collect();
    let stop_gap = |k: usize| -> Option<f32> {
        let stop = stops[k]?;
        let hull = frames[stop.frame].tank(seat)?;
        Some(gap_to_hull(hull, stop.x, stop.y, shots::half_extent(tracks[k].kind)))
    };
    let mut claimed = vec![false; tracks.len()];
    let (mut afar, mut lead) = (Vec::new(), Vec::new());
    for (i, f) in frames.iter().enumerate() {
        for e in &f.events {
            let EventSample::HitPlayer { player, x, y } = *e else { continue };
            if player as usize != seat {
                continue;
            }
            out.hits += 1;
            // The shot that made it: one the picture took off in flight
            // on a path through the room's impact point, or one it drew
            // bursting there - a shot that burst anywhere else was the
            // room's word on another target.
            let best = (0..tracks.len())
                .filter(|&k| !claimed[k])
                .filter_map(|k| {
                    let stop = stops[k]?;
                    let fresh = stop.frame <= i && f.t_ms - frames[stop.frame].t_ms <= STRIKE_WINDOW_MS;
                    let made_it = if stop.vanished {
                        tracks[k].path_distance(x, y, PATH_EXTEND_PX) <= PATH_MATCH_PX
                    } else {
                        ((stop.x - x).powi(2) + (stop.y - y).powi(2)).sqrt() <= PATH_MATCH_PX
                    };
                    (fresh && made_it).then(|| (k, stop.frame, stop_gap(k).unwrap_or(f32::INFINITY)))
                })
                .min_by(|a, b| a.2.total_cmp(&b.2).then(b.1.cmp(&a.1)));
            match best {
                Some((k, e, gap)) => {
                    claimed[k] = true;
                    afar.push(gap as f64);
                    lead.push(f.t_ms - frames[e].t_ms);
                }
                None => out.unseen += 1,
            }
        }
    }
    // A strike the picture drew with no hit after it, where the frames ran
    // on long enough for one to have been handed over.
    let end = frames.last().map_or(0.0, |f| f.t_ms);
    out.phantom_strikes = (0..tracks.len())
        .filter(|&k| !claimed[k])
        .filter(|&k| stops[k].is_some_and(|s| end - frames[s.frame].t_ms > STRIKE_WINDOW_MS))
        .filter(|&k| stop_gap(k).is_some_and(|g| g <= STRIKE_PX))
        .count();
    out.from_afar_px = Stat::of(&afar);
    out.strike_to_hit_ms = Stat::of(&lead);
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
    /// Which snapshot carried each host press's `Fired`, where the tap
    /// read the wire.
    pub wire: Option<&'a WireFired>,
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
    let intervals: Vec<f64> = view.host.windows(2).map(|w| w[1].t_ms - w[0].t_ms).collect();
    Metrics {
        frames: view.host.len(),
        own_input_ms: Stat::of(&own.iter().map(|o| o.0).collect::<Vec<_>>()),
        own_input_frames: Stat::of(&own.iter().map(|o| o.1 as f64).collect::<Vec<_>>()),
        remote_pacing,
        remote_lag_ms: Stat::of(&lags),
        shots: shot_ledger(view.host, view.guest, host_seat, view.scenario == Scenario::Shoot, view.wire),
        incoming: incoming_fire(view.host, host_seat),
        frame_cpu_us: Stat::of(&view.host.iter().map(|f| f.cpu_us).collect::<Vec<_>>()),
        frame_interval_ms: Stat::of(&intervals),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sample::{ShotSample, TankSample, dir_index};
    use bongbong::net::predict::PROVISIONAL_ID_BASE;

    fn frame(t_ms: f64, dir: Option<Dir>, x: f32, y: f32) -> FrameSample {
        FrameSample {
            t_ms,
            move_dirs: [dir.map(dir_index), None],
            tanks: vec![TankSample { slot: 0, x, y, row: 0, dir: 3, wreck: false, player: true }],
            ..FrameSample::default()
        }
    }

    /// Frames 16 ms apart with seat 0's hull at the origin.
    fn still(n: usize) -> Vec<FrameSample> {
        (0..n).map(|i| frame(i as f64 * 16.0, None, 0.0, 0.0)).collect()
    }

    fn own(id: u32, x: f32, provisional: bool, flying: bool) -> ShotSample {
        ShotSample { id, kind: 0, x, y: 0.0, seat: Some(0), provisional, flying, impact: !flying }
    }

    fn enemy(id: u32, x: f32, flying: bool) -> ShotSample {
        ShotSample { id, kind: 0, x, y: 0.0, seat: None, provisional: false, flying, impact: !flying }
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

    /// Each seat's latency runs from its own script's change: seat 1 is
    /// told to go down on frame 4 and its hull moves on frame 6, while
    /// seat 0's script turns on frame 1 - which seat 1's hull is not
    /// answering.
    #[test]
    fn own_input_latency_reads_the_seats_own_direction() {
        let frames: Vec<FrameSample> = (0..8)
            .map(|i| {
                let mut f = frame(i as f64 * 16.0, (i >= 1).then_some(Dir::Right), 0.0, 0.0);
                f.move_dirs[1] = (i >= 4).then_some(dir_index(Dir::Down));
                let y = if i >= 6 { 50.0 + (i - 5) as f32 } else { 50.0 };
                f.tanks.push(TankSample { slot: 1, x: 300.0, y, row: 0, dir: 1, wreck: false, player: true });
                f
            })
            .collect();
        assert_eq!(own_input_latency(&frames, 1), vec![(32.0, 2)], "from seat 1's own turn on frame 4");
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

    /// A provisional swapped for the room's copy: the jump is measured
    /// from where the provisional would have been on the swap frame.
    #[test]
    fn a_swap_is_a_handoff_with_its_gap() {
        let mut frames = still(10);
        frames[2].fires[0] = true;
        frames[3].shots = vec![own(PROVISIONAL_ID_BASE, 10.0, true, true)];
        frames[4].shots = vec![own(PROVISIONAL_ID_BASE, 18.0, true, true)];
        frames[5].shots = vec![own(3, 21.0, false, true)];
        frames[5].events = vec![EventSample::Fired { slot: 0 }];
        frames[5].tick = 40;
        let mut guest = frames.clone();
        guest[5].events.clear();
        guest[7].events = vec![EventSample::Fired { slot: 0 }];
        guest[7].tick = 40;
        let l = shot_ledger(&frames, &guest, 0, false, None);
        assert_eq!(l.presses, 1);
        assert_eq!(l.drawn_ms.p50, Some(16.0));
        assert_eq!(l.fired_ms.p50, Some(48.0));
        assert_eq!(l.fired_pairing, "order");
        assert_eq!(l.handoff_gap_px.n, 1);
        assert_eq!(l.handoff_gap_px.p50, Some(5.0), "the provisional was carried to 26, the copy drawn at 21");
        assert_eq!(l.room_copies_shown, Some(1));
        assert_eq!(l.guest_fired_ms.p50, Some(80.0));
        assert_eq!(l.unanswered, 0);
    }

    /// The present timeline: the provisional is the shot for its whole
    /// life, its ids shifting as an older one retires, and the room's copy
    /// never shows - no hand-off at all.
    #[test]
    fn a_shot_drawn_once_has_no_handoff() {
        let mut frames = still(30);
        frames[2].fires[0] = true;
        frames[4].fires[0] = true;
        for i in 3..20 {
            let mut shots = Vec::new();
            if i < 12 {
                shots.push(own(PROVISIONAL_ID_BASE, 10.0 + 8.0 * (i - 3) as f32, true, true));
            }
            if i >= 5 {
                // The second shell: under the first's id once that retires.
                let id = if i < 12 { PROVISIONAL_ID_BASE + 1 } else { PROVISIONAL_ID_BASE };
                shots.push(own(id, 12.0 + 8.0 * (i - 5) as f32, true, true));
            }
            frames[i].shots = shots;
        }
        let l = shot_ledger(&frames, &frames, 0, false, None);
        assert_eq!(l.presses, 2);
        assert_eq!(l.drawn_ms.n, 2, "both presses drew their shot");
        assert_eq!(l.handoff_gap_px.n, 0, "{:?}", l.handoff_gap_px);
        assert_eq!(l.room_copies_shown, Some(0));
    }

    /// The provisional plays its impact and goes; the room's copy, still
    /// flying a round trip behind, is shown after the hold: a shot drawn
    /// twice, and the jump back to it.
    #[test]
    fn a_room_copy_shown_after_its_provisional_is_a_handoff() {
        let mut frames = still(20);
        frames[1].fires[0] = true;
        for i in 2..8 {
            frames[i].shots = vec![own(PROVISIONAL_ID_BASE, 20.0 + 8.0 * (i - 2) as f32, true, true)];
        }
        for i in 8..11 {
            frames[i].shots = vec![own(PROVISIONAL_ID_BASE, 60.0, true, false)];
        }
        for i in 14..17 {
            frames[i].shots = vec![own(9, 30.0 + 8.0 * (i - 14) as f32, false, true)];
        }
        let l = shot_ledger(&frames, &frames, 0, false, None);
        assert_eq!(l.handoff_gap_px.n, 1);
        assert_eq!(l.handoff_gap_px.p50, Some(30.0), "from its impact at 60 back to 30");
        assert_eq!(l.room_copies_shown, Some(1));
    }

    /// The room's copy shown at the muzzle while its provisional is still
    /// flying ahead of it on the same line: one shot drawn twice - not a
    /// hand-off. A room copy of an older shot, ahead of the newer
    /// provisional on that line, is not mistaken for its twin.
    #[test]
    fn a_room_copy_beside_its_provisional_is_a_shot_drawn_twice() {
        let mut frames = still(20);
        frames[1].fires[0] = true;
        for i in 2..16 {
            let mut shots = vec![own(PROVISIONAL_ID_BASE, 20.0 + 8.0 * (i - 2) as f32, true, i < 14)];
            if i >= 8 {
                shots.push(own(12, 20.0 + 8.0 * (i - 8) as f32, false, true));
            }
            frames[i].shots = shots;
        }
        let l = shot_ledger(&frames, &frames, 0, false, None);
        assert_eq!((l.drawn_twice, l.handoff_gap_px.n), (1, 0));
        assert_eq!(l.room_copies_shown, Some(1));

        // An older shot's copy, 100 px ahead of the newer provisional: its
        // own provisional left the picture a few frames before.
        let mut frames = still(20);
        frames[1].fires[0] = true;
        frames[6].fires[0] = true;
        for i in 2..10 {
            frames[i].shots = vec![own(PROVISIONAL_ID_BASE, 20.0 + 8.0 * (i - 2) as f32, true, true)];
        }
        for i in 7..16 {
            let mut shots = vec![own(if i < 10 { PROVISIONAL_ID_BASE + 1 } else { PROVISIONAL_ID_BASE }, 20.0 + 8.0 * (i - 7) as f32, true, true)];
            if i >= 12 {
                shots.push(own(4, 120.0 + 8.0 * (i - 12) as f32, false, true));
            }
            frames[i].shots = shots;
        }
        let l = shot_ledger(&frames, &frames, 0, false, None);
        assert_eq!((l.drawn_twice, l.handoff_gap_px.n), (0, 1), "{l:?}");
    }

    /// With the wire's reading, a press is paired with the `Fired` that
    /// names its own tick: a refused press stays unanswered, where pairing
    /// by order would hand it the next press's `Fired`.
    #[test]
    fn the_wire_pairs_a_press_with_its_own_fired() {
        let mut frames = still(40);
        frames[2].fires[0] = true;
        frames[20].fires[0] = true;
        for (i, f) in frames.iter_mut().enumerate() {
            f.tick = 100 + i as u64;
        }
        frames[25].events = vec![EventSample::Fired { slot: 0 }];
        let by_order = shot_ledger(&frames, &frames, 0, false, None);
        assert_eq!(by_order.unanswered, 1);
        assert_eq!(by_order.fired_ms.p50, Some((25 - 2) as f64 * 16.0), "order hands the first press the second's Fired");

        let wire = WireFired { host: vec![None, Some(125)], guest: vec![None, Some(127)] };
        let exact = shot_ledger(&frames, &frames, 0, false, Some(&wire));
        assert_eq!(exact.fired_pairing, "input_tick");
        assert_eq!(exact.unanswered, 1);
        assert_eq!(exact.fired_ms.n, 1);
        assert_eq!(exact.fired_ms.p50, Some(5.0 * 16.0));
        assert_eq!(exact.guest_fired_ms.p50, Some(7.0 * 16.0));
    }

    /// A local round: the enemy shell stops inside the hull on the frame
    /// its hit lands.
    #[test]
    fn a_local_hit_is_from_nowhere_and_on_time() {
        let mut frames = still(10);
        for i in 2..6 {
            frames[i].shots = vec![enemy(5, 60.0 - 10.0 * (i - 2) as f32, true)];
        }
        frames[6].shots = vec![enemy(5, 20.0, false)];
        frames[6].events = vec![EventSample::HitPlayer { player: 0, x: 20.0, y: 0.0 }];
        let inc = incoming_fire(&frames, 0);
        assert_eq!((inc.hits, inc.unseen, inc.phantom_strikes), (1, 0, 0));
        assert_eq!(inc.from_afar_px.p50, Some(0.0));
        assert_eq!(inc.strike_to_hit_ms.p50, Some(0.0));
    }

    /// Incoming fire in the present: the shell is taken off the picture
    /// where it meets the drawn hull, and the room's hit lands later.
    #[test]
    fn a_strike_drawn_in_the_present_is_answered_by_the_later_hit() {
        let mut frames = still(20);
        for i in 2..6 {
            frames[i].shots = vec![enemy(5, 60.0 - 10.0 * (i - 2) as f32, true)];
        }
        frames[12].events = vec![EventSample::HitPlayer { player: 0, x: 22.0, y: 0.0 }];
        let inc = incoming_fire(&frames, 0);
        assert_eq!((inc.hits, inc.unseen, inc.phantom_strikes), (1, 0, 0));
        assert_eq!(inc.from_afar_px.p50, Some(0.0), "carried a frame on, it is inside the hull");
        assert_eq!(inc.strike_to_hit_ms.p50, Some((12 - 6) as f64 * 16.0));
    }

    /// The room says hit where the picture had the shell a hull's length
    /// away: hit from afar, by that gap.
    #[test]
    fn a_hit_where_the_picture_had_the_shell_elsewhere_is_from_afar() {
        let mut frames = still(10);
        for i in 2..6 {
            frames[i].shots = vec![enemy(5, 160.0 - 10.0 * (i - 2) as f32, true)];
        }
        frames[6].shots = vec![enemy(5, 100.0, false)];
        frames[6].events = vec![EventSample::HitPlayer { player: 0, x: 100.0, y: 0.0 }];
        let inc = incoming_fire(&frames, 0);
        let gap = inc.from_afar_px.p50.expect("a gap");
        // Facing right, the scout's turret box reaches 28 px ahead of the
        // hull's centre (9 px out, 19 half long), and the shell's own 3.
        assert!((gap - (100.0 - 28.0 - 3.0)).abs() < 0.01, "{gap}");
    }

    /// A shell that burst somewhere else on the same line was the room's
    /// word on another target: a later hit on that line is not put down
    /// to it, from however far.
    #[test]
    fn a_shell_that_burst_elsewhere_did_not_make_the_hit() {
        let mut frames: Vec<FrameSample> = (0..40).map(|i| frame(i as f64 * 16.0, None, 0.0, 0.0)).collect();
        for i in 2..6 {
            frames[i].shots = vec![enemy(5, 200.0 - 10.0 * (i - 2) as f32, true)];
        }
        for i in 6..12 {
            frames[i].shots = vec![enemy(5, 150.0, false)];
        }
        frames[30].events = vec![EventSample::HitPlayer { player: 0, x: 30.0, y: 0.0 }];
        let inc = incoming_fire(&frames, 0);
        assert_eq!((inc.hits, inc.unseen), (1, 1), "{inc:?}");
        assert_eq!(inc.from_afar_px.n, 0);
    }

    /// A shell that left the picture in flight far from the hull - the
    /// room cleared it - did not make a hit on its line: the picture never
    /// showed it hitting.
    #[test]
    fn a_shell_that_vanished_away_from_the_hull_did_not_make_the_hit() {
        let mut frames = still(20);
        // Flying at the hull, gone 100 px short of it: further than a frame
        // and a catch-up could have carried it.
        for i in 2..6 {
            frames[i].shots = vec![enemy(5, -124.0 + 8.0 * (i - 2) as f32, true)];
        }
        frames[12].events = vec![EventSample::HitPlayer { player: 0, x: -30.0, y: 0.0 }];
        let inc = incoming_fire(&frames, 0);
        assert_eq!((inc.hits, inc.unseen, inc.phantom_strikes), (1, 1, 0), "{inc:?}");
    }

    /// A shell the picture first drew already bursting - it flew and hit
    /// while the picture stood still - still accounts for the hit it burst
    /// at.
    #[test]
    fn a_shell_first_drawn_bursting_at_the_hit_accounts_for_it() {
        let mut frames = still(10);
        for i in 4..8 {
            frames[i].shots = vec![enemy(5, 20.0, false)];
        }
        frames[4].events = vec![EventSample::HitPlayer { player: 0, x: 20.0, y: 0.0 }];
        let inc = incoming_fire(&frames, 0);
        assert_eq!((inc.hits, inc.unseen), (1, 0));
        assert_eq!(inc.from_afar_px.p50, Some(0.0));
    }

    /// A shell taken off the picture a hull's length short of the hull,
    /// flying at it, was struck there by a client drawing incoming fire
    /// ahead of the room: its strike is on the hull, not where its last
    /// frame of flight would have put it.
    #[test]
    fn a_shell_taken_off_flying_at_the_hull_struck_it() {
        let mut frames = still(20);
        for i in 2..6 {
            frames[i].shots = vec![enemy(5, 100.0 - 8.0 * (i - 2) as f32, true)];
        }
        frames[9].events = vec![EventSample::HitPlayer { player: 0, x: 25.0, y: 0.0 }];
        let inc = incoming_fire(&frames, 0);
        assert_eq!((inc.hits, inc.unseen), (1, 0));
        assert_eq!(inc.from_afar_px.p50, Some(0.0), "{inc:?}");
    }

    /// A strike the room never answered, and a hit nothing drawn accounts
    /// for.
    #[test]
    fn a_strike_with_no_hit_is_a_phantom_and_a_hit_with_no_shot_unseen() {
        let mut frames: Vec<FrameSample> = (0..100).map(|i| frame(i as f64 * 16.0, None, 0.0, 0.0)).collect();
        for i in 2..6 {
            frames[i].shots = vec![enemy(5, 60.0 - 10.0 * (i - 2) as f32, true)];
        }
        frames[90].events = vec![EventSample::HitPlayer { player: 0, x: 0.0, y: 300.0 }];
        let inc = incoming_fire(&frames, 0);
        assert_eq!((inc.hits, inc.unseen, inc.phantom_strikes), (1, 1, 1));
    }
}
