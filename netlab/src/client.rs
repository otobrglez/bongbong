//! A headless player: the window's own `OnlineRound` over the real
//! `NativeTransport`, driven by a script at a fixed frame rate on its own
//! thread, recording every frame it draws.
//!
//! Everything a window does in an online round happens here except the
//! drawing: the lobby (host creates, guest joins by code and readies, host
//! starts), one `OnlineRound::frame` per rendered frame with the frame's
//! real `dt`, and the replica that frame leaves behind read back into a
//! `FrameSample`.

use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use bongbong::ai::Intent;
use bongbong::net::client::{Identity, Phase, RoomClient, RoomSetup};
use bongbong::net::clock::RttReport;
use bongbong::net::interp::InterpReport;
use bongbong::net::native::NativeTransport;
use bongbong::net::predict::PredictionReport;
use bongbong::net::round::OnlineRound;
use bongbong::simulation::{Game, Outcome};
use bongbong::tank::Dir;

use crate::proxy::Clock;
use crate::sample::{self, FrameSample, LinkSample};
use crate::script::{Scenario, Script};

/// How long a seat waits for the room to start the round before the run
/// is abandoned.
pub const START_TIMEOUT: Duration = Duration::from_secs(20);

/// How long a dial the network refused waits before the next: a room
/// server started beside netlab may not be listening yet, and a refused
/// connection comes back at once.
pub const DIAL_RETRY: Duration = Duration::from_millis(250);

/// How a dial that never got through reads: `net::native` puts every
/// failed dial as "cannot reach URL: why", and this is tungstenite's why
/// when no address took the TCP connection - nothing listening yet. A name
/// that does not resolve, a TLS failure or an HTTP answer to the upgrade
/// (a wrong path, a preview that is not deployed) read otherwise, and are
/// the run's answer at once.
pub const DIAL_REFUSED: (&str, &str) = ("cannot reach", "Unable to connect");

/// The last millisecond of each frame's wait is spun rather than slept,
/// so frames land on their deadline rather than a scheduler tick later.
pub const SPIN: Duration = Duration::from_millis(2);

/// What the seats of one run share: the room code the host was given, how
/// many seats have finished their script, and a stop for everyone.
#[derive(Debug, Default)]
pub struct Rendezvous {
    pub code: Mutex<Option<String>>,
    pub finished: AtomicUsize,
    pub abort: AtomicBool,
}

/// Who this seat is.
#[derive(Clone, Debug)]
pub enum Role {
    Host(RoomSetup),
    Guest,
}

/// One seat's orders.
#[derive(Clone, Debug)]
pub struct SeatPlan {
    pub url: String,
    pub role: Role,
    /// The script's seat: 0 the host's, 1 the guest's.
    pub script_seat: usize,
    pub scenario: Scenario,
    pub fps: f64,
    pub seconds: f64,
    pub client_hull: bool,
    /// Seats in the run: everyone keeps drawing until they have all
    /// finished, so nobody's picture freezes under someone still recording.
    pub seats: usize,
    /// A unique device token per run and seat.
    pub token: String,
    /// The most taps the script makes (`Scenario::script`).
    pub tap_limit: Option<u32>,
}

/// What one seat brought back.
#[derive(Clone, Debug, Default)]
pub struct SeatRun {
    pub seat: Option<u8>,
    pub samples: Vec<FrameSample>,
    /// When the script began on the process clock.
    pub t0_ms: Option<f64>,
    pub prediction: Option<PredictionReport>,
    pub interp_start: Option<InterpReport>,
    pub interp_end: Option<InterpReport>,
    pub rtt: Option<RttReport>,
    /// Why the seat stopped early, if it did.
    pub error: Option<String>,
    /// The round ended before the script did.
    pub ended_early: bool,
    /// How the room said the round went, if it ended.
    pub outcome: Option<String>,
    /// Dials the network refused before the socket opened.
    pub dial_retries: u32,
}

/// The direction from seat 0's hull toward the centre of the live enemies
/// - what the shooter faces - read off any round.
pub fn aim_of(game: &Game) -> Dir {
    let state = game.drawable_state();
    let Some(me) = state.tanks.iter().find(|t| t.player == Some(0)) else { return Dir::Right };
    let foes: Vec<_> = state.tanks.iter().filter(|t| t.player.is_none() && !t.wreck).collect();
    if foes.is_empty() {
        return Dir::Right;
    }
    let n = foes.len() as f32;
    let cx = foes.iter().map(|t| t.x as f32).sum::<f32>() / n;
    let cy = foes.iter().map(|t| t.y as f32).sum::<f32>() / n;
    Dir::toward(bongbong::Position::new(me.x as f32, me.y as f32), bongbong::Position::new(cx, cy))
}

