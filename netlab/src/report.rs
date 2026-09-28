//! What a run found: the online view and the local twin side by side, the
//! client's own counters, the wire as the tap saw it, and a verdict.

use serde::{Deserialize, Serialize};

use crate::link::Impairment;
use crate::metrics::{Metrics, Stat, WireFired};
use crate::proxy::TapLog;
use crate::sample::{FrameSample, LinkSample};
use crate::script::Scenario;

/// One seat's own readings, off its `OnlineRound` at the end of its
/// script.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SeatSummary {
    pub seat: Option<u8>,
    pub frames: usize,
    /// Corrections the prediction eased off, and took whole.
    pub nudges: u32,
    pub snaps: u32,
    pub max_error_px: f32,
    /// Packets the lead added, and skipped.
    pub lead_up: u32,
    pub lead_down: u32,
    pub shots_drawn: u32,
    pub shots_refused: u32,
    pub crossings: u32,
    pub crossings_hit: u32,
    pub crossings_missed: u32,
    /// The interpolation delay in force, per frame.
    pub interp_delay_ms: Stat,
    pub interp_jitter_ms: f64,
    pub interp_interval_ms: f64,
    /// Frames drawn past the newest snapshot over the script.
    pub extrapolated_frames: u64,
    /// Arrival lateness behind the clock's envelope at the end of the
    /// script, median and 95th percentile: what the delay covers.
    pub lateness_p50_ms: f64,
    pub lateness_p95_ms: f64,
    /// Head-of-line stalls the interpolator rode out over the script.
    pub stalls: u64,
    /// The playout clock's speed, per frame.
    pub playout_rate: Stat,
    /// Corrections the interpolator eased by an error offset over the
    /// script, and their 95th-percentile size at its end.
    pub interp_corrections: u64,
    pub interp_correction_p95_px: f64,
    pub rtt_p50_ms: Option<f64>,
    pub rtt_p95_ms: Option<f64>,
    pub rtt_min_ms: Option<f64>,
    /// Dials the network refused before the socket opened.
    pub dial_retries: u32,
    pub ended_early: bool,
    /// How the round ended, when it ended before the script.
    pub outcome: Option<String>,
    pub error: Option<String>,
}

/// One frame of a seat's own readings, for a time series.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SeriesRow {
    /// The process clock, milliseconds.
    pub t_ms: f64,
    /// Seconds into the seat's script.
    pub script_s: f64,
    /// The tick the picture drew.
    pub tick: u64,
    #[serde(flatten)]
    pub link: LinkSample,
}

/// A seat's readings frame by frame over its script.
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct SeatSeries {
    pub seat: Option<u8>,
    pub frames: Vec<SeriesRow>,
}

impl SeatSeries {
    /// The rows of every recorded frame that carries the client's
    /// readings.
    pub fn of(seat: Option<u8>, samples: &[FrameSample]) -> SeatSeries {
        let frames = samples
            .iter()
            .filter_map(|f| f.link.map(|link| SeriesRow { t_ms: f.t_ms, script_s: f.script_s, tick: f.tick, link }))
            .collect();
        SeatSeries { seat, frames }
    }
}

/// The wire as the proxy saw it (in-process mode only).
#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
pub struct TapMetrics {
    /// Client to server, reaching the proxy to leaving it: the link's own
    /// delay as applied, head-of-line holds included.
    pub upstream_ms: Stat,
    /// Server to client, the same.
    pub downstream_ms: Stat,
    /// An intent delivered to the server, to the first snapshot leaving
    /// the server whose `acked` covers it.
    pub server_hold_ms: Stat,
    /// Between two snapshots leaving the proxy toward the same client.
    pub snapshot_gap_ms: Stat,
    pub stalls_over_50ms: usize,
    pub stalls_over_100ms: usize,
    pub stalls_over_250ms: usize,
    pub up_bytes_per_s: f64,
    pub down_bytes_per_s: f64,
    pub intents: usize,
    pub snapshots: usize,
    /// Chunks the link held for a retransmit, each way.
    pub up_lost: u32,
    pub down_lost: u32,
    pub tap_failed: Option<String>,
}

