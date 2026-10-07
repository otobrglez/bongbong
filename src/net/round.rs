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
//!    so (`Lead`, docs/online-coop-prd.md §4.12), and only while the
//!    room's round is playing: the lobby before and after a round sends
//!    nothing, so a rematch never opens on the last round's poses.
//! 3. Write the interpolated picture into the replica, the predicted
//!    hull and shots over it, and tick its cosmetics.
//!
//! Nothing here runs `Game::update`, and the local round in
//! `mode::Session` is untouched: online is a third driver beside Play and
//! Build, never a replacement for the in-process round.

use crate::Position;
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

/// Where an own shot crossing `from` to `to` this frame stops first: the
/// drawn world's contact (`world_hit`, and whether it is a tank's), or an
/// opposing shell of `shells` it meets within `reach` by the room's rule
/// (`present::shells_meet`), whichever comes sooner along the way - with
/// the shell's id, so it bursts too. A shell already met this frame
/// (`met`) meets nothing else.
pub fn first_contact(
    from: crate::math::Vec2,
    to: crate::math::Vec2,
    world_hit: Option<(crate::math::Vec2, bool)>,
    shells: &[IncomingShell],
    reach: f32,
    met: &[(u16, crate::math::Vec2)],
) -> Option<(crate::math::Vec2, bool, Option<u16>)> {
    let span = ((to.x - from.x).powi(2) + (to.y - from.y).powi(2)).sqrt().max(f32::EPSILON);
    let along = |at: crate::math::Vec2| ((at.x - from.x).powi(2) + (at.y - from.y).powi(2)).sqrt() / span;
    let shell = shells
        .iter()
        .filter(|s| !met.iter().any(|&(id, _)| id == s.id))
        .filter_map(|s| crate::simulation::present::shells_meet(from, to, s.from, s.to, reach).map(|(t, at)| (t, at, s.id)))
        .min_by(|a, b| a.0.total_cmp(&b.0));
    match (world_hit, shell) {
        (Some((at, tank)), Some((t, _, _))) if along(at) <= t => Some((at, tank, None)),
        (_, Some((_, at, id))) => Some((at, false, Some(id))),
        (Some((at, tank)), None) => Some((at, tank, None)),
        (None, None) => None,
    }
}

