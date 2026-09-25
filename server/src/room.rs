//! A room (docs/online-coop-prd.md §4.1, §4.7, §4.9): one task that owns
//! the authoritative `Game`, its seats and the lobby. Its life is
//! **waiting** (not ticking) → **playing** (a 60 Hz interval, each tick
//! sampling every seat's mailbox into one `Input` and calling
//! `Game::update`) → **paused** when nobody is connected (not ticking, the
//! world kept in memory, resumed where it stopped) → **ended** (results
//! kept a while for a rematch). The end screen belongs to **playing**:
//! the room goes on ticking and sending while the shots land and the
//! restart counts down, and ends the round on the tick before the one
//! `Game::update` would call `init` on - a local round starts over
//! there, a room would be dragging its seats into a round nobody asked
//! for. The snapshots stop with it, so how the round went travels as a
//! `Lobby::Ended` to every seat and the roster behind it is the one the
//! room screen comes back on; `Start` from the host is then the rematch.
//!
//! Every third tick a snapshot is encoded once, as a delta against the
//! previous one, and offered to every seat's outbox; a seat that skipped
//! one, or just arrived, gets the next in full. The bytes come from
//! `net::encode` (`snapshot`, `welcome`); the room only adds its clock
//! and the events of the two ticks between snapshots. Seats outlive connections: a disconnect keeps the seat for
//! its device token, and a reconnect reclaims it with a fresh `Welcome`
//! cut from the live world, holes in the map included.
//!
//! The durations below are server policy, not gameplay tuning. The one
//! thing here that does touch the tuning table is `tuning_patch`: the
//! wave plan a room sizes to its team (docs/online-coop-prd.md §4.11),
//! put on the table for the one call to `Game::init` that reads it and
//! sent to every seat in the `Welcome` so a replica resolves the plan
//! its room is fought under.

use std::collections::BTreeSet;
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::sync::atomic::Ordering;
use std::time::Duration;

use axum::body::Bytes;
use bongbong::PHYSICS_FIXED_DT;
use bongbong::level::{LevelOverrides, Mission};
use bongbong::map::MapFile;
use bongbong::net::codec::{self, Msg};
use bongbong::net::delta::delta;
use bongbong::net::encode;
use bongbong::net::events::WireEvent;
use bongbong::net::wire::{Lobby, RosterSeat, Seat as WireSeat, Snapshot, Welcome};
use bongbong::net::{MAX_SEATS, PROTOCOL_VERSION};
use bongbong::simulation::{Game, Input, Outcome, PlayerCount};
use bongbong::tuning::{self, Tuning, tuning};
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, Interval, MissedTickBehavior};
use tracing::{info, warn};

use crate::conn::{Delivery, Outbox};
use crate::hub::{Hub, RoomStats};
use crate::mailbox::Mailbox;

/// The tick: one `PHYSICS_FIXED_DT`, 60 Hz.
pub const TICK: Duration = Duration::from_nanos(1_000_000_000 / 60);

/// A snapshot goes out every this many ticks: **one, so 60 Hz - a
/// snapshot per tick.**
///
/// **This is the cheapest latency in the system.** A client draws
/// `online_interpolation_delay_ms` behind so that two snapshots always
/// bracket render time, so that delay can be no shorter than one
/// snapshot interval without the replica extrapolating most frames: the
/// cadence sets a floor under every un-predicted thing a player sees.
/// 20 Hz put that floor at 50 ms and made 100 ms the comfortable delay;
/// 30 Hz made it 66; at 60 Hz the interval is 16.7 ms and 33 ms is two
/// whole intervals of margin. Measured press-to-shell over a perfect
/// link fell from about 100 ms to about 60.
///
/// It is paid for in bandwidth and nothing else, and the bill is small:
/// a live room measured 2.1 KB/s at 30 Hz, so about 4 KB/s here, against
/// a tick whose p99 is 383 µs of its 16,600 - 2.3 %, with no overrun in
/// thousands of ticks. Deltas are encoded once and handed to every seat,
/// so the cost is one more encode per tick, not per seat, and a delta
/// over one tick's change is smaller than one over two.
///
/// A slow client never queues these up: `conn`'s bounded outbox drops
/// snapshots for a seat that cannot keep up and marks it for a full one.
pub const SNAPSHOT_EVERY: u64 = 1;

/// Seats a room takes: the simulation's own `MAX_SEATS`
/// (docs/online-coop-prd.md §4.11), since the bar now reads a whole team
/// (the compact strip in `hud.rs`). A join past it is refused by name.
/// The wave plan is still authored for one tank, so a big team has an
/// easy round until the curve scales with the seat count.
pub const SEATS_PLAYABLE: usize = MAX_SEATS;

/// A waiting room with nobody connected is reaped after this.
pub const WAITING_TTL: Duration = Duration::from_secs(30 * 60);

/// A paused round (nobody connected) is dropped after this.
pub const EMPTY_TTL: Duration = Duration::from_secs(5 * 60);

/// An ended room keeps its results and roster this long for a rematch.
pub const ENDED_TTL: Duration = Duration::from_secs(5 * 60);

/// An ended room on a draining server keeps its results this long -
/// long enough to read how the round went, since no rematch can follow.
pub const DRAIN_ENDED_TTL: Duration = Duration::from_secs(15);

/// A round ends after this whatever the field says.
pub const ROUND_MAX: Duration = Duration::from_secs(30 * 60);

/// After a disconnect the seat is "reconnecting" this long; past it the
/// seat is away. In a waiting room an away seat is freed; in a round it
/// stays owned by its token to the end.
pub const SEAT_GRACE: Duration = Duration::from_secs(30);

/// The longest nickname kept; the rest is cut.
pub const NICK_MAX: usize = 24;

/// The simulation build stamped in a waiting room's `Welcome` (a round's
/// comes from `encode::welcome`); the workspace shares one version.
pub const SIM_VERSION: &str = env!("CARGO_PKG_VERSION");

/// The tuning diff a room that has not started carries in its `Welcome`:
/// there is no round yet to size, so it is the empty patch. A round's is
/// `tuning_patch` for the seats it starts with.
pub const ROOM_TUNING_JSON: &str = "{}";

/// The tuning rows a round is played under, as the JSON patch the
/// `Welcome` carries: the map's wave plan sized to the team that fights
/// it (docs/online-coop-prd.md §4.11, docs/maps-to-levels.md
/// "Difficulty by seat count").
///
/// Every seat past the first widens each wave by
/// `online_wave_size_per_seat` of what the map authored, and every
/// `online_wave_tier_seats_per_step` seats lift the tier ramp a rung -
/// both dials of the room's, not the round's, so a couch round and the
/// probe never read them. **One seat is the empty patch**: a room of one
/// plays exactly the round a single player plays offline, down to the
/// bytes.
pub fn tuning_patch(seats: usize) -> String {
    // The two dials off the live table, which between rounds is the
    // table the server started with: the rows a round's patch writes are
    // never these.
    let (per_seat, per_step) = {
        let t = tuning();
        (t.online_wave_size_per_seat, t.online_wave_tier_seats_per_step)
    };
    let extra = seats.saturating_sub(1);
    let scale = 1.0 + extra as f32 * per_seat;
    let step = extra / per_step.max(1);
    if scale == 1.0 && step == 0 {
        return ROOM_TUNING_JSON.to_string();
    }
    format!("{{\"wave_size_scale\":{scale},\"wave_tier_step\":{step}}}")
}

/// One room at a time through `Game::init`.
///
/// The tuning table is the process's and the process holds every room,
/// so a room puts its own patch on it for exactly the one call that
/// reads it.
/// The rows the patch touches (`wave_size_scale`, `wave_tier_step`) are
/// `Restart` rows, read where the spawn plan is resolved and nowhere
/// else, so a room already playing reads nothing that moves under it -
/// the lock has only to cover the patch and the `init` beside it.
static ROUND_TUNING: Mutex<()> = Mutex::new(());

