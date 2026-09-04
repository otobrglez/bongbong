//! Local dev server: lets tooling (the `bbmcp` MCP adapter, `just
//! mcp-call`, a bare `nc`) drive the *windowed* game between frames -
//! step it in lockstep at the fixed timestep, read snapshots and events,
//! inject input, set up scenarios, take screenshots, tune knobs. Native
//! only, `--features dev-tools`; see docs/dev-server-design.md.
//!
//! Wire protocol: newline-delimited JSON on `127.0.0.1:<port>`. A request
//! is `{"id": <any>, "method": "<tool>", "params": {...}}`; the reply is
//! `{"id": <same>, "result": ...}` or `{"id": <same>, "error": "message"}`.
//! Methods are the entries of [`TOOLS`], which is also what the adapter
//! advertises to the MCP client, so the two can't drift.
//!
//! Threading follows `tuning.rs`'s staging pattern: socket threads only
//! queue [`Request`]s; the main loop drains them at the frame boundary
//! (`before_frame`), so every read and write happens while no `update`
//! is running and the round RNG sits in `Game::rng`.

use std::collections::VecDeque;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde::Serialize;
use serde_json::{Map, Value, json};
use sola_raylib::prelude::{RaylibHandle, RaylibTexture2D, RaylibThread, RenderTexture2D};

use crate::ai::Intent;
use crate::map::MapFile;
use crate::simulation::debug::{CLUSTER_RADIUS_PX, Detail, TankPatch, TrackRow};
use crate::simulation::{Event, Game, Input, Overlays};
use crate::tank::Dir;
use crate::tuning;
use crate::level::{Mission, SpawnKind, Tier};
use crate::{PHYSICS_FIXED_DT, Position, parse_seed};

/// Port the game listens on unless `--dev-port`/`BONGBONG_DEV_PORT` says
/// otherwise; the adapter defaults to the same.
pub const DEFAULT_PORT: u16 = 4747;

/// Longest a socket thread waits for the main loop to answer one request
/// (a long `step` still finishes in well under a second).
const REPLY_TIMEOUT: Duration = Duration::from_secs(120);
/// Idle connections drop after this long without a request.
const IDLE_TIMEOUT: Duration = Duration::from_secs(300);
/// Upper bound on `step`'s frame count per request.
const MAX_STEP_FRAMES: u64 = 100_000;
/// Events kept for the `events` tool (the AI-decision events are chatty:
/// a full round of enemies produces tens per second).
const EVENT_RING: usize = 4096;
/// Frames of per-tank history kept for the `history` tool (60 s).
const HISTORY_FRAMES: usize = 3600;
/// Rows one `history` reply returns at most.
const HISTORY_MAX_ROWS: usize = 2000;
/// Events returned inline by one `step` reply.
const STEP_EVENT_CAP: usize = 256;
/// Where screenshots land (under the gitignored `target/`).
const SHOT_DIR: &str = "target/devshots";

/// One tool: its wire/MCP name, the description the model reads, and its
/// input JSON schema (an `object` schema, as a string so this table can be
/// a `const`).
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: &'static str,
}

const NO_PARAMS: &str = r#"{"type":"object","properties":{}}"#;
const SLOT_PARAMS: &str = r#"{"type":"object","properties":{"slot":{"type":"integer","description":"Owner slot: 0 = player, enemies from 1 (see snapshot.tanks[].slot)"}},"required":["slot"]}"#;