/// A whole run.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Report {
    pub label: String,
    pub profile: String,
    pub link: Impairment,
    pub scenario: Scenario,
    pub client_hull: bool,
    pub fps: f64,
    pub seconds: f64,
    pub seed: u64,
    pub enemies: usize,
    pub tank: String,
    pub map: String,
    pub remote: Option<String>,
    /// The magazine netlab pinned on the room, the clients and the twin
    /// (`max_shells`), so no scripted press is refused for ammo; `None` on
    /// a remote room, whose magazine is its own - there the scripts stop
    /// tapping at `tap_limit`.
    pub max_shells: Option<i32>,
    /// The most presses each script made.
    pub tap_limit: Option<u32>,
    /// The online view: the host's frames for everything the host sees,
    /// the guest's for how the host looks to someone else.
    pub online: Option<Metrics>,
    /// The same metrics off the local twin.
    pub twin: Metrics,
    /// Host first.
    pub seats: Vec<SeatSummary>,
    /// Each seat's own readings frame by frame (`SeatSeries`), host first.
    pub series: Vec<SeatSeries>,
    pub tap: Option<TapMetrics>,
    pub verdict: String,
    /// What kept the verdict from being `local`.
    pub misses: Vec<String>,
    pub errors: Vec<String>,
}

/// The tap's metrics over `[from_ms, to_ms]` of the process clock.
pub fn tap_metrics(log: &TapLog, from_ms: f64, to_ms: f64) -> TapMetrics {
    let inside = |t: f64| t >= from_ms && t <= to_ms;
    let mut up = Vec::new();
    let mut down = Vec::new();
    let mut hold = Vec::new();
    let mut gaps = Vec::new();
    let (mut up_bytes, mut down_bytes) = (0usize, 0usize);
    let mut out = TapMetrics::default();
    for c in &log.conns {
        out.up_lost += c.up_lost;
        out.down_lost += c.down_lost;
        if out.tap_failed.is_none() {
            out.tap_failed = c.tap_failed.clone();
        }
        up_bytes += c.up_chunks.iter().filter(|(t, _)| inside(*t)).map(|(_, n)| n).sum::<usize>();
        down_bytes += c.down_chunks.iter().filter(|(t, _)| inside(*t)).map(|(_, n)| n).sum::<usize>();
        for i in c.intents.iter().filter(|i| inside(i.ingress_ms)) {
            up.push(i.egress_ms - i.ingress_ms);
            out.intents += 1;
        }
        let snaps: Vec<_> = c.snapshots.iter().filter(|s| inside(s.ingress_ms)).collect();
        out.snapshots += snaps.len();
        down.extend(snaps.iter().map(|s| s.egress_ms - s.ingress_ms));
        for w in snaps.windows(2) {
            let gap = w[1].egress_ms - w[0].egress_ms;
            gaps.push(gap);
            out.stalls_over_50ms += (gap > 50.0) as usize;
            out.stalls_over_100ms += (gap > 100.0) as usize;
            out.stalls_over_250ms += (gap > 250.0) as usize;
        }
        let Some(seat) = c.seat else { continue };
        let seat = seat as usize;
        for i in c.intents.iter().filter(|i| inside(i.egress_ms) && i.tick > 0) {
            // `acked` only grows within a round, so the first snapshot
            // covering the intent is a partition point; from there, the
            // first that left after the intent reached the server.
            let from = snaps.partition_point(|s| s.acked[seat] < i.tick);
            if let Some(s) = snaps[from..].iter().find(|s| s.ingress_ms >= i.egress_ms) {
                hold.push(s.ingress_ms - i.egress_ms);
            }
        }
    }
    let seconds = ((to_ms - from_ms) / 1000.0).max(1e-3);
    out.upstream_ms = Stat::of(&up);
    out.downstream_ms = Stat::of(&down);
    out.server_hold_ms = Stat::of(&hold);
    out.snapshot_gap_ms = Stat::of(&gaps);
    out.up_bytes_per_s = up_bytes as f64 / seconds;
    out.down_bytes_per_s = down_bytes as f64 / seconds;
    out
}

