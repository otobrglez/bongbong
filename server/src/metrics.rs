//! What `/metrics` reports (docs/online-coop-prd.md §4.9, §4.16): rooms
//! by phase and the cap on them, seats, tick time and tick lateness percentiles from rings of
//! recent ticks, snapshot bytes per second, reconnects. Counters are
//! atomics the room and connection tasks bump; the three windows sit
//! behind short mutexes.

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
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
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
}

pub struct Metrics {
    ticks: Mutex<TickRing>,
    /// How late each tick started against the room's schedule
    /// (`net::authority::TickClock`).
    lateness: Mutex<TickRing>,
    bytes: Mutex<BytesWindow>,
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
                    "{{version=\"{}\",protocol=\"{}\"}}",
                    env!("CARGO_PKG_VERSION"),
                    bongbong::net::PROTOCOL_VERSION
                ),
                1.0,
            )],
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
}
