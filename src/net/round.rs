//! An online round as the window plays it (docs/online-coop-prd.md §4.5,
//! "One client, three modes"): a `RoomClient` on one side, a replica
//! `Game` on the other, and one call a frame between them.
//!
//! The replica is an ordinary `Game` that nothing ever `update`s. Its
//! world is written from the snapshots the room sends, blended by
//! `net::interp`, and everything between two snapshots - the hulls
//! swinging round, the tread marks, the grass, the fires, the banners -
//! is `Game::tick_presentation`'s. That is what makes `Game::render`,
//! `hud`, `fx`, `view` and the touch scheme work here without knowing a
//! network exists.
//!
//! One frame, in order:
//!
//! 1. `poll` the client: a `Welcome` builds the replica (`net::apply`), a
//!    snapshot reconciles the prediction, goes into the interpolator and
//!    steers the lead, a refusal or a close is kept for the status line.
//! 2. Send the local seat's intent - the same `Intent` a local round
//!    would drive player 1 with, seat 0 of the frame's `Input` - one
//!    packet per tick of real time, plus or minus one when the lead says
//!    so (`Lead`, docs/online-coop-prd.md §4.12).
//! 3. Write the interpolated picture into the replica, the predicted
//!    hull and shots over it, and tick its cosmetics.
//!
//! Nothing here runs `Game::update`, and the local round in
//! `mode::Session` is untouched: online is a third driver beside Play and
//! Build, never a replacement for the in-process round.

use std::collections::BTreeSet;
use std::time::Instant;

use crate::ai::Intent;
use crate::net::apply;
use crate::net::client::{ClientEvent, Phase, RoomClient};
use crate::net::clock::{RttClock, RttReport};
use crate::net::interp::{InterpReport, Interpolator};
use crate::net::mailbox;
use crate::net::predict::{PredictionReport, Predictor};
use crate::net::transport::Transport;
use crate::net::events::WireEvent;
use crate::net::wire::{RoundOutcome, Snapshot, Welcome, dequantise_pos};
use crate::simulation::Game;
use crate::tank::Tank;
use crate::tuning::{self, tuning};
use crate::PHYSICS_FIXED_DT;

/// The online round the window holds: any transport, behind one type, so
/// `mode::Session` is the same struct whether the round is played over a
/// socket or over the rig's in-process link.
pub type AnyRound = OnlineRound<Box<dyn Transport>>;

/// How many snapshots the lead waits between two adjustments, and how
/// long a starvation counts as recent: sixty, a second at the room's
/// cadence. One packet's worth of lead per second is as fast as the loop
/// needs to move and slow enough that it cannot hunt.
pub const LEAD_WINDOW: u32 = 60;

/// A reported depth above this, smoothed, for two windows without a
/// starvation is more lead than the link needs, and one packet is
/// skipped to give it back. Two and a half: the depth a steady link
/// shows flickers between nought and one, so this is well clear of it.
pub const LEAD_DEPTH_MAX: f32 = 2.5;

/// How much of each reported depth the smoothed depth takes.
pub const LEAD_GAIN: f32 = 0.1;

/// The most ticks' worth of packets one rendered frame may send: a frame
/// that took longer than a tick owes more than one packet, and a client
/// drawing at 30 fps has to send sixty intents a second all the same -
/// or the room starves every other tick, repeats the last intent, and the
/// hull moves on ticks the sandbox never stepped, which arrives as a
/// correction on every snapshot. Four is `app::SIM_MAX_STEPS_PER_FRAME`'s
/// number: past it the frame was a stall and the time is dropped.
pub const SEND_CATCH_UP_TICKS: u32 = 4;

/// The most ticks the incoming fire is drawn ahead of where the room
/// has it (`draw_incoming_in_present`): half a second, past which a link
/// is not worth hiding.
pub const MAX_LEAD_TICKS: f32 = 30.0;

/// How long a foreign shot takes to catch up from where it is drawn in
/// the past to where it is drawn in the present, from the moment it is
/// first seen in flight: it leaves the drawn (past) muzzle and eases
/// forward, so it never appears ahead of the tank that fired it.
pub const CATCH_UP_MS: f32 = 120.0;

/// A drawn own hull that moved further than this since the last frame
/// jumped - a `Placed`, a portal, a correction taken whole - rather than
/// drove, and presses no tread marks across the gap: the distance
/// `net::apply` reads a jump by for the hulls it writes, half again a
/// cell, far past what a hull covers in a frame.
pub const OWN_JUMP_PX: f32 = 48.0;

/// What the lead asks of the frame's packet.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Adjust {
    /// One packet, as every tick.
    Keep,
    /// Two: the next tick's intent goes early, and the mailbox is one
    /// deeper from here on.
    Extra,
    /// None: the mailbox drains one, and the sandbox holds a tick.
    Skip,
}

/// The client's lead over the room, kept as the depth of its mailbox
/// (docs/online-coop-prd.md §4.12, "The lead").
///
/// Every snapshot reports how deep this seat's mailbox stood after the
/// tick's read and whether that read starved (`Snapshot::mailbox`). A
/// starvation means a packet arrived after the tick that wanted it: one
/// extra packet, once, and the buffer holds one more from then on. A
/// depth that sits high with no starvation in sight is lead the link is
/// not using, paid for in latency: one packet skipped, once. At most one
/// adjustment per `LEAD_WINDOW`.
#[derive(Clone, Copy, Debug)]
struct Lead {
    /// The reported depth, smoothed.
    depth: f32,
    /// Snapshots since the last reported starvation.
    since_starved: u32,
    /// Snapshots since the last adjustment.
    since_adjust: u32,
    /// Whether any reading has arrived at all.
    seen: bool,
    /// Adjustments made, for the report.
    up: u32,
    down: u32,
}

impl Lead {
    fn new() -> Lead {
        Lead { depth: 0.0, since_starved: u32::MAX, since_adjust: 0, seen: false, up: 0, down: 0 }
    }

    /// One snapshot's reading for this seat.
    fn observe(&mut self, state: u8) {
        let (depth, starved) = mailbox::unpack(state);
        self.seen = true;
        self.depth += (depth as f32 - self.depth) * LEAD_GAIN;
        self.since_starved = if starved { 0 } else { self.since_starved.saturating_add(1) };
        self.since_adjust = self.since_adjust.saturating_add(1);
    }

    /// What this tick's packet should be.
    fn decide(&mut self) -> Adjust {
        if !self.seen || self.since_adjust < LEAD_WINDOW {
            return Adjust::Keep;
        }
        if self.since_starved < LEAD_WINDOW {
            self.since_adjust = 0;
            // One starvation buys one packet; the next has to be a new one.
            self.since_starved = LEAD_WINDOW;
            self.up += 1;
            return Adjust::Extra;
        }
        if self.since_starved >= 2 * LEAD_WINDOW && self.depth > LEAD_DEPTH_MAX {
            self.since_adjust = 0;
            self.depth -= 1.0;
            self.down += 1;
            return Adjust::Skip;
        }
        Adjust::Keep
    }
}

/// A seat in a room, the replica it draws, and the clock between them.
pub struct OnlineRound<T: Transport> {
    client: RoomClient<T>,
    replica: Option<Game>,
    interp: Interpolator,
    /// What this round is called on screen: the room, or the rig.
    label: &'static str,
    /// The local clock the interpolator measures against.
    opened: Instant,
    /// The start has gone out; it is not asked for twice.
    start_sent: bool,
    /// Real seconds owed toward the next intent packet.
    send_owed: f32,
    /// A trigger pulled since the last packet went out, so a tap between
    /// two packets still reaches the room.
    pending_fire: bool,
    /// The frame's own trigger, for the flamethrower's cone: a stream
    /// the local seat is drawn holding while the key is down.
    trigger_down: bool,
    /// The seat's lead over the room, steered by the mailbox readings.
    lead: Lead,
    /// This client owns its hull (docs/online-coop-prd.md §4.14,
    /// `online_client_hull`): the sandbox drives it, every packet carries
    /// its pose, and the room follows; off, stage 2 predicts it.
    client_hull: bool,
    /// The measured round trip and the server's clock, from `Ping`s
    /// (`net::clock`, docs/online-coop-prd.md §4.15).
    rtt: RttClock,
    /// The tick of the world the last drawn frame showed and how far past
    /// it in 256ths: what every packet carries as `IntentMsg::view_tick`
    /// for lag compensation (§4.16).
    view: (u32, u8),
    /// When each foreign shot was first seen in flight, local ms: its
    /// catch-up into the present runs from there (`CATCH_UP_MS`).
    flying_since: std::collections::BTreeMap<u16, i64>,
    /// Foreign shots this client already drew meeting its own hull: kept
    /// off the picture until the room's copy goes.
    struck: std::collections::BTreeSet<u16>,
    /// The local seat's own hull, run ahead of the room and pulled back
    /// by each snapshot (stage 2, docs/online-coop-prd.md §4.12). Built
    /// beside the replica from the same `Welcome`, so the two step
    /// against the same statics. `None` until a welcome arrives, and
    /// read only while `online_predict_own_tank` is on - the knob is
    /// live, so turning it off hands the hull straight back to the
    /// interpolator without dropping the sandbox.
    predictor: Option<Predictor>,
    /// The last thing the room refused or the socket said on its way out.
    note: Option<String>,
    /// How the round the room just finished went, from the moment it
    /// says so until the next one starts. While it is set the window
    /// belongs in the lobby, not over the field.
    ended: Option<RoundOutcome>,
    /// Reused by `frame` so a frame allocates nothing.
    scratch: Vec<ClientEvent>,
    /// The tuning table as it stood before the room's first patch went
    /// on it, put back when the seat is given up (see `welcomed`).
    tuning_before: Option<tuning::Tuning>,
}