/// An opposing shell's stretch of this frame as it is drawn in the
/// present, for this seat's own shells to meet.
#[derive(Clone, Copy, Debug)]
pub struct IncomingShell {
    pub id: u16,
    pub from: crate::math::Vec2,
    pub to: crate::math::Vec2,
}

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
    /// A lamp tap on a frame that sent nothing, carried into the next
    /// packet like `pending_fire`.
    pending_lamp: bool,
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
    /// Foreign shots this client drew bursting - on its own hull, or
    /// against one of its own shells - and where: kept off the picture
    /// until the room's copy goes, or until the room's copy is seen flying
    /// on past that point, which is the room saying it missed.
    struck: std::collections::BTreeMap<u16, crate::math::Vec2>,
    /// Where each foreign shot was drawn last frame, for the stretch it
    /// crossed this one.
    incoming_drawn: std::collections::BTreeMap<u16, crate::math::Vec2>,
    /// This seat's drones launched on the press and still drawn by this
    /// client, oldest first (`OwnDrone`, docs/fpv-swarm.md "Wire").
    own_drones: Vec<OwnDrone>,
    /// The next own launch's number (`OwnDrone::id`), from 0 each round.
    next_own_drone: u32,
    /// This seat's rods called on its own release, drawn until the room's
    /// zone claims them (`OwnCall`).
    own_calls: Vec<OwnCall>,
    /// The next own call's number (`OwnCall::id`), from 0 each round.
    next_own_call: u32,
    /// This seat's gravity well orbs launched on its own press, flown here
    /// until they anchor (`OwnOrb`), and the anchors it drew until the
    /// room's `WellAnchored` claims them (`OwnAnchor`).
    own_orbs: Vec<OwnOrb>,
    own_anchors: Vec<OwnAnchor>,
    next_own_orb: u32,
    /// How many ticks ahead of the picture this seat's latest input lands
    /// in the room (`incoming_lead_ticks`), as last measured: how long a
    /// press waits to be seen in the room's picture.
    lead_ticks: f32,
    /// The local seat's own hull, run ahead of the room and pulled back
    /// by each snapshot (stage 2, docs/online-coop-prd.md §4.12). Built
    /// beside the replica from the same `Welcome`, so the two step
    /// against the same statics. `None` until a welcome arrives. Drawn
    /// only while `online_predict_own_tank` is on - the knob is live, so
    /// turning it off hands the drawn hull straight back to the
    /// interpolator without dropping the sandbox - and kept in step with
    /// the room while that knob is on or the client owns its hull, since
    /// an owned hull's poses come from it whatever is drawn.
    predictor: Option<Predictor>,
    /// `online_predict_own_tank` pinned for a test, which may not write
    /// the process-wide table.
    #[cfg(test)]
    predict_own_tank: Option<bool>,
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
            pending_lamp: false,
            trigger_down: false,
            lead: Lead::new(),
            client_hull: tuning().online_client_hull,
            rtt: RttClock::default(),
            view: (0, 0),
            flying_since: std::collections::BTreeMap::new(),
            struck: std::collections::BTreeMap::new(),
            incoming_drawn: std::collections::BTreeMap::new(),
            own_drones: Vec::new(),
            next_own_drone: 0,
            own_calls: Vec::new(),
            next_own_call: 0,
            own_orbs: Vec::new(),
            own_anchors: Vec::new(),
            next_own_orb: 0,
            lead_ticks: 0.0,
            note: None,
            ended: None,
            scratch: Vec::new(),
            tuning_before: None,
            #[cfg(test)]
            predict_own_tank: None,
        }
    }

    /// Whether the own hull is drawn predicted (`online_predict_own_tank`,
    /// live).
    fn predict_own_tank(&self) -> bool {
        #[cfg(test)]
        if let Some(on) = self.predict_own_tank {
            return on;
        }
        tuning().online_predict_own_tank
    }

    /// Whether provisional shots are drawn: the own hull predicted and
    /// `online_predict_shots` on, both live.
    fn predict_shots(&self) -> bool {
        self.predict_own_tank() && tuning().online_predict_shots
    }

    /// Whether the sandbox keeps in step with the room: it takes every
    /// snapshot's world, accounts and confirms its presses and takes the
    /// room's shoves. Always while the client owns its hull, whose every
    /// pose comes from the sandbox whether or not the prediction is drawn;
    /// otherwise only while the prediction is.
    fn sandbox_follows_room(&self) -> bool {
        self.client_hull || self.predict_own_tank()
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
        use crate::text::{keys, text};
        let t = text();
        let code = self.client.code().unwrap_or("-----");
        let seat = self.client.seat().map_or_else(|| "-".to_string(), |s| format!("{}", s + 1));
        // The label is a word where the catalogue has one (`ROOM`), and
        // itself where it does not (the rig's `RIG`).
        let room = t.named("status-label", &self.label.to_ascii_lowercase());
        // The round trip, where one has been measured, as its own words;
        // empty otherwise, so the line reads the same with or without it.
        let rtt = self.rtt.report().map_or_else(String::new, |r| format!(" - {}", t.fmt(keys::STATUS_PING, &[("ms", (r.rtt_ms.round() as i64).into())])));
        let args = |ms: i64| vec![("room", room.as_str().into()), ("code", code.into()), ("seat", seat.as_str().into()), ("rtt", rtt.as_str().into()), ("ms", ms.into())];
        let mut line = match self.client.phase() {
            Phase::Connecting => t.fmt(keys::STATUS_CONNECTING, &args(0)),
            Phase::Greeting => t.fmt(keys::STATUS_GREETING, &args(0)),
            Phase::Lobby => t.fmt(keys::STATUS_LOBBY, &args(0)),
            Phase::Playing => {
                // The buffer's depth is what the delay is buying. Below
                // zero the picture has run past everything that arrived -
                // a stalled link, or a room that has stopped ticking
                // because the round is over - and a number would only
                // say how long ago that was.
                match self.interp.lead_ms(self.local_ms()).unwrap_or(0.0).round() as i64 {
                    lead if lead >= 0 => t.fmt(keys::STATUS_BUFFER, &args(lead)),
                    _ => t.fmt(keys::STATUS_WAITING, &args(0)),
                }
            }
            Phase::Closed(closed) => t.fmt(keys::STATUS_OFFLINE, &[("room", room.as_str().into()), ("reason", closed.reason.as_str().into())]),
        };
        if let Some(note) = &self.note {
            line.push_str(" - ");
            line.push_str(&crate::text::fold(note));
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
                ClientEvent::Refused(refusal) => {
                    // A refused start is askable again: the room says why
                    // (a seat that is not ready), and the reason goes away.
                    self.start_sent = false;
                    self.note = Some(crate::text::refusal(&refusal));
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
        self.own_drones.clear();
        self.own_orbs.clear();
        self.own_anchors.clear();
        self.next_own_drone = 0;
        self.own_calls.clear();
        self.next_own_call = 0;
        let patch = welcome.tuning_json.trim();
        if !patch.is_empty() && patch != "{}" {
            if self.tuning_before.is_none() {
                self.tuning_before = Some(tuning::current());
            }
            match tuning::submit_json(patch) {
                Ok(_) => {
                    tuning::apply_pending();
                }
                Err(e) => self.note = Some(crate::text::text().fmt(crate::text::keys::NOTE_TUNING_REFUSED, &[("detail", e.to_string().into())])),
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
            Err(e) => self.note = Some(crate::text::text().fmt(crate::text::keys::NOTE_WELCOME_REFUSED, &[("detail", e.into())])),
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
    ///
    /// Only while the room's round is playing. In the lobby - waiting for
    /// the first round, or back from one - there is nothing to steer, and
    /// an owned hull's packets would carry the finished round's pose into
    /// the next one's mailbox, so a rematch would open with the seat placed
    /// where the last round left it. Nothing is owed across the gap: the
    /// round starts on a fresh tick and an untouched trigger.
    fn send(&mut self, intent: &Intent, dt: f32) {
        if *self.client.phase() != Phase::Playing {
            self.send_owed = 0.0;
            self.pending_fire = false;
            self.pending_lamp = false;
            return;
        }
        self.pending_fire |= intent.fire;
        self.pending_lamp |= intent.lamp;
        self.send_owed = (self.send_owed + dt.max(0.0)).min(SEND_CATCH_UP_TICKS as f32 * PHYSICS_FIXED_DT);
        while self.send_owed >= PHYSICS_FIXED_DT {
            self.send_owed -= PHYSICS_FIXED_DT;
            // An owned hull's poses are played out on the room's own clock,
            // which keeps its margin itself, so there is no queue to steer:
            // one packet and one sandbox tick per tick of real time, never
            // two and never none (§4.16).
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
        out.lamp |= self.pending_lamp;
        // One counter for both: the sandbox steps on exactly the input
        // the packet carries - fire hold included, since that is the bit
        // the server's press edge will see - stamped with exactly the
        // tick the server will name back in `acked`.
        let Some(msg) = self.client.prepare_intent(&out) else { return };
        let mut msg = msg.with_view(self.view.0, self.view.1);
        self.pending_fire = false;
        self.pending_lamp = false;
        let shots = self.predict_shots();
        if let Some(predictor) = self.predictor.as_mut() {
            // The knob is live: read each tick, so a shot pressed after
            // it was turned on is drawn and one after it was turned off
            // is not. Shots are drawn only with the own tank predicted -
            // the one test `draw` asks before it keeps the room's own
            // muzzle ripples and beams off the picture.
            predictor.set_shots_enabled(shots);
            predictor.step_at(msg.tick, msg.intent());
            // Owning the hull, the packet says where this tick left it,
            // and the room puts the seat there (§4.14).
            if self.client_hull && let Some(pose) = predictor.pose() {
                msg = msg.with_pose(pose);
            }
            // A rod's reticle stands where this client has it, on the
            // room's copy too (docs/rod-from-god.md "The reticle report").
            if let Some(cell) = predictor.reticle_report() {
                msg = msg.with_reticle(crate::net::encode::reticle_code(predictor.field_cols(), cell));
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
    /// In order (docs/online-coop-prd.md §4.16): this seat's laser `Fired`
    /// in the frame's snapshot claim the beams this client drew
    /// (`confirm_presses`); the interpolated room is written into the
    /// replica, less what this client drew itself - its shots' muzzle
    /// ripples and the room's copies of the beams just claimed; this
    /// seat's other `Fired` confirm their presses; the room's copies of
    /// this seat's shots are paired with the provisionals and kept off the
    /// picture while a provisional stands for them; the predicted hull is
    /// written at the sandbox's newest tick; everyone else's shots are
    /// carried forward to the present, then the provisionals fly one frame
    /// and stop at the first thing they meet in the drawn world, an
    /// opposing shell among it; beams pressed since the last frame are
    /// drawn to where they stop; then the frame's cosmetics tick.
    fn draw(&mut self, dt: f32, now: i64) {
        if self.replica.is_none() {
            return;
        }
        let sampled = self.interp.sample(now);
        let predicting_shots = self.predict_shots() && self.predictor.is_some();
        if let Some(frame) = &sampled {
            let presses = self.confirm_presses(&frame.snapshot);
            // What this seat drew on its own press - its shots' muzzle
            // ripples, the beams and waves it claimed - is not drawn twice.
            let show = match self.client.seat() {
                Some(seat) if predicting_shots => apply::Show::OwnShotsDrawn { seat, presses },
                _ => apply::Show::All,
            };
            // A launch it drew claims the room's copy of that drone, and a
            // call it drew the room's zone.
            if let Some(seat) = self.client.seat() {
                for i in show.presses_drawn(&frame.snapshot) {
                    claim_own_drone(&mut self.own_drones, &frame.snapshot.events, i, seat);
                    claim_own_call(&mut self.own_calls, &frame.snapshot.events, i, seat);
                }
                self.claim_own_anchors(&frame.snapshot.events, seat);
            }
            if let Some(game) = self.replica.as_mut() {
                apply::snapshot_with(game, &frame.snapshot, show);
            }
            let frac = (frame.ahead / PHYSICS_FIXED_DT * 256.0).clamp(0.0, 255.0) as u8;
            self.view = (frame.snapshot.tick, frac);
            self.confirm_shots(&frame.snapshot);
        }
        // How long a press waits for its `Fired` follows the link whether
        // its shots are drawn or not: an owned hull's undrawn presses run
        // the gate and kick the hull on the same wait.
        if let Some(predictor) = self.predictor.as_mut() {
            let delay = self.interp.delay_ms();
            let rtt = self.rtt.report().map_or(0.0, |r| r.rtt_p95_ms);
            predictor.set_refusal_after(((rtt + delay) / 1000.0) as f32 + REFUSAL_MARGIN_SECONDS);
        }
        if predicting_shots {
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
            // Incoming first: this frame's stretch of every opposing shell
            // is what the own shells are swept against.
            let shells = match &sampled {
                Some(frame) => self.draw_incoming_in_present(frame, now, world, seat),
                None => Vec::new(),
            };
            self.fly_own_shots(dt, world, seat, &shells);
            self.fly_own_orbs(dt, world, seat);
            self.fly_own_drones(dt, seat, sampled.is_some());
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
        self.place_own_calls(dt);
    }

    /// One frame of this seat's own orbs (`OwnOrb`, docs/gravity-well.md
    /// "Wire"): each room copy of this seat's orbs pairs with the oldest
    /// own orb waiting for one and is kept off the picture while that orb,
    /// or the anchor it made, is drawn; each own orb flies one frame - bent
    /// by the wells on this client's present, swallowed at a core, anchored
    /// on the first thing it meets in the drawn world or at its range - and
    /// goes back on the picture under its own id.
    fn fly_own_orbs(&mut self, dt: f32, world: &crate::simulation::present::PresentWorld, seat: u8) {
        let refusal = self.predictor.as_ref().map_or(1.0, |p| p.refusal_after());
        let Some(game) = self.replica.as_mut() else { return };
        let t = tuning();
        let owner = crate::shell::Owner::Player(seat);
        let base = crate::net::predict::PROVISIONAL_ID_BASE;
        let mut room: Vec<u16> = game.orbs.iter().filter(|o| o.id < base && o.owner == owner).map(|o| o.id as u16).collect();
        room.sort_unstable();
        let mut taken: BTreeSet<u16> = self.own_orbs.iter().filter_map(|o| o.room).chain(self.own_anchors.iter().filter_map(|a| a.room)).collect();
        for own in self.own_orbs.iter_mut().filter(|o| o.room.is_none()) {
            if let Some(&id) = room.iter().find(|id| !taken.contains(id)) {
                own.room = Some(id);
                taken.insert(id);
            }
        }
        let wells = game.present_wells();
        let mut landed: Vec<(Position, Option<u16>)> = Vec::new();
        let mut swallowed: Vec<Position> = Vec::new();
        self.own_orbs.retain_mut(|own| {
            own.age += dt;
            let from = own.at;
            if !wells.is_empty() {
                own.velocity = crate::well::bend(own.velocity, wells.shot_accel(own.at, &t), dt);
            }
            let to = own.at + own.velocity * dt;
            if let Some((_, k)) = wells.core_hit(from, to, &t) {
                swallowed.push(from + (to - from) * k);
                return false;
            }
            if let Some(at) = world.orb_contact(seat, from, to, t.well_orb_half_px) {
                let travel = to - from;
                let len = travel.length();
                let back = if len > 1e-4 { travel * ((t.well_orb_half_px + 2.0) / len) } else { crate::math::Vec2::zero() };
                landed.push((at - back, own.room));
                return false;
            }
            own.flown += from.distance_to(to);
            own.at = to;
            if own.flown >= t.well_orb_range_px {
                landed.push((own.at, own.room));
                return false;
            }
            // An orb the room never launched is refused.
            own.room.is_some() || own.age < refusal
        });
        if !landed.is_empty()
            && let Some(predictor) = self.predictor.as_mut()
        {
            predictor.orb_landed();
        }
        for (at, room_id) in landed {
            anchor_own(game, &mut self.own_anchors, seat, at, room_id);
        }
        for at in swallowed {
            game.swallow_show(at);
            if let Some(predictor) = self.predictor.as_mut() {
                predictor.orb_landed();
            }
        }
        // The room's copies drawn by this client stay off the picture; the
        // own orbs go on under their own ids.
        let hidden: BTreeSet<u32> =
            self.own_orbs.iter().filter_map(|o| o.room).chain(self.own_anchors.iter().filter_map(|a| a.room)).map(u32::from).collect();
        game.orbs.retain(|o| o.id < base && !hidden.contains(&o.id));
        for own in &self.own_orbs {
            let mut orb = crate::well::Orb::launch(own.at, crate::math::Vec2::new(0.0, -1.0), owner, &t);
            orb.id = own.id;
            orb.velocity = own.velocity;
            orb.rotation = crate::well::heading_deg(own.velocity);
            orb.flown = own.flown;
            orb.age = own.age;
            game.orbs.push(orb);
        }
        game.orbs.sort_by_key(|o| o.id);
    }

    /// The room's `WellAnchored` for this seat in `events` claims this
    /// client's anchors oldest first: the drawn anchor goes, the room's well
    /// standing where it has it from then on. One the client never drew -
    /// its orb still flying here, the room's met something first - takes
    /// that orb and shows the snap where the room has it.
    fn claim_own_anchors(&mut self, events: &[WireEvent], seat: u8) {
        let anchors: Vec<Position> = events
            .iter()
            .filter_map(|e| match *e {
                WireEvent::WellAnchored { seat: s, x, y, .. } if s == seat => {
                    Some(Position::new(crate::net::wire::dequantise_pos(x), crate::net::wire::dequantise_pos(y)))
                }
                _ => None,
            })
            .collect();
        for at in anchors {
            if let Some(i) = self.own_anchors.iter().position(|a| !a.claimed) {
                self.own_anchors[i].claimed = true;
            } else if !self.own_orbs.is_empty() {
                self.own_orbs.remove(0);
                if let Some(predictor) = self.predictor.as_mut() {
                    predictor.orb_landed();
                }
                if let Some(game) = self.replica.as_mut() {
                    game.show_well_anchor(seat, at);
                }
            }
        }
        self.own_anchors.retain(|a| !a.claimed);
    }

    /// One frame of this seat's own calls (`OwnCall`) and every zone's
    /// countdown: the countdowns run on this client's present - the
    /// picture's clock plus the lead its own inputs are ahead of it in
    /// the room, the time the seat has to get its hull out - and each call
    /// it drew is a provisional zone from its release until the room's
    /// zone claims it, or until the refusal wait passes with none.
    fn place_own_calls(&mut self, dt: f32) {
        let lead = self.lead_ticks * PHYSICS_FIXED_DT;
        let refusal = self.predictor.as_ref().map_or(1.0, |p| p.refusal_after());
        let seat = self.client.seat();
        let Some(game) = self.replica.as_mut() else { return };
        game.set_zone_lead(lead);
        for own in &mut self.own_calls {
            own.since += dt;
        }
        self.own_calls.retain(|own| !own.claimed && own.since < refusal);
        for own in &mut self.own_anchors {
            own.since += dt;
        }
        self.own_anchors.retain(|own| !own.claimed && own.since < refusal);
        let Some(seat) = seat else {
            game.set_provisional_zones(&[]);
            return;
        };
        let countdown = tuning().rod_countdown_seconds;
        let form = tuning().well_form_seconds;
        let now = game.time + lead;
        // A well this seat anchored itself: forming from its anchor frame.
        let wells: Vec<crate::zone::Zone> = self
            .own_anchors
            .iter_mut()
            .enumerate()
            .map(|(k, own)| {
                let until = *own.until.get_or_insert(now + form - own.since);
                crate::zone::Zone {
                    id: crate::zone::PROVISIONAL_ZONE_BASE + OWN_WELL_ZONE_OFFSET + k as u32,
                    kind: crate::zone::ZoneKind::Well(crate::well::WellZone {
                        stage: if until <= now { crate::well::WellStage::Pulling } else { crate::well::WellStage::Forming },
                        by: crate::well::AnchorBy::Press,
                        seat: Some(seat),
                        emp_collapse: false,
                    }),
                    owner: crate::shell::Owner::Player(seat),
                    centre: own.at,
                    until: if until <= now { until + tuning().well_pull_seconds } else { until },
                }
            })
            .collect();
        let zones: Vec<crate::zone::Zone> = self
            .own_calls
            .iter_mut()
            .map(|own| {
                let until = *own.until.get_or_insert(now + countdown - own.since);
                crate::zone::Zone {
                    id: crate::zone::PROVISIONAL_ZONE_BASE + own.id,
                    kind: crate::zone::ZoneKind::Rod(crate::rod::RodCall { cell: own.press.cell, seat: Some(seat) }),
                    owner: crate::shell::Owner::Player(seat),
                    centre: crate::map::cell_to_world(own.press.cell.0, own.press.cell.1),
                    until,
                }
            })
            .chain(wells)
            .collect();
        game.set_provisional_zones(&zones);
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
    /// The hull is drawn at the sandbox's newest tick
    /// (`Predictor::drawn_pose`), as a local round draws its newest step.
    ///
    /// True when the seat was written: the drawn hull is the prediction's.
    fn write_predicted(&mut self) -> bool {
        if !self.predict_own_tank() {
            return false;
        }
        let (Some(predictor), Some(game)) = (self.predictor.as_ref(), self.replica.as_mut()) else { return false };
        let (Some(seat), Some((position, rotation))) = (self.client.seat(), predictor.drawn_pose()) else { return false };
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
        // An EMP press this client drew takes its special offline here at
        // once, as the room's will a round trip later: the HUD says so on
        // the press frame.
        if predictor.offline_left() > 0.0 {
            game.hold_seat_offline(seat, predictor.offline_left());
        }
        // A charge is drawn as the sandbox holds it - its glow, the
        // module's cells and the HUD's gauge from the press frame to the
        // release frame (docs/gauss-rail.md "Online").
        game.set_seat_charge(seat as usize, predictor.charge());
        // A rod's reticle likewise, stepping with the stick at once.
        game.show_seat_reticle(seat as usize, predictor.reticle());
        true
    }

    /// This frame's room copies of this seat's shots, at the picture's
    /// tick: handed to the predictor to pair with its provisionals, then
    /// taken off the picture where a provisional stands for them - the
    /// provisional is the one drawn for the shot's whole life, on this
    /// client's timeline, and is never swapped for the room's copy in the
    /// past (§4.16).
    fn pair_own_shots(&mut self) {
        let tick = self.view.0;
        let (Some(predictor), Some(game), Some(seat)) = (self.predictor.as_mut(), self.replica.as_mut(), self.client.seat()) else { return };
        let own = game.seat_shots(seat);
        predictor.observe_server_shots(tick, &own);
        game.remove_shots(&predictor.hidden_server_shots());
    }

    /// One frame of this seat's provisional shots against the drawn world:
    /// a press since the last frame ripples the muzzle its first shot left
    /// and kicks the turret (what a replica does on a `Fired`, drawn here on
    /// the press instead of a round trip later); each shot flies by its own state
    /// machine and stops at the first tile, edge, tank or frog it meets,
    /// its impact drawn there at once; beams pressed since the last frame
    /// are drawn to where they stop.
    fn fly_own_shots(&mut self, dt: f32, world: &crate::simulation::present::PresentWorld, seat: u8, shells: &[IncomingShell]) {
        use crate::net::predict::ShotStop;
        use crate::simulation::present::{Contact, shot_half_extent};
        let (Some(predictor), Some(game)) = (self.predictor.as_mut(), self.replica.as_mut()) else { return };
        for (at, kind) in predictor.take_muzzles() {
            game.draw_muzzle(at);
            game.kick_seat(seat, kind);
        }
        // An own shell meets an opposing one where the room's
        // `shell_vs_shell` would have them meet: both are drawn on the
        // room's clock of this client's present, so the crossing in the
        // picture is the crossing in the room.
        let reach = tuning().shell_hit_half_extent * 2.0;
        let mut met: Vec<(u16, crate::math::Vec2)> = Vec::new();
        predictor.advance_shots(dt, |kind, from, to, leaving| {
            // Into a portal before anything else: judged no further, as the
            // room judges it (`Game::resolve_projectiles`).
            let portal = world.portal_entry(from, to, leaving);
            let to = portal.unwrap_or(to);
            let mut world_hit = world
                .shot_contact(Some(seat), from, to, shot_half_extent(kind))
                .map(|(at, contact)| (at, matches!(contact, Contact::Tank { .. } | Contact::Frog)));
            // A bullet stops at a drone of the other side's it crosses
            // first, as the room's sweep has it; the drone stays up until
            // the room says it is down.
            if kind == crate::simulation::ProvisionalKind::Bullet
                && let Some(at) = world.air_contact(Some(seat), from, to, shot_half_extent(kind))
                && world_hit.is_none_or(|(hit, _)| from.distance_to(at) < from.distance_to(hit))
            {
                world_hit = Some((at, false));
            }
            let opposing: &[IncomingShell] = if kind == crate::simulation::ProvisionalKind::Shell { shells } else { &[] };
            let Some((at, tank, shell)) = first_contact(from, to, world_hit, opposing, reach, &met) else {
                return portal.map(|at| (at, ShotStop::Portal));
            };
            if let Some(id) = shell {
                met.push((id, at));
            }
            Some((at, if tank { ShotStop::Body } else { ShotStop::Struck }))
        });
        for (id, at) in met {
            self.struck.insert(id, at);
            game.remove_shots(&[id]);
        }
        // A beam is traced as far as the room traces it, or into the first
        // portal on its way: the room draws which one it comes out of, and
        // the legs past it arrive as the room's own `LaserBeam`s.
        let beam_reach = crate::simulation::laser_reach(game.map.field_size());
        for show in predictor.take_press_shows() {
            match show {
                crate::net::predict::PressShow::Beam(beam) => {
                    let far = crate::math::Vec2::new(beam.start.x + beam.dir.x * beam_reach, beam.start.y + beam.dir.y * beam_reach);
                    let far = world.portal_entry(beam.start, far, world.portal_inside(beam.start)).unwrap_or(far);
                    let end = world
                        .shot_contact(Some(seat), beam.start, far, tuning().shell_hit_half_extent)
                        .map_or(far, |(at, _)| at);
                    game.draw_beam(beam.lens, end, beam.variant);
                    game.flash_seat_laser(seat);
                }
                // The wave, cast against the replica's own tiles from the
                // predicted pose; the dish fires with it.
                crate::net::predict::PressShow::Sonic(press) => {
                    game.draw_press_show(seat, crate::tank::ActiveWeapon::SonicHammer, press.origin, press.facing)
                }
                // The ring, from the predicted pivot; the coil fires with it.
                crate::net::predict::PressShow::Emp(press) => {
                    game.draw_press_show(seat, crate::tank::ActiveWeapon::Emp, press.origin, crate::tank::Dir::Up)
                }
                // A rail's slug, traced as the room traces it through the
                // drawn world - or into the first portal on its way, the
                // legs past it the room's own `RailSlug`s - from the bore.
                crate::net::predict::PressShow::Rail(press) => {
                    let far = crate::math::Vec2::new(press.start.x + press.dir.x * beam_reach, press.start.y + press.dir.y * beam_reach);
                    let entry = world.portal_entry(press.start, far, world.portal_inside(press.start));
                    let trace = world.rail_trace(Some(seat), press.start, entry.unwrap_or(far), tuning().gauss_half_width, press.overcharged);
                    let portal = entry.is_some_and(|at| at.distance_to(trace.end) < 0.5);
                    game.draw_rail_press(seat, press.muzzle, trace, portal, press.overcharged);
                }
                crate::net::predict::PressShow::ChargeEnd(end) => game.charge_end_show(end.at, end.end),
                // A drone off the halo: drawn from here on by
                // `fly_own_drones`, its module's relay flashing now.
                crate::net::predict::PressShow::Drone(press) => {
                    game.flash_seat_fpv(seat);
                    let lead = self.lead_ticks * PHYSICS_FIXED_DT;
                    let id = self.next_own_drone;
                    self.next_own_drone = self.next_own_drone.wrapping_add(1) & OWN_DRONE_ID_MASK;
                    self.own_drones.push(OwnDrone { id, press, since: 0.0, lead, room: None });
                }
                // A call: its beam and circle from the release frame, its
                // uplink lit now.
                crate::net::predict::PressShow::Rod(press) => {
                    game.flash_seat_rod(seat);
                    let id = self.next_own_call;
                    self.next_own_call = self.next_own_call.wrapping_add(1) & OWN_CALL_ID_MASK;
                    self.own_calls.push(OwnCall { id, press, since: 0.0, until: None, claimed: false });
                }
                // An orb off the gun line, flown from here on by
                // `fly_own_orbs`; its projector lit now.
                crate::net::predict::PressShow::Orb(press) => {
                    game.flash_seat_well(seat, false);
                    let id = crate::net::predict::PROVISIONAL_ID_BASE + OWN_ORB_ID_OFFSET + (self.next_own_orb & OWN_ORB_ID_MASK);
                    self.next_own_orb = self.next_own_orb.wrapping_add(1);
                    let velocity = press.dir * tuning().well_orb_speed;
                    self.own_orbs.push(OwnOrb { id, at: press.muzzle, velocity, flown: 0.0, age: 0.0, room: None });
                }
                // The anchor press: the oldest orb in flight anchors where
                // it stands this frame.
                crate::net::predict::PressShow::Anchor { .. } => {
                    if !self.own_orbs.is_empty() {
                        let orb = self.own_orbs.remove(0);
                        anchor_own(game, &mut self.own_anchors, seat, orb.at, orb.room);
                    }
                }
            }
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
    /// the room's, and arrives with its `Hit` - or until the room's copy is
    /// seen flying on past it, when it is drawn again. Returns this frame's
    /// stretch of every opposing shell, which this seat's own shells meet
    /// the way the room's `shell_vs_shell` has them meet.
    fn draw_incoming_in_present(
        &mut self,
        frame: &crate::net::interp::Frame,
        now: i64,
        world: &crate::simulation::present::PresentWorld,
        seat: u8,
    ) -> Vec<IncomingShell> {
        use crate::simulation::present::segment_box;
        let (Some(predictor), Some(newest)) = (self.predictor.as_ref(), self.interp.newest()) else { return Vec::new() };
        let acked = newest.acked.get(seat as usize).copied().unwrap_or(0);
        if acked == 0 {
            return Vec::new();
        }
        let render = frame.snapshot.tick as f64 + (frame.ahead / PHYSICS_FIXED_DT) as f64;
        let lead_ticks = incoming_lead_ticks(newest.tick, acked, predictor.tick().wrapping_sub(1), render);
        self.lead_ticks = lead_ticks;
        let hull = world.seat_hull(seat);
        let Some(game) = self.replica.as_mut() else { return Vec::new() };
        let foreign = game.foreign_flying_shots(seat);
        let alive: std::collections::BTreeSet<u16> = foreign.iter().map(|s| s.id).collect();
        self.flying_since.retain(|id, _| alive.contains(id));
        self.incoming_drawn.retain(|id, _| alive.contains(id));
        self.struck.retain(|id, _| game.has_shot(*id));
        // A shot the frame shows coming out of a portal starts afresh at
        // the exit, as from a muzzle: eased up to the lead again, with no
        // stretch drawn from the far side of the field.
        for event in &frame.snapshot.events {
            if let WireEvent::ShotTeleported { id: Some(id), .. } = *event {
                self.flying_since.insert(id, now);
                self.incoming_drawn.remove(&id);
            }
        }
        let mut struck_now = Vec::new();
        let mut in_portal = Vec::new();
        let mut shells = Vec::new();
        for shot in foreign {
            if let Some(&at) = self.struck.get(&shot.id) {
                // The room's copy itself - where the room has it, not
                // carried ahead - flying on past where this client drew
                // it burst: the room missed, and the shot is drawn again.
                let speed = (shot.velocity.x * shot.velocity.x + shot.velocity.y * shot.velocity.y).sqrt();
                let past = ((shot.position.x - at.x) * shot.velocity.x + (shot.position.y - at.y) * shot.velocity.y) / speed.max(f32::EPSILON);
                if past <= crate::net::predict::MISS_MARGIN_PX {
                    continue;
                }
                self.struck.remove(&shot.id);
            }
            let since = *self.flying_since.entry(shot.id).or_insert(now);
            let catch = smoothstep(((now - since) as f32 / CATCH_UP_MS).clamp(0.0, 1.0));
            let ahead = lead_ticks * PHYSICS_FIXED_DT * catch;
            let mut to = crate::math::Vec2::new(shot.position.x + shot.velocity.x * ahead, shot.position.y + shot.velocity.y * ahead);
            if let Some(stop) = world.static_contact(shot.position, to) {
                to = stop;
            }
            // Carried into a portal: it is in there until the room's copy
            // comes out of whichever one the room drew.
            if world.portal_entry(shot.position, to, None).is_some() {

                in_portal.push(shot.id);
                self.incoming_drawn.remove(&shot.id);
                continue;
            }
            if let Some((centre, half)) = hull {
                let grow = crate::math::Vec2::new(shot.half_extent, shot.half_extent);
                if let Some(t) = segment_box(shot.position, to, centre, half + grow) {
                    let at = crate::math::Vec2::new(
                        shot.position.x + (to.x - shot.position.x) * t,
                        shot.position.y + (to.y - shot.position.y) * t,
                    );
                    game.draw_impact(at);
                    struck_now.push((shot.id, at));
                    continue;
                }
            }
            let from = self.incoming_drawn.insert(shot.id, to).unwrap_or(to);
            if shot.opposing_shell {
                shells.push(IncomingShell { id: shot.id, from, to });
            }
            game.move_shot(shot.id, to);
        }
        self.struck.extend(struck_now);
        let mut hidden: Vec<u16> = self.struck.keys().copied().collect();
        hidden.extend(in_portal);
        game.remove_shots(&hidden);
        shells
    }

    /// Pull the sandbox back into line with a snapshot that just landed.
    ///
    /// The whole snapshot goes in, not this seat's hull picked out of it:
    /// the sandbox is a projection of the server's world, so everything
    /// the prediction steps against - other hulls, destroyed walls, a
    /// speed boost - arrives by the same path the replica takes
    /// (`net::predict::Predictor::reconcile`). An owned hull's sandbox takes
    /// it whether or not the prediction is drawn: its poses are the seat's.
    fn reconcile(&mut self, snapshot: &Snapshot) {
        if !self.sandbox_follows_room() {
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
        if !self.sandbox_follows_room() {
            return;
        }
        let (Some(predictor), Some(seat)) = (self.predictor.as_mut(), self.client.seat()) else { return };
        for event in &snapshot.events {
            match *event {
                WireEvent::Fired { slot, weapon, input_tick } if slot as u8 == seat => predictor.note_fired(weapon, input_tick),
                WireEvent::Shoved { seat: s, vx, vy, skid } if s == seat && self.client_hull => {
                    predictor.shove(
                        crate::math::Vec2::new(crate::net::wire::dequantise_velocity(vx), crate::net::wire::dequantise_velocity(vy)),
                        crate::net::wire::dequantise_seconds(skid),
                    );
                }
                _ => {}
            }
        }
    }

    /// The interpolator handed a snapshot's events over: each `Fired` of
    /// this seat's confirms the press that travelled on its input tick
    /// (`Predictor::confirm_fired`), and one with no press waiting seeds
    /// the local gate from the room's. A drawn-on-press weapon's is
    /// `confirm_presses`'. Each
    /// `ShotTeleported` tells the predictor a room shot came through a
    /// portal, for a provisional that went into one to hand over to it
    /// (`Predictor::shot_teleported`).
    fn confirm_shots(&mut self, frame: &Snapshot) {

        if !self.sandbox_follows_room() {
            return;
        }
        let (Some(predictor), Some(seat)) = (self.predictor.as_mut(), self.client.seat()) else { return };
        for event in &frame.events {
            match *event {
                WireEvent::Fired { slot, weapon, input_tick } if slot as u8 == seat => predictor.confirm_fired(weapon, input_tick),
                WireEvent::ShotTeleported { id: Some(id), .. } => predictor.shot_teleported(id),
                _ => {}
            }
        }

    }

    /// The interpolator is handing a snapshot's events over, before they
    /// are applied: each `Fired` of this seat's of a weapon drawn on the
    /// press (`WeaponKind::drawn_on_press`) claims the last press of that
    /// kind this client drew at or before its input tick, earlier ones still
    /// waiting having been refused (`Predictor::confirm_press`), and the
    /// ones that did are the room's shows for this seat - beams, waves - the
    /// replica leaves out: bit `k` for the `k`th such `Fired`
    /// (`apply::Show::OwnShotsDrawn`). A press the client never drew - the
    /// local gate refused one the room fired - is not claimed, so the room's
    /// show is drawn, and it seeds the local gate instead.
    fn confirm_presses(&mut self, frame: &Snapshot) -> u8 {
        if !self.sandbox_follows_room() {
            return 0;
        }
        let (Some(predictor), Some(seat)) = (self.predictor.as_mut(), self.client.seat()) else { return 0 };
        let mut claimed = 0u8;
        let mut k = 0u32;
        for event in &frame.events {
            if let WireEvent::Fired { slot, weapon, input_tick } = *event
                && slot == seat as u16
                && weapon.drawn_on_press()
            {
                if predictor.confirm_press(weapon, input_tick) && k < u8::BITS {
                    claimed |= 1 << k;
                }
                k += 1;
            }
        }
        claimed
    }
}

/// A drone this seat launched on its press (docs/fpv-swarm.md "Wire"),
/// drawn by this client from the press until the room's copy takes over.
///
/// The room launches it `lead` seconds after the press, as the picture
/// runs, so its copy would appear that late, out of a halo slot the client
/// already emptied. Its climb is the same whatever it goes after
/// (`fpv::launch_path`), so the client draws it at once - slowed, its age
/// `since * h / (h + lead)`, so that it is `h` into the climb at `h + lead`
/// - and the room's copy, kept off the picture until it is `h` into the
/// climb too, takes over there, on the same point of the same path.
/// `h` is `FPV_LAUNCH_HANDOVER_SECONDS`. A room's copy struck down in its
/// climb, or gone, takes over at once: the fall is the room's.
#[derive(Clone, Copy, Debug)]
struct OwnDrone {
    /// Its number this round (`OnlineRound::next_own_drone`): its id in the
    /// replica is `OWN_DRONE_ID_BASE` past it, the same every frame, so its
    /// rotors, lamp and particles keep their phase.
    id: u32,
    press: crate::net::predict::DronePress,
    /// Seconds since the press.
    since: f32,
    /// `incoming_lead_ticks` at the press, in seconds.
    lead: f32,
    /// The room's copy - its id and its seconds in the picture - once its
    /// `DroneLaunched` has been handed over and claimed this press.
    room: Option<(u16, f32)>,
}

/// A rod this seat called on its own release (docs/rod-from-god.md
/// "Wire"), drawn by this client as a provisional zone until the room's
/// `RodCalled` for it is handed over - the room's zone, on the same cell,
/// is the call from then on.
#[derive(Clone, Copy, Debug)]
struct OwnCall {
    /// Its number this round (`OnlineRound::next_own_call`): its provisional
    /// zone's id past `zone::PROVISIONAL_ZONE_BASE`, the same every frame it
    /// is drawn.
    id: u32,
    press: crate::net::predict::RodPress,
    /// Seconds since the release.
    since: f32,
    /// When it lands on the replica's clock, set the first frame it is
    /// drawn: the countdown's full length from the present.
    until: Option<f32>,
    /// The room's `RodCalled` for it has been handed over.
    claimed: bool,
}

/// A gravity well orb this seat launched on its own press (docs/gravity-
/// well.md "Wire"), flown by this client until it anchors: from the gun
/// line's muzzle at `well_orb_speed`, bent by the wells on the client's
/// present, meeting the drawn world. `room` is the room's copy it paired
/// with, kept off the picture while this is drawn.
#[derive(Clone, Copy, Debug)]
struct OwnOrb {
    id: u32,
    at: Position,
    velocity: crate::math::Vec2,
    flown: f32,
    age: f32,
    room: Option<u16>,
}

/// A well this seat anchored on its own press or its own orb's contact,
/// drawn as a provisional zone from then until the room's `WellAnchored`
/// claims it - the room's well standing within a tick's flight of it.
#[derive(Clone, Copy, Debug)]
struct OwnAnchor {
    at: Position,
    /// Seconds since it anchored.
    since: f32,
    /// When it starts to pull on the replica's present, set the first
    /// frame it is drawn.
    until: Option<f32>,
    /// The room's copy of the orb that made it, kept off the picture.
    room: Option<u16>,
    claimed: bool,
}

/// Where this client's own orbs' ids start past `PROVISIONAL_ID_BASE`, clear
/// of the provisional shots' (`PROVISIONAL_ID_MASK`).
const OWN_ORB_ID_OFFSET: u32 = 0x0800;

/// `OwnOrb::id` wraps within this past the offset.
const OWN_ORB_ID_MASK: u32 = 0x00FF;

/// Where this client's own wells' zone ids start past
/// `PROVISIONAL_ZONE_BASE`, clear of its rod calls'.
const OWN_WELL_ZONE_OFFSET: u32 = 0x100;

/// An own orb anchors at `at`: the snap and the projector's anchor cell,
/// and a provisional well forming there (`OwnAnchor`).
fn anchor_own(game: &mut Game, anchors: &mut Vec<OwnAnchor>, seat: u8, at: Position, room: Option<u16>) {
    game.show_well_anchor(seat, at);
    anchors.push(OwnAnchor { at, since: 0.0, until: None, room, claimed: false });
}

/// The id a drone drawn by this client carries in the replica: past every
/// id the wire can name (`u16`), so `apply` never takes one for the room's
/// copy of a drone.
const OWN_DRONE_ID_BASE: u32 = 1 << 20;

/// `OwnDrone::id` wraps within this, so `OWN_DRONE_ID_BASE` plus it stays
/// past every id the wire can name.
const OWN_DRONE_ID_MASK: u32 = (1 << 20) - 1;

/// The numbers an own call's provisional zone id takes past
/// `zone::PROVISIONAL_ZONE_BASE`.
const OWN_CALL_ID_MASK: u32 = (1 << 20) - 1;

/// The room's `RodCalled` at `events[called]` is the show of a release this
/// client drew: it claims the call that release drew - the last one waiting
/// at or before the input tick the seat's `Fired` before it names - and any
/// drawn call before that one still waiting was refused, and goes
/// (`claim_own_drone`'s rule).
fn claim_own_call(own: &mut Vec<OwnCall>, events: &[WireEvent], called: usize, seat: u8) {
    let Some(&WireEvent::RodCalled { .. }) = events.get(called) else { return };
    let input_tick = events[..called].iter().rev().find_map(|e| match *e {
        WireEvent::Fired { slot, weapon: crate::net::wire::WeaponKind::RodFromGod, input_tick } if slot == seat as u16 => Some(input_tick),
        _ => None,
    });
    let waiting = |c: &OwnCall| !c.claimed;
    let claimed = match input_tick {
        Some(tick) => own.iter().rposition(|c| waiting(c) && c.press.tick <= tick),
        None => own.iter().position(waiting),
    };
    let Some(i) = claimed else { return };
    own[i].claimed = true;
    let mut k = 0;
    own.retain(|c| {
        let refused = k < i && waiting(c);
        k += 1;
        !refused
    });
}

/// The room's `DroneLaunched` at `events[launched]` is the show of a press
/// this client drew (`apply::Show::presses_drawn`): its copy goes to the
/// launch that press drew - the last one waiting at or before the input
/// tick the seat's `Fired` before it names, as `Predictor::confirm_press`
/// claims the press. Any drawn launch before that one still waiting was
/// refused, and goes.
fn claim_own_drone(own: &mut Vec<OwnDrone>, events: &[WireEvent], launched: usize, seat: u8) {
    let Some(&WireEvent::DroneLaunched { id, .. }) = events.get(launched) else { return };
    let input_tick = events[..launched].iter().rev().find_map(|e| match *e {
        WireEvent::Fired { slot, weapon: crate::net::wire::WeaponKind::FpvSwarm, input_tick } if slot == seat as u16 => Some(input_tick),
        _ => None,
    });
    let waiting = |d: &OwnDrone| d.room.is_none();
    let claimed = match input_tick {
        Some(tick) => own.iter().rposition(|d| waiting(d) && d.press.tick <= tick),
        None => own.iter().position(waiting),
    };
    let Some(i) = claimed else { return };
    own[i].room = Some((id, 0.0));
    let mut k = 0;
    own.retain(|d| {
        let refused = k < i && waiting(d);
        k += 1;
        !refused
    });
}

impl<T: Transport> OnlineRound<T> {
    /// One frame of this seat's own launches (`OwnDrone`): each drawn on its
    /// slowed climb, the room's copy it claimed kept off the picture until
    /// it is `FPV_LAUNCH_HANDOVER_SECONDS` into its own - or at once, if the
    /// room's copy is struck down in its climb or gone from a snapshot
    /// `applied` this frame; then the room's copy is the drone. One the room
    /// never launched - a press it refused - is dropped after the refusal
    /// wait. The seat's halo is drawn without the drones still in flight
    /// here and not yet in the room's count.
    fn fly_own_drones(&mut self, dt: f32, seat: u8, applied: bool) {
        let Some(game) = self.replica.as_mut() else { return };
        let handover = crate::fpv::FPV_LAUNCH_HANDOVER_SECONDS.min(tuning().fpv_launch_seconds);
        let refusal = self.predictor.as_ref().map_or(1.0, |p| p.refusal_after());
        // The room's copies as this frame's snapshot left them: still
        // climbing, or not.
        let room_copies: Vec<(u32, crate::fpv::DroneStage)> = game.drones().iter().map(|d| (d.id, d.stage)).collect();
        for own in &mut self.own_drones {
            own.since += dt;
            if let Some((id, age)) = own.room.as_mut() {
                *age += dt;
                let stage = room_copies.iter().find(|&&(k, _)| k == *id as u32).map(|&(_, stage)| stage);
                let fell = stage.is_some_and(|s| s == crate::fpv::DroneStage::Falling);
                if fell || (applied && stage.is_none()) {
                    *age = handover;
                }
            }
        }
        self.own_drones.retain(|own| match own.room {
            Some((_, age)) => age < handover,
            None => own.since < refusal + handover,
        });
        let hidden: Vec<u32> = self.own_drones.iter().filter_map(|own| own.room.map(|(id, _)| id as u32)).collect();
        game.remove_drones(|id| id >= OWN_DRONE_ID_BASE || hidden.contains(&id));
        let owner = crate::shell::Owner::Player(seat);
        for own in &self.own_drones {
            let age = (own.since * handover / (handover + own.lead.max(0.0))).min(handover);
            let (ground, height) = crate::fpv::launch_path(own.press.origin, own.press.out, age);
            let mut d = crate::fpv::Drone::launch(own.press.origin, own.press.out, own.press.slot, owner, crate::fpv::DroneLock::None, ground);
            d.id = OWN_DRONE_ID_BASE + own.id;
            d.ground = ground;
            d.height = height;
            d.age = own.since;
            d.stage_time = own.since;
            game.add_drone(d);
        }
        let lifting = self.own_drones.iter().filter(|own| own.room.is_none()).count() as u8;
        game.set_seat_lifting(seat, lifting);
        game.count_drones_out();
    }
}

/// Lift the tread-mark trail of every tank whose owner slot is in `slots`,
/// as `net::apply` does for a hull it moves further than driving would:
/// the next `tick_presentation` starts the trail again from where the hull
/// landed instead of pressing marks across the jump.
fn lift_trails(game: &mut Game, slots: &BTreeSet<usize>) {
    if slots.is_empty() {
        return;
    }
    for tank in game.world.query::<&mut Tank>().iter() {
        if slots.contains(&tank.owner_slot()) {
            tank.track_from = None;
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
        // owned hull's play point keeps its own margin and needs none.
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

    /// **Own shells meet opposing ones in the picture** where the room's
    /// `shell_vs_shell` meets them: head-on, between frames, at the
    /// midpoint of their closest approach; a wall nearer along the way
    /// stops the shell first; a shell met once meets nothing else; and a
    /// shot of another kind never meets a shell.
    #[test]
    fn an_own_shell_meets_an_opposing_one_where_the_room_would() {
        use crate::math::Vec2;
        let reach = tuning().shell_hit_half_extent * 2.0;
        // Ours climbs 100 -> 80, theirs falls 60 -> 90: they pass between
        // the frame's two ends.
        let coming = IncomingShell { id: 7, from: Vec2::new(10.0, 60.0), to: Vec2::new(10.0, 90.0) };
        let (from, to) = (Vec2::new(10.0, 100.0), Vec2::new(10.0, 80.0));
        let (at, tank, id) = first_contact(from, to, None, &[coming], reach, &[]).expect("they meet");
        assert_eq!((tank, id), (false, Some(7)));
        assert!((at.x - 10.0).abs() < 0.01 && (80.0..=100.0).contains(&at.y), "met at {at:?}");
        // A wall at y = 95, before the meeting: the wall.
        let wall = Some((Vec2::new(10.0, 95.0), false));
        assert_eq!(first_contact(from, to, wall, &[coming], reach, &[]), Some((Vec2::new(10.0, 95.0), false, None)));
        // A wall past it: the shell.
        let beyond = Some((Vec2::new(10.0, 81.0), false));
        assert_eq!(first_contact(from, to, beyond, &[coming], reach, &[]).map(|c| c.2), Some(Some(7)));
        // Met already this frame: nothing left to meet.
        assert_eq!(first_contact(from, to, None, &[coming], reach, &[(7, at)]), None);
        // Side by side a lane apart: no meeting.
        let wide = IncomingShell { id: 8, from: Vec2::new(60.0, 60.0), to: Vec2::new(60.0, 90.0) };
        assert_eq!(first_contact(from, to, None, &[wide], reach, &[]), None);
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

    /// **A press ripples its muzzle on the press** (§4.16): the provisional
    /// shell leaves the barrel with the ripple a replica puts on a `Fired`,
    /// on the frame of the press and once, where the shell is drawn; the
    /// room's `Fired` for it, handed over a round trip and the picture's
    /// delay later, draws no second one.
    #[test]
    fn a_press_ripples_the_muzzle_once_on_the_press_frame() {
        if !tuning().online_predict_own_tank || !tuning().online_predict_shots {
            return;
        }
        let (mut room, mut round) = room_and_round();
        room.welcome();
        round.frame(&Intent::default(), 1.0 / 60.0);
        // The room's world on its schedule, with nothing happening in it.
        let quiet = |room: &Room, tick: u32| {
            let mut s = encode::snapshot(&room.game, [0; MAX_SEATS]);
            s.events.clear();
            s.tick = tick;
            s.server_ms = 5_000 + ((tick - 1) as f64 * 1000.0 / 60.0).round() as u32;
            s
        };
        for tick in 1..=20u32 {
            let s = quiet(&room, tick);
            room.say(Msg::Snapshot(s));
        }
        round.frame(&Intent::default(), 1.0 / 60.0);
        room.heard();
        // Ripples put on this frame: the presentation tick has aged them
        // once.
        let fresh = |round: &OnlineRound<loopback::Loopback>| -> Vec<crate::math::Vec2> {
            let game = round.game().expect("a replica");
            game.muzzle_flashes.iter().filter(|m| m.time < 1.5 / 60.0).map(|m| m.center).collect()
        };
        assert!(fresh(&round).is_empty(), "nothing fired yet");

        round.frame(&Intent { fire: true, ..Intent::default() }, 1.0 / 60.0);
        let rippled = fresh(&round);
        assert_eq!(rippled.len(), 1, "one ripple on the press frame: {rippled:?}");
        let shell = round
            .game()
            .expect("a replica")
            .drawable_state()
            .shots
            .into_iter()
            .find(|s| s.id >= crate::net::predict::PROVISIONAL_ID_BASE)
            .expect("the provisional shell, drawn on the press");
        let at = crate::math::Vec2::new(shell.x as f32 / 4.0, shell.y as f32 / 4.0);
        assert!(at.distance_to(rippled[0]) <= 0.25, "the ripple is at the muzzle the shell left: {at:?} vs {:?}", rippled[0]);
        let pressed = room
            .heard()
            .iter()
            .find_map(|m| if let Msg::Intent(i) = m { i.fire.then_some(i.tick) } else { None })
            .expect("the press went out");

        // The room fires it. Render time reaches its `Fired` the picture's
        // delay later, in real time.
        let mut fired = quiet(&room, 21);
        fired.events = vec![WireEvent::Fired { slot: 0, weapon: crate::net::wire::WeaponKind::Shell, input_tick: pressed }];
        room.say(Msg::Snapshot(fired));
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        let mut handed_over = false;
        while !handed_over && Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(4));
            round.frame(&Intent::default(), 1.0 / 60.0);
            let game = round.game().expect("a replica");
            handed_over = game.events().iter().any(|e| matches!(e, crate::simulation::Event::Fired { slot: 0, .. }));
            assert!(fresh(&round).is_empty(), "a second ripple for the press: {:?}", fresh(&round));
        }
        assert!(handed_over, "the room's `Fired` reached the picture");
    }

    /// The room's world on its schedule, nothing happening in it, with
    /// this seat's input `tick` applied.
    fn on_schedule(room: &Room, tick: u32) -> Snapshot {
        let mut s = encode::snapshot(&room.game, [0; MAX_SEATS]);
        s.events.clear();
        s.tick = tick;
        s.acked[0] = tick;
        s.server_ms = 5_000 + ((tick - 1) as f64 * 1000.0 / 60.0).round() as u32;
        s
    }

    /// Frames, a few milliseconds apart, until the picture hands over this
    /// seat's `Fired` - render time reaching the snapshot that carried it -
    /// and that frame's events; `None` if it never does. The frames owe no
    /// packets (`dt` 0), so the sandbox stands on the tick it had: render
    /// time runs on the wall clock whatever a frame's `dt` says.
    fn until_handed_over(round: &mut OnlineRound<loopback::Loopback>) -> Option<Vec<crate::simulation::Event>> {
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        while Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(4));
            round.frame(&Intent::default(), 0.0);
            let events = round.game().expect("a replica").events();
            if events.iter().any(|e| matches!(e, crate::simulation::Event::Fired { slot: 0, .. })) {
                return Some(events.to_vec());
            }
        }
        None
    }

    /// **Only a beam this client drew keeps the room's off the picture**
    /// (§4.16): the room fires two beams for this seat in one snapshot, one
    /// for the press the client drew on and one for an input it drew
    /// nothing on - a press its gate refused. The first `Fired` claims
    /// nothing, so its beam is the room's to draw; the second claims the
    /// drawn one, and its beam is not drawn again.
    #[test]
    fn the_rooms_beam_is_left_out_only_for_a_press_this_client_drew() {
        use crate::net::wire::{WeaponKind, quantise_pos};
        if !tuning().online_predict_shots {
            return;
        }
        let (mut room, mut round) = room_and_round();
        round.predict_own_tank = Some(true);
        let laser = crate::simulation::debug::TankPatch { laser_charges: Some(5), ..Default::default() };
        room.game.debug_set_tank(0, &laser).expect("the seat's tank");
        room.welcome();
        round.frame(&Intent::default(), 1.0 / 60.0);
        for tick in 1..=20u32 {
            let s = on_schedule(&room, tick);
            room.say(Msg::Snapshot(s));
        }
        round.frame(&Intent::default(), 1.0 / 60.0);
        room.heard();
        round.frame(&Intent { fire: true, ..Intent::default() }, 1.0 / 60.0);
        let pressed = room
            .heard()
            .iter()
            .find_map(|m| if let Msg::Intent(i) = m { i.fire.then_some(i.tick) } else { None })
            .expect("the press went out");
        assert!(round.game().expect("a replica").laser_beams.len() == 1, "the beam is drawn on the press");
        assert!(pressed > 0, "an input before the press to have fired on");

        let (refused_x, drawn_x) = (160.0f32, 320.0f32);
        let beam = |x: f32| WireEvent::LaserBeam {
            x0: quantise_pos(x),
            y0: quantise_pos(200.0),
            x1: quantise_pos(x),
            y1: quantise_pos(40.0),
            variant: 0,
            seat: 0,
            leg: 0,
            portal: false,
        };
        let mut fired = on_schedule(&room, 21);
        fired.events = vec![
            WireEvent::Fired { slot: 0, weapon: WeaponKind::Laser, input_tick: pressed - 1 },
            beam(refused_x),
            WireEvent::Fired { slot: 0, weapon: WeaponKind::Laser, input_tick: pressed },
            beam(drawn_x),
        ];
        room.say(Msg::Snapshot(fired));
        let events = until_handed_over(&mut round).expect("the room's `Fired` reached the picture");
        let beams: Vec<f32> = events
            .iter()
            .filter_map(|e| if let crate::simulation::Event::LaserBeam { x0, .. } = e { Some(*x0) } else { None })
            .collect();
        assert_eq!(beams, vec![refused_x], "only the beam this client never drew is the room's");
        let drawn = round.game().expect("a replica").laser_beams.iter().any(|b| b.start.x == refused_x);
        assert!(drawn, "and it is drawn");
    }

    /// **A launch is drawn on the press and handed to the room's copy**
    /// (docs/fpv-swarm.md "Wire"): the drone leaves the halo on the press
    /// frame - one fewer in the halo at once -, the room's copy of it is
    /// kept off the picture once its `DroneLaunched` is handed over until
    /// it is as far into the climb, and at no frame are two drones of this
    /// seat drawn.
    #[test]
    fn a_launch_is_drawn_on_the_press_and_handed_to_the_rooms_copy() {
        use crate::net::wire::WeaponKind;
        if !tuning().online_predict_shots {
            return;
        }
        let (mut room, mut round) = room_and_round();
        round.predict_own_tank = Some(true);
        let swarm = crate::simulation::debug::TankPatch { fpv_drones: Some(6), ..Default::default() };
        room.game.debug_set_tank(0, &swarm).expect("the seat's tank");
        room.welcome();
        round.frame(&Intent::default(), 1.0 / 60.0);
        for tick in 1..=20u32 {
            let s = on_schedule(&room, tick);
            room.say(Msg::Snapshot(s));
        }
        round.frame(&Intent::default(), 1.0 / 60.0);
        room.heard();
        let seat_drones = |round: &OnlineRound<loopback::Loopback>| -> Vec<u32> {
            round.game().expect("a replica").drones().iter().filter(|d| d.owner == crate::shell::Owner::Player(0)).map(|d| d.id).collect()
        };
        let halo = |round: &OnlineRound<loopback::Loopback>| -> usize {
            let game = round.game().expect("a replica");
            let entity = game.seat(0).expect("the seat");
            crate::simulation::with_tank(&game.world, entity, |t| crate::game::halo_of(t, game.time).map_or(0, |h| h.drones))
        };
        assert_eq!((seat_drones(&round).len(), halo(&round)), (0, 6), "six in the halo, none up");

        round.frame(&Intent { fire: true, ..Intent::default() }, 1.0 / 60.0);
        let own = seat_drones(&round);
        assert_eq!(own.len(), 1, "one drone up on the press frame: {own:?}");
        assert!(own[0] >= OWN_DRONE_ID_BASE, "the client's own");
        assert_eq!(halo(&round), 5, "and one fewer in the halo at once");
        let pressed = room
            .heard()
            .iter()
            .find_map(|m| if let Msg::Intent(i) = m { i.fire.then_some(i.tick) } else { None })
            .expect("the press went out");

        // The room launches it on its tick 21.
        let (w, h) = room.game.map.field_size();
        room.game.update(Input::single(Intent { fire: true, ..Intent::default() }), PHYSICS_FIXED_DT, w, h);
        let mut launched = on_schedule(&room, 21);
        launched.acked[0] = pressed;
        launched.events = encode::wire_events(room.game.events())
            .into_iter()
            .map(|e| match e {
                WireEvent::Fired { slot, weapon, .. } => WireEvent::Fired { slot, weapon, input_tick: pressed },
                other => other,
            })
            .collect();
        assert!(launched.events.iter().any(|e| matches!(e, WireEvent::Fired { weapon: WeaponKind::FpvSwarm, .. })));
        assert_eq!(launched.drones.len(), 1, "the room's copy in its snapshot");
        let room_id = launched.drones[0].id as u32;
        room.say(Msg::Snapshot(launched));
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        let (mut handed_over, mut swapped) = (false, false);
        while !swapped && Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(4));
            // The drone's climb runs on the frame's time.
            round.frame(&Intent::default(), 0.004);
            let drones = seat_drones(&round);
            assert!(drones.len() <= 1, "two drones of the seat drawn: {drones:?}");
            // The claimed `DroneLaunched` is the client's own and left out
            // of the replica's events; its `Fired` is not.
            handed_over |= round.game().expect("a replica").events().iter().any(|e| matches!(e, crate::simulation::Event::Fired { slot: 0, .. }));
            if handed_over {
                assert_eq!(halo(&round), 5, "the room's count has it");
            }
            swapped = drones == vec![room_id];
        }
        assert!(handed_over, "the room's launch reached the picture");
        assert!(swapped, "and its copy took over from the client's");
    }

    /// **A rod's call is drawn on the release and handed to the room's**
    /// (docs/rod-from-god.md "Wire"): the reticle is the sandbox's from the
    /// press, every packet reports its cell, the release puts a provisional
    /// zone on the picture at once on that cell, and the room's `RodCalled`
    /// - claimed by its `Fired`'s input tick - hands the picture to the
    /// room's zone on the same cell; never two calls of the seat drawn.
    #[test]
    fn a_call_is_drawn_on_the_release_and_handed_to_the_rooms_zone() {
        if !tuning().online_predict_shots {
            return;
        }
        let (mut room, mut round) = room_and_round();
        round.predict_own_tank = Some(true);
        let rods = crate::simulation::debug::TankPatch { rods: Some(2), ..Default::default() };
        room.game.debug_set_tank(0, &rods).expect("the seat's tank");
        room.welcome();
        round.frame(&Intent::default(), 1.0 / 60.0);
        for tick in 1..=20u32 {
            let s = on_schedule(&room, tick);
            room.say(Msg::Snapshot(s));
        }
        round.frame(&Intent::default(), 1.0 / 60.0);
        room.heard();
        let seat_zones = |round: &OnlineRound<loopback::Loopback>| -> Vec<(u32, (i32, i32))> {
            round.game().expect("a replica").zones().iter().filter_map(|z| z.rod().map(|c| (z.id, c.cell))).collect()
        };
        let held = crate::tank::ticks_of(tuning().rod_settle_seconds) + 4;
        for _ in 0..held {
            round.frame(&Intent { fire: true, ..Intent::default() }, 1.0 / 60.0);
        }
        let reticle = round.game().expect("a replica").seat_reticle(0).expect("the reticle drawn from the press").cell;
        // The trigger goes up on the packets after `FIRE_HOLD_TICKS`: the
        // call is drawn on the frame the release is stepped.
        let mut drawn = Vec::new();
        for _ in 0..=crate::net::client::FIRE_HOLD_TICKS {
            round.frame(&Intent::default(), 1.0 / 60.0);
            drawn = seat_zones(&round);
            if !drawn.is_empty() {
                break;
            }
        }
        assert_eq!(drawn.len(), 1, "one call drawn on the release: {drawn:?}");
        assert!(drawn[0].0 >= crate::zone::PROVISIONAL_ZONE_BASE, "the client's own");
        assert_eq!(drawn[0].1, reticle, "on the reticle's cell");
        let intents: Vec<crate::net::wire::IntentMsg> = room.heard().into_iter().filter_map(|m| if let Msg::Intent(i) = m { Some(i) } else { None }).collect();
        let cols = encode::field_cols(&room.game);
        assert!(intents.iter().filter(|i| i.fire).all(|i| encode::reticle_from_code(cols, i.reticle) == Some(reticle)), "every held packet reports the cell");
        let released = intents.iter().find(|i| !i.fire && i.reticle != 0).expect("the release reports it too");
        assert_eq!(encode::reticle_from_code(cols, released.reticle), Some(reticle));

        // The room calls on the same cell on its tick 21: the release.
        let (w, h) = room.game.map.field_size();
        let at = crate::map::cell_to_world(reticle.0, reticle.1);
        let id = room.game.debug_call_rod(at, false).expect("a call");
        room.game.events.insert(0, crate::simulation::Event::Fired { slot: 0, weapon: "rod_from_god" });
        let mut called = on_schedule(&room, 21);
        called.acked[0] = released.tick;
        called.events = encode::wire_events(room.game.events())
            .into_iter()
            .map(|e| match e {
                WireEvent::Fired { slot, weapon, .. } => WireEvent::Fired { slot, weapon, input_tick: released.tick },
                other => other,
            })
            .collect();
        let _ = (w, h);
        assert_eq!(called.zones.len(), 1, "the room's zone in its snapshot");
        room.say(Msg::Snapshot(called));
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        let mut swapped = false;
        while !swapped && Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(4));
            round.frame(&Intent::default(), 0.004);
            let zones = seat_zones(&round);
            assert!(zones.len() <= 1, "two calls of the seat drawn: {zones:?}");
            swapped = zones == vec![(id & 0xFFFF, reticle)];
        }
        assert!(swapped, "the room's zone took over from the client's");
    }

    /// **A launch the room downs in its climb is handed over at once**:
    /// the room's copy, struck two ticks after its launch, is the drone the
    /// picture shows as soon as its fall reaches the picture - not the
    /// client's still climbing for the rest of the hand-over - and at no
    /// frame are two drones of the seat drawn.
    #[test]
    fn a_launch_downed_in_its_climb_is_handed_to_the_rooms_fall_at_once() {
        use crate::net::wire::WeaponKind;
        if !tuning().online_predict_shots {
            return;
        }
        let (mut room, mut round) = room_and_round();
        round.predict_own_tank = Some(true);
        let swarm = crate::simulation::debug::TankPatch { fpv_drones: Some(6), ..Default::default() };
        room.game.debug_set_tank(0, &swarm).expect("the seat's tank");
        room.welcome();
        round.frame(&Intent::default(), 1.0 / 60.0);
        for tick in 1..=20u32 {
            let s = on_schedule(&room, tick);
            room.say(Msg::Snapshot(s));
        }
        round.frame(&Intent::default(), 1.0 / 60.0);
        room.heard();
        round.frame(&Intent { fire: true, ..Intent::default() }, 1.0 / 60.0);
        let pressed = room
            .heard()
            .iter()
            .find_map(|m| if let Msg::Intent(i) = m { i.fire.then_some(i.tick) } else { None })
            .expect("the press went out");
        let (w, h) = room.game.map.field_size();
        room.game.update(Input::single(Intent { fire: true, ..Intent::default() }), PHYSICS_FIXED_DT, w, h);
        let mut launched = on_schedule(&room, 21);
        launched.acked[0] = pressed;
        launched.events = encode::wire_events(room.game.events())
            .into_iter()
            .map(|e| match e {
                WireEvent::Fired { slot, weapon, .. } => WireEvent::Fired { slot, weapon, input_tick: pressed },
                other => other,
            })
            .collect();
        assert!(launched.events.iter().any(|e| matches!(e, WireEvent::Fired { weapon: WeaponKind::FpvSwarm, .. })));
        let room_id = launched.drones[0].id as u32;
        room.say(Msg::Snapshot(launched));
        // A tesla's arc takes it on the room's next tick.
        for d in room.game.world.query_mut::<&mut crate::fpv::Drone>() {
            d.down(crate::air::AirStrike::Tesla);
        }
        room.game.update(Input::single(Intent::default()), PHYSICS_FIXED_DT, w, h);
        let downed = on_schedule(&room, 22);
        assert_eq!(downed.drones.first().map(|d| d.stage), Some(crate::fpv::DroneStage::Falling.code()), "falling in the room");
        room.say(Msg::Snapshot(downed));
        let seat_drones = |round: &OnlineRound<loopback::Loopback>| -> Vec<(u32, crate::fpv::DroneStage)> {
            round
                .game()
                .expect("a replica")
                .drones()
                .iter()
                .filter(|d| d.owner == crate::shell::Owner::Player(0))
                .map(|d| (d.id, d.stage))
                .collect()
        };
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        let mut claimed_at: Option<Instant> = None;
        let mut fall_shown = None;
        while fall_shown.is_none() && Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(4));
            round.frame(&Intent::default(), 0.004);
            let drones = seat_drones(&round);
            assert!(drones.len() <= 1, "two drones of the seat drawn: {drones:?}");
            if claimed_at.is_none()
                && round.game().expect("a replica").events().iter().any(|e| matches!(e, crate::simulation::Event::Fired { slot: 0, .. }))
            {
                claimed_at = Some(Instant::now());
            }
            if drones == vec![(room_id, crate::fpv::DroneStage::Falling)] {
                fall_shown = claimed_at.map(|at| at.elapsed());
            }
        }
        let fall_shown = fall_shown.expect("the room's falling copy took over");
        assert!(
            fall_shown.as_secs_f32() < crate::fpv::FPV_LAUNCH_HANDOVER_SECONDS * 0.6,
            "the fall is shown once it reaches the picture, not at the end of the hand-over: {fall_shown:?}"
        );
    }

    /// **A launch the room refuses goes**: drawn on the press, never
    /// claimed - the room's snapshots go on with six in the halo and no
    /// `Fired` - it is taken off the picture after the refusal wait and the
    /// hand-over, and the halo and the HUD have the drone back.
    #[test]
    fn a_refused_launch_goes_and_the_halo_has_its_drone_back() {
        if !tuning().online_predict_shots {
            return;
        }
        let (mut room, mut round) = room_and_round();
        round.predict_own_tank = Some(true);
        let swarm = crate::simulation::debug::TankPatch { fpv_drones: Some(6), ..Default::default() };
        room.game.debug_set_tank(0, &swarm).expect("the seat's tank");
        room.welcome();
        round.frame(&Intent::default(), 1.0 / 60.0);
        for tick in 1..=20u32 {
            let s = on_schedule(&room, tick);
            room.say(Msg::Snapshot(s));
        }
        round.frame(&Intent::default(), 1.0 / 60.0);
        let count = |round: &OnlineRound<loopback::Loopback>| {
            crate::hud::HudModel::gather(round.game().expect("a replica"), Some(0)).local.weapon.count
        };
        let seat_drones = |round: &OnlineRound<loopback::Loopback>| {
            round.game().expect("a replica").drones().iter().filter(|d| d.owner == crate::shell::Owner::Player(0)).count()
        };
        round.frame(&Intent { fire: true, ..Intent::default() }, 1.0 / 60.0);
        assert_eq!((seat_drones(&round), count(&round)), (1, 5), "drawn on the press, the count one down");
        // The room never launches it: its snapshots go on as they were.
        let deadline = Instant::now() + std::time::Duration::from_secs(4);
        let mut tick = 21u32;
        while seat_drones(&round) > 0 && Instant::now() < deadline {
            let s = on_schedule(&room, tick);
            room.say(Msg::Snapshot(s));
            tick += 1;
            std::thread::sleep(std::time::Duration::from_millis(4));
            round.frame(&Intent::default(), 1.0 / 60.0);
        }
        assert_eq!(seat_drones(&round), 0, "the refused launch left the picture");
        assert_eq!(count(&round), 6, "and the halo has its drone back");
    }

    /// A drone the room launched is the drawn launch of the press its
    /// `Fired` names - the last one waiting at or before that input tick -
    /// and any drawn launch before that one still waiting was refused.
    #[test]
    fn the_rooms_launch_claims_the_drawn_launch_of_its_input_tick() {
        use crate::net::wire::WeaponKind;
        let press = |tick: u32| crate::net::predict::DronePress { slot: 5, origin: crate::math::Vec2::zero(), out: crate::math::Vec2::new(0.0, -1.0), tick };
        let own = |id: u32, tick: u32| OwnDrone { id, press: press(tick), since: 0.0, lead: 0.1, room: None };
        let launch = |input_tick: u32, id: u16| {
            vec![
                WireEvent::Fired { slot: 0, weapon: WeaponKind::FpvSwarm, input_tick },
                WireEvent::DroneLaunched { id, slot: 0, x: 0, y: 0, target: crate::net::events::NO_TARGET, frog: false },
            ]
        };
        // The room fired the second press: the first was refused.
        let mut drawn = vec![own(0, 10), own(1, 20)];
        claim_own_drone(&mut drawn, &launch(21, 7), 1, 0);
        assert_eq!(drawn.iter().map(|d| (d.id, d.room.map(|r| r.0))).collect::<Vec<_>>(), vec![(1, Some(7))]);
        // The room fired the first: the second still waits for its own.
        let mut drawn = vec![own(0, 10), own(1, 20)];
        claim_own_drone(&mut drawn, &launch(12, 8), 1, 0);
        assert_eq!(drawn.iter().map(|d| (d.id, d.room.map(|r| r.0))).collect::<Vec<_>>(), vec![(0, Some(8)), (1, None)]);
        // Another seat's `Fired` in between names nothing of this seat's.
        let mut drawn = vec![own(0, 10), own(1, 20)];
        let mut events = launch(21, 9);
        events.insert(1, WireEvent::Fired { slot: 1, weapon: WeaponKind::FpvSwarm, input_tick: 12 });
        claim_own_drone(&mut drawn, &events, 2, 0);
        assert_eq!(drawn.iter().map(|d| (d.id, d.room.map(|r| r.0))).collect::<Vec<_>>(), vec![(1, Some(9))]);
    }

    /// **An owned hull's sandbox follows the room with the prediction's
    /// knob off** (§4.14): its poses are the seat's whether or not the
    /// prediction is drawn, so it takes the room's world - here the seat
    /// out of shells, so a press kicks nothing -, the room's shoves, and
    /// the room's `Fired`, which holds the local gate.
    #[test]
    fn an_owned_hull_follows_the_room_with_the_prediction_knob_off() {
        use crate::net::wire::{WeaponKind, quantise_velocity};
        let (mut room, mut round) = room_and_round();
        round.set_client_hull(true);
        round.predict_own_tank = Some(false);
        room.welcome();
        round.frame(&Intent::default(), 1.0 / 60.0);
        let dry = crate::simulation::debug::TankPatch { shells_ammo: Some(0), ..Default::default() };
        room.game.debug_set_tank(0, &dry).expect("the seat's tank");
        for tick in 1..=20u32 {
            let s = on_schedule(&room, tick);
            room.say(Msg::Snapshot(s));
        }
        round.frame(&Intent::default(), 1.0 / 60.0);
        let still = round.own_hull_at().expect("a hull");

        // The room's word is no shells: the press is refused here too, and
        // kicks nothing.
        round.frame(&Intent { fire: true, ..Intent::default() }, 1.0 / 60.0);
        for _ in 0..5 {
            round.frame(&Intent::default(), 1.0 / 60.0);
        }
        let after = round.own_hull_at().expect("a hull");
        assert!((after.0 - still.0).abs() + (after.1 - still.1).abs() < 0.01, "a press on no shells kicked the hull: {still:?} -> {after:?}");
        assert_eq!(round.prediction().map(|p| p.cooldown), Some(0.0), "and opened no cooldown");

        // The room shoved the hull: the poses carry it on.
        let mut shoved = on_schedule(&room, 21);
        shoved.events = vec![WireEvent::Shoved { seat: 0, vx: quantise_velocity(240.0), vy: 0, skid: 0 }];
        room.say(Msg::Snapshot(shoved));
        for _ in 0..6 {
            round.frame(&Intent::default(), 1.0 / 60.0);
        }
        let pushed = round.own_hull_at().expect("a hull");
        assert!(pushed.0 > after.0 + 1.0, "the shove did not move the owned hull: {after:?} -> {pushed:?}");

        // The room fired for the seat on its latest input: handed over, its
        // `Fired` holds the local gate.
        let mut fired = on_schedule(&room, 22);
        let latest = round.client.intent_tick() - 1;
        fired.events = vec![WireEvent::Fired { slot: 0, weapon: WeaponKind::Shell, input_tick: latest }];
        room.say(Msg::Snapshot(fired));
        until_handed_over(&mut round).expect("the room's `Fired` reached the picture");
        let cooldown = round.prediction().map_or(0.0, |p| p.cooldown);
        assert!(cooldown > 0.0, "the room's shot did not hold the local gate");
    }

    /// **The lobby sends nothing** (§4.14): a seat welcomed into a waiting
    /// room, or back in the room after a round, has no round to steer, and
    /// an owned hull's packets would carry the last round's pose into the
    /// next. Intents go out only while the round is playing, and a trigger
    /// pulled in the lobby is not carried into the round.
    #[test]
    fn the_lobby_sends_no_intents_before_or_after_a_round() {
        let (mut room, mut round) = room_and_round();
        round.set_client_hull(true);
        let intents = |room: &mut Room| -> Vec<crate::net::wire::IntentMsg> {
            room.heard().into_iter().filter_map(|m| if let Msg::Intent(i) = m { Some(i) } else { None }).collect()
        };
        let fire = Intent { move_dir: Some(Dir::Up), fire: true, ..Intent::default() };
        // A waiting room's welcome: seated, no round yet.
        let roster = vec![Seat { seat: 0, nick: "host".into(), chassis: 3 }];
        let mut waiting = encode::welcome(&room.game, 0, roster, "{}".into(), [0; MAX_SEATS]).expect("the map serialises");
        waiting.protocol = PROTOCOL_VERSION;
        waiting.snapshot.server_ms = room.server_ms;
        room.say(Msg::Lobby(Lobby::RoomCreated { code: "AK7QX".into() }));
        room.say(Msg::Welcome(waiting));
        for _ in 0..10 {
            round.frame(&fire, 1.0 / 60.0);
        }
        assert_eq!(*round.phase(), Phase::Lobby);
        assert!(intents(&mut room).is_empty(), "a seat in a waiting room steered nothing");

        // The round starts: one packet a tick, and no trigger from the lobby.
        room.welcome();
        round.frame(&Intent::default(), 1.0 / 60.0);
        assert_eq!(*round.phase(), Phase::Playing);
        for _ in 0..4 {
            round.frame(&Intent::default(), 1.0 / 60.0);
        }
        let playing = intents(&mut room);
        assert_eq!(playing.len(), 5, "one packet a tick while the round plays");
        assert!(playing.iter().all(|i| !i.fire), "a trigger pulled in the lobby rode into the round");
        assert!(playing.iter().all(|i| i.owned), "and each carries the owned hull's pose");

        // The round is over: back in the lobby, nothing goes out.
        room.say(Msg::Lobby(Lobby::Ended { outcome: RoundOutcome::Won }));
        for _ in 0..10 {
            round.frame(&fire, 1.0 / 60.0);
        }
        assert_eq!(*round.phase(), Phase::Lobby);
        assert!(intents(&mut room).is_empty(), "the lobby sent the finished round's poses");
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
        room.say(Msg::Lobby(Lobby::Error { refusal: crate::net::wire::Refusal::RoomFull { seats: 8 } }));
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
        // A reading is the room's send time less the arrival: the arrival
        // it was taken at is the send time less the estimate.
        let offset = round.interp().clock().offset_ms().expect("the welcome was read");
        let arrived = room.server_ms as f64 - offset;
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