/// Which snapshot carried each host press's `Fired`, off the wire: the
/// press's intent is the first on the host's connection to raise the
/// trigger after the press's frame, and its `Fired` is the one whose
/// `input_tick` falls from that intent's tick up to the next press's (the
/// room merges a tick's inputs newest-wins, so it can name a later tick
/// than the press's own). The guest's is the same `Fired` on the guest's
/// connection. `presses` are the host frames' press times on the process
/// clock the proxy stamps with.
pub fn wire_fired(log: &TapLog, presses: &[f64], host_seat: u8, guest_seat: u8) -> WireFired {
    let conn = |seat: u8| log.conns.iter().find(|c| c.seat == Some(seat));
    let Some(host) = conn(host_seat) else { return WireFired::default() };
    // The intents that raised the trigger: a tap is held over a few
    // ticks, so only the first of each run is a press.
    let mut edges: Vec<(f64, u32)> = Vec::new();
    let mut held = false;
    for i in &host.intents {
        if i.fire && !held {
            edges.push((i.ingress_ms, i.tick));
        }
        held = i.fire;
    }
    let press_ticks: Vec<Option<u32>> = presses
        .iter()
        .enumerate()
        .map(|(k, &p)| {
            let until = presses.get(k + 1).copied().unwrap_or(f64::INFINITY);
            edges.iter().find(|(at, _)| *at >= p && *at < until).map(|&(_, tick)| tick)
        })
        .collect();
    let carried = |c: Option<&crate::proxy::ConnTap>, k: usize| -> Option<u32> {
        let from = press_ticks[k]?;
        let until = press_ticks[k + 1..].iter().flatten().next().copied().unwrap_or(u32::MAX);
        c?.fired
            .iter()
            .find(|f| f.slot == host_seat as u16 && f.input_tick >= from && f.input_tick < until)
            .map(|f| f.snapshot_tick)
    };
    WireFired {
        host: (0..presses.len()).map(|k| carried(Some(host), k)).collect(),
        guest: (0..presses.len()).map(|k| carried(conn(guest_seat), k)).collect(),
    }
}

/// The verdict's thresholds (docs/online-coop-prd.md §4.16): within
/// these, networked play measures as local.
pub mod local {
    /// Own input no more than one frame behind the twin's.
    pub const OWN_INPUT_FRAMES: f64 = 1.0;
    /// Remote stalls no more than this many points over the twin's.
    pub const STALL_PCT: f64 = 2.0;
    pub const JUMP_PCT: f64 = 2.0;
    pub const BACKWARD_PCT: f64 = 0.5;
    pub const HANDOFF_GAP_PX: f64 = 2.0;
    pub const HIT_FROM_AFAR_PX: f64 = 12.0;
}

/// How much looser "close" is than "local": every threshold times this
/// (the frame budget, three frames rather than one), and a handful of
/// corrections allowed where local allows none.
pub const CLOSE_FACTOR: f64 = 3.0;
pub const CLOSE_CORRECTIONS: u32 = 5;

