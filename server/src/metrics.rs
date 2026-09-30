//! What `/metrics` reports (docs/online-coop-prd.md §4.9, §4.16): rooms
//! by phase and the cap on them, rooms in play and rounds by map and
//! mission, seats, the clients by build, tick time and tick lateness
//! percentiles from rings of recent ticks, snapshot bytes per second,
//! reconnects. Counters are atomics the room and connection tasks bump;
//! the windows and the labelled families sit behind short mutexes.

use std::collections::BTreeMap;
use std::fmt::Write;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Ticks kept for the percentiles: at 60 Hz across every room, the last
/// few seconds of a busy server.
pub const TICK_RING: usize = 1024;

/// A tick that starts later than this after its schedule said is counted
/// in `ticks_late_total`: a third of a tick, past the scheduler's own
/// wake-up noise and far enough behind to be felt downstream as a
/// snapshot arriving out of step.
pub const LATE_TICK: Duration = Duration::from_millis(5);

/// The most distinct client builds the client families label; a build
/// past it counts under `other`. A label value is whatever a client
/// sends, so without a cap a client could mint a series per connection.
pub const CLIENT_LABELS_MAX: usize = 32;

/// The window `snapshot_bytes_per_second` averages over.
const BYTES_WINDOW: Duration = Duration::from_secs(1);

/// The last `TICK_RING` tick readings in microseconds, oldest overwritten.
struct TickRing {
    micros: Vec<u32>,
    next: usize,
}

impl TickRing {
    fn record(&mut self, micros: u32) {
        if self.micros.len() < TICK_RING {
            self.micros.push(micros);
        } else {
            self.micros[self.next] = micros;
        }
        self.next = (self.next + 1) % TICK_RING;
    }

    /// (p50, p99) in microseconds; zeros with no tick recorded.
    fn percentiles(&self) -> (u32, u32) {
        if self.micros.is_empty() {
            return (0, 0);
        }
        let mut sorted = self.micros.clone();
        sorted.sort_unstable();
        let at = |q: f64| sorted[((sorted.len() - 1) as f64 * q).round() as usize];
        (at(0.5), at(0.99))
    }
}

/// Snapshot bytes over the last whole `BYTES_WINDOW`.
struct BytesWindow {
    started: Instant,
    bytes: u64,
    rate: f64,
}

impl BytesWindow {
    fn add(&mut self, bytes: u64, now: Instant) {
        let elapsed = now.saturating_duration_since(self.started);
        if elapsed >= BYTES_WINDOW {
            self.rate = self.bytes as f64 / elapsed.as_secs_f64();
            self.bytes = 0;
            self.started = now;
        }
        self.bytes += bytes;
    }
}

/// Room counts by phase, gathered by the hub for one rendering.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RoomCounts {
    pub waiting: usize,
    pub playing: usize,
    pub paused: usize,
    pub ended: usize,
    /// The most rooms this server holds at once (`--max-rooms`).
    pub max_rooms: usize,
    /// Seats with somebody on the socket.
    pub seats_connected: usize,
    /// Seats owned but nobody on the socket.
    pub seats_away: usize,
    /// Rooms playing, by (map label, mission name).
    pub playing_by: BTreeMap<(String, &'static str), usize>,
}

/// A client build as the metrics label it (`ClientLabel::of`): every value
/// bounded, so a client choosing what it sends cannot choose what the
/// series are.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct ClientLabel {
    pub version: String,
    pub protocol: String,
    pub platform: String,
}

impl ClientLabel {
    /// The label for what a client said about itself; `unknown` for a
    /// build too old to say, `invalid` for a version that is not a short
    /// dotted version string, `other` for a platform off the list.
    pub fn of(client: Option<&bongbong::net::wire::ClientInfo>) -> ClientLabel {
        let Some(c) = client else {
            return ClientLabel { version: "unknown".into(), protocol: "unknown".into(), platform: "unknown".into() };
        };
        let version_ok = !c.version.is_empty()
            && c.version.len() <= 24
            && c.version.chars().all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '-' | '+'));
        let platform = match c.platform.as_str() {
            p @ ("web" | "ios" | "android" | "macos" | "windows" | "linux") => p,
            _ => "other",
        };
        ClientLabel {
            version: if version_ok { c.version.clone() } else { "invalid".into() },
            protocol: c.protocol.to_string(),
            platform: platform.into(),
        }
    }

    fn other() -> ClientLabel {
        ClientLabel { version: "other".into(), protocol: "other".into(), platform: "other".into() }
    }
}