/// Every tool the server answers, in the order the adapter lists them.
pub const TOOLS: &[ToolSpec] = &[
    ToolSpec {
        name: "status",
        description: "Where the running game is: seed, frame, time, outcome, paused/lockstep, tank count, overlay flags, the loaded map. Cheap; call first.",
        schema: NO_PARAMS,
    },
    ToolSpec {
        name: "snapshot",
        description: "World state as JSON: every tank (position, velocity, damage/hp, ammo, weapon, shield/boost, nearest_ally_px), projectiles, pickups, frog, `engage` (the engagement ring: per enemy its status - engaged/wreck/fleeing/retreating/out_of_range - the ring slot it holds and its target point; an engaged enemy with ring=null steers at the player directly, the pile-up case) and `clusters` (groups of live enemies within 90 px of each other). detail=full adds each enemy's AI memory (waypoint, committed heading, last behaviour-tree action, stuck timer, intent), the per-enemy slot rejection tally (claimed/off_map/unreachable/no_los) and the 16-slot table (point, line of sight, who holds it).",
        schema: r#"{"type":"object","properties":{"detail":{"type":"string","enum":["compact","full"],"default":"compact"}}}"#,
    },
    ToolSpec {
        name: "events",
        description: "Gameplay events recorded since `since` (a seq number; 0 = everything kept, up to 4096): fired, hit, wreck, ram, deflected (off a shield), shells_collided, frog_bite, pickup_collected, pickup_respawned, round_started, round_ended, plus AI decisions - ai_action (behaviour-tree action changed), engage_slot (ring slot changed; null = steering at the player), stuck_escape, breach (dir, or null when it ends), retreat (on/off), alert (shared last-known player position on/off). Each carries the frame it happened on. `kinds` keeps only those event names, `exclude` drops them.",
        schema: r#"{"type":"object","properties":{"since":{"type":"integer","default":0,"description":"Return events with seq > since"},"limit":{"type":"integer","default":200},"kinds":{"type":"array","items":{"type":"string"},"description":"Only these event names"},"exclude":{"type":"array","items":{"type":"string"},"description":"Drop these event names"}}}"#,
    },
    ToolSpec {
        name: "step",
        description: "Freeze the game in lockstep and advance exactly `frames` simulation frames at the fixed 1/60 s timestep (all in one rendered frame, so it is fast and deterministic). Optional player input is held for those frames; shells/plasma fire once per press, so set fire_every=N to tap the trigger every N frames instead of holding it. Replies with the events of the step (first 256; `kinds`/`exclude` filter by event name, see `events`) and, by default, a compact snapshot. Use `resume` to let the game run in real time again.",
        schema: r#"{"type":"object","properties":{"frames":{"type":"integer","default":1,"minimum":1,"maximum":100000},"move_dir":{"type":"string","enum":["up","down","left","right"]},"face":{"type":"string","enum":["up","down","left","right"]},"fire":{"type":"boolean"},"fire_every":{"type":"integer","minimum":1,"description":"With fire=true: press the trigger on frames 0, N, 2N... and release in between"},"snapshot":{"type":"boolean","default":true},"detail":{"type":"string","enum":["compact","full"],"default":"compact"},"kinds":{"type":"array","items":{"type":"string"}},"exclude":{"type":"array","items":{"type":"string"}}}}"#,
    },
    ToolSpec {
        name: "input",
        description: "Override the player's input for the next `frames` real-time frames (keyboard is ignored meanwhile). Works while the game runs; in lockstep prefer step's own input fields. `cycle_overlays: true` presses the I key once: cycles the overlay presets off -> inspect -> all -> off (with no move_dir/face/fire it leaves the keyboard alone).",
        schema: r#"{"type":"object","properties":{"move_dir":{"type":"string","enum":["up","down","left","right"]},"face":{"type":"string","enum":["up","down","left","right"]},"fire":{"type":"boolean"},"frames":{"type":"integer","default":1},"cycle_overlays":{"type":"boolean","default":false}}}"#,
    },
    ToolSpec {
        name: "pause",
        description: "Enter lockstep: the game stops advancing (no PAUSED overlay, so screenshots stay clean) until `step` or `resume`.",
        schema: NO_PARAMS,
    },
    ToolSpec {
        name: "resume",
        description: "Leave lockstep and clear the P-key pause: the game runs in real time with wall-clock dt again.",
        schema: NO_PARAMS,
    },
    ToolSpec {
        name: "restart",
        description: "Start a fresh round, frozen in lockstep (call `resume` to let it run in real time). Optional seed (number or 0x-hex string; pinned for later restarts too), enemy count, player chassis row (0-11), the map: `map` (a path to a TOML under maps/) or `map_toml` (the map's TOML text inline - see `map_get` for the format; the round keeps its current map when neither is given), and the level: `mission` (protect|hunt|destroy), `spawn` (band|waves) with `waves`/`wave_size`/`wave_growth`/`tier_start`/`tier_end` (light|medium|heavy|super) - each pinned for later restarts too, overriding the map's own [mission]/[spawn] tables. `intro: true` starts the round frozen behind the mission banner (off by default so `step` counts play frames). Same seed + same steps replays bit-for-bit.",
        schema: r#"{"type":"object","properties":{"seed":{"type":["integer","string"]},"enemies":{"type":"integer","minimum":1,"maximum":31},"tank_row":{"type":"integer","minimum":0,"maximum":11},"map":{"type":"string","description":"Path to a map .toml, relative to the game's working directory"},"map_toml":{"type":"string","description":"Map TOML text, e.g. `version = 1\ntanks = 4\ncells.\"20,8\" = { kind = \"wall\", material = \"iron\" }`"},"mission":{"type":"string","enum":["protect","hunt","destroy"]},"spawn":{"type":"string","enum":["band","waves"]},"waves":{"type":"integer","minimum":1},"wave_size":{"type":"integer","minimum":1},"wave_growth":{"type":"integer","minimum":0},"tier_start":{"type":"string","enum":["light","medium","heavy","super"]},"tier_end":{"type":"string","enum":["light","medium","heavy","super"]},"intro":{"type":"boolean"}}}"#,
    },
    ToolSpec {
        name: "map_get",
        description: "The current map as TOML text (plus name, cell count, default tank count) - edit it and hand it back through `restart {map_toml}`. Format: `version = 1`, optional `tanks = N` (default enemy count), and one `cells.\"col,row\"` entry per occupied 32 px grid cell (40 columns x 23 rows at 1280x720, col/row from 0 at the top-left): `{ kind = \"wall\", material = \"brick\"|\"iron\"|\"wood\"|\"glass\" }`, `{ kind = \"road\" }`, `{ kind = \"frog\" }` (one), `{ kind = \"start\" }` (the player, one), `{ kind = \"pickup\", pickup = \"health\"|\"ammo\"|\"laser\"|\"minigun\"|\"plasma\"|\"speedup\"|\"shield\" }`. Iron is indestructible, the rest can be shot away. Border walls and enemy spawns are added by the game on top.",
        schema: NO_PARAMS,
    },
    ToolSpec {
        name: "history",
        description: "Per-tank rows recorded every frame (last 60 s, cleared on restart): position, behaviour-tree action, ring slot, stuck, touching terrain. Replies with every N-th frame's rows (`every`) over the last `last` frames, optionally one `slot`, plus per-tank aggregates over the whole window: frames seen, distance travelled, net displacement, cluster_frames (2+ other live enemies within 90 px), stuck_frames, no_ring_frames (engaged without a slot), touching_frames. The live-game counterpart of the probe's per-round stats.",
        schema: r#"{"type":"object","properties":{"slot":{"type":"integer","description":"Only this tank's rows (aggregates still cover every tank)"},"last":{"type":"integer","default":600,"minimum":1,"maximum":3600,"description":"Window in frames, ending at the latest recorded one"},"every":{"type":"integer","default":10,"minimum":1,"description":"Row sampling stride in frames"}}}"#,
    },
    ToolSpec {
        name: "screenshot",
        description: "Capture the current frame (the state after the latest step) as a PNG: returned inline and saved under target/devshots/. scale 0.5 (default) halves it; use 1.0 to read overlay text. Optionally set overlay flags in the same call (same as the `overlays` tool). source=scene skips the HUD and overlays.",
        schema: r#"{"type":"object","properties":{"scale":{"type":"number","default":0.5,"minimum":0.1,"maximum":1},"source":{"type":"string","enum":["screen","scene"],"default":"screen"},"overlays":{"type":"object","properties":{"nav_grid":{"type":"boolean"},"ai":{"type":"boolean"},"projectiles":{"type":"boolean"},"engage":{"type":"boolean"},"pickups":{"type":"boolean"},"inspect":{"type":"boolean"}}}}}"#,
    },
    ToolSpec {
        name: "overlays",
        description: "Set persistent debug overlays drawn on top of the game (visible to the human too), one flag at a time: nav_grid (blocked pathfinding cells), ai (each enemy's waypoint, heading, last behaviour-tree action), projectiles (hit boxes + velocity), engage (engagement-ring targets), pickups (collect radius), inspect (tank hitboxes + stat readout). Omitted flags keep their value; replies with the current flags. The I key in the game window cycles presets instead (off -> inspect -> all); `input {cycle_overlays: true}` presses it.",
        schema: r#"{"type":"object","properties":{"nav_grid":{"type":"boolean"},"ai":{"type":"boolean"},"projectiles":{"type":"boolean"},"engage":{"type":"boolean"},"pickups":{"type":"boolean"},"inspect":{"type":"boolean"}}}"#,
    },
    ToolSpec {
        name: "nav_grid",
        description: "The AI's pathfinding grid as text (# blocked, . open) with tanks (P player, digits enemies, x wrecks), the frog (F) and pickups (*) marked - the cheapest way to reason about the layout.",
        schema: NO_PARAMS,
    },
    ToolSpec {
        name: "teleport",
        description: "Move a tank to (x, y) in screen pixels (velocity zeroed) and optionally snap its facing.",
        schema: r#"{"type":"object","properties":{"slot":{"type":"integer"},"x":{"type":"number"},"y":{"type":"number"},"facing":{"type":"string","enum":["up","down","left","right"]}},"required":["slot","x","y"]}"#,
    },
    ToolSpec {
        name: "set_tank",
        description: "Overwrite a tank's damage (0 = pristine, 100 = wreck), ammo counts (setting a special weapon's stock above 0 also arms it, like its pickup would), shield and speed-boost timers. Omitted fields are untouched.",
        schema: r#"{"type":"object","properties":{"slot":{"type":"integer"},"damage":{"type":"number"},"shells_ammo":{"type":"integer"},"minigun_ammo":{"type":"integer"},"plasma_ammo":{"type":"integer"},"laser_charges":{"type":"integer"},"shield_timer":{"type":"number"},"speed_boost_timer":{"type":"number"}},"required":["slot"]}"#,
    },
    ToolSpec {
        name: "kill",
        description: "Destroy a tank on the next simulated frame through the normal kill path (explosion, shockwave, wreck event, round end).",
        schema: SLOT_PARAMS,
    },
    ToolSpec {
        name: "spawn_enemy",
        description: "Add an enemy at (x, y) with an optional chassis row (0-11). Draws from the round RNG, so the round stops being the seeded replay afterwards. Returns the new slot.",
        schema: r#"{"type":"object","properties":{"x":{"type":"number"},"y":{"type":"number"},"row":{"type":"integer","minimum":0,"maximum":11}},"required":["x","y"]}"#,
    },
    ToolSpec {
        name: "tuning_get",
        description: "Current tuning knobs as {name: value}; diff_only=true returns just the knobs that differ from the compiled defaults.",
        schema: r#"{"type":"object","properties":{"diff_only":{"type":"boolean","default":false}}}"#,
    },
    ToolSpec {
        name: "tuning_set",
        description: "Apply a tuning patch {knob: value} (array knobs as name.label or a full array) at this frame boundary. Range-checked; the whole patch is rejected on any bad key.",
        schema: r#"{"type":"object","properties":{"patch":{"type":"object","additionalProperties":{"type":["number","boolean","array"]}}},"required":["patch"]}"#,
    },
    ToolSpec {
        name: "tuning_reset",
        description: "Restore every tuning knob to its compiled default.",
        schema: NO_PARAMS,
    },
    ToolSpec {
        name: "tuning_schema",
        description: "The knob table (name, group, type, doc, range, default, when it applies). ~185 rows, so filter by group and/or a name substring.",
        schema: r#"{"type":"object","properties":{"group":{"type":"string"},"name_contains":{"type":"string"}}}"#,
    },
];

/// One queued call from a socket thread; the main loop answers through
/// `reply` (dropping it without answering reads as "game loop gone").
pub struct Request {
    pub method: String,
    pub params: Value,
    pub reply: mpsc::Sender<Result<Value, String>>,
}

type Reply = mpsc::Sender<Result<Value, String>>;

/// An event with its position in the server's stream: `seq` never resets,
/// `frame` is the round frame it happened on.
#[derive(Clone, Serialize)]
struct EventRecord {
    seq: u64,
    frame: u64,
    /// The event's serialised `event` tag, for `EventFilter`.
    #[serde(skip)]
    kind: String,
    #[serde(flatten)]
    event: Event,
}