/// Sleep until `deadline`, spinning the last `SPIN`.
pub fn wait_until(deadline: Instant) {
    loop {
        let now = Instant::now();
        if now >= deadline {
            return;
        }
        let left = deadline - now;
        if left > SPIN {
            std::thread::sleep(left - SPIN);
        } else {
            std::hint::spin_loop();
        }
    }
}

/// Open this seat's round: the host creates the room, the guest joins
/// `code`.
fn dial(plan: &SeatPlan, code: Option<&str>) -> OnlineRound<NativeTransport> {
    let identity = Identity::new(if plan.script_seat == 0 { "host" } else { "guest" }, plan.token.clone());
    let transport = NativeTransport::connect(&plan.url);
    let client = match &plan.role {
        Role::Host(setup) => RoomClient::host(transport, identity, setup.clone()),
        Role::Guest => RoomClient::join(transport, identity, code.unwrap_or_default().to_string()),
    };
    let mut round = OnlineRound::new(client, "NETLAB");
    round.set_client_hull(plan.client_hull);
    round
}

/// Whether a round's TCP connection was refused before its socket ever
/// opened (`DIAL_REFUSED`) - the dial is worth making again while the room
/// server may still be starting - as opposed to a dial the server or the
/// network answered, or a room that answered and then closed (`answered`).
pub fn refused_before_open(phase: &Phase, answered: bool) -> bool {
    let (prefix, why) = DIAL_REFUSED;
    !answered && matches!(phase, Phase::Closed(c) if c.reason.starts_with(prefix) && c.reason.contains(why))
}

