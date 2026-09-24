//! The offline rig (docs/online-coop-prd.md §4.13): an authoritative
//! `Game` on a background thread, the window's replica on the other end
//! of a `loopback` link, and a dial for delay, jitter and loss. No
//! server, no socket, no port - the whole client half of online play,
//! from the encoder to the feel of the hull at 80 ms, in one process.
//!
//! The thread runs the shape of the room server's loop
//! (`bongbong_server::room`): sample the seat's mailbox into an `Input`,
//! `Game::update` at `PHYSICS_FIXED_DT`, and every `SNAPSHOT_EVERY` ticks
//! encode a snapshot and send it. Two things differ, both on purpose.
//! The lobby is one seat: the create is answered with the code, the
//! roster, the start and the `Welcome` in one breath, so `--rig` puts a
//! round on screen with nothing to press. And every snapshot goes in
//! full rather than as a delta: a delta chain is broken by the one thing
//! the dial is here to produce, a lost packet, and the rig has no ack
//! path to ask for a fresh baseline with. A full snapshot makes a loss
//! exactly what the replica has to survive - a gap - which is the point
//! of `--loss`.
//!
//! The end of a round is the room server's: the end screen is ticked out
//! and the round is announced over on the tick before `Game::update`
//! would restart it, so a `--rig` round comes back to the lobby the way
//! a room's does rather than quietly starting again.
//!
//! The round runs without the mission banner, as a room server's does,
//! so the picture moves from the first frame.
//!
//! Threads mean this is not the emscripten build's; the web reaches a
//! room over its own socket.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

use crate::ai::Intent;
use crate::level::LevelOverrides;
use crate::map::MapFile;
use crate::net::apply;
use crate::net::client::{ClientEvent, Identity, RoomClient, RoomSetup};
use crate::net::codec::{self, Msg};
use crate::net::encode;
use crate::net::loopback::{self, LinkQuality, Loopback};
use crate::net::transport::Transport;
use crate::net::wire::{IntentMsg, Lobby, RosterSeat, RoundOutcome, Seat, Snapshot};
use crate::net::MAX_SEATS;
use crate::simulation::{Game, Input, Outcome};
use crate::PHYSICS_FIXED_DT;

/// The code the rig's room answers with. It is `CODE_LETTERS` long, so
/// the status line and a screenshot look like the real thing - the
/// letters are not the room server's alphabet, which is the giveaway
/// that no server was dialled.
pub const RIG_CODE: &str = "RIG00";

/// A snapshot every this many ticks: 20 Hz, the room server's cadence
/// (`bongbong_server::room::SNAPSHOT_EVERY`).
pub const SNAPSHOT_EVERY: u64 = 3;

/// How long a tick keeps repeating the seat's last intent with nothing
/// newer arrived, the room server's `INTENT_COAST`: a hiccup coasts
/// rather than stops, a disconnect stops.
const INTENT_COAST: Duration = Duration::from_millis(500);

/// What the rig's round is.
#[derive(Clone, Debug)]
pub struct RigOptions {
    pub map: MapFile,
    /// Pin the round's seed; absent, `Game::init` draws one.
    pub seed: Option<u64>,
    pub enemies: Option<usize>,
    pub overrides: LevelOverrides,
    /// The seat's chassis row; absent, the map's or a roll.
    pub tank_row: Option<i32>,
    /// Delay, jitter and loss on the link, both ways.
    pub quality: LinkQuality,
    /// The link's own RNG seed, so a run's parcels replay.
    pub link_seed: u64,
}

impl Default for RigOptions {
    fn default() -> RigOptions {
        RigOptions {
            map: MapFile::new(),
            seed: None,
            enemies: None,
            overrides: LevelOverrides::default(),
            tank_row: None,
            quality: LinkQuality::new(80, 20, 0.02),
            link_seed: 0xB0B5,
        }
    }
}

/// The running rig. Dropping it stops the thread and closes the link, so
/// a window that closes takes its authority with it.
pub struct Rig {
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
    tick: Arc<AtomicU64>,
    latest: Arc<Mutex<Option<Snapshot>>>,
}

impl Rig {
    /// The authoritative round's tick.
    pub fn tick(&self) -> u64 {
        self.tick.load(Ordering::Relaxed)
    }