/// `kinds`/`exclude` from an `events` or `step` request; empty = keep all.
#[derive(Clone, Default)]
struct EventFilter {
    kinds: Vec<String>,
    exclude: Vec<String>,
}

impl EventFilter {
    fn keeps(&self, kind: &str) -> bool {
        (self.kinds.is_empty() || self.kinds.iter().any(|k| k == kind)) && !self.exclude.iter().any(|k| k == kind)
    }
}

/// One frame of the history ring: every live tank's `TrackRow`.
struct HistoryFrame {
    frame: u64,
    rows: Vec<TrackRow>,
}

/// Per-tank aggregates over a `history` window.
#[derive(Default, Serialize)]
struct TrackStats {
    frames: u32,
    /// Path length driven.
    distance: f32,
    /// Straight-line distance from the first to the last position.
    net: f32,
    /// Frames with at least two other live enemies within `CLUSTER_RADIUS_PX`.
    cluster_frames: u32,
    stuck_frames: u32,
    /// Frames an enemy held no ring slot.
    no_ring_frames: u32,
    touching_frames: u32,
    #[serde(skip)]
    first: Option<Position>,
    #[serde(skip)]
    last: Option<Position>,
}

struct PendingStep {
    remaining: u64,
    intent: Option<Intent>,
    /// Tap the trigger every this many frames instead of holding it.
    fire_every: Option<u64>,
    want_snapshot: bool,
    detail: Detail,
    filter: EventFilter,
    events: Vec<EventRecord>,
    restarted: bool,
    reply: Reply,
}

#[derive(Clone, Copy, PartialEq)]
enum ShotSource {
    Screen,
    Scene,
}

struct PendingShot {
    scale: f32,
    source: ShotSource,
    /// Set once a frame has been presented since arming: the framebuffer
    /// read-back lags one present, so the capture waits for the next.
    presented: bool,
    reply: Reply,
}

pub struct DevServer {
    rx: mpsc::Receiver<Request>,
    port: u16,
    /// While set, the main loop only advances the game through `step`.
    lockstep: bool,
    pending_step: Option<PendingStep>,
    /// Player intent to substitute for the keyboard, and frames left.
    injected: Option<(Intent, u32)>,
    /// An `input {cycle_overlays}` request waiting to press the I key on
    /// the next frame's input.
    cycle_overlays_pending: bool,
    pending_shot: Option<PendingShot>,
    events: VecDeque<EventRecord>,
    next_seq: u64,
    shot_seq: u64,
    /// One entry per simulated frame, oldest first - see `history`.
    history: VecDeque<HistoryFrame>,
}