/// The table the server started with, read once under `ROUND_TUNING` before
/// the first round patches it: a room's own rows go on top of this, not
/// on top of the room that started before it.
static BASE_TUNING: OnceLock<Tuning> = OnceLock::new();

/// Run `game.init` with `patch` on the tuning table, on top of the
/// server's own table rather than whatever the last room left there.
fn init_under(patch: &str, game: &mut Game, width: f32, height: f32) {
    let _held = ROUND_TUNING.lock().unwrap_or_else(PoisonError::into_inner);
    tuning::replace_now(*BASE_TUNING.get_or_init(tuning::current));
    if let Err(e) = tuning::submit_json(patch) {
        warn!(error = e, patch, "the room's tuning patch was refused; the round runs the defaults");
    }
    tuning::apply_pending();
    game.init(width, height);
}

/// Where a room stands. The discriminants are what `RoomStats` stores.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Phase {
    Waiting = 0,
    Playing = 1,
    Paused = 2,
    Ended = 3,
}

impl Phase {
    pub fn from_u8(v: u8) -> Phase {
        match v {
            1 => Phase::Playing,
            2 => Phase::Paused,
            3 => Phase::Ended,
            _ => Phase::Waiting,
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            Phase::Waiting => "waiting",
            Phase::Playing => "playing",
            Phase::Paused => "paused",
            Phase::Ended => "ended",
        }
    }
}

/// What a deadline that came due asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Expiry {
    /// Drop the room.
    Reap,
    /// The round hit `ROUND_MAX`.
    EndRound,
}

/// The room's clock rules, apart from the world so they test with paused
/// time: which phase, since when, how many seats are connected, and when
/// the next deadline falls.
///
/// On a draining server only a round being played is worth waiting for:
/// a waiting or paused room is due the moment the drain starts, and an
/// ended one after `DRAIN_ENDED_TTL`, so the process exits as soon as
/// the last round in play is over.
#[derive(Clone, Copy, Debug)]
pub struct Lifecycle {
    phase: Phase,
    /// When the phase was entered (for `Waiting`, when the last seat
    /// disconnected, or creation).
    since: Instant,
    round_started: Option<Instant>,
    connected: usize,
    /// When the server began draining, if it has.
    drain_at: Option<Instant>,
}

impl Lifecycle {
    pub fn new(now: Instant) -> Lifecycle {
        Lifecycle { phase: Phase::Waiting, since: now, round_started: None, connected: 0, drain_at: None }
    }

    /// The server is draining; the first call counts.
    pub fn drain(&mut self, now: Instant) {
        self.drain_at.get_or_insert(now);
    }

    pub fn draining(&self) -> bool {
        self.drain_at.is_some()
    }

    pub fn phase(&self) -> Phase {
        self.phase
    }

    pub fn connected(&self) -> usize {
        self.connected
    }

    /// The world advances only while playing.
    pub fn ticking(&self) -> bool {
        self.phase == Phase::Playing
    }

    /// A seat came onto a socket; a paused round resumes.
    pub fn connect(&mut self, now: Instant) {
        self.connected += 1;
        if self.phase == Phase::Paused {
            self.phase = Phase::Playing;
            self.since = now;
        }
    }

    /// A seat left its socket; the last one pauses a round and starts
    /// the waiting room's clock.
    pub fn disconnect(&mut self, now: Instant) {
        self.connected = self.connected.saturating_sub(1);
        if self.connected == 0 {
            match self.phase {
                Phase::Playing => {
                    self.phase = Phase::Paused;
                    self.since = now;
                }
                Phase::Waiting => self.since = now,
                Phase::Paused | Phase::Ended => {}
            }
        }
    }

    /// The round starts (or restarts: a rematch from `Ended`).
    pub fn start(&mut self, now: Instant) {
        self.phase = if self.connected == 0 { Phase::Paused } else { Phase::Playing };
        self.since = now;
        self.round_started = Some(now);
    }

    /// The round is over.
    pub fn end(&mut self, now: Instant) {
        self.phase = Phase::Ended;
        self.since = now;
        self.round_started = None;
    }

    /// When the next `Expiry` falls due, if one does.
    pub fn deadline(&self) -> Option<Instant> {
        let round_max = self.round_started.map(|t| t + ROUND_MAX);
        if let Some(drain_at) = self.drain_at {
            return match self.phase {
                Phase::Playing => round_max,
                Phase::Waiting | Phase::Paused => Some(drain_at),
                Phase::Ended => Some(self.since + DRAIN_ENDED_TTL),
            };
        }
        match self.phase {
            Phase::Waiting => (self.connected == 0).then(|| self.since + WAITING_TTL),
            Phase::Playing => round_max,
            Phase::Paused => {
                let empty = self.since + EMPTY_TTL;
                Some(round_max.map_or(empty, |r| r.min(empty)))
            }
            Phase::Ended => Some(self.since + ENDED_TTL),
        }
    }

    /// What `now` asks for, if the deadline has passed.
    pub fn expired(&self, now: Instant) -> Option<Expiry> {
        let due = self.deadline()?;
        if now < due {
            return None;
        }
        Some(match self.phase {
            Phase::Waiting | Phase::Ended => Expiry::Reap,
            Phase::Playing => Expiry::EndRound,
            Phase::Paused => {
                if self.round_started.is_some_and(|t| t + ROUND_MAX <= now) {
                    Expiry::EndRound
                } else {
                    Expiry::Reap
                }
            }
        })
    }
}

/// A connection as the room sees it: its id (to tell a stale disconnect
/// from the socket that replaced it) and the way to its writer.
pub struct ConnLink {
    pub id: u64,
    pub outbox: Outbox,
}

/// What a room is created with.
pub struct RoomParams {
    pub map: MapFile,
    pub mission: Mission,
    /// A pinned seed; absent, the room draws one at start (and another
    /// for a rematch).
    pub seed: Option<u64>,
}

#[cfg(feature = "dev-tools")]
impl RoomParams {
    /// The setup a dev tool's `room_open` asks for, resolved the way a
    /// hosting client's `Create` resolves it: a shipped map by name or a
    /// whole `map_toml`, the mission by name, and an optional pinned
    /// seed.
    pub fn for_dev(
        map: &str,
        map_toml: Option<&str>,
        mission: Option<&str>,
        seed: Option<u64>,
    ) -> Result<RoomParams, String> {
        let map = match map_toml {
            Some(toml) => bongbong::map::MapFile::from_toml_str(toml),
            None => bongbong::map::open_map(map),
        }
        .map_err(|e| format!("bad map: {e}"))?;
        let mission = match mission {
            None => Mission::Protect,
            Some("protect") => Mission::Protect,
            Some("hunt") => Mission::Hunt,
            Some("destroy") => Mission::Destroy,
            Some(other) => return Err(format!("mission: {other:?} is not protect|hunt|destroy")),
        };
        Ok(RoomParams { map, mission, seed })
    }
}

/// The answer to a `Command::Join`: the seat and the mailbox its intents
/// go into.
pub struct Joined {
    pub seat: u8,
    pub mailbox: Arc<Mailbox>,
}

/// What a connection task asks a room.
pub enum Command {
    Join { nick: String, device_token: String, conn: ConnLink, reply: oneshot::Sender<Result<Joined, String>> },
    Lobby { conn_id: u64, msg: Lobby },
    Disconnected { conn_id: u64 },
    /// A dev tool's call (`devserver`, feature `dev-tools`), answered
    /// here rather than from the socket task: the room owns the `Game`,
    /// and a command is drained between ticks, so nothing reads a world
    /// mid-update. The game's own dev server keeps the same rule.
    #[cfg(feature = "dev-tools")]
    Dev(crate::devserver::DevRequest),
}

/// A dev tool's standing input for one seat (`seat_intent`): the same
/// intent every tick until `remaining` runs out, with the trigger tapped
/// every `fire_every` ticks rather than held - a shell fires once per
/// press and a held one never re-arms.
#[cfg(feature = "dev-tools")]
#[derive(Clone, Copy)]
pub struct DevScript {
    pub intent: bongbong::ai::Intent,
    pub remaining: u64,
    pub fire_every: Option<u64>,
    /// The client tick the next post carries, so the seat's counter runs
    /// on unbroken whether a bot or a real client is stamping it.
    pub tick: u32,
    /// How many have gone out, for the tap phase.
    pub sent: u64,
}