    /// The newest snapshot the authority sent: what the replica is
    /// catching up to, for a status line or a test that wants the truth
    /// rather than the picture.
    pub fn authority(&self) -> Option<Snapshot> {
        self.latest.lock().expect("the rig thread did not panic").clone()
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// Start the rig and hand back the window's end of the link. The round
/// itself waits for the client's create, so nothing is simulated until
/// somebody is seated.
pub fn start(options: RigOptions) -> (Rig, Loopback) {
    let (client_end, room_end) = loopback::pair(options.quality, options.link_seed);
    let stop = Arc::new(AtomicBool::new(false));
    let room = Room::new(room_end, options);
    // The two the window reads while the thread writes them.
    let tick = Arc::clone(&room.tick);
    let latest = Arc::clone(&room.latest);
    let flag = Arc::clone(&stop);
    let thread = thread::Builder::new()
        .name("bongbong-rig".into())
        .spawn(move || run(room, flag))
        .expect("the rig thread starts");
    (Rig { stop, thread: Some(thread), tick, latest }, client_end)
}

/// The rig with no thread and no clock: the authoritative round, the
/// link and the replica all on the caller's, stepped a tick at a time.
///
/// `step` is to a networked round what the dev server's `step` is to a
/// local one (docs/dev-server-design.md section 3): the same options and
/// the same steps give the same two rounds, byte for byte. The room's
/// own `now` advances by exactly `PHYSICS_FIXED_DT` a tick, so even
/// `Snapshot::server_ms` replays, and the link hands everything over at
/// once, so nothing depends on when a step was taken. After a step the
/// replica stands on the last snapshot the authority cut, which is the
/// tick `authority()` - the round itself, not just its snapshot, since
/// it is on this thread - can be compared against.
///
/// Two things the threaded rig has are deliberately missing.
///
/// The link is perfect. Delay and jitter are dials on *real* time - a
/// parcel becomes readable at an `Instant` and `Transport::drain` reads
/// the wall clock - so no amount of stepping makes them repeatable;
/// judging the feel of a delayed round is `--rig --delay`'s job. Loss
/// would replay (it is a seeded roll, not a clock), but a snapshot
/// dropped here is only a tick the replica never stands on, so nothing
/// is dropped either.
///
/// And nothing is interpolated: `net::interp` measures render time
/// against the wall clock too, so the replica is put *on* the newest
/// snapshot's tick rather than a fraction past it. That is the seam
/// `net::apply`'s round trips already compare on; the smoothing between
/// two snapshots stays the windowed round's (`net::round`).
pub struct Lockstep {
    room: Room,
    client: RoomClient<Loopback>,
    replica: Option<Game>,
    /// The seat's command, sent every tick until it is changed.
    intent: Intent,
    /// What the room refused, if anything.
    note: Option<String>,
    /// How the round the room finished went, `None` while one runs.
    ended: Option<RoundOutcome>,
    /// Reused by `catch_up` so a step allocates nothing.
    scratch: Vec<ClientEvent>,
}

impl Lockstep {
    /// A room, a seat in it and the replica it welcomed, all stepped by
    /// hand. The round has begun by the time this returns, exactly as
    /// `--rig`'s one-seat lobby begins it.
    pub fn start(options: RigOptions) -> Lockstep {
        let (client_end, room_end) = loopback::pair(LinkQuality::PERFECT, options.link_seed);
        let setup = RoomSetup { seed: options.seed, ..RoomSetup::default() };
        let mut rig = Lockstep {
            room: Room::new(room_end, options),
            client: RoomClient::host(client_end, Identity::new("rig", "tok-rig"), setup),
            replica: None,
            intent: Intent::default(),
            note: None,
            ended: None,
            scratch: Vec::new(),
        };
        // One turn each: the client says hello, the room answers with the
        // code, the roster, the start and the welcome, and the client
        // builds the replica from it.
        rig.catch_up();
        rig.room.pump();
        rig.catch_up();
        rig
    }

    /// What the seat is doing from here on: repeated every tick until it
    /// is changed, the way a held key reaches a local round.
    pub fn drive(&mut self, intent: Intent) {
        self.intent = intent;
    }

    /// Run the authority `ticks` ticks and let the replica catch up to
    /// the last snapshot they earned. Returns the replica's tick.
    pub fn step(&mut self, ticks: u64) -> u64 {
        let step = Duration::from_secs_f32(PHYSICS_FIXED_DT);
        for _ in 0..ticks {
            self.client.send_intent(&self.intent);
            self.room.now += step;
            if !self.room.pump() {
                break;
            }
            self.room.tick();
        }
        self.catch_up();
        self.replica.as_ref().map_or(0, Game::frame)
    }

    /// The round the room is simulating: the truth a test checks the
    /// picture against.
    pub fn authority(&self) -> Option<&Game> {
        self.room.game.as_ref()
    }

    /// The replica the seat draws, once the welcome has built one.
    pub fn replica(&self) -> Option<&Game> {
        self.replica.as_ref()
    }

    /// The authoritative round's tick.
    pub fn tick(&self) -> u64 {
        self.room.game.as_ref().map_or(0, Game::frame)
    }

    /// The seat this client was given.
    pub fn seat(&self) -> Option<u8> {
        self.client.seat()
    }

    /// The room's code (`RIG_CODE`).
    pub fn code(&self) -> Option<&str> {
        self.client.code()
    }

    /// The last thing the room refused, if it refused anything.
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }

    /// How the round the room finished went, once it has said so;
    /// `None` while one is running. The room stops ticking here, so a
    /// later `step` moves nothing until `rematch`.
    pub fn ended(&self) -> Option<RoundOutcome> {
        self.ended
    }

    /// Play again: the `Start` the room server takes from a host once a
    /// round is over, on a fresh seed and a fresh `Welcome`.
    pub fn rematch(&mut self) {
        self.client.start();
        self.catch_up();
        self.room.pump();
        self.catch_up();
    }

    /// The newest snapshot the authority cut, stamped with the room's
    /// clock - the bytes a test can compare between two runs.
    pub fn snapshot(&self) -> Option<Snapshot> {
        self.room.latest.lock().expect("nothing else holds this lock").clone()
    }

    /// Everything the room has said since the last step, applied to the
    /// replica in the order it arrived.
    fn catch_up(&mut self) {
        let mut events = std::mem::take(&mut self.scratch);
        events.clear();
        self.client.poll(&mut events);
        for event in events.drain(..) {
            match event {
                // The rig's welcome carries the empty tuning patch, so
                // there is nothing to stage before the replica is built.
                ClientEvent::Welcomed(welcome) => match apply::welcome(&welcome) {
                    Ok(game) => self.replica = Some(game),
                    Err(e) => self.note = Some(e),
                },
                ClientEvent::Snapshot(snapshot) => {
                    if let Some(replica) = self.replica.as_mut() {
                        apply::snapshot(replica, &snapshot);
                    }
                }
                ClientEvent::Refused(message) => self.note = Some(message),
                ClientEvent::Closed(closed) => self.note = Some(closed.reason),
                ClientEvent::Ended { outcome } => self.ended = Some(outcome),
                ClientEvent::Started => self.ended = None,
                ClientEvent::Created { .. } | ClientEvent::Roster { .. } => {}
                ClientEvent::Said { .. } => {}
            }
        }
        self.scratch = events;
    }
}

/// The authority's loop: answer what the seat said, take a tick, sleep
/// until the next one is due. A tick that overran does not try to catch
/// up - the round loses that time, exactly as `app::StepClock` drops it.
fn run(mut room: Room, stop: Arc<AtomicBool>) {
    let step = Duration::from_secs_f32(PHYSICS_FIXED_DT);
    let mut due = Instant::now();
    while !stop.load(Ordering::Relaxed) {
        room.now = Instant::now();
        if !room.pump() {
            break;
        }
        room.tick();
        due += step;
        let now = Instant::now();
        match due.checked_duration_since(now) {
            Some(left) => thread::sleep(left),
            None => due = now,
        }
    }
    room.link.close();
}

/// The authoritative side of the link: one room, one seat.
struct Room {
    link: Loopback,
    options: RigOptions,
    /// `None` until the seat has greeted and the round has begun.
    game: Option<Game>,
    /// The round has been played out and announced; the world stays for
    /// a test to read, and the next `Start` begins a fresh one.
    ended: bool,
    opened: Instant,
    /// What the room calls now: the wall clock on the thread, a count of
    /// fixed steps under `Lockstep`. Everything here that asks the time
    /// reads this, so a stepped room needs no clock at all.
    now: Instant,
    nick: String,
    /// The newest intent and when it arrived; older than `INTENT_COAST`
    /// it reads as no input.
    intent: Option<(IntentMsg, Instant)>,
    /// The events of the ticks since the last snapshot, ahead of the
    /// next one's own.
    pending_events: Vec<crate::net::events::WireEvent>,
    scratch: Vec<Msg>,
    tick: Arc<AtomicU64>,
    latest: Arc<Mutex<Option<Snapshot>>>,
}

impl Room {
    /// A room on `link`, with no round until the seat greets it.
    fn new(link: Loopback, options: RigOptions) -> Room {
        let opened = Instant::now();
        Room {
            link,
            options,
            game: None,
            ended: false,
            opened,
            now: opened,
            nick: "rig".into(),
            intent: None,
            pending_events: Vec::new(),
            scratch: Vec::new(),
            tick: Arc::default(),
            latest: Arc::default(),
        }
    }