/// The client families: open connections and seats taken, by build.
#[derive(Default)]
struct Clients {
    open: BTreeMap<ClientLabel, i64>,
    seats_total: BTreeMap<ClientLabel, u64>,
}

impl Clients {
    /// The key `label` counts under: itself once seen or while there is
    /// room for another, `other` past `CLIENT_LABELS_MAX`.
    fn key(&self, label: &ClientLabel) -> ClientLabel {
        let known = self.open.contains_key(label) || self.seats_total.contains_key(label);
        let distinct = self.open.keys().chain(self.seats_total.keys()).collect::<std::collections::BTreeSet<_>>().len();
        if known || distinct < CLIENT_LABELS_MAX { label.clone() } else { ClientLabel::other() }
    }
}

pub struct Metrics {
    ticks: Mutex<TickRing>,
    /// How late each tick started against the room's schedule
    /// (`net::authority::TickClock`).
    lateness: Mutex<TickRing>,
    bytes: Mutex<BytesWindow>,
    /// Rounds started, by (map label, mission).
    rounds_started: Mutex<BTreeMap<(String, &'static str), u64>>,
    /// Rounds ended, by (map label, mission, outcome).
    rounds_ended: Mutex<BTreeMap<(String, &'static str, &'static str), u64>>,
    clients: Mutex<Clients>,
    pub snapshot_bytes_total: AtomicU64,
    pub snapshots_skipped_total: AtomicU64,
    pub tick_overruns_total: AtomicU64,
    /// Ticks that started more than `LATE_TICK` after they were due.
    pub ticks_late_total: AtomicU64,
    /// Ticks a room gave up on after a stall longer than its catch-up
    /// (`net::authority::CATCH_UP_TICKS`): the round ran that much less
    /// than wall time.
    pub ticks_dropped_total: AtomicU64,
    pub ticks_total: AtomicU64,
    pub reconnects_total: AtomicU64,
    /// Ticks that found a seat's mailbox empty and repeated its last
    /// intent (`mailbox::Mailbox`). For a server-driven seat a steady
    /// climb means clients are not stamping far enough ahead for the
    /// link's jitter, which is what §4.12's adaptive lead is for; for an
    /// owned seat each is a packet late enough that the room
    /// dead-reckoned its hull.
    pub intent_starvations_total: AtomicU64,
    pub rooms_created_total: AtomicU64,
    pub connections_total: AtomicU64,
}

impl Default for Metrics {
    fn default() -> Self {
        Metrics {
            ticks: Mutex::new(TickRing { micros: Vec::with_capacity(TICK_RING), next: 0 }),
            lateness: Mutex::new(TickRing { micros: Vec::with_capacity(TICK_RING), next: 0 }),
            bytes: Mutex::new(BytesWindow { started: Instant::now(), bytes: 0, rate: 0.0 }),
            rounds_started: Mutex::new(BTreeMap::new()),
            rounds_ended: Mutex::new(BTreeMap::new()),
            clients: Mutex::new(Clients::default()),
            snapshot_bytes_total: AtomicU64::new(0),
            snapshots_skipped_total: AtomicU64::new(0),
            tick_overruns_total: AtomicU64::new(0),
            ticks_late_total: AtomicU64::new(0),
            ticks_dropped_total: AtomicU64::new(0),
            ticks_total: AtomicU64::new(0),
            reconnects_total: AtomicU64::new(0),
            intent_starvations_total: AtomicU64::new(0),
            rooms_created_total: AtomicU64::new(0),
            connections_total: AtomicU64::new(0),
        }
    }
}

impl Metrics {
    pub fn new() -> Metrics {
        Metrics::default()
    }

    /// One `Game::update` took `took`.
    pub fn record_tick(&self, took: Duration) {
        self.ticks_total.fetch_add(1, Ordering::Relaxed);
        let micros = took.as_micros().min(u32::MAX as u128) as u32;
        self.ticks.lock().expect("tick ring poisoned").record(micros);
    }