/// One seat, owned by its device token for the room's life.
struct Seat {
    nick: String,
    device_token: String,
    ready: bool,
    /// A seat with no client at all, taken by `room_open` and driven by
    /// `seat_intent`. It counts as connected - the tick samples its
    /// mailbox and the lifecycle does not pause the room for it - and
    /// nothing is ever sent to it, since there is nowhere to send.
    #[cfg(feature = "dev-tools")]
    bot: bool,
    /// What a dev tool's `seat_intent` is driving this seat with: one
    /// intent posted per tick until it runs out.
    ///
    /// It has to be *fed*, not posted all at once: the mailbox is a
    /// jitter buffer capped at `BUFFER_MAX`, so dropping a hundred
    /// intents in keeps the last eight and throws the rest away. One a
    /// tick is also what a real client does, which is the point - the
    /// input goes down the path a player's does, ordering and `acked`
    /// included.
    #[cfg(feature = "dev-tools")]
    script: Option<DevScript>,
    conn: Option<ConnLink>,
    /// When the socket dropped; `None` while connected.
    away_since: Option<Instant>,
    mailbox: Arc<Mailbox>,
    /// The next snapshot goes in full: the seat just (re)joined or
    /// skipped a delta, so its baseline is not the room's.
    needs_full: bool,
}

impl Seat {
    #[cfg(not(feature = "dev-tools"))]
    fn connected(&self) -> bool {
        self.conn.is_some()
    }

    /// A bot seat is "connected" to everything that samples or counts
    /// seats: it has a mailbox being driven and the room must keep
    /// ticking for it. Only the send paths look at `conn` itself.
    #[cfg(feature = "dev-tools")]
    fn connected(&self) -> bool {
        self.conn.is_some() || self.bot
    }
}

struct Room {
    hub: Arc<Hub>,
    code: String,
    stats: Arc<RoomStats>,
    map: MapFile,
    overrides: LevelOverrides,
    pinned_seed: Option<u64>,
    /// The seed the current round runs on (0 before the first start).
    round_seed: u64,
    /// Seat number is the index; `None` is a seat that left mid-round
    /// (its tank stays, idle). A waiting room compacts instead.
    seats: Vec<Option<Seat>>,
    /// Device tokens the host kicked; they do not come back.
    kicked: BTreeSet<String>,
    game: Option<Box<Game>>,
    life: Lifecycle,
    interval: Interval,
    /// The last snapshot sent: the baseline of the next delta.
    prev: Snapshot,
    /// The events of the ticks since the last snapshot that sent none
    /// (`encode::wire_events`), prepended to the next one's own.
    pending_events: Vec<WireEvent>,
    /// The tuning patch the current round is played under and every
    /// `Welcome` carries (`tuning_patch`); the empty patch until a round
    /// starts.
    tuning_json: String,
    created: Instant,
    commands: mpsc::Receiver<Command>,
    /// `room_step` has taken the room off real time; it advances only
    /// when a dev tool says so, until `room_resume`.
    #[cfg(feature = "dev-tools")]
    frozen: bool,
    /// The events of the round, for `room_events`: a ring of the last
    /// `DEV_EVENT_RING`, each with the seq and frame it happened on.
    #[cfg(feature = "dev-tools")]
    dev_events: std::collections::VecDeque<serde_json::Value>,
    #[cfg(feature = "dev-tools")]
    dev_seq: u64,
}

/// The room task: runs until the room is reaped.
pub async fn run(hub: Arc<Hub>, code: String, params: RoomParams, commands: mpsc::Receiver<Command>, stats: Arc<RoomStats>) {
    let now = Instant::now();
    let mut interval = tokio::time::interval(TICK);
    interval.set_missed_tick_behavior(MissedTickBehavior::Delay);
    // Subscribed before the first look, so a drain that begins between
    // the two is still seen.
    let mut drain = hub.drain_signal();
    let mut room = Room {
        hub,
        code,
        stats,
        map: params.map,
        overrides: LevelOverrides { mission: Some(params.mission), ..LevelOverrides::default() },
        pinned_seed: params.seed,
        round_seed: 0,
        seats: Vec::new(),
        kicked: BTreeSet::new(),
        game: None,
        life: Lifecycle::new(now),
        interval,
        prev: Snapshot::default(),
        pending_events: Vec::new(),
        tuning_json: ROOM_TUNING_JSON.to_string(),
        created: now,
        commands,
        #[cfg(feature = "dev-tools")]
        frozen: false,
        #[cfg(feature = "dev-tools")]
        dev_events: std::collections::VecDeque::new(),
        #[cfg(feature = "dev-tools")]
        dev_seq: 0,
    };
    info!(code = room.code, map = room.map.name.as_deref().unwrap_or("?"), "room created");
    room.refresh_stats();
    if *drain.borrow_and_update() {
        room.begin_drain();
    }
    loop {
        let deadline = room.next_deadline();
        tokio::select! {
            biased;
            Ok(()) = drain.changed(), if !room.life.draining() => room.begin_drain(),
            cmd = room.commands.recv() => match cmd {
                Some(cmd) => room.handle(cmd),
                None => break,
            },
            _ = room.interval.tick(), if room.life.ticking() && !room.dev_frozen() => room.tick(),
            _ = tokio::time::sleep_until(deadline.unwrap_or_else(far_future)), if deadline.is_some() => {
                if room.on_deadline() {
                    break;
                }
            }
        }
    }
    room.close_all(if room.life.draining() { "this server is restarting; make a new room in a moment" } else { "the room closed" });
    info!(code = room.code, phase = room.life.phase().name(), "room dropped");
    room.hub.remove(&room.code);
}

fn far_future() -> Instant {
    Instant::now() + Duration::from_secs(365 * 24 * 3600)
}

impl Room {
    // -----------------------------------------------------------------
    // Seats and the lobby

    fn handle(&mut self, cmd: Command) {
        match cmd {
            Command::Join { nick, device_token, conn, reply } => {
                let _ = reply.send(self.join(nick, device_token, conn));
            }
            Command::Lobby { conn_id, msg } => {
                let Some(seat) = self.seat_of_conn(conn_id) else { return };
                if let Err(message) = self.lobby(seat, msg) {
                    self.lobby_to(seat, Lobby::Error { message });
                }
            }
            Command::Disconnected { conn_id } => self.disconnected(conn_id),
            #[cfg(feature = "dev-tools")]
            Command::Dev(request) => {
                let answer = self.dev(&request.method, &request.params);
                let _ = request.reply.send(answer);
            }
        }
    }

    fn seat_of_conn(&self, conn_id: u64) -> Option<u8> {
        self.seats
            .iter()
            .position(|s| s.as_ref().and_then(|s| s.conn.as_ref()).is_some_and(|c| c.id == conn_id))
            .map(|i| i as u8)
    }

    fn seat(&self, seat: u8) -> Option<&Seat> {
        self.seats.get(seat as usize).and_then(Option::as_ref)
    }

    fn seat_mut(&mut self, seat: u8) -> Option<&mut Seat> {
        self.seats.get_mut(seat as usize).and_then(Option::as_mut)
    }

    /// The host is the lowest seat: it passes on when the host leaves.
    fn host(&self) -> Option<u8> {
        self.seats.iter().position(Option::is_some).map(|i| i as u8)
    }

    fn occupied(&self) -> usize {
        self.seats.iter().filter(|s| s.is_some()).count()
    }