    /// Everything the seat has said. `false` once the link is gone.
    fn pump(&mut self) -> bool {
        let mut scratch = std::mem::take(&mut self.scratch);
        scratch.clear();
        self.link.drain(&mut scratch);
        let mut left = false;
        for msg in scratch.drain(..) {
            match msg {
                Msg::Lobby(Lobby::Create { nick, .. }) | Msg::Lobby(Lobby::Join { nick, .. }) => {
                    self.nick = nick;
                    self.say(Msg::Lobby(Lobby::RoomCreated { code: RIG_CODE.into() }));
                    self.begin();
                }
                Msg::Lobby(Lobby::Start) => self.begin(),
                Msg::Lobby(Lobby::Leave) => left = true,
                Msg::Intent(intent) => self.intent = Some((intent, self.now)),
                // Ready, chat, kick and everything a room says rather
                // than hears mean nothing to one seat playing alone.
                _ => {}
            }
        }
        self.scratch = scratch;
        !left && self.link.is_open()
    }

    /// Start the round and welcome the seat into it. A second call while
    /// one is running is the start button pressed twice; one after the
    /// round was played out is the rematch.
    fn begin(&mut self) {
        if self.game.is_some() && !self.ended {
            return;
        }
        self.ended = false;
        self.pending_events.clear();
        let mut game = Game::default();
        game.map = self.options.map.clone();
        game.seed_override = self.options.seed;
        game.enemy_count_override = self.options.enemies;
        game.level_overrides = self.options.overrides;
        game.player_row_override = self.options.tank_row;
        // A room server's round has no mission banner to freeze behind.
        game.show_intro = false;
        let (width, height) = game.map.field_size();
        game.init(width, height);
        self.game = Some(game);
        self.say(Msg::Lobby(Lobby::Roster {
            host: 0,
            seats: vec![RosterSeat {
                seat: 0,
                nick: self.nick.clone(),
                chassis: self.chassis(),
                ready: true,
                connected: true,
            }],
        }));
        self.say(Msg::Lobby(Lobby::Started));
        self.welcome();
    }

