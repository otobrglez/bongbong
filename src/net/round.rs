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

use std::time::Instant;

use crate::ai::Intent;
use crate::net::apply;
use crate::net::client::{ClientEvent, Phase, RoomClient};
use crate::net::interp::{InterpReport, Interpolator};
use crate::net::mailbox;
use crate::net::predict::{PredictionReport, Predictor};
use crate::net::transport::Transport;
use crate::net::events::WireEvent;
use crate::net::wire::{RoundOutcome, Snapshot, Welcome};
use crate::simulation::Game;
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
        self.send(intent, dt);
        self.draw(dt, now);
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
                match self.interp.lead_ms(self.local_ms()).unwrap_or(0.0).round() as i64 {
                    lead if lead >= 0 => format!("{} {code} - SEAT {seat} - BUFFER {lead} MS", self.label),
                    _ => format!("{} {code} - SEAT {seat} - WAITING FOR THE ROOM", self.label),
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

    /// Everything the room has said since the last frame.
    fn poll(&mut self, now: i64) {
        let mut events = std::mem::take(&mut self.scratch);
        events.clear();
        self.client.poll(&mut events);
        for event in events.drain(..) {
            match event {
                ClientEvent::Welcomed(welcome) => self.welcomed(&welcome, now),
                ClientEvent::Snapshot(snapshot) => {
                    self.reconcile(&snapshot);
                    self.note_fired(&snapshot);
                    if let Some(seat) = self.client.seat()
                        && let Some(&state) = snapshot.mailbox.get(seat as usize)
                    {
                        self.lead.observe(state);
                    }
                    self.interp.accept(*snapshot, now);
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
    fn welcomed(&mut self, welcome: &Welcome, now: i64) {
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
                self.predictor = apply::welcome(welcome)
                    .ok()
                    .map(|sandbox| Predictor::new(sandbox, welcome.seat as usize, self.client.intent_tick()));
                self.replica = Some(game);
                self.interp.restart(&welcome.snapshot, now);
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
        self.send_owed = (self.send_owed + dt.max(0.0)).min(2.0 * PHYSICS_FIXED_DT);
        if self.send_owed < PHYSICS_FIXED_DT {
            return;
        }
        self.send_owed -= PHYSICS_FIXED_DT;
        let packets = match self.lead.decide() {
            Adjust::Keep => 1,
            Adjust::Extra => 2,
            Adjust::Skip => 0,
        };
        for _ in 0..packets {
            self.send_one(intent);
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
        let Some(sent) = self.client.send_intent(&out) else { return };
        self.pending_fire = false;
        if let Some(predictor) = self.predictor.as_mut() {
            // The knob is live: read each tick, so a shot pressed after
            // it was turned on is drawn and one after it was turned off
            // is not.
            predictor.set_shots_enabled(tuning().online_predict_shots);
            predictor.step_at(sent.tick, sent.intent());
        }
    }

    /// The picture at render time, then the cosmetics of one frame.
    fn draw(&mut self, dt: f32, now: i64) {
        let Some(game) = self.replica.as_mut() else { return };
        let sampled = self.interp.sample(now);
        if let Some(frame) = &sampled {
            apply::snapshot(game, &frame.snapshot);
            self.confirm_shots(&frame.snapshot);
        }
        // **Before `tick_presentation`, not after.** The presentation
        // pass eases `visual_rotation` toward `rotation` and presses the
        // tread marks out of the hull's displacement, so it has to run on
        // the pose that will actually be drawn. Writing the prediction
        // after it left the rendered hull chasing the server's facing
        // from `online_interpolation_delay_ms` ago while its position was
        // already at the present - a tank that slides without turning.
        self.write_predicted();
        let Some(game) = self.replica.as_mut() else { return };
        game.tick_presentation(dt);
        if let Some(frame) = &sampled {
            // `apply` puts the clock on the snapshot's own tick; the
            // picture stands a fraction of an interval past it, and the
            // water, the fire loops and the sprite cycles read it.
            game.time = frame.snapshot.tick as f32 * PHYSICS_FIXED_DT + frame.ahead;
        }
        if let Some(predictor) = self.predictor.as_mut() {
            predictor.decay(dt);
            predictor.advance_shots(dt);
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
    fn write_predicted(&mut self) {
        if !tuning().online_predict_own_tank {
            return;
        }
        let (Some(predictor), Some(game)) = (self.predictor.as_ref(), self.replica.as_mut()) else { return };
        let (Some(seat), Some(position)) = (self.client.seat(), predictor.drawn_position()) else { return };
        // The velocity is the prediction's own, not zero: `fx` reads it
        // for the spray and the dust, and a hull the solver believes is
        // stopped settles differently from one that is moving.
        let Some((_, rotation, velocity)) = predictor.motion() else { return };
        // The replica's own boost flag is the server's and already
        // applied by `apply::snapshot`; the prediction only moves the
        // hull, so it is carried through unchanged.
        game.place_seat(seat as usize, position, rotation, velocity);
        // Beside the server's shots, not instead of them: `apply` has
        // just despawned everything the snapshot did not list, so these
        // are put back every frame until the server confirms or refuses
        // them.
        for (id, shot) in predictor.shots() {
            game.add_provisional_shot(id, &shot);
        }
        // The flamethrower's stream is a flag the wire carries; while the
        // local key is down the seat is drawn streaming at once, if it is
        // armed and fuelled, rather than a round trip later. Released,
        // the flag is the server's again and goes out when its word does.
        if self.trigger_down && tuning().online_predict_shots {
            game.hold_flame(seat as usize);
        }
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

    /// A snapshot *arrived* carrying this seat's `Fired`: the press it
    /// names has its ammo in the snapshot the sandbox just took, so it
    /// stops counting against the local gate (`Predictor::note_fired`).
    /// The shot itself is retired later, when the frame reaches it.
    fn note_fired(&mut self, snapshot: &Snapshot) {
        if !tuning().online_predict_own_tank {
            return;
        }
        let (Some(predictor), Some(seat)) = (self.predictor.as_mut(), self.client.seat()) else { return };
        for event in &snapshot.events {
            if matches!(event, WireEvent::Fired { slot, .. } if *slot as u8 == seat) {
                predictor.note_fired();
            }
        }
    }

    /// The interpolator handed a snapshot's events over: each `Fired` of
    /// this seat's retires the oldest press still drawn, on this frame,
    /// the one the server's own shot appears in - so the provisional and
    /// the real shot swap places rather than leaving a gap. A `Fired`
    /// with no press waiting seeds the local gate from the room's
    /// (`Predictor::confirm_shot`), measured back from the snapshot's
    /// `acked` to the tick the client is on now.
    fn confirm_shots(&mut self, frame: &Snapshot) {
        if !tuning().online_predict_own_tank {
            return;
        }
        let (Some(predictor), Some(seat)) = (self.predictor.as_mut(), self.client.seat()) else { return };
        let acked = frame.acked.get(seat as usize).copied().unwrap_or(0);
        let ticks_ago = predictor.tick().wrapping_sub(acked);
        for event in &frame.events {
            if let WireEvent::Fired { slot, weapon } = event
                && *slot as u8 == seat
            {
                predictor.confirm_shot(*weapon, ticks_ago);
            }
        }
    }
}

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
}