impl<T: Transport> OnlineRound<T> {
    /// A round over `client`, shown under `label`.
    ///
    /// The round itself is the room's to begin: a host asks for it with
    /// `start_round` once whoever is joining has a seat, and the rig's
    /// room - one seat, nobody to wait for - has already started by the
    /// time it answers the create.
    pub fn new(client: RoomClient<T>, label: &'static str) -> OnlineRound<T> {
        OnlineRound {
            client,
            replica: None,
            interp: Interpolator::default(),
            label,
            opened: Instant::now(),
            start_sent: false,
            predictor: None,
            send_owed: 0.0,
            pending_fire: false,
            trigger_down: false,
            lead: Lead::new(),
            client_hull: tuning().online_client_hull,
            rtt: RttClock::default(),
            view: (0, 0),
            flying_since: std::collections::BTreeMap::new(),
            struck: std::collections::BTreeSet::new(),
            note: None,
            ended: None,
            scratch: Vec::new(),
            tuning_before: None,
        }
    }

    /// One rendered frame: what arrived, what this seat is doing, and the
    /// picture to draw. `intent` is seat 0 of the frame's `Input` - the
    /// local player's, whichever seat the room gave them.
    pub fn frame(&mut self, intent: &Intent, dt: f32) {
        let now = self.local_ms();
        self.trigger_down = intent.fire;
        self.poll(now);
        if self.rtt.due(now) {
            self.client.ping(now as u32);
        }
        self.send(intent, dt);
        self.draw(dt, now);
    }

    /// The measured round trip and the server's clock (`net::clock`),
    /// once a probe has been answered.
    pub fn rtt(&self) -> Option<RttReport> {
        self.rtt.report()
    }

    /// Milliseconds since this round was opened: the clock every one of
    /// its stamps is read on, for a harness that wants to line its own
    /// samples up with them.
    pub fn local_clock_ms(&self) -> i64 {
        self.local_ms()
    }

    /// Whether this client owns its hull (stage 3) or predicts it (stage
    /// 2). Read from `online_client_hull` when the round is opened; a
    /// test sets it by hand.
    pub fn set_client_hull(&mut self, owned: bool) {
        self.client_hull = owned;
        if let Some(predictor) = self.predictor.as_mut() {
            predictor.set_owned(owned);
        }
    }

    /// Whether this client owns its hull.
    pub fn client_hull(&self) -> bool {
        self.client_hull
    }

    /// Where the room's newest snapshot has this seat's hull, in field
    /// pixels: what a test compares the owned hull against.
    pub fn room_has_seat_at(&self) -> Option<(f32, f32)> {
        let seat = self.client.seat()?;
        let t = self.interp.newest()?.tanks.iter().find(|t| t.id as u8 == seat)?;
        Some((dequantise_pos(t.x), dequantise_pos(t.y)))
    }

    /// Where this client has its own hull.
    pub fn own_hull_at(&self) -> Option<(f32, f32)> {
        let p = self.predictor.as_ref()?.pose()?;
        Some((p.position.x, p.position.y))
    }

    /// The predictor's counters, with the lead's own, once a welcome has
    /// built a sandbox (docs/online-coop-prd.md §4.12, "Measured").
    pub fn prediction(&self) -> Option<PredictionReport> {
        self.predictor.as_ref().map(|p| p.report(self.lead.up, self.lead.down))
    }

    /// What the interpolator is doing: the delay in force, the jitter it
    /// covers, the cadence, the frames it had to guess.
    pub fn interpolation(&self) -> InterpReport {
        self.interp.report()
    }

    /// The lead's smoothed reading of this seat's mailbox depth.
    pub fn lead_depth(&self) -> f32 {
        self.lead.depth
    }

    /// Everything a measurement wants from this round in one value: the
    /// link, the interpolator, the prediction, and the picture as this
    /// frame drew it - the own hull and every other tank, in field
    /// pixels, on this round's clock (docs/online-coop-prd.md §4.16,
    /// "Measurement first"). The dev server's `status.round` and the web
    /// build's `bb_net_stats` both read it, so a browser session against a
    /// deployed room is measured by the same numbers a native one is.
    pub fn stats_json(&self) -> serde_json::Value {
        use serde_json::json;
        let interp = self.interpolation();
        let tanks: Vec<serde_json::Value> = self
            .game()
            .map(|g| {
                g.tank_snapshots()
                    .into_iter()
                    .map(|t| json!([t.slot, (t.position.x * 4.0).round() / 4.0, (t.position.y * 4.0).round() / 4.0]))
                    .collect()
            })
            .unwrap_or_default();
        json!({
            "clock_ms": self.local_ms(),
            "seat": self.seat(),
            "client_hull": self.client_hull,
            "server_tick": self.interp.newest_tick(),
            "buffer_ms": self.buffer_ms(),
            "rtt": self.rtt().map(|r| json!({
                "rtt_ms": r.rtt_ms,
                "rtt_p95_ms": r.rtt_p95_ms,
                "rtt_min_ms": r.rtt_min_ms,
                "offset_ms": r.offset_ms,
                "samples": r.samples,
            })),
            "interpolation": interp_json(&interp),
            "prediction": self.prediction().map(|p| json!({
                "ignored": p.ignored,
                "nudges": p.nudges,
                "snaps": p.snaps,
                "error_buckets": p.error_buckets,
                "max_error_px": p.max_error_px,
                "shots_drawn": p.shots_drawn,
                "shots_refused": p.shots_refused,
                "shots_on_screen": p.shots_on_screen,
                "in_flight": p.in_flight,
                "cooldown": p.cooldown,
                "lead_up": p.lead_up,
                "lead_down": p.lead_down,
                "lead_depth": self.lead.depth,
                "crossings": p.crossings,
                "crossings_hit": p.crossings_hit,
                "crossings_missed": p.crossings_missed,
            })),
            "tanks": tanks,
        })
    }

    /// The replica, once a `Welcome` has built one. The window draws the
    /// local round until then.
    pub fn game(&self) -> Option<&Game> {
        self.replica.as_ref()
    }

    /// The replica, to set a *drawing* flag on - the dev server's overlay
    /// flags have to land on the `Game` that is drawn. The round's own
    /// state is the room's: nothing but `net::apply` ever writes it.
    pub fn game_mut(&mut self) -> Option<&mut Game> {
        self.replica.as_mut()
    }

    /// How far ahead of render time the newest snapshot is, in
    /// milliseconds: what the interpolation delay is buying, and negative
    /// once the picture has run past everything that arrived. `None`
    /// before the first snapshot.
    pub fn buffer_ms(&self) -> Option<f64> {
        self.interp.lead_ms(self.local_ms())
    }

    /// Ask the room to start the round (the host's to give, and a no-op
    /// for anyone else - the room refuses it by name).
    pub fn start_round(&mut self) {
        if self.start_sent || !self.client.is_host() {
            return;
        }
        self.start_sent = true;
        self.client.start();
    }

    /// Whether pressing start would mean anything: this client holds the
    /// room, the round has not begun, and every other seat has said it is
    /// ready - the room server's own three conditions, so the button is
    /// dead exactly when a press would come back refused.
    pub fn can_start(&self) -> bool {
        let seat = self.client.seat();
        self.client.is_host()
            && !self.start_sent
            && *self.client.phase() == Phase::Lobby
            && self.client.roster().iter().all(|s| s.ready || Some(s.seat) == seat)
    }

    /// Say this seat is ready for the round.
    pub fn ready(&mut self) {
        self.client.ready();
    }

    /// Put a seat out of the room (the host's to give; the room refuses
    /// it from anyone else, and refuses a host kicking themself).
    pub fn kick(&mut self, seat: u8) {
        self.client.kick(seat);
    }

    /// Every seat in the room, by seat number.
    pub fn roster(&self) -> &[crate::net::wire::RosterSeat] {
        self.client.roster()
    }

    /// The seat that holds the room.
    pub fn host_seat(&self) -> u8 {
        self.client.host_seat()
    }

    /// Whether this client holds the room.
    pub fn is_host(&self) -> bool {
        self.client.is_host()
    }