    /// The seat's chassis row, as the replica's `init` has to roll it.
    fn chassis(&self) -> u8 {
        self.game.as_ref().and_then(|g| g.player_chassis()).map_or(0, |kind| kind.row() as u8)
    }

    fn welcome(&mut self) {
        let roster = vec![Seat { seat: 0, nick: self.nick.clone(), chassis: self.chassis() }];
        let server_ms = self.server_ms();
        let Some(game) = &self.game else { return };
        match encode::welcome(game, 0, roster, "{}".into(), self.acked()) {
            Ok(mut welcome) => {
                welcome.snapshot.server_ms = server_ms;
                self.say(Msg::Welcome(welcome));
            }
            Err(e) => eprintln!("[rig] the map would not serialise: {e}"),
        }
    }

    /// One tick of the round, and the snapshot it earns. The end screen
    /// is a tick like any other - the room server's shape - except for
    /// the restart at the bottom of it: a `Game::update` that took the
    /// countdown past zero would start a second round nobody asked for,
    /// so the round is announced over instead.
    fn tick(&mut self) {
        if self.ended {
            return;
        }
        if self.game.as_ref().is_some_and(|g| g.outcome() != Outcome::Playing && g.restart_countdown() <= PHYSICS_FIXED_DT) {
            let outcome = self.game.as_ref().map_or(Outcome::Playing, Game::outcome);
            self.ended = true;
            self.say(Msg::Lobby(Lobby::Ended { outcome: outcome.into() }));
            return;
        }
        let intent = self.sampled_intent();
        let acked = self.acked();
        let server_ms = self.server_ms();
        let Some(game) = &mut self.game else { return };
        let (width, height) = game.map.field_size();
        game.update(Input::single(intent), PHYSICS_FIXED_DT, width, height);
        let frame = game.frame();
        self.tick.store(frame, Ordering::Relaxed);
        if frame % SNAPSHOT_EVERY != 0 {
            // A tick that sends nothing banks its events for the next
            // snapshot, whose own events are its frame's.
            self.pending_events.extend(encode::wire_events(game.events()));
            return;
        }
        let mut snapshot = encode::snapshot(game, acked);
        snapshot.server_ms = server_ms;
        if !self.pending_events.is_empty() {
            let mut events = std::mem::take(&mut self.pending_events);
            events.append(&mut snapshot.events);
            snapshot.events = events;
        }
        *self.latest.lock().expect("the window did not panic holding the lock") = Some(snapshot.clone());
        self.say(Msg::Snapshot(snapshot));
    }