impl DevServer {
    /// Bind `127.0.0.1:port` (0 picks a free one - see `port()`) and start
    /// accepting connections. Fails only on the bind, so a port already in
    /// use surfaces here and the caller can run without the server.
    pub fn start(port: u16) -> std::io::Result<DevServer> {
        let listener = TcpListener::bind(SocketAddr::from(([127, 0, 0, 1], port)))?;
        let port = listener.local_addr()?.port();
        let (tx, rx) = mpsc::channel();
        thread::Builder::new().name("devserver-accept".into()).spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let tx = tx.clone();
                let _ = thread::Builder::new()
                    .name("devserver-conn".into())
                    .spawn(move || serve_connection(stream, tx));
            }
        })?;
        Ok(Self::with_receiver(rx, port))
    }

    /// A server with no socket: requests arrive through the returned
    /// sender. For tests.
    pub fn headless() -> (DevServer, mpsc::Sender<Request>) {
        let (tx, rx) = mpsc::channel();
        (Self::with_receiver(rx, 0), tx)
    }

    fn with_receiver(rx: mpsc::Receiver<Request>, port: u16) -> DevServer {
        DevServer {
            rx,
            port,
            lockstep: false,
            pending_step: None,
            injected: None,
            cycle_overlays_pending: false,
            pending_shot: None,
            events: VecDeque::with_capacity(EVENT_RING),
            next_seq: 1,
            shot_seq: 0,
            history: VecDeque::with_capacity(HISTORY_FRAMES),
        }
    }

    /// The port actually bound (differs from the request only for 0).
    pub fn port(&self) -> u16 {
        self.port
    }

    pub fn lockstep(&self) -> bool {
        self.lockstep
    }

    /// Frame boundary, before input is read: answer every queued request
    /// that can be answered now and arm `step`/`screenshot` for later in
    /// this frame. `width`/`height` are the battlefield size.
    pub fn before_frame(&mut self, game: &mut Game, width: f32, height: f32) {
        // The AI-decision events exist for this server's `events` feed.
        game.trace_ai = true;
        while let Ok(req) = self.rx.try_recv() {
            self.dispatch(game, req, width, height);
        }
    }

    /// Substitute injected player intent for the keyboard's, if any is
    /// pending (counts that override down by one frame), and press the I
    /// key for this one frame when an `input {cycle_overlays}` is waiting.
    pub fn shape_input(&mut self, real: Input) -> Input {
        let mut input = real;
        if let Some((intent, left)) = self.injected {
            self.injected = (left > 1).then_some((intent, left - 1));
            input.player_intent = intent;
        }
        if std::mem::take(&mut self.cycle_overlays_pending) {
            input.cycle_overlays_pressed = true;
        }
        input
    }

    /// Advance the game for this rendered frame: a pending `step` runs its
    /// frames back-to-back at `PHYSICS_FIXED_DT` and replies; otherwise one
    /// real-time update unless lockstep holds the game still.
    pub fn advance(&mut self, game: &mut Game, input: Input, real_dt: f32, width: f32, height: f32) {
        if let Some(mut step) = self.pending_step.take() {
            for i in 0..step.remaining {
                let mut player_intent = step.intent.unwrap_or(input.player_intent);
                if let Some(n) = step.fire_every {
                    player_intent.fire = player_intent.fire && i % n == 0;
                }
                let before = game.frame();
                game.update(Input { player_intent, ..Input::default() }, PHYSICS_FIXED_DT, width, height);
                if game.frame() != before + 1 {
                    step.restarted = true;
                }
                let mut sink = std::mem::take(&mut step.events);
                self.drain_events(game, Some((&mut sink, &step.filter)));
                step.events = sink;
                self.record_history(game);
            }
            let snapshot = step.want_snapshot.then(|| to_value(game.debug_snapshot(width, height, step.detail)));
            let _ = step.reply.send(Ok(json!({
                "frame": game.frame(),
                "time": game.debug_snapshot(width, height, Detail::Compact).time,
                "outcome": game.outcome(),
                "restarted": step.restarted,
                "lockstep": true,
                "events": step.events,
                "snapshot": snapshot,
            })));
        } else if !self.lockstep {
            game.update(input, real_dt, width, height);
            self.drain_events(game, None);
            self.record_history(game);
        }
    }

    /// Append this frame's `TrackRow`s to the history ring. A frame number
    /// that doesn't follow the last one means the round restarted, so the
    /// ring starts over.
    fn record_history(&mut self, game: &Game) {
        if self.history.back().is_some_and(|last| game.frame() <= last.frame) {
            self.history.clear();
        }
        if self.history.len() == HISTORY_FRAMES {
            self.history.pop_front();
        }
        self.history.push_back(HistoryFrame { frame: game.frame(), rows: game.debug_track_rows() });
    }

    /// The `history` reply: sampled rows plus per-tank aggregates over the
    /// last `last` frames.
    fn history_json(&self, last: usize, every: usize, slot: Option<usize>) -> Value {
        let skip = self.history.len().saturating_sub(last);
        let window: Vec<&HistoryFrame> = self.history.iter().skip(skip).collect();
        let mut stats: std::collections::BTreeMap<usize, TrackStats> = std::collections::BTreeMap::new();
        let mut rows = Vec::new();
        for (i, hf) in window.iter().enumerate() {
            let sample = i % every == 0 && rows.len() < HISTORY_MAX_ROWS;
            for row in &hf.rows {
                let pos = Position::new(row.x, row.y);
                let others_near = hf
                    .rows
                    .iter()
                    .filter(|o| o.slot != row.slot && o.slot != 0 && Position::new(o.x, o.y).distance_to(pos) <= CLUSTER_RADIUS_PX)
                    .count();
                let st = stats.entry(row.slot).or_default();
                st.frames += 1;
                if let Some(prev) = st.last {
                    st.distance += prev.distance_to(pos);
                }
                st.first.get_or_insert(pos);
                st.last = Some(pos);
                st.cluster_frames += u32::from(row.slot != 0 && others_near >= 2);
                st.stuck_frames += u32::from(row.stuck);
                st.no_ring_frames += u32::from(row.slot != 0 && row.ring.is_none());
                st.touching_frames += u32::from(row.touching_static);
                if sample && slot.is_none_or(|s| s == row.slot) {
                    let mut v = to_value(row);
                    v["frame"] = json!(hf.frame);
                    rows.push(v);
                }
            }
        }
        for st in stats.values_mut() {
            st.net = match (st.first, st.last) {
                (Some(a), Some(b)) => a.distance_to(b),
                _ => 0.0,
            };
            st.distance = (st.distance * 10.0).round() / 10.0;
            st.net = (st.net * 10.0).round() / 10.0;
        }
        json!({
            "from": window.first().map(|f| f.frame),
            "to": window.last().map(|f| f.frame),
            "every": every,
            "rows": rows,
            "tanks": stats,
        })
    }

    /// After `Game::render`: take the pending screenshot and reply with it.
    /// Reading the screen returns the frame presented *before* this one
    /// (raylib reads back after the buffer swap), so a shot armed this
    /// frame is captured on the next - in lockstep that frame is identical
    /// and carries any overlay flags set alongside the request.
    pub fn after_render(&mut self, rl: &mut RaylibHandle, thread: &RaylibThread, scene: &RenderTexture2D, game: &Game) {
        match self.pending_shot.as_mut() {
            None => return,
            Some(shot) if !shot.presented => {
                shot.presented = true;
                return;
            }
            Some(_) => {}
        }
        let shot = self.pending_shot.take().expect("checked above");
        let result = self.capture(rl, thread, scene, game, shot.scale, shot.source);
        let _ = shot.reply.send(result);
    }

    fn capture(
        &mut self,
        rl: &mut RaylibHandle,
        thread: &RaylibThread,
        scene: &RenderTexture2D,
        game: &Game,
        scale: f32,
        source: ShotSource,
    ) -> Result<Value, String> {
        let mut image = match source {
            ShotSource::Screen => rl.load_image_from_screen(thread),
            ShotSource::Scene => {
                // A render texture reads back bottom-up.
                let mut image = scene.load_image().map_err(|e| e.to_string())?;
                image.flip_vertical();
                image
            }
        };
        if (scale - 1.0).abs() > 1e-3 {
            let w = ((image.width() as f32 * scale).round() as i32).max(1);
            let h = ((image.height() as f32 * scale).round() as i32).max(1);
            image.resize_nn(w, h);
        }
        let png = image.export_image_to_memory(".png").map_err(|e| e.to_string())?;
        self.shot_seq += 1;
        let dir = Path::new(SHOT_DIR);
        fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let path = dir.join(format!("{:05}-f{}.png", self.shot_seq, game.frame()));
        fs::write(&path, &png).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(json!({
            "frame": game.frame(),
            "path": path.display().to_string(),
            "width": image.width(),
            "height": image.height(),
            "bytes": png.len(),
            "png_base64": base64_encode(&png),
        }))
    }

    /// Move the game's per-frame events into the ring (and `sink`, if
    /// given). Call exactly once per `update`/`init`, never otherwise -
    /// `Game::events` holds the last frame's events until the next.
    fn drain_events(&mut self, game: &Game, mut sink: Option<(&mut Vec<EventRecord>, &EventFilter)>) {
        for event in game.events() {
            let kind = to_value(event).get("event").and_then(Value::as_str).unwrap_or("?").to_string();
            let record = EventRecord { seq: self.next_seq, frame: game.frame(), kind, event: event.clone() };
            self.next_seq += 1;
            if self.events.len() == EVENT_RING {
                self.events.pop_front();
            }
            if let Some((sink, filter)) = sink.as_mut()
                && sink.len() < STEP_EVENT_CAP
                && filter.keeps(&record.kind)
            {
                sink.push(record.clone());
            }
            self.events.push_back(record);
        }
    }

    fn status(&self, game: &Game, width: f32, height: f32) -> Value {
        let snap = game.debug_snapshot(width, height, Detail::Compact);
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "port": self.port,
            "seed": snap.seed,
            "frame": snap.frame,
            "time": snap.time,
            "outcome": snap.outcome,
            "mission": game.mission.name(),
            "spawn": game.spawn_plan,
            "intro_seconds_left": game.intro_timer,
            "paused": snap.paused,
            "lockstep": self.lockstep,
            "step_pending": self.pending_step.is_some(),
            "tanks": snap.tanks.len(),
            "enemies_alive": snap.tanks.iter().filter(|t| !t.is_player && !t.wreck).count(),
            "width": width,
            "height": height,
            "overlays": overlays_json(game),
            "map": map_json(&game.map),
            "events_kept": self.events.len(),
            "next_event_seq": self.next_seq,
            "history_frames": self.history.len(),
        })
    }

    fn dispatch(&mut self, game: &mut Game, req: Request, width: f32, height: f32) {
        let Request { method, params, reply } = req;
        let result = match method.as_str() {
            "status" => Ok(self.status(game, width, height)),
            "snapshot" => detail_param(&params).map(|d| to_value(game.debug_snapshot(width, height, d))),
            "events" => event_filter(&params).and_then(|filter| {
                let since = params.get("since").and_then(Value::as_u64).unwrap_or(0);
                let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(200) as usize;
                let events: Vec<&EventRecord> =
                    self.events.iter().filter(|e| e.seq > since && filter.keeps(&e.kind)).take(limit).collect();
                let next = events.last().map_or(since, |e| e.seq);
                Ok(json!({ "next": next, "events": events }))
            }),
            "step" => {
                if self.pending_step.is_some() {
                    Err("a step is already in progress".to_string())
                } else {
                    match (frames_param(&params, 1), parse_intent(&params), detail_param(&params), event_filter(&params)) {
                        (Ok(remaining), Ok(intent), Ok(detail), Ok(filter)) => {
                            self.lockstep = true;
                            self.pending_step = Some(PendingStep {
                                remaining,
                                intent,
                                fire_every: params.get("fire_every").and_then(Value::as_u64).filter(|&n| n >= 1),
                                want_snapshot: params.get("snapshot").and_then(Value::as_bool).unwrap_or(true),
                                detail,
                                filter,
                                events: Vec::new(),
                                restarted: false,
                                reply,
                            });
                            return;
                        }
                        (Err(e), _, _, _) | (_, Err(e), _, _) | (_, _, Err(e), _) | (_, _, _, Err(e)) => Err(e),
                    }
                }
            }
            "input" => match (parse_intent(&params), frames_param(&params, 1)) {
                (Ok(intent), Ok(frames)) => {
                    if let Some(intent) = intent {
                        self.injected = Some((intent, frames.min(u32::MAX as u64) as u32));
                    }
                    self.cycle_overlays_pending = params.get("cycle_overlays").and_then(Value::as_bool).unwrap_or(false);
                    Ok(json!({ "frames": frames, "cycle_overlays": self.cycle_overlays_pending }))
                }
                (Err(e), _) | (_, Err(e)) => Err(e),
            },
            "pause" => {
                self.lockstep = true;
                Ok(self.status(game, width, height))
            }
            "resume" => {
                self.lockstep = false;
                game.paused = false;
                Ok(self.status(game, width, height))
            }
            "restart" => self.restart(game, &params, width, height),
            "screenshot" => {
                if self.pending_shot.is_some() {
                    Err("a screenshot is already pending".to_string())
                } else {
                    let scale = params.get("scale").and_then(Value::as_f64).unwrap_or(0.5) as f32;
                    let source = match params.get("source").and_then(Value::as_str).unwrap_or("screen") {
                        "screen" => Ok(ShotSource::Screen),
                        "scene" => Ok(ShotSource::Scene),
                        other => Err(format!("unknown source {other:?} (screen|scene)")),
                    };
                    match source {
                        Ok(source) => {
                            if let Some(flags) = params.get("overlays") {
                                apply_overlays(game, flags);
                            }
                            self.pending_shot = Some(PendingShot { scale: scale.clamp(0.1, 1.0), source, presented: false, reply });
                            return;
                        }
                        Err(e) => Err(e),
                    }
                }
            }
            "overlays" => {
                apply_overlays(game, &params);
                Ok(overlays_json(game))
            }
            "nav_grid" => Ok(json!({ "grid": game.nav_grid_ascii(width, height) })),
            "map_get" => game.map.to_toml_string().map(|toml| {
                let mut v = map_json(&game.map);
                v["toml"] = Value::String(toml);
                v
            }),
            "history" => {
                let last = params.get("last").and_then(Value::as_u64).unwrap_or(600).clamp(1, HISTORY_FRAMES as u64) as usize;
                let every = params.get("every").and_then(Value::as_u64).unwrap_or(10).max(1) as usize;
                let slot = params.get("slot").and_then(Value::as_u64).map(|s| s as usize);
                Ok(self.history_json(last, every, slot))
            }
            "teleport" => match (slot_param(&params), f32_param(&params, "x"), f32_param(&params, "y")) {
                (Ok(slot), Some(x), Some(y)) => {
                    let facing = match params.get("facing").and_then(Value::as_str) {
                        Some(s) => Dir::parse(s).map(|d| Some(d.rotation())).ok_or_else(|| format!("unknown facing {s:?}")),
                        None => Ok(None),
                    };
                    facing
                        .and_then(|rotation| game.debug_teleport(slot, Position::new(x, y), rotation))
                        .map(|()| json!({ "slot": slot, "x": x, "y": y }))
                }
                (Err(e), _, _) => Err(e),
                _ => Err("x and y are required".to_string()),
            },
            "set_tank" => slot_param(&params).and_then(|slot| {
                let patch: TankPatch = serde_json::from_value(params.clone()).map_err(|e| e.to_string())?;
                game.debug_set_tank(slot, &patch)?;
                let snap = game.debug_snapshot(width, height, Detail::Compact);
                Ok(to_value(snap.tanks.into_iter().find(|t| t.slot == slot)))
            }),
            "kill" => slot_param(&params).and_then(|slot| game.debug_kill(slot).map(|()| json!({ "slot": slot, "applied_on_next_frame": true }))),
            "spawn_enemy" => match (f32_param(&params, "x"), f32_param(&params, "y")) {
                (Some(x), Some(y)) => {
                    let row = params.get("row").and_then(Value::as_i64).map(|r| r as i32);
                    game.debug_spawn_enemy(Position::new(x, y), row).map(|slot| json!({ "slot": slot }))
                }
                _ => Err("x and y are required".to_string()),
            },
            "tuning_get" => {
                let diff_only = params.get("diff_only").and_then(Value::as_bool).unwrap_or(false);
                let text = if diff_only { tuning::diff_json() } else { tuning::current_json() };
                serde_json::from_str(&text).map_err(|e| e.to_string())
            }
            "tuning_set" => match params.get("patch") {
                Some(patch) if patch.is_object() => tuning::submit_json(&patch.to_string()).map(|n| json!({ "staged": n })),
                _ => Err("params.patch must be an object of {knob: value}".to_string()),
            },
            "tuning_reset" => {
                tuning::submit_reset();
                Ok(json!({ "reset": true }))
            }
            "tuning_schema" => {
                let group = params.get("group").and_then(Value::as_str);
                let needle = params.get("name_contains").and_then(Value::as_str);
                serde_json::from_str::<Vec<Value>>(&tuning::schema_json())
                    .map_err(|e| e.to_string())
                    .map(|rows| {
                        let rows: Vec<Value> = rows
                            .into_iter()
                            .filter(|r| group.is_none_or(|g| r.get("group").and_then(Value::as_str) == Some(g)))
                            .filter(|r| {
                                needle.is_none_or(|n| r.get("name").and_then(Value::as_str).is_some_and(|name| name.contains(n)))
                            })
                            .collect();
                        json!({ "count": rows.len(), "rows": rows })
                    })
            }
            other => Err(format!("unknown method {other:?}; see TOOLS")),
        };
        let _ = reply.send(result);
    }

    fn restart(&mut self, game: &mut Game, params: &Value, width: f32, height: f32) -> Result<Value, String> {
        match params.get("seed") {
            None | Some(Value::Null) => {}
            Some(Value::Number(n)) => game.seed_override = Some(n.as_u64().ok_or("seed must be a non-negative integer")?),
            Some(Value::String(s)) => game.seed_override = Some(parse_seed(s)?),
            Some(other) => return Err(format!("seed must be a number or string, got {other}")),
        }
        if let Some(n) = params.get("enemies") {
            game.enemy_count_override = Some(n.as_u64().ok_or("enemies must be an integer")? as usize);
        }
        if let Some(row) = params.get("tank_row") {
            game.player_row_override = Some(row.as_i64().ok_or("tank_row must be an integer")? as i32);
        }
        let enum_param = |key: &str| -> Result<Option<String>, String> {
            match params.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(Value::String(s)) => Ok(Some(s.clone())),
                Some(other) => Err(format!("{key} must be a string, got {other}")),
            }
        };
        let u32_param = |key: &str| -> Result<Option<u32>, String> {
            match params.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(v) => v.as_u64().map(|n| Some(n as u32)).ok_or_else(|| format!("{key} must be a non-negative integer")),
            }
        };
        use clap::ValueEnum;
        if let Some(m) = enum_param("mission")? {
            game.level_overrides.mission = Some(Mission::from_str(&m, true).map_err(|e| format!("mission: {e}"))?);
        }
        if let Some(k) = enum_param("spawn")? {
            game.level_overrides.spawn = Some(SpawnKind::from_str(&k, true).map_err(|e| format!("spawn: {e}"))?);
        }
        if let Some(t) = enum_param("tier_start")? {
            game.level_overrides.tier_start = Some(Tier::from_str(&t, true).map_err(|e| format!("tier_start: {e}"))?);
        }
        if let Some(t) = enum_param("tier_end")? {
            game.level_overrides.tier_end = Some(Tier::from_str(&t, true).map_err(|e| format!("tier_end: {e}"))?);
        }
        if let Some(n) = u32_param("waves")? {
            game.level_overrides.waves = Some(n);
        }
        if let Some(n) = u32_param("wave_size")? {
            game.level_overrides.wave_size = Some(n);
        }
        if let Some(n) = u32_param("wave_growth")? {
            game.level_overrides.wave_growth = Some(n);
        }
        // Headless default: no frozen intro, so `step` counts are play
        // frames. Ask for it to screenshot the banner.
        game.show_intro = params.get("intro").and_then(Value::as_bool).unwrap_or(false);
        let map_path = params.get("map").filter(|v| !v.is_null());
        let map_toml = params.get("map_toml").filter(|v| !v.is_null());
        match (map_path, map_toml) {
            (Some(_), Some(_)) => return Err("give either map (a path) or map_toml (inline TOML), not both".to_string()),
            (Some(path), None) => {
                let path = path.as_str().ok_or("map must be a path string")?;
                game.map = MapFile::load(Path::new(path))?;
            }
            (None, Some(text)) => {
                let text = text.as_str().ok_or("map_toml must be a string of map TOML")?;
                game.map = MapFile::from_toml_str(text).map_err(|e| format!("map_toml: {e}"))?;
            }
            (None, None) => {}
        }
        game.init(width, height);
        self.drain_events(game, None);
        self.record_history(game);
        // A restart is the start of a repro: hold the new round still so no
        // wall-clock frames slip in before the first `step`.
        self.lockstep = true;
        Ok(self.status(game, width, height))
    }
}