    /// The last thing the room refused or the socket said on its way out,
    /// which is what the lobby shows under the seats.
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// How the round the room just finished went, `None` while one is
    /// running or waiting to start. The room keeps ticking through the
    /// end screen and says this when the countdown has run out, so the
    /// banner has played by the time it is set: `mode::Session` reads it
    /// to hand the window back to the lobby, on this seat and this room.
    pub fn ended(&self) -> Option<RoundOutcome> {
        self.ended
    }

    /// Give up the seat and close the socket.
    pub fn leave(&mut self) {
        self.client.leave();
        self.client.close();
    }

    /// The seat this client was given, once the room has said.
    pub fn seat(&self) -> Option<u8> {
        self.client.seat()
    }

    /// The room's code, to read out to whoever is joining.
    pub fn code(&self) -> Option<&str> {
        self.client.code()
    }

    /// How far along the client is.
    pub fn phase(&self) -> &Phase {
        self.client.phase()
    }

    /// The snapshots waiting and the clock they are drawn against.
    pub fn interp(&self) -> &Interpolator {
        &self.interp
    }

    /// One line of chrome over the field while the round runs: the room,
    /// this seat, and how deep the snapshot buffer is. Everything before
    /// the round - the code, the QR, the seats, the buttons - is the
    /// lobby screen's (`lobby.rs`), which this line never repeats.
    pub fn status(&self) -> String {
        let code = self.client.code().unwrap_or("-----");
        let seat = self.client.seat().map_or_else(|| "-".to_string(), |s| format!("{}", s + 1));
        let mut line = match self.client.phase() {
            Phase::Connecting => format!("{} - CONNECTING", self.label),
            Phase::Greeting => format!("{} - ASKING FOR A SEAT", self.label),
            Phase::Lobby => format!("{} {code} - SEAT {seat} - IN THE LOBBY", self.label),
            Phase::Playing => {
                // The buffer's depth is what the delay is buying. Below
                // zero the picture has run past everything that arrived -
                // a stalled link, or a room that has stopped ticking
                // because the round is over - and a number would only
                // say how long ago that was.
                let rtt = self.rtt.report().map_or_else(String::new, |r| format!(" - PING {} MS", r.rtt_ms.round() as i64));
                match self.interp.lead_ms(self.local_ms()).unwrap_or(0.0).round() as i64 {
                    lead if lead >= 0 => format!("{} {code} - SEAT {seat}{rtt} - BUFFER {lead} MS", self.label),
                    _ => format!("{} {code} - SEAT {seat}{rtt} - WAITING FOR THE ROOM", self.label),
                }
            }
            Phase::Closed(closed) => format!("{} - OFFLINE: {closed}", self.label),
        };
        if let Some(note) = &self.note {
            line.push_str(" - ");
            line.push_str(note);
        }
        line
    }

    /// Local time in milliseconds since this round was opened: the clock
    /// the server's `server_ms` stamps are measured against.
    fn local_ms(&self) -> i64 {
        self.opened.elapsed().as_millis().min(i64::MAX as u128) as i64
    }

    /// An arrival instant on the same clock as `local_ms`.
    fn ms_at(&self, at: Instant) -> i64 {
        at.saturating_duration_since(self.opened).as_millis().min(i64::MAX as u128) as i64
    }

    /// Everything the room has said since the last frame.
    fn poll(&mut self, now: i64) {
        let mut events = std::mem::take(&mut self.scratch);
        events.clear();
        self.client.poll(&mut events);
        for event in events.drain(..) {
            match event {
                ClientEvent::Welcomed { welcome, arrived } => {
                    // The clock takes the welcome's reading at the instant
                    // it came off the socket, as it does a snapshot's.
                    let at = self.ms_at(arrived).min(now);
                    self.welcomed(&welcome, at);
                }
                ClientEvent::Snapshot { snapshot, arrived } => {
                    self.place_from(&snapshot);
                    self.reconcile(&snapshot);
                    self.note_fired(&snapshot);
                    if let Some(seat) = self.client.seat()
                        && let Some(&state) = snapshot.mailbox.get(seat as usize)
                    {
                        self.lead.observe(state);
                    }
                    // Stamped with when it came off the socket, not with
                    // this frame: the clock and the jitter reading are
                    // the link's, not the frame rate's (§4.15).
                    let at = self.ms_at(arrived).min(now);
                    self.interp.accept(*snapshot, at);
                }
                ClientEvent::Refused(message) => {
                    // A refused start is askable again: the room says why
                    // (a seat that is not ready), and the reason goes away.
                    self.start_sent = false;
                    self.note = Some(message);
                }
                ClientEvent::Closed(closed) => self.note = Some(closed.reason),
                ClientEvent::Ended { outcome } => {
                    self.ended = Some(outcome);
                    // The room is back in its lobby, so the rematch is
                    // the host's to ask for again.
                    self.start_sent = false;
                }
                // A round begun is the end screen cleared; the welcome
                // that follows builds the replica for it.
                ClientEvent::Started => self.ended = None,
                // The code and the roster are read back off the client
                // itself.
                ClientEvent::Created { .. } | ClientEvent::Roster { .. } => {}
                ClientEvent::Said { .. } => {}
                ClientEvent::Pong { client_ms, server_ms, arrived } => {
                    let at = self.ms_at(arrived).min(now);
                    self.rtt.observe(client_ms, server_ms, at);
                }
            }
        }
        self.scratch = events;
    }

    /// The room's round, as a replica.
    ///
    /// The room's tuning patch is staged and applied before the replica
    /// is built, because `Game::init` reads the knobs: a round is drawn
    /// with the numbers it is played with. A room of two or more sends
    /// the wave plan it sized to its team (docs/online-coop-prd.md
    /// section 4.11), so the table is the window's own again the moment
    /// the seat is given up - the local round behind the room is played
    /// with the build's numbers, never the room's.
    ///
    /// `arrived` is when the welcome came off the socket, on `local_ms`'s
    /// clock.
    fn welcomed(&mut self, welcome: &Welcome, arrived: i64) {
        let patch = welcome.tuning_json.trim();
        if !patch.is_empty() && patch != "{}" {
            if self.tuning_before.is_none() {
                self.tuning_before = Some(tuning::current());
            }
            match tuning::submit_json(patch) {
                Ok(_) => {
                    tuning::apply_pending();
                }
                Err(e) => self.note = Some(format!("the room's tuning was refused: {e}")),
            }
        }
        match apply::welcome(welcome) {
            Ok(game) => {
                // The sandbox is a second round off the same welcome, so
                // its walls, obstacles and deep water are the replica's
                // and the room's by construction (`net::predict`). Built
                // whether or not prediction is on: the knob is live, and
                // a sandbox that started late would have no history to
                // replay.
                self.predictor = apply::welcome(welcome).ok().map(|sandbox| {
                    let mut predictor = Predictor::new(sandbox, welcome.seat as usize, self.client.intent_tick());
                    predictor.set_owned(self.client_hull);
                    predictor
                });
                self.replica = Some(game);
                self.interp.set_seat(Some(welcome.seat));
                self.interp.restart(&welcome.snapshot, arrived);
                self.note = None;
            }
            Err(e) => self.note = Some(e),
        }
        // A fresh welcome is a fresh round: the next one has to be asked
        // for again.
        self.start_sent = *self.client.phase() == Phase::Playing;
    }

    /// This seat's intent, one packet per simulation tick of real time.
    ///
    /// A rendered frame is not a tick - a 120 Hz display has two frames
    /// per tick and a slow one none - and the room samples one intent per
    /// tick, so the packets are paced the way `app::StepClock` paces the
    /// steps of a local round. A trigger pulled on a frame that sends
    /// nothing rides along with the next packet, so a tap between two of
    /// them is never lost. The lead may make a tick's packet two, or
    /// none (`Lead`).
    fn send(&mut self, intent: &Intent, dt: f32) {
        self.pending_fire |= intent.fire;
        self.send_owed = (self.send_owed + dt.max(0.0)).min(SEND_CATCH_UP_TICKS as f32 * PHYSICS_FIXED_DT);
        while self.send_owed >= PHYSICS_FIXED_DT {
            self.send_owed -= PHYSICS_FIXED_DT;
            // An owned hull's pose is taken newest-wins by the room, so
            // there is no queue to steer: one packet and one sandbox tick
            // per tick of real time, never two and never none (§4.16).
            let packets = if self.client_hull {
                1
            } else {
                match self.lead.decide() {
                    Adjust::Keep => 1,
                    Adjust::Extra => 2,
                    Adjust::Skip => 0,
                }
            };
            for _ in 0..packets {
                self.send_one(intent);
            }
        }
    }