    fn join(&mut self, nick: String, device_token: String, conn: ConnLink) -> Result<Joined, String> {
        let now = Instant::now();
        if self.kicked.contains(&device_token) {
            return Err("the host removed you from this room".into());
        }
        let existing = self.seats.iter().position(|s| s.as_ref().is_some_and(|s| s.device_token == device_token));
        let seat = match existing {
            Some(i) => {
                let seat = self.seats[i].as_mut().expect("found by position");
                if let Some(old) = seat.conn.take() {
                    old.outbox.lobby(Lobby::Error { message: "reconnected from another socket".into() });
                    old.outbox.close();
                } else {
                    self.life.connect(now);
                }
                seat.conn = Some(conn);
                seat.away_since = None;
                seat.needs_full = true;
                seat.mailbox.clear();
                self.hub.metrics.reconnects_total.fetch_add(1, Ordering::Relaxed);
                info!(code = self.code, seat = i, nick = seat.nick, "seat reclaimed");
                i as u8
            }
            None => {
                if self.life.phase() != Phase::Waiting {
                    return Err("the round has started; this room takes no new seats".into());
                }
                // A waiting room's seats are dense - `remove_seat` and
                // `expire_graces` compact them and only a waiting room
                // takes a join - so this caps the seat vector too.
                if self.occupied() >= SEATS_PLAYABLE {
                    return Err(format!("the room is full: {SEATS_PLAYABLE} seats"));
                }
                let nick = clean_nick(&nick);
                let i = self.seats.len();
                self.seats.push(Some(Seat {
                    nick: nick.clone(),
                    device_token,
                    ready: false,
                    #[cfg(feature = "dev-tools")]
                    bot: false,
                    #[cfg(feature = "dev-tools")]
                    script: None,
                    conn: Some(conn),
                    away_since: None,
                    mailbox: Arc::new(Mailbox::new()),
                    needs_full: true,
                }));
                self.life.connect(now);
                info!(code = self.code, seat = i, nick, "seat joined");
                i as u8
            }
        };
        if self.life.ticking() {
            self.interval.reset();
        }
        let mailbox = self.seat(seat).expect("just seated").mailbox.clone();
        let welcome = self.welcome(seat);
        self.send_to(seat, Msg::Welcome(welcome));
        if self.life.phase() == Phase::Ended {
            // A seat reclaimed after the round finished is welcomed into
            // the last tick of it, which no snapshot will ever follow, so
            // it is told the round is over in the same breath.
            let outcome = self.game.as_ref().map_or(Outcome::Playing, |g| g.outcome());
            self.lobby_to(seat, Lobby::Ended { outcome: outcome.into() });
        }
        self.broadcast_roster();
        self.refresh_stats();
        Ok(Joined { seat, mailbox })
    }

    fn disconnected(&mut self, conn_id: u64) {
        let Some(i) = self.seat_of_conn(conn_id) else { return };
        let now = Instant::now();
        let seat = self.seat_mut(i).expect("found by conn");
        seat.conn = None;
        seat.away_since = Some(now);
        seat.mailbox.clear();
        let nick = seat.nick.clone();
        info!(code = self.code, seat = i, nick, "seat dropped, grace running");
        self.life.disconnect(now);
        if self.life.phase() == Phase::Paused {
            info!(code = self.code, frame = self.game.as_ref().map_or(0, |g| g.frame()), "nobody connected, round paused");
        }
        self.broadcast_roster();
        self.refresh_stats();
    }

    fn lobby(&mut self, seat: u8, msg: Lobby) -> Result<(), String> {
        match msg {
            Lobby::Ready => {
                if let Some(s) = self.seat_mut(seat) {
                    s.ready = true;
                }
                self.broadcast_roster();
                Ok(())
            }
            Lobby::Start => self.start(seat),
            Lobby::Leave => {
                self.remove_seat(seat, "you left the room");
                Ok(())
            }
            Lobby::Kick { seat: target } => {
                if self.host() != Some(seat) {
                    return Err("only the host kicks".into());
                }
                if target == seat {
                    return Err("the host cannot kick themself".into());
                }
                let token = self.seat(target).map(|s| s.device_token.clone()).ok_or("no such seat")?;
                self.kicked.insert(token);
                self.remove_seat(target, "the host removed you from the room");
                Ok(())
            }
            Lobby::Chat { text } => {
                let text: String = text.chars().take(200).collect();
                self.lobby_to_all(Lobby::Said { seat, text });
                Ok(())
            }
            _ => Err("that message is the server's to send".into()),
        }
    }

    /// Free `seat`: told why, closed, and in a waiting room the seats
    /// above it move down (a fresh `Welcome` at start names the final
    /// number).
    fn remove_seat(&mut self, seat: u8, why: &str) {
        let Some(s) = self.seats.get_mut(seat as usize).and_then(Option::take) else { return };
        if let Some(conn) = &s.conn {
            conn.outbox.lobby(Lobby::Error { message: why.into() });
            conn.outbox.close();
            self.life.disconnect(Instant::now());
        }
        info!(code = self.code, seat, nick = s.nick, why, "seat freed");
        if self.life.phase() == Phase::Waiting {
            self.seats.retain(Option::is_some);
        }
        self.broadcast_roster();
        self.refresh_stats();
    }

    /// In a waiting room, seats past their grace are freed.
    fn expire_graces(&mut self) {
        if self.life.phase() != Phase::Waiting {
            return;
        }
        let now = Instant::now();
        let expired: Vec<u8> = self
            .seats
            .iter()
            .enumerate()
            .filter(|(_, s)| s.as_ref().is_some_and(|s| s.away_since.is_some_and(|t| t + SEAT_GRACE <= now)))
            .map(|(i, _)| i as u8)
            .collect();
        // Highest first, so the compaction never moves a seat still to go.
        for seat in expired.into_iter().rev() {
            self.remove_seat(seat, "grace over");
        }
    }

    fn start(&mut self, by: u8) -> Result<(), String> {
        if self.host() != Some(by) {
            return Err("only the host starts".into());
        }
        match self.life.phase() {
            Phase::Waiting => {}
            Phase::Ended if self.hub.draining() => {
                return Err("this server is draining; make a new room for the rematch".into());
            }
            Phase::Ended => {}
            Phase::Playing | Phase::Paused => return Err("the round is in progress".into()),
        }
        if let Some(not_ready) = self.seats.iter().flatten().find(|s| !s.ready && s.device_token != self.seat(by).expect("host").device_token) {
            return Err(format!("{} is not ready", not_ready.nick));
        }
        if self.life.phase() == Phase::Waiting {
            self.seats.retain(Option::is_some);
        }
        // As many players as the room has seats up to its highest
        // occupied one, so every seat on the roster has a tank on the
        // field. In a waiting room the seats are dense and that is
        // `occupied()`; a rematch out of a round somebody left in the
        // middle of keeps the hole and its idle tank rather than handing
        // a seat a tank that was never spawned.
        let players = self.seats.iter().rposition(Option::is_some).map_or(1, |i| i + 1).clamp(1, SEATS_PLAYABLE);
        let seed = self.pinned_seed.unwrap_or_else(|| rand::random::<u64>());
        let mut game = Box::new(Game::default());
        game.map = self.map.clone();
        game.level_overrides = self.overrides;
        game.players = PlayerCount::from_count(players).expect("clamped to the seats a room takes");
        game.seed_override = Some(seed);
        let (w, h) = self.map.field_size();
        // The wave plan this team's size asks for, on the table for the
        // one call that reads it and in every `Welcome` behind it.
        self.tuning_json = tuning_patch(players);
        init_under(&self.tuning_json, &mut game, w, h);
        self.round_seed = game.round_seed();
        // `init`'s own events (the round start) travel in the welcome's
        // snapshot, which is frame 0's.
        self.pending_events.clear();
        self.game = Some(game);
        for seat in self.seats.iter_mut().flatten() {
            seat.ready = false;
            seat.mailbox.clear();
        }
        self.life.start(Instant::now());
        self.interval.reset();
        info!(code = self.code, seed = format!("{:#x}", self.round_seed), players, tuning = self.tuning_json, "round started");
        self.lobby_to_all(Lobby::Started);
        // Everyone's baseline is the welcome's snapshot (the same frame
        // for every seat), so the first delta applies straight onto it.
        for i in 0..self.seats.len() {
            let Some(seat) = self.seats[i].as_mut() else { continue };
            seat.needs_full = false;
            let welcome = self.welcome(i as u8);
            self.prev = welcome.snapshot.clone();
            self.send_to(i as u8, Msg::Welcome(welcome));
        }
        self.refresh_stats();
        Ok(())
    }