/// One connection: a request line in, a reply line out, until the peer
/// hangs up, idles past `IDLE_TIMEOUT`, or the main loop is gone.
fn serve_connection(stream: TcpStream, tx: mpsc::Sender<Request>) {
    let _ = stream.set_read_timeout(Some(IDLE_TIMEOUT));
    let Ok(mut writer) = stream.try_clone() else { return };
    let reader = BufReader::new(stream);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let (id, outcome) = match serde_json::from_str::<Value>(&line) {
            Err(e) => (Value::Null, Err(format!("bad request JSON: {e}"))),
            Ok(msg) => {
                let id = msg.get("id").cloned().unwrap_or(Value::Null);
                match msg.get("method").and_then(Value::as_str) {
                    None => (id, Err("request needs a \"method\"".to_string())),
                    Some(method) => {
                        let params = msg.get("params").cloned().unwrap_or_else(|| Value::Object(Map::new()));
                        let (reply, rx) = mpsc::channel();
                        let req = Request { method: method.to_string(), params, reply };
                        if tx.send(req).is_err() {
                            (id, Err("game loop is gone".to_string()))
                        } else {
                            match rx.recv_timeout(REPLY_TIMEOUT) {
                                Ok(result) => (id, result),
                                Err(mpsc::RecvTimeoutError::Timeout) => (id, Err("timed out waiting for the game loop".to_string())),
                                Err(mpsc::RecvTimeoutError::Disconnected) => (id, Err("game loop dropped the request".to_string())),
                            }
                        }
                    }
                }
            }
        };
        let frame = match outcome {
            Ok(result) => json!({ "id": id, "result": result }),
            Err(error) => json!({ "id": id, "error": error }),
        };
        if writeln!(writer, "{frame}").and_then(|()| writer.flush()).is_err() {
            break;
        }
    }
}