    /// One packet, and the sandbox's tick on it.
    fn send_one(&mut self, intent: &Intent) {
        let mut out = *intent;
        out.fire |= self.pending_fire;
        // One counter for both: the sandbox steps on exactly the input
        // the packet carries - fire hold included, since that is the bit
        // the server's press edge will see - stamped with exactly the
        // tick the server will name back in `acked`.
        let Some(msg) = self.client.prepare_intent(&out) else { return };
        let mut msg = msg.with_view(self.view.0, self.view.1);
        self.pending_fire = false;
        if let Some(predictor) = self.predictor.as_mut() {
            // The knob is live: read each tick, so a shot pressed after
            // it was turned on is drawn and one after it was turned off
            // is not.
            predictor.set_shots_enabled(tuning().online_predict_shots);
            predictor.step_at(msg.tick, msg.intent());
            // Owning the hull, the packet says where this tick left it,
            // and the room puts the seat there (§4.14).
            if self.client_hull && let Some(pose) = predictor.pose() {
                msg = msg.with_pose(pose);
            }
        }
        self.client.send_prepared(&msg);
    }

    /// The room moved this seat's hull itself: `Placed` (a refused pose,
    /// or a run of them), a portal's `Teleported`, or the seat coming
    /// back through a gate (`TankEntered`). An owned hull snaps to where
    /// the room says on arrival, since the room is not going to follow
    /// it there.
    fn place_from(&mut self, snapshot: &Snapshot) {
        if !self.client_hull {
            return;
        }
        let (Some(predictor), Some(seat)) = (self.predictor.as_mut(), self.client.seat()) else { return };
        for event in &snapshot.events {
            match *event {
                WireEvent::Placed { seat: s, x, y, dir } if s == seat => {
                    let rotation = crate::net::wire::dir_from_index(dir).unwrap_or(crate::tank::Dir::Up).rotation();
                    predictor.place_own(crate::math::Vec2::new(dequantise_pos(x), dequantise_pos(y)), rotation);
                }
                WireEvent::Teleported { slot, to_x, to_y, .. } if slot as u8 == seat => {
                    let rotation = predictor.pose().map_or(0.0, |p| p.rotation);
                    predictor.place_own(crate::math::Vec2::new(dequantise_pos(to_x), dequantise_pos(to_y)), rotation);
                }
                WireEvent::TankEntered { slot } if slot as u8 == seat => {
                    if let Some(t) = snapshot.tanks.iter().find(|t| t.id as u8 == seat) {
                        let rotation = crate::net::wire::dir_from_index(t.dir).unwrap_or(crate::tank::Dir::Up).rotation();
                        predictor.place_own(crate::math::Vec2::new(dequantise_pos(t.x), dequantise_pos(t.y)), rotation);
                    }
                }
                _ => {}
            }
        }
    }

    /// The picture at render time, then the cosmetics of one frame.
    ///
    /// In order (docs/online-coop-prd.md §4.16): the interpolated room is
    /// written into the replica; this seat's `Fired` confirm its presses;
    /// the room's copies of this seat's shots are paired with the
    /// provisionals and taken off the picture; the owned hull is drawn
    /// between its ticks; the provisionals fly one frame and stop at the
    /// first thing they meet in the drawn world; beams pressed since the
    /// last frame are drawn; everyone else's shots are carried forward to
    /// the present; then the frame's cosmetics tick.
    fn draw(&mut self, dt: f32, now: i64) {
        if self.replica.is_none() {
            return;
        }
        let sampled = self.interp.sample(now);
        let predicting_shots = tuning().online_predict_own_tank && tuning().online_predict_shots && self.predictor.is_some();
        if let Some(frame) = &sampled {
            let mut snapshot = frame.snapshot.clone();
            // A beam this seat drew on its press is not drawn twice.
            if predicting_shots && let Some(seat) = self.client.seat() {
                snapshot.events.retain(|e| !matches!(e, WireEvent::LaserBeam { seat: s, .. } if *s == seat));
            }
            if let Some(game) = self.replica.as_mut() {
                apply::snapshot(game, &snapshot);
            }
            let frac = (frame.ahead / PHYSICS_FIXED_DT * 256.0).clamp(0.0, 255.0) as u8;
            self.view = (frame.snapshot.tick, frac);
            self.confirm_shots(&frame.snapshot);
        }
        if predicting_shots {
            let delay = self.interp.delay_ms();
            let rtt = self.rtt.report().map_or(0.0, |r| r.rtt_p95_ms);
            if let Some(predictor) = self.predictor.as_mut() {
                predictor.set_refusal_after(((rtt + delay) / 1000.0) as f32 + REFUSAL_MARGIN_SECONDS);
            }
            self.pair_own_shots();
        }
        // **Before `tick_presentation`, not after.** The presentation
        // pass eases `visual_rotation` toward `rotation` and presses the
        // tread marks out of the hull's displacement, so it has to run on
        // the pose that will actually be drawn. Writing the prediction
        // after it left the rendered hull chasing the server's facing
        // from `online_interpolation_delay_ms` ago while its position was
        // already at the present - a tank that slides without turning.
        let own_drawn = self.write_predicted();
        // The world as it is drawn this frame, own hull in the present:
        // what the present-time shots are tested against.
        let world = self.replica.as_ref().map(Game::present_world);
        if let (Some(world), Some(seat)) = (&world, self.client.seat()) {
            self.fly_own_shots(dt, world, seat);
            if let Some(frame) = &sampled {
                self.draw_incoming_in_present(frame, now, world, seat);
            }
        }
        let seat = self.client.seat();
        let Some(game) = self.replica.as_mut() else { return };
        // A hull that jumped onto this frame - a portal, a placement - is
        // drawn there with no trail behind it: `tick_presentation` presses
        // tread marks along every hull's displacement since the last
        // frame, which for a jump is a line across the field. The own hull
        // drawn by the prediction jumps when its drawn pose does, not when
        // the interpolator reaches the room's copy of the jump.
        let mut jumped: BTreeSet<usize> =
            sampled.iter().flat_map(|frame| frame.snapped.iter().map(|&id| id as usize)).collect();
        if own_drawn && let Some(seat) = seat {
            jumped.remove(&(seat as usize));
            if own_hull_jumped(game, seat as usize) {
                jumped.insert(seat as usize);
            }
        }
        lift_trails(game, &jumped);
        game.tick_presentation(dt);
        if let Some(frame) = &sampled {
            // `apply` puts the clock on the snapshot's own tick; the
            // picture stands a fraction of an interval past it, and the
            // water, the fire loops and the sprite cycles read it.
            game.time = frame.snapshot.tick as f32 * PHYSICS_FIXED_DT + frame.ahead;
        }
        if let Some(predictor) = self.predictor.as_mut() {
            predictor.decay(dt);
        }
    }

    /// Put the predicted hull where the local seat is drawn.
    ///
    /// **The replica stays the one thing anything draws.** Rather than
    /// teach the renderer, the HUD and the dev server's readers about a
    /// second `Game`, the prediction is written over the one seat the
    /// interpolator has no business owning, on the frame it is drawn.
    /// Everything downstream is unchanged, and turning the knob off on
    /// any frame simply stops the write - the interpolated hull is
    /// already underneath it.
    ///
    /// An owned hull is drawn between its last two ticks by the time owed
    /// toward the next (`Predictor::drawn_pose`): the sandbox steps whole
    /// ticks, and a frame that ran none or two of them would otherwise
    /// show it standing or lurching.
    ///
    /// True when the seat was written: the drawn hull is the prediction's.
    fn write_predicted(&mut self) -> bool {
        if !tuning().online_predict_own_tank {
            return false;
        }
        let alpha = if self.client_hull { self.send_owed / PHYSICS_FIXED_DT } else { 1.0 };
        let (Some(predictor), Some(game)) = (self.predictor.as_ref(), self.replica.as_mut()) else { return false };
        let (Some(seat), Some((position, rotation))) = (self.client.seat(), predictor.drawn_pose(alpha)) else { return false };
        // The velocity is the prediction's own, not zero: `fx` reads it
        // for the spray and the dust, and a hull the solver believes is
        // stopped settles differently from one that is moving.
        let Some((_, _, velocity)) = predictor.motion() else { return false };
        // The replica's own boost flag is the server's and already
        // applied by `apply::snapshot`; the prediction only moves the
        // hull, so it is carried through unchanged.
        game.place_seat(seat as usize, position, rotation, velocity);
        // The flamethrower's stream is a flag the wire carries; while the
        // local key is down the seat is drawn streaming at once, if it is
        // armed and fuelled, rather than a round trip later. Released,
        // the flag is the server's again and goes out when its word does.
        if self.trigger_down && tuning().online_predict_shots {
            game.hold_flame(seat as usize);
        }
        true
    }

    /// This frame's room copies of this seat's shots: handed to the
    /// predictor to pair with its provisionals, then taken off the picture
    /// where a provisional stands for them - the provisional is the one
    /// drawn for the shot's whole life, on this client's timeline, and is
    /// never swapped for the room's copy in the past (§4.16).
    fn pair_own_shots(&mut self) {
        let (Some(predictor), Some(game), Some(seat)) = (self.predictor.as_mut(), self.replica.as_mut(), self.client.seat()) else { return };
        let own = game.seat_shots(seat);
        predictor.observe_server_shots(&own);
        game.remove_shots(&predictor.hidden_server_shots());
    }

