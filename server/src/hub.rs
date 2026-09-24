//! The server's rooms (docs/online-coop-prd.md §4.9): a map from code to
//! the handle of the task that owns the room, the room cap, and the drain.
//! There is one of these per process and one process, so this map is where
//! every room in the world is.
//! Nothing here touches a `Game`; the hub only creates, finds and forgets
//! rooms.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicU8, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::{Notify, mpsc, watch};

use crate::code;
use crate::metrics::{Metrics, RoomCounts};
use crate::room::{self, Command, Phase, RoomParams};

/// Commands a room task takes before its mailbox pushes back on the
/// sender; the lobby is chatty at the scale of a few seats, not thousands.
pub const ROOM_COMMANDS: usize = 64;

/// How long a draining server waits for its last round before exiting
/// anyway (docs/online-coop-prd.md §4.8: the workload's grace period).
pub const DRAIN_MAX: std::time::Duration = std::time::Duration::from_secs(30 * 60);

/// What the hub keeps of a room: the way in and the counters it reports.
#[derive(Clone)]
pub struct RoomHandle {
    pub code: String,
    pub commands: mpsc::Sender<Command>,
    pub stats: Arc<RoomStats>,
}

/// A room's counters as the room task keeps them, read by `/metrics`.
#[derive(Default)]
pub struct RoomStats {
    /// `Phase as u8`.
    pub phase: AtomicU8,
    pub seats_connected: AtomicUsize,
    pub seats_away: AtomicUsize,
}

impl RoomStats {
    pub fn set_phase(&self, phase: Phase) {
        self.phase.store(phase as u8, Ordering::Relaxed);
    }

    pub fn phase(&self) -> Phase {
        Phase::from_u8(self.phase.load(Ordering::Relaxed))
    }
}

pub struct Hub {
    /// The most rooms this server holds at once.
    pub max_rooms: usize,
    pub metrics: Arc<Metrics>,
    rooms: Mutex<BTreeMap<String, RoomHandle>>,
    draining: AtomicBool,
    /// Signalled whenever a room goes, so `drained` can re-check.
    room_gone: Notify,
    /// Flipped once the drain is over: every connection task closes its
    /// socket so the HTTP server can finish.
    shutdown: watch::Sender<bool>,
    next_conn_id: AtomicU64,
}

impl Hub {
    pub fn new(max_rooms: usize, metrics: Arc<Metrics>) -> Arc<Hub> {
        Arc::new(Hub {
            max_rooms,
            metrics,
            rooms: Mutex::new(BTreeMap::new()),
            draining: AtomicBool::new(false),
            room_gone: Notify::new(),
            shutdown: watch::Sender::new(false),
            next_conn_id: AtomicU64::new(1),
        })
    }

    /// A fresh id for a connection, so a room can tell a stale
    /// disconnect from the socket that replaced it.
    pub fn next_conn_id(&self) -> u64 {
        self.next_conn_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Open a room and spawn its task; `Err` names why not (draining, or
    /// the server is full).
    pub fn create_room(self: &Arc<Self>, params: RoomParams) -> Result<RoomHandle, String> {
        if self.draining() {
            return Err("this server is draining; try again in a moment".into());
        }
        let mut rooms = self.rooms.lock().expect("rooms poisoned");
        if rooms.len() >= self.max_rooms {
            return Err(format!("this server is full ({} rooms)", self.max_rooms));
        }
        let mut rng = rand::rng();
        let code = loop {
            let candidate = code::mint(&mut rng);
            if !rooms.contains_key(&candidate) {
                break candidate;
            }
        };
        let (tx, rx) = mpsc::channel(ROOM_COMMANDS);
        let stats = Arc::new(RoomStats::default());
        let handle = RoomHandle { code: code.clone(), commands: tx, stats: stats.clone() };
        rooms.insert(code.clone(), handle.clone());
        drop(rooms);
        self.metrics.rooms_created_total.fetch_add(1, Ordering::Relaxed);
        tokio::spawn(room::run(Arc::clone(self), code, params, rx, stats));
        Ok(handle)
    }

    /// The room `code` names, once the code is a code at all.
    pub fn find(&self, code: &str) -> Result<RoomHandle, String> {
        let code = code::check(code).map_err(|e| e.to_string())?;
        self.rooms
            .lock()
            .expect("rooms poisoned")
            .get(&code)
            .cloned()
            .ok_or_else(|| format!("no room {code} here"))
    }

    /// The room task is done with `code`.
    pub fn remove(&self, code: &str) {
        self.rooms.lock().expect("rooms poisoned").remove(code);
        self.room_gone.notify_one();
    }

    pub fn room_count(&self) -> usize {
        self.rooms.lock().expect("rooms poisoned").len()
    }

    /// The counts `/metrics` reports.
    pub fn counts(&self) -> RoomCounts {
        let mut counts = RoomCounts::default();
        for handle in self.rooms.lock().expect("rooms poisoned").values() {
            match handle.stats.phase() {
                Phase::Waiting => counts.waiting += 1,
                Phase::Playing => counts.playing += 1,
                Phase::Paused => counts.paused += 1,
                Phase::Ended => counts.ended += 1,
            }
            counts.seats_connected += handle.stats.seats_connected.load(Ordering::Relaxed);
            counts.seats_away += handle.stats.seats_away.load(Ordering::Relaxed);
        }
        counts
    }

    /// Stop taking rooms and rematches; the rooms in progress keep ticking.
    pub fn begin_drain(&self) {
        self.draining.store(true, Ordering::Relaxed);
    }

    pub fn draining(&self) -> bool {
        self.draining.load(Ordering::Relaxed)
    }

    /// Resolves once no room is left (re-checked each time one goes).
    pub async fn drained(&self) {
        loop {
            if self.room_count() == 0 {
                return;
            }
            self.room_gone.notified().await;
        }
    }

    /// The drain is over: every connection closes.
    pub fn shutdown(&self) {
        self.shutdown.send_replace(true);
    }

    /// Resolves once `shutdown` was called.
    pub async fn shut_down(&self) {
        let mut rx = self.shutdown.subscribe();
        let _ = rx.wait_for(|down| *down).await;
    }
}