    /// A scheduled tick started `late` after it was due, and the schedule
    /// gave up on `dropped` ticks to get there.
    pub fn record_tick_start(&self, late: Duration, dropped: u32) {
        if late > LATE_TICK {
            self.ticks_late_total.fetch_add(1, Ordering::Relaxed);
        }
        if dropped > 0 {
            self.ticks_dropped_total.fetch_add(dropped as u64, Ordering::Relaxed);
        }
        let micros = late.as_micros().min(u32::MAX as u128) as u32;
        self.lateness.lock().expect("lateness ring poisoned").record(micros);
    }

    /// `bytes` of snapshot went to one seat.
    pub fn record_snapshot_bytes(&self, bytes: usize) {
        self.snapshot_bytes_total.fetch_add(bytes as u64, Ordering::Relaxed);
        self.bytes.lock().expect("bytes window poisoned").add(bytes as u64, Instant::now());
    }

    /// A room on `map` started a round of `mission`.
    pub fn round_started(&self, map: &str, mission: &'static str) {
        *self.rounds_started.lock().expect("rounds poisoned").entry((map.to_string(), mission)).or_default() += 1;
    }

    /// A room on `map` ended a round of `mission` with `outcome` (`won`,
    /// `lost`, or `unfinished` for one the room stopped before either).
    pub fn round_ended(&self, map: &str, mission: &'static str, outcome: &'static str) {
        *self.rounds_ended.lock().expect("rounds poisoned").entry((map.to_string(), mission, outcome)).or_default() += 1;
    }

    /// A connection asked for a room, from the build `label` names; it
    /// counts as open until `client_closed` with the label this returns.
    pub fn client_opened(&self, label: &ClientLabel) -> ClientLabel {
        let mut clients = self.clients.lock().expect("clients poisoned");
        let key = clients.key(label);
        *clients.open.entry(key.clone()).or_default() += 1;
        key
    }

    /// The connection `client_opened` counted is gone.
    pub fn client_closed(&self, key: &ClientLabel) {
        if let Some(n) = self.clients.lock().expect("clients poisoned").open.get_mut(key) {
            *n -= 1;
        }
    }

    /// A client of `label`'s build took a seat.
    pub fn client_seated(&self, label: &ClientLabel) {
        let mut clients = self.clients.lock().expect("clients poisoned");
        let key = clients.key(label);
        *clients.seats_total.entry(key).or_default() += 1;
    }

    /// (p50, p99) of the recorded ticks, microseconds.
    pub fn tick_percentiles(&self) -> (u32, u32) {
        self.ticks.lock().expect("tick ring poisoned").percentiles()
    }

    /// (p50, p99) of how late the recorded ticks started, microseconds.
    pub fn lateness_percentiles(&self) -> (u32, u32) {
        self.lateness.lock().expect("lateness ring poisoned").percentiles()
    }

    /// The counters as JSON, for the dev tools' `server_status` - the
    /// same numbers `render` publishes, without the Prometheus text.
    #[cfg(feature = "dev-tools")]
    pub fn summary(&self) -> serde_json::Value {
        let (p50, p99) = self.tick_percentiles();
        let (late50, late99) = self.lateness_percentiles();
        serde_json::json!({
            "tick_p50_us": p50,
            "tick_p99_us": p99,
            "tick_lateness_p50_us": late50,
            "tick_lateness_p99_us": late99,
            "ticks_total": self.ticks_total.load(Ordering::Relaxed),
            "tick_overruns_total": self.tick_overruns_total.load(Ordering::Relaxed),
            "ticks_late_total": self.ticks_late_total.load(Ordering::Relaxed),
            "ticks_dropped_total": self.ticks_dropped_total.load(Ordering::Relaxed),
            "snapshot_bytes_total": self.snapshot_bytes_total.load(Ordering::Relaxed),
            "snapshots_skipped_total": self.snapshots_skipped_total.load(Ordering::Relaxed),
            "reconnects_total": self.reconnects_total.load(Ordering::Relaxed),
            "intent_starvations_total": self.intent_starvations_total.load(Ordering::Relaxed),
            "rooms_created_total": self.rooms_created_total.load(Ordering::Relaxed),
            "connections_total": self.connections_total.load(Ordering::Relaxed),
        })
    }