    /// One frame of this seat's provisional shots against the drawn world:
    /// each flies by its own state machine and stops at the first tile,
    /// edge, tank or frog it meets, its impact drawn there at once; beams
    /// pressed since the last frame are drawn to where they stop.
    fn fly_own_shots(&mut self, dt: f32, world: &crate::simulation::present::PresentWorld, seat: u8) {
        use crate::simulation::present::{Contact, shot_half_extent};
        let (Some(predictor), Some(game)) = (self.predictor.as_mut(), self.replica.as_mut()) else { return };
        predictor.advance_shots(dt, |kind, from, to| {
            world
                .shot_contact(Some(seat), from, to, shot_half_extent(kind))
                .map(|(at, contact)| (at, matches!(contact, Contact::Tank { .. } | Contact::Frog)))
        });
        for beam in predictor.take_beams() {
            let far = crate::math::Vec2::new(beam.start.x + beam.dir.x * LASER_REACH_PX, beam.start.y + beam.dir.y * LASER_REACH_PX);
            let end = world
                .shot_contact(Some(seat), beam.start, far, tuning().shell_hit_half_extent)
                .map_or(far, |(at, _)| at);
            game.draw_beam(beam.start, end, beam.variant);
        }
        for at in predictor.take_impacts() {
            game.draw_impact(at);
        }
        // Beside the room's shots, not instead of them: `apply` has just
        // despawned everything the snapshot did not list, so these are
        // put back every frame for as long as they live.
        for (id, shot) in predictor.shots() {
            game.add_provisional_shot(id, &shot);
        }
    }

    /// Everyone else's shots, carried from where the room had them at
    /// render time to where they will be when this client's present pose
    /// reaches the room - the moment the room decides whether they hit it
    /// (§4.16, "Incoming fire in the present").
    ///
    /// That lead is exact to the tick: the newest snapshot says the room
    /// applied this seat's input `acked` on its tick `tick`, one input per
    /// tick, so this client's latest input lands on room tick
    /// `tick + (latest - acked)`; the picture stands at render tick `R`,
    /// so a straight shot is `(that - R)` ticks of flight ahead. A shot
    /// first seen in flight eases from its drawn place to the full lead
    /// over `CATCH_UP_MS`, so it leaves the (past) muzzle it came from
    /// rather than appearing ahead of it. It stops at walls; one that
    /// reaches this seat's drawn hull shows its impact there at once and
    /// is kept off the picture until the room's copy goes - the damage is
    /// the room's, and arrives with its `Hit`.
    fn draw_incoming_in_present(&mut self, frame: &crate::net::interp::Frame, now: i64, world: &crate::simulation::present::PresentWorld, seat: u8) {
        use crate::simulation::present::segment_box;
        let (Some(predictor), Some(newest)) = (self.predictor.as_ref(), self.interp.newest()) else { return };
        let acked = newest.acked.get(seat as usize).copied().unwrap_or(0);
        if acked == 0 {
            return;
        }
        let render = frame.snapshot.tick as f64 + (frame.ahead / PHYSICS_FIXED_DT) as f64;
        let lead_ticks = incoming_lead_ticks(newest.tick, acked, predictor.tick().wrapping_sub(1), render);
        let hull = world.seat_hull(seat);
        let Some(game) = self.replica.as_mut() else { return };
        let foreign = game.foreign_flying_shots(seat);
        let alive: std::collections::BTreeSet<u16> = foreign.iter().map(|s| s.id).collect();
        self.flying_since.retain(|id, _| alive.contains(id));
        let gone: Vec<u16> = self.struck.iter().copied().filter(|id| !game.has_shot(*id)).collect();
        for id in gone {
            self.struck.remove(&id);
        }
        let mut struck_now = Vec::new();
        for shot in foreign {
            if self.struck.contains(&shot.id) {
                continue;
            }
            let since = *self.flying_since.entry(shot.id).or_insert(now);
            let catch = smoothstep(((now - since) as f32 / CATCH_UP_MS).clamp(0.0, 1.0));
            let ahead = lead_ticks * PHYSICS_FIXED_DT * catch;
            let mut to = crate::math::Vec2::new(shot.position.x + shot.velocity.x * ahead, shot.position.y + shot.velocity.y * ahead);
            if let Some(stop) = world.static_contact(shot.position, to) {
                to = stop;
            }
            if let Some((centre, half)) = hull {
                let grow = crate::math::Vec2::new(shot.half_extent, shot.half_extent);
                if let Some(t) = segment_box(shot.position, to, centre, half + grow) {
                    let at = crate::math::Vec2::new(
                        shot.position.x + (to.x - shot.position.x) * t,
                        shot.position.y + (to.y - shot.position.y) * t,
                    );
                    game.draw_impact(at);
                    struck_now.push(shot.id);
                    continue;
                }
            }
            game.move_shot(shot.id, to);
        }
        game.remove_shots(&struck_now);
        self.struck.extend(struck_now);
        let hidden: Vec<u16> = self.struck.iter().copied().collect();
        game.remove_shots(&hidden);
    }

    /// Pull the sandbox back into line with a snapshot that just landed.
    ///
    /// The whole snapshot goes in, not this seat's hull picked out of it:
    /// the sandbox is a projection of the server's world, so everything
    /// the prediction steps against - other hulls, destroyed walls, a
    /// speed boost - arrives by the same path the replica takes
    /// (`net::predict::Predictor::reconcile`).
    fn reconcile(&mut self, snapshot: &Snapshot) {
        if !tuning().online_predict_own_tank {
            return;
        }
        let (Some(predictor), Some(seat)) = (self.predictor.as_mut(), self.client.seat()) else { return };
        let acked = snapshot.acked.get(seat as usize).copied().unwrap_or(0);
        // A server that has applied nothing for this seat yet has nothing
        // to reconcile against - the hull is still where it spawned.
        if acked == 0 {
            return;
        }
        predictor.reconcile(snapshot, acked);
    }

    /// A snapshot *arrived*: this seat's `Fired` in it put their presses'
    /// ammo in the snapshot the sandbox just took, so they stop counting
    /// against the local gate (`Predictor::note_fired`); a `Shoved` for this
    /// seat is applied to the owned hull now, so the next pose carries it.
    /// The shots themselves are confirmed later, when the frame reaches
    /// them.
    fn note_fired(&mut self, snapshot: &Snapshot) {
        if !tuning().online_predict_own_tank {
            return;
        }
        let (Some(predictor), Some(seat)) = (self.predictor.as_mut(), self.client.seat()) else { return };
        for event in &snapshot.events {
            match *event {
                WireEvent::Fired { slot, weapon, input_tick } if slot as u8 == seat => predictor.note_fired(weapon, input_tick),
                WireEvent::Shoved { seat: s, vx, vy } if s == seat && self.client_hull => {
                    predictor.shove(crate::math::Vec2::new(
                        crate::net::wire::dequantise_velocity(vx),
                        crate::net::wire::dequantise_velocity(vy),
                    ));
                }
                _ => {}
            }
        }
    }

    /// The interpolator handed a snapshot's events over: each `Fired` of
    /// this seat's confirms the press that travelled on its input tick
    /// (`Predictor::confirm_fired`), and one with no press waiting seeds
    /// the local gate from the room's.
    fn confirm_shots(&mut self, frame: &Snapshot) {
        if !tuning().online_predict_own_tank {
            return;
        }
        let (Some(predictor), Some(seat)) = (self.predictor.as_mut(), self.client.seat()) else { return };
        for event in &frame.events {
            if let WireEvent::Fired { slot, weapon, input_tick } = *event
                && slot as u8 == seat
            {
                predictor.confirm_fired(weapon, input_tick);
            }
        }
    }
}

/// Lift the tread-mark trail of every tank whose owner slot is in `slots`
/// and put its ring follower under the hull, as `net::apply` does for a
/// hull it moves further than driving would: the next `tick_presentation`
/// starts the trail again from where the hull landed instead of pressing
/// marks across the jump, and the ring does not glide over after it.
fn lift_trails(game: &mut Game, slots: &BTreeSet<usize>) {
    if slots.is_empty() {
        return;
    }
    for tank in game.world.query::<&mut Tank>().iter() {
        if slots.contains(&tank.owner_slot()) {
            tank.track_from = None;
            tank.ring_position = tank.position;
            tank.ring_velocity = crate::math::Vec2::new(0.0, 0.0);
        }
    }
}

/// Whether the seat's drawn hull moved further than `OWN_JUMP_PX` since
/// the last `tick_presentation` put its trail down.
fn own_hull_jumped(game: &Game, seat: usize) -> bool {
    let Some(entity) = game.seats.get(seat).copied().flatten() else { return false };
    let Ok(tank) = game.world.get::<&Tank>(entity) else { return false };
    tank.track_from.is_some_and(|from| from.distance_to(tank.position) > OWN_JUMP_PX)
}