/// Play one seat to the end of its script (and until every other seat is
/// done too).
pub fn run_seat(plan: SeatPlan, clock: Clock, meet: &Rendezvous) -> SeatRun {
    let mut out = SeatRun::default();
    let code = match &plan.role {
        Role::Host(_) => None,
        Role::Guest => {
            // The host may spend its own start timeout dialling a server
            // that is still coming up, and gives the run up if it never
            // answers: this waits out both.
            let deadline = Instant::now() + 2 * START_TIMEOUT;
            loop {
                if let Some(code) = meet.code.lock().unwrap_or_else(PoisonError::into_inner).clone() {
                    break Some(code);
                }
                if meet.abort.load(Ordering::Relaxed) || Instant::now() > deadline {
                    out.error = Some("no room code from the host".into());
                    meet.finished.fetch_add(1, Ordering::SeqCst);
                    return out;
                }
                std::thread::sleep(Duration::from_millis(5));
            }
        }
    };
    let mut round = dial(&plan, code.as_deref());
    // The socket opened and the room has said something: from here a
    // close is the room's word, not a server still starting.
    let mut answered = false;
    let mut redial_at: Option<Instant> = None;

    let frame = Duration::from_secs_f64(1.0 / plan.fps);
    let opened = Instant::now();
    let mut next = opened + frame;
    let mut last = opened;
    let mut readied = false;
    let mut script: Option<Script> = None;
    let mut done = false;
    let mut prev_tick: Option<u64> = None;
    let mut prev_extrapolated: Option<u64> = None;
    loop {
        wait_until(next);
        let now = Instant::now();
        let dt = (now - last).as_secs_f32();
        last = now;
        next += frame;
        // A frame far behind its deadline (the machine stalled) starts
        // the cadence again rather than bursting to catch up.
        if now > next + 4 * frame {
            next = now + frame;
        }
        let now_ms = clock.ms(now);

        answered |= !matches!(round.phase(), Phase::Connecting | Phase::Closed(_));
        if refused_before_open(round.phase(), answered) && now - opened < START_TIMEOUT && !meet.abort.load(Ordering::SeqCst) {
            match redial_at {
                None => redial_at = Some(now + DIAL_RETRY),
                Some(at) if now >= at => {
                    round = dial(&plan, code.as_deref());
                    out.dial_retries += 1;
                    redial_at = None;
                }
                Some(_) => {}
            }
            continue;
        }

        match &plan.role {
            Role::Host(_) => {
                if let Some(code) = round.code() {
                    let mut slot = meet.code.lock().unwrap_or_else(PoisonError::into_inner);
                    if slot.is_none() {
                        *slot = Some(code.to_string());
                    }
                }
                let others = round.roster().len();
                if others >= plan.seats && round.can_start() {
                    round.start_round();
                }
            }
            Role::Guest => {
                if !readied && *round.phase() == Phase::Lobby {
                    round.ready();
                    readied = true;
                }
            }
        }

        let playing = *round.phase() == Phase::Playing && round.game().is_some();
        if script.is_none() && playing && !done {
            let aim = round.game().map_or(Dir::Right, aim_of);
            script = Some(plan.scenario.script(plan.script_seat, aim, plan.tap_limit));
            out.t0_ms = Some(now_ms);
            out.interp_start = Some(round.interpolation());
        }
        let script_s = out.t0_ms.map(|t0| (now_ms - t0) / 1000.0);
        let recording = !done && script.is_some() && script_s.is_some_and(|s| s < plan.seconds);
        let intent = match (&mut script, recording) {
            (Some(script), true) => script.intent(script_s.unwrap_or(0.0)),
            _ => Intent::default(),
        };

        let cpu = Instant::now();
        round.frame(&intent, dt);
        let cpu_us = cpu.elapsed().as_secs_f64() * 1e6;
        let interp = round.interpolation();

        if recording && let Some(game) = round.game() {
            let mut s = FrameSample { t_ms: now_ms, script_s: script_s.unwrap_or(0.0), cpu_us, ..FrameSample::default() };
            let seat = round.seat().map_or(plan.script_seat, usize::from);
            sample::read_intent(seat, &intent, &mut s);
            sample::read_picture(game, round.seat(), &mut s);
            // A replica's events are the ones the interpolator handed over
            // this frame; they stay on the `Game` until the next hand-over,
            // so they are read on the frame the drawn tick moved.
            if prev_tick != Some(s.tick) {
                sample::read_events(game.events(), &mut s.events);
            }
            prev_tick = Some(s.tick);
            s.link = Some(LinkSample::read(
                round.prediction(),
                round.lead_depth(),
                round.rtt(),
                round.buffer_ms(),
                &interp,
                prev_extrapolated,
            ));
            out.samples.push(s);
        }
        prev_extrapolated = Some(interp.extrapolated_frames);

        // The round decided (the end screen up) or over: the script stops
        // here, as the twin's does.
        let ended = round.ended().is_some() || round.game().is_some_and(|g| g.outcome() != Outcome::Playing);
        let closed = matches!(round.phase(), Phase::Closed(_));
        if !done && script.is_some() && (!recording || ended || closed) {
            done = true;
            out.ended_early = ended || closed;
            out.outcome = round
                .ended()
                .map(|o| format!("{o:?}").to_lowercase())
                .or_else(|| round.game().map(|g| format!("{:?}", g.outcome()).to_lowercase()));
            out.prediction = round.prediction();
            out.interp_end = Some(round.interpolation());
            out.rtt = round.rtt();
            out.seat = round.seat();
            if closed {
                out.error = round.note().map(str::to_string).or(Some("the socket closed".into()));
            }
            meet.finished.fetch_add(1, Ordering::SeqCst);
        }
        // A dial the network refused is made again on the next frames
        // (above) until the start timeout, not given up on here.
        let redialling = refused_before_open(round.phase(), answered) && now - opened < START_TIMEOUT;
        if !done && script.is_none() && ((closed && !redialling) || now - opened > START_TIMEOUT) {
            out.error = Some(format!("the round never started: {}", round.status()));
            meet.abort.store(true, Ordering::SeqCst);
            meet.finished.fetch_add(1, Ordering::SeqCst);
            done = true;
        }
        if meet.abort.load(Ordering::SeqCst) && !done {
            out.error.get_or_insert_with(|| "another seat gave up".into());
            meet.finished.fetch_add(1, Ordering::SeqCst);
            break;
        }
        if done && (meet.finished.load(Ordering::SeqCst) >= plan.seats || meet.abort.load(Ordering::SeqCst)) {
            break;
        }
        // A seat that finished waits for the others, but not forever.
        if now - opened > START_TIMEOUT + Duration::from_secs_f64(plan.seconds) + START_TIMEOUT {
            break;
        }
    }
    round.leave();
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use bongbong::net::transport::Closed;

    /// Only a TCP connection refused before the socket ever opened is made
    /// again; a server that answered the upgrade, a name that does not
    /// resolve, a TLS failure or a room that answered and then closed has
    /// said its word.
    #[test]
    fn only_a_refused_dial_is_made_again() {
        let refused = Phase::Closed(Closed::fault("cannot reach ws://127.0.0.1:4848/ws: URL error: Unable to connect to ws://127.0.0.1:4848/ws"));
        assert!(refused_before_open(&refused, false));
        assert!(!refused_before_open(&refused, true), "the room had answered");
        for said in [
            "cannot reach wss://rooms.bongbong.io/pr-48/ws: HTTP error: 404 Not Found",
            "cannot reach wss://rooms.bongbong.io/pr-48/ws: HTTP error: 503 Service Unavailable",
            "cannot reach wss://rooms.bongbong.example/ws: IO error: failed to lookup address information: nodename nor servname provided, or not known",
            "cannot reach wss://127.0.0.1:4848/ws: TLS error: invalid peer certificate",
            "the room closed the connection",
        ] {
            assert!(!refused_before_open(&Phase::Closed(Closed::fault(said)), false), "{said}");
        }
        assert!(!refused_before_open(&Phase::Connecting, false));
    }
}