/// `local`, `close` or `far`, with what missed `local`.
pub fn verdict(online: &Metrics, twin: &Metrics, seats: &[SeatSummary], client_hull: bool) -> (String, Vec<String>) {
    let mut misses = Vec::new();
    let mut far = false;
    let mut check = |name: &str, value: Option<f64>, limit: f64, close_limit: f64| {
        let Some(v) = value else { return };
        if v > limit {
            misses.push(format!("{name} {v:.1} > {limit:.1}"));
            if v > close_limit {
                far = true;
            }
        }
    };
    let twin_own = twin.own_input_frames.p95.unwrap_or(0.0);
    check(
        "own input p95 frames",
        online.own_input_frames.p95,
        twin_own + local::OWN_INPUT_FRAMES,
        twin_own + CLOSE_FACTOR * local::OWN_INPUT_FRAMES,
    );
    if let (Some(o), Some(t)) = (&online.remote_pacing, &twin.remote_pacing)
        && o.frames > 0
    {
        check("remote stall%", Some(o.stall_pct), t.stall_pct + local::STALL_PCT, t.stall_pct + CLOSE_FACTOR * local::STALL_PCT);
        check("remote jump%", Some(o.jump_pct), local::JUMP_PCT, CLOSE_FACTOR * local::JUMP_PCT);
        check("remote backward%", Some(o.backward_pct), local::BACKWARD_PCT, CLOSE_FACTOR * local::BACKWARD_PCT);
    }
    check("handoff gap p95 px", online.shots.handoff_gap_px.p95, local::HANDOFF_GAP_PX, CLOSE_FACTOR * local::HANDOFF_GAP_PX);
    // A local round never draws a shot twice; a handful is close.
    check("own shots drawn twice", Some(online.shots.drawn_twice as f64), 0.0, CLOSE_CORRECTIONS as f64);
    check("hit from afar p95 px", online.incoming.from_afar_px.p95, local::HIT_FROM_AFAR_PX, CLOSE_FACTOR * local::HIT_FROM_AFAR_PX);
    if client_hull {
        let corrections: u32 = seats.iter().map(|s| s.nudges + s.snaps).sum();
        if corrections > 0 {
            misses.push(format!("nudges+snaps {corrections} > 0"));
            if corrections > CLOSE_CORRECTIONS {
                far = true;
            }
        }
    }
    let verdict = if misses.is_empty() {
        "local"
    } else if far {
        "far"
    } else {
        "close"
    };
    (verdict.to_string(), misses)
}

fn ms(v: Option<f64>) -> String {
    v.map_or_else(|| "-".to_string(), |v| format!("{v:.0}"))
}

fn one(v: Option<f64>) -> String {
    v.map_or_else(|| "-".to_string(), |v| format!("{v:.1}"))
}

fn rate(v: Option<f64>) -> String {
    v.map_or_else(|| "-".to_string(), |v| format!("{v:.3}"))
}

/// A count, with its p95 beside it when there is one: `0`, `3 (12.5)`.
fn count_and(s: Stat) -> String {
    match s.p95 {
        Some(p) if s.n > 0 => format!("{} ({p:.1})", s.n),
        _ => s.n.to_string(),
    }
}

/// A p95 with its count beside it: `12.5 (7)`, `- (0)`.
fn p95_and_n(s: Stat) -> String {
    format!("{} ({})", one(s.p95), s.n)
}