/// Every reading of the interpolator, as `stats_json` carries it. The
/// report is taken apart field by field with no rest pattern, so a reading
/// added to `InterpReport` does not compile until it is carried here too.
fn interp_json(report: &InterpReport) -> serde_json::Value {
    let InterpReport {
        delay_ms,
        target_ms,
        jitter_ms,
        interval_ms,
        buffered,
        extrapolated_frames,
        lateness_p50_ms,
        lateness_p95_ms,
        stalls,
        rate,
        corrections,
        error_p95_px,
        stale_events,
    } = *report;
    serde_json::json!({
        "delay_ms": delay_ms,
        "target_ms": target_ms,
        "jitter_ms": jitter_ms,
        "interval_ms": interval_ms,
        "buffered": buffered,
        "extrapolated_frames": extrapolated_frames,
        "lateness_p50_ms": lateness_p50_ms,
        "lateness_p95_ms": lateness_p95_ms,
        "stalls": stalls,
        "rate": rate,
        "corrections": corrections,
        "error_p95_px": error_p95_px,
        "stale_events": stale_events,
    })
}

/// How many ticks ahead of the picture a straight shot is drawn so it
/// stands where it will be when this client's latest input lands on the
/// room's clock (`OnlineRound::draw_incoming_in_present`): the newest
/// snapshot `newest_tick` applied this seat's input `acked`, the room
/// applies one input per tick, so input `latest` lands on
/// `newest_tick + (latest - acked)`; the picture stands at `render_tick`.
pub fn incoming_lead_ticks(newest_tick: u32, acked: u32, latest: u32, render_tick: f64) -> f32 {
    let lands = newest_tick as f64 + (latest as i64 - acked as i64) as f64;
    ((lands - render_tick) as f32).clamp(0.0, MAX_LEAD_TICKS)
}

/// `x` eased in and out: 0 at 0, 1 at 1, flat at both ends.
fn smoothstep(x: f32) -> f32 {
    x * x * (3.0 - 2.0 * x)
}

/// What a press's refusal timeout adds to the link's round trip and
/// picture delay: a few ticks of the room's own scheduling.
const REFUSAL_MARGIN_SECONDS: f32 = 0.15;

/// How far a predicted beam is traced before it stops at the first thing
/// it meets: longer than any field's diagonal, as `weapons` traces it.
const LASER_REACH_PX: f32 = 4000.0;