    fn end_round(&mut self, why: &str) {
        let frame = self.game.as_ref().map_or(0, |g| g.frame());
        let outcome = self.game.as_ref().map_or(Outcome::Playing, |g| g.outcome());
        self.life.end(Instant::now());
        info!(code = self.code, frame, ?outcome, why, "round ended");
        // The snapshots stop with the tick, so how the round went travels
        // as a lobby message instead, and the roster behind it is the one
        // the room screen comes back on - every seat unready again, ready
        // for the host's rematch.
        self.lobby_to_all(Lobby::Ended { outcome: outcome.into() });
        self.broadcast_roster();
        self.refresh_stats();
    }

    /// The server began draining: this room's deadlines shorten to the
    /// drain's (`Lifecycle::deadline`).
    fn begin_drain(&mut self) {
        self.life.drain(Instant::now());
        info!(code = self.code, phase = self.life.phase().name(), "draining");
    }

    /// A deadline came due; `true` when the room is to be dropped.
    fn on_deadline(&mut self) -> bool {
        let now = Instant::now();
        match self.life.expired(now) {
            Some(Expiry::Reap) => return true,
            Some(Expiry::EndRound) => {
                let acked = self.acked();
                self.broadcast_snapshot(acked);
                self.end_round("round time limit");
            }
            None => {}
        }
        self.expire_graces();
        false
    }

    /// The next moment something is due: the lifecycle's deadline or a
    /// waiting seat's grace.
    fn next_deadline(&self) -> Option<Instant> {
        let mut due = self.life.deadline();
        if self.life.phase() == Phase::Waiting {
            for seat in self.seats.iter().flatten() {
                if let Some(t) = seat.away_since {
                    let grace = t + SEAT_GRACE;
                    due = Some(due.map_or(grace, |d| d.min(grace)));
                }
            }
        }
        due
    }

    fn close_all(&mut self, why: &str) {
        for seat in self.seats.iter_mut().flatten() {
            if let Some(conn) = seat.conn.take() {
                conn.outbox.lobby(Lobby::Error { message: why.into() });
                conn.outbox.close();
            }
        }
    }

    fn refresh_stats(&self) {
        self.stats.set_phase(self.life.phase());
        let connected = self.seats.iter().flatten().filter(|s| s.connected()).count();
        self.stats.seats_connected.store(connected, Ordering::Relaxed);
        self.stats.seats_away.store(self.occupied() - connected, Ordering::Relaxed);
    }

    // -----------------------------------------------------------------
    // Messages out

    fn roster(&self) -> Lobby {
        let seats = self
            .seats
            .iter()
            .enumerate()
            .filter_map(|(i, s)| {
                let s = s.as_ref()?;
                Some(RosterSeat { seat: i as u8, nick: s.nick.clone(), chassis: self.chassis(i as u8), ready: s.ready, connected: s.connected() })
            })
            .collect();
        Lobby::Roster { host: self.host().unwrap_or(0), seats }
    }

    fn broadcast_roster(&self) {
        self.lobby_to_all(self.roster());
    }

    fn lobby_to_all(&self, msg: Lobby) {
        let bytes = Bytes::from(codec::encode(&Msg::Lobby(msg)));
        for seat in self.seats.iter().flatten() {
            if let Some(conn) = &seat.conn {
                conn.outbox.send(bytes.clone());
            }
        }
    }

    fn lobby_to(&self, seat: u8, msg: Lobby) {
        self.send_to(seat, Msg::Lobby(msg));
    }

    fn send_to(&self, seat: u8, msg: Msg) {
        if let Some(conn) = self.seat(seat).and_then(|s| s.conn.as_ref()) {
            conn.outbox.send(Bytes::from(codec::encode(&msg)));
        }
    }

    /// The chassis row a seat's tank has, 0 before the round rolls it.
    fn chassis(&self, seat: u8) -> u8 {
        let Some(game) = &self.game else { return 0 };
        let kind = match seat {
            0 => game.player_chassis(),
            1 => game.player2_chassis(),
            _ => None,
        };
        kind.map_or(0, |k| k.row() as u8)
    }

    /// The welcome for `seat`: in a round, `encode::welcome` on the live
    /// world (its current snapshot, the holes shot in the map, the pins
    /// the replica's `init` needs), so a joiner or a rejoiner builds the
    /// round as it stands now; in a waiting room, the map and the roster
    /// with an empty snapshot.
    fn welcome(&self, seat: u8) -> Welcome {
        let roster: Vec<WireSeat> = self
            .seats
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.as_ref().map(|s| WireSeat { seat: i as u8, nick: s.nick.clone(), chassis: self.chassis(i as u8) }))
            .collect();
        let acked = self.acked();
        if let Some(game) = &self.game {
            match encode::welcome(game, seat, roster.clone(), self.tuning_json.clone(), acked) {
                Ok(mut welcome) => {
                    welcome.snapshot.server_ms = self.server_ms();
                    return welcome;
                }
                Err(e) => warn!(code = self.code, error = e, "welcome could not be encoded; sending the map alone"),
            }
        }
        Welcome {
            protocol: PROTOCOL_VERSION,
            sim_version: SIM_VERSION.into(),
            seat,
            roster,
            map_toml: self.map.to_toml_string().unwrap_or_default(),
            seed: self.round_seed,
            tuning_json: ROOM_TUNING_JSON.into(),
            overrides: self.overrides.into(),
            enemy_count: None,
            oil_cells: Vec::new(),
            dead_cells: Vec::new(),
            snapshot: Snapshot { acked, server_ms: self.server_ms(), ..Snapshot::default() },
        }
    }

    /// The room's clock, milliseconds since it was created.
    fn server_ms(&self) -> u32 {
        self.created.elapsed().as_millis().min(u32::MAX as u128) as u32
    }

    // -----------------------------------------------------------------
    // The tick and the snapshots

    fn acked(&self) -> [u32; MAX_SEATS] {
        let mut acked = [0; MAX_SEATS];
        for (i, seat) in self.seats.iter().enumerate().take(MAX_SEATS) {
            if let Some(s) = seat {
                acked[i] = s.mailbox.acked_tick();
            }
        }
        acked
    }

    /// The end screen's countdown runs out on the next `update`, which is
    /// where a local round calls `init` and starts over. A room does not:
    /// its seats would be dragged into a round nobody asked for, so the
    /// round ends here instead and the host asks for the rematch.
    fn restart_is_due(&self) -> bool {
        self.game
            .as_ref()
            .is_some_and(|g| g.outcome() != Outcome::Playing && g.restart_countdown() <= PHYSICS_FIXED_DT)
    }

    fn tick(&mut self) {
        if self.restart_is_due() {
            // The ticks since the last snapshot banked events nobody has
            // been sent; they go out with the state they happened in
            // before the stream stops.
            if !self.pending_events.is_empty() {
                let acked = self.acked();
                self.broadcast_snapshot(acked);
            }
            self.end_round("outcome");
            return;
        }
        let now = Instant::now();
        #[cfg(feature = "dev-tools")]
        self.feed_dev_scripts(now);
        let mut input = Input::default();
        for (i, seat) in self.seats.iter().enumerate().take(MAX_SEATS) {
            if let Some(s) = seat
                && s.connected()
            {
                let before = s.mailbox.starvations();
                input.seats[i] = s.mailbox.read(now).map(|m| m.intent()).unwrap_or_default();
                // A starved tick means this seat's client is not stamping
                // far enough ahead for the link (`mailbox`, §4.12).
                let starved = s.mailbox.starvations() - before;
                if starved > 0 {
                    self.hub.metrics.intent_starvations_total.fetch_add(starved, Ordering::Relaxed);
                }
            }
        }
        let acked = self.acked();
        let (w, h) = self.map.field_size();
        let game = self.game.as_mut().expect("a playing room has a game");
        let was_playing = game.outcome() == Outcome::Playing;
        let began = std::time::Instant::now();
        game.update(input, PHYSICS_FIXED_DT, w, h);
        let took = began.elapsed();
        self.hub.metrics.record_tick(took);
        if took > TICK {
            self.hub.metrics.tick_overruns_total.fetch_add(1, Ordering::Relaxed);
            warn!(code = self.code, frame = game.frame(), took_us = took.as_micros() as u64, "tick overran");
        }
        let frame = game.frame();
        #[cfg(feature = "dev-tools")]
        self.record_dev_events(frame);
        let game = self.game.as_mut().expect("a playing room has a game");
        // The round is over from this tick on, and the end screen is a
        // tick like any other: the shots land, the fires burn down and
        // the restart counts toward zero, all of it on the room's clock
        // and in the snapshots, so every seat watches the same end. The
        // tick the counter runs out on never happens (`restart_is_due`).
        // The tick it turned on gets a snapshot out of cadence, because
        // the `RoundEnded` event in it is what puts the banner up.
        let just_ended = was_playing && game.outcome() != Outcome::Playing;
        if frame % SNAPSHOT_EVERY == 0 || just_ended {
            self.broadcast_snapshot(acked);
        } else {
            // A tick that sends nothing banks its events for the next
            // snapshot, whose own events are its frame's.
            self.pending_events.extend(encode::wire_events(game.events()));
        }
    }

    /// The state now as a `Snapshot`: the encoder's, stamped with the
    /// room's clock, the banked events of the ticks since the last
    /// snapshot ahead of this frame's own.
    fn fresh_snapshot(&self, acked: [u32; MAX_SEATS]) -> Snapshot {
        let mut snap = match &self.game {
            Some(game) => encode::snapshot(game, acked),
            None => Snapshot { acked, ..Snapshot::default() },
        };
        snap.server_ms = self.server_ms();
        if !self.pending_events.is_empty() {
            let mut events = self.pending_events.clone();
            events.append(&mut snap.events);
            snap.events = events;
        }
        snap
    }

    /// Encode this tick's snapshot once - the delta for every seat in
    /// step, the full one for those that need it - and offer it to every
    /// connected seat's outbox.
    fn broadcast_snapshot(&mut self, acked: [u32; MAX_SEATS]) {
        let snap = self.fresh_snapshot(acked);
        self.pending_events.clear();
        let delta_bytes = Bytes::from(codec::encode(&Msg::Delta(delta(&self.prev, &snap))));
        let mut full_bytes: Option<Bytes> = None;
        let metrics = self.hub.metrics.clone();
        for seat in self.seats.iter_mut().flatten() {
            let Some(conn) = &seat.conn else { continue };
            let bytes = if seat.needs_full {
                full_bytes.get_or_insert_with(|| Bytes::from(codec::encode(&Msg::Snapshot(snap.clone())))).clone()
            } else {
                delta_bytes.clone()
            };
            let len = bytes.len();
            match conn.outbox.offer(bytes) {
                Delivery::Sent => {
                    seat.needs_full = false;
                    metrics.record_snapshot_bytes(len);
                }
                Delivery::Skipped => {
                    seat.needs_full = true;
                    metrics.snapshots_skipped_total.fetch_add(1, Ordering::Relaxed);
                }
                Delivery::Gone => {}
            }
        }
        self.prev = snap;
    }
}