    /// The seat's command this tick: the newest intent, repeated while it
    /// is younger than `INTENT_COAST`.
    fn sampled_intent(&self) -> Intent {
        match &self.intent {
            Some((msg, at)) if self.now.saturating_duration_since(*at) < INTENT_COAST => msg.intent(),
            _ => Intent::default(),
        }
    }

    /// The last intent tick the round has taken (stage 2's ack).
    fn acked(&self) -> [u32; MAX_SEATS] {
        let mut acked = [0; MAX_SEATS];
        acked[0] = self.intent.as_ref().map_or(0, |(msg, _)| msg.tick);
        acked
    }

    /// The room's clock, milliseconds since the rig started.
    fn server_ms(&self) -> u32 {
        self.now.saturating_duration_since(self.opened).as_millis().min(u32::MAX as u128) as u32
    }

    fn say(&mut self, msg: Msg) {
        self.link.send(&codec::encode(&msg));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::net::client::{Identity, RoomClient, RoomSetup};
    use crate::net::round::OnlineRound;
    use crate::tank::Dir;
    use crate::tuning::tuning;

    const DEFAULT_MAP: &str = include_str!("../../maps/default.toml");

    /// How long a frame of the window is here: 60 Hz, paced by the clock
    /// like the real one, since the rig's authority and the interpolator
    /// both run on real time.
    const FRAME: Duration = Duration::from_millis(16);

    /// One hull as both sides spell it: slot, and quarter pixels.
    type Hull = (usize, i32, i32);

    /// What one window frame drew, and what the room had said by then.
    struct Drawn {
        newest: Option<u32>,
        tick: u64,
        hulls: Vec<Hull>,
    }

    fn options(quality: LinkQuality) -> RigOptions {
        RigOptions {
            map: MapFile::from_toml_str(DEFAULT_MAP).expect("the default map parses"),
            seed: Some(0xB0B5),
            enemies: Some(4),
            tank_row: Some(3),
            quality,
            ..RigOptions::default()
        }
    }

    /// The hulls of an authoritative snapshot, in the replica's spelling.
    fn hulls(snapshot: &Snapshot) -> Vec<Hull> {
        snapshot.tanks.iter().map(|t| (t.id as usize, t.x as i32, t.y as i32)).collect()
    }

    /// One rig, one window, `frames` frames of it: what the window drew
    /// each frame, and every authoritative snapshot the run produced.
    fn play(quality: LinkQuality, frames: usize) -> (Rig, OnlineRound<Loopback>, Vec<Drawn>, Vec<(u32, Vec<Hull>)>) {
        let (rig, link) = start(options(quality));
        let client = RoomClient::host(link, Identity::new("rig", "tok-rig"), RoomSetup::default());
        let mut round = OnlineRound::new(client, "RIG");
        let mut seen: Vec<Drawn> = Vec::new();
        let mut authority: Vec<(u32, Vec<Hull>)> = Vec::new();
        let drive = Intent { move_dir: Some(Dir::Right), ..Intent::default() };
        for _ in 0..frames {
            round.frame(&drive, FRAME.as_secs_f32());
            // The truth, as often as the loop catches it: every snapshot
            // the authority cut, for the drawn frames to be checked
            // against.
            if let Some(snapshot) = rig.authority()
                && authority.last().is_none_or(|(tick, _)| *tick != snapshot.tick)
            {
                authority.push((snapshot.tick, hulls(&snapshot)));
            }
            if let Some(game) = round.game() {
                let state = game.drawable_state();
                seen.push(Drawn {
                    newest: round.interp().newest_tick(),
                    tick: game.frame(),
                    hulls: state.tanks.iter().map(|t| (t.slot, t.x, t.y)).collect(),
                });
            }
            thread::sleep(FRAME);
        }
        (rig, round, seen, authority)
    }

    /// The whole lane end to end: an authoritative round on a thread, a
    /// link with delay and jitter, and a replica that draws a round it
    /// never simulates.
    #[test]
    fn the_rig_draws_a_round_it_does_not_simulate() {
        let (rig, round, seen, authority) = play(LinkQuality::new(40, 10, 0.0), 150);
        let replica = round.game().expect("the rig welcomed the window into its round");
        assert!(replica.player().is_some(), "the seat's tank is there for the HUD to read");
        assert!(rig.tick() > 60, "the authority ran its own round: tick {}", rig.tick());
        assert!(seen.len() > 100, "the window drew {} frames", seen.len());

        // The replica only ever stands on a tick the authority cut a
        // snapshot at: nothing here runs `Game::update`.
        for frame in &seen {
            assert_eq!(frame.tick % SNAPSHOT_EVERY, 0, "the replica invented tick {}", frame.tick);
        }
        let ticks: Vec<u64> = seen.iter().map(|f| f.tick).collect();
        assert!(ticks.windows(2).all(|w| w[1] >= w[0]), "the replica's clock went back: {ticks:?}");
        assert!(ticks.last() > ticks.first(), "the replica followed the round along");

        // It draws about the interpolation delay behind the room and
        // never ahead of it.
        let newest = rig.authority().expect("the authority sent a snapshot").tick as u64;
        let last = *ticks.last().expect("a frame");
        assert!(last <= newest, "the replica ran ahead of the room");
        let behind = newest - last;
        let delay_ticks = (tuning().online_interpolation_delay_ms / 1000.0 / PHYSICS_FIXED_DT) as u64;
        assert!(
            behind <= delay_ticks * 4 + 4 * SNAPSHOT_EVERY,
            "the replica fell {behind} ticks behind a {delay_ticks}-tick delay"
        );

        // Every hull is drawn where the room put it, or between where it
        // put it on two snapshots in a row: the picture is the room's
        // own, interpolated, never a guess of its own. A hull that only
        // appears in one end of the bracket (a wave tank arriving, a
        // wreck despawning) has nothing to be between.
        //
        // **The local seat is exempt, and that is stage 2 working.** With
        // `online_predict_own_tank` on, the one hull this client steers
        // is drawn at the present rather than interpolated in the past
        // (docs/online-coop-prd.md §4.12); it is the seat whose position
        // is deliberately *not* the room's most recent word. Every other
        // hull still is, which is what this check is for.
        let predicted = tuning().online_predict_own_tank.then_some(0usize);
        let mut checked = 0;
        for frame in &seen {
            let Some(i) = authority.iter().position(|(tick, _)| *tick as u64 == frame.tick) else { continue };
            let Some((_, from)) = authority.get(i) else { continue };
            let Some((_, to)) = authority.get(i + 1) else { continue };
            for &(slot, x, y) in &frame.hulls {
                if Some(slot) == predicted {
                    continue;
                }
                let Some(a) = from.iter().find(|h| h.0 == slot) else { continue };
                let Some(b) = to.iter().find(|h| h.0 == slot) else { continue };
                for (drawn, one, other) in [(x, a.1, b.1), (y, a.2, b.2)] {
                    let (low, high) = (one.min(other), one.max(other));
                    assert!(
                        (low - 1..=high + 1).contains(&drawn),
                        "slot {slot} drawn at {drawn} on tick {}, the room had it between {low} and {high}",
                        frame.tick
                    );
                    checked += 1;
                }
            }
        }
        assert!(checked > 100, "only {checked} positions could be checked against the room");

        // The point of the whole thing: the picture moves on frames no
        // snapshot arrived on. Without interpolation this count is zero
        // and the round is drawn at 20 fps.
        let smoothed = seen
            .windows(2)
            .filter(|pair| pair[0].newest == pair[1].newest && pair[0].hulls != pair[1].hulls)
            .count();
        assert!(smoothed > 20, "only {smoothed} of {} frames interpolated", seen.len());
    }

    /// The seat's hull on the replica, in quarter pixels.
    fn seat_hull(rig: &Lockstep) -> (i32, i32) {
        let state = rig.replica().expect("the room welcomed the seat").drawable_state();
        let tank = state.tanks.iter().find(|t| t.slot == 0).expect("the seat's tank");
        (tank.x, tank.y)
    }

    /// Lockstep for a networked round, the way `devserver`'s `step` is
    /// lockstep for a local one: the same options and the same steps give
    /// the same round twice - the bytes on the wire included, since the
    /// room's clock is a count of steps - and the replica ends standing
    /// exactly on the last snapshot the authority cut.
    #[test]
    fn a_stepped_rig_replays_and_the_replica_lands_on_the_authoritys_tick() {
        let run = || {
            let mut rig = Lockstep::start(options(LinkQuality::PERFECT));
            rig.drive(Intent { move_dir: Some(Dir::Right), ..Intent::default() });
            rig.step(90);
            rig.drive(Intent { move_dir: Some(Dir::Up), fire: true, ..Intent::default() });
            rig.step(90);
            rig
        };
        let a = run();
        let b = run();
        assert_eq!(a.seat(), Some(0));
        assert_eq!(a.code(), Some(RIG_CODE));
        assert_eq!(a.note(), None, "the room refused something");
        assert_eq!(a.tick(), 180, "the authority took every step");
        let truth = a.authority().expect("the room is running a round");
        assert_eq!(truth.drawable_state(), b.authority().expect("a round").drawable_state(), "two runs differ");
        assert_eq!(
            codec::encode(&Msg::Snapshot(a.snapshot().expect("a snapshot"))),
            codec::encode(&Msg::Snapshot(b.snapshot().expect("a snapshot"))),
            "the wire does not replay byte for byte"
        );

        // The replica stands on a snapshot tick, never between two, and
        // draws the round the authority is running.
        let replica = a.replica().expect("the welcome built a replica");
        assert_eq!(replica.frame() % SNAPSHOT_EVERY, 0, "the replica invented tick {}", replica.frame());
        assert_eq!(replica.frame(), a.tick(), "the replica did not catch up");
        assert_eq!(replica.drawable_state(), truth.drawable_state(), "the replica draws another picture");
        assert_eq!(b.replica().expect("a replica").drawable_state(), replica.drawable_state());
    }

    /// The fixture the end of a round is tested on: a Hunt won in well
    /// under a second by a seat that pulls its trigger, whatever seed
    /// the round runs on (its own header says how). The room server's
    /// `tests/round.rs` ends a round with the same file.
    const HUNT_MAP: &str = include_str!("../../maps/test/online/hunt-duel.toml");

    fn hunt_options() -> RigOptions {
        RigOptions {
            map: MapFile::from_toml_str(HUNT_MAP).expect("the hunt map parses"),
            seed: None,
            quality: LinkQuality::PERFECT,
            ..RigOptions::default()
        }
    }

    /// A room ticks its round through the end screen and stops before the
    /// restart a local round would take: the countdown runs down in the
    /// snapshots, the world stays on the round that ended, and the next
    /// `Start` is a rematch on a fresh seed.
    #[test]
    fn a_room_plays_the_end_screen_out_and_stops_short_of_the_restart() {
        let mut rig = Lockstep::start(hunt_options());
        // A player's shell is fired on the press, so the trigger is
        // pulled rather than held; the weapon's own cooldown paces it.
        let mut won_at = None;
        for tick in 0..600 {
            rig.drive(Intent { face: Some(Dir::Up), fire: tick % 6 == 0, ..Intent::default() });
            rig.step(1);
            let game = rig.authority().expect("the room is running a round");
            if won_at.is_none() && game.outcome() != crate::simulation::Outcome::Playing {
                won_at = Some(game.frame());
            }
            if rig.ended().is_some() {
                break;
            }
        }
        let won_at = won_at.expect("the round never ended");
        assert_eq!(rig.ended(), Some(RoundOutcome::Won), "the hunt is won by killing the enemy frog");

        // The end screen was played, not skipped: the room went on
        // ticking for the whole countdown and only then said so.
        let truth = rig.authority().expect("the world outlives the round");
        let screen = truth.frame() - won_at;
        let expected = (tuning().restart_delay / PHYSICS_FIXED_DT) as u64;
        assert!(
            screen.abs_diff(expected) <= 2,
            "the end screen ran {screen} ticks of an expected {expected}"
        );
        assert!(truth.restart_countdown() <= PHYSICS_FIXED_DT, "the countdown was not run down");
        assert_ne!(truth.outcome(), crate::simulation::Outcome::Playing, "the room started a round of its own");
        let seed = truth.round_seed();

        // Nothing moves any more, however long the room is left alone.
        let standing = truth.frame();
        rig.step(120);
        assert_eq!(rig.authority().expect("a world").frame(), standing, "the room went on ticking after the round");

        // The rematch: a fresh round on a new seed, and the replica is
        // welcomed into it.
        rig.rematch();
        assert_eq!(rig.ended(), None, "the end screen is cleared by the round that follows");
        let again = rig.authority().expect("the rematch built a round");
        assert_eq!(again.frame(), 0, "the rematch starts at the top");
        assert_ne!(again.round_seed(), seed, "the rematch replays the round it just finished");
        assert_eq!(rig.replica().expect("a replica").frame(), 0);
        rig.step(30);
        assert_eq!(rig.tick(), 30, "the rematch ticks like any round");
    }

    /// The seat's command travels: a stepped rig that is driven ends
    /// somewhere one that is left alone does not.
    #[test]
    fn a_stepped_seats_intent_reaches_the_authority() {
        let mut driven = Lockstep::start(options(LinkQuality::PERFECT));
        let start = seat_hull(&driven);
        driven.drive(Intent { move_dir: Some(Dir::Right), ..Intent::default() });
        assert_eq!(driven.step(60), 60);
        assert_ne!(seat_hull(&driven), start, "the seat's intent never reached the room");

        let mut idle = Lockstep::start(options(LinkQuality::PERFECT));
        idle.step(60);
        assert_eq!(seat_hull(&idle), start, "a seat that asked for nothing moved");
    }

    /// **Stage 2's whole point** (docs/online-coop-prd.md §4.12): the
    /// hull this client steers is drawn where it will be, not where the
    /// room last said it was.
    ///
    /// `play` drives right the whole run, so the predicted hull is always
    /// further right than the interpolated picture the room's snapshots
    /// bracket. The gap is the lead plus the interpolation delay, which
    /// is exactly the latency stage 2 removes.
    #[test]
    fn the_steered_hull_is_drawn_ahead_of_the_room() {
        if !tuning().online_predict_own_tank {
            return; // the build ships with it on; nothing to compare.
        }
        let (_, round, seen, authority) = play(LinkQuality::PERFECT, 120);
        let drawn = seen.last().expect("a drawn frame").hulls.iter().find(|(slot, _, _)| *slot == 0).expect("the seat").1;
        let room = authority
            .last()
            .expect("a snapshot")
            .1
            .iter()
            .find(|(slot, _, _)| *slot == 0)
            .expect("the seat")
            .1;
        assert!(round.game().is_some(), "the welcome arrived");
        assert!(
            drawn > room,
            "the steered hull was drawn at {drawn}, not ahead of the room's {room} - prediction did nothing"
        );
    }

    /// **The hull has to turn with the prediction, not behind it.**
    ///
    /// `tick_presentation` eases `visual_rotation` toward `rotation`, so
    /// it has to run on the pose that will be drawn. With the write
    /// after it, every frame went: snapshot sets the facing to the
    /// server's (a hundred milliseconds old), the easing chases *that*,
    /// then the prediction overwrites it - and the next frame's snapshot
    /// overwrites the prediction again. The drawn hull never once saw the
    /// predicted facing, so it slid across the field still pointing the
    /// old way.
    ///
    /// Which is why this turns late and looks straight away: driving one
    /// way the whole run, the server and the prediction agree and the bug
    /// is invisible.
    #[test]
    fn the_predicted_hull_turns_with_the_prediction() {
        if !tuning().online_predict_own_tank {
            return;
        }
        let (_rig, link) = start(options(LinkQuality::PERFECT));
        let client = RoomClient::host(link, Identity::new("rig", "tok-rig"), RoomSetup::default());
        let mut round = OnlineRound::new(client, "RIG");

        // Long enough that the room is welcoming, ticking and agreeing
        // that this hull faces right.
        for _ in 0..90 {
            round.frame(&Intent { move_dir: Some(Dir::Right), ..Intent::default() }, FRAME.as_secs_f32());
            thread::sleep(FRAME);
        }
        // Then turn, and look before the room's word can catch up.
        for _ in 0..4 {
            round.frame(&Intent { move_dir: Some(Dir::Down), ..Intent::default() }, FRAME.as_secs_f32());
            thread::sleep(FRAME);
        }

        let replica = round.game().expect("a replica");
        let mut seen = None;
        for t in replica.world.query::<&crate::tank::Tank>().iter() {
            if t.owner_slot() == 0 {
                seen = Some((t.visual_rotation, t.rotation));
            }
        }
        let (visual, facing) = seen.expect("the seat's tank");
        assert_eq!(facing, Dir::Down.rotation(), "the prediction turned");
        assert_ne!(
            visual,
            Dir::Right.rotation(),
            "the drawn hull is still pointing the old way at {visual} while the prediction faces {facing}: \
             the presentation pass never saw the prediction"
        );
    }

    /// The dial's whole point: a lossy link costs the picture nothing it
    /// cannot ride out.
    #[test]
    fn a_lossy_link_keeps_the_round_running() {
        let (rig, round, seen, _) = play(LinkQuality::new(60, 20, 0.05), 100);
        assert!(round.game().is_some(), "the welcome arrived over a lossy link");
        assert!(rig.tick() > 40, "the authority kept ticking");
        let ticks: Vec<u64> = seen.iter().map(|f| f.tick).collect();
        assert!(ticks.windows(2).all(|w| w[1] >= w[0]), "a lost snapshot rewound the picture: {ticks:?}");
        assert!(ticks.last() > ticks.first(), "the round went nowhere: {ticks:?}");
    }
}