/// Giving the seat up puts the tuning table back where the room found
/// it, so the local round the window comes back to is played with the
/// build's numbers rather than the room's team-sized wave plan.
impl<T: Transport> Drop for OnlineRound<T> {
    fn drop(&mut self) {
        if let Some(before) = self.tuning_before.take() {
            tuning::replace_now(before);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::MapFile;
    use crate::net::client::{Identity, RoomSetup};
    use crate::net::codec::{self, Msg};
    use crate::net::encode;
    use crate::net::loopback::{self, LinkQuality};
    use crate::net::wire::{Lobby, Seat, Snapshot};
    use crate::net::{MAX_SEATS, PROTOCOL_VERSION};
    use crate::simulation::Input;
    use crate::tank::Dir;

    const DEFAULT_MAP: &str = include_str!("../../maps/default.toml");

    /// A round as a room server would run one: a band of three at a
    /// pinned seed.
    fn authority() -> Game {
        let mut game = Game::default();
        game.seed_override = Some(0xB0B5);
        game.enemy_count_override = Some(3);
        game.player_row_override = Some(3);
        game.level_overrides.spawn = Some(crate::level::SpawnKind::Band);
        game.map = MapFile::from_toml_str(DEFAULT_MAP).expect("the default map parses");
        let (w, h) = game.map.field_size();
        game.init(w, h);
        game
    }

    /// The room's end of a loopback link, answered by hand.
    struct Room {
        link: loopback::Loopback,
        game: Game,
        server_ms: u32,
    }

    impl Room {
        fn say(&mut self, msg: Msg) {
            self.link.send(&codec::encode(&msg));
        }

        fn welcome(&mut self) {
            let roster = vec![Seat { seat: 0, nick: "host".into(), chassis: 3 }];
            let mut welcome =
                encode::welcome(&self.game, 0, roster, "{}".into(), [0; MAX_SEATS]).expect("the map serialises");
            welcome.protocol = PROTOCOL_VERSION;
            welcome.snapshot.server_ms = self.server_ms;
            self.say(Msg::Lobby(Lobby::RoomCreated { code: "AK7QX".into() }));
            self.say(Msg::Lobby(Lobby::Started));
            self.say(Msg::Welcome(welcome));
        }

        /// Three ticks of the round, then the snapshot they earned.
        fn tick(&mut self, intent: Intent) -> Snapshot {
            let (w, h) = self.game.map.field_size();
            for _ in 0..3 {
                self.game.update(Input::single(intent), PHYSICS_FIXED_DT, w, h);
            }
            self.server_ms += 50;
            let mut snapshot = encode::snapshot(&self.game, [0; MAX_SEATS]);
            snapshot.server_ms = self.server_ms;
            self.say(Msg::Snapshot(snapshot.clone()));
            snapshot
        }

        fn heard(&mut self) -> Vec<Msg> {
            let mut out = Vec::new();
            self.link.drain(&mut out);
            out
        }
    }

    fn room_and_round() -> (Room, OnlineRound<loopback::Loopback>) {
        let (client_end, room_end) = loopback::pair(LinkQuality::PERFECT, 11);
        let room = Room { link: room_end, game: authority(), server_ms: 1_000 };
        let client =
            RoomClient::host(client_end, Identity::new("host", "tok"), RoomSetup { seed: Some(0xB0B5), ..RoomSetup::default() });
        (room, OnlineRound::new(client, "RIG"))
    }

    #[test]
    fn a_welcome_builds_the_replica_and_a_snapshot_draws_it() {
        let (mut room, mut round) = room_and_round();
        round.frame(&Intent::default(), 1.0 / 60.0);
        assert!(round.game().is_none(), "nothing to draw before the room has answered");
        room.welcome();
        round.frame(&Intent::default(), 1.0 / 60.0);
        let replica = round.game().expect("the welcome built a replica");
        assert_eq!(replica.frame(), room.game.frame());
        assert_eq!(
            replica.drawable_state(),
            room.game.drawable_state(),
            "the welcome's own snapshot is the round as it stands"
        );
        assert_eq!(round.seat(), Some(0));
        assert_eq!(round.code(), Some("AK7QX"));
    }

    #[test]
    fn the_seats_intent_goes_out_once_per_tick_and_a_tap_is_never_lost() {
        let (mut room, mut round) = room_and_round();
        room.welcome();
        round.frame(&Intent::default(), 1.0 / 60.0);
        room.heard();
        // Two 120 Hz frames pay for one packet, and the tap on the frame
        // that sends nothing rides along with it.
        let tap = Intent { move_dir: Some(Dir::Up), fire: true, ..Intent::default() };
        round.frame(&tap, 1.0 / 120.0);
        assert!(room.heard().is_empty(), "half a tick owes no packet");
        round.frame(&Intent { move_dir: Some(Dir::Up), ..Intent::default() }, 1.0 / 120.0);
        let heard = room.heard();
        assert_eq!(heard.len(), 1, "one packet a tick: {heard:?}");
        let Msg::Intent(sent) = &heard[0] else { panic!("not an intent: {heard:?}") };
        assert!(sent.fire, "the tap from the frame in between");
        assert_eq!(sent.move_dir, 1);
    }

    /// A slow frame owes the room every tick it covered: a client at 20
    /// fps still sends sixty intents a second, and its sandbox steps
    /// sixty times, so the room never starves and the prediction never
    /// falls behind the hull the room moved on repeated intents.
    #[test]
    fn a_slow_frame_sends_every_tick_it_covered() {
        let (mut room, mut round) = room_and_round();
        room.welcome();
        round.frame(&Intent::default(), 1.0 / 60.0);
        room.heard();
        let drive = Intent { move_dir: Some(Dir::Up), ..Intent::default() };
        // A 20 fps frame: three ticks' worth, and a hair over so the
        // accumulator's float arithmetic cannot land a hair under.
        round.frame(&drive, 3.0 * PHYSICS_FIXED_DT + 1e-4);
        let sent = room.heard().iter().filter(|m| matches!(m, Msg::Intent(_))).count();
        assert_eq!(sent, 3, "three ticks of frame, three packets");
        assert_eq!(round.prediction().map(|p| p.in_flight), Some(0), "nothing acked yet, nothing reconciled");
        // A stall is dropped past `SEND_CATCH_UP_TICKS`, as `app::StepClock` drops it.
        round.frame(&drive, 1.0);
        let sent = room.heard().iter().filter(|m| matches!(m, Msg::Intent(_))).count();
        assert_eq!(sent as u32, SEND_CATCH_UP_TICKS, "a second-long stall does not send sixty packets");
    }

    #[test]
    fn the_replica_draws_between_the_snapshots_and_never_simulates() {
        let (mut room, mut round) = room_and_round();
        room.welcome();
        round.frame(&Intent::default(), 1.0 / 60.0);
        let drive = Intent { move_dir: Some(Dir::Right), ..Intent::default() };
        // Enough snapshots that render time is well inside the buffer.
        let mut snapshots = Vec::new();
        for _ in 0..12 {
            snapshots.push(room.tick(drive));
        }
        // The replica's frame only ever moves in snapshot steps, and the
        // round it draws is the room's, not one of its own.
        let mut frames = Vec::new();
        for _ in 0..30 {
            round.frame(&drive, 1.0 / 60.0);
            frames.push(round.game().expect("a replica").frame());
        }
        assert!(frames.windows(2).all(|w| w[1] >= w[0]), "the replica's clock never goes back: {frames:?}");
        let newest = snapshots.last().expect("a snapshot").tick as u64;
        assert!(frames.iter().all(|&f| f <= newest), "the replica is never ahead of the room");
        assert!(
            frames.iter().any(|&f| f > 0),
            "the replica reached a snapshot after the welcome's: {frames:?}"
        );
        // Every drawn hull is one the room sent, at a position the room
        // sent or between two of them.
        let drawn = round.game().expect("a replica").drawable_state();
        let slots: Vec<usize> = drawn.tanks.iter().map(|t| t.slot).collect();
        let room_slots: Vec<usize> = room.game.drawable_state().tanks.iter().map(|t| t.slot).collect();
        assert_eq!(slots, room_slots, "the same hulls, by slot");
    }

    /// **The lead is a depth** (docs/online-coop-prd.md §4.12): a room
    /// that reports a starvation gets one extra packet, once, and the
    /// client's tick runs one further ahead from then on; a room that
    /// reports a deep buffer with no starvation for two windows gets one
    /// packet fewer, once.
    #[test]
    fn a_starvation_widens_the_lead_by_one_packet_and_a_deep_buffer_narrows_it() {
        let (mut room, mut round) = room_and_round();
        // The lead steers the mailbox of a predicted (stage 2) seat; an
        // owned hull's poses are taken newest-wins and need none.
        round.set_client_hull(false);
        room.welcome();
        round.frame(&Intent::default(), 1.0 / 60.0);
        room.heard();
        let sent = |room: &mut Room| room.heard().iter().filter(|m| matches!(m, Msg::Intent(_))).count();

        // A window of quiet snapshots: one packet a tick, as always.
        for _ in 0..LEAD_WINDOW {
            room.tick(Intent::default());
            round.frame(&Intent::default(), 1.0 / 60.0);
        }
        assert_eq!(sent(&mut room), LEAD_WINDOW as usize, "one packet per tick on a quiet link");

        // The room starved once. The next packet is two, and only once.
        let mut starved = encode::snapshot(&room.game, [0; MAX_SEATS]);
        starved.server_ms = room.server_ms + 1;
        starved.mailbox[0] = mailbox::STARVED_BIT;
        room.say(Msg::Snapshot(starved));
        let before = round.prediction().map(|p| p.lead_up).unwrap_or(0);
        round.frame(&Intent::default(), 1.0 / 60.0);
        assert_eq!(sent(&mut room), 2, "the starvation bought one extra packet");
        assert_eq!(round.prediction().map(|p| p.lead_up), Some(before + 1));
        for _ in 0..10 {
            room.tick(Intent::default());
            round.frame(&Intent::default(), 1.0 / 60.0);
        }
        assert_eq!(sent(&mut room), 10, "and no more than one");

        // The buffer sits four deep with no starvation for two windows:
        // a packet is skipped to give the lead back - one per window
        // for as long as the room keeps reporting it deep, and never
        // two in one.
        let frames = 2 * LEAD_WINDOW + 5;
        for _ in 0..frames {
            let mut deep = encode::snapshot(&room.game, [0; MAX_SEATS]);
            room.server_ms += 16;
            deep.server_ms = room.server_ms;
            deep.mailbox[0] = 4;
            room.say(Msg::Snapshot(deep));
            round.frame(&Intent::default(), 1.0 / 60.0);
        }
        let heard = sent(&mut room) as u32;
        let down = round.prediction().map(|p| p.lead_down).expect("a sandbox");
        assert!(down >= 1, "a deep buffer was never narrowed");
        assert!(down <= frames / LEAD_WINDOW, "narrowed {down} times in {frames} snapshots");
        assert_eq!(heard, frames - down, "each narrowing is exactly one packet fewer");
    }

    /// **Stage 3 on the wire** (docs/online-coop-prd.md §4.14): with the
    /// client owning its hull, every packet carries where the hull is,
    /// a snapshot that has it somewhere older pulls nothing back, and a
    /// `Placed` from the room snaps it.
    #[test]
    fn an_owned_hull_travels_on_the_intent_and_only_a_placed_moves_it() {
        use crate::net::wire::quantise_pos;
        let (mut room, mut round) = room_and_round();
        round.set_client_hull(true);
        room.welcome();
        round.frame(&Intent::default(), 1.0 / 60.0);
        room.heard();
        let start = round.own_hull_at().expect("a hull");
        let drive = Intent { move_dir: Some(Dir::Right), ..Intent::default() };
        for _ in 0..30 {
            round.frame(&drive, 1.0 / 60.0);
        }
        let heard = room.heard();
        let packets: Vec<crate::net::wire::IntentMsg> =
            heard.iter().filter_map(|m| if let Msg::Intent(i) = m { Some(*i) } else { None }).collect();
        assert_eq!(packets.len(), 30);
        assert!(packets.iter().all(|p| p.owned), "every packet owns the hull");
        let last = packets.last().expect("a packet").pose().expect("a pose");
        let mine = round.own_hull_at().expect("a hull");
        assert!(mine.0 > start.0 + 20.0, "the hull drove right: {start:?} -> {mine:?}");
        assert!((last.position.x - mine.0).abs() < 0.5, "the packet says where the hull is: {last:?} vs {mine:?}");

        // The room's snapshot still has the hull at the start, a round
        // trip behind: an owned hull is not pulled back to it.
        let mut lagging = encode::snapshot(&room.game, [0; MAX_SEATS]);
        lagging.server_ms = room.server_ms + 50;
        lagging.acked[0] = 20;
        room.say(Msg::Snapshot(lagging));
        round.frame(&drive, 1.0 / 60.0);
        let after = round.own_hull_at().expect("a hull");
        assert!(after.0 >= mine.0, "the lagging snapshot pulled the hull back: {mine:?} -> {after:?}");
        assert_eq!(round.prediction().map(|p| (p.nudges, p.snaps)), Some((0, 0)));

        // The room moved it itself: `Placed` is obeyed, whole.
        let there = (start.0 - 96.0, start.1);
        let mut placed = encode::snapshot(&room.game, [0; MAX_SEATS]);
        placed.tick = 2;
        placed.server_ms = room.server_ms + 100;
        placed.events = vec![WireEvent::Placed { seat: 0, x: quantise_pos(there.0), y: quantise_pos(there.1), dir: 0 }];
        room.say(Msg::Snapshot(placed));
        round.frame(&Intent::default(), 1.0 / 60.0);
        let snapped = round.own_hull_at().expect("a hull");
        assert!((snapped.0 - there.0).abs() < 0.5 && (snapped.1 - there.1).abs() < 0.5, "not placed: {snapped:?} vs {there:?}");
    }

    /// An owned hull the room places elsewhere is drawn there with no
    /// trail behind it: the drawn pose jumped, so its tread marks start
    /// again where it landed rather than being pressed along the gap.
    #[test]
    fn an_owned_hull_placed_by_the_room_presses_no_tread_marks_across_the_jump() {
        use crate::net::wire::quantise_pos;
        if !tuning().online_predict_own_tank {
            return;
        }
        let (mut room, mut round) = room_and_round();
        round.set_client_hull(true);
        room.welcome();
        for _ in 0..3 {
            round.frame(&Intent::default(), 1.0 / 60.0);
        }
        let start = round.own_hull_at().expect("a hull");
        let tracks_before = round.game().expect("a replica").tracks.len();
        let there = (start.0 - 96.0, start.1);
        let mut placed = encode::snapshot(&room.game, [0; MAX_SEATS]);
        placed.tick = 2;
        placed.server_ms = room.server_ms + 33;
        placed.events = vec![WireEvent::Placed { seat: 0, x: quantise_pos(there.0), y: quantise_pos(there.1), dir: 0 }];
        room.say(Msg::Snapshot(placed));
        for _ in 0..3 {
            round.frame(&Intent::default(), 1.0 / 60.0);
        }
        let landed = round.own_hull_at().expect("a hull");
        assert!((landed.0 - there.0).abs() < 0.5, "not placed: {landed:?} vs {there:?}");
        assert_eq!(round.game().expect("a replica").tracks.len(), tracks_before, "tread marks were pressed across the placement");
    }

    /// **A shot stops where it is seen to hit** (§4.16): a provisional
    /// shell fired at an enemy drawn 90 px ahead (inside the clear stretch
    /// of the default map's lane - a wall stands 80 px beyond) stops at
    /// that enemy's near face and plays its impact there at once, and is
    /// counted as a tank hit this client drew - it never sails on through
    /// the hull.
    #[test]
    fn a_provisional_shot_stops_at_the_enemy_it_meets() {
        if !tuning().online_predict_own_tank || !tuning().online_predict_shots {
            return;
        }
        let (mut room, mut round) = room_and_round();
        room.welcome();
        round.frame(&Intent::default(), 1.0 / 60.0);
        let seat = room.game.tank_snapshots().into_iter().find(|t| t.slot == 0).expect("the seat");
        let rad = seat.rotation.to_radians();
        let ahead = crate::math::Vec2::new(seat.position.x + rad.sin() * 90.0, seat.position.y - rad.cos() * 90.0);
        room.game.debug_teleport(1, ahead, Some(seat.rotation)).expect("an enemy in slot 1");
        // Twenty ticks of that world, stamped on the room's schedule:
        // render time stands the delay behind the newest tick, which has to
        // be one of the teleported world's rather than the welcome's.
        for i in 0..20u32 {
            let mut s = encode::snapshot(&room.game, [0; MAX_SEATS]);
            s.tick = i + 1;
            s.server_ms = 5_000 + (i as f64 * 1000.0 / 60.0).round() as u32;
            room.say(Msg::Snapshot(s));
        }
        round.frame(&Intent::default(), 1.0 / 60.0);
        room.heard();
        round.frame(&Intent { fire: true, ..Intent::default() }, 1.0 / 60.0);
        // The room fires it: a `Fired` naming the press's input tick.
        let pressed = room
            .heard()
            .iter()
            .find_map(|m| if let Msg::Intent(i) = m { i.fire.then_some(i.tick) } else { None })
            .expect("the press went out");
        let mut fired = encode::snapshot(&room.game, [0; MAX_SEATS]);
        fired.tick = 21;
        fired.server_ms = 5_000 + (20.0 * 1000.0 / 60.0f64).round() as u32;
        fired.events = vec![WireEvent::Fired { slot: 0, weapon: crate::net::wire::WeaponKind::Shell, input_tick: pressed }];
        room.say(Msg::Snapshot(fired));
        let mut furthest = 0.0f32;
        for _ in 0..60 {
            round.frame(&Intent::default(), 1.0 / 60.0);
            let game = round.game().expect("a replica");

            for shot in game.drawable_state().shots {
                let (x, y) = (shot.x as f32 / 4.0, shot.y as f32 / 4.0);
                furthest = furthest.max((x - seat.position.x) * rad.sin() - (y - seat.position.y) * rad.cos());
            }
        }
        let report = round.prediction().expect("a report");
        assert_eq!(report.crossings, 1, "the shell met the enemy in this client's picture: {report:?}");
        assert!(furthest < 90.0, "the shell was drawn {furthest} px out - past the enemy at 90");
    }

    /// The incoming-fire lead is the gap between where this client's
    /// latest pose will land on the room's clock and the tick the picture
    /// stands at: the newest snapshot's tick plus the inputs sent since the
    /// one it acknowledges, less the render tick.
    #[test]
    fn the_incoming_lead_is_where_the_latest_pose_lands_less_the_picture() {
        // The room applied input 100 on its tick 500; this client has
        // since sent input 106; the picture stands at tick 497.5.
        assert_eq!(incoming_lead_ticks(500, 100, 106, 497.5), 8.5);
        // Never negative, never past the cap.
        assert_eq!(incoming_lead_ticks(500, 100, 100, 501.0), 0.0);
        assert_eq!(incoming_lead_ticks(500, 100, 400, 10.0), MAX_LEAD_TICKS);
    }

    #[test]
    fn a_refusal_and_a_close_are_kept_for_the_status_line() {
        let (mut room, mut round) = room_and_round();
        round.frame(&Intent::default(), 1.0 / 60.0);
        room.say(Msg::Lobby(Lobby::Error { message: "the room is full".into() }));
        round.frame(&Intent::default(), 1.0 / 60.0);
        assert!(round.status().contains("the room is full"), "{}", round.status());
        room.link.close();
        // The link hands over what is in flight before it reports the end.
        for _ in 0..3 {
            round.frame(&Intent::default(), 1.0 / 60.0);
        }
        assert!(matches!(round.phase(), Phase::Closed(_)), "{:?}", round.phase());
        assert!(round.status().contains("OFFLINE"), "{}", round.status());
    }

    /// The clock reads a welcome at the instant it came off the socket, as
    /// it reads every snapshot - not at the frame that got round to it.
    #[test]
    fn a_welcome_is_read_at_its_arrival_not_at_the_frame_that_polls_it() {
        let (mut room, mut round) = room_and_round();
        round.frame(&Intent::default(), 1.0 / 60.0);
        // Encoding the welcome takes a moment; it goes on the link at the
        // end of it, and a perfect link has it readable at once.
        let before = round.local_clock_ms();
        room.welcome();
        let after = round.local_clock_ms();
        std::thread::sleep(std::time::Duration::from_millis(120));
        round.frame(&Intent::default(), 1.0 / 60.0);
        assert!(round.game().is_some(), "the welcome built a replica");
        // A reading is the tick's time less the arrival: the arrival it
        // was taken at is the tick's time less the estimate.
        let tick = room.game.frame() as u32;
        let offset = round.interp().clock().offset_ms().expect("the welcome was read");
        let arrived = crate::net::interp::tick_ms(tick) - offset;
        assert!(
            (before as f64 - 1.0..=after as f64 + 1.0).contains(&arrived),
            "the welcome was read as arriving at {arrived} ms; it came off the socket between {before} and {after} ms"
        );
    }

    /// A hull the room moved by a portal onto the frame's near end is
    /// drawn there with no tread marks pressed along the way: the
    /// interpolator names it in `Frame::snapped`, and its trail is lifted
    /// before `tick_presentation` lays the frame's marks. The hop is
    /// shorter than `net::apply`'s own jump distance, so nothing else
    /// would catch it.
    #[test]
    fn a_hull_that_jumped_presses_no_tread_marks_across_the_jump() {
        let (mut room, mut round) = room_and_round();
        room.welcome();
        round.frame(&Intent::default(), 1.0 / 60.0);
        let tracks_before = round.game().expect("a replica").tracks.len();
        let enemy = room.game.tank_snapshots().into_iter().find(|t| t.slot == 1).expect("an enemy in slot 1");
        let from = enemy.position;
        let to = crate::math::Vec2::new(from.x + 40.0, from.y);
        let send = |room: &mut Room, tick: u32, events: Vec<WireEvent>| {
            let mut s = encode::snapshot(&room.game, [0; MAX_SEATS]);
            s.tick = tick;
            s.server_ms = 1_000 + crate::net::interp::tick_ms(tick).round() as u32;
            s.events = events;
            room.say(Msg::Snapshot(s));
        };
        for tick in 1..8 {
            send(&mut room, tick, Vec::new());
        }
        room.game.debug_teleport(1, to, None).expect("the enemy moves");
        let hop = WireEvent::Teleported {
            slot: 1,
            x: crate::net::wire::quantise_pos(from.x),
            y: crate::net::wire::quantise_pos(from.y),
            to_x: crate::net::wire::quantise_pos(to.x),
            to_y: crate::net::wire::quantise_pos(to.y),
        };
        send(&mut room, 8, vec![hop]);
        for tick in 9..=20 {
            send(&mut room, tick, Vec::new());
        }
        // Render time steers toward the newest ticks at its own pace; the
        // frames run until it has passed the hop.
        for _ in 0..100 {
            round.frame(&Intent::default(), 1.0 / 60.0);
            if round.game().is_some_and(|g| g.frame() >= 10) {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(8));
        }
        let replica = round.game().expect("a replica");
        assert!(replica.frame() >= 10, "the picture passed the hop: frame {}", replica.frame());
        let drawn = replica.tank_snapshots().into_iter().find(|t| t.slot == 1).expect("the enemy");
        assert!(drawn.position.distance_to(to) < 1.0, "drawn where it landed: {:?}", drawn.position);
        assert_eq!(replica.tracks.len(), tracks_before, "tread marks were pressed across the hop");
    }

    /// Every reading of the interpolator's report reaches `stats_json`,
    /// and through it `status.round.interpolation` and `bb_net_stats`.
    #[test]
    fn stats_json_carries_every_interpolation_reading() {
        let (mut room, mut round) = room_and_round();
        room.welcome();
        round.frame(&Intent::default(), 1.0 / 60.0);
        let stats = round.stats_json();
        let interp = stats["interpolation"].as_object().expect("an interpolation block");
        let readings = [
            "delay_ms",
            "target_ms",
            "jitter_ms",
            "interval_ms",
            "buffered",
            "extrapolated_frames",
            "lateness_p50_ms",
            "lateness_p95_ms",
            "stalls",
            "rate",
            "corrections",
            "error_p95_px",
            "stale_events",
        ];
        for key in readings {
            assert!(interp.get(key).is_some_and(|v| v.is_number()), "{key} is missing from {stats}");
        }
        assert_eq!(interp.len(), readings.len(), "{stats}");
    }
}