/// A few lines per run for a terminal.
pub fn summary(r: &Report) -> String {
    let mut s = String::new();
    let link = &r.link;
    s.push_str(&format!(
        "netlab {} - {} ({} ms ±{} loss {:.1}% rto {} ms{}) - {} - client hull {} - {} fps - {} s{}\n",
        r.label,
        r.profile,
        link.delay_ms,
        link.jitter_ms,
        link.loss * 100.0,
        link.rto_ms,
        if link.nagle { " nagle" } else { "" },
        r.scenario.name(),
        if r.client_hull { "on" } else { "off" },
        r.fps,
        r.seconds,
        r.remote.as_ref().map_or(String::new(), |u| format!(" - remote {u}")),
    ));
    let rows: Vec<(&str, &Metrics)> = r.online.iter().map(|m| ("online", m)).chain([("twin", &r.twin)]).collect();
    for (name, m) in rows {
        let pacing = m.remote_pacing.as_ref().map_or("-".to_string(), |p| {
            format!("stall {:.1}% jump {:.1}% back {:.1}% cv {:.2} ({} frames)", p.stall_pct, p.jump_pct, p.backward_pct, p.cv, p.frames)
        });
        s.push_str(&format!(
            "  {name:6} own input p50/p95 {}/{} ms ({}/{} frames) | remote lag p50/p95/max {}/{}/{} ms | pacing {pacing}\n",
            ms(m.own_input_ms.p50),
            ms(m.own_input_ms.p95),
            ms(m.own_input_frames.p50),
            ms(m.own_input_frames.p95),
            ms(m.remote_lag_ms.p50),
            ms(m.remote_lag_ms.p95),
            ms(m.remote_lag_ms.max),
        ));
        let l = &m.shots;
        if l.presses > 0 {
            s.push_str(&format!(
                "         shots {} presses: drawn p50/p95 {}/{} ms, fired {}/{} ms (by {}), guest fired {}/{} ms, hit {}/{} ms (n {}), unanswered {}\n",
                l.presses,
                ms(l.drawn_ms.p50),
                ms(l.drawn_ms.p95),
                ms(l.fired_ms.p50),
                ms(l.fired_ms.p95),
                l.fired_pairing,
                ms(l.guest_fired_ms.p50),
                ms(l.guest_fired_ms.p95),
                ms(l.hit_ms.p50),
                ms(l.hit_ms.p95),
                l.hit_ms.n,
                l.unanswered,
            ));
            s.push_str(&format!(
                "         own shots: {} hand-offs, gap p50/p95/max {}/{}/{} px, {} drawn twice, room copies shown {}\n",
                l.handoff_gap_px.n,
                one(l.handoff_gap_px.p50),
                one(l.handoff_gap_px.p95),
                one(l.handoff_gap_px.max),
                l.drawn_twice,
                l.room_copies_shown.map_or_else(|| "- (no shot drawn ahead of the room)".to_string(), |n| n.to_string()),
            ));
        }
        let inc = &m.incoming;
        if inc.hits > 0 || inc.phantom_strikes > 0 {
            s.push_str(&format!(
                "         incoming {} hits: from afar p50/p95/max {}/{}/{} px, strike to hit p50/p95 {}/{} ms, {} with no shot drawn, {} strikes drawn with no hit\n",
                inc.hits,
                one(inc.from_afar_px.p50),
                one(inc.from_afar_px.p95),
                one(inc.from_afar_px.max),
                ms(inc.strike_to_hit_ms.p50),
                ms(inc.strike_to_hit_ms.p95),
                inc.unseen,
                inc.phantom_strikes,
            ));
        }
        s.push_str(&format!(
            "         frame cpu p50/p99/max {}/{}/{} us, frame interval p50/p99/max {}/{}/{} ms\n",
            ms(m.frame_cpu_us.p50),
            ms(m.frame_cpu_us.p99),
            ms(m.frame_cpu_us.max),
            one(m.frame_interval_ms.p50),
            one(m.frame_interval_ms.p99),
            one(m.frame_interval_ms.max),
        ));
    }
    for seat in &r.seats {
        s.push_str(&format!(
            "  seat {}: nudges {} snaps {} max err {:.1} px, lead +{}/-{}, interp delay p50 {} ms jitter {:.1} ms lateness p50/p95 {:.1}/{:.1} ms, extrapolated {} frames, stalls {}, rate min/max {}/{}, corrections {} (p95 {:.1} px), rtt p50/p95 {}/{} ms{}{}{}\n",
            seat.seat.map_or("-".to_string(), |s| s.to_string()),
            seat.nudges,
            seat.snaps,
            seat.max_error_px,
            seat.lead_up,
            seat.lead_down,
            ms(seat.interp_delay_ms.p50),
            seat.interp_jitter_ms,
            seat.lateness_p50_ms,
            seat.lateness_p95_ms,
            seat.extrapolated_frames,
            seat.stalls,
            rate(seat.playout_rate.min),
            rate(seat.playout_rate.max),
            seat.interp_corrections,
            seat.interp_correction_p95_px,
            ms(seat.rtt_p50_ms),
            ms(seat.rtt_p95_ms),
            if seat.dial_retries > 0 { format!(" - {} dials refused", seat.dial_retries) } else { String::new() },
            if seat.ended_early { format!(" - round ended early ({})", seat.outcome.as_deref().unwrap_or("closed")) } else { String::new() },
            seat.error.as_ref().map_or(String::new(), |e| format!(" - {e}")),
        ));
    }
    match (r.max_shells, r.tap_limit) {
        (Some(n), _) => s.push_str(&format!("  magazine: {n} shells, pinned on the room, the clients and the twin\n")),
        (None, Some(n)) => s.push_str(&format!("  magazine: the room's own (unpinned); each script stops tapping after {n} presses\n")),
        (None, None) => s.push_str("  magazine: the room's own (unpinned)\n"),
    }
    if let Some(t) = &r.tap {
        s.push_str(&format!(
            "  tap: up p50/p95 {}/{} ms, down {}/{} ms, server hold {}/{} ms, snapshot gap p50/p95/p99/max {}/{}/{}/{} ms, stalls >50/100/250 {}/{}/{}, {:.0}/{:.0} B/s up/down, lost chunks {}/{}{}\n",
            one(t.upstream_ms.p50),
            one(t.upstream_ms.p95),
            one(t.downstream_ms.p50),
            one(t.downstream_ms.p95),
            one(t.server_hold_ms.p50),
            one(t.server_hold_ms.p95),
            one(t.snapshot_gap_ms.p50),
            one(t.snapshot_gap_ms.p95),
            one(t.snapshot_gap_ms.p99),
            one(t.snapshot_gap_ms.max),
            t.stalls_over_50ms,
            t.stalls_over_100ms,
            t.stalls_over_250ms,
            t.up_bytes_per_s,
            t.down_bytes_per_s,
            t.up_lost,
            t.down_lost,
            t.tap_failed.as_ref().map_or(String::new(), |e| format!(", tap failed: {e}")),
        ));
    }
    for e in &r.errors {
        s.push_str(&format!("  error: {e}\n"));
    }
    s.push_str(&format!(
        "  verdict: {}{}\n",
        r.verdict,
        if r.misses.is_empty() { String::new() } else { format!(" ({})", r.misses.join("; ")) }
    ));
    s
}