fn to_value<T: Serialize>(v: T) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// The loaded map's identity for `status`/`map_get`.
fn map_json(map: &MapFile) -> Value {
    json!({
        "name": map.name.as_deref().unwrap_or("inline"),
        "cells": map.cells.len(),
        "tanks": map.tanks,
    })
}

/// `kinds`/`exclude` from `params`: string arrays, both optional.
fn event_filter(params: &Value) -> Result<EventFilter, String> {
    let list = |key: &str| -> Result<Vec<String>, String> {
        match params.get(key) {
            None | Some(Value::Null) => Ok(Vec::new()),
            Some(Value::Array(items)) => items
                .iter()
                .map(|v| v.as_str().map(str::to_string).ok_or_else(|| format!("{key} must be an array of event names")))
                .collect(),
            Some(other) => Err(format!("{key} must be an array of event names, got {other}")),
        }
    };
    Ok(EventFilter { kinds: list("kinds")?, exclude: list("exclude")? })
}

fn overlays_json(game: &Game) -> Value {
    to_value(game.debug_overlays)
}

/// Set only the overlay flags present in `flags`.
fn apply_overlays(game: &mut Game, flags: &Value) {
    let flag = |name: &str| flags.get(name).and_then(Value::as_bool);
    let o: &mut Overlays = &mut game.debug_overlays;
    if let Some(b) = flag("nav_grid") {
        o.nav_grid = b;
    }
    if let Some(b) = flag("ai") {
        o.ai = b;
    }
    if let Some(b) = flag("projectiles") {
        o.projectiles = b;
    }
    if let Some(b) = flag("engage") {
        o.engage = b;
    }
    if let Some(b) = flag("pickups") {
        o.pickups = b;
    }
    if let Some(b) = flag("inspect") {
        o.inspect = b;
    }
}

fn detail_param(params: &Value) -> Result<Detail, String> {
    match params.get("detail") {
        None | Some(Value::Null) => Ok(Detail::Compact),
        Some(v) => serde_json::from_value(v.clone()).map_err(|_| format!("detail must be compact|full, got {v}")),
    }
}

fn frames_param(params: &Value, default: u64) -> Result<u64, String> {
    match params.get("frames") {
        None | Some(Value::Null) => Ok(default),
        Some(v) => match v.as_u64() {
            Some(n) if (1..=MAX_STEP_FRAMES).contains(&n) => Ok(n),
            _ => Err(format!("frames must be 1..={MAX_STEP_FRAMES}, got {v}")),
        },
    }
}

fn slot_param(params: &Value) -> Result<usize, String> {
    params
        .get("slot")
        .and_then(Value::as_u64)
        .map(|s| s as usize)
        .ok_or_else(|| "slot (0 = player, enemies from 1) is required".to_string())
}

fn f32_param(params: &Value, key: &str) -> Option<f32> {
    params.get(key).and_then(Value::as_f64).map(|v| v as f32)
}

/// `move_dir`/`face`/`fire` from `params`: `None` when none is given (the
/// keyboard stays in charge), an error for an unknown direction.
fn parse_intent(params: &Value) -> Result<Option<Intent>, String> {
    let dir = |key: &str| -> Result<Option<Dir>, String> {
        match params.get(key).and_then(Value::as_str) {
            None => Ok(None),
            Some(s) => Dir::parse(s).map(Some).ok_or_else(|| format!("{key} must be up|down|left|right, got {s:?}")),
        }
    };
    let move_dir = dir("move_dir")?;
    let face = dir("face")?;
    let fire = params.get("fire").and_then(Value::as_bool);
    if move_dir.is_none() && face.is_none() && fire.is_none() {
        return Ok(None);
    }
    Ok(Some(Intent { move_dir, face, fire: fire.unwrap_or(false), fire_aim_offset: 0.0 }))
}