/// A nickname as the roster shows it: trimmed, cut at `NICK_MAX`, never
/// empty.
fn clean_nick(nick: &str) -> String {
    let nick: String = nick.trim().chars().take(NICK_MAX).collect();
    if nick.is_empty() { "player".into() } else { nick }
}

/// Whether a dev tool has taken this room off real time (`room_step`).
/// Always false in a build without the tools, so the tick guard reads
/// the same either way.
impl Room {
    #[cfg(not(feature = "dev-tools"))]
    fn dev_frozen(&self) -> bool {
        false
    }

    #[cfg(feature = "dev-tools")]
    fn dev_frozen(&self) -> bool {
        self.frozen
    }
}

/// The dev tools' side of a room (`crate::devserver`, feature
/// `dev-tools`). Every method here runs between ticks, with the room's
/// whole state in hand.
#[cfg(feature = "dev-tools")]
mod dev {
    use super::*;
    use bongbong::ai::Intent;
    use bongbong::net::wire::IntentMsg;
    use bongbong::simulation::debug::Detail;
    use bongbong::tank::Dir;
    use serde_json::{Value, json};

    /// Events kept for `room_events`, the game's dev server's own depth.
    const DEV_EVENT_RING: usize = 4096;

    fn dir(params: &Value, key: &str) -> Result<Option<Dir>, String> {
        match params.get(key).and_then(Value::as_str) {
            None => Ok(None),
            Some("up") => Ok(Some(Dir::Up)),
            Some("down") => Ok(Some(Dir::Down)),
            Some("left") => Ok(Some(Dir::Left)),
            Some("right") => Ok(Some(Dir::Right)),
            Some(other) => Err(format!("{key}: {other:?} is not up|down|left|right")),
        }
    }

    impl Room {
        /// One dev call. The single dispatch table: every arm is a row of
        /// `bongbong::devserver::ROOM_TOOLS` and every row is an arm.
        pub(super) fn dev(&mut self, method: &str, params: &Value) -> Result<Value, String> {
            match method {
                "room" => Ok(self.dev_room()),
                "room_open" => self.dev_open(params),
                "room_step" => self.dev_step(params),
                "room_resume" => {
                    self.frozen = false;
                    self.interval.reset();
                    Ok(json!({ "frozen": false, "tick": self.dev_tick() }))
                }
                "seat_intent" => self.dev_seat_intent(params),
                "room_snapshot" => self.dev_snapshot(params),
                "room_events" => Ok(self.dev_events_since(params)),
                "room_close" => {
                    self.end_round("a dev tool closed the room");
                    Ok(json!({ "closed": true, "code": self.code }))
                }
                other => Err(format!("unknown tool {other:?}")),
            }
        }

        fn dev_tick(&self) -> u64 {
            self.game.as_ref().map_or(0, |g| g.frame())
        }

        /// The room in full, seats and mailboxes included - the reading
        /// that says whether an input was lost on the way in.
        fn dev_room(&self) -> Value {
            let seats: Vec<Value> = self
                .seats
                .iter()
                .enumerate()
                .map(|(i, seat)| match seat {
                    None => json!({ "seat": i, "empty": true }),
                    Some(s) => json!({
                        "seat": i,
                        "nick": s.nick,
                        "host": self.host() == Some(i as u8),
                        "ready": s.ready,
                        "connected": s.conn.is_some(),
                        "bot": s.bot,
                        "driving_ticks_left": s.script.map(|d| d.remaining),
                        "mailbox": {
                            "depth": s.mailbox.depth(),
                            "acked": s.mailbox.acked_tick(),
                            "starvations": s.mailbox.starvations(),
                        },
                    }),
                })
                .collect();
            json!({
                "code": self.code,
                "phase": self.life.phase().name(),
                "frozen": self.frozen,
                "tick": self.dev_tick(),
                "map": self.map.name,
                "seed": self.round_seed,
                "outcome": self.game.as_ref().map(|g| format!("{:?}", g.outcome())),
                "players": self.game.as_ref().map(|g| g.players.count()),
                "tuning_patch": serde_json::from_str::<Value>(&self.tuning_json).unwrap_or(Value::Null),
                "seats": seats,
            })
        }

        /// Seat `seats` bots and start at once: the lobby's dance with
        /// the waiting-for-a-human part left out.
        fn dev_open(&mut self, params: &Value) -> Result<Value, String> {
            let seats = params.get("seats").and_then(Value::as_u64).unwrap_or(1) as usize;
            if self.life.phase() != Phase::Waiting {
                return Err("this room has already started".into());
            }
            let now = Instant::now();
            for i in 0..seats {
                self.seats.push(Some(Seat {
                    nick: format!("bot{i}"),
                    device_token: format!("dev-bot-{}-{i}", self.code),
                    // Ready by construction: `start` refuses otherwise,
                    // and a bot has nothing to wait for.
                    ready: true,
                    bot: true,
                    script: None,
                    conn: None,
                    away_since: None,
                    mailbox: Arc::new(Mailbox::new()),
                    needs_full: true,
                }));
                self.life.connect(now);
            }
            self.start(0)?;
            self.refresh_stats();
            Ok(json!({ "seats": seats, "tick": self.dev_tick(), "seed": self.round_seed }))
        }

