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
use crate::net::codec::{self, Msg};
use crate::net::encode;
use crate::net::loopback::{self, LinkQuality, Loopback};
use crate::net::transport::Transport;
use crate::net::wire::{IntentMsg, Lobby, RosterSeat, Seat, Snapshot};
use crate::net::MAX_SEATS;
use crate::simulation::{Game, Input};
use crate::PHYSICS_FIXED_DT;

/// The code the rig's room answers with. It reads as a room code (a pod
/// letter and four more) so the status line and a screenshot look like
/// the real thing.
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
    let tick = Arc::new(AtomicU64::new(0));
    let latest: Arc<Mutex<Option<Snapshot>>> = Arc::default();
    let room = Room {
        link: room_end,
        options,
        game: None,
        opened: Instant::now(),
        nick: "rig".into(),
        intent: None,
        pending_events: Vec::new(),
        scratch: Vec::new(),
        tick: Arc::clone(&tick),
        latest: Arc::clone(&latest),
    };
    let flag = Arc::clone(&stop);
    let thread = thread::Builder::new()
        .name("bongbong-rig".into())
        .spawn(move || run(room, flag))
        .expect("the rig thread starts");
    (Rig { stop, thread: Some(thread), tick, latest }, client_end)
}

/// The authority's loop: answer what the seat said, take a tick, sleep
/// until the next one is due. A tick that overran does not try to catch
/// up - the round loses that time, exactly as `app::StepClock` drops it.
fn run(mut room: Room, stop: Arc<AtomicBool>) {
    let step = Duration::from_secs_f32(PHYSICS_FIXED_DT);
    let mut due = Instant::now();
    while !stop.load(Ordering::Relaxed) {
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
    opened: Instant,
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
                Msg::Intent(intent) => self.intent = Some((intent, Instant::now())),
                // Ready, chat, kick and everything a room says rather
                // than hears mean nothing to one seat playing alone.
                _ => {}
            }
        }
        self.scratch = scratch;
        !left && self.link.is_open()
    }

    /// Start the round and welcome the seat into it. A second call while
    /// one is running is the start button pressed twice.
    fn begin(&mut self) {
        if self.game.is_some() {
            return;
        }
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

    /// One tick of the round, and the snapshot it earns.
    fn tick(&mut self) {
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
            Some((msg, at)) if at.elapsed() < INTENT_COAST => msg.intent(),
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
        self.opened.elapsed().as_millis().min(u32::MAX as u128) as u32
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
        assert!(replica.player.is_some(), "the seat's tank is there for the HUD to read");
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
        let mut checked = 0;
        for frame in &seen {
            let Some(i) = authority.iter().position(|(tick, _)| *tick as u64 == frame.tick) else { continue };
            let Some((_, from)) = authority.get(i) else { continue };
            let Some((_, to)) = authority.get(i + 1) else { continue };
            for &(slot, x, y) in &frame.hulls {
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