/// The suite's table header.
pub const TABLE_HEADER: &str = "| profile | scenario | mode | own input p50/p95 ms (p95 frames) | remote lag p50/p95 ms | stall % | jump % | back % | cv | shot drawn p50 ms | fired p50 ms | hit p50 ms | hand-offs n (gap p95 px) | drawn twice | hit from afar p95 px (n) | nudges+snaps | rtt p50 ms | snap gap p99/max ms | verdict |\n|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|---|";

/// One row of the suite's table: the online view of `r`, or its twin.
pub fn table_row(r: &Report, twin: bool) -> String {
    let m = if twin { &r.twin } else { r.online.as_ref().unwrap_or(&r.twin) };
    let (profile, mode) = if twin {
        ("local twin".to_string(), "-".to_string())
    } else {
        (r.profile.clone(), format!("hull {}", if r.client_hull { "on" } else { "off" }))
    };
    let p = m.remote_pacing.unwrap_or_default();
    let pacing = |v: f64| if m.remote_pacing.is_some_and(|p| p.frames > 0) { format!("{v:.1}") } else { "-".into() };
    let corrections = if twin { "-".to_string() } else { r.seats.iter().map(|s| s.nudges + s.snaps).sum::<u32>().to_string() };
    let rtt = if twin { "-".into() } else { ms(r.seats.first().and_then(|s| s.rtt_p50_ms)) };
    let gap = match (&r.tap, twin) {
        (Some(t), false) => format!("{}/{}", ms(t.snapshot_gap_ms.p99), ms(t.snapshot_gap_ms.max)),
        _ => "-".into(),
    };
    let verdict = if twin { "reference".to_string() } else { r.verdict.clone() };
    format!(
        "| {profile} | {} | {mode} | {}/{} ({}) | {}/{} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {} | {corrections} | {rtt} | {gap} | {verdict} |",
        r.scenario.name(),
        ms(m.own_input_ms.p50),
        ms(m.own_input_ms.p95),
        ms(m.own_input_frames.p95),
        ms(m.remote_lag_ms.p50),
        ms(m.remote_lag_ms.p95),
        pacing(p.stall_pct),
        pacing(p.jump_pct),
        pacing(p.backward_pct),
        if m.remote_pacing.is_some_and(|p| p.frames > 0) { format!("{:.2}", p.cv) } else { "-".into() },
        ms(m.shots.drawn_ms.p50),
        ms(m.shots.fired_ms.p50),
        ms(m.shots.hit_ms.p50),
        count_and(m.shots.handoff_gap_px),
        if twin { "-".to_string() } else { m.shots.drawn_twice.to_string() },
        p95_and_n(m.incoming.from_afar_px),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::{ConnTap, FiredRec, IntentRec};

    fn intent(at: f64, tick: u32, fire: bool) -> IntentRec {
        IntentRec { ingress_ms: at, egress_ms: at + 1.0, tick, owned: true, fire }
    }

    fn fired(slot: u16, input_tick: u32, snapshot_tick: u32) -> FiredRec {
        FiredRec { egress_ms: 0.0, snapshot_tick, slot, input_tick }
    }

    /// A press is its intent that raised the trigger; its `Fired` is the
    /// host's that names a tick from there up to the next press's - later
    /// than the press's own when the room merged the tick's inputs - and
    /// the guest's is that same `Fired`. A press the room refused has none.
    #[test]
    fn the_wire_names_the_snapshot_each_press_fired_in() {
        let host = ConnTap {
            seat: Some(0),
            // The first tap held over two ticks, the second over two more.
            intents: vec![intent(105.0, 10, true), intent(122.0, 11, true), intent(139.0, 12, false), intent(505.0, 34, true), intent(522.0, 35, true)],
            fired: vec![fired(1, 12, 58), fired(0, 11, 60)],
            ..ConnTap::default()
        };
        let guest = ConnTap { seat: Some(1), fired: vec![fired(0, 11, 63)], ..ConnTap::default() };
        let log = TapLog { conns: vec![host, guest] };
        let wire = wire_fired(&log, &[100.0, 500.0], 0, 1);
        assert_eq!(wire.host, vec![Some(60), None]);
        assert_eq!(wire.guest, vec![Some(63), None]);
    }

    /// Every row of the suite's table has the header's columns.
    #[test]
    fn a_table_row_has_the_headers_columns() {
        let report = Report {
            label: "t".into(),
            profile: "lan".into(),
            link: Impairment::NONE,
            scenario: Scenario::Shoot,
            client_hull: true,
            fps: 60.0,
            seconds: 1.0,
            seed: 1,
            enemies: 0,
            tank: "scout".into(),
            map: "m".into(),
            remote: None,
            max_shells: Some(100),
            tap_limit: None,
            online: Some(Metrics::default()),
            twin: Metrics::default(),
            seats: vec![SeatSummary::default()],
            series: Vec::new(),
            tap: None,
            verdict: "local".into(),
            misses: Vec::new(),
            errors: Vec::new(),
        };
        let columns = |line: &str| line.matches('|').count();
        let header = TABLE_HEADER.lines().next().expect("a header");
        assert_eq!(columns(TABLE_HEADER.lines().nth(1).expect("a rule")), columns(header));
        assert_eq!(columns(&table_row(&report, false)), columns(header));
        assert_eq!(columns(&table_row(&report, true)), columns(header));
    }
}