/// Standard base64 (RFC 4648, padded) - the one encoder this crate needs,
/// so no dependency for it.
pub fn base64_encode(bytes: &[u8]) -> String {
    const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        out.push(TABLE[(n >> 18) as usize & 63] as char);
        out.push(TABLE[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { TABLE[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { TABLE[n as usize & 63] as char } else { '=' });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::TankSnapshot;
    use std::io::BufRead;

    const W: f32 = 1280.0;
    const H: f32 = 720.0;

    fn game(seed: u64) -> Game {
        let mut game = Game::default();
        game.enemy_count_override = Some(4);
        game.seed_override = Some(seed);
        game.map = MapFile::from_toml_str(include_str!("../maps/default.toml")).expect("embedded default map parses");
        game.init(W, H);
        game
    }

    /// Queue `method` on a headless server and return its reply receiver.
    fn call(tx: &mpsc::Sender<Request>, method: &str, params: Value) -> mpsc::Receiver<Result<Value, String>> {
        let (reply, rx) = mpsc::channel();
        tx.send(Request { method: method.into(), params, reply }).unwrap();
        rx
    }

    fn key(s: &TankSnapshot) -> (u32, u32, u32, u32, u32, i32, bool) {
        (
            s.position.x.to_bits(),
            s.position.y.to_bits(),
            s.velocity.x.to_bits(),
            s.velocity.y.to_bits(),
            s.damage.to_bits(),
            s.shells_ammo,
            s.is_wreck,
        )
    }

    #[test]
    fn every_tool_has_an_object_schema_and_a_unique_name() {
        let mut names = std::collections::HashSet::new();
        for tool in TOOLS {
            assert!(names.insert(tool.name), "duplicate tool {}", tool.name);
            assert!(tool.name.chars().all(|c| c.is_ascii_lowercase() || c == '_'), "{}", tool.name);
            let schema: Value = serde_json::from_str(tool.schema).unwrap_or_else(|e| panic!("{}: {e}", tool.name));
            assert_eq!(schema.get("type").and_then(Value::as_str), Some("object"), "{}", tool.name);
            assert!(!tool.description.is_empty());
        }
    }

    #[test]
    fn base64_matches_rfc_4648_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
    }

    #[test]
    fn status_reports_a_fresh_round() {
        let (mut server, tx) = DevServer::headless();
        let mut game = game(11);
        let rx = call(&tx, "status", json!({}));
        server.before_frame(&mut game, W, H);
        let status = rx.recv().unwrap().unwrap();
        assert_eq!(status["frame"], 0);
        assert_eq!(status["tanks"], 5);
        assert_eq!(status["outcome"], "playing");
        assert_eq!(status["lockstep"], false);
    }

    #[test]
    fn step_matches_manual_fixed_dt_updates_bit_for_bit() {
        let (mut server, tx) = DevServer::headless();
        let mut stepped = game(0xB0B5);
        let mut manual = game(0xB0B5);
        let held = json!({ "frames": 90, "move_dir": "up", "fire": true, "snapshot": false });
        let rx = call(&tx, "step", held);
        server.before_frame(&mut stepped, W, H);
        assert!(server.lockstep(), "step enters lockstep");
        server.advance(&mut stepped, Input::default(), 0.123, W, H);
        let reply = rx.recv().unwrap().unwrap();
        assert_eq!(reply["frame"], 90);
        assert_eq!(reply["restarted"], false);
        assert!(reply["events"].as_array().unwrap().iter().any(|e| e["event"] == "fired"), "{}", reply["events"]);

        let intent = Intent { move_dir: Some(Dir::Up), fire: true, ..Intent::default() };
        for _ in 0..90 {
            manual.update(Input { player_intent: intent, ..Input::default() }, PHYSICS_FIXED_DT, W, H);
        }
        let (a, b) = (stepped.tank_snapshots(), manual.tank_snapshots());
        assert_eq!(a.len(), b.len());
        for (i, (x, y)) in a.iter().zip(&b).enumerate() {
            assert_eq!(key(x), key(y), "tank {i} diverged");
        }
        // Lockstep holds the game still until the next step.
        server.advance(&mut stepped, Input::default(), 0.016, W, H);
        assert_eq!(stepped.frame(), 90);
    }

    #[test]
    fn fire_every_taps_the_trigger_instead_of_holding_it() {
        let (mut server, tx) = DevServer::headless();
        let mut game = game(4);
        let rx = call(&tx, "step", json!({ "frames": 120, "fire": true, "fire_every": 40, "snapshot": false }));
        server.before_frame(&mut game, W, H);
        server.advance(&mut game, Input::default(), 0.016, W, H);
        let reply = rx.recv().unwrap().unwrap();
        let shots = reply["events"].as_array().unwrap().iter().filter(|e| e["event"] == "fired" && e["slot"] == 0).count();
        assert_eq!(shots, 3, "{}", reply["events"]);
    }

    #[test]
    fn teleport_moves_the_tank_and_physics_agrees() {
        let (mut server, tx) = DevServer::headless();
        let mut game = game(3);
        let rx = call(&tx, "teleport", json!({ "slot": 0, "x": 400.0, "y": 300.0, "facing": "left" }));
        server.before_frame(&mut game, W, H);
        rx.recv().unwrap().unwrap();
        let rx = call(&tx, "step", json!({ "frames": 1 }));
        server.before_frame(&mut game, W, H);
        server.advance(&mut game, Input::default(), 0.016, W, H);
        let reply = rx.recv().unwrap().unwrap();
        let player = &reply["snapshot"]["tanks"][0];
        assert_eq!(player["slot"], 0);
        assert!((player["x"].as_f64().unwrap() - 400.0).abs() < 1.0, "{player}");
        assert!((player["y"].as_f64().unwrap() - 300.0).abs() < 1.0, "{player}");
        assert_eq!(player["rotation"], 270.0);
    }

    #[test]
    fn kill_runs_through_the_explosion_path() {
        let (mut server, tx) = DevServer::headless();
        let mut game = game(5);
        let rx = call(&tx, "kill", json!({ "slot": 2 }));
        server.before_frame(&mut game, W, H);
        rx.recv().unwrap().unwrap();
        let rx = call(&tx, "step", json!({ "frames": 1 }));
        server.before_frame(&mut game, W, H);
        server.advance(&mut game, Input::default(), 0.016, W, H);
        let reply = rx.recv().unwrap().unwrap();
        let events = reply["events"].as_array().unwrap();
        assert!(events.iter().any(|e| e["event"] == "wreck" && e["slot"] == 2), "{events:?}");
        assert!(reply["snapshot"]["tanks"].as_array().unwrap().iter().any(|t| t["slot"] == 2 && t["wreck"] == true));
    }

    #[test]
    fn spawn_enemy_and_set_tank_change_the_roster() {
        let (mut server, tx) = DevServer::headless();
        let mut game = game(8);
        let rx = call(&tx, "spawn_enemy", json!({ "x": 640.0, "y": 100.0, "row": 3 }));
        server.before_frame(&mut game, W, H);
        let slot = rx.recv().unwrap().unwrap()["slot"].as_u64().unwrap() as usize;
        assert_eq!(slot, 5);
        let rx = call(&tx, "set_tank", json!({ "slot": slot, "damage": 42.0, "laser_charges": 3 }));
        server.before_frame(&mut game, W, H);
        let tank = rx.recv().unwrap().unwrap();
        assert_eq!(tank["damage"], 42.0);
        assert_eq!(tank["weapon"], "laser");
        assert_eq!(tank["row"], 3);
    }

    #[test]
    fn overlays_sets_inspect_like_any_other_flag() {
        let (mut server, tx) = DevServer::headless();
        let mut game = game(6);
        let rx = call(&tx, "overlays", json!({ "inspect": true, "ai": true }));
        server.before_frame(&mut game, W, H);
        let flags = rx.recv().unwrap().unwrap();
        assert_eq!(flags["inspect"], true, "{flags}");
        assert_eq!(flags["ai"], true, "{flags}");
        assert_eq!(flags["nav_grid"], false, "{flags}");
        let rx = call(&tx, "status", json!({}));
        server.before_frame(&mut game, W, H);
        let status = rx.recv().unwrap().unwrap();
        assert_eq!(status["overlays"]["inspect"], true, "{status}");
    }

    #[test]
    fn input_cycle_overlays_presses_the_i_key_once() {
        let (mut server, tx) = DevServer::headless();
        let mut game = game(7);
        assert!(!server.lockstep(), "a fresh server runs in real time");
        let expected = [Overlays::INSPECT, Overlays::ALL, Overlays::NONE];
        for preset in expected {
            let rx = call(&tx, "input", json!({ "cycle_overlays": true }));
            server.before_frame(&mut game, W, H);
            rx.recv().unwrap().unwrap();
            let input = server.shape_input(Input::default());
            assert!(input.cycle_overlays_pressed);
            assert!(input.player_intent.move_dir.is_none(), "a bare cycle request leaves the keyboard alone");
            server.advance(&mut game, input, 0.016, W, H);
            assert_eq!(game.debug_overlays, preset);
            // One-shot: the next frame's input does not press the key again.
            let input = server.shape_input(Input::default());
            assert!(!input.cycle_overlays_pressed);
            server.advance(&mut game, input, 0.016, W, H);
            assert_eq!(game.debug_overlays, preset);
        }
    }

    #[test]
    fn nav_grid_and_full_snapshot_have_the_expected_shape() {
        let (mut server, tx) = DevServer::headless();
        let mut game = game(21);
        let rx = call(&tx, "nav_grid", json!({}));
        server.before_frame(&mut game, W, H);
        let grid = rx.recv().unwrap().unwrap()["grid"].as_str().unwrap().to_string();
        let lines: Vec<&str> = grid.lines().collect();
        assert_eq!(lines.len(), 16, "15 rows plus the legend");
        assert!(lines[..15].iter().all(|l| l.len() == 27));
        assert!(grid.contains('P') && grid.contains('F'));

        let rx = call(&tx, "step", json!({ "frames": 120, "detail": "full" }));
        server.before_frame(&mut game, W, H);
        server.advance(&mut game, Input::default(), 0.016, W, H);
        let reply = rx.recv().unwrap().unwrap();
        let text = reply["snapshot"].to_string();
        assert!(text.len() < 16_000, "full snapshot is {} bytes", text.len());
        let enemy = &reply["snapshot"]["tanks"][1];
        assert!(enemy["ai"]["last_action"].is_string(), "{enemy}");
        let engage = &reply["snapshot"]["engage"];
        assert_eq!(engage["tanks"].as_array().unwrap().len(), 4, "{engage}");
        assert!(engage["tanks"][0]["status"].is_string(), "{engage}");
        assert!(reply["snapshot"]["clusters"].is_array());
        if engage["built"] == true {
            assert_eq!(engage["slots"].as_array().unwrap().len(), 16, "{engage}");
        }
        let rx = call(&tx, "snapshot", json!({}));
        server.before_frame(&mut game, W, H);
        let compact = rx.recv().unwrap().unwrap();
        assert!(compact["engage"]["slots"].is_null(), "the slot table is full-detail only");
        assert!(compact.to_string().len() < 6_000, "compact snapshot is {} bytes", compact.to_string().len());
    }

    const INLINE_MAP: &str = r#"
version = 1
tanks = 2
cells."5,5" = { kind = "start" }
cells."30,15" = { kind = "frog" }
cells."20,8" = { kind = "wall", material = "iron" }
"#;

    #[test]
    fn restart_with_map_toml_starts_a_round_on_it() {
        let (mut server, tx) = DevServer::headless();
        let mut game = game(12);
        let rx = call(&tx, "restart", json!({ "map_toml": INLINE_MAP, "seed": 5, "enemies": 2 }));
        server.before_frame(&mut game, W, H);
        let status = rx.recv().unwrap().unwrap();
        assert_eq!(status["map"]["name"], "inline", "{status}");
        assert_eq!(status["map"]["cells"], 3);
        assert_eq!(status["map"]["tanks"], 2);
        assert_eq!(status["frame"], 0);
        assert_eq!(status["lockstep"], true);
        assert_eq!(status["tanks"], 3);
        // The start cell put the player at (5, 5) cells.
        let rx = call(&tx, "snapshot", json!({}));
        server.before_frame(&mut game, W, H);
        let snap = rx.recv().unwrap().unwrap();
        assert_eq!(snap["tanks"][0]["x"], 160.0, "{}", snap["tanks"][0]);
        assert_eq!(snap["tanks"][0]["y"], 160.0);
        assert_eq!(snap["frog"]["x"], 960.0, "{}", snap["frog"]);
        assert_eq!(snap["obstacles_alive"], 1);
        // A bad map is an error and leaves the round alone.
        let bad = r#"version = 1
cells."1,1" = { kind = "wall" }"#;
        let rx = call(&tx, "restart", json!({ "map_toml": bad }));
        server.before_frame(&mut game, W, H);
        assert!(rx.recv().unwrap().unwrap_err().starts_with("map_toml:"));
        let rx = call(&tx, "restart", json!({ "map_toml": INLINE_MAP, "map": "maps/default.toml" }));
        server.before_frame(&mut game, W, H);
        assert!(rx.recv().unwrap().is_err(), "map and map_toml together");
    }

    #[test]
    fn map_get_round_trips_through_restart() {
        let (mut server, tx) = DevServer::headless();
        let mut game = game(13);
        let rx = call(&tx, "map_get", json!({}));
        server.before_frame(&mut game, W, H);
        let got = rx.recv().unwrap().unwrap();
        let cells = got["cells"].as_u64().unwrap();
        assert!(cells > 100, "{cells} cells in the default map");
        let toml = got["toml"].as_str().unwrap().to_string();
        let rx = call(&tx, "restart", json!({ "map_toml": toml }));
        server.before_frame(&mut game, W, H);
        let status = rx.recv().unwrap().unwrap();
        assert_eq!(status["map"]["cells"], cells);
    }

    #[test]
    fn history_after_a_step_has_rows_and_aggregates() {
        let (mut server, tx) = DevServer::headless();
        let mut game = game(14);
        let rx = call(&tx, "step", json!({ "frames": 120, "snapshot": false }));
        server.before_frame(&mut game, W, H);
        server.advance(&mut game, Input::default(), 0.016, W, H);
        rx.recv().unwrap().unwrap();
        let rx = call(&tx, "history", json!({ "last": 100, "every": 10, "slot": 1 }));
        server.before_frame(&mut game, W, H);
        let history = rx.recv().unwrap().unwrap();
        assert_eq!(history["from"], 21);
        assert_eq!(history["to"], 120);
        let rows = history["rows"].as_array().unwrap();
        assert_eq!(rows.len(), 10, "{history}");
        assert!(rows.iter().all(|r| r["slot"] == 1 && r["frame"].is_u64()));
        let stats = &history["tanks"]["1"];
        assert_eq!(stats["frames"], 100);
        assert!(stats["distance"].as_f64().unwrap() >= stats["net"].as_f64().unwrap(), "{stats}");
        assert!(history["tanks"]["0"]["frames"] == 100, "aggregates cover every tank: {}", history["tanks"]);
        // A restart clears the ring.
        let rx = call(&tx, "restart", json!({ "seed": 1 }));
        server.before_frame(&mut game, W, H);
        rx.recv().unwrap().unwrap();
        let rx = call(&tx, "history", json!({}));
        server.before_frame(&mut game, W, H);
        let history = rx.recv().unwrap().unwrap();
        assert_eq!(history["from"], 0);
        assert_eq!(history["to"], 0);
    }

    #[test]
    fn events_kinds_filter_keeps_only_requested_kinds() {
        let (mut server, tx) = DevServer::headless();
        let mut game = game(15);
        let rx = call(&tx, "step", json!({ "frames": 300, "snapshot": false, "kinds": ["ai_action"] }));
        server.before_frame(&mut game, W, H);
        assert!(game.trace_ai, "the server switches AI tracing on");
        server.advance(&mut game, Input::default(), 0.016, W, H);
        let step = rx.recv().unwrap().unwrap();
        let events = step["events"].as_array().unwrap();
        assert!(!events.is_empty(), "300 frames of AI produce action changes");
        assert!(events.iter().all(|e| e["event"] == "ai_action"), "{events:?}");
        let rx = call(&tx, "events", json!({ "exclude": ["ai_action", "engage_slot"], "limit": 4096 }));
        server.before_frame(&mut game, W, H);
        let all = rx.recv().unwrap().unwrap();
        let kinds: Vec<&str> = all["events"].as_array().unwrap().iter().map(|e| e["event"].as_str().unwrap()).collect();
        assert!(kinds.contains(&"fired"), "{kinds:?}");
        assert!(!kinds.iter().any(|k| *k == "ai_action" || *k == "engage_slot"), "{kinds:?}");
        let rx = call(&tx, "events", json!({ "kinds": "ai_action" }));
        server.before_frame(&mut game, W, H);
        assert!(rx.recv().unwrap().is_err(), "kinds must be an array");
    }

    #[test]
    fn restart_with_a_seed_replays_identically() {
        let (mut server, tx) = DevServer::headless();
        let mut game = game(1);
        let mut runs = Vec::new();
        for _ in 0..2 {
            let rx = call(&tx, "restart", json!({ "seed": "0xC0FFEE", "enemies": 3 }));
            server.before_frame(&mut game, W, H);
            let status = rx.recv().unwrap().unwrap();
            assert_eq!(status["seed"], "0xc0ffee");
            assert_eq!(status["tanks"], 4);
            let rx = call(&tx, "step", json!({ "frames": 300 }));
            server.before_frame(&mut game, W, H);
            server.advance(&mut game, Input::default(), 0.016, W, H);
            runs.push(rx.recv().unwrap().unwrap()["snapshot"].to_string());
        }
        assert_eq!(runs[0], runs[1]);
        let rx = call(&tx, "events", json!({ "since": 0, "limit": 5 }));
        server.before_frame(&mut game, W, H);
        let events = rx.recv().unwrap().unwrap();
        assert_eq!(events["events"][0]["event"], "round_started");
    }

    #[test]
    fn tuning_set_stages_a_patch_and_bad_keys_are_rejected() {
        let (mut server, tx) = DevServer::headless();
        let mut game = game(2);
        let rx = call(&tx, "tuning_set", json!({ "patch": { "no_such_knob": 1 } }));
        server.before_frame(&mut game, W, H);
        assert!(rx.recv().unwrap().is_err());
        let rx = call(&tx, "tuning_schema", json!({ "name_contains": "tank_speed" }));
        server.before_frame(&mut game, W, H);
        let schema = rx.recv().unwrap().unwrap();
        assert!(schema["count"].as_u64().unwrap() >= 1);
    }

    #[test]
    fn socket_round_trip_answers_a_status_request() {
        let mut server = DevServer::start(0).expect("bind an ephemeral port");
        let port = server.port();
        let mut game = game(9);
        let (done_tx, done_rx) = mpsc::channel();
        thread::spawn(move || {
            let stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
            let mut writer = stream.try_clone().unwrap();
            writeln!(writer, r#"{{"id": 7, "method": "status", "params": {{}}}}"#).unwrap();
            let mut line = String::new();
            BufReader::new(stream).read_line(&mut line).unwrap();
            done_tx.send(line).unwrap();
        });
        let mut reply = None;
        for _ in 0..2000 {
            server.before_frame(&mut game, W, H);
            if let Ok(line) = done_rx.try_recv() {
                reply = Some(line);
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }
        let reply: Value = serde_json::from_str(&reply.expect("reply within 4s")).unwrap();
        assert_eq!(reply["id"], 7);
        assert_eq!(reply["result"]["frame"], 0);
    }
}