        /// Freeze and advance exactly `ticks`, the game's `step` for a
        /// room: deterministic, and as fast as the ticks take.
        fn dev_step(&mut self, params: &Value) -> Result<Value, String> {
            if self.game.is_none() {
                return Err("this room has no round yet - `room_open` it or have a client start it".into());
            }
            let ticks = params.get("ticks").and_then(Value::as_u64).unwrap_or(1);
            if !(1..=100_000).contains(&ticks) {
                return Err("ticks must be 1..=100000".into());
            }
            self.frozen = true;
            let from = self.dev_seq;
            for _ in 0..ticks {
                if !self.life.ticking() {
                    break;
                }
                self.tick();
            }
            let mut out = json!({
                "frozen": true,
                "tick": self.dev_tick(),
                "phase": self.life.phase().name(),
                "events": self.dev_events_from(from, 256, params),
            });
            if params.get("snapshot").and_then(Value::as_bool).unwrap_or(true)
                && let Some(value) = self.dev_snapshot_value(Detail::Compact)
                && let Some(obj) = out.as_object_mut()
            {
                obj.insert("snapshot".into(), value);
            }
            Ok(out)
        }

        /// Drive a seat for the next `ticks` ticks. The script is *fed*
        /// one intent per tick by `feed_dev_scripts`, not posted in one
        /// go: the mailbox is a jitter buffer capped at `BUFFER_MAX`, so
        /// a hundred intents dropped in at once keeps the last eight.
        /// One a tick is also exactly what a real client does.
        fn dev_seat_intent(&mut self, params: &Value) -> Result<Value, String> {
            let seat = params.get("seat").and_then(Value::as_u64).unwrap_or(0) as usize;
            let ticks = params.get("ticks").and_then(Value::as_u64).unwrap_or(1);
            if !(1..=100_000).contains(&ticks) {
                return Err("ticks must be 1..=100000".into());
            }
            let intent = Intent {
                move_dir: dir(params, "move_dir")?,
                face: dir(params, "face")?,
                fire: params.get("fire").and_then(Value::as_bool).unwrap_or(false),
                ..Intent::default()
            };
            let fire_every = params.get("fire_every").and_then(Value::as_u64);
            let Some(Some(s)) = self.seats.get_mut(seat) else {
                return Err(format!("no seat {seat} in this room"));
            };
            // Carry on from wherever this seat's client tick has reached,
            // so a bot and a real client stamp one unbroken counter.
            let tick = s.mailbox.acked_tick() + s.mailbox.depth() as u32 + 1;
            s.script = Some(DevScript { intent, remaining: ticks, fire_every, tick, sent: 0 });
            Ok(json!({ "seat": seat, "driving": ticks, "from_tick": tick }))
        }

        /// One intent per scripted seat, posted just before the tick
        /// samples the mailboxes - the moment a packet would have landed.
        pub(super) fn feed_dev_scripts(&mut self, now: Instant) {
            for seat in self.seats.iter_mut().flatten() {
                let Some(script) = seat.script.as_mut() else { continue };
                if script.remaining == 0 {
                    seat.script = None;
                    continue;
                }
                // A shell fires once per press: with `fire_every` the
                // trigger is down on 0, N, 2N... and up in between, the
                // only way a held intent ever re-arms one.
                let fire = script.intent.fire
                    && script.fire_every.is_none_or(|n| script.sent % n == 0);
                let msg = IntentMsg::new(script.tick, &Intent { fire, ..script.intent });
                seat.mailbox.post(msg, now);
                script.tick = script.tick.wrapping_add(1);
                script.sent += 1;
                script.remaining -= 1;
            }
        }

        fn dev_snapshot_value(&self, detail: Detail) -> Option<Value> {
            let game = self.game.as_ref()?;
            let (w, h) = self.map.field_size();
            serde_json::to_value(game.debug_snapshot(w, h, detail)).ok()
        }

        fn dev_snapshot(&self, params: &Value) -> Result<Value, String> {
            let detail = match params.get("detail").and_then(Value::as_str) {
                Some("full") => Detail::Full,
                _ => Detail::Compact,
            };
            self.dev_snapshot_value(detail).ok_or_else(|| "this room has no round yet".to_string())
        }

        /// Bank this tick's events into the ring `room_events` reads.
        pub(super) fn record_dev_events(&mut self, frame: u64) {
            let Some(game) = self.game.as_ref() else { return };
            for event in game.events() {
                self.dev_seq += 1;
                let mut value = serde_json::to_value(event).unwrap_or(Value::Null);
                if let Some(obj) = value.as_object_mut() {
                    obj.insert("seq".into(), json!(self.dev_seq));
                    obj.insert("frame".into(), json!(frame));
                }
                self.dev_events.push_back(value);
            }
            while self.dev_events.len() > DEV_EVENT_RING {
                self.dev_events.pop_front();
            }
        }

        fn dev_events_since(&self, params: &Value) -> Value {
            let since = params.get("since").and_then(Value::as_u64).unwrap_or(0);
            let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(200) as usize;
            let events = self.dev_events_from(since, limit, params);
            let next = events.last().and_then(|e| e.get("seq").and_then(Value::as_u64)).unwrap_or(since);
            json!({ "next": next, "events": events })
        }

        /// The ring past `since`, `kinds`/`exclude` applied - the game's
        /// `events` filter, spelled the same way.
        fn dev_events_from(&self, since: u64, limit: usize, params: &Value) -> Vec<Value> {
            let names = |key: &str| {
                params
                    .get(key)
                    .and_then(Value::as_array)
                    .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect::<Vec<_>>())
            };
            let (keep, drop) = (names("kinds"), names("exclude"));
            self.dev_events
                .iter()
                .filter(|e| e.get("seq").and_then(Value::as_u64).is_some_and(|s| s > since))
                .filter(|e| {
                    let kind = e.get("event").and_then(Value::as_str).unwrap_or("");
                    keep.as_ref().is_none_or(|k| k.iter().any(|n| n == kind))
                        && drop.as_ref().is_none_or(|d| !d.iter().any(|n| n == kind))
                })
                .take(limit)
                .cloned()
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bongbong::level::{LevelOverrides, SpawnConfig, SpawnKind, Tier};

    async fn advance(d: Duration) {
        tokio::time::advance(d).await;
    }

    /// The patch as a client reads it: onto a *local* copy of the
    /// defaults, which is exactly what `tuning::submit_json` does to the
    /// staged table on the window's side, and never onto the global one
    /// (these tests run in parallel with rooms that own it).
    fn applied(seats: usize) -> Tuning {
        Tuning::DEFAULT.with_json_patch(&tuning_patch(seats)).expect("every row the room sends is a row the build has")
    }

    #[test]
    fn a_room_of_one_sends_the_empty_patch() {
        assert_eq!(tuning_patch(1), "{}");
        assert_eq!(tuning_patch(0), "{}", "a room with nobody in it is a room of one");
        let solo = applied(1);
        assert_eq!(solo.wave_size_scale, Tuning::DEFAULT.wave_size_scale);
        assert_eq!(solo.wave_tier_step, Tuning::DEFAULT.wave_tier_step);
    }