    /// Prometheus text exposition.
    pub fn render(&self, rooms: RoomCounts, draining: bool) -> String {
        let (p50, p99) = self.tick_percentiles();
        let (late50, late99) = self.lateness_percentiles();
        let rate = self.bytes.lock().expect("bytes window poisoned").rate;
        let mut out = String::new();
        let playing: Vec<(String, f64)> = rooms
            .playing_by
            .iter()
            .map(|((map, mission), n)| (format!("{{map=\"{map}\",mission=\"{mission}\"}}"), *n as f64))
            .collect();
        let mut gauge = |name: &str, help: &str, rows: &[(&str, f64)]| {
            let _ = writeln!(out, "# HELP {name} {help}\n# TYPE {name} gauge");
            for (labels, value) in rows {
                let _ = writeln!(out, "{name}{labels} {value}");
            }
        };
        gauge(
            "bongbong_rooms",
            "Rooms on this server by phase.",
            &[
                ("{phase=\"waiting\"}", rooms.waiting as f64),
                ("{phase=\"playing\"}", rooms.playing as f64),
                ("{phase=\"paused\"}", rooms.paused as f64),
                ("{phase=\"ended\"}", rooms.ended as f64),
            ],
        );
        gauge(
            "bongbong_rooms_playing",
            "Rooms playing a round, by map (a shipped map's name, or custom) and mission.",
            &playing.iter().map(|(l, v)| (l.as_str(), *v)).collect::<Vec<_>>(),
        );
        gauge(
            "bongbong_rooms_max",
            "The most rooms this server holds at once (--max-rooms).",
            &[("", rooms.max_rooms as f64)],
        );
        gauge(
            "bongbong_seats",
            "Seats on this server by connection state.",
            &[("{state=\"connected\"}", rooms.seats_connected as f64), ("{state=\"away\"}", rooms.seats_away as f64)],
        );
        gauge(
            "bongbong_tick_microseconds",
            "Game::update wall time over the last ticks.",
            &[("{quantile=\"0.5\"}", p50 as f64), ("{quantile=\"0.99\"}", p99 as f64)],
        );
        gauge(
            "bongbong_tick_lateness_microseconds",
            "How late ticks started against the room's schedule, over the last ticks.",
            &[("{quantile=\"0.5\"}", late50 as f64), ("{quantile=\"0.99\"}", late99 as f64)],
        );
        gauge("bongbong_snapshot_bytes_per_second", "Snapshot bytes sent over the last second.", &[("", rate)]);
        gauge(
            "bongbong_draining",
            "1 while the server refuses new rooms and waits for its rounds to end.",
            &[("", draining as u8 as f64)],
        );
        gauge(
            "bongbong_info",
            "Build and protocol.",
            &[(
                &format!(
                    "{{version=\"{}\",game_version=\"{}\",protocol=\"{}\"}}",
                    env!("CARGO_PKG_VERSION"),
                    bongbong::net::BUILD_VERSION,
                    bongbong::net::PROTOCOL_VERSION
                ),
                1.0,
            )],
        );
        {
            let clients = self.clients.lock().expect("clients poisoned");
            let rows: Vec<(String, f64)> = clients.open.iter().map(|(k, n)| (client_labels(k), *n as f64)).collect();
            gauge(
                "bongbong_clients",
                "Open connections that asked for a room, by the client's build (version, protocol, platform).",
                &rows.iter().map(|(l, v)| (l.as_str(), *v)).collect::<Vec<_>>(),
            );
            labelled_counter(
                &mut out,
                "bongbong_client_seats_total",
                "Seats taken, by the client's build (version, protocol, platform).",
                clients.seats_total.iter().map(|(k, n)| (client_labels(k), *n)),
            );
        }
        labelled_counter(
            &mut out,
            "bongbong_rounds_started_total",
            "Rounds started, by map (a shipped map's name, or custom) and mission.",
            self.rounds_started
                .lock()
                .expect("rounds poisoned")
                .iter()
                .map(|((map, mission), n)| (format!("{{map=\"{map}\",mission=\"{mission}\"}}"), *n)),
        );
        labelled_counter(
            &mut out,
            "bongbong_rounds_ended_total",
            "Rounds ended, by map, mission and outcome (won, lost, unfinished).",
            self.rounds_ended
                .lock()
                .expect("rounds poisoned")
                .iter()
                .map(|((map, mission, outcome), n)| {
                    (format!("{{map=\"{map}\",mission=\"{mission}\",outcome=\"{outcome}\"}}"), *n)
                }),
        );
        let counters: [(&str, &str, &AtomicU64); 10] = [
            ("bongbong_ticks_total", "Game::update calls.", &self.ticks_total),
            ("bongbong_tick_overruns_total", "Ticks whose update took longer than the tick.", &self.tick_overruns_total),
            ("bongbong_ticks_late_total", "Ticks that started more than 5 ms after they were due.", &self.ticks_late_total),
            (
                "bongbong_ticks_dropped_total",
                "Ticks a room skipped after a stall longer than its catch-up.",
                &self.ticks_dropped_total,
            ),
            ("bongbong_snapshot_bytes_total", "Snapshot bytes handed to seat writers.", &self.snapshot_bytes_total),
            ("bongbong_snapshots_skipped_total", "Snapshots a slow seat did not get.", &self.snapshots_skipped_total),
            ("bongbong_reconnects_total", "Seats reclaimed with their device token.", &self.reconnects_total),
            (
                "bongbong_intent_starvations_total",
                "Ticks that found a seat's intent buffer empty and repeated the last one.",
                &self.intent_starvations_total,
            ),
            ("bongbong_rooms_created_total", "Rooms created.", &self.rooms_created_total),
            ("bongbong_connections_total", "WebSocket connections accepted.", &self.connections_total),
        ];
        for (name, help, value) in counters {
            let _ = writeln!(out, "# HELP {name} {help}\n# TYPE {name} counter\n{name} {}", value.load(Ordering::Relaxed));
        }
        out
    }
}

/// `{version="..",protocol="..",platform=".."}` for a client label.
fn client_labels(k: &ClientLabel) -> String {
    format!("{{version=\"{}\",protocol=\"{}\",platform=\"{}\"}}", k.version, k.protocol, k.platform)
}

/// A counter family with one row per label set.
fn labelled_counter(out: &mut String, name: &str, help: &str, rows: impl Iterator<Item = (String, u64)>) {
    let _ = writeln!(out, "# HELP {name} {help}\n# TYPE {name} counter");
    for (labels, value) in rows {
        let _ = writeln!(out, "{name}{labels} {value}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percentiles_come_from_the_ring_and_the_ring_wraps() {
        let m = Metrics::new();
        assert_eq!(m.tick_percentiles(), (0, 0));
        for i in 0..(TICK_RING + 100) as u64 {
            m.record_tick(Duration::from_micros(i));
        }
        let (p50, p99) = m.tick_percentiles();
        assert!(p50 >= 100, "the oldest 100 ticks were overwritten: p50 {p50}");
        assert!(p99 > p50);
        assert!(p99 <= (TICK_RING + 99) as u32);
    }

    /// Lateness has its own ring, and a tick past `LATE_TICK` or a
    /// schedule that gave up on ticks is counted.
    #[test]
    fn tick_lateness_is_a_ring_and_the_late_ones_are_counted() {
        let m = Metrics::new();
        assert_eq!(m.lateness_percentiles(), (0, 0));
        for micros in [100, 200, 300, 400] {
            m.record_tick_start(Duration::from_micros(micros), 0);
        }
        m.record_tick_start(LATE_TICK + Duration::from_micros(1), 0);
        m.record_tick_start(Duration::from_millis(90), 5);
        let (p50, p99) = m.lateness_percentiles();
        assert_eq!(p50, 400, "the middle of six");
        assert_eq!(p99, 90_000);
        assert_eq!(m.ticks_late_total.load(Ordering::Relaxed), 2, "two started past {LATE_TICK:?}");
        assert_eq!(m.ticks_dropped_total.load(Ordering::Relaxed), 5);
        assert_eq!(m.tick_percentiles(), (0, 0), "lateness is not the tick's own time");
    }

    #[test]
    fn render_is_prometheus_text_with_every_series() {
        let m = Metrics::new();
        m.record_tick(Duration::from_micros(1500));
        m.record_tick_start(Duration::from_micros(700), 0);
        m.record_snapshot_bytes(200);
        m.reconnects_total.fetch_add(2, Ordering::Relaxed);
        let text = m.render(RoomCounts { playing: 1, max_rooms: 25, seats_connected: 2, ..Default::default() }, true);
        for needle in [
            "bongbong_rooms{phase=\"playing\"} 1",
            "bongbong_rooms_max 25",
            "bongbong_seats{state=\"connected\"} 2",
            "bongbong_tick_microseconds{quantile=\"0.5\"} 1500",
            "bongbong_tick_microseconds{quantile=\"0.99\"} 1500",
            "bongbong_tick_lateness_microseconds{quantile=\"0.5\"} 700",
            "bongbong_ticks_late_total 0",
            "bongbong_ticks_dropped_total 0",
            "bongbong_snapshot_bytes_per_second ",
            "bongbong_snapshot_bytes_total 200",
            "bongbong_reconnects_total 2",
            "bongbong_draining 1",
            &format!("protocol=\"{}\"", bongbong::net::PROTOCOL_VERSION),
        ] {
            assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
        }
        assert!(text.lines().filter(|l| l.starts_with("# TYPE")).count() >= 14);
    }

    /// Rooms in play and rounds are labelled by map and mission, and the
    /// clients by the build they said they were.
    #[test]
    fn rounds_and_clients_are_labelled() {
        use bongbong::net::wire::ClientInfo;
        let m = Metrics::new();
        m.round_started("lotus-lagoon", "protect");
        m.round_started("lotus-lagoon", "protect");
        m.round_ended("lotus-lagoon", "protect", "won");
        let web = ClientLabel::of(Some(&ClientInfo { version: "0.2.4".into(), protocol: 10, platform: "web".into() }));
        let key = m.client_opened(&web);
        m.client_seated(&web);
        m.client_opened(&ClientLabel::of(None));
        let mut rooms = RoomCounts::default();
        rooms.playing_by.insert(("custom".into(), "hunt"), 2);
        let text = m.render(rooms, false);
        for needle in [
            "bongbong_rooms_playing{map=\"custom\",mission=\"hunt\"} 2",
            "bongbong_rounds_started_total{map=\"lotus-lagoon\",mission=\"protect\"} 2",
            "bongbong_rounds_ended_total{map=\"lotus-lagoon\",mission=\"protect\",outcome=\"won\"} 1",
            "bongbong_clients{version=\"0.2.4\",protocol=\"10\",platform=\"web\"} 1",
            "bongbong_clients{version=\"unknown\",protocol=\"unknown\",platform=\"unknown\"} 1",
            "bongbong_client_seats_total{version=\"0.2.4\",protocol=\"10\",platform=\"web\"} 1",
            &format!("game_version=\"{}\"", bongbong::net::BUILD_VERSION),
        ] {
            assert!(text.contains(needle), "missing {needle:?} in:\n{text}");
        }
        m.client_closed(&key);
        assert!(m.render(RoomCounts::default(), false).contains("bongbong_clients{version=\"0.2.4\",protocol=\"10\",platform=\"web\"} 0"));
    }

    /// What a client sends cannot mint series: a strange version reads
    /// `invalid`, a strange platform `other`, and past `CLIENT_LABELS_MAX`
    /// builds every new one counts as `other`.
    #[test]
    fn client_labels_are_bounded() {
        use bongbong::net::wire::ClientInfo;
        let odd = ClientLabel::of(Some(&ClientInfo { version: "1.0\"} x{".into(), protocol: 10, platform: "toaster".into() }));
        assert_eq!((odd.version.as_str(), odd.platform.as_str()), ("invalid", "other"));
        let m = Metrics::new();
        for i in 0..CLIENT_LABELS_MAX + 5 {
            let label = ClientLabel::of(Some(&ClientInfo { version: format!("0.0.{i}"), protocol: 10, platform: "web".into() }));
            m.client_opened(&label);
        }
        let text = m.render(RoomCounts::default(), false);
        let series = text.lines().filter(|l| l.starts_with("bongbong_clients{")).count();
        assert_eq!(series, CLIENT_LABELS_MAX + 1, "the cap plus other:\n{text}");
        assert!(text.contains("bongbong_clients{version=\"other\",protocol=\"other\",platform=\"other\"} 5"), "{text}");
    }
}