    #[test]
    fn the_patch_sizes_the_wave_plan_to_the_team() {
        let (per_seat, per_step) = {
            let t = tuning();
            (t.online_wave_size_per_seat, t.online_wave_tier_seats_per_step)
        };
        for seats in [2usize, 4, 8] {
            let t = applied(seats);
            let extra = seats - 1;
            assert_eq!(t.wave_size_scale, 1.0 + extra as f32 * per_seat, "{seats} seats");
            assert_eq!(t.wave_tier_step, extra / per_step, "{seats} seats");
            assert!(t.wave_size_scale > 1.0, "{seats} seats meet more than one player does");
        }
        // The shape the two dials currently make, spelled out: a pair
        // meets half a wave more than one player does, a team of four two
        // and a half times the authored wave and one rung up the tier
        // ladder, a full room of eight four and a half times it and two
        // rungs up.
        assert_eq!(tuning_patch(2), r#"{"wave_size_scale":1.5,"wave_tier_step":0}"#);
        assert_eq!(tuning_patch(4), r#"{"wave_size_scale":2.5,"wave_tier_step":1}"#);
        assert_eq!(tuning_patch(8), r#"{"wave_size_scale":4.5,"wave_tier_step":2}"#);
    }

    /// The knobs are only half the answer: this is the plan a client's
    /// `Game::init` resolves once it has applied them - the same call the
    /// room made for its own world.
    #[test]
    fn a_client_applying_the_patch_resolves_the_rooms_plan() {
        // maps/default.toml's plan: four waves of 2, growing by 1, light
        // up to super.
        let map = SpawnConfig {
            kind: SpawnKind::Waves,
            waves: Some(4),
            size: Some(2),
            growth: Some(1),
            tier_start: Some(Tier::Light),
            tier_end: Some(Tier::Super),
        };
        let authored = LevelOverrides::default().resolve_spawn(&map, None);
        let plan = |seats: usize| {
            let t = applied(seats);
            authored.scaled(t.wave_size_scale, t.wave_tier_step)
        };
        let sizes = |seats: usize| (0..4).map(|i| plan(seats).wave_size(i)).collect::<Vec<_>>();
        assert_eq!(sizes(1), vec![2, 3, 4, 5], "a room of one fights the map as authored");
        // **The opening is the number that matters**, and the scaling
        // multiplies it rather than only the ramp: a pair opens one tank
        // above the solo round, not three or four above it.
        assert_eq!(sizes(2), vec![3, 5, 7, 9]);
        assert_eq!(sizes(4), vec![5, 8, 11, 14]);
        assert_eq!(sizes(8), vec![9, 14, 19, 24]);
        assert_eq!(plan(1).wave_tier(0), Tier::Light);
        assert_eq!(plan(4).wave_tier(0), Tier::Medium, "four seats start a rung up the ladder");
        assert_eq!(plan(8).wave_tier(0), Tier::Heavy);
        assert_eq!(plan(8).wave_tier(3), Tier::Super, "and still finish at the top of it");
    }

    #[tokio::test(start_paused = true)]
    async fn a_waiting_room_is_reaped_thirty_minutes_after_the_last_seat_leaves() {
        let mut life = Lifecycle::new(Instant::now());
        assert_eq!(life.phase(), Phase::Waiting);
        assert!(!life.ticking());
        life.connect(Instant::now());
        assert_eq!(life.deadline(), None, "a room with somebody in it waits as long as it likes");
        advance(Duration::from_secs(3600)).await;
        assert_eq!(life.expired(Instant::now()), None);
        life.disconnect(Instant::now());
        assert_eq!(life.deadline(), Some(Instant::now() + WAITING_TTL));
        advance(WAITING_TTL - Duration::from_secs(1)).await;
        assert_eq!(life.expired(Instant::now()), None);
        advance(Duration::from_secs(1)).await;
        assert_eq!(life.expired(Instant::now()), Some(Expiry::Reap));
    }

    #[tokio::test(start_paused = true)]
    async fn a_round_pauses_when_the_last_seat_drops_and_is_dropped_after_five_minutes() {
        let mut life = Lifecycle::new(Instant::now());
        life.connect(Instant::now());
        life.connect(Instant::now());
        life.start(Instant::now());
        assert_eq!(life.phase(), Phase::Playing);
        assert!(life.ticking());
        life.disconnect(Instant::now());
        assert_eq!(life.phase(), Phase::Playing, "one seat still connected keeps ticking");
        life.disconnect(Instant::now());
        assert_eq!(life.phase(), Phase::Paused);
        assert!(!life.ticking());
        assert_eq!(life.deadline(), Some(Instant::now() + EMPTY_TTL));
        advance(Duration::from_secs(60)).await;
        life.connect(Instant::now());
        assert_eq!(life.phase(), Phase::Playing, "a reconnect resumes where it stopped");
        life.disconnect(Instant::now());
        advance(EMPTY_TTL).await;
        assert_eq!(life.expired(Instant::now()), Some(Expiry::Reap));
    }

    #[tokio::test(start_paused = true)]
    async fn a_round_ends_at_the_time_limit_and_an_ended_room_keeps_five_minutes() {
        let mut life = Lifecycle::new(Instant::now());
        life.connect(Instant::now());
        life.start(Instant::now());
        assert_eq!(life.deadline(), Some(Instant::now() + ROUND_MAX));
        advance(ROUND_MAX).await;
        assert_eq!(life.expired(Instant::now()), Some(Expiry::EndRound));
        life.end(Instant::now());
        assert_eq!(life.phase(), Phase::Ended);
        assert!(!life.ticking());
        assert_eq!(life.deadline(), Some(Instant::now() + ENDED_TTL));
        advance(ENDED_TTL - Duration::from_secs(1)).await;
        assert_eq!(life.expired(Instant::now()), None);
        advance(Duration::from_secs(1)).await;
        assert_eq!(life.expired(Instant::now()), Some(Expiry::Reap));
        life.start(Instant::now());
        assert_eq!(life.phase(), Phase::Playing, "a rematch from ended plays again");
    }

    #[tokio::test(start_paused = true)]
    async fn a_paused_round_still_hits_the_round_limit() {
        let mut life = Lifecycle::new(Instant::now());
        life.connect(Instant::now());
        life.start(Instant::now());
        advance(ROUND_MAX - Duration::from_secs(30)).await;
        life.disconnect(Instant::now());
        assert_eq!(life.phase(), Phase::Paused);
        assert_eq!(life.deadline(), Some(Instant::now() + Duration::from_secs(30)), "the round limit comes before the empty limit");
        advance(Duration::from_secs(30)).await;
        assert_eq!(life.expired(Instant::now()), Some(Expiry::EndRound));
    }

    #[tokio::test(start_paused = true)]
    async fn a_drain_waits_for_a_round_in_play_and_nothing_else() {
        let mut waiting = Lifecycle::new(Instant::now());
        waiting.connect(Instant::now());
        let mut playing = waiting;
        playing.start(Instant::now());
        let mut paused = playing;
        paused.disconnect(Instant::now());
        let mut ended = playing;
        advance(Duration::from_secs(60)).await;
        for life in [&mut waiting, &mut playing, &mut paused, &mut ended] {
            life.drain(Instant::now());
        }
        assert_eq!(waiting.expired(Instant::now()), Some(Expiry::Reap), "a waiting room goes at once, connected or not");
        assert_eq!(paused.expired(Instant::now()), Some(Expiry::Reap), "a paused round goes at once");
        assert_eq!(playing.deadline(), Some(Instant::now() + ROUND_MAX - Duration::from_secs(60)), "a round in play keeps its own limit");
        advance(Duration::from_secs(120)).await;
        ended.end(Instant::now());
        assert_eq!(ended.deadline(), Some(Instant::now() + DRAIN_ENDED_TTL), "an ended room keeps only long enough to read the result");
        advance(DRAIN_ENDED_TTL).await;
        assert_eq!(ended.expired(Instant::now()), Some(Expiry::Reap));
    }

    #[test]
    fn phase_round_trips_through_its_byte() {
        for phase in [Phase::Waiting, Phase::Playing, Phase::Paused, Phase::Ended] {
            assert_eq!(Phase::from_u8(phase as u8), phase);
        }
    }

    #[test]
    fn nicknames_are_trimmed_cut_and_never_empty() {
        assert_eq!(clean_nick("  oto "), "oto");
        assert_eq!(clean_nick("   "), "player");
        assert_eq!(clean_nick(&"x".repeat(100)).len(), NICK_MAX);
    }
}
