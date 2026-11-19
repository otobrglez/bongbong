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

use std::collections::{BTreeMap, VecDeque};
#[cfg(feature = "render")]
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use serde::Serialize;
use serde_json::{Map, Value, json};
use crate::math::Vec2;
#[cfg(feature = "render")]
use sola_raylib::prelude::{RaylibHandle, RaylibTexture2D, RaylibThread, RenderTexture2D};

use crate::ai::Intent;
use crate::bong::{BongKind, Side};
use crate::editor::{
    Axis, BuilderFrame, BuilderInput, Category, CellChange, CellRect, EditorAction, MapEditor, Shape, Tool, parse_mission, parse_spawn, parse_tank, parse_tier,
};
use crate::hud::{players_dialog_rects, CornerButton, CornerShape, Corners, UiFrame};
use crate::map::MapFile;
use crate::maplint::LintSeverity;
use crate::mode::{Driver, Session};
use crate::net::client::Phase;
use crate::net::round::AnyRound;
use crate::obstacle::Obstacle;
use crate::simulation::debug::{CLUSTER_RADIUS_PX, Detail, FieldTarget, JITTER_WINDOW_FRAMES, SPIN_FULL_CIRCLE_DEG, SPIN_NET_MAX, SPIN_WINDOW_FRAMES, TrackRow, r1, signed_quarter_turn};
use crate::simulation::{Event, Game, Input, Overlays, PlayerCount};
use crate::tank::{Dir, TankKind};
use crate::tuning;
use crate::level::{Mission, SpawnKind, Tier};
use crate::level_select::SelectInput;
use crate::follow::{CameraMode, CameraReport, FollowReport};
use crate::framing::Seating;
use crate::view::{Camera, View};
use crate::{Layout, PHYSICS_FIXED_DT, Position, parse_seed};

/// Port the game listens on unless `--dev-port`/`BONGBONG_DEV_PORT` says
/// otherwise; the adapter defaults to the same.
pub const DEFAULT_PORT: u16 = 4747;

/// The room server's dev port (`ROOM_TOOLS`), one past the room server's
/// own 4848 so both can run on one machine. `bbmcp rooms` dials it and
/// `--dev-port`/`BONGBONG_ROOMS_DEV_PORT` moves it.
pub const ROOMS_DEV_PORT: u16 = 4849;

/// Longest a socket thread waits for the main loop to answer one request
/// (a long `step` still finishes in well under a second). The adapter's
/// own read timeout is longer than this, so the reply a client sees for a
/// hung request is this server's message, not a bare socket error.
pub const REPLY_TIMEOUT: Duration = Duration::from_secs(120);
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
#[cfg_attr(not(feature = "render"), allow(dead_code))]
const SHOT_DIR: &str = "target/devshots";
/// How far apart the points of a `click {drag_to}` drag are sampled: well
/// under a 32 px cell, so the stroke crosses every cell on the line.
const CLICK_DRAG_STEP_PX: f32 = 8.0;

/// The `input` fields an online round refuses: a window holds one seat in
/// a room, which the frame's first intent drives, so player 2's keys
/// steer nobody, and the I key's overlay cycle is the local round's (the
/// replica is never `update`d - `overlays` sets its flags).
const ONLINE_INPUT_REFUSED: &[&str] = &["p2_move_dir", "p2_face", "p2_fire", "p2_lamp", "cycle_overlays"];

/// Tiles one `terrain` reply lists at most (the standard 34 x 17 field
/// has 578 cells; a size-study map can have more).
const TERRAIN_MAX_TILES: usize = 800;

mod headless;
mod tools;

use tools::KEY_NAMES;
pub use headless::Headless;
pub use tools::room as room_params;
pub use tools::{
    ATTACH, AttachParams, BATCH, BatchParams, HELP, PROBE, Params, Present, ProbeParams, RELOAD, ROOM_TOOLS, STATUS_SECTIONS, TOOLS, ToolSpec, find, help, parse,
};

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
    fn of(kinds: Option<Vec<String>>, exclude: Option<Vec<String>>) -> EventFilter {
        EventFilter { kinds: kinds.unwrap_or_default(), exclude: exclude.unwrap_or_default() }
    }

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
    /// Frames in hull contact with another tank - the non-saturating
    /// counterpart of a ram count, which caps at roughly one event per tank
    /// per `ram_damage_cooldown` (docs/enemy-command-and-control-prd.md
    /// section 10).
    tank_touching_frames: u32,
    /// The tank's heading-turn counters since the round began (not just
    /// this window) - see `TurnStats`.
    round: Option<TurnStats>,
    #[serde(skip)]
    first: Option<Position>,
    #[serde(skip)]
    last: Option<Position>,
}

/// One tank's heading-rotation counters since the round began, kept for
/// as long as the history ring is (cleared with it on a restart). Fed one
/// `TrackRow` per simulated frame by `record_history`; the rules are the
/// probe's `spin`/`jitter` ones from `simulation::debug`, so a live count
/// and a probe count of the same round agree. The visible symptom of a
/// bad value is a turret sweeping round and round: both sprite angles
/// only ever chase `Tank::rotation`, so the turret is counted here from
/// the heading that drives it, and `turret_deg` reports the sweep itself.
#[derive(Clone, Default, Serialize)]
struct TurnStats {
    /// Frames on which `rotation` changed.
    turns: u32,
    /// 180-degree flips (up to down in one step).
    u_turns: u32,
    /// A -> B -> A heading triples inside `JITTER_WINDOW_FRAMES` - the
    /// probe's jitter unit.
    reversals: u32,
    /// Full same-direction circles inside `SPIN_WINDOW_FRAMES` with under
    /// `SPIN_NET_MAX` px of drift - the probe's `spin` anomaly, once per
    /// circle.
    spins: u32,
    /// The longest same-direction chain of quarter-turns seen, in degrees
    /// (270 means it got three turns into a circle and stopped).
    max_spin_deg: f32,
    /// Degrees the turret sprite has swept in total, the short way round
    /// each frame.
    turret_deg: f32,
    /// The frame `rotation` last changed on.
    last_turn_frame: Option<u64>,
    #[serde(skip)]
    last_rotation: f32,
    #[serde(skip)]
    last_turret: f32,
    /// The last three distinct headings with the frame each began on.
    #[serde(skip)]
    headings: VecDeque<(f32, u64)>,
    #[serde(skip)]
    spin_sum: f32,
    #[serde(skip)]
    spin_start_frame: u64,
    #[serde(skip)]
    spin_start_pos: Position,
}

impl TurnStats {
    fn start(row: &TrackRow, frame: u64) -> TurnStats {
        let pos = Position::new(row.x, row.y);
        let mut headings = VecDeque::with_capacity(3);
        headings.push_back((row.rotation, frame));
        TurnStats {
            last_rotation: row.rotation,
            last_turret: row.turret,
            headings,
            spin_start_frame: frame,
            spin_start_pos: pos,
            ..TurnStats::default()
        }
    }

    /// Account for this frame's row.
    fn observe(&mut self, row: &TrackRow, frame: u64) {
        let pos = Position::new(row.x, row.y);
        let mut turret_delta = (row.turret - self.last_turret).rem_euclid(360.0);
        if turret_delta > 180.0 {
            turret_delta -= 360.0;
        }
        self.turret_deg += turret_delta.abs();
        self.last_turret = row.turret;
        if row.rotation == self.last_rotation {
            return;
        }
        self.turns += 1;
        self.last_turn_frame = Some(frame);
        self.headings.push_back((row.rotation, frame));
        if self.headings.len() > 3 {
            self.headings.pop_front();
        }
        if let (Some(a), Some(c)) = (self.headings.front(), self.headings.back())
            && self.headings.len() == 3
            && a.0 == c.0
            && frame - a.1 <= u64::from(JITTER_WINDOW_FRAMES)
        {
            self.reversals += 1;
        }
        match signed_quarter_turn(self.last_rotation, row.rotation) {
            Some(delta) => {
                let same_dir = self.spin_sum != 0.0 && (self.spin_sum > 0.0) == (delta > 0.0);
                let in_window = frame - self.spin_start_frame <= u64::from(SPIN_WINDOW_FRAMES);
                if same_dir && in_window {
                    self.spin_sum += delta;
                } else {
                    self.spin_sum = delta;
                    self.spin_start_frame = frame;
                    self.spin_start_pos = pos;
                }
                self.max_spin_deg = self.max_spin_deg.max(self.spin_sum.abs());
                if self.spin_sum.abs() >= SPIN_FULL_CIRCLE_DEG && pos.distance_to(self.spin_start_pos) <= SPIN_NET_MAX {
                    self.spins += 1;
                    self.spin_sum = 0.0;
                }
            }
            None => {
                self.u_turns += 1;
                self.spin_sum = 0.0;
                self.spin_start_frame = frame;
                self.spin_start_pos = pos;
            }
        }
        self.last_rotation = row.rotation;
    }
}

struct PendingStep {
    remaining: u64,
    intent: Option<Intent>,
    /// Player 2's held intent (`p2_*`), two-player rounds.
    intent2: Option<Intent>,
    /// Tap the trigger every this many frames instead of holding it.
    fire_every: Option<u64>,
    want_snapshot: bool,
    detail: Detail,
    filter: EventFilter,
    events: Vec<EventRecord>,
    restarted: bool,
    /// Stop after the frame one of these events happens on (`step {until}`).
    until: Vec<String>,
    /// Stop after the frame the round ends on (`step {until_outcome}`).
    until_outcome: bool,
    reply: Reply,
}

#[derive(Clone, Copy, PartialEq)]
enum ShotSource {
    Screen,
    Scene,
}

/// A `screenshot` request armed for `after_render`. Without the `render`
/// feature nothing captures, so a headless server only ever arms it.
#[cfg_attr(not(feature = "render"), allow(dead_code))]
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
    /// The same for player 2 (`input {p2_*}`).
    injected2: Option<(Intent, u32)>,
    /// An `input {cycle_overlays}` request waiting to press the I key on
    /// the next frame's input.
    cycle_overlays_pending: bool,
    pending_shot: Option<PendingShot>,
    events: VecDeque<EventRecord>,
    next_seq: u64,
    #[cfg_attr(not(feature = "render"), allow(dead_code))]
    shot_seq: u64,
    /// One entry per simulated frame, oldest first - see `history`.
    history: VecDeque<HistoryFrame>,
    /// Per-slot heading-turn counters for the whole round, cleared with
    /// `history`.
    turns: BTreeMap<usize, TurnStats>,
    /// The replica tick whose events and track rows are already banked,
    /// in an online round; `None` in every other mode.
    shown_frame: Option<u64>,
    /// The view the `camera` tool pinned; `None` leaves the view to the
    /// map: the whole field for an arena, a followed view for a field map.
    camera: Option<CameraPin>,
    /// The camera the window drew its last frame with, as it handed it
    /// over (`publish_camera`): what `status.camera` reports, and the
    /// bitmap a `click` lands on. `None` in a server with no window.
    drawn: Option<CameraReport>,
    /// The window's UI frame for the frame it drew last
    /// (`publish_ui`): what `status.ui` reports and what a `click` on the
    /// chrome is measured in. `None` in a server with no window, which
    /// lays the chrome out in the bitmap's own size (`UiFrame::plain`).
    drawn_ui: Option<UiFrame>,
    /// The fingers a `builder_touch {hold: true}` left down, in window
    /// coordinates: the window's builder frames take them in place of its own
    /// empty touch screen (`held_touches`) until the next `builder_touch`
    /// lifts them, so a screenshot catches a gesture mid-way.
    held_touches: Vec<crate::touch::TouchPoint>,
    /// The input a tool last stood in for - `key` the keys, a
    /// `builder_touch` or a `click {touch}` a tap -, which the window's
    /// hints turn to (`take_hints`, `hud::Hints`); a server with no window
    /// lays its chrome out with it.
    hints: Option<crate::hud::Hints>,
    /// No window drives this server: `devserver::Headless` does.
    windowless: bool,
}

/// A view the `camera` tool pinned: the world point at its centre and how
/// many times the field is magnified (`view::Camera::zoomed`, which keeps
/// it inside whichever field is on screen).
#[derive(Clone, Copy, Debug, PartialEq)]
struct CameraPin {
    center: Vec2,
    zoom: f32,
}

/// `status.frame_stages` (`frame_stages.rs`): where the window's frames
/// spend their time, ms a frame averaged over about half a second, one key
/// a stage; `draws`, what the last whole frame drew; `pace`, the worst
/// frame and the 99th percentile over the last `frames` frames, ms, and
/// how many of them ran 0, 1, .. simulation steps.
fn frame_stages_json() -> Value {
    let ms = |ms: f32| json!((ms * 1000.0).round() / 1000.0);
    let mut stages: serde_json::Map<String, Value> = crate::frame_stages::averages().into_iter().map(|(name, t)| (name.to_string(), ms(t))).collect();
    let draws: serde_json::Map<String, Value> = crate::frame_stages::draws().named().into_iter().map(|(name, n)| (name.to_string(), json!(n))).collect();
    let pace = crate::frame_stages::pace();
    stages.insert("draws".to_string(), Value::Object(draws));
    stages.insert("pace".to_string(), json!({ "frames": pace.frames, "max_ms": ms(pace.max_ms), "p99_ms": ms(pace.p99_ms), "steps": pace.steps }));
    Value::Object(stages)
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
            injected2: None,
            cycle_overlays_pending: false,
            pending_shot: None,
            events: VecDeque::with_capacity(EVENT_RING),
            next_seq: 1,
            shot_seq: 0,
            history: VecDeque::with_capacity(HISTORY_FRAMES),
            turns: BTreeMap::new(),
            shown_frame: None,
            camera: None,
            drawn: None,
            drawn_ui: None,
            held_touches: Vec::new(),
            hints: None,
            windowless: false,
        }
    }

    /// The input a tool stood in for since the window last asked - the
    /// keys or a tap -, for the window's hints to follow as they follow
    /// the input it reads itself (`hud::Hints`).
    pub fn take_hints(&mut self) -> Option<crate::hud::Hints> {
        self.hints.take()
    }

    /// The fingers a `builder_touch {hold: true}` keeps down, which the
    /// window's builder frames take in place of its own touch screen's
    /// while it reports none; empty when none are held.
    pub fn held_touches(&self) -> &[crate::touch::TouchPoint] {
        &self.held_touches
    }

    /// The view the `camera` tool pinned, over a field of `field`; `None`
    /// leaves the view to the map.
    pub fn pinned_camera(&self, field: (f32, f32)) -> Option<Camera> {
        self.camera.map(|pin| Camera::zoomed(field, pin.center, pin.zoom))
    }

    /// The pinned view over a field of `field`, else the whole field: what
    /// an arena draws.
    pub fn camera(&self, field: (f32, f32)) -> Camera {
        self.pinned_camera(field).unwrap_or(Camera::whole(field))
    }

    /// The window's camera for the frame it is drawing, handed over once a
    /// frame: `status.camera` reports the last one, and `click` hit-tests
    /// on its bitmap.
    pub fn publish_camera(&mut self, report: CameraReport) {
        self.drawn = Some(report);
    }

    /// The window's UI frame for the frame it is drawing, handed over once
    /// a frame like the camera: `status.ui` reports it.
    pub fn publish_ui(&mut self, ui: UiFrame) {
        self.drawn_ui = Some(ui);
    }

    /// What a `click` lands on, in the window's own coordinates: the bitmap
    /// the window drew last and the view that put it there, and the UI
    /// frame its chrome is laid out in - while that bitmap is this mode's
    /// (the builder's canvas, or the round). With no window drawn yet, or a
    /// mode switched since, the live mode's own bitmap in a window of
    /// exactly its size - the builder's canvas with its plates over it
    /// (`BuilderFrame::headless`), or play's field alone - which is all a
    /// server with no window ever has.
    fn click_frame(&self, session: &Session, width: f32, height: f32) -> (Layout, View, UiFrame) {
        let build = session.mode() == Driver::Build;
        match (&self.drawn, self.drawn_ui) {
            (Some(drawn), Some(ui)) if build == (drawn.mode == CameraMode::Build) => (drawn.layout, drawn.view, ui),
            _ if build => {
                let map = session.builder.map();
                let frame = BuilderFrame::headless(map.field_size(), map.class());
                (frame.layout, frame.view, frame.ui.with_hints(self.hints.unwrap_or_default()))
            }
            _ => {
                let layout = Layout::bare(width, height);
                let (w, h) = layout.window_size();
                let window = (w as f32, h as f32);
                (layout, View::fit(window, window), UiFrame::plain(window).with_hints(self.hints.unwrap_or_default()))
            }
        }
    }

    /// The builder's frame a `click` or a `builder_touch` lands in
    /// (`click_frame`): its canvas through the view, its chrome in the UI.
    fn builder_frame(&self, session: &Session, width: f32, height: f32) -> BuilderFrame {
        let (layout, view, ui) = self.click_frame(session, width, height);
        BuilderFrame { layout, view, ui }
    }

    /// A rect of the builder's chrome, in UI points, as `status.builder`
    /// reports it: in window coordinates - what `click` and `builder_touch`
    /// take - with the points themselves as `ui`.
    fn ui_rect_json(ui: &UiFrame, r: crate::math::Rectangle) -> Value {
        let w = ui.rect_to_window(r);
        json!({ "x": w.x, "y": w.y, "w": w.width, "h": w.height, "ui": { "x": r.x, "y": r.y, "w": r.width, "h": r.height } })
    }

    /// `status.builder.navigator`: where the builder's navigator stands
    /// (`MapEditor::navigator_rect`) through the frame the window last drew
    /// the builder in - in window coordinates, what `click` and
    /// `builder_touch` take, with its `ui` points; `null` where there is
    /// none (FIT on an arena) or the window is not in build mode.
    fn navigator_json(&self, session: &Session, width: f32, height: f32) -> Value {
        if session.mode() != Driver::Build {
            return Value::Null;
        }
        let frame = self.builder_frame(session, width, height);
        session.builder.navigator_rect(&frame).map_or(Value::Null, |r| Self::ui_rect_json(&frame.ui, r))
    }

    /// `status.builder.buttons`: the builder's own buttons a tool clicks
    /// by name (`MapEditor::named_buttons`) through the frame the window
    /// last drew the builder in: window coordinates, what `click` and
    /// `builder_touch` take, each with its `ui` points. Empty outside build
    /// mode.
    fn builder_buttons_json(&self, session: &Session, width: f32, height: f32) -> Value {
        if session.mode() != Driver::Build {
            return json!({});
        }
        let frame = self.builder_frame(session, width, height);
        let buttons: Map<String, Value> =
            session.builder.named_buttons(&frame).into_iter().map(|(name, r)| (name, Self::ui_rect_json(&frame.ui, r))).collect();
        Value::Object(buttons)
    }

    /// `status.builder.loupe`: the loupe over a painting finger
    /// (`MapEditor::loupe`) - where it stands in window coordinates, with
    /// its `ui` points; the world it shows; its device pixels per world
    /// pixel; the cell the stroke paints. `null` while there is none - no
    /// finger painting, a mouse, a cell over `builder_loupe_cell_mm` - and
    /// outside build mode. A `builder_touch` lifts its fingers at its end
    /// unless it `hold`s them, which is how a shell reads it mid-stroke.
    fn loupe_json(&self, session: &Session, width: f32, height: f32) -> Value {
        if session.mode() != Driver::Build {
            return Value::Null;
        }
        let frame = self.builder_frame(session, width, height);
        let Some(loupe) = session.builder.loupe(&frame) else { return Value::Null };
        let mut v = Self::ui_rect_json(&frame.ui, loupe.rect);
        let w = loupe.world;
        v["world"] = json!({ "x": w.x, "y": w.y, "w": w.width, "h": w.height });
        v["device_scale"] = json!(loupe.device_scale);
        v["cell"] = json!([loupe.cell.0, loupe.cell.1]);
        v["erase"] = json!(loupe.erase);
        v
    }

    /// The corners the window lays out for the live mode in `ui`
    /// (`hud::corners`), `None` where it draws none.
    fn corners(session: &Session, ui: &UiFrame) -> Option<Corners> {
        CornerShape::of(&session.play_chrome(), session.shown().players.count()).map(|shape| crate::hud::corners(ui, &shape))
    }

    /// `camera`: pin a view, let it go, or say where it is. A pin's field
    /// left out keeps the one it had (the field's centre at zoom 1 for a
    /// first pin).
    fn pin_camera(&mut self, params: &tools::CameraParams, session: &Session, field: (f32, f32)) -> Result<Value, String> {
        if params.reset.unwrap_or(false) {
            self.camera = None;
            return Ok(self.camera_json(session, field));
        }
        let number = |key: &str, given: Option<f64>| -> Result<Option<f32>, String> {
            match given {
                None => Ok(None),
                Some(n) if n.is_finite() => Ok(Some(n as f32)),
                Some(n) => Err(format!("{key} must be a number, got {n}")),
            }
        };
        let (x, y, zoom) = (number("x", params.x)?, number("y", params.y)?, number("zoom", params.zoom)?);
        if x.is_some() || y.is_some() || zoom.is_some() {
            let pin = self.camera.unwrap_or(CameraPin { center: Vec2::new(field.0 / 2.0, field.1 / 2.0), zoom: 1.0 });
            let zoom = zoom.unwrap_or(pin.zoom);
            if zoom < 1.0 {
                return Err(format!("zoom must be 1 or more (1 is the whole field), got {zoom}"));
            }
            self.camera = Some(CameraPin { center: Vec2::new(x.unwrap_or(pin.center.x), y.unwrap_or(pin.center.y)), zoom });
        }
        Ok(self.camera_json(session, field))
    }

    /// `status.ui`: the chrome as the window last laid it out
    /// (`capi::ui_status`) - in window coordinates, which is what `click`
    /// takes. With no window, the frame `click` lays the chrome out in
    /// (`click_frame`).
    fn ui_json(&self, session: &Session, width: f32, height: f32) -> Value {
        let (_, _, ui) = self.click_frame(session, width, height);
        crate::capi::ui_status(session, &ui)
    }

    /// `status.camera` and the `camera` tool's reply: the view in force -
    /// `pinned`, `whole` or `follow` - the world rectangle on screen, its
    /// scale and the scene target's size; a pin's centre and zoom; and a
    /// followed view's seat, focus and framing as the window last drew it.
    /// With no frame drawn since the pin went (or no window at all), the
    /// view the map's class gives, its position unknown. `motion` is the
    /// motion switch in force (`motion_json`).
    fn camera_json(&self, session: &Session, field: (f32, f32)) -> Value {
        let mut v = self.view_json(session, field);
        v["motion"] = motion_json();
        v
    }

    /// `camera_json` but for the motion switch.
    fn view_json(&self, session: &Session, field: (f32, f32)) -> Value {
        let plain = |view: &str, camera: Camera| {
            let rect = camera.rect();
            let (w, h) = camera.target_size();
            json!({
                "view": view,
                "rect": { "x": rect.x, "y": rect.y, "w": rect.width, "h": rect.height },
                "scale": camera.scale,
                "target": [w, h],
                "center": self.camera.map(|pin| json!({ "x": pin.center.x, "y": pin.center.y })),
                "zoom": self.camera.map(|pin| pin.zoom),
            })
        };
        if let Some(camera) = self.pinned_camera(field) {
            return plain("pinned", camera);
        }
        match &self.drawn {
            Some(drawn) if drawn.mode != CameraMode::Pinned => {
                let mut v = plain(drawn.mode.name(), drawn.camera);
                // Where the field area landed on the window, in its points.
                let (view, field) = (drawn.view, drawn.layout.field);
                let corner = view.to_window(Vec2::new(field.x, field.y));
                v["window_field"] = json!({ "x": corner.x, "y": corner.y, "w": field.w * view.scale, "h": field.h * view.scale });
                if let Some(follow) = &drawn.follow {
                    v.as_object_mut().expect("an object").extend(follow_json(follow, &drawn.camera, &drawn.view, &drawn.layout).as_object().expect("an object").clone());
                }
                v
            }
            _ if session.mode() != Driver::Build && session.shown().map.class().follows() => json!({
                "view": "follow",
                "rect": Value::Null,
                "center": Value::Null,
                "zoom": Value::Null,
            }),
            _ => plain("whole", Camera::whole(field)),
        }
    }

    /// `status.host` and `mode.host`: what answers - a window or a
    /// headless round - with its process and the directory it runs in,
    /// which is how `bbmcp` tells this checkout's game from another's on
    /// the same port.
    fn host_json(&self) -> Value {
        json!({
            "kind": if self.windowless { "headless" } else { "window" },
            "pid": std::process::id(),
            "cwd": std::env::current_dir().ok().map(|dir| dir.display().to_string()),
        })
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
    pub fn before_frame(&mut self, session: &mut Session, width: f32, height: f32) {
        session.watch_originals();
        // The AI-decision events exist for this server's `events` feed.
        session.game.trace_ai = true;
        self.observe_replica(session);
        while let Ok(req) = self.rx.try_recv() {
            self.dispatch(session, req, width, height);
        }
        // Held fingers are the builder's alone: another mode lets them go.
        if session.mode() != Driver::Build {
            self.held_touches.clear();
        }
    }

    /// Bank an online replica's events and track rows on the frames a
    /// snapshot moved it on, so `events` and `history` describe the round
    /// on screen. A local round does this in `advance`, after every
    /// update; an online one is never updated here, and its events are
    /// the snapshot's (`net::apply` writes them), so the guard is the
    /// replica's tick - the same one `Fx::observe` watches.
    fn observe_replica(&mut self, session: &Session) {
        if session.mode() != Driver::Online {
            self.shown_frame = None;
            return;
        }
        let Some(game) = session.online.as_ref().and_then(AnyRound::game) else { return };
        let frame = game.frame();
        if self.shown_frame == Some(frame) {
            return;
        }
        self.shown_frame = Some(frame);
        self.drain_events(game, None);
        self.record_history(game);
    }

    /// Substitute injected player intent for the keyboard's, if any is
    /// pending (counts that override down by one frame), and press the I
    /// key for this one frame when an `input {cycle_overlays}` is waiting.
    pub fn shape_input(&mut self, real: Input) -> Input {
        let mut input = real;
        if let Some((intent, left)) = self.injected {
            self.injected = (left > 1).then_some((intent, left - 1));
            input.seats[0] = intent;
        }
        if let Some((intent, left)) = self.injected2 {
            self.injected2 = (left > 1).then_some((intent, left - 1));
            input.seats[1] = intent;
        }
        if std::mem::take(&mut self.cycle_overlays_pending) {
            input.cycle_overlays_pressed = true;
        }
        input
    }

    /// Advance the game for this rendered frame: a pending `step` runs its
    /// frames back-to-back at `PHYSICS_FIXED_DT` and replies; otherwise the
    /// `steps` the loop's clock paid for (each at `PHYSICS_FIXED_DT`, the
    /// one-shot presses in `input` spent by the first) unless lockstep holds
    /// the game still. `after_step` runs after every update, on the state
    /// it produced - the presentation's event readers hook in there so a
    /// frame of several steps drops none of their events.
    pub fn advance(&mut self, game: &mut Game, input: Input, steps: u32, width: f32, height: f32, after_step: &mut dyn FnMut(&Game)) {
        if let Some(mut step) = self.pending_step.take() {
            let (mut frames_run, mut stopped) = (0, "frames");
            for i in 0..step.remaining {
                let mut player1 = step.intent.unwrap_or(input.seat(0));
                let mut player2 = step.intent2.unwrap_or(input.seat(1));
                if let Some(n) = step.fire_every {
                    player1.fire = player1.fire && i % n == 0;
                    player2.fire = player2.fire && i % n == 0;
                }
                let before = game.frame();
                game.update(Input::two(player1, player2), PHYSICS_FIXED_DT, width, height);
                if game.frame() != before + 1 {
                    step.restarted = true;
                }
                let mut sink = std::mem::take(&mut step.events);
                let first = self.next_seq;
                self.drain_events(game, Some((&mut sink, &step.filter)));
                step.events = sink;
                self.record_history(game);
                after_step(game);
                frames_run = i + 1;
                // The frame is whole either way: a stop is between frames.
                if self.events.iter().rev().take_while(|e| e.seq >= first).any(|e| step.until.contains(&e.kind)) {
                    stopped = "event";
                    break;
                }
                if step.until_outcome && game.outcome() != crate::simulation::Outcome::Playing {
                    stopped = "outcome";
                    break;
                }
            }
            let snapshot = step.want_snapshot.then(|| to_value(game.debug_snapshot(width, height, step.detail)));
            let _ = step.reply.send(Ok(json!({
                "frame": game.frame(),
                "time": r1(game.time),
                "outcome": game.outcome(),
                "restarted": step.restarted,
                "lockstep": true,
                "frames_run": frames_run,
                "stopped": stopped,
                "events": step.events,
                "snapshot": snapshot,
            })));
        } else if !self.lockstep {
            for i in 0..steps {
                let input = if i == 0 { input } else { input.held_only() };
                game.update(input, PHYSICS_FIXED_DT, width, height);
                self.drain_events(game, None);
                self.record_history(game);
                after_step(game);
            }
        }
    }

    /// Append this frame's `TrackRow`s to the history ring. A frame number
    /// that doesn't follow the last one means the round restarted, so the
    /// ring starts over.
    fn record_history(&mut self, game: &Game) {
        let frame = game.frame();
        if self.history.back().is_some_and(|last| frame <= last.frame) {
            self.history.clear();
            self.turns.clear();
        }
        if self.history.len() == HISTORY_FRAMES {
            self.history.pop_front();
        }
        let rows = game.debug_track_rows();
        for row in &rows {
            match self.turns.get_mut(&row.slot) {
                Some(stats) => stats.observe(row, frame),
                None => {
                    self.turns.insert(row.slot, TurnStats::start(row, frame));
                }
            }
        }
        self.history.push_back(HistoryFrame { frame, rows });
    }

    /// `status.turns`: the turn counters summed over every tank seen this
    /// round - a non-zero `spins` is the cheap "something is rotating in
    /// place" flag, `history` names the tank.
    fn turns_summary(&self) -> Value {
        let sum = |f: fn(&TurnStats) -> u32| self.turns.values().map(f).sum::<u32>();
        json!({
            "turns": sum(|t| t.turns),
            "u_turns": sum(|t| t.u_turns),
            "reversals": sum(|t| t.reversals),
            "spins": sum(|t| t.spins),
        })
    }

    /// The `history` reply: sampled rows plus per-tank aggregates over the
    /// last `last` frames.
    fn history_json(&self, last: usize, every: usize, slot: Option<usize>) -> Value {
        let skip = self.history.len().saturating_sub(last);
        let window: Vec<&HistoryFrame> = self.history.iter().skip(skip).collect();
        let mut stats: BTreeMap<usize, TrackStats> = BTreeMap::new();
        let mut rows = Vec::new();
        for (i, hf) in window.iter().enumerate() {
            let sample = i % every == 0 && rows.len() < HISTORY_MAX_ROWS;
            for row in &hf.rows {
                let pos = Position::new(row.x, row.y);
                let others_near = hf
                    .rows
                    .iter()
                    .filter(|o| o.slot != row.slot && !o.is_player && Position::new(o.x, o.y).distance_to(pos) <= CLUSTER_RADIUS_PX)
                    .count();
                let st = stats.entry(row.slot).or_default();
                st.frames += 1;
                if let Some(prev) = st.last {
                    st.distance += prev.distance_to(pos);
                }
                st.first.get_or_insert(pos);
                st.last = Some(pos);
                st.cluster_frames += u32::from(!row.is_player && others_near >= 2);
                st.stuck_frames += u32::from(row.stuck);
                st.no_ring_frames += u32::from(!row.is_player && row.ring.is_none());
                st.touching_frames += u32::from(row.touching_static);
                st.tank_touching_frames += u32::from(row.touching_tank);
                if sample && slot.is_none_or(|s| s == row.slot) {
                    let mut v = to_value(row);
                    v["frame"] = json!(hf.frame);
                    rows.push(v);
                }
            }
        }
        for (slot, st) in stats.iter_mut() {
            st.net = match (st.first, st.last) {
                (Some(a), Some(b)) => a.distance_to(b),
                _ => 0.0,
            };
            st.distance = r1(st.distance);
            st.net = r1(st.net);
            st.round = self.turns.get(slot).cloned().map(|mut t| {
                t.max_spin_deg = r1(t.max_spin_deg);
                t.turret_deg = r1(t.turret_deg);
                t
            });
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
    #[cfg(feature = "render")]
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

    #[cfg(feature = "render")]
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
        // The file is a convenience for the desktop; a phone's working
        // directory is not writable (and the caller could not read the
        // file anyway), so there the PNG travels in the reply alone.
        let path = if crate::EMBEDDED {
            None
        } else {
            let dir = Path::new(SHOT_DIR);
            fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
            let path = dir.join(format!("{:05}-f{}.png", self.shot_seq, game.frame()));
            fs::write(&path, &png).map_err(|e| format!("{}: {e}", path.display()))?;
            Some(path.display().to_string())
        };
        Ok(json!({
            "frame": game.frame(),
            "path": path,
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

    /// The round on screen, described: in an online round that is the
    /// room's replica, and `round` says so (see `round_json`).
    fn status(&self, session: &Session, width: f32, height: f32) -> Value {
        let game = session.shown();
        let snap = game.debug_snapshot(width, height, Detail::Compact);
        let mut status = json!({
            "version": env!("CARGO_PKG_VERSION"),
            "port": self.port,
            "round": round_json(session),
            "seed": snap.seed,
            "frame": snap.frame,
            "time": snap.time,
            "outcome": snap.outcome,
            "mission": game.mission.name(),
            "spawn": game.spawn_plan,
            "wave": game.wave_status(),
            "pacing": game.pacing(),
            "training": game.training_status(),
            "intro_seconds_left": game.intro_timer,
            "paused": snap.paused,
            "lockstep": self.lockstep,
            "step_pending": self.pending_step.is_some(),
            "tanks": snap.tanks.len(),
            "enemies_alive": snap.tanks.iter().filter(|t| !t.is_player && !t.wreck).count(),
            "width": width,
            "height": height,
            "players": game.players.count(),
            "map": map_json(&game.map),
            "mode": session.mode().name(),
            "language": crate::text::language(),
            "back": back_json(session),
            "players_dialog_open": session.players_dialog,
            "levels_open": session.level_select.is_some(),
            "levels_focus": session.level_select.as_ref().zip(session.campaign.as_ref()).map(|(select, campaign)| campaign.levels.number(select.focus())),
            "level": level_json(session),
            // The builder's PLAY HERE spot the local round started from,
            // `null` for a round from the map's own start.
            "play_here": session.game.start_override.map(|(c, r)| [c, r]),
            "events_kept": self.events.len(),
            "next_event_seq": self.next_seq,
            "history_frames": self.history.len(),
        });
        // Past the macro's recursion limit, so set here.
        status["host"] = self.host_json();
        status["question"] = question_json(session);
        status["map_modding"] = json!(crate::mapstore::enabled());
        status
    }

    /// `status`: the core (`status()`) plus the sections asked for - the
    /// heavy parts a caller rarely needs all of (`STATUS_SECTIONS`; `all`
    /// is every one).
    fn status_with(&self, session: &Session, width: f32, height: f32, sections: &[String]) -> Result<Value, String> {
        if let Some(unknown) = sections.iter().find(|name| *name != "all" && !STATUS_SECTIONS.contains(&name.as_str())) {
            return Err(format!("status: no section is called {unknown:?}; there are {}, or all", STATUS_SECTIONS.join(", ")));
        }
        let mut status = self.status(session, width, height);
        let all = sections.iter().any(|name| name == "all");
        let game = session.shown();
        for &section in STATUS_SECTIONS {
            if !all && !sections.iter().any(|name| name == section) {
                continue;
            }
            status[section] = match section {
                "camera" => self.camera_json(session, (width, height)),
                "ui" => self.ui_json(session, width, height),
                "builder" => self.builder_json(session, width, height),
                "frame_stages" => frame_stages_json(),
                "weather" => weather_json(game),
                "stats" => to_value(game.round_stats()),
                "turns" => self.turns_summary(),
                "overlays" => overlays_json(game),
                _ => unreachable!("every STATUS_SECTIONS name has an arm"),
            };
        }
        Ok(status)
    }

    /// `status.builder`: the builder's state - see the `status` tool.
    fn builder_json(&self, session: &Session, width: f32, height: f32) -> Value {
        let b = &session.builder;
        json!({
            "dirty": b.dirty(),
            "tool": b.tool().name(),
            "shape": b.shape().name(),
            "rect": cell_rect_json(b.rect_stroke().map(|(rect, _)| rect)),
            "message": b.status(),
            "selection": cell_rect_json(b.selection()),
            "ghost": ghost_json(b),
            "clipboard": clipboard_json(b),
            "stamps": b.stamps().len(),
            "camera": builder_camera_json(&session.builder),
            "sky": b.sky().name(),
            "day": b.day(),
            "navigator": self.navigator_json(session, width, height),
            "buttons": self.builder_buttons_json(session, width, height),
            "check": builder_check_json(&session.builder),
            "clear": builder_clear_json(session),
            "loupe": self.loupe_json(session, width, height),
            "thumbnails": thumbnails_json(b),
        })
    }

    fn dispatch(&mut self, session: &mut Session, req: Request, width: f32, height: f32) {
        let Request { method, params, reply } = req;
        // The row is the tool: its parameter struct checks the call, and
        // its `in_build`/`online` say where it may run.
        let Some(spec) = find(TOOLS, &method) else {
            let _ = reply.send(Err(format!("unknown tool {method:?}; `help` lists them")));
            return;
        };
        // A tool of the round refuses while the builder is live rather
        // than touching a frozen round (docs/game-editor-fusion.md
        // section 11).
        if session.mode() == Driver::Build && !spec.in_build {
            let _ = reply.send(Err(format!("{method} needs play mode: the builder is live - call `play` first")));
            return;
        }
        // A step while the round stands still behind a question would
        // wait for frames that never run (`app.rs` advances only while
        // `Session::playing`): say what is asking instead.
        if method == "step" && session.mode() == Driver::Play && !session.playing() {
            let what = if session.question.is_some() {
                "a question about a kept map is open - click status.ui.screen_buttons' yes or no"
            } else if session.level_select.is_some() {
                "the level select is open - key escape closes it, key enter or a click on a tile starts a level"
            } else if session.players_dialog {
                "the players dialog is open - key escape closes it"
            } else {
                "the leave dialog is open - key escape keeps playing"
            };
            let _ = reply.send(Err(format!("step needs the round running: {what}")));
            return;
        }
        // The same refusal for a round that belongs to a room: only the
        // server simulates it, so a write would move the picture and
        // reach nobody.
        if session.mode() == Driver::Online && !spec.online {
            let room = session.online.as_ref().and_then(AnyRound::code).unwrap_or("-----");
            let _ = reply.send(Err(format!(
                "{method} needs a local round: this window holds a seat in room {room}, whose round only the server \
                 simulates - the replica on screen is a picture of it, not the authority. Give the seat up with \
                 `key {{\"key\": \"escape\"}}` to come back to the local round; the reading tools (status, snapshot, \
                 terrain, events, history, nav_grid, field, screenshot) already describe the online one"
            )));
            return;
        }
        // Where it may run is settled; now what it was asked.
        if let Err(e) = spec.check(&params) {
            let _ = reply.send(Err(e));
            return;
        }
        // The tools that work on the whole session - the mode switch and
        // the builder - before the ones that only see the round.
        if let Some(result) = self.dispatch_session(session, &method, &params, width, height) {
            let _ = reply.send(result);
            return;
        }
        // The `input` fields that would reach nobody in an online round.
        let online_refused = (session.mode() == Driver::Online)
            .then(|| ONLINE_INPUT_REFUSED.iter().copied().find(|f| params.get(*f).is_some()))
            .flatten();
        // The round on screen: the room's replica in an online round, the
        // session's own otherwise. Only `screenshot`/`overlays` write
        // through it, and only drawing flags.
        let game = session.shown_mut();
        let result = match method.as_str() {
            "snapshot" => parse::<tools::SnapshotParams>(&params)
                .and_then(|p| detail_of(p.detail.as_deref()))
                .map(|d| to_value(game.debug_snapshot(width, height, d))),
            "events" => parse::<tools::EventsParams>(&params).map(|p| {
                let filter = EventFilter::of(p.kinds, p.exclude);
                let since = p.since.unwrap_or(0);
                let limit = p.limit.unwrap_or(200) as usize;
                let events: Vec<&EventRecord> =
                    self.events.iter().filter(|e| e.seq > since && filter.keeps(&e.kind)).take(limit).collect();
                let next = events.last().map_or(since, |e| e.seq);
                json!({ "next": next, "events": events })
            }),
            "step" => {
                if self.pending_step.is_some() {
                    Err("a step is already in progress".to_string())
                } else {
                    let armed = parse::<tools::StepParams>(&params).and_then(|p| {
                        Ok(PendingStep {
                            remaining: frames_of(p.frames, 1)?,
                            intent: p.intent.seat(0)?,
                            intent2: p.intent.seat(1)?,
                            fire_every: p.fire_every.filter(|&n| n >= 1),
                            want_snapshot: p.snapshot.unwrap_or(false),
                            detail: detail_of(p.detail.as_deref())?,
                            filter: EventFilter::of(p.kinds, p.exclude),
                            events: Vec::new(),
                            restarted: false,
                            until: p.until.unwrap_or_default(),
                            until_outcome: p.until_outcome.unwrap_or(false),
                            reply: reply.clone(),
                        })
                    });
                    match armed {
                        Ok(step) => {
                            self.lockstep = true;
                            self.pending_step = Some(step);
                            return;
                        }
                        Err(e) => Err(e),
                    }
                }
            }
            "input" if online_refused.is_some() => Err(format!(
                "input's {} needs a local round: this window holds one seat in a room, which move_dir, face, fire and \
                 lamp drive - give the seat up with `key {{\"key\": \"escape\"}}` to drive the local round",
                online_refused.unwrap_or_default()
            )),
            "input" => parse::<tools::InputParams>(&params).and_then(|p| {
                let (intent, intent2, frames) = (p.intent.seat(0)?, p.intent.seat(1)?, frames_of(p.frames, 1)?);
                let held = frames.min(u32::MAX as u64) as u32;
                if let Some(intent) = intent {
                    self.injected = Some((intent, held));
                }
                if let Some(intent) = intent2 {
                    self.injected2 = Some((intent, held));
                }
                self.cycle_overlays_pending = p.cycle_overlays.unwrap_or(false);
                Ok(json!({ "frames": frames, "cycle_overlays": self.cycle_overlays_pending }))
            }),
            "screenshot" => {
                if self.pending_shot.is_some() {
                    Err("a screenshot is already pending".to_string())
                } else {
                    let armed = parse::<tools::ScreenshotParams>(&params).and_then(|p| {
                        let source = match p.source.as_deref().unwrap_or("screen") {
                            "screen" => ShotSource::Screen,
                            "scene" => ShotSource::Scene,
                            other => return Err(format!("unknown source {other:?} (screen|scene)")),
                        };
                        if let Some(flags) = &p.overlays {
                            flags.apply(&mut game.debug_overlays);
                        }
                        Ok(PendingShot { scale: (p.scale.unwrap_or(0.5) as f32).clamp(0.1, 1.0), source, presented: false, reply: reply.clone() })
                    });
                    match armed {
                        Ok(shot) => {
                            self.pending_shot = Some(shot);
                            return;
                        }
                        Err(e) => Err(e),
                    }
                }
            }
            "overlays" => parse::<tools::OverlayFlags>(&params).map(|flags| {
                flags.apply(&mut game.debug_overlays);
                overlays_json(game)
            }),
            "nav_grid" => Ok(json!({ "grid": game.nav_grid_ascii(width, height) })),
            "field" => parse::<tools::FieldParams>(&params).and_then(|p| {
                let target = match p.target.as_deref().unwrap_or("player") {
                    "player" => FieldTarget::Player(0),
                    "player2" => FieldTarget::Player(1),
                    "bong" => FieldTarget::Bong,
                    other => return Err(format!("unknown field target {other:?}: player, player2 or bong")),
                };
                game.field_dump(width, height, target)
                    .map(|dump| serde_json::to_value(dump).expect("FieldDump serialises"))
                    .ok_or_else(|| format!("no {target:?} in the round to build a field toward"))
            }),
            "terrain" => parse::<tools::TerrainParams>(&params).and_then(|p| terrain_json(game, &p)),
            "map_get" => game.map.to_toml_string().map(|toml| {
                let mut v = map_json(&game.map);
                v["toml"] = Value::String(toml);
                v
            }),
            "history" => parse::<tools::HistoryParams>(&params).map(|p| {
                let last = p.last.unwrap_or(600).clamp(1, HISTORY_FRAMES as u64) as usize;
                let every = p.every.unwrap_or(10).max(1) as usize;
                self.history_json(last, every, p.slot.map(|s| s as usize))
            }),
            "teleport" => parse::<tools::TeleportParams>(&params).and_then(|p| {
                let rotation = match p.facing.as_deref() {
                    Some(s) => Some(Dir::parse(s).map(|d| d.rotation()).ok_or_else(|| format!("unknown facing {s:?}"))?),
                    None => None,
                };
                let (slot, x, y) = (p.slot as usize, p.x as f32, p.y as f32);
                game.debug_teleport(slot, Position::new(x, y), rotation).map(|()| json!({ "slot": slot, "x": x, "y": y }))
            }),
            "set_tank" => parse::<tools::SetTankParams>(&params).and_then(|p| {
                let slot = p.slot as usize;
                game.debug_set_tank(slot, &p.patch)?;
                let snap = game.debug_snapshot(width, height, Detail::Compact);
                Ok(to_value(snap.tanks.into_iter().find(|t| t.slot == slot)))
            }),
            "kill" => parse::<tools::SlotParams>(&params).and_then(|p| {
                let slot = p.slot as usize;
                game.debug_kill(slot).map(|()| json!({ "slot": slot, "applied_on_next_frame": true }))
            }),
            "spawn_enemy" => parse::<tools::SpawnEnemyParams>(&params).and_then(|p| {
                let role = match p.role.as_deref() {
                    None => None,
                    Some(name) => Some(
                        serde_json::from_value::<crate::ai::Role>(json!(name))
                            .map_err(|_| format!("role: expected player, hunter or guard, got {name:?}"))?,
                    ),
                };
                game.debug_spawn_enemy(Position::new(p.x as f32, p.y as f32), p.row.map(|r| r as i32), role).map(|slot| json!({ "slot": slot }))
            }),
            "spawn_pickup" => parse::<tools::SpawnPickupParams>(&params).and_then(|p| {
                let kind = crate::pickup::PickupKind::parse(&p.kind).ok_or_else(|| format!("kind: no pickup is called {:?}", p.kind))?;
                game.debug_spawn_pickup(kind, Position::new(p.x as f32, p.y as f32)).map(|at| json!({ "kind": kind.name(), "x": at.x, "y": at.y }))
            }),
            "rod_call" => parse::<tools::StrikeParams>(&params).and_then(|p| {
                let at = Position::new(p.x as f32, p.y as f32);
                let id = game.debug_call_rod(at, p.enemy.unwrap_or(false)).ok_or("(x, y) is off the field")?;
                let cell = crate::map::world_to_cell(at);
                let land = game.zones().iter().find(|z| z.id == id).map_or(0.0, |z| z.until);
                Ok(json!({ "id": id, "cell": [cell.0, cell.1], "land": land }))
            }),
            "well_at" => parse::<tools::StrikeParams>(&params).and_then(|p| {
                let (x, y) = (p.x as f32, p.y as f32);
                let (id, until) =
                    game.debug_well(Position::new(x, y), p.enemy.unwrap_or(false)).ok_or("(x, y) is off the field or inside a solid cell")?;
                Ok(json!({ "id": id, "x": x, "y": y, "pulls_at": until }))
            }),
            "tuning_get" => parse::<tools::TuningGetParams>(&params).and_then(|p| {
                let text = if p.diff_only.unwrap_or(false) { tuning::diff_json() } else { tuning::current_json() };
                serde_json::from_str(&text).map_err(|e| e.to_string())
            }),
            "tuning_set" => parse::<tools::TuningSetParams>(&params)
                .and_then(|p| tuning::submit_json(&json!(p.patch).to_string()).map(|n| json!({ "staged": n }))),
            "tuning_reset" => {
                tuning::submit_reset();
                Ok(json!({ "reset": true }))
            }
            "tuning_schema" => parse::<tools::TuningSchemaParams>(&params).and_then(|p| {
                let rows: Vec<Value> = serde_json::from_str(&tuning::schema_json()).map_err(|e| e.to_string())?;
                let named = |r: &Value, key: &str| r.get(key).and_then(Value::as_str).map(str::to_string);
                let rows: Vec<Value> = rows
                    .into_iter()
                    .filter(|r| p.group.is_none() || named(r, "group") == p.group)
                    .filter(|r| p.name_contains.as_ref().is_none_or(|n| named(r, "name").is_some_and(|name| name.contains(n.as_str()))))
                    .collect();
                Ok(json!({ "count": rows.len(), "rows": rows }))
            }),
            other => Err(format!("{other} is in the table and has no handler")),
        };
        let _ = reply.send(result);
    }

    fn restart(&mut self, session: &mut Session, params: &tools::RestartParams) -> Result<Value, String> {
        let game = &mut session.game;
        match &params.seed {
            None => {}
            Some(tools::SeedParam::Number(n)) => game.seed_override = Some(*n),
            Some(tools::SeedParam::Text(s)) => game.seed_override = Some(parse_seed(s)?),
        }
        if let Some(n) = params.enemies {
            game.enemy_count_override = Some(n as usize);
        }
        // A chassis as a row (the sheet order) or as its name (the spelling
        // maps, `builder_settings` and `--tank` use); not both.
        let chassis = |name_key: &str, name: &Option<String>, row_key: &str, row: Option<i64>| -> Result<Option<i32>, String> {
            match (name, row) {
                (Some(_), Some(_)) => Err(format!("give {name_key} (a name) or {row_key} (0-11), not both")),
                (Some(s), None) => parse_tank(s).map(|k| Some(k.row())).ok_or_else(|| {
                    format!("unknown {name_key} {s:?}; one of {}", TankKind::ALL.iter().map(|k| k.name()).collect::<Vec<_>>().join(", "))
                }),
                (None, Some(row)) => Ok(Some(row as i32)),
                (None, None) => Ok(None),
            }
        };
        if let Some(row) = chassis("tank", &params.tank, "tank_row", params.tank_row)? {
            game.player_row_override = Some(row);
        }
        if let Some(row) = chassis("tank2", &params.tank2, "tank2_row", params.tank2_row)? {
            game.player2_row_override = Some(row);
        }
        if let Some(n) = params.players {
            game.players =
                PlayerCount::from_count(n as usize).ok_or_else(|| format!("players must be 1 to {}, got {n}", crate::MAX_SEATS))?;
        }
        use clap::ValueEnum;
        if let Some(m) = &params.mission {
            game.level_overrides.mission = Some(Mission::from_str(m, true).map_err(|e| format!("mission: {e}"))?);
        }
        if let Some(k) = &params.spawn {
            game.level_overrides.spawn = Some(SpawnKind::from_str(k, true).map_err(|e| format!("spawn: {e}"))?);
        }
        if let Some(t) = &params.tier_start {
            game.level_overrides.tier_start = Some(Tier::from_str(t, true).map_err(|e| format!("tier_start: {e}"))?);
        }
        if let Some(t) = &params.tier_end {
            game.level_overrides.tier_end = Some(Tier::from_str(t, true).map_err(|e| format!("tier_end: {e}"))?);
        }
        if let Some(n) = params.waves {
            game.level_overrides.waves = Some(n);
        }
        if let Some(n) = params.wave_size {
            game.level_overrides.wave_size = Some(n);
        }
        if let Some(n) = params.wave_growth {
            game.level_overrides.wave_growth = Some(n);
        }
        // Headless default: no frozen intro, so `step` counts are play
        // frames. Ask for it to screenshot the banner.
        game.show_intro = params.intro.unwrap_or(false);
        // A restart is a play-mode thing: a builder session ends here, and
        // a new map replaces the builder's canvas as well as the round's.
        if let Some(map) = map_param(params.map.as_deref(), params.map_toml.as_deref())? {
            session.replace_map(map);
        }
        session.driver = Driver::Play;
        session.players_dialog = false;
        // The field is the map's, which a new map may have just changed.
        let (width, height) = session.game.map.field_size();
        session.game.init(width, height);
        self.round_started(session);
        Ok(self.status(session, width, height))
    }

    /// `weather`: the sky on screen, and with `name` that sky put on the
    /// round's map and the round started over under it on its own seed -
    /// a sky is settled when a round starts, since the rules read it
    /// (docs/weather.md) - frozen in lockstep as `restart` leaves it. A
    /// room's round is fought under the room's sky, so an online window
    /// only reports.
    fn weather(&mut self, session: &mut Session, params: &tools::WeatherParams) -> Result<Value, String> {
        if let Some(sky) = weather_name(params.name.as_ref())? {
            if session.mode() == Driver::Online {
                return Err("weather: this window holds a seat in a room, whose round is fought under the room's map's sky - \
                            only reporting works here"
                    .into());
            }
            let game = &mut session.game;
            if params.restart.unwrap_or(false) {
                game.map.weather = sky;
                let pinned = game.seed_override.replace(game.round_seed());
                let (width, height) = game.map.field_size();
                game.init(width, height);
                game.seed_override = pinned;
                self.round_started(session);
            } else {
                game.change_weather(sky);
            }
        }
        Ok(weather_json(session.shown()))
    }

    /// A round began outside `advance` - `restart`, `play`, or a click or
    /// Tab that pressed PLAY: bank its `round_started` event, start the
    /// history ring over and hold it still in lockstep, so no wall-clock
    /// frames slip in before the first `step` and a repro starts here.
    fn round_started(&mut self, session: &Session) {
        self.drain_events(&session.game, None);
        self.record_history(&session.game);
        self.lockstep = true;
    }

    /// The tools that address the whole session rather than the round:
    /// `status` (which reports the mode), the lockstep switches, `restart`,
    /// and the mode/builder tools of docs/game-editor-fusion.md section 11.
    /// `None` for a method that is not one of these.
    fn dispatch_session(
        &mut self,
        session: &mut Session,
        method: &str,
        params: &Value,
        width: f32,
        height: f32,
    ) -> Option<Result<Value, String>> {
        let result = match method {
            "status" => parse::<tools::StatusParams>(params)
                .and_then(|p| self.status_with(session, width, height, p.sections.as_deref().unwrap_or_default())),
            "pause" => {
                self.lockstep = true;
                Ok(self.status(session, width, height))
            }
            "resume" => {
                self.lockstep = false;
                session.game.paused = false;
                Ok(self.status(session, width, height))
            }
            "restart" => parse::<tools::RestartParams>(params).and_then(|p| self.restart(session, &p)),
            "weather" => parse::<tools::WeatherParams>(params).and_then(|p| self.weather(session, &p)),
            "camera" => parse::<tools::CameraParams>(params).and_then(|p| self.pin_camera(&p, session, (width, height))),
            "lint" => parse::<tools::LintParams>(params).and_then(|p| lint_json(session, &p)),
            "mode" => {
                let mut mode = mode_json(session);
                mode["host"] = self.host_json();
                Ok(mode)
            }
            "lang" => parse::<tools::LangParams>(params).map(|p| {
                if let Some(tag) = p.tag.as_deref() {
                    crate::text::set_language(tag);
                }
                json!({
                    "language": crate::text::language(),
                    "shipped": crate::text::SHIPPED_LANGS.iter().map(|(tag, _)| *tag).collect::<Vec<_>>(),
                })
            }),
            "build" => {
                session.press_build();
                Ok(mode_json(session))
            }
            "back" => {
                session.back();
                Ok(mode_json(session))
            }
            "players" => parse::<tools::PlayersParams>(params).and_then(|p| match p.count {
                None => {
                    session.press_players();
                    Ok(mode_json(session))
                }
                Some(n) => {
                    let count =
                        PlayerCount::from_count(n as usize).ok_or_else(|| format!("count must be 1 to {}, got {n}", crate::MAX_SEATS))?;
                    let before = session.game.players;
                    if !session.players_dialog {
                        session.press_players();
                    }
                    session.answer_players(count);
                    if session.game.players != before {
                        self.round_started(session);
                    }
                    Ok(mode_json(session))
                }
            }),
            "play" => parse::<tools::PlayParams>(params).and_then(|p| {
                if session.mode() == Driver::Play {
                    Err("already in play mode: `restart` starts a fresh round here, `build` enters the builder".to_string())
                } else {
                    session.game.show_intro = p.intro.unwrap_or(false);
                    session.play();
                    self.round_started(session);
                    Ok(self.status(session, width, height))
                }
            }),
            "builder_tool" => parse::<tools::BuilderToolParams>(params).and_then(|p| {
                let (tool, shape) = (tool_named(p.tool.as_deref())?, shape_named(p.shape.as_deref())?);
                brush_from_params(&mut session.builder, tool, shape);
                Ok(tool_json(&session.builder))
            }),
            "builder_paint" => parse::<tools::BuilderPaintParams>(params).and_then(|p| {
                if p.cells.is_empty() {
                    return Err("cells must hold at least one [col, row] pair".to_string());
                }
                let cells: Vec<(i32, i32)> = p.cells.iter().map(|c| (c[0] as i32, c[1] as i32)).collect();
                let (right, tool, shape) = (right_button(p.button.as_deref())?, tool_named(p.tool.as_deref())?, shape_named(p.shape.as_deref())?);
                brush_from_params(&mut session.builder, tool, shape);
                let changes = session.builder.stroke(&cells, right);
                Ok(json!({
                    "changes": changes_json(&changes),
                    "undo_depth": session.builder.history().undo_depth(),
                    "message": session.builder.status(),
                }))
            }),
            "builder_undo" | "builder_redo" => parse::<tools::StepsParams>(params).and_then(|p| {
                let steps = match p.steps {
                    None => 1,
                    Some(n) if n >= 1 => n,
                    Some(n) => return Err(format!("steps must be a positive integer, got {n}")),
                };
                let undo = method == "builder_undo";
                let mut done = 0;
                for _ in 0..steps {
                    if !(if undo { session.builder.undo() } else { session.builder.redo() }) {
                        break;
                    }
                    done += 1;
                }
                let history = session.builder.history();
                let last = match (done, undo) {
                    (0, _) => None,
                    (_, true) => history.last_undone(),
                    (_, false) => history.last_done(),
                };
                let mut v = json!({
                    "undo_depth": history.undo_depth(),
                    "redo_depth": history.redo_depth(),
                    "cells": changes_json(&last.map(|s| s.cells()).unwrap_or_default()),
                });
                v[if undo { "undone" } else { "redone" }] = json!(done);
                Ok(v)
            }),
            "builder_settings" => parse::<tools::BuilderSettingsParams>(params).and_then(|p| builder_settings(session, &p)),
            "builder_map" => parse::<tools::BuilderMapParams>(params).and_then(|p| {
                let loads_one = p.map.is_some() || p.map_toml.is_some();
                if p.clear.unwrap_or(false) {
                    if p.name.is_some() || loads_one {
                        return Err("clear takes no map: give one of name, map, map_toml or clear".to_string());
                    }
                    session.builder.clear();
                } else if let Some(name) = &p.name {
                    if loads_one {
                        return Err("give one of name, map or map_toml".to_string());
                    }
                    session.builder.load_named(name)?;
                } else if let Some(map) = map_param(p.map.as_deref(), p.map_toml.as_deref())? {
                    session.builder.load(map);
                }
                builder_map_json(&session.builder)
            }),
            "builder_camera" => parse::<tools::BuilderCameraParams>(params).map(|p| {
                let b = &mut session.builder;
                if p.fit.unwrap_or(false) {
                    b.fit_camera();
                } else {
                    let (x, y, zoom) = (p.x.map(|v| v as f32), p.y.map(|v| v as f32), p.zoom.map(|v| v as f32));
                    if x.is_some() || y.is_some() || zoom.is_some() {
                        let vp = b.viewport();
                        let center = b.camera().center(&vp);
                        let zoom = zoom.unwrap_or(b.camera().scale(&vp) / vp.fit_scale());
                        b.frame_camera(Vec2::new(x.unwrap_or(center.x), y.unwrap_or(center.y)), zoom);
                    }
                }
                builder_camera_json(&session.builder)
            }),
            "builder_touch" => {
                if session.mode() != Driver::Build {
                    Err("builder_touch needs build mode - call `build` first".to_string())
                } else {
                    // Fingers on the glass: the hints turn to taps.
                    self.hints = Some(crate::hud::Hints::Touch);
                    let frame = self.builder_frame(session, width, height);
                    parse::<tools::BuilderTouchParams>(params).and_then(|p| {
                        let hold = p.hold.unwrap_or(false);
                        let (frames, dt) = touch_frames(&p)?;
                        // Fingers a held call left down are still down.
                        let mut down = !self.held_touches.is_empty();
                        let mut last = Vec::new();
                        let lift = (!hold).then(Vec::new);
                        for touches in frames.into_iter().chain(lift) {
                            let now = !touches.is_empty();
                            let input = BuilderInput {
                                pointer: touches.first().map(|t| t.pos),
                                pressed: now && !down,
                                held: now,
                                touches: touches.clone(),
                                dt,
                                ..BuilderInput::default()
                            };
                            session.update_builder(&input, &frame);
                            down = now;
                            last = touches;
                        }
                        self.held_touches = if hold { last } else { Vec::new() };
                        let mut reply = mode_json(session);
                        reply["camera"] = builder_camera_json(&session.builder);
                        reply["held"] = json!(self.held_touches.len());
                        Ok(reply)
                    })
                }
            }
            "builder_select" => parse::<tools::BuilderSelectParams>(params).and_then(|p| builder_select(&mut session.builder, &p)),
            "builder_stamp" => parse::<tools::BuilderStampParams>(params).and_then(|p| builder_stamp(&mut session.builder, &p)),
            "builder_files" => Ok(json!({
                "maps": crate::map::available_maps(),
                "can_save": crate::map::saving_available(),
            })),
            "builder_save" => parse::<tools::BuilderSaveParams>(params)
                .and_then(|p| session.builder.save(p.name.as_deref()).and_then(|_| builder_map_json(&session.builder))),
            "click" => parse::<tools::ClickParams>(params).and_then(|p| self.click(session, &p, width, height)),
            "key" => parse::<tools::KeyParams>(params).and_then(|p| self.key(session, &p, width, height)),
            _ => return None,
        };
        Some(result)
    }

    /// `click`: one press (and optionally a drag) at a window position,
    /// through the same hit-tests `app.rs` runs on the mouse: the corners'
    /// buttons in UI points, the dialogs and the builder in the bitmap's
    /// pixels, each through the frame the window last drew
    /// (`click_frame`).
    fn click(&mut self, session: &mut Session, params: &tools::ClickParams, width: f32, height: f32) -> Result<Value, String> {
        let (x, y) = (params.x as f32, params.y as f32);
        let right = right_button(params.button.as_deref())?;
        let drag_to = params.drag_to.map(|[dx, dy]| Vec2::new(dx as f32, dy as f32));
        // A tap rather than a mouse press: the hints turn to taps.
        if params.touch.unwrap_or(false) {
            self.hints = Some(crate::hud::Hints::Touch);
        }
        let (layout, view, ui) = self.click_frame(session, width, height);
        let window_point = Vec2::new(x, y);
        // In the bitmap's pixels for the builder, in UI points for
        // everything play, the lobby and a room draw over the world.
        let point = view.to_bitmap(window_point);
        let p = ui.to_ui(window_point);
        let corner = Self::corners(session, &ui).and_then(|corners| corners.hit(p));
        match session.mode() {
            Driver::Play => {
                // The same order as `app.rs`: the level select or an
                // open dialog eats every press while it is up, then the
                // end screen, then the corners' buttons.
                if session.question.is_some() {
                    let level = (session.game.map.name.clone(), session.game.frame());
                    session.press_question(p, ui.area);
                    if (session.game.map.name.clone(), session.game.frame()) != level {
                        self.round_started(session);
                    }
                } else if session.level_select.is_some() {
                    let input = SelectInput { pointer: Some(p), pressed: !right, ..SelectInput::default() };
                    if session.update_level_select(&input, ui.area) {
                        self.round_started(session);
                    }
                } else if session.players_dialog {
                    let rects = players_dialog_rects(ui.area);
                    let before = session.game.players;
                    if rects.one.contains(p) {
                        session.answer_players(PlayerCount::ONE);
                    } else if rects.two.contains(p) {
                        session.answer_players(PlayerCount::TWO);
                    } else if !rects.panel.contains(p) {
                        session.close_players_dialog();
                    }
                    if session.game.players != before {
                        self.round_started(session);
                    }
                } else if !right && session.press_result_at_once(p, ui.area) {
                    // A level's end screen: PLAY AGAIN or the way on start
                    // a round; LEVELS opens the level select over this one.
                    if session.level_select.is_none() {
                        self.round_started(session);
                    }
                } else if !right && session.level_button().is_some() && corner == Some(CornerButton::Level) {
                    session.press_levels();
                } else if corner == Some(CornerButton::Build) {
                    session.press_build();
                } else if crate::TWO_PLAYERS_AVAILABLE && corner == Some(CornerButton::Players) {
                    session.press_players();
                } else if crate::ONLINE_AVAILABLE && corner == Some(CornerButton::Online) {
                    session.press_online();
                } else if !right && corner == Some(CornerButton::Pause) {
                    // What the P key's `Input::pause_pressed` does at the
                    // top of the next update, here at the frame boundary.
                    session.game.paused = !session.game.paused;
                } else if !crate::KEYBOARD_AVAILABLE && corner == Some(CornerButton::Restart) {
                    crate::tuning::request_restart();
                }
            }
            // The lobby's own hit tests, on the same `LobbyInput`
            // `app.rs` fills: a tool's click lands where a finger does.
            Driver::Lobby => {
                let input = crate::lobby::LobbyInput { pointer: Some(p), pressed: !right, ..crate::lobby::LobbyInput::default() };
                session.update_lobby(&input, ui.area, crate::PHYSICS_FIXED_DT);
            }
            // An online round is the room's: the corners carry the one
            // button that is this window's to press, and the round itself
            // is left to the keyboard and the touch scheme.
            Driver::Online => {
                if !right && corner == Some(CornerButton::Leave) {
                    session.leave_online();
                }
            }
            Driver::Build => {
                // The builder takes the window's own coordinates, its frame
                // putting each point on the canvas or on its chrome.
                let frame = BuilderFrame { layout, view, ui };
                let press = BuilderInput {
                    pointer: Some(window_point),
                    pressed: !right,
                    held: !right,
                    right_pressed: right,
                    right_held: right,
                    ..BuilderInput::default()
                };
                let action = session.update_builder(&press, &frame);
                let mut last = window_point;
                if let Some(to) = drag_to {
                    // Steps of about `CLICK_DRAG_STEP_PX` on the canvas.
                    let span = view.to_bitmap(window_point).distance_to(view.to_bitmap(to));
                    let steps = (span / CLICK_DRAG_STEP_PX).ceil().max(1.0) as usize;
                    for i in 1..=steps {
                        let t = i as f32 / steps as f32;
                        last = Vec2::new(window_point.x + (to.x - window_point.x) * t, window_point.y + (to.y - window_point.y) * t);
                        let held = BuilderInput { pointer: Some(last), held: !right, right_held: right, ..BuilderInput::default() };
                        session.update_builder(&held, &frame);
                    }
                }
                session.update_builder(&BuilderInput { pointer: Some(last), ..BuilderInput::default() }, &frame);
                // The press may have been PLAY, a new round; BACK comes
                // back to the paused one, which carries on as it was.
                if session.mode() == Driver::Play && action != EditorAction::Back {
                    self.round_started(session);
                }
            }
        }
        let mut reply = mode_json(session);
        // Where the press landed in the world, through the camera the
        // window last drew with (`Camera::to_world`): on a followed field
        // map that is wherever the view stood.
        if let Some(drawn) = &self.drawn
            && session.mode() != Driver::Build
            && drawn.layout.field.contains(point)
        {
            let at = drawn.layout.to_field(point);
            // On a couch's split screen, past the divider is the second
            // half's view.
            let camera = match drawn.follow.and_then(|f| f.shot.split.zip(f.second)) {
                Some((split, second)) if split.in_second(at) => second,
                _ => drawn.camera,
            };
            let world = camera.to_world(at);
            reply["world"] = json!({ "x": world.x, "y": world.y });
        }
        // In the builder, the world point and the cell under the press
        // through the builder's own camera.
        let frame = BuilderFrame { layout, view, ui };
        if session.mode() == Driver::Build
            && let Some(world) = session.builder.world_at(window_point, &frame)
        {
            reply["world"] = json!({ "x": world.x, "y": world.y });
            reply["cell"] = json!(session.builder.cell_at(window_point, &frame).map(|(c, r)| [c, r]));
        }
        Ok(reply)
    }

    /// `key`: one key for one frame, or typed text, through the same
    /// paths `app.rs` takes for the keyboard.
    fn key(&mut self, session: &mut Session, params: &tools::KeyParams, width: f32, height: f32) -> Result<Value, String> {
        let (layout, view, ui) = self.click_frame(session, width, height);
        let frame = BuilderFrame { layout, view, ui };
        let key = params.key.as_deref();
        if let Some(k) = key
            && !KEY_NAMES.contains(&k)
        {
            return Err(format!("unknown key {k:?}; one of {}", KEY_NAMES.join("|")));
        }
        let text = params.text.clone().unwrap_or_default();
        if key.is_none() && text.is_empty() {
            return Err(format!("key needs `key` ({}) or `text`", KEY_NAMES.join("|")));
        }
        // A key pressed: the hints turn back to the keys.
        self.hints = Some(crate::hud::Hints::Keys);
        match session.mode() {
            // The level select's keys, as `app.rs` reads them: the arrows
            // walk the open tiles, Enter starts one, Esc and Tab close it.
            Driver::Play if session.level_select.is_some() => {
                let input = SelectInput {
                    left: key == Some("left"),
                    right: key == Some("right"),
                    up: key == Some("up"),
                    down: key == Some("down"),
                    enter: key == Some("enter"),
                    escape: matches!(key, Some("escape") | Some("tab")),
                    ..SelectInput::default()
                };
                if session.update_level_select(&input, ui.area) {
                    self.round_started(session);
                }
            }
            Driver::Play if session.players_dialog => {
                let before = session.game.players;
                match key {
                    Some("1") => {
                        session.answer_players(PlayerCount::ONE);
                    }
                    Some("2") => {
                        session.answer_players(PlayerCount::TWO);
                    }
                    Some("enter") => {
                        let other = if before == PlayerCount::ONE { PlayerCount::TWO } else { PlayerCount::ONE };
                        session.answer_players(other);
                    }
                    Some("escape") | Some("tab") => session.close_players_dialog(),
                    _ => {}
                }
                if session.game.players != before {
                    self.round_started(session);
                }
            }
            Driver::Play => match key {
                Some("tab") => {
                    session.press_build();
                }
                // Esc opens the level select.
                Some("escape") => {
                    session.press_levels();
                }
                // A level's end screen takes Enter, as `app.rs` does.
                Some("enter") => {
                    if session.enter_result_at_once() {
                        self.round_started(session);
                    }
                }
                // undo/redo/backspace, 1/2 and typed text mean nothing in play.
                _ => {}
            },
            // The lobby takes typed characters for the code entry and
            // the same three keys `app.rs` reads.
            Driver::Lobby => {
                let input = crate::lobby::LobbyInput {
                    typed: text,
                    backspace: key == Some("backspace"),
                    enter: key == Some("enter"),
                    escape: key == Some("escape"),
                    ..crate::lobby::LobbyInput::default()
                };
                session.update_lobby(&input, ui.area, crate::PHYSICS_FIXED_DT);
            }
            // The one key an online round answers, the same one `app.rs`
            // reads: Esc gives the seat up and comes back to the local
            // round. Starting the round is the lobby's `START`.
            Driver::Online => {
                if key == Some("escape") {
                    session.leave_online();
                }
            }
            Driver::Build => {
                if key == Some("tab") {
                    session.toggle();
                    if session.mode() == Driver::Play {
                        self.round_started(session);
                    }
                } else {
                    let arrow = |less: &str, more: &str| (key == Some(more)) as i32 as f32 - (key == Some(less)) as i32 as f32;
                    let input = BuilderInput {
                        escape: key == Some("escape"),
                        enter: key == Some("enter"),
                        backspace: key == Some("backspace"),
                        undo: key == Some("undo"),
                        redo: key == Some("redo"),
                        copy: key == Some("copy"),
                        cut: key == Some("cut"),
                        paste: key == Some("paste"),
                        delete: key == Some("delete"),
                        zoom_in: key == Some("zoom_in"),
                        zoom_out: key == Some("zoom_out"),
                        pan_keys: Vec2::new(arrow("left", "right"), arrow("up", "down")),
                        dt: crate::PHYSICS_FIXED_DT,
                        typed: text,
                        ..BuilderInput::default()
                    };
                    session.update_builder(&input, &frame);
                }
            }
        }
        Ok(mode_json(session))
    }
}

/// `lint`: the map linter over the builder's canvas or the round's map,
/// each set up as a fresh headless round the way PLAY would (the session's
/// seed, player count and overrides), since `maplint::lint` reads the
/// terrain a `Game::init` laid out.
fn lint_json(session: &Session, params: &tools::LintParams) -> Result<Value, String> {
    // A map given by path or as text is linted as it is, loaded nowhere.
    let given = map_param(params.map.as_deref(), params.map_toml.as_deref())?;
    let source = match (params.source.as_deref(), &given) {
        (Some(_), Some(_)) => return Err("give a source (builder|round) or a map (map|map_toml), not both".to_string()),
        (None, Some(_)) if params.map.is_some() => "file",
        (None, Some(_)) => "text",
        (None, None) if session.mode() == Driver::Build => "builder",
        (None, None) => "round",
        (Some(s @ ("builder" | "round")), None) => s,
        (Some(other), None) => return Err(format!("source must be builder|round, got {other:?}")),
    };
    // In an online round the map to lint is the room's, as the replica
    // was built from it.
    let live = session.shown();
    let map = match &given {
        Some(map) => map,
        None if source == "builder" => session.builder.map(),
        None => &live.map,
    };
    let (game, findings) = crate::maplint::lint_map(map, &crate::maplint::LintSetup::of(live));
    let count = |severity: LintSeverity| findings.iter().filter(|f| f.severity == severity).count();
    Ok(json!({
        "source": source,
        "map": map_json(&game.map),
        "seed": format!("{:#x}", game.round_seed()),
        "players": game.players.count(),
        "errors": count(LintSeverity::Error),
        "warnings": count(LintSeverity::Warning),
        "infos": count(LintSeverity::Info),
        "findings": findings.iter().map(lint_finding_json).collect::<Vec<_>>(),
    }))
}

/// `status.builder.check`: the builder's CHECK panel - whether it is
/// open, its last report (counts, findings as the `lint` tool spells them)
/// and whether the map has moved on since it ran, and the finding the
/// canvas marks. `null` before the panel first opened.
fn builder_check_json(b: &MapEditor) -> Value {
    let Some((report, stale)) = b.lint_report() else { return Value::Null };
    json!({
        "open": b.open_menu() == Some("check"),
        "stale": stale,
        "errors": report.count(LintSeverity::Error),
        "warnings": report.count(LintSeverity::Warning),
        "infos": report.count(LintSeverity::Info),
        "findings": report.findings.iter().map(lint_finding_json).collect::<Vec<_>>(),
        "marked": b.lint_marked().map(lint_finding_json),
    })
}

/// `status.builder.clear`: the clear check (docs/large-maps-patterns.md,
/// "Clear check before sharing") - the canvas's `revision` as a map file's
/// stamp spells it, whether that revision is `cleared` (won from plain
/// PLAY with no edit since) and its `par` in seconds (`null` until it is),
/// and `attempt`, the revision the local round's PLAY started on (`null`
/// for a round PLAY did not start - a level, PLAY HERE, a restart on
/// another map).
fn builder_clear_json(session: &Session) -> Value {
    let b = &session.builder;
    json!({
        "revision": crate::map::revision_text(b.revision()),
        "cleared": b.par().is_some(),
        "par": b.par(),
        "attempt": session.clear_attempt.map(crate::map::revision_text),
    })
}

/// One finding as the `lint` tool and `status.builder.check` spell it: its
/// severity, kind and message, the map cells and the nav cells it is
/// about, and its quick fix, if it has one.
fn lint_finding_json(f: &crate::maplint::LintFinding) -> Value {
    use crate::maplint::{LintCell, LintFix};
    let map: Vec<[i32; 2]> = f.cells.iter().filter_map(|c| if let LintCell::Map(col, row) = *c { Some([col, row]) } else { None }).collect();
    let nav: Vec<[usize; 2]> = f.cells.iter().filter_map(|c| if let LintCell::Nav(col, row) = *c { Some([col, row]) } else { None }).collect();
    let fix = f.fix.map(|fix| match fix {
        LintFix::Move { from, to } => json!({ "move": { "from": [from.0, from.1], "to": [to.0, to.1] } }),
        LintFix::Place { object, at } => json!({ "place": { "object": object, "at": [at.0, at.1] } }),
        LintFix::Remove { at } => json!({ "remove": { "at": [at.0, at.1] } }),
    });
    json!({ "severity": f.severity.to_string(), "kind": f.kind.tag(), "message": f.message, "cells": map, "nav_cells": nav, "fix": fix })
}

/// `terrain`: every live tile plus the fire layer - see the tool's
/// description for the shape. Tiles are sorted by row then column, quiet
/// fields left out, so an untouched wall is one short line.
fn terrain_json(game: &Game, params: &tools::TerrainParams) -> Result<Value, String> {
    let only = params.only.as_deref().unwrap_or("all");
    if !["all", "damaged", "burning", "fused"].contains(&only) {
        return Err(format!("only must be all|damaged|burning|fused, got {only:?}"));
    }
    let materials: &[String] = params.materials.as_deref().unwrap_or_default();
    let material_name = |m: crate::obstacle::Material| to_value(m).as_str().unwrap_or("?").to_string();
    let cell_of = |p: Position| {
        let (c, r) = crate::map::world_to_cell(p);
        json!([c, r])
    };
    let mut live: Vec<(i32, i32, &Obstacle)> = Vec::new();
    let mut query = game.world.query::<&Obstacle>();
    for o in query.iter().filter(|o| !o.destroyed) {
        let (c, r) = crate::map::world_to_cell(o.position);
        live.push((r, c, o));
    }
    live.sort_by_key(|(r, c, _)| (*r, *c));
    let total = live.len();
    let mut tiles = Vec::new();
    let mut truncated = false;
    for (r, c, o) in live {
        let material = material_name(o.material);
        if !materials.is_empty() && !materials.contains(&material) {
            continue;
        }
        let fused = o.fuse.is_some();
        let damaged = o.health < o.max_health || o.burning || fused || o.scorched != 0 || o.heat > 0.0 || o.ram_timer > 0.0;
        let keep = match only {
            "damaged" => damaged,
            "burning" => o.burning,
            "fused" => fused,
            _ => true,
        };
        if !keep {
            continue;
        }
        if tiles.len() == TERRAIN_MAX_TILES {
            truncated = true;
            break;
        }
        let mut tile = json!({
            "cell": [c, r],
            "x": r1(o.position.x),
            "y": r1(o.position.y),
            "material": material,
            "hp": r1(o.health),
            "max_hp": r1(o.max_health),
            "flammable": o.flammable,
        });
        if let Some(drum) = o.drum() {
            tile["drum"] = json!(drum.name());
        }
        if o.burning {
            tile["burning"] = json!(true);
            tile["burn_elapsed"] = json!(r1(o.burn_elapsed));
        }
        if let Some(fuse) = &o.fuse {
            tile["fuse"] = json!({ "left": r1(fuse.left), "total": r1(fuse.total) });
        }
        if o.heat > 0.0 {
            tile["heat"] = json!(r1(o.heat));
        }
        if o.scorched != 0 {
            tile["scorched"] = json!(o.scorched);
        }
        if o.ram_timer > 0.0 {
            tile["ram_timer"] = json!(r1(o.ram_timer));
        }
        tiles.push(tile);
    }
    let fires: Vec<Value> = game
        .fires
        .iter()
        .map(|f| json!({ "cell": [f.cell.0, f.cell.1], "left": r1(f.left), "total": r1(f.total), "pool": f.pool }))
        .collect();
    let flames: Vec<Value> = game
        .flames()
        .iter()
        .map(|j| {
            json!({
                "slot": j.owner.slot(),
                "x": r1(j.origin.x),
                "y": r1(j.origin.y),
                "dx": r1(j.dir.x),
                "dy": r1(j.dir.y),
                "range": r1(j.range),
                "reach": r1(j.reach),
            })
        })
        .collect();
    let positions_with = |items: Vec<(Position, f32)>, key: &str| -> Vec<Value> {
        items.into_iter().map(|(p, v)| json!({ "x": r1(p.x), "y": r1(p.y), key: r1(v) })).collect()
    };
    Ok(json!({
        "frame": game.frame(),
        "total": total,
        "count": tiles.len(),
        "truncated": truncated,
        "tiles": tiles,
        "fires": fires,
        "fused": game.fused_barrels().into_iter().map(cell_of).collect::<Vec<_>>(),
        "burning_tiles": game.burning_tiles().len(),
        "flames": flames,
        "burning_tanks": positions_with(game.burning_tanks(), "left"),
        "burning_wrecks": positions_with(game.burning_wrecks(), "age"),
        "flying_drums": game.flying_drums.len(),
        "portals": game.portals().iter().map(|p| json!({ "x": r1(p.x), "y": r1(p.y), "cell": cell_of(*p) })).collect::<Vec<_>>(),
        "portals_active": game.portals_active(),
        "oil_cells": game.oil_cells.len(),
        "oil_prints": game.oil_prints.len(),
        "grass_cells": game.grass_cells.len(),
        "grass_trampled": game.grass_trampled.iter().map(|&(c, r)| json!([c, r])).collect::<Vec<_>>(),
        "hot_cells": game.heat.len(),
    }))
}

/// `mode`'s reply: the session's mode and the builder's state in one look.
/// The level the local round is (docs/levels.md) - its number, map and
/// title, and the furthest one reached - or `null` in free play.
/// `status.question`: the question about a kept map (BB-33) - `revert` or
/// `original_changed` and the map - `null` when none is asked; a `click`
/// on `status.ui.screen_buttons`' `yes`/`no` answers it.
fn question_json(session: &Session) -> Value {
    session.question.as_ref().map_or(Value::Null, |q| {
        json!({
            "kind": match q {
                crate::mapstore::Question::Revert { .. } => "revert",
                crate::mapstore::Question::OriginalChanged { .. } => "original_changed",
            },
            "map": q.name(),
        })
    })
}

fn level_json(session: &Session) -> Value {
    let (Some(i), Some(campaign)) = (session.level(), session.campaign.as_ref()) else { return Value::Null };
    let Some(level) = campaign.levels.get(i) else { return Value::Null };
    json!({
        "number": campaign.levels.number(i),
        "count": campaign.levels.last_number(),
        "map": level.map,
        "title": level.title(),
        "reached": campaign.levels.number(campaign.reached()),
        "last": campaign.is_last(i),
    })
}

fn mode_json(session: &Session) -> Value {
    let b = &session.builder;
    json!({
        "mode": session.mode().name(),
        "back": back_json(session),
        "players_dialog_open": session.players_dialog,
        "levels_open": session.level_select.is_some(),
        "levels_focus": session.level_select.as_ref().zip(session.campaign.as_ref()).map(|(select, campaign)| campaign.levels.number(select.focus())),
        "players": session.game.players.count(),
        "dirty": b.dirty(),
        "map_name": b.name(),
        "tool": b.tool().name(),
        "category": b.active_category().map(category_name),
        "open_menu": b.open_menu(),
        "undo_depth": b.history().undo_depth(),
        "redo_depth": b.history().redo_depth(),
        "level": level_json(session),
    })
}

/// BACK in the builder (`mode::Session::back`): `null` where there is no
/// paused round to go back to, and in every mode but build; else whether
/// it would put edits away (`MapEditor::off_round`).
fn back_json(session: &Session) -> Value {
    let b = &session.builder;
    if session.mode() == Driver::Build && b.can_go_back() { json!({ "puts_away": b.off_round() }) } else { Value::Null }
}

/// Which round every reading tool is describing: the session's own, or
/// the replica of a room's round. An online round names the room, the
/// seat, how deep the snapshot buffer is and how far the server had got,
/// so a `status` or a `snapshot` is never mistaken for the local round's.
fn round_json(session: &Session) -> Value {
    let round = match session.mode() {
        Driver::Online => session.online.as_ref(),
        _ => None,
    };
    let Some(round) = round else {
        return json!({ "kind": "local" });
    };
    let mut out = round.stats_json();
    if let Some(map) = out.as_object_mut() {
        map.insert("kind".into(), json!("online"));
        map.insert("room".into(), json!(round.code()));
        map.insert("phase".into(), json!(phase_name(round.phase())));
        // What the interpolation delay is buying, rounded to the
        // millisecond: negative once the picture has run past everything
        // that arrived, null before the first snapshot.
        map.insert("buffer_ms".into(), json!(round.buffer_ms().map(|ms| ms.round() as i64)));
        // False between taking the seat and the room's `Welcome`: until
        // then the window still draws the local round.
        map.insert("replica".into(), json!(round.game().is_some()));
        map.remove("tanks");
    }
    out
}

/// How far along the seat is, as one word.
fn phase_name(phase: &Phase) -> &'static str {
    match phase {
        Phase::Connecting => "connecting",
        Phase::Greeting => "greeting",
        Phase::Lobby => "lobby",
        Phase::Playing => "playing",
        Phase::Closed(_) => "closed",
    }
}

/// The category's name as the tools spell it (`wall`, not `WALL`).
fn category_name(category: Category) -> String {
    category.label().to_ascii_lowercase()
}

/// `builder_tool`'s reply: the active tool and every category's list.
fn tool_json(b: &MapEditor) -> Value {
    let categories: Vec<Value> = Category::ALL
        .iter()
        .map(|&c| {
            json!({
                "name": category_name(c),
                "current": b.current_tool(c).name(),
                "tools": c.tools().map(Tool::name).collect::<Vec<_>>(),
            })
        })
        .collect();
    json!({ "tool": b.tool().name(), "shape": b.shape().name(), "category": b.active_category().map(category_name), "categories": categories })
}

/// `status.builder.thumbnails`: the Load list's thumbnails made so far,
/// while it is open.
fn thumbnails_json(b: &MapEditor) -> Value {
    Value::Array(b.thumbnails().map(|(name, t)| json!({ "name": name, "cells": [t.cells.0, t.cells.1], "picture": t.image.is_some() })).collect())
}

/// A rectangle of cells as `status.builder` and `builder_select` spell it:
/// its top-left cell and its size, `null` for none.
fn cell_rect_json(rect: Option<CellRect>) -> Value {
    rect.map_or(Value::Null, |r| json!({ "col": r.col, "row": r.row, "cols": r.cols, "rows": r.rows }))
}

/// The paste ghost: where it stands and how many cells it carries.
fn ghost_json(b: &MapEditor) -> Value {
    match b.ghost() {
        Some(ghost) => {
            let mut v = cell_rect_json(Some(ghost.rect()));
            v["cells"] = json!(ghost.clip.cells.len());
            v
        }
        None => Value::Null,
    }
}

/// The clipboard: its size and how many cells it holds.
fn clipboard_json(b: &MapEditor) -> Value {
    b.clipboard().map_or(Value::Null, |c| json!({ "cols": c.cols, "rows": c.rows, "cells": c.cells.len() }))
}

/// `builder_select`'s reply.
fn select_json(b: &MapEditor, changes: &[CellChange]) -> Value {
    json!({
        "tool": b.tool().name(),
        "selection": cell_rect_json(b.selection()),
        "ghost": ghost_json(b),
        "clipboard": clipboard_json(b),
        "changes": changes_json(changes),
        "undo_depth": b.history().undo_depth(),
    })
}

/// `n` integers under `key`, `None` when absent.
fn builder_select(b: &mut MapEditor, params: &tools::BuilderSelectParams) -> Result<Value, String> {
    let rect = params.rect.map(|r| r.map(|v| v as i32));
    let move_by = params.move_by.map(|v| v.map(|v| v as i32));
    let at = params.at.map(|v| (v[0] as i32, v[1] as i32));
    let action = params.action.as_deref();
    let mut changes = Vec::new();
    if params.clear.unwrap_or(false) {
        b.cancel_ghost();
        b.clear_selection();
    }
    if let Some(r) = rect
        && !b.select_cells(CellRect::spanning((r[0], r[1]), (r[2], r[3])))
    {
        return Err(format!("rect {r:?} is off the map, whose cells are {:?}", b.field_cells()));
    }
    let selected = |b: &MapEditor, what: &str| if b.selection().is_some() { Ok(()) } else { Err(format!("{what} needs a selection: give `rect` first")) };
    if let Some(by) = move_by {
        selected(b, "move_by")?;
        changes.extend(b.move_selection((by[0], by[1])));
    }
    match action {
        None => {}
        Some("copy") => {
            selected(b, "copy")?;
            b.copy_selection();
        }
        Some("cut") => {
            selected(b, "cut")?;
            changes.extend(b.cut_selection());
        }
        Some("delete") => {
            selected(b, "delete")?;
            changes.extend(b.delete_selection());
        }
        Some(flip @ ("flip_h" | "flip_v")) => {
            if b.selection().is_none() && b.ghost().is_none() {
                return Err(format!("{flip} needs a selection or a paste ghost"));
            }
            changes.extend(b.flip(if flip == "flip_h" { Axis::Horizontal } else { Axis::Vertical }));
        }
        Some("stamp") => {
            selected(b, "stamp")?;
            if b.save_stamp().is_none() {
                return Err("the selection holds nothing to keep as a stamp".to_string());
            }
        }
        Some("paste") => {
            if !b.paste(at) {
                return Err("the clipboard is empty: copy or cut first".to_string());
            }
        }
        Some("place") => {
            if b.ghost().is_none() {
                return Err("no paste ghost to place: paste or builder_stamp first".to_string());
            }
            if let Some(at) = at {
                b.move_ghost(at);
            }
            changes.extend(b.place_ghost());
        }
        Some("cancel") => b.cancel_ghost(),
        Some(other) => return Err(format!("unknown action {other:?}; one of copy, cut, delete, flip_h, flip_v, stamp, paste, place, cancel")),
    }
    Ok(select_json(b, &changes))
}

/// `builder_stamp`: the STAMPS list, or one of them as the paste ghost.
fn builder_stamp(b: &mut MapEditor, params: &tools::BuilderStampParams) -> Result<Value, String> {
    let at = params.at.map(|v| (v[0] as i32, v[1] as i32));
    let mut changes = Vec::new();
    if let Some(name) = &params.name {
        let Some(index) = b.stamps().iter().position(|s| s.key() == *name) else {
            let keys: Vec<String> = b.stamps().iter().map(|s| s.key()).collect();
            return Err(format!("no stamp {name:?}; one of {}", keys.join(", ")));
        };
        b.use_stamp(index);
        if let Some(at) = at {
            b.move_ghost(at);
        }
        if params.place.unwrap_or(false) {
            changes = b.place_ghost();
        }
    }
    let stamps: Vec<Value> = b
        .stamps()
        .iter()
        .map(|s| json!({ "key": s.key(), "name": s.label(), "cols": s.clip.cols, "rows": s.clip.rows, "cells": s.clip.cells.len() }))
        .collect();
    Ok(json!({
        "stamps": stamps,
        "ghost": ghost_json(b),
        "changes": changes_json(&changes),
        "undo_depth": b.history().undo_depth(),
    }))
}

/// `builder_touch`'s frames - each a list of `{id, x, y}` touch points in
/// bitmap pixels - and its seconds per frame.
fn touch_frames(params: &tools::BuilderTouchParams) -> Result<(Vec<Vec<crate::touch::TouchPoint>>, f32), String> {
    let dt = match params.dt {
        None => PHYSICS_FIXED_DT,
        Some(s) if s.is_finite() && s >= 0.0 => s as f32,
        Some(s) => return Err(format!("dt must be seconds, 0 or more, got {s}")),
    };
    let frames = params
        .frames
        .iter()
        .map(|points| points.iter().map(|p| crate::touch::TouchPoint { id: p.id as i32, pos: Vec2::new(p.x as f32, p.y as f32) }).collect())
        .collect();
    Ok((frames, dt))
}

/// `status.builder.camera` and `builder_camera`'s reply: the builder's
/// view over the canvas area it was last given - FIT or not, the world it
/// shows, how large, and a cell's size on a touch screen's glass.
fn builder_camera_json(b: &MapEditor) -> Value {
    let vp = b.viewport();
    let camera = b.camera();
    let scale = camera.scale(&vp);
    let fit = vp.fit_scale();
    let rect = camera.view(&vp).rect();
    json!({
        "fit": camera.is_fit(),
        "rect": { "x": rect.x, "y": rect.y, "w": rect.width, "h": rect.height },
        "scale": scale,
        "zoom": scale / fit,
        "fit_scale": fit,
        "device_scale": vp.device_scale(scale),
        "cell_mm": vp.cell_mm(scale),
        "area": [vp.area.0, vp.area.1],
    })
}

/// `builder_map`'s reply: the canvas as TOML plus how it differs from
/// the baseline.
fn builder_map_json(b: &MapEditor) -> Result<Value, String> {
    let toml = b.map().to_toml_string()?;
    Ok(json!({
        "toml": toml,
        "name": b.name(),
        "cells": b.map().cells.len(),
        "dirty": b.dirty(),
        "diff": b.diff(),
        "undo_depth": b.history().undo_depth(),
    }))
}

/// `builder_settings`: apply each given field as its own undo step, in
/// field order, then an optional reset; reply with the current values.
fn builder_settings(session: &mut Session, params: &tools::BuilderSettingsParams) -> Result<Value, String> {
    fn parse_or<T>(key: &str, s: &str, parse: impl Fn(&str) -> Option<T>, valid: &[&str]) -> Result<T, String> {
        parse(s).ok_or_else(|| format!("unknown {key} {s:?}; one of {}", valid.join(", ")))
    }
    const MISSIONS: &[&str] = &["protect", "hunt", "destroy"];
    const SPAWNS: &[&str] = &["band", "waves"];
    const TIERS: &[&str] = &["light", "medium", "heavy", "super"];
    const THEMES: &[&str] = &["grass", "desert"];
    let tanks: Vec<&str> = TankKind::ALL.iter().map(|k| k.name()).collect();

    let b = &mut session.builder;
    // One `apply_settings` per field so each change is its own step (and
    // the clamped result is re-read before the next).
    let mut s = b.settings();
    if let Some(v) = params.tanks {
        s.tanks = v;
        b.apply_settings(s);
        s = b.settings();
    }
    if let Some(v) = params.tank.clone() {
        s.tank = v.map(|n| parse_or("tank", &n, parse_tank, &tanks)).transpose()?;
        b.apply_settings(s);
        s = b.settings();
    }
    if let Some(v) = params.tank2.clone() {
        s.tank2 = v.map(|n| parse_or("tank2", &n, parse_tank, &tanks)).transpose()?;
        b.apply_settings(s);
        s = b.settings();
    }
    for (key, side, given) in [("bong", Side::Player, params.bong.clone()), ("enemy_bong", Side::Enemy, params.enemy_bong.clone())] {
        if let Some(v) = given {
            let kinds: Vec<&str> = BongKind::of(side).iter().map(|k| k.name()).collect();
            let pick = |n: &str| BongKind::parse(n).filter(|k| k.side() == side);
            let kind = v.map(|n| parse_or(key, &n, pick, &kinds)).transpose()?;
            match side {
                Side::Player => s.bong = kind,
                Side::Enemy => s.enemy_bong = kind,
            }
            b.apply_settings(s);
            s = b.settings();
        }
    }
    if let Some(v) = params.mission.clone() {
        s.mission = v.map(|n| parse_or("mission", &n, parse_mission, MISSIONS)).transpose()?.unwrap_or_default();
        b.apply_settings(s);
        s = b.settings();
    }
    if let Some(v) = params.spawn.clone() {
        s.spawn = v.map(|n| parse_or("spawn", &n, parse_spawn, SPAWNS)).transpose()?.unwrap_or_default();
        b.apply_settings(s);
        s = b.settings();
    }
    if let Some(v) = params.waves {
        s.waves = v;
        b.apply_settings(s);
        s = b.settings();
    }
    if let Some(v) = params.wave_size {
        s.size = v;
        b.apply_settings(s);
        s = b.settings();
    }
    if let Some(v) = params.wave_growth {
        s.growth = v;
        b.apply_settings(s);
        s = b.settings();
    }
    if let Some(v) = params.tier_start.clone() {
        s.tier_start = v.map(|n| parse_or("tier_start", &n, parse_tier, TIERS)).transpose()?;
        b.apply_settings(s);
        s = b.settings();
    }
    if let Some(v) = params.tier_end.clone() {
        s.tier_end = v.map(|n| parse_or("tier_end", &n, parse_tier, TIERS)).transpose()?;
        b.apply_settings(s);
        s = b.settings();
    }
    if let Some(v) = params.theme.clone() {
        s.theme = v
            .map(|n| parse_or("theme", &n, crate::map::Theme::parse, THEMES))
            .transpose()?
            .unwrap_or_default();
        b.apply_settings(s);
        s = b.settings();
    }
    match &params.weather {
        None => {}
        Some(None) => {
            s.weather = crate::map::Skies::CLEAR;
            b.apply_settings(s);
        }
        Some(Some(value)) => {
            s.weather = skies_value("weather", value)?;
            b.apply_settings(s);
        }
    }
    // The anchor first, so a size in the same call is placed by it.
    if let Some(name) = params.anchor.as_deref() {
        let names: Vec<&str> = crate::editor::Anchor::ALL.iter().map(|a| a.name()).collect();
        b.set_resize_anchor(parse_or("anchor", name, crate::editor::Anchor::parse, &names)?);
    }
    if let Some([cols, rows]) = params.size {
        if !cols.is_finite() || !rows.is_finite() {
            return Err(format!("size must be [cols, rows] numbers, got [{cols}, {rows}]"));
        }
        let anchor = b.resize_anchor();
        b.resize(cols as f32, rows as f32, anchor);
    }
    if params.reset.unwrap_or(false) {
        b.reset();
    }
    Ok(settings_json(session))
}

/// The builder's settings as the tools spell them (null = auto) plus
/// which of them the round would override (`Game`'s CLI/restart values
/// outrank the map's, exactly as they outrank a file's).
fn settings_json(session: &Session) -> Value {
    let s = session.builder.settings();
    let g = &session.game;
    let size = {
        let (cols, rows) = session.builder.size_cells();
        [cols, rows]
    };
    json!({
        "tanks": s.tanks,
        "tank": s.tank.map(TankKind::name),
        "tank2": s.tank2.map(TankKind::name),
        "bong": s.bong.map(BongKind::name),
        "enemy_bong": s.enemy_bong.map(BongKind::name),
        "mission": s.mission.name(),
        "spawn": s.spawn.name(),
        "waves": s.waves,
        "wave_size": s.size,
        "wave_growth": s.growth,
        "tier_start": s.tier_start.map(Tier::name),
        "tier_end": s.tier_end.map(Tier::name),
        "theme": s.theme.name(),
        "weather": s.weather,
        "size": size,
        "anchor": session.builder.resize_anchor().name(),
        "cli_overrides": {
            "tanks": g.enemy_count_override.is_some(),
            "tank": g.player_row_override.is_some(),
            "tank2": g.player2_row_override.is_some(),
            "mission": g.level_overrides.mission.is_some(),
            "spawn": g.level_overrides.spawn.is_some(),
            "waves": g.level_overrides.waves.is_some(),
            "wave_size": g.level_overrides.wave_size.is_some(),
            "wave_growth": g.level_overrides.wave_growth.is_some(),
            "tier_start": g.level_overrides.tier_start.is_some(),
            "tier_end": g.level_overrides.tier_end.is_some(),
            "weather": crate::tuning::tuning().weather_override >= 0,
        },
    })
}

/// A stroke's changes for a reply: each cell's object before and after
/// in the map's own serialised shape (`{kind, ...}`, null = empty).
fn changes_json(changes: &[CellChange]) -> Value {
    Value::Array(
        changes.iter().map(|c| json!({ "col": c.col, "row": c.row, "before": c.before, "after": c.after })).collect(),
    )
}

/// The map a `map` path or a `map_toml` text names (`restart` and
/// `builder_map`): `None` when neither is given, an error for both.
fn map_param(path: Option<&str>, toml: Option<&str>) -> Result<Option<MapFile>, String> {
    match (path, toml) {
        (Some(_), Some(_)) => Err("give either map (a path) or map_toml (inline TOML), not both".to_string()),
        (Some(path), None) => MapFile::load(Path::new(path)).map(Some),
        (None, Some(text)) => MapFile::from_toml_str(text).map(Some).map_err(|e| format!("map_toml: {e}")),
        (None, None) => Ok(None),
    }
}

/// A builder tool by name: `None` when none is given, an error naming
/// every valid spelling for an unknown one.
fn tool_named(name: Option<&str>) -> Result<Option<Tool>, String> {
    name.map(|s| {
        Tool::parse(s).ok_or_else(|| {
            let names: Vec<&str> = crate::editor::TOOLS.iter().map(|t| t.name()).collect();
            format!("unknown tool {s:?}; one of {}", names.join(", "))
        })
    })
    .transpose()
}

/// How the brush paints, by name (`Shape::parse`).
fn shape_named(name: Option<&str>) -> Result<Option<Shape>, String> {
    name.map(|s| Shape::parse(s).ok_or_else(|| format!("unknown shape {s:?}; one of pen, rect, fill, scatter"))).transpose()
}

/// `builder_tool`'s and `builder_paint`'s brush: the tool first, then the
/// shape - which takes a painting brush back from the select tool, as
/// BRUSH's list does.
fn brush_from_params(b: &mut MapEditor, tool: Option<Tool>, shape: Option<Shape>) {
    if let Some(tool) = tool {
        b.select_tool(tool);
    }
    if let Some(shape) = shape {
        b.set_shape(shape);
    }
}

/// A mouse button by name: `true` for the secondary (right) one.
fn right_button(name: Option<&str>) -> Result<bool, String> {
    match name {
        None | Some("left") => Ok(false),
        Some("right") => Ok(true),
        Some(other) => Err(format!("button must be left|right, got {other:?}")),
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

/// `status.camera.motion`: the one motion switch (`motion.rs`) - whether
/// motion is `reduced` this frame, the `reduce_motion` row's `setting`
/// (`platform`, `off` or `on`) and what the `platform` said at startup
/// (`null` where it says nothing).
fn motion_json() -> Value {
    json!({
        "reduced": crate::motion::reduced(),
        "setting": crate::motion::setting_name(crate::tuning::tuning().reduce_motion),
        "platform": crate::motion::platform(),
    })
}

/// A followed view's half of `status.camera` (docs/large-maps-follow-camera.md
/// §5, §6): the seat it follows and how (`focus`: `seat`, `shared`,
/// `split` - a couch pair too far apart for one view, each in its half of a
/// split screen - `spectating` or `nobody`), whether this frame cut,
/// the look-ahead and the sub-block offset the picture was shifted by;
/// the framing behind its size - the world in cells, device pixels per
/// world pixel and per block, whether the zoom snapped to whole blocks,
/// the tank in points and millimetres, the bars past the aspect clamp;
/// and the sight box the view keeps, with whether all of it is on screen;
/// and the `split` (`null` for one view): the divider through `line` (a
/// point and the unit normal into the second half, in the followed
/// bitmap's pixels, and `window_line` the point on the window), how far
/// `apart` the halves' views stand, and each half's seat, world `rect`,
/// sub-block `offset`, whether it `cut` and whether its seat's sight box
/// is `in_view`, the first half - the view above - first.
fn follow_json(f: &FollowReport, camera: &Camera, view: &View, layout: &Layout) -> Value {
    let fr = &f.framing;
    let (cols, rows) = fr.visible_cells();
    let shot = &f.shot;
    let room = fr.room_outside(f.sight);
    json!({
        "seat": shot.seat,
        "focus": shot.kind.name(),
        "cut": shot.cut,
        "glide": shot.glide.map(|q| (q * 100.0).round() / 100.0),
        "lead": { "x": r1(shot.lead.x), "y": r1(shot.lead.y) },
        "offset": { "x": camera.offset.x, "y": camera.offset.y },
        "seating": match f.seating { Seating::Room => "room", Seating::Local => "local" },
        "framing": {
            "visible_cells": [cols, rows],
            "device_scale": fr.scale,
            "point_scale": fr.point_scale,
            "block_px": fr.block(),
            "snapped": fr.snapped,
            "tank_points": fr.tank_points,
            "tank_mm": fr.tank_mm,
            "bars": [fr.bars.0, fr.bars.1],
        },
        "sight_box": {
            "half": [f.sight.half.0, f.sight.half.1],
            "room": [room.0, room.1],
            "in_view": shot.boxes_in(camera.rect(), camera.field, f.sight, 1.0),
        },
        "establishing": {
            "phase": f.establishing.name(),
            "progress": (f.establishing.progress() * 100.0).round() / 100.0,
        },
        "split": match (shot.split, f.second) {
            (Some(split), Some(second)) => {
                let rect = |r: crate::math::Rectangle| json!({ "x": r.x, "y": r.y, "w": r.width, "h": r.height });
                let on_window = view.to_window(Vec2::new(layout.field.x + split.at.x, layout.field.y + split.at.y));
                json!({
                    "line": { "x": split.at.x, "y": split.at.y, "nx": split.normal.x, "ny": split.normal.y },
                    "window_line": { "x": on_window.x, "y": on_window.y },
                    "apart": r1(split.apart),
                    "halves": [
                        {
                            "seat": shot.seat,
                            "rect": rect(camera.rect()),
                            "offset": { "x": camera.offset.x, "y": camera.offset.y },
                            "cut": shot.cut,
                            "in_view": shot.boxes_in(camera.rect(), camera.field, f.sight, 1.0),
                        },
                        {
                            "seat": split.seat,
                            "rect": rect(second.rect()),
                            "offset": { "x": second.offset.x, "y": second.offset.y },
                            "cut": split.cut,
                            "in_view": split.boxes_in(second.rect(), second.field, f.sight, 1.0),
                        },
                    ],
                })
            }
            _ => Value::Null,
        },
    })
}

/// The loaded map's identity for `status`/`map_get`.
fn map_json(map: &MapFile) -> Value {
    json!({
        "name": map.name.as_deref().unwrap_or("inline"),
        "cells": map.cells.len(),
        "tanks": map.tanks,
        "theme": map.theme.name(),
        "weather": map.weather,
    })
}

/// The `weather` tool's reply: what is drawn, the map's own key, the
/// override knob's pick, and every name (docs/weather.md).
fn weather_json(game: &Game) -> Value {
    let t = crate::tuning::tuning();
    let pick = crate::weather::knob(&t);
    let sky = game.weather();
    // What the sky does to the round now (docs/weather.md "The rules"):
    // how far the enemies see, the grip wet ground leaves, whether the
    // water is ice, and the gust on player 1 and the one crossing the
    // field.
    let p1 = game.player().map(|e| crate::simulation::with_tank(&game.world, e, |tank| tank.position));
    let gust_p1 = p1.map(|at| crate::weather::gust_at(sky, at, game.time, &t)).map(|v| [v.x, v.y]);
    let (width, height) = game.map.field_size();
    let front = (t.weather_rules && sky == crate::map::Weather::Sandstorm)
        .then(|| crate::weather::gust_on_field(game.time, width, height, &t))
        .flatten()
        .map(|g| json!({ "start": g.start, "dir": [g.dir.x, g.dir.y] }));
    json!({
        "in_force": sky.name(),
        "map": game.map.weather,
        "override": pick.map(crate::map::Weather::name),
        "without_shaders": crate::weather::plain::without_shaders(&t),
        "names": crate::map::Weather::ALL.iter().map(|w| w.name()).collect::<Vec<_>>(),
        "rules": {
            "on": t.weather_rules,
            "enemy_sight_px": game.enemy_sight(),
            "grip": crate::weather::grip_factor(sky, &t),
            "frozen": game.water().is_frozen(),
            "gust_on_player": gust_p1,
            "gust_front": front,
        },
    })
}

/// The `weather` tool's `name`, if it gave one.
fn weather_name(name: Option<&Value>) -> Result<Option<crate::map::Skies>, String> {
    match name {
        None | Some(Value::Null) => Ok(None),
        Some(value) => skies_value("name", value).map(Some),
    }
}

/// A map's skies as a tool spells them: one weather's name, or a list of
/// them (`["rain", "snow"]`, `[]` for clear), as the map file does.
fn skies_value(key: &str, value: &Value) -> Result<crate::map::Skies, String> {
    match value {
        Value::String(_) | Value::Array(_) => serde_json::from_value(value.clone()).map_err(|e| format!("{key}: {e}")),
        other => Err(format!("{key} must be a weather's name or a list of them, got {other}")),
    }
}

fn overlays_json(game: &Game) -> Value {
    to_value(game.debug_overlays)
}

impl tools::OverlayFlags {
    /// Set the flags this patch names; one it leaves out keeps its value.
    fn apply(&self, o: &mut Overlays) {
        let set = |to: &mut bool, flag: Option<bool>| {
            if let Some(on) = flag {
                *to = on;
            }
        };
        set(&mut o.nav_grid, self.nav_grid);
        set(&mut o.ai, self.ai);
        set(&mut o.projectiles, self.projectiles);
        set(&mut o.engage, self.engage);
        set(&mut o.pickups, self.pickups);
        set(&mut o.hitboxes, self.hitboxes);
        set(&mut o.stats, self.stats);
    }
}

/// A snapshot's detail by name; compact when none is given.
fn detail_of(name: Option<&str>) -> Result<Detail, String> {
    match name {
        None | Some("compact") => Ok(Detail::Compact),
        Some("full") => Ok(Detail::Full),
        Some(other) => Err(format!("detail must be compact|full, got {other:?}")),
    }
}

/// A frame count within what one `step` may run; `default` when none is
/// given.
fn frames_of(frames: Option<u64>, default: u64) -> Result<u64, String> {
    match frames {
        None => Ok(default),
        Some(n) if (1..=MAX_STEP_FRAMES).contains(&n) => Ok(n),
        Some(n) => Err(format!("frames must be 1..={MAX_STEP_FRAMES}, got {n}")),
    }
}

impl tools::IntentParams {
    /// One seat's intent from `move_dir`/`face`/`fire`/`lamp` - seat 0's -
    /// or from the same under `p2_` - seat 1's; `None` when none of them is
    /// given, so the keyboard keeps that seat; an error for an unknown
    /// direction.
    fn seat(&self, seat: usize) -> Result<Option<Intent>, String> {
        let (prefix, move_dir, face, fire, lamp) = if seat == 0 {
            ("", &self.move_dir, &self.face, self.fire, self.lamp)
        } else {
            ("p2_", &self.p2_move_dir, &self.p2_face, self.p2_fire, self.p2_lamp)
        };
        let dir = |key: &str, name: &Option<String>| -> Result<Option<Dir>, String> {
            match name.as_deref() {
                None => Ok(None),
                Some(s) => Dir::parse(s).map(Some).ok_or_else(|| format!("{prefix}{key} must be up|down|left|right, got {s:?}")),
            }
        };
        let (move_dir, face) = (dir("move_dir", move_dir)?, dir("face", face)?);
        if move_dir.is_none() && face.is_none() && fire.is_none() && lamp.is_none() {
            return Ok(None);
        }
        Ok(Some(Intent { move_dir, face, fire: fire.unwrap_or(false), lamp: lamp.unwrap_or(false), ..Intent::default() }))
    }
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
    use crate::net::client::{Identity, RoomClient, RoomSetup};
    use crate::net::codec::{self, Msg};
    use crate::net::encode as enc;
    use crate::net::loopback::{self, LinkQuality, Loopback};
    use crate::net::round::OnlineRound;
    use crate::net::transport::Transport;
    use crate::net::wire::{Lobby, Seat};
    use crate::net::{MAX_SEATS, PROTOCOL_VERSION};
    use crate::simulation::TankSnapshot;
    use std::io::BufRead;

    const W: f32 = 1280.0;
    const H: f32 = 720.0;

    /// A four-enemy Protect band round on the shipped map. The level is
    /// pinned here because these tests measure the dev server, not
    /// whatever mission/spawn plan `maps/default.toml` currently ships.
    fn game(seed: u64) -> Session {
        let mut game = Game::default();
        game.enemy_count_override = Some(4);
        game.level_overrides.mission = Some(crate::level::Mission::Protect);
        game.level_overrides.spawn = Some(crate::level::SpawnKind::Band);
        game.seed_override = Some(seed);
        game.map = MapFile::from_toml_str(include_str!("../maps/default.toml")).expect("embedded default map parses");
        game.init(W, H);
        Session::new(game)
    }

    /// A fresh round on a map shown whole: the inline 34 x 17 map, an
    /// arena, which no shipped map is.
    fn arena(seed: u64) -> Session {
        let mut s = game(seed);
        s.replace_map(MapFile::from_toml_str(INLINE_MAP).expect("inline map parses"));
        s.game.init(W, H);
        s
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

    /// The `overlays` schema, the `screenshot` schema's `overlays` object
    /// and `apply_overlays`'s accepted names are exactly the `Overlays`
    /// fields, so a new layer cannot land in the struct without the tools.
    #[test]
    fn overlay_schemas_match_the_struct() {
        let fields: std::collections::BTreeSet<String> = to_value(Overlays::ALL).as_object().unwrap().keys().cloned().collect();
        let keys = |schema: &Value| -> std::collections::BTreeSet<String> { schema["properties"].as_object().unwrap().keys().cloned().collect() };
        let spec = |name: &str| find(TOOLS, name).unwrap().schema();
        assert_eq!(keys(&spec("overlays")), fields);
        assert_eq!(keys(&spec("screenshot")["properties"]["overlays"]), fields);
    }

    #[test]
    fn every_tool_has_an_object_schema_and_a_unique_name() {
        let mut names = std::collections::HashSet::new();
        for tool in TOOLS {
            assert!(names.insert(tool.name), "duplicate tool {}", tool.name);
            assert!(tool.name.chars().all(|c| c.is_ascii_lowercase() || c == '_'), "{}", tool.name);
            let schema = tool.schema();
            assert_eq!(schema.get("type").and_then(Value::as_str), Some("object"), "{}", tool.name);
            assert!(!tool.description.is_empty());
            assert!(!(tool.read_only && tool.destructive), "{} cannot be both", tool.name);
            let a = tool.annotations();
            assert_eq!(a["readOnlyHint"], tool.read_only, "{}", tool.name);
            assert_eq!(a["destructiveHint"], tool.destructive, "{}", tool.name);
        }
        // The read-only set is exactly the tools that change nothing.
        let read_only: Vec<&str> = TOOLS.iter().filter(|t| t.read_only).map(|t| t.name).collect();
        assert_eq!(
            read_only,
            ["status", "snapshot", "events", "map_get", "lint", "terrain", "history", "nav_grid", "field", "tuning_get", "tuning_schema", "mode", "builder_files"]
        );
        let destructive: Vec<&str> = TOOLS.iter().filter(|t| t.destructive).map(|t| t.name).collect();
        assert_eq!(destructive, ["restart", "kill", "tuning_reset", "players", "play", "builder_save"]);
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
        let rx = call(&tx, "status", json!({ "sections": ["all"] }));
        server.before_frame(&mut game, W, H);
        let status = rx.recv().unwrap().unwrap();
        assert_eq!(status["frame"], 0);
        assert_eq!(status["tanks"], 5);
        assert_eq!(status["outcome"], "playing");
        assert_eq!(status["lockstep"], false);
        assert_eq!(status["players"], 1);
        assert_eq!(status["players_dialog_open"], false);
    }

    /// The camera tool pins the view the window draws and `status` reports
    /// it; a field left out keeps the pin's, a corner is clamped into the
    /// field, a zoom under one or a word for a number is refused, and
    /// `reset` shows the whole field again. The round never sees any of it.
    #[test]
    fn the_camera_tool_pins_a_view_and_lets_it_go() {
        let (mut server, tx) = DevServer::headless();
        let mut s = arena(7);
        let field = (W, H);
        let before = s.game.frame();
        let status = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        assert_eq!(status["camera"]["view"], "whole", "{status}");
        assert_eq!(status["camera"]["scale"], 1.0);
        assert_eq!(status["camera"]["target"], json!([1280, 720]));
        // The motion switch follows a platform that, in a test, said nothing.
        assert_eq!(status["camera"]["motion"], json!({ "reduced": false, "setting": "platform", "platform": null }), "{status}");

        let pinned = ask(&mut server, &tx, &mut s, "camera", json!({ "x": 640.0, "y": 360.0, "zoom": 2.0 })).unwrap();
        assert_eq!(pinned["view"], "pinned", "{pinned}");
        assert_eq!(pinned["rect"], json!({ "x": 320.0, "y": 180.0, "w": 640.0, "h": 360.0 }));
        assert_eq!(pinned["scale"], 2.0);
        assert_eq!(server.camera(field), Camera::zoomed(field, Vec2::new(640.0, 360.0), 2.0));
        let status = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        assert_eq!(status["camera"], pinned);

        ask(&mut server, &tx, &mut s, "camera", json!({ "zoom": 4.0 })).unwrap();
        assert_eq!(server.camera(field), Camera::zoomed(field, Vec2::new(640.0, 360.0), 4.0), "the centre stays");
        let corner = ask(&mut server, &tx, &mut s, "camera", json!({ "x": 0.0, "y": 0.0 })).unwrap();
        assert_eq!((corner["rect"]["x"].as_f64(), corner["rect"]["y"].as_f64()), (Some(0.0), Some(0.0)), "{corner}");
        assert_eq!(corner["zoom"], 4.0, "the zoom stays");
        let report = ask(&mut server, &tx, &mut s, "camera", json!({})).unwrap();
        assert_eq!(report, corner, "no parameters only report");

        assert!(ask(&mut server, &tx, &mut s, "camera", json!({ "zoom": 0.5 })).unwrap_err().contains("zoom"));
        assert!(ask(&mut server, &tx, &mut s, "camera", json!({ "x": "left" })).unwrap_err().contains("camera: x: invalid type"));
        assert_eq!(server.camera(field), Camera::zoomed(field, Vec2::new(0.0, 0.0), 4.0), "a refused call changes nothing");

        let whole = ask(&mut server, &tx, &mut s, "camera", json!({ "reset": true })).unwrap();
        assert_eq!(whole["view"], "whole");
        assert_eq!(whole["zoom"], Value::Null);
        assert_eq!(server.camera(field), Camera::whole(field));
        assert_eq!(s.game.frame(), before, "the round never moved");
    }

    /// A field map with no window behind the server is still reported as
    /// followed; with the window's frame handed over, `status.camera`
    /// carries the followed seat, the framing (scale, snap, the tank's
    /// size) and whether the sight box is on screen, and a `click` lands
    /// on the bitmap that frame drew. A pin outranks it, and `reset` hands
    /// the view back to the map.
    #[test]
    fn status_reports_a_followed_view_as_the_window_drew_it() {
        use crate::follow::{CameraMode, CameraReport, FollowReport, Shot, ShotKind};
        use crate::framing::{Screen, SightBox, ViewRules};
        let (mut server, tx) = DevServer::headless();
        let mut s = game(7);
        s.game.map = MapFile::from_toml_str("version = 1\nsize = [48, 24]\n").unwrap();
        let field = s.game.map.field_size();
        s.game.init(field.0, field.1);
        let at = |server: &mut DevServer, s: &mut Session, method: &str, params: Value| {
            let rx = call(&tx, method, params);
            server.before_frame(s, field.0, field.1);
            rx.recv().unwrap()
        };
        let status = at(&mut server, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        assert_eq!(status["camera"]["view"], "follow", "{status}");
        assert_eq!(status["camera"]["rect"], Value::Null, "no frame drawn yet");

        // The window's frame: the 1080p monitor's local view, the whole
        // window the world's, the seat followed near the middle.
        let sight = SightBox::from_cells(11.5, 7.5);
        let screen = Screen::new(1920.0, 1080.0, 1.0, 92.0).with_panel_width(1920.0);
        let frame = crate::view::FollowFrame::new(screen, (1920.0, 1080.0), Seating::Local, sight, &ViewRules::of(&crate::tuning::Tuning::DEFAULT));
        let framing = frame.framing;
        let seat = Vec2::new(760.0, 380.0);
        let corner = Vec2::new(seat.x - 640.0, seat.y - 360.0);
        let camera = Camera::following(field, corner, framing.visible, 1.0, 1.5);
        let (layout, view) = (frame.layout, frame.view);
        let shot = Shot {
            corner,
            center: seat,
            kind: ShotKind::Seat,
            seat: Some(0),
            keeps: [Some(seat), None],
            lead: Vec2::new(0.0, 0.0),
            cut: false,
            glide: None,
            split: None,
        };
        server.publish_camera(CameraReport {
            mode: CameraMode::Follow,
            camera,
            layout,
            view,
            follow: Some(FollowReport { framing, seating: Seating::Local, sight, shot, establishing: crate::establish::Phase::Follow, second: None }),
        });
        server.publish_ui(UiFrame::plain((1920.0, 1080.0)));
        let status = at(&mut server, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        let c = status["camera"].clone();
        assert_eq!(c["view"], "follow", "{c}");
        assert_eq!(c["seat"], 0);
        assert_eq!(c["focus"], "seat");
        assert_eq!(c["rect"], json!({ "x": 120.0, "y": 20.0, "w": 1280.0, "h": 720.0 }));
        assert_eq!(c["framing"]["visible_cells"], json!([40.0, 22.5]), "no bar: the whole window is the world's");
        assert_eq!(c["framing"]["device_scale"], 1.5);
        assert_eq!(c["framing"]["snapped"], true);
        assert_eq!(c["framing"]["tank_points"], 96.0);
        assert!(c["framing"]["tank_mm"].as_f64().is_some_and(|mm| (mm - 26.5).abs() < 0.1), "{c}");
        assert_eq!(c["sight_box"]["in_view"], true);
        assert_eq!(c["target"], json!([1282, 722]));
        // The corners' buttons are hit on the window, where `status.ui`
        // says they are: BUILD at the window's top-right.
        let b = &status["ui"]["buttons"]["build"];
        let (bx, by) = (b["x"].as_f64().unwrap() + 36.0, b["y"].as_f64().unwrap() + 16.0);
        assert!(bx > 1800.0 && by < 60.0, "{b}");
        let m = at(&mut server, &mut s, "click", json!({ "x": bx, "y": by })).unwrap();
        assert_eq!(m["mode"], "build", "{m}");
        let m = at(&mut server, &mut s, "key", json!({ "key": "escape" })).unwrap();
        assert_eq!(m["mode"], "play", "Escape with nothing to take back is BACK: {m}");
        // A press on the field lands in the world through that camera: the
        // window's middle is the followed seat.
        let m = at(&mut server, &mut s, "click", json!({ "x": 960.0, "y": 540.0 })).unwrap();
        assert_eq!(m["world"], json!({ "x": seat.x, "y": seat.y }), "{m}");
        assert_eq!(m["mode"], "play");
        // A pin outranks the follow; `reset` gives the view back.
        let pinned = at(&mut server, &mut s, "camera", json!({ "zoom": 2.0 })).unwrap();
        assert_eq!(pinned["view"], "pinned");
        let back = at(&mut server, &mut s, "camera", json!({ "reset": true })).unwrap();
        assert_eq!(back["view"], "follow", "{back}");
    }

    /// A couch's split screen as the window drew it: `status.camera` names
    /// the split, the divider on the bitmap and on the window, and each
    /// half's seat and camera; a `click` lands in the world of the half it
    /// falls in.
    #[test]
    fn status_reports_a_split_screen_and_a_click_lands_in_its_half() {
        use crate::follow::{CameraMode, CameraReport, FollowReport, Shot, ShotKind, Split};
        use crate::framing::{Screen, SightBox, ViewRules};
        let (mut server, tx) = DevServer::headless();
        let mut s = game(7);
        s.game.map = MapFile::from_toml_str("version = 1\nsize = [100, 50]\n").unwrap();
        let field = s.game.map.field_size();
        s.game.init(field.0, field.1);
        let at = |server: &mut DevServer, s: &mut Session, method: &str, params: Value| {
            let rx = call(&tx, method, params);
            server.before_frame(s, field.0, field.1);
            rx.recv().unwrap()
        };
        let sight = SightBox::from_cells(11.5, 7.5);
        let screen = Screen::new(1920.0, 1080.0, 1.0, 92.0).with_panel_width(1920.0);
        let frame = crate::view::FollowFrame::new(screen, (1920.0, 1080.0), Seating::Local, sight, &ViewRules::of(&crate::tuning::Tuning::DEFAULT));
        let framing = frame.framing;
        // Seat 0's half on the left, seat 1's on the right, the divider
        // straight down the middle of the 1280 x 720 view.
        let (seat_a, seat_b) = (Vec2::new(760.0, 380.0), Vec2::new(2400.0, 400.0));
        let (corner_a, corner_b) = (Vec2::new(120.0, 20.0), Vec2::new(1900.0, 40.0));
        let first = Camera::following(field, corner_a, framing.visible, 1.0, 1.5);
        let second = Camera::following(field, corner_b, framing.visible, 1.0, 1.5);
        let split = Split {
            corner: corner_b,
            center: corner_b + Vec2::new(640.0, 360.0),
            seat: 1,
            keeps: [Some(seat_b), None],
            cut: false,
            at: Vec2::new(640.0, 360.0),
            normal: Vec2::new(1.0, 0.0),
            apart: 1780.0,
        };
        let shot = Shot {
            corner: corner_a,
            center: corner_a + Vec2::new(640.0, 360.0),
            kind: ShotKind::Split,
            seat: Some(0),
            keeps: [Some(seat_a), None],
            lead: Vec2::new(0.0, 0.0),
            cut: false,
            glide: None,
            split: Some(split),
        };
        server.publish_camera(CameraReport {
            mode: CameraMode::Follow,
            camera: first,
            layout: frame.layout,
            view: frame.view,
            follow: Some(FollowReport { framing, seating: Seating::Local, sight, shot, establishing: crate::establish::Phase::Follow, second: Some(second) }),
        });
        server.publish_ui(UiFrame::plain((1920.0, 1080.0)));
        let status = at(&mut server, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        let c = &status["camera"];
        assert_eq!(c["focus"], "split", "{c}");
        let sp = &c["split"];
        assert_eq!(sp["line"], json!({ "x": 640.0, "y": 360.0, "nx": 1.0, "ny": 0.0 }), "{sp}");
        assert_eq!(sp["window_line"], json!({ "x": 960.0, "y": 540.0 }), "the bitmap's middle is the window's");
        assert_eq!(sp["apart"], 1780.0);
        assert_eq!((sp["halves"][0]["seat"].clone(), sp["halves"][1]["seat"].clone()), (json!(0), json!(1)));
        assert_eq!(sp["halves"][0]["rect"], json!({ "x": 120.0, "y": 20.0, "w": 1280.0, "h": 720.0 }));
        assert_eq!(sp["halves"][1]["rect"], json!({ "x": 1900.0, "y": 40.0, "w": 1280.0, "h": 720.0 }));
        assert_eq!((sp["halves"][0]["in_view"].clone(), sp["halves"][1]["in_view"].clone()), (json!(true), json!(true)));
        // Left of the divider is seat 0's world, right of it seat 1's.
        let m = at(&mut server, &mut s, "click", json!({ "x": 660.0, "y": 540.0 })).unwrap();
        assert_eq!(m["world"], json!({ "x": 560.0, "y": 380.0 }), "{m}");
        let m = at(&mut server, &mut s, "click", json!({ "x": 1260.0, "y": 540.0 })).unwrap();
        assert_eq!(m["world"], json!({ "x": 2740.0, "y": 400.0 }), "{m}");
    }

    /// The players tool and the button/keys behind it: the dialog freezes
    /// the round, a new count restarts in that mode with player 2 in slot
    /// 1, `step` drives player 2 through `p2_*`, and `restart {players: 1}`
    /// goes back.
    #[test]
    fn players_tool_switches_mode_and_the_button_opens_the_dialog() {
        let (mut server, tx) = DevServer::headless();
        let mut s = game(41);
        // The button opens it and the round freezes; a press outside closes it.
        let m = ask(&mut server, &tx, &mut s, "players", json!({})).unwrap();
        assert_eq!(m["players_dialog_open"], true, "{m}");
        assert!(!s.playing());
        let m = ask(&mut server, &tx, &mut s, "click", json!({ "x": 10.0, "y": 100.0 })).unwrap();
        assert_eq!(m["players_dialog_open"], false, "{m}");
        let b = corner_button(&mut server, &tx, &mut s, "players");
        let m = ask(&mut server, &tx, &mut s, "click", b).unwrap();
        assert_eq!(m["players_dialog_open"], true, "{m}");
        let m = ask(&mut server, &tx, &mut s, "key", json!({ "key": "escape" })).unwrap();
        assert_eq!(m["players_dialog_open"], false);
        // Answering with two restarts in two-player mode, frozen in lockstep.
        let m = ask(&mut server, &tx, &mut s, "players", json!({ "count": 2 })).unwrap();
        assert_eq!(m["players"], 2, "{m}");
        assert_eq!(m["players_dialog_open"], false);
        assert!(server.lockstep());
        assert!(s.playing());
        assert_eq!(s.game.frame(), 0);
        assert!(s.game.seat(1).is_some());
        let st = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        assert_eq!(st["players"], 2);
        assert_eq!(st["enemies_alive"], 4, "both players excluded from the enemy count");
        let snap = ask(&mut server, &tx, &mut s, "snapshot", json!({})).unwrap();
        let tanks = snap["tanks"].as_array().unwrap();
        assert_eq!(tanks[0]["player"], 0);
        assert_eq!(tanks[1]["player"], 1);
        assert_eq!(tanks[1]["slot"], 1);
        assert_eq!(tanks[2]["slot"], 2, "enemies count from 2");
        assert!(tanks[2]["player"].is_null());
        // The same count again: no restart.
        for _ in 0..3 {
            let rx = call(&tx, "step", json!({ "frames": 1, "snapshot": false }));
            server.before_frame(&mut s, W, H);
            server.advance(&mut s.game, Input::default(), 1, W, H, &mut |_| {});
            rx.recv().unwrap().unwrap();
        }
        ask(&mut server, &tx, &mut s, "players", json!({ "count": 2 })).unwrap();
        assert_eq!(s.game.frame(), 3);
        // `p2_*` drives player 2 and nobody else.
        let before: Vec<_> = s.game.tank_snapshots().into_iter().filter(|t| t.is_player).map(|t| (t.player, t.position)).collect();
        let rx = call(&tx, "step", json!({ "frames": 30, "p2_move_dir": "down", "snapshot": false }));
        server.before_frame(&mut s, W, H);
        server.advance(&mut s.game, Input::default(), 1, W, H, &mut |_| {});
        rx.recv().unwrap().unwrap();
        let after: Vec<_> = s.game.tank_snapshots().into_iter().filter(|t| t.is_player).map(|t| (t.player, t.position)).collect();
        assert!((after[0].1.y - before[0].1.y).abs() < 1.0, "player 1 stayed put");
        assert!(after[1].1.y > before[1].1.y + 20.0, "player 2 drove down");
        // The keys answer the dialog; Enter means the other count.
        ask(&mut server, &tx, &mut s, "players", json!({})).unwrap();
        let m = ask(&mut server, &tx, &mut s, "key", json!({ "key": "enter" })).unwrap();
        assert_eq!(m["players"], 1, "{m}");
        assert!(s.game.seat(1).is_none());
        ask(&mut server, &tx, &mut s, "players", json!({})).unwrap();
        let m = ask(&mut server, &tx, &mut s, "key", json!({ "key": "2" })).unwrap();
        assert_eq!(m["players"], 2, "{m}");
        // Past the couch's two: the round seats up to `MAX_SEATS`, and
        // nothing outside that range.
        let m = ask(&mut server, &tx, &mut s, "players", json!({ "count": 4 })).unwrap();
        assert_eq!(m["players"], 4, "{m}");
        assert!(s.game.seat(3).is_some());
        assert!(ask(&mut server, &tx, &mut s, "players", json!({ "count": 0 })).is_err());
        assert!(ask(&mut server, &tx, &mut s, "players", json!({ "count": crate::MAX_SEATS + 1 })).is_err());
        // restart takes the count too, and clears an open dialog.
        ask(&mut server, &tx, &mut s, "players", json!({})).unwrap();
        let st = ask(&mut server, &tx, &mut s, "restart", json!({ "players": 1, "seed": 3 })).unwrap();
        assert_eq!(st["players"], 1, "{st}");
        assert_eq!(st["players_dialog_open"], false);
        assert!(s.game.seat(1).is_none());
        // Refused in build mode, like the other game-only tools.
        enter_build(&mut server, &tx, &mut s);
        assert!(ask(&mut server, &tx, &mut s, "players", json!({ "count": 2 })).is_err());
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
        server.advance(&mut stepped, Input::default(), 1, W, H, &mut |_| {});
        let reply = rx.recv().unwrap().unwrap();
        assert_eq!(reply["frame"], 90);
        assert_eq!(reply["restarted"], false);
        assert!(reply["events"].as_array().unwrap().iter().any(|e| e["event"] == "fired"), "{}", reply["events"]);

        let intent = Intent { move_dir: Some(Dir::Up), fire: true, ..Intent::default() };
        for _ in 0..90 {
            manual.update(Input::single(intent), PHYSICS_FIXED_DT, W, H);
        }
        assert_eq!(reply["time"], r1(manual.time), "the reply's time is the game's, at snapshot precision");
        let (a, b) = (stepped.tank_snapshots(), manual.tank_snapshots());
        assert_eq!(a.len(), b.len());
        for (i, (x, y)) in a.iter().zip(&b).enumerate() {
            assert_eq!(key(x), key(y), "tank {i} diverged");
        }
        // Lockstep holds the game still until the next step.
        server.advance(&mut stepped, Input::default(), 1, W, H, &mut |_| {});
        assert_eq!(stepped.frame(), 90);
    }

    #[test]
    fn fire_every_taps_the_trigger_instead_of_holding_it() {
        let (mut server, tx) = DevServer::headless();
        let mut game = game(4);
        let rx = call(&tx, "step", json!({ "frames": 120, "fire": true, "fire_every": 40, "snapshot": false }));
        server.before_frame(&mut game, W, H);
        server.advance(&mut game, Input::default(), 1, W, H, &mut |_| {});
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
        let rx = call(&tx, "step", json!({ "frames": 1, "snapshot": true }));
        server.before_frame(&mut game, W, H);
        server.advance(&mut game, Input::default(), 1, W, H, &mut |_| {});
        let reply = rx.recv().unwrap().unwrap();
        let player = &reply["snapshot"]["tanks"][0];
        assert_eq!(player["slot"], 0);
        assert!((player["x"].as_f64().unwrap() - 400.0).abs() < 1.0, "{player}");
        assert!((player["y"].as_f64().unwrap() - 300.0).abs() < 1.0, "{player}");
        assert_eq!(player["rotation"], 270.0);
        assert_eq!(player["facing"], "left");
        assert_eq!(player["cell"], json!([13, 9]), "400/32 = 12.5 and 300/32 = 9.4, rounded: {player}");
        assert!(player["speed"].as_f64().unwrap() < 1.0, "{player}");
        assert!(player["heading"].is_null(), "no heading while still: {player}");
        assert!(player["turret"].is_number() && player["hull"].is_number());
        assert!(player["dist_to_player"].is_null(), "players have no dist_to_player");
        assert!(reply["snapshot"]["tanks"][1]["dist_to_player"].is_number());
    }

    #[test]
    fn kill_runs_through_the_explosion_path() {
        let (mut server, tx) = DevServer::headless();
        let mut game = game(5);
        let rx = call(&tx, "kill", json!({ "slot": 2 }));
        server.before_frame(&mut game, W, H);
        rx.recv().unwrap().unwrap();
        let rx = call(&tx, "step", json!({ "frames": 1, "snapshot": true }));
        server.before_frame(&mut game, W, H);
        server.advance(&mut game, Input::default(), 1, W, H, &mut |_| {});
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

    /// `set_tank {sonic_ammo}` arms the hammer, and `spawn_pickup` puts a
    /// crate down at the cell nearest the point, refusing a kind it does
    /// not know and a cell that holds one already.
    #[test]
    fn set_tank_arms_the_hammer_and_spawn_pickup_drops_its_crate() {
        let (mut server, tx) = DevServer::headless();
        let mut game = game(3);
        let ask = |server: &mut DevServer, game: &mut Session, tool: &str, params: Value| {
            let rx = call(&tx, tool, params);
            server.before_frame(game, W, H);
            rx.recv().unwrap()
        };
        let tank = ask(&mut server, &mut game, "set_tank", json!({ "slot": 0, "sonic_ammo": 4 })).unwrap();
        assert_eq!((tank["weapon"].as_str(), tank["sonic"].as_i64()), (Some("sonic_hammer"), Some(4)), "{tank}");
        // The first open cell along a row of the field, a point off its
        // centre: the crate lands on the centre.
        let (x, y, crate_at) = (2..30)
            .find_map(|col| {
                let (x, y) = (col as f64 * 32.0 + 5.0, 8.0 * 32.0 - 7.0);
                ask(&mut server, &mut game, "spawn_pickup", json!({ "kind": "sonic_hammer", "x": x, "y": y })).ok().map(|r| (x, y, r))
            })
            .expect("an open cell");
        assert_eq!(crate_at, json!({ "kind": "sonic_hammer", "x": x - 5.0, "y": y + 7.0 }));
        let again = ask(&mut server, &mut game, "spawn_pickup", json!({ "kind": "sonic_hammer", "x": x, "y": y }));
        assert!(again.is_err(), "{again:?}");
        let unknown = ask(&mut server, &mut game, "spawn_pickup", json!({ "kind": "railgun", "x": 100.0, "y": 100.0 }));
        assert!(unknown.unwrap_err().contains("railgun"));
    }

    /// `set_tank {gauss_slugs}` arms the rail and `charge` puts a charge on
    /// it, which the tank's line reports with its stage.
    #[test]
    fn set_tank_arms_the_rail_and_charges_it() {
        let (mut server, tx) = DevServer::headless();
        let mut game = game(3);
        let ask = |server: &mut DevServer, game: &mut Session, tool: &str, params: Value| {
            let rx = call(&tx, tool, params);
            server.before_frame(game, W, H);
            rx.recv().unwrap()
        };
        let tank = ask(&mut server, &mut game, "set_tank", json!({ "slot": 0, "gauss_slugs": 3 })).unwrap();
        assert_eq!((tank["weapon"].as_str(), tank["gauss"].as_i64()), (Some("gauss_rail"), Some(3)), "{tank}");
        let tank = ask(&mut server, &mut game, "set_tank", json!({ "slot": 0, "charge": 1.6 })).unwrap();
        assert_eq!((tank["charge"][0].as_str(), tank["charge"][2].as_str()), (Some("gauss_rail"), Some("full")), "{tank}");
        assert!((tank["charge"][1].as_f64().unwrap() - 1.6).abs() < 1e-3, "{tank}");
        let tank = ask(&mut server, &mut game, "set_tank", json!({ "slot": 0, "charge": 0.0 })).unwrap();
        assert!(tank["charge"].is_null(), "{tank}");
    }

    /// `set_tank {emp_charges}` arms the EMP, `disabled` and
    /// `special_offline` put a tank's outages on and off by hand, and
    /// `spawn_pickup` drops the EMP's crate.
    #[test]
    fn set_tank_arms_the_emp_and_takes_it_offline() {
        let (mut server, tx) = DevServer::headless();
        let mut game = game(3);
        let ask = |server: &mut DevServer, game: &mut Session, tool: &str, params: Value| {
            let rx = call(&tx, tool, params);
            server.before_frame(game, W, H);
            rx.recv().unwrap()
        };
        let tank = ask(&mut server, &mut game, "set_tank", json!({ "slot": 0, "emp_charges": 2 })).unwrap();
        assert_eq!((tank["weapon"].as_str(), tank["emp"].as_i64()), (Some("emp_burst"), Some(2)), "{tank}");
        let tank = ask(&mut server, &mut game, "set_tank", json!({ "slot": 0, "special_offline": 1.5 })).unwrap();
        assert_eq!((tank["weapon"].as_str(), tank["offline"].as_f64()), (Some("shell"), Some(1.5)), "offline, it fires shells: {tank}");
        let enemy = game.first_enemy_slot();
        let tank = ask(&mut server, &mut game, "set_tank", json!({ "slot": enemy, "disabled": 2.0 })).unwrap();
        assert_eq!(tank["disabled"].as_f64(), Some(2.0), "{tank}");
        let tank = ask(&mut server, &mut game, "set_tank", json!({ "slot": enemy, "disabled": 0.0 })).unwrap();
        assert_eq!(tank["disabled"].as_f64(), Some(0.0), "{tank}");
        let dropped = (2..30).find_map(|col| {
            let (x, y) = (col as f64 * 32.0, 8.0 * 32.0);
            ask(&mut server, &mut game, "spawn_pickup", json!({ "kind": "emp_burst", "x": x, "y": y })).ok()
        });
        assert_eq!(dropped.expect("an open cell")["kind"], "emp_burst");
    }

    /// The two tank layers are independent flags: one on leaves the other
    /// where it was, and `status` reports the same values.
    #[test]
    fn overlays_sets_hitboxes_and_stats_independently() {
        let (mut server, tx) = DevServer::headless();
        let mut game = game(6);
        let set = |server: &mut DevServer, game: &mut Session, params: Value| {
            let rx = call(&tx, "overlays", params);
            server.before_frame(game, W, H);
            rx.recv().unwrap().unwrap()
        };
        let flags = set(&mut server, &mut game, json!({ "hitboxes": true, "ai": true }));
        assert_eq!(flags["hitboxes"], true, "{flags}");
        assert_eq!(flags["stats"], false, "{flags}");
        assert_eq!(flags["ai"], true, "{flags}");
        assert_eq!(flags["nav_grid"], false, "{flags}");
        assert!(flags.get("inspect").is_none(), "{flags}");
        let flags = set(&mut server, &mut game, json!({ "stats": true }));
        assert_eq!(flags["hitboxes"], true, "{flags}");
        assert_eq!(flags["stats"], true, "{flags}");
        let flags = set(&mut server, &mut game, json!({ "hitboxes": false }));
        assert_eq!(flags["hitboxes"], false, "{flags}");
        assert_eq!(flags["stats"], true, "{flags}");
        let rx = call(&tx, "status", json!({ "sections": ["all"] }));
        server.before_frame(&mut game, W, H);
        let status = rx.recv().unwrap().unwrap();
        assert_eq!(status["overlays"]["hitboxes"], false, "{status}");
        assert_eq!(status["overlays"]["stats"], true, "{status}");
    }

    /// A flag name the struct does not have is refused, naming the real
    /// ones, and the current flags are untouched - on `overlays` and on
    /// `screenshot {overlays}` alike.
    #[test]
    fn overlays_rejects_unknown_flags() {
        let (mut server, tx) = DevServer::headless();
        let mut game = game(6);
        let rx = call(&tx, "overlays", json!({ "inspect": true, "ai": true }));
        server.before_frame(&mut game, W, H);
        let err = rx.recv().unwrap().unwrap_err();
        assert!(err.contains("inspect") && err.contains("hitboxes") && err.contains("stats"), "{err}");
        assert_eq!(game.debug_overlays, Overlays::NONE);
        let rx = call(&tx, "screenshot", json!({ "overlays": { "inspect": true } }));
        server.before_frame(&mut game, W, H);
        let err = rx.recv().unwrap().unwrap_err();
        assert!(err.contains("inspect"), "{err}");
        assert_eq!(game.debug_overlays, Overlays::NONE);
    }

    /// `weather` reports and sets the round's sky; `status` and `map_get`
    /// carry it, an unknown name is refused naming the real ones, and the
    /// builder's row is `builder_settings {weather}`, refused by name there.
    #[test]
    fn weather_sets_the_rounds_sky_and_every_reader_reports_it() {
        let (mut server, tx) = DevServer::headless();
        let mut game = game(6);
        let ask = |server: &mut DevServer, game: &mut Session, method: &str, params: Value| {
            let rx = call(&tx, method, params);
            server.before_frame(game, W, H);
            rx.recv().unwrap()
        };
        let w = ask(&mut server, &mut game, "weather", json!({})).unwrap();
        assert_eq!(w["in_force"], "clear", "{w}");
        assert_eq!(w["override"], Value::Null, "{w}");
        assert_eq!(w["names"].as_array().unwrap().len(), crate::map::Weather::ALL.len(), "{w}");
        assert_eq!(w["rules"]["enemy_sight_px"], json!(crate::tuning::tuning().enemy_view_range), "{w}");
        assert_eq!(w["rules"]["grip"], json!(1.0), "{w}");
        // No window compiled anything here, and the knob is off.
        assert_eq!(w["without_shaders"], false, "{w}");
        // Setting a sky changes it mid-round: the round runs on under it.
        for _ in 0..5 {
            game.game.update(Input::default(), crate::PHYSICS_FIXED_DT, W, H);
        }
        let seed = game.game.round_seed();
        let w = ask(&mut server, &mut game, "weather", json!({ "name": "fog" })).unwrap();
        assert_eq!((w["map"].as_str(), w["in_force"].as_str()), (Some("fog"), Some("fog")), "{w}");
        assert_eq!(game.game.frame(), 5, "the round runs on");
        // `restart` starts it over under the sky on its own seed instead.
        let w = ask(&mut server, &mut game, "weather", json!({ "name": "storm", "restart": true })).unwrap();
        assert_eq!(w["map"], "storm", "{w}");
        assert_eq!(w["in_force"], "storm", "{w}");
        assert_eq!(game.game.map.weather, crate::map::Weather::Storm.into());
        assert_eq!(game.game.frame(), 0, "the round started over");
        assert_eq!(game.game.round_seed(), seed, "on the seed it had");
        assert_eq!(game.game.seed_override, Some(6), "and the seed setting is left as it was");
        assert_eq!(w["rules"]["enemy_sight_px"], json!(crate::tuning::tuning().enemy_view_range * crate::tuning::tuning().night_sight_factor), "{w}");
        assert_eq!(w["rules"]["grip"], json!(crate::tuning::tuning().rain_grip_factor), "{w}");
        let w = ask(&mut server, &mut game, "weather", json!({ "name": ["fog", "dusk"] })).unwrap();
        assert_eq!(w["map"], json!(["dusk", "fog"]), "{w}");
        assert!(matches!(w["in_force"].as_str(), Some("dusk" | "fog")), "{w}");
        ask(&mut server, &mut game, "weather", json!({ "name": "storm" })).unwrap();
        let err = ask(&mut server, &mut game, "weather", json!({ "name": "hail" })).unwrap_err();
        assert!(err.contains("heat_haze"), "{err}");
        assert_eq!(game.game.map.weather, crate::map::Weather::Storm.into(), "a refused name changes nothing");
        // A random sky is the round seed's pick, and a restart on the same
        // seed brings it back.
        let w = ask(&mut server, &mut game, "weather", json!({ "name": "random" })).unwrap();
        assert_eq!(w["map"], "random", "{w}");
        assert_eq!(w["in_force"], crate::weather::random_sky(game.game.round_seed()).name(), "{w}");
        for _ in 0..2 {
            ask(&mut server, &mut game, "restart", json!({ "seed": "0xB0B5" })).unwrap();
            let w = ask(&mut server, &mut game, "weather", json!({})).unwrap();
            assert_eq!(w["map"], "random", "a restart keeps the map's key: {w}");
            assert_eq!(w["in_force"], crate::weather::random_sky(0xB0B5).name(), "{w}");
        }
        ask(&mut server, &mut game, "weather", json!({ "name": "storm" })).unwrap();
        let status = ask(&mut server, &mut game, "status", json!({ "sections": ["all"] })).unwrap();
        assert_eq!(status["weather"]["in_force"], "storm", "{status}");
        assert_eq!(status["map"]["weather"], "storm", "{status}");
        let map = ask(&mut server, &mut game, "map_get", json!({})).unwrap();
        assert!(map["toml"].as_str().unwrap().contains("weather = \"storm\""), "{map}");
        let settings = ask(&mut server, &mut game, "builder_settings", json!({ "weather": "fog" })).unwrap();
        assert_eq!(settings["weather"], "fog", "{settings}");
        assert_eq!(game.builder.map().weather, crate::map::Weather::Fog.into());
        // Several skies are a list, in the map file's order.
        let settings = ask(&mut server, &mut game, "builder_settings", json!({ "weather": ["snow", "rain"] })).unwrap();
        assert_eq!(settings["weather"], json!(["rain", "snow"]), "{settings}");
        let settings = ask(&mut server, &mut game, "builder_settings", json!({ "weather": null })).unwrap();
        assert_eq!(settings["weather"], "clear", "{settings}");
        assert!(ask(&mut server, &mut game, "builder_settings", json!({ "weather": ["rain", "hail"] })).is_err());
        ask(&mut server, &mut game, "build", json!({})).unwrap();
        let err = ask(&mut server, &mut game, "weather", json!({ "name": "rain" })).unwrap_err();
        assert!(err.contains("play"), "{err}");
    }

    /// `status.frame_stages` carries the last frame's draws and the
    /// window's pace beside the stage averages.
    #[test]
    fn status_reports_the_frames_draws_and_pace() {
        let (mut server, tx) = DevServer::headless();
        let mut s = game(7);
        crate::frame_stages::blit(3);
        crate::frame_stages::rect();
        let _ = crate::frame_stages::target(());
        crate::frame_stages::steps(2);
        crate::frame_stages::frame_done(20.0);
        let status = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        let stages = &status["frame_stages"];
        assert_eq!(stages["frame"], json!(20.0));
        assert_eq!(stages["draws"], json!({ "blits": 1, "rects": 1, "shapes": 0, "texture_switches": 2, "blend_scopes": 0, "shader_scopes": 0, "target_scopes": 1 }));
        assert_eq!(stages["pace"], json!({ "frames": 1, "max_ms": 20.0, "p99_ms": 20.0, "steps": [0, 0, 1, 0, 0] }));
    }

    /// The windowed loop pays real time out in whole steps and hands the
    /// count here: a live server runs each at the fixed step and spends
    /// the one-shot presses on the first, a frozen one runs none of them,
    /// and `after_step` sees every step's state.
    #[test]
    fn advance_runs_the_clocks_steps_unless_frozen() {
        let (mut server, tx) = DevServer::headless();
        let mut s = game(7);
        let mut seen = Vec::new();
        server.advance(&mut s.game, Input::default(), 3, W, H, &mut |g| seen.push(g.frame()));
        assert_eq!(s.game.frame(), 3);
        assert_eq!(seen, vec![1, 2, 3]);
        // A frame of two steps toggles pause once, not twice.
        let pause = Input { pause_pressed: true, ..Input::default() };
        server.advance(&mut s.game, pause, 2, W, H, &mut |_| {});
        assert!(s.game.paused);
        assert_eq!(s.game.frame(), 5, "paused updates still count frames");
        server.advance(&mut s.game, pause, 1, W, H, &mut |_| {});
        assert!(!s.game.paused);
        // Frozen: the steps the clock paid for do not run.
        ask(&mut server, &tx, &mut s, "pause", json!({})).unwrap();
        assert!(server.lockstep());
        let mut ran = 0;
        server.advance(&mut s.game, Input::default(), 4, W, H, &mut |_| ran += 1);
        assert_eq!((s.game.frame(), ran), (6, 0));
        ask(&mut server, &tx, &mut s, "resume", json!({})).unwrap();
        server.advance(&mut s.game, Input::default(), 1, W, H, &mut |_| ran += 1);
        assert_eq!((s.game.frame(), ran), (7, 1));
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
            assert!(input.seats[0].move_dir.is_none(), "a bare cycle request leaves the keyboard alone");
            server.advance(&mut game, input, 1, W, H, &mut |_| {});
            assert_eq!(game.debug_overlays, preset);
            // One-shot: the next frame's input does not press the key again.
            let input = server.shape_input(Input::default());
            assert!(!input.cycle_overlays_pressed);
            server.advance(&mut game, input, 1, W, H, &mut |_| {});
            assert_eq!(game.debug_overlays, preset);
        }
    }

    /// `field` reads the frame's routing grid toward a target: the goal
    /// cell is marked and costs 0, every open reachable cell has an
    /// arrow and a cost, blocked cells are `#`/-1, and a target that is
    /// not in the round is an error naming it.
    #[test]
    fn field_dumps_the_flow_toward_a_target() {
        let (mut server, tx) = DevServer::headless();
        let mut game = game(21);
        let rx = call(&tx, "field", json!({}));
        server.before_frame(&mut game, W, H);
        let reply = rx.recv().unwrap().unwrap();
        let arrows = reply["arrows"].as_str().unwrap();
        let lines: Vec<&str> = arrows.lines().collect();
        assert_eq!(lines.len(), 23);
        assert!(lines.iter().all(|l| l.len() == 40));
        let goal = (reply["goal"][0].as_u64().unwrap() as usize, reply["goal"][1].as_u64().unwrap() as usize);
        assert_eq!(lines[goal.1].as_bytes()[goal.0], b'G');
        let costs = reply["costs"].as_array().unwrap();
        assert_eq!(costs.len(), 23);
        assert_eq!(costs[goal.1][goal.0], 0);
        let (mut arrows_seen, mut stranded) = (0, Vec::new());
        for (r, line) in lines.iter().enumerate() {
            for (c, ch) in line.bytes().enumerate() {
                let cost = costs[r][c].as_i64().unwrap();
                match ch {
                    b'#' => assert_eq!(cost, -1, "({c}, {r})"),
                    b'.' => {
                        assert_eq!(cost, -1, "({c}, {r})");
                        stranded.push((c, r));
                    }
                    b'G' => assert_eq!(cost, 0),
                    b'^' | b'v' | b'<' | b'>' => {
                        assert!(cost > 0, "({c}, {r})");
                        arrows_seen += 1;
                    }
                    other => panic!("unexpected {:?} at ({c}, {r})", other as char),
                }
            }
        }
        // A stranded cell is exactly one the flood fill says has no
        // route to the player (the test window is larger than the map's
        // field, so the open ground past the border walls is all of
        // that, plus the roll-in lanes the walls seal from inside).
        assert!(arrows_seen > 150, "flowing {arrows_seen}");
        let grid = game.route_grid(W, H);
        let cell = reply["cell"].as_f64().unwrap() as f32;
        let centre = |(c, r): (usize, usize)| Position::new((c as f32 + 0.5) * cell, (r as f32 + 0.5) * cell);
        assert!(!stranded.is_empty());
        for &at in &stranded {
            assert!(!grid.connected(centre(at), centre(goal)), "{at:?} is stranded but connected");
        }
        for (r, line) in lines.iter().enumerate() {
            for (c, ch) in line.bytes().enumerate() {
                if matches!(ch, b'^' | b'v' | b'<' | b'>') {
                    assert!(grid.connected(centre((c, r)), centre(goal)), "({c}, {r}) flows but is not connected");
                }
            }
        }

        let rx = call(&tx, "field", json!({ "target": "player2" }));
        server.before_frame(&mut game, W, H);
        let err = rx.recv().unwrap().unwrap_err();
        assert!(err.contains("Player(1)"), "{err}");
    }

    #[test]
    fn nav_grid_and_full_snapshot_have_the_expected_shape() {
        let (mut server, tx) = DevServer::headless();
        let mut game = game(21);
        let rx = call(&tx, "nav_grid", json!({}));
        server.before_frame(&mut game, W, H);
        let grid = rx.recv().unwrap().unwrap()["grid"].as_str().unwrap().to_string();
        let lines: Vec<&str> = grid.lines().collect();
        // The 1280x720 test field at PATHFIND_CELL_SIZE (= OBSTACLE_GRID_SIZE,
        // 32px): 40 columns and ceil(22.5) = 23 rows.
        assert_eq!(lines.len(), 24, "23 rows plus the legend");
        assert!(lines[..23].iter().all(|l| l.len() == 40));
        assert!(grid.contains('P') && grid.contains('F'));

        let rx = call(&tx, "step", json!({ "frames": 120, "snapshot": true, "detail": "full" }));
        server.before_frame(&mut game, W, H);
        server.advance(&mut game, Input::default(), 1, W, H, &mut |_| {});
        let reply = rx.recv().unwrap().unwrap();
        let text = reply["snapshot"].to_string();
        assert!(text.len() < 16_000, "full snapshot is {} bytes", text.len());
        let enemy = &reply["snapshot"]["tanks"][1];
        assert!(enemy["ai"]["last_action"].is_string(), "{enemy}");
        let engage = &reply["snapshot"]["engage"];
        assert_eq!(engage["tanks"].as_array().unwrap().len(), 4, "{engage}");
        assert!(engage["tanks"][0]["status"].is_string(), "{engage}");
        assert!(reply["snapshot"]["clusters"].is_array());
        assert!(reply["snapshot"]["command"]["enabled"].is_boolean(), "{}", reply["snapshot"]["command"]);
        assert!(reply["snapshot"]["command"]["orders"].is_object());
        if engage["built"] == true {
            assert_eq!(engage["slots"].as_array().unwrap().len(), 16, "{engage}");
        }
        let rx = call(&tx, "snapshot", json!({}));
        server.before_frame(&mut game, W, H);
        let compact = rx.recv().unwrap().unwrap();
        assert!(compact["engage"]["slots"].is_null(), "the slot table is full-detail only");
        assert!(compact.get("command").is_none(), "the command report is full-detail only");
        // An order of size, not a byte count: the floats' text runs a few
        // bytes longer or shorter whenever the round plays differently.
        assert!(compact.to_string().len() < 6_500, "compact snapshot is {} bytes", compact.to_string().len());
    }

    const INLINE_MAP: &str = r#"
version = 1
tanks = 2
cells."5,5" = { kind = "start" }
cells."30,15" = { kind = "bong" }
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
        assert_eq!(snap["bongs"][0]["x"], 960.0, "{}", snap["bongs"]);
        assert_eq!(snap["bongs"][0]["side"], "player");
        assert_eq!(snap["bongs"].as_array().map(Vec::len), Some(1), "a protect round has one bong");
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

    /// A waves restart reports the scheduler in `status.wave`, and the
    /// snapshot flags a tank still rolling in.
    #[test]
    fn a_waves_restart_reports_wave_status_and_entering_tanks() {
        let (mut server, tx) = DevServer::headless();
        let mut game = game(15);
        let rx = call(&tx, "restart", json!({ "spawn": "waves", "waves": 3, "wave_size": 2, "seed": 5 }));
        server.before_frame(&mut game, W, H);
        let status = rx.recv().unwrap().unwrap();
        assert_eq!(status["tanks"], 1, "nobody but the player at init: {status}");
        assert_eq!(status["wave"]["total"], 3, "{status}");
        assert_eq!(status["wave"]["index"], 0);
        let rx = call(&tx, "step", json!({ "frames": 1, "snapshot": true }));
        server.before_frame(&mut game, W, H);
        server.advance(&mut game, Input::default(), 1, W, H, &mut |_| {});
        let stepped = rx.recv().unwrap().unwrap();
        let tanks = stepped["snapshot"]["tanks"].as_array().unwrap();
        assert_eq!(tanks.len(), 2, "wave 1's first tank is rolling in: {stepped}");
        assert_eq!(tanks[1]["entering"], true);
        assert_eq!(tanks[0]["entering"], false);
        let rx = call(&tx, "status", json!({ "sections": ["all"] }));
        server.before_frame(&mut game, W, H);
        let status = rx.recv().unwrap().unwrap();
        assert_eq!(status["wave"]["index"], 1, "{status}");
        assert_eq!(status["wave"]["alive"], 1);
        assert_eq!(status["wave"]["pending"], 1);
        // A band round has no wave block.
        let rx = call(&tx, "restart", json!({ "spawn": "band", "enemies": 2 }));
        server.before_frame(&mut game, W, H);
        let status = rx.recv().unwrap().unwrap();
        assert!(status["wave"].is_null(), "{status}");
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
        server.advance(&mut game, Input::default(), 1, W, H, &mut |_| {});
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
        server.advance(&mut game, Input::default(), 1, W, H, &mut |_| {});
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
            let rx = call(&tx, "step", json!({ "frames": 300, "snapshot": true }));
            server.before_frame(&mut game, W, H);
            server.advance(&mut game, Input::default(), 1, W, H, &mut |_| {});
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

    // ----- the two modes and the builder -----

    /// One request, answered at the next frame boundary.
    fn ask(server: &mut DevServer, tx: &mpsc::Sender<Request>, session: &mut Session, method: &str, params: Value) -> Result<Value, String> {
        let rx = call(tx, method, params);
        server.before_frame(session, W, H);
        rx.recv().unwrap()
    }

    /// Where `click` presses the corners' button `name`: the middle of
    /// its rect as `status.ui.buttons` reports it, in window coordinates.
    fn corner_button(server: &mut DevServer, tx: &mpsc::Sender<Request>, session: &mut Session, name: &str) -> Value {
        ui_button(server, tx, session, "buttons", name)
    }

    /// Where `click` presses the button `name` of whatever stands over the
    /// round, as `status.ui.screen_buttons` reports it.
    fn screen_button(server: &mut DevServer, tx: &mpsc::Sender<Request>, session: &mut Session, name: &str) -> Value {
        ui_button(server, tx, session, "screen_buttons", name)
    }

    fn ui_button(server: &mut DevServer, tx: &mpsc::Sender<Request>, session: &mut Session, list: &str, name: &str) -> Value {
        let status = ask(server, tx, session, "status", json!({ "sections": ["all"] })).unwrap();
        let b = &status["ui"][list][name];
        let at = |k: &str| b[k].as_f64().unwrap_or_else(|| panic!("no {name} in `ui.{list}` of {status}"));
        json!({ "x": at("x") + at("w") / 2.0, "y": at("y") + at("h") / 2.0 })
    }

    /// BUILD: the builder is live afterwards, the round paused behind it.
    fn enter_build(server: &mut DevServer, tx: &mpsc::Sender<Request>, session: &mut Session) {
        let m = ask(server, tx, session, "build", json!({})).unwrap();
        assert_eq!(m["mode"], "build", "{m}");
    }

    #[test]
    fn build_enters_at_once_back_comes_back_and_the_game_only_tools_refuse_in_build_mode() {
        let (mut server, tx) = DevServer::headless();
        let mut s = game(31);
        let m = ask(&mut server, &tx, &mut s, "mode", json!({})).unwrap();
        assert_eq!(m["mode"], "play");
        assert!(m["back"].is_null(), "no BACK in play mode: {m}");
        assert!(m["open_menu"].is_null());
        assert_eq!(m["tool"], "brick");
        assert_eq!(m["category"], "wall");
        assert_eq!(m["map_name"], "untitled", "an inline map has no file stem");
        // BUILD mid-round asks nothing: the builder is live, the round
        // waits behind it and the round tools refuse.
        enter_build(&mut server, &tx, &mut s);
        assert!(!s.playing());
        let err = ask(&mut server, &tx, &mut s, "step", json!({ "frames": 1 })).unwrap_err();
        assert!(err.contains("play"), "{err}");
        for tool in TOOLS.iter().filter(|t| !t.in_build).map(|t| t.name) {
            assert!(ask(&mut server, &tx, &mut s, tool, json!({})).is_err(), "{tool} must refuse in build mode");
        }
        let status = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        assert_eq!(status["mode"], "build", "{status}");
        assert_eq!(status["back"], json!({ "puts_away": false }), "{status}");
        assert_eq!(status["builder"]["dirty"], false);
        assert_eq!(status["builder"]["tool"], "brick");
        // The tools that must keep working in build mode.
        for (tool, params) in [
            ("mode", json!({})),
            ("overlays", json!({ "hitboxes": true })),
            ("map_get", json!({})),
            ("tuning_get", json!({ "diff_only": true })),
            ("tuning_schema", json!({ "name_contains": "tank_speed" })),
            ("builder_tool", json!({})),
            ("builder_settings", json!({})),
            ("builder_map", json!({})),
        ] {
            ask(&mut server, &tx, &mut s, tool, params).unwrap_or_else(|e| panic!("{tool} in build mode: {e}"));
        }
        for tool in ["status", "mode", "screenshot", "overlays", "map_get", "tuning_get", "tuning_set", "tuning_reset", "tuning_schema", "restart", "build", "back", "play", "builder_tool", "builder_paint", "builder_undo", "builder_redo", "builder_settings", "builder_map", "builder_files", "builder_save", "click", "key"] {
            assert!(find(TOOLS, tool).unwrap_or_else(|| panic!("{tool} is advertised")).in_build, "{tool} must run in build mode");
        }
        // A second BUILD press in build mode is a no-op; `back` comes back
        // to the round where it stood, a paint since put away first.
        let m = ask(&mut server, &tx, &mut s, "build", json!({})).unwrap();
        assert_eq!(m["mode"], "build");
        let frame = s.game.frame();
        ask(&mut server, &tx, &mut s, "builder_paint", json!({ "cells": [[3, 3]], "tool": "iron" })).unwrap();
        let m = ask(&mut server, &tx, &mut s, "mode", json!({})).unwrap();
        assert_eq!(m["back"], json!({ "puts_away": true }), "{m}");
        let m = ask(&mut server, &tx, &mut s, "back", json!({})).unwrap();
        assert_eq!(m["mode"], "play", "{m}");
        assert!(m["back"].is_null());
        assert_eq!(s.game.frame(), frame, "the same round");
        assert_eq!(s.builder.map().cell(3, 3), s.game.map.cell(3, 3), "the paint put away");
        assert_eq!(ask(&mut server, &tx, &mut s, "back", json!({})).unwrap()["mode"], "play", "a no-op in play mode");
        // The end screen: `build` and `back` come back to it.
        s.game.debug_kill(0).unwrap();
        s.game.update(Input::default(), PHYSICS_FIXED_DT, W, H);
        let m = ask(&mut server, &tx, &mut s, "build", json!({})).unwrap();
        assert_eq!(m["mode"], "build", "{m}");
        let m = ask(&mut server, &tx, &mut s, "back", json!({})).unwrap();
        assert_eq!(m["mode"], "play", "{m}");
        assert_ne!(s.game.outcome(), crate::simulation::Outcome::Playing);
    }

    /// A session holding a seat in a room, plus the authoritative round
    /// the room is running and the room's end of the link.
    ///
    /// The room is answered by hand, as `net::round`'s tests answer one:
    /// a perfect loopback, the code, the start and a `Welcome` built from
    /// a round of its own. That round is deliberately unlike the
    /// session's local one - another seed, two enemies instead of four -
    /// so a reply that describes it cannot be mistaken for the local
    /// round's.
    struct Room {
        link: Loopback,
        game: Game,
    }

    impl Room {
        fn say(&mut self, msg: Msg) {
            self.link.send(&codec::encode(&msg));
        }

        /// Run the round on `ticks` ticks and send the snapshot they
        /// earned, stamped `server_ms` - the room's clock, which a test
        /// moves so the interpolator's render time reaches the snapshot
        /// it means to draw (render time trails the newest stamp by the
        /// interpolation delay, by design). The interval's events ride
        /// along, as a room server's do.
        fn tick(&mut self, ticks: u32, server_ms: u32) {
            let (w, h) = self.game.map.field_size();
            let mut events = Vec::new();
            for _ in 0..ticks {
                self.game.update(Input::default(), PHYSICS_FIXED_DT, w, h);
                events.extend(enc::wire_events(self.game.events()));
            }
            let mut snapshot = enc::snapshot(&self.game, [0; MAX_SEATS]);
            snapshot.server_ms = server_ms;
            snapshot.events = events;
            self.say(Msg::Snapshot(snapshot));
        }
    }

    /// A session in an online round, welcomed into `Room`'s round.
    fn online(seed: u64) -> (Session, Room) {
        let mut session = game(seed);
        let mut authority = Game::default();
        authority.enemy_count_override = Some(2);
        authority.level_overrides.mission = Some(crate::level::Mission::Protect);
        authority.level_overrides.spawn = Some(crate::level::SpawnKind::Band);
        authority.seed_override = Some(seed ^ 0xABC);
        authority.player_row_override = Some(3);
        // A room server's round has no mission banner to freeze behind.
        authority.show_intro = false;
        authority.map = session.game.map.clone();
        let (w, h) = authority.map.field_size();
        authority.init(w, h);

        let (client_end, room_end) = loopback::pair(LinkQuality::PERFECT, seed);
        let client = RoomClient::host(
            Box::new(client_end) as Box<dyn Transport>,
            Identity::new("dev", "tok-dev"),
            RoomSetup::default(),
        );
        session.go_online(OnlineRound::new(client, "ROOM"));
        // The create goes out on the first frame; the room answers it.
        session.update_online(&Intent::default(), PHYSICS_FIXED_DT);
        let mut room = Room { link: room_end, game: authority };
        let roster = vec![Seat { seat: 0, nick: "dev".into(), chassis: 3 }];
        let mut welcome =
            enc::welcome(&room.game, 0, roster, "{}".into(), [0; MAX_SEATS]).expect("the map serialises");
        welcome.protocol = PROTOCOL_VERSION;
        welcome.snapshot.server_ms = 1_000;
        room.say(Msg::Lobby(Lobby::RoomCreated { code: "AK7QX".into() }));
        room.say(Msg::Lobby(Lobby::Started));
        room.say(Msg::Welcome(welcome));
        session.update_online(&Intent::default(), PHYSICS_FIXED_DT);
        assert!(session.online.as_ref().and_then(AnyRound::game).is_some(), "the welcome built no replica");
        (session, room)
    }

    /// One windowed frame in `app.rs`'s order: the dev server at the
    /// boundary, then the online round's own frame. A snapshot's events
    /// are handed over exactly once (`net::interp`), so the boundary has
    /// to come first - as it does in the loop - for the server to see
    /// them.
    fn online_frame(server: &mut DevServer, session: &mut Session) {
        server.before_frame(session, W, H);
        session.update_online(&Intent::default(), PHYSICS_FIXED_DT);
    }

    /// In an online round every reading tool describes the round on
    /// screen - the room's replica - and `status` says whose round that
    /// is: the room, the seat, the buffer and how far the server has got.
    #[test]
    fn the_reading_tools_describe_the_online_round() {
        let (mut s, mut room) = online(51);
        let (mut server, tx) = DevServer::headless();
        online_frame(&mut server, &mut s);
        // Two snapshots: the one the replica is meant to land on, and a
        // later one that carries the room's clock far enough forward for
        // render time to reach it.
        room.game.debug_kill(1).expect("enemy in slot 1");
        room.tick(30, 1_500);
        room.tick(30, 4_000);
        online_frame(&mut server, &mut s);

        let st = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        assert_eq!(st["mode"], "online", "{st}");
        let round = &st["round"];
        assert_eq!(round["kind"], "online", "{round}");
        assert_eq!(round["room"], "AK7QX");
        assert_eq!(round["seat"], 0);
        assert_eq!(round["phase"], "playing");
        assert_eq!(round["replica"], true);
        assert_eq!(round["server_tick"], 60, "the room's own tick: {round}");
        assert!(round["buffer_ms"].is_i64(), "the snapshot buffer's depth: {round}");
        // The stage-2 readings ride along: what the interpolator is
        // doing and what the prediction has cost so far.
        assert!(round["interpolation"]["delay_ms"].is_number(), "{round}");
        assert!(round["interpolation"]["jitter_ms"].is_number(), "{round}");
        let prediction = &round["prediction"];
        assert!(prediction.is_object(), "a welcome built a sandbox: {round}");
        assert_eq!(prediction["error_buckets"].as_array().map(Vec::len), Some(6), "{prediction}");
        assert!(prediction["shots_drawn"].is_number() && prediction["lead_up"].is_number(), "{prediction}");
        assert_eq!(st["frame"], 30, "the replica's tick, the room's newest less the buffer: {st}");

        // The numbers are the room's round, not the local one standing
        // frozen behind it: another seed and two enemies, not four.
        assert_eq!(st["seed"], format!("{:#x}", room.game.round_seed()), "{st}");
        assert_ne!(st["seed"], format!("{:#x}", s.game.round_seed()));
        assert_eq!(st["tanks"], 3, "the replica's tanks: {st}");
        assert_eq!(st["enemies_alive"], 1, "one of the room's two enemies is a wreck: {st}");
        let snap = ask(&mut server, &tx, &mut s, "snapshot", json!({})).unwrap();
        assert_eq!(snap["tanks"].as_array().unwrap().len(), 3, "{snap}");
        // And the other readers answer about it rather than refusing.
        for (tool, params) in [
            ("terrain", json!({ "only": "damaged" })),
            ("events", json!({})),
            ("history", json!({})),
            ("nav_grid", json!({})),
            ("field", json!({ "target": "player" })),
            ("map_get", json!({})),
            ("lint", json!({ "source": "round" })),
            ("mode", json!({})),
            ("overlays", json!({ "hitboxes": true })),
            ("tuning_get", json!({ "diff_only": true })),
            ("builder_files", json!({})),
        ] {
            ask(&mut server, &tx, &mut s, tool, params).unwrap_or_else(|e| panic!("{tool} in an online round: {e}"));
        }
        // The replica's own events and track rows are banked as the
        // snapshots move it on, since nothing `advance`s it here.
        let st = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        assert!(st["events_kept"].as_u64().unwrap() > 0, "no event of the replica's was kept: {st}");
        assert!(st["history_frames"].as_u64().unwrap() > 0, "no history of the replica's: {st}");
        // An overlay flag landed on the round that is drawn.
        assert!(s.shown().debug_overlays.hitboxes, "the overlay flag went to the local round");
        assert!(!s.game.debug_overlays.hitboxes);
    }

    /// The other half of the rule: anything that would write refuses, by
    /// name, and says what to do instead. Giving the seat up comes back
    /// to the local round, and every tool works again.
    #[test]
    fn the_writing_tools_refuse_in_an_online_round() {
        let (mut s, _room) = online(52);
        let (mut server, tx) = DevServer::headless();
        for spec in TOOLS.iter().filter(|t| !t.online) {
            let tool = spec.name;
            assert!(!spec.read_only, "{tool} reads only - it should describe the online round, not refuse");
            let err = ask(&mut server, &tx, &mut s, tool, json!({})).unwrap_err();
            assert!(err.contains("AK7QX"), "{tool}: {err}");
            assert!(err.contains("escape"), "{tool}: {err}");
        }
        // The round that is not this window's is untouched by the asking.
        assert_eq!(s.mode(), Driver::Online);
        assert_eq!(s.game.frame(), 0, "the local round took a step");
        assert!(!server.lockstep(), "a refused `pause` still froze the game");
        // Naming a tool that does not exist still reads as unknown.
        assert!(ask(&mut server, &tx, &mut s, "nonsense", json!({})).unwrap_err().contains("unknown tool"));

        // Esc gives the seat up; the local round is back and answers.
        let m = ask(&mut server, &tx, &mut s, "key", json!({ "key": "escape" })).unwrap();
        assert_eq!(m["mode"], "play", "{m}");
        let st = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        assert_eq!(st["round"]["kind"], "local", "{st}");
        assert!(st["round"]["room"].is_null());
        assert_eq!(st["tanks"], 5, "the local round is the one on screen again: {st}");
        // A `step` is answered from `advance`, one rendered frame later.
        let rx = call(&tx, "step", json!({ "frames": 1 }));
        server.before_frame(&mut s, W, H);
        server.advance(&mut s.game, Input::default(), 1, W, H, &mut |_| {});
        assert_eq!(rx.recv().unwrap().unwrap()["frame"], 1);
    }

    /// **`input` drives this window's seat in a room** (BB-58): it
    /// stands in for the keyboard, which the seat's intent is read from,
    /// so a direction and a press put through it go to the room as that
    /// seat's packets - `app.rs`'s frame, shaped here and handed to the
    /// round - and the keys again once its frames run out. The player-2
    /// fields and the overlay cycle reach nobody there and are refused by
    /// name.
    #[test]
    fn input_drives_the_seat_in_an_online_round() {
        let (mut s, mut room) = online(54);
        let (mut server, tx) = DevServer::headless();
        for field in ["p2_fire", "cycle_overlays"] {
            let err = ask(&mut server, &tx, &mut s, "input", json!({ field: true })).unwrap_err();
            assert!(err.contains(field) && err.contains("escape"), "{err}");
        }
        let r = ask(&mut server, &tx, &mut s, "input", json!({ "move_dir": "right", "fire": true, "frames": 3 })).unwrap();
        assert_eq!(r["frames"], 3, "{r}");
        let mut heard = Vec::new();
        for _ in 0..5 {
            server.before_frame(&mut s, W, H);
            let intent = server.shape_input(Input::default()).seat(0);
            s.update_online(&intent, PHYSICS_FIXED_DT);
        }
        room.link.drain(&mut heard);
        let sent: Vec<Intent> = heard.iter().filter_map(|m| if let Msg::Intent(i) = m { Some(i.intent()) } else { None }).collect();
        let driven = sent.iter().filter(|i| i.fire && i.move_dir == Some(Dir::Right)).count();
        assert_eq!(driven, 3, "the room heard {sent:?}");
        assert_eq!(sent.last().map(|i| (i.fire, i.move_dir)), Some((false, None)), "the keys again after: {sent:?}");
        assert_eq!(s.mode(), Driver::Online);
        assert_eq!(s.game.frame(), 0, "the local round took a step");
    }

    /// The corners' own way out of a room: a `click` on `LEAVE` lands on
    /// the hit test a finger lands on, and comes back to the local round
    /// the way Esc does.
    #[test]
    fn a_click_on_leave_gives_the_seat_up() {
        let (mut s, _room) = online(53);
        let (mut server, tx) = DevServer::headless();
        let leave = corner_button(&mut server, &tx, &mut s, "leave");
        // A press anywhere else on the replica does nothing.
        let m = ask(&mut server, &tx, &mut s, "click", json!({ "x": 10.0, "y": 200.0 })).unwrap();
        assert_eq!(m["mode"], "online", "{m}");
        let m = ask(&mut server, &tx, &mut s, "click", leave).unwrap();
        assert_eq!(m["mode"], "play", "{m}");
        let st = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        assert_eq!(st["round"]["kind"], "local", "{st}");
    }

    #[test]
    fn builder_paint_obeys_the_stroke_rules_and_undo_redo_reverse_it() {
        let (mut server, tx) = DevServer::headless();
        let mut s = game(32);
        // A known-empty row: the inline map has nothing on row 5 past col 5.
        ask(&mut server, &tx, &mut s, "restart", json!({ "map_toml": INLINE_MAP, "seed": 2 })).unwrap();
        enter_build(&mut server, &tx, &mut s);
        let t = ask(&mut server, &tx, &mut s, "builder_tool", json!({ "tool": "iron" })).unwrap();
        assert_eq!(t["tool"], "iron");
        assert_eq!(t["category"], "wall");
        let cats = t["categories"].as_array().unwrap();
        assert_eq!(cats.len(), 6);
        assert_eq!(cats[0]["name"], "wall");
        assert_eq!(cats[0]["current"], "iron");
        assert_eq!(cats[3]["name"], "plant");
        assert_eq!(cats[3]["tools"].as_array().unwrap().len(), 16, "{}", cats[3]);
        assert_eq!(cats[5]["tools"].as_array().unwrap().len(), 21, "{}", cats[5]);
        let err = ask(&mut server, &tx, &mut s, "builder_tool", json!({ "tool": "granite" })).unwrap_err();
        assert!(err.contains("brick") && err.contains("eraser"), "{err}");

        // The restart's map load is already one step on the stack.
        let base = s.builder.history().undo_depth() as u64;
        let r = ask(&mut server, &tx, &mut s, "builder_paint", json!({ "cells": [[10, 5], [11, 5], [12, 5]] })).unwrap();
        let changes = r["changes"].as_array().unwrap();
        assert_eq!(changes.len(), 3, "{r}");
        assert_eq!(changes[1]["col"], 11);
        assert!(changes[1]["before"].is_null());
        assert_eq!(changes[1]["after"]["kind"], "wall");
        assert_eq!(changes[1]["after"]["material"], "iron");
        assert_eq!(r["undo_depth"], base + 1);
        // The same brush on an iron cell toggle-erases it.
        let r = ask(&mut server, &tx, &mut s, "builder_paint", json!({ "cells": [[11, 5]] })).unwrap();
        let changes = r["changes"].as_array().unwrap();
        assert_eq!(changes.len(), 1, "{r}");
        assert_eq!(changes[0]["before"]["material"], "iron");
        assert!(changes[0]["after"].is_null());
        assert_eq!(r["undo_depth"], base + 2);
        assert_eq!(s.builder.map().cell(11, 5), None);
        // Undo brings it back; redo takes it away again.
        let u = ask(&mut server, &tx, &mut s, "builder_undo", json!({})).unwrap();
        assert_eq!(u["undone"], 1, "{u}");
        assert_eq!(u["undo_depth"], base + 1);
        assert_eq!(u["redo_depth"], 1);
        assert_eq!(u["cells"].as_array().unwrap().len(), 1);
        assert_eq!(s.builder.map().cell(11, 5), Some(&crate::map::CellObject::Wall { material: crate::obstacle::Material::Iron }));
        let r = ask(&mut server, &tx, &mut s, "builder_redo", json!({ "steps": 5 })).unwrap();
        assert_eq!(r["redone"], 1, "{r}");
        assert_eq!(r["redo_depth"], 0);
        assert_eq!(s.builder.map().cell(11, 5), None);
        // Right button erases whatever the brush; a bad cell list is an error.
        let r = ask(&mut server, &tx, &mut s, "builder_paint", json!({ "cells": [[10, 5]], "tool": "road", "button": "right" })).unwrap();
        assert!(r["changes"][0]["after"].is_null(), "{r}");
        assert!(ask(&mut server, &tx, &mut s, "builder_paint", json!({ "cells": [] })).is_err());
        assert!(ask(&mut server, &tx, &mut s, "builder_paint", json!({ "cells": [[1]] })).is_err());
        let m = ask(&mut server, &tx, &mut s, "mode", json!({})).unwrap();
        assert_eq!(m["dirty"], true);
        assert_eq!(m["tool"], "road");
    }

    /// `builder_tool {shape}` sets how the brush paints and `builder_paint`
    /// lays the shape: a rectangle between the first and last cells, a
    /// fill from the one cell, a scatter's footprints, each one step; a
    /// fill past `builder_fill_max_cells` changes nothing and says why in
    /// `message`, as `status.builder` does.
    #[test]
    fn builder_shapes_lay_rects_fills_and_scatters() {
        let (mut server, tx) = DevServer::headless();
        let mut s = game(35);
        ask(&mut server, &tx, &mut s, "restart", json!({ "map_toml": INLINE_MAP, "seed": 2 })).unwrap();
        enter_build(&mut server, &tx, &mut s);
        let base = s.builder.history().undo_depth() as u64;
        let t = ask(&mut server, &tx, &mut s, "builder_tool", json!({ "tool": "water", "shape": "rect" })).unwrap();
        assert_eq!((&t["tool"], &t["shape"]), (&json!("water"), &json!("rect")), "{t}");
        let err = ask(&mut server, &tx, &mut s, "builder_tool", json!({ "shape": "lasso" })).unwrap_err();
        assert!(err.contains("scatter"), "{err}");
        let r = ask(&mut server, &tx, &mut s, "builder_paint", json!({ "cells": [[12, 9], [10, 8]] })).unwrap();
        let changes = r["changes"].as_array().unwrap();
        assert_eq!(changes.len(), 6, "{r}");
        assert!(changes.iter().all(|c| c["after"]["kind"] == "water"), "{r}");
        assert_eq!(r["undo_depth"], base + 1);
        assert!(r["message"].is_null());
        let st = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        assert_eq!((&st["builder"]["shape"], &st["builder"]["rect"]), (&json!("rect"), &Value::Null), "{}", st["builder"]);
        // A fill of road over the water: the six cells joined to the one pressed.
        let r = ask(&mut server, &tx, &mut s, "builder_paint", json!({ "cells": [[11, 8]], "tool": "road", "shape": "fill" })).unwrap();
        assert_eq!(r["changes"].as_array().unwrap().len(), 6, "{r}");
        assert_eq!(r["undo_depth"], base + 2);
        // A scatter of trees round a short line: empty cells only, the start kept.
        let r = ask(&mut server, &tx, &mut s, "builder_paint", json!({ "cells": [[3, 5], [7, 5]], "tool": "tree", "shape": "scatter" })).unwrap();
        let changes = r["changes"].as_array().unwrap();
        assert!(!changes.is_empty() && changes.iter().all(|c| c["before"].is_null() && c["after"]["kind"] == "tree"), "{r}");
        assert_eq!(s.builder.map().cell(5, 5), Some(&crate::map::CellObject::Start));
        // A field past the cap: a fill of its open ground is refused.
        ask(&mut server, &tx, &mut s, "builder_settings", json!({ "size": [80, 60] })).unwrap();
        let depth = s.builder.history().undo_depth() as u64;
        let r = ask(&mut server, &tx, &mut s, "builder_paint", json!({ "cells": [[60, 40]], "tool": "tall_grass", "shape": "fill" })).unwrap();
        assert!(r["changes"].as_array().unwrap().is_empty(), "{r}");
        assert_eq!(r["undo_depth"], depth);
        let max = crate::tuning::Tuning::DEFAULT.builder_fill_max_cells.to_string();
        assert!(r["message"].as_str().is_some_and(|m| m.contains(&max)), "{r}");
        let st = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        assert_eq!(st["builder"]["message"], r["message"]);
    }

    /// FILE > LOAD's thumbnails: the list's page makes them a picture a
    /// frame, `status.builder.thumbnails` names each with its size in cells
    /// while the list is open, and none once it closes.
    #[test]
    fn the_load_list_reports_its_thumbnails_while_it_is_open() {
        let (mut server, tx) = DevServer::headless();
        let mut s = game(36);
        enter_build(&mut server, &tx, &mut s);
        let press = |name: &str, server: &mut DevServer, s: &mut Session| {
            let st = ask(server, &tx, s, "status", json!({ "sections": ["all"] })).unwrap();
            let b = &st["builder"]["buttons"][name];
            assert!(b.is_object(), "no {name}: {}", st["builder"]["buttons"]);
            let (x, y) = (b["x"].as_f64().unwrap() + b["w"].as_f64().unwrap() / 2.0, b["y"].as_f64().unwrap() + b["h"].as_f64().unwrap() / 2.0);
            ask(server, &tx, s, "click", json!({ "x": x, "y": y })).unwrap();
        };
        let st = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        assert_eq!(st["builder"]["thumbnails"], json!([]), "nothing while the list is closed");
        press("file", &mut server, &mut s);
        press("load", &mut server, &mut s);
        // A frame a key press the list does not take.
        for _ in 0..12 {
            ask(&mut server, &tx, &mut s, "key", json!({ "key": "left" })).unwrap();
        }
        let st = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        let thumbs = st["builder"]["thumbnails"].as_array().expect("a list").clone();
        assert!(thumbs.len() >= 2, "{thumbs:?}");
        let buttons = &st["builder"]["buttons"];
        for t in &thumbs {
            let name = t["name"].as_str().unwrap();
            assert!(buttons[format!("map_{name}")].is_object(), "{name} is a row of the page");
            if t["picture"] == true {
                assert!(t["cells"][0].as_f64().unwrap() > 0.0 && t["cells"][1].as_f64().unwrap() > 0.0, "{t}");
            }
        }
        ask(&mut server, &tx, &mut s, "key", json!({ "key": "escape" })).unwrap();
        let st = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        assert_eq!(st["builder"]["thumbnails"], json!([]), "none once it closes");
    }

    /// `builder_select` and `builder_stamp` drive the select tool the way a
    /// drag and the strip do - a rectangle, a move, a copy pasted and
    /// placed where `at` says, a stamp kept and one put down, each edit one
    /// undo step - and `status.builder` reports it; the strip's live
    /// buttons come by name for a `click`, and the keys copy, paste and
    /// let go.
    #[test]
    fn builder_select_and_stamp_drive_the_select_tool() {
        let (mut server, tx) = DevServer::headless();
        let mut s = game(34);
        ask(&mut server, &tx, &mut s, "restart", json!({ "map_toml": INLINE_MAP, "seed": 2 })).unwrap();
        enter_build(&mut server, &tx, &mut s);
        let iron = crate::map::CellObject::Wall { material: crate::obstacle::Material::Iron };
        let base = s.builder.history().undo_depth() as u64;
        // A rectangle round the iron wall at (20, 8).
        let r = ask(&mut server, &tx, &mut s, "builder_select", json!({ "rect": [21, 9, 19, 7] })).unwrap();
        assert_eq!(r["tool"], "select", "{r}");
        assert_eq!(r["selection"], json!({ "col": 19, "row": 7, "cols": 3, "rows": 3 }));
        let st = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        let b = &st["builder"];
        assert_eq!((&b["tool"], &b["shape"], &b["selection"]["cols"]), (&json!("select"), &json!("pen"), &json!(3)), "{b}");
        assert!(b["buttons"]["sel_copy"].is_object() && b["buttons"]["sel_paste"].is_null(), "PASTE is dim with nothing copied: {}", b["buttons"]);
        // Two cells right: one step, the wall with it.
        let r = ask(&mut server, &tx, &mut s, "builder_select", json!({ "move_by": [2, 0] })).unwrap();
        assert_eq!(r["changes"].as_array().unwrap().len(), 2, "{r}");
        assert_eq!(r["undo_depth"], base + 1);
        assert_eq!(s.builder.map().cell(22, 8), Some(&iron));
        assert_eq!(r["selection"]["col"], 21);
        // Copied, pasted as a ghost, put down with its top-left at (5, 12).
        ask(&mut server, &tx, &mut s, "builder_select", json!({ "action": "copy" })).unwrap();
        let r = ask(&mut server, &tx, &mut s, "builder_select", json!({ "action": "paste", "at": [8, 12] })).unwrap();
        assert_eq!(r["ghost"]["cells"], 1, "{r}");
        assert_eq!(r["clipboard"], json!({ "cols": 3, "rows": 3, "cells": 1 }));
        let r = ask(&mut server, &tx, &mut s, "builder_select", json!({ "action": "place", "at": [5, 12] })).unwrap();
        assert!(r["ghost"].is_null(), "{r}");
        assert_eq!(s.builder.map().cell(6, 13), Some(&iron));
        assert_eq!(r["undo_depth"], base + 2);
        assert!(ask(&mut server, &tx, &mut s, "builder_select", json!({ "action": "spin" })).is_err());
        assert!(ask(&mut server, &tx, &mut s, "builder_select", json!({ "rect": [1, 2] })).is_err());
        assert!(ask(&mut server, &tx, &mut s, "builder_select", json!({ "action": "place" })).is_err(), "no ghost to place");
        // The stamps: listed, one kept, the bunker put down.
        let r = ask(&mut server, &tx, &mut s, "builder_stamp", json!({})).unwrap();
        let keys: Vec<&str> = r["stamps"].as_array().unwrap().iter().map(|s| s["key"].as_str().unwrap()).collect();
        assert_eq!(keys, ["fort", "bunker", "river-bend"]);
        ask(&mut server, &tx, &mut s, "builder_select", json!({ "action": "stamp" })).unwrap();
        let r = ask(&mut server, &tx, &mut s, "builder_stamp", json!({ "name": "bunker", "at": [2, 2], "place": true })).unwrap();
        assert_eq!(r["stamps"].as_array().unwrap().len(), 4, "{r}");
        assert_eq!(r["stamps"][3]["key"], "saved_1");
        assert!(!r["changes"].as_array().unwrap().is_empty());
        assert_eq!(s.builder.map().cell(3, 2), Some(&iron));
        assert_eq!(s.builder.map().start_cell(), Some((5, 5)), "the bunker's empty middle left the start where it was");
        assert!(ask(&mut server, &tx, &mut s, "builder_stamp", json!({ "name": "castle" })).is_err());
        // The strip's DELETE by a click on it.
        ask(&mut server, &tx, &mut s, "builder_select", json!({ "rect": [21, 7, 23, 9] })).unwrap();
        let st = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        let del = &st["builder"]["buttons"]["sel_delete"];
        let (x, y) = (del["x"].as_f64().unwrap() + del["w"].as_f64().unwrap() / 2.0, del["y"].as_f64().unwrap() + del["h"].as_f64().unwrap() / 2.0);
        ask(&mut server, &tx, &mut s, "click", json!({ "x": x, "y": y })).unwrap();
        assert_eq!(s.builder.map().cell(22, 8), None);
        // The keys: undo, copy, paste, and Escape twice - the ghost, then
        // the selection.
        ask(&mut server, &tx, &mut s, "key", json!({ "key": "undo" })).unwrap();
        assert_eq!(s.builder.map().cell(22, 8), Some(&iron));
        ask(&mut server, &tx, &mut s, "key", json!({ "key": "copy" })).unwrap();
        ask(&mut server, &tx, &mut s, "key", json!({ "key": "paste" })).unwrap();
        assert!(s.builder.ghost().is_some());
        ask(&mut server, &tx, &mut s, "key", json!({ "key": "escape" })).unwrap();
        assert!(s.builder.ghost().is_none() && s.builder.selection().is_some());
        ask(&mut server, &tx, &mut s, "key", json!({ "key": "escape" })).unwrap();
        assert!(s.builder.selection().is_none());
        let m = ask(&mut server, &tx, &mut s, "builder_tool", json!({ "tool": "road" })).unwrap();
        assert_eq!((&m["tool"], &m["shape"]), (&json!("road"), &json!("pen")));
    }

    /// `builder_map {clear}` is FILE > CLEAR MAP: the cells go, the
    /// settings stay, the baseline stays, and it takes no map.
    #[test]
    fn builder_map_clear_empties_the_canvas_and_takes_no_map() {
        let (mut server, tx) = DevServer::headless();
        let mut s = game(33);
        enter_build(&mut server, &tx, &mut s);
        let before = ask(&mut server, &tx, &mut s, "builder_map", json!({})).unwrap();
        assert_eq!(before["dirty"], false);
        assert!(!s.builder.map().cells.is_empty(), "the default map has cells");
        let settings = s.builder.settings();
        let r = ask(&mut server, &tx, &mut s, "builder_map", json!({ "clear": true })).unwrap();
        assert!(s.builder.map().cells.is_empty(), "{r}");
        assert_eq!(r["dirty"], true, "{r}");
        assert_eq!(r["diff"]["added"], 0, "{r}");
        assert_eq!(r["diff"]["removed"], s.builder.baseline().cells.len(), "{r}");
        assert_eq!(s.builder.settings(), settings, "settings survive a clear");
        assert_eq!(s.builder.history().undo_depth(), 1);
        let err = ask(&mut server, &tx, &mut s, "builder_map", json!({ "clear": true, "name": "default" })).unwrap_err();
        assert!(err.contains("clear"), "{err}");
        let err = ask(&mut server, &tx, &mut s, "builder_map", json!({ "clear": true, "map_toml": "version = 1" })).unwrap_err();
        assert!(err.contains("clear"), "{err}");
        assert!(s.builder.map().cells.is_empty(), "a refused call changes nothing");
        // `clear: false` is the plain read.
        let r = ask(&mut server, &tx, &mut s, "builder_map", json!({ "clear": false })).unwrap();
        assert_eq!(r["dirty"], true, "{r}");
        assert_eq!(s.builder.history().undo_depth(), 1);
    }

    #[test]
    fn builder_settings_round_trip_null_as_auto_and_report_cli_overrides() {
        let (mut server, tx) = DevServer::headless();
        let mut s = game(33);
        enter_build(&mut server, &tx, &mut s);
        let r = ask(&mut server, &tx, &mut s, "builder_settings", json!({ "tanks": 3, "mission": "hunt" })).unwrap();
        assert_eq!(r["tanks"], 3, "{r}");
        assert_eq!(r["mission"], "hunt");
        assert!(r["spawn"].is_string(), "the map's own plan, whatever it ships with: {r}");
        assert!(r["tank"].is_string(), "the default map names a chassis: {r}");
        // The test round pins the enemy count, mission and spawn plan the
        // way `-e`/`--mission`/`--spawn` would.
        assert_eq!(r["cli_overrides"]["tanks"], true, "{r}");
        assert_eq!(r["cli_overrides"]["mission"], true);
        assert_eq!(r["cli_overrides"]["spawn"], true);
        assert_eq!(r["cli_overrides"]["tank"], false);
        assert_eq!(r["cli_overrides"]["waves"], false);
        assert_eq!(s.builder.history().undo_depth(), 2, "one step per changed field");
        let r = ask(&mut server, &tx, &mut s, "builder_settings", json!({ "tanks": null, "tank": "titan", "tank2": "scout", "tier_start": "heavy" })).unwrap();
        assert!(r["tanks"].is_null(), "{r}");
        assert_eq!(r["tank"], "titan");
        assert_eq!(r["tank2"], "scout");
        assert_eq!(r["cli_overrides"]["tank2"], false);
        assert_eq!(r["tier_start"], "heavy");
        let m = ask(&mut server, &tx, &mut s, "builder_map", json!({})).unwrap();
        assert!(m["toml"].as_str().unwrap().contains("tank2 = \"scout\""), "{m}");
        assert!(m["diff"]["settings"].as_array().unwrap().iter().any(|f| f == "tank2"), "{m}");
        assert_eq!(r["mission"], "hunt", "an absent field is untouched");
        let err = ask(&mut server, &tx, &mut s, "builder_settings", json!({ "mission": "conquer" })).unwrap_err();
        assert!(err.contains("protect") && err.contains("destroy"), "{err}");
        let err = ask(&mut server, &tx, &mut s, "builder_settings", json!({ "tank": "tractor" })).unwrap_err();
        assert!(err.contains("titan"), "{err}");
        // Null on mission/spawn means the default.
        let r = ask(&mut server, &tx, &mut s, "builder_settings", json!({ "mission": null })).unwrap();
        assert_eq!(r["mission"], "protect");
        // Reset reverts everything, as one undoable step.
        let m = ask(&mut server, &tx, &mut s, "builder_map", json!({})).unwrap();
        assert_eq!(m["dirty"], true);
        assert_eq!(m["diff"]["settings"], json!(["tank", "tank2", "tier_start"]), "{}", m["diff"]);
        let r = ask(&mut server, &tx, &mut s, "builder_settings", json!({ "reset": true })).unwrap();
        let baseline = crate::editor::MapSettings::of(s.builder.baseline());
        assert_eq!(r["tank"], json!(baseline.tank.map(TankKind::name)), "{r}");
        assert_eq!(r["tier_start"], json!(baseline.tier_start.map(Tier::name)), "{r}");
        assert!(!s.builder.dirty());
        ask(&mut server, &tx, &mut s, "builder_undo", json!({})).unwrap();
        assert!(s.builder.dirty());
    }

    #[test]
    fn builder_map_reports_the_diff_and_loads_a_map_as_the_new_baseline() {
        let (mut server, tx) = DevServer::headless();
        let mut s = game(34);
        enter_build(&mut server, &tx, &mut s);
        let m = ask(&mut server, &tx, &mut s, "builder_map", json!({})).unwrap();
        assert_eq!(m["dirty"], false);
        assert_eq!(m["name"], "untitled");
        assert!(m["toml"].as_str().unwrap().contains("version = 1"));
        // Paint the first cell the default map leaves empty.
        let free =(0..40).flat_map(|c| (0..23).map(move |r| (c, r))).find(|&(c, r)| s.builder.map().cell(c, r).is_none()).unwrap();
        ask(&mut server, &tx, &mut s, "builder_paint", json!({ "cells": [[free.0, free.1]], "tool": "sandbag" })).unwrap();
        let m = ask(&mut server, &tx, &mut s, "builder_map", json!({})).unwrap();
        assert_eq!(m["dirty"], true, "{m}");
        assert_eq!(m["diff"]["added"], 1);
        assert_eq!(m["diff"]["removed"], 0);
        assert!(m["toml"].as_str().unwrap().contains("sandbag"));
        // Loading clears dirty (new baseline) and is one undo step.
        let before = s.builder.history().undo_depth();
        let m = ask(&mut server, &tx, &mut s, "builder_map", json!({ "map_toml": INLINE_MAP })).unwrap();
        assert_eq!(m["dirty"], false, "{m}");
        assert_eq!(m["cells"], 3);
        assert_eq!(m["name"], "untitled");
        assert_eq!(s.builder.history().undo_depth(), before + 1);
        // The round's own map is untouched until PLAY.
        let g = ask(&mut server, &tx, &mut s, "map_get", json!({})).unwrap();
        assert!(g["cells"].as_u64().unwrap() > 100, "still the default map: {g}");
        assert!(ask(&mut server, &tx, &mut s, "builder_map", json!({ "map_toml": INLINE_MAP, "map": "maps/default.toml" })).is_err());
        assert!(ask(&mut server, &tx, &mut s, "builder_map", json!({ "map_toml": "version = \"x\"" })).unwrap_err().starts_with("map_toml:"));
    }

    #[test]
    fn play_starts_a_round_on_the_edited_map_and_leaves_it_frozen() {
        let (mut server, tx) = DevServer::headless();
        let mut s = game(35);
        ask(&mut server, &tx, &mut s, "restart", json!({ "map_toml": INLINE_MAP, "seed": 9 })).unwrap();
        assert!(ask(&mut server, &tx, &mut s, "play", json!({})).unwrap_err().contains("already in play mode"));
        enter_build(&mut server, &tx, &mut s);
        ask(&mut server, &tx, &mut s, "builder_paint", json!({ "cells": [[10, 5]], "tool": "iron" })).unwrap();
        // Lockstep is off in build mode (nothing to hold still) and `play`
        // turns it on for the new round.
        server.lockstep = false;
        let status = ask(&mut server, &tx, &mut s, "play", json!({})).unwrap();
        assert_eq!(status["mode"], "play", "{status}");
        assert_eq!(status["lockstep"], true);
        assert_eq!(status["frame"], 0);
        assert_eq!(status["map"]["cells"], 4);
        assert_eq!(status["seed"], "0x9", "the pinned seed stays pinned");
        assert_eq!(s.game.map.cell(10, 5), Some(&crate::map::CellObject::Wall { material: crate::obstacle::Material::Iron }));
        assert!(!s.game.show_intro);
        assert!(server.lockstep());
        // The history ring and the event stream start over with the round.
        let ev = ask(&mut server, &tx, &mut s, "events", json!({ "limit": 4096 })).unwrap();
        assert_eq!(ev["events"].as_array().unwrap().last().unwrap()["event"], "round_started", "{ev}");
        let step = call(&tx, "step", json!({ "frames": 2, "snapshot": false }));
        server.before_frame(&mut s, W, H);
        server.advance(&mut s.game, Input::default(), 1, W, H, &mut |_| {});
        assert_eq!(step.recv().unwrap().unwrap()["frame"], 2);
        // The edit survives the round trip back into the builder.
        enter_build(&mut server, &tx, &mut s);
        assert_eq!(s.builder.map().cell(10, 5), Some(&crate::map::CellObject::Wall { material: crate::obstacle::Material::Iron }));
        let status = ask(&mut server, &tx, &mut s, "play", json!({ "intro": true })).unwrap();
        assert!(status["intro_seconds_left"].as_f64().unwrap() > 0.0, "{status}");
    }

    /// A level's end screen answers `click` and `key` the way `app.rs`
    /// answers the mouse and the keyboard: NEXT LEVEL by its button, PLAY
    /// AGAIN by Enter after a loss, and `status` names the level.
    #[test]
    fn a_levels_end_screen_takes_clicks_and_enter() {
        let (mut server, tx) = DevServer::headless();
        // Past Boot Camp, level 0, to the first level with an enemy.
        let mut campaign = crate::levels::Campaign::new(crate::levels::Levels::shipped(), None);
        campaign.won(0);
        let mut game = Game::default();
        game.enemy_count_override = Some(1);
        game.seed_override = Some(3);
        game.map = campaign.map(1).expect("level 2 opens");
        let (w, h) = game.map.field_size();
        game.init(w, h);
        let mut s = Session::new(game);
        s.set_campaign(campaign);
        let status = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        assert_eq!(status["level"]["number"], 1, "{status}");
        assert_eq!(status["level"]["map"], "lotus-lagoon");
        assert_eq!(status["stats"]["enemies"], 1);

        let enemy = s.game.world.query::<&crate::tank::Tank>().with::<&crate::ai::Ai>().iter().map(|t| t.owner_slot()).min().unwrap();
        s.game.debug_kill(enemy).unwrap();
        s.game.update(Input::default(), crate::PHYSICS_FIXED_DT, w, h);
        let status = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        assert_eq!((status["outcome"].as_str(), status["stats"]["destroyed"].as_u64()), (Some("won"), Some(1)), "{status}");
        let at = screen_button(&mut server, &tx, &mut s, "next");
        let m = ask(&mut server, &tx, &mut s, "click", at).unwrap();
        assert_eq!(m["level"]["number"], 2, "{m}");
        assert_eq!(m["mode"], "play");
        assert!(server.lockstep(), "a new round, frozen like `restart`'s");

        s.game.debug_kill(0).unwrap();
        let (w, h) = s.game.map.field_size();
        s.game.update(Input::default(), crate::PHYSICS_FIXED_DT, w, h);
        assert_eq!(s.game.outcome(), crate::simulation::Outcome::Lost);
        let m = ask(&mut server, &tx, &mut s, "key", json!({ "key": "enter" })).unwrap();
        assert_eq!(m["level"]["number"], 2, "Enter after a loss is the same level again: {m}");
        assert_eq!(s.game.outcome(), crate::simulation::Outcome::Playing);
    }

    /// The level select through the tools: the HUD's level button and
    /// The pause button left of BUILD is the P key with no keyboard: a
    /// click pauses the round, `status.ui` keeps reporting it where it
    /// stood, and a second click takes the round on again.
    #[test]
    fn the_pause_button_pauses_and_resumes_the_round() {
        let (mut server, tx) = DevServer::headless();
        let mut game = Game::default();
        game.enemy_count_override = Some(1);
        game.seed_override = Some(3);
        game.init(crate::DEFAULT_SCREEN_WIDTH as f32, crate::DEFAULT_SCREEN_HEIGHT as f32);
        let mut s = Session::new(game);
        assert!(!s.game.paused);
        let button = corner_button(&mut server, &tx, &mut s, "pause");
        let build = corner_button(&mut server, &tx, &mut s, "build");
        assert!(button["x"].as_f64() < build["x"].as_f64(), "pause stands left of BUILD: {button} {build}");
        ask(&mut server, &tx, &mut s, "click", button.clone()).unwrap();
        assert!(s.game.paused, "a click pauses the round");
        assert!(s.play_chrome().paused, "the button shows the play triangle");
        assert_eq!(corner_button(&mut server, &tx, &mut s, "pause"), button, "the button stays where it was");
        ask(&mut server, &tx, &mut s, "click", button).unwrap();
        assert!(!s.game.paused, "a second click takes the round on again");
    }

    /// Esc open it, a locked tile is no button, the arrows and Enter start
    /// a level reached, and `step` refuses by name while the screen stands
    /// over the round rather than waiting for frames that never run.
    #[test]
    fn the_level_select_takes_clicks_and_keys() {
        let (mut server, tx) = DevServer::headless();
        let mut campaign = crate::levels::Campaign::new(crate::levels::Levels::shipped(), None);
        campaign.won(0);
        let mut game = Game::default();
        game.enemy_count_override = Some(1);
        game.seed_override = Some(3);
        game.map = campaign.map(1).expect("level 2 opens");
        let (w, h) = game.map.field_size();
        game.init(w, h);
        let mut s = Session::new(game);
        s.set_campaign(campaign);
        let button = corner_button(&mut server, &tx, &mut s, "level");
        let m = ask(&mut server, &tx, &mut s, "click", button).unwrap();
        assert_eq!((m["levels_open"].as_bool(), m["levels_focus"].as_u64()), (Some(true), Some(1)), "{m}");
        let err = ask(&mut server, &tx, &mut s, "step", json!({ "frames": 1 })).unwrap_err();
        assert!(err.contains("level select"), "{err}");

        // Only the open tiles are buttons: level 2 is locked, and a press
        // on it - where the window draws it, the chrome's area of a window
        // the size of play's bitmap - does nothing.
        let status = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        let screen = &status["ui"]["screen_buttons"];
        assert!(screen["level_1"].is_object() && screen["level_2"].is_null() && screen["back"].is_object(), "{screen}");
        let (ww, wh) = Layout::bare(w, h).window_size();
        let r = crate::level_select::tile_rect(UiFrame::plain((ww as f32, wh as f32)).area, 2);
        let locked = json!({ "x": r.x + r.width / 2.0, "y": r.y + r.height / 2.0 });
        let m = ask(&mut server, &tx, &mut s, "click", locked).unwrap();
        assert_eq!((m["levels_open"].as_bool(), m["level"]["number"].as_u64()), (Some(true), Some(1)), "a locked tile: {m}");

        let m = ask(&mut server, &tx, &mut s, "key", json!({ "key": "escape" })).unwrap();
        assert_eq!(m["levels_open"], false, "{m}");
        let m = ask(&mut server, &tx, &mut s, "key", json!({ "key": "escape" })).unwrap();
        assert_eq!(m["levels_open"], true, "Esc opens it over the round: {m}");
        let m = ask(&mut server, &tx, &mut s, "key", json!({ "key": "left" })).unwrap();
        assert_eq!(m["levels_focus"], 0, "{m}");
        let m = ask(&mut server, &tx, &mut s, "key", json!({ "key": "enter" })).unwrap();
        assert_eq!((m["levels_open"].as_bool(), m["level"]["number"].as_u64()), (Some(false), Some(0)), "{m}");
        assert!(server.lockstep(), "a new round, frozen like `restart`'s");
    }

    /// The builder's own camera: `status.builder.camera` reports FIT,
    /// `builder_camera` frames a world point at a zoom, a `click` then lands
    /// on the cell drawn under it, `fit` takes the view back, and the
    /// round's `camera` pin never moves it.
    #[test]
    fn builder_camera_frames_the_canvas_and_clicks_land_through_it() {
        let (mut server, tx) = DevServer::headless();
        let mut s = game(42);
        ask(&mut server, &tx, &mut s, "restart", json!({ "map_toml": INLINE_MAP, "seed": 1 })).unwrap();
        enter_build(&mut server, &tx, &mut s);
        let status = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        let cam = &status["builder"]["camera"];
        assert_eq!((cam["fit"].as_bool(), cam["zoom"].as_f64()), (Some(true), Some(1.0)), "{status}");
        assert_eq!(cam["rect"], json!({ "x": 0.0, "y": 0.0, "w": 1088.0, "h": 544.0 }));

        let framed = ask(&mut server, &tx, &mut s, "builder_camera", json!({ "x": 300.0, "y": 200.0, "zoom": 3.0 })).unwrap();
        assert_eq!(framed["fit"], false, "{framed}");
        assert!((framed["zoom"].as_f64().unwrap() - 3.0).abs() < 1e-4, "{framed}");
        let r = &framed["rect"];
        let mid = (r["x"].as_f64().unwrap() + r["w"].as_f64().unwrap() / 2.0, r["y"].as_f64().unwrap() + r["h"].as_f64().unwrap() / 2.0);
        assert!((mid.0 - 300.0).abs() <= 1.0 && (mid.1 - 200.0).abs() <= 1.0, "{framed}");
        // The middle of the canvas is that world point: a click there paints
        // the cell under it.
        ask(&mut server, &tx, &mut s, "builder_tool", json!({ "tool": "glass" })).unwrap();
        let m = ask(&mut server, &tx, &mut s, "click", json!({ "x": 544.0, "y": 32.0 + 272.0 })).unwrap();
        assert_eq!(m["cell"], json!([9, 6]), "{m}");
        let glass = crate::map::CellObject::Wall { material: crate::obstacle::Material::Glass };
        assert_eq!(s.builder.map().cell(9, 6), Some(&glass));
        // Only the zoom: the middle stays.
        let zoomed = ask(&mut server, &tx, &mut s, "builder_camera", json!({ "zoom": 2.0 })).unwrap();
        assert!((zoomed["zoom"].as_f64().unwrap() - 2.0).abs() < 1e-4);
        // The round's pin is the round's.
        ask(&mut server, &tx, &mut s, "camera", json!({ "zoom": 2.0 })).unwrap();
        let report = ask(&mut server, &tx, &mut s, "builder_camera", json!({})).unwrap();
        assert_eq!(report, zoomed, "no parameters only report, and the round's pin changed nothing");
        let fit = ask(&mut server, &tx, &mut s, "builder_camera", json!({ "fit": true })).unwrap();
        assert_eq!(fit["fit"], true);
        // A zoom that cannot be: FIT is the floor, the largest cell the roof.
        let far = ask(&mut server, &tx, &mut s, "builder_camera", json!({ "zoom": 1000.0 })).unwrap();
        assert!(far["zoom"].as_f64().unwrap() < 1000.0 && far["cell_mm"].as_f64().unwrap() > 0.0, "{far}");
    }

    /// `builder_touch` plays frames of fingers through the builder: a
    /// quick tap paints, a two-finger tap undoes it, spread fingers zoom,
    /// and the tool refuses outside build mode or on a malformed frame.
    #[test]
    fn builder_touch_drives_the_builder_with_fingers() {
        let (mut server, tx) = DevServer::headless();
        let mut s = game(43);
        ask(&mut server, &tx, &mut s, "restart", json!({ "map_toml": INLINE_MAP, "seed": 1 })).unwrap();
        let refused = ask(&mut server, &tx, &mut s, "builder_touch", json!({ "frames": [] })).unwrap_err();
        assert!(refused.contains("build mode"), "{refused}");
        enter_build(&mut server, &tx, &mut s);
        // A screen where a cell is a finger's size (the window hands the
        // builder its screen every frame; a test hands it once).
        let screen = crate::editor::CanvasScreen { device_per_px: 3.0, points_per_px: 1.5, coarse: false, ..Default::default() };
        let frame = crate::editor::BuilderFrame::headless((1088.0, 544.0), crate::framing::MapClass::Arena);
        s.update_builder(&BuilderInput { screen: Some(screen), ..BuilderInput::default() }, &frame);
        ask(&mut server, &tx, &mut s, "builder_tool", json!({ "tool": "iron" })).unwrap();
        let base = s.builder.history().undo_depth() as u64;
        let (x, y) = (12.0 * 32.0, 32.0 + 7.0 * 32.0);
        let tap = ask(&mut server, &tx, &mut s, "builder_touch", json!({ "frames": [[{ "id": 1, "x": x, "y": y }]] })).unwrap();
        assert_eq!(tap["undo_depth"], base + 1, "{tap}");
        let iron = crate::map::CellObject::Wall { material: crate::obstacle::Material::Iron };
        assert_eq!(s.builder.map().cell(12, 7), Some(&iron));
        let two = json!([[{ "id": 1, "x": 300.0, "y": 300.0 }], [{ "id": 1, "x": 300.0, "y": 300.0 }, { "id": 2, "x": 380.0, "y": 300.0 }]]);
        let undone = ask(&mut server, &tx, &mut s, "builder_touch", json!({ "frames": two })).unwrap();
        assert_eq!(undone["undo_depth"], base, "a two-finger tap undoes: {undone}");
        assert_eq!(s.builder.map().cell(12, 7), None);
        let pinch: Vec<Value> = (0..=12)
            .map(|i| json!([{ "id": 3, "x": 500.0 - 10.0 * i as f32, "y": 300.0 }, { "id": 4, "x": 580.0 + 10.0 * i as f32, "y": 300.0 }]))
            .collect();
        let zoomed = ask(&mut server, &tx, &mut s, "builder_touch", json!({ "frames": pinch })).unwrap();
        assert_eq!(zoomed["camera"]["fit"], false, "{zoomed}");
        assert!(zoomed["camera"]["zoom"].as_f64().unwrap() > 2.0, "{zoomed}");
        assert!(ask(&mut server, &tx, &mut s, "builder_touch", json!({ "frames": [[{ "id": 1 }]] })).unwrap_err().contains("frames: missing field `x`"));
        assert!(ask(&mut server, &tx, &mut s, "builder_touch", json!({})).unwrap_err().contains("frames"));
    }

    /// `status.builder.clear` is the clear check as the builder keeps it:
    /// the canvas's revision, not cleared until a win in the round its
    /// PLAY started (`attempt`), then cleared with the round clock as its
    /// par; and the bar's flag is a button `click` reaches, which opens
    /// the CHECK panel - on a map wide enough that the headless window's
    /// bar keeps the flag rather than folding it into CHECK.
    #[test]
    fn status_reports_the_clear_check_and_click_reaches_its_flag() {
        let (mut server, tx) = DevServer::headless();
        let mut game = Game::default();
        game.seed_override = Some(3);
        game.show_intro = false;
        game.map = MapFile::from_toml_str("version = 1\nsize = [40, 17]\ntanks = 1\ncells.\"3,8\" = { kind = \"start\" }\n").unwrap();
        let (w, h) = game.map.field_size();
        game.init(w, h);
        let mut s = Session::new(game);
        enter_build(&mut server, &tx, &mut s);
        let before = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap()["builder"]["clear"].clone();
        assert_eq!(before["cleared"], false, "{before}");
        assert_eq!(before["par"], Value::Null);
        assert_eq!(before["revision"].as_str().map(str::len), Some(16), "{before}");
        // The flag opens the CHECK panel.
        let status = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        let flag = &status["builder"]["buttons"]["clear"];
        let (x, y) = (flag["x"].as_f64().unwrap() + flag["w"].as_f64().unwrap() / 2.0, flag["y"].as_f64().unwrap() + flag["h"].as_f64().unwrap() / 2.0);
        ask(&mut server, &tx, &mut s, "click", json!({ "x": x, "y": y })).unwrap();
        assert_eq!(s.builder.open_menu(), Some("check"));
        ask(&mut server, &tx, &mut s, "play", json!({})).unwrap();
        let playing = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap()["builder"]["clear"].clone();
        assert_eq!(playing["attempt"], before["revision"], "{playing}");
        let enemy = s.game.world.query::<&crate::tank::Tank>().with::<&crate::ai::Ai>().iter().map(|t| t.owner_slot()).min().expect("an enemy");
        ask(&mut server, &tx, &mut s, "kill", json!({ "slot": enemy })).unwrap();
        for _ in 0..2 {
            s.game.update(Input::default(), crate::PHYSICS_FIXED_DT, w, h);
        }
        // What `app.rs` does after every frame's steps.
        s.note_outcome();
        let after = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap()["builder"]["clear"].clone();
        assert_eq!(after["cleared"], true, "{after}");
        assert_eq!(after["revision"], before["revision"]);
        assert!(after["par"].as_f64().is_some_and(|par| par >= 0.0), "{after}");
    }

    /// `builder_touch {hold: true}` leaves a painting finger down, so the
    /// loupe stands over it in `status.builder.loupe` - over the cell the
    /// stroke paints -, the window's frames keep it there, the next call
    /// lifts it and the stroke is one undo step, and leaving build mode
    /// lets held fingers go.
    #[test]
    fn builder_touch_holds_a_finger_for_the_loupe() {
        let (mut server, tx) = DevServer::headless();
        let mut s = game(45);
        ask(&mut server, &tx, &mut s, "restart", json!({ "map_toml": INLINE_MAP, "seed": 1 })).unwrap();
        enter_build(&mut server, &tx, &mut s);
        // Cells of 7.6 mm on the glass: a finger paints, and the loupe shows.
        let screen = crate::editor::CanvasScreen { device_per_px: 3.0, points_per_px: 1.5, coarse: false, ..Default::default() };
        let frame = crate::editor::BuilderFrame::headless((1088.0, 544.0), crate::framing::MapClass::Arena);
        s.update_builder(&BuilderInput { screen: Some(screen), ..BuilderInput::default() }, &frame);
        ask(&mut server, &tx, &mut s, "builder_tool", json!({ "tool": "brick" })).unwrap();
        let base = s.builder.history().undo_depth() as u64;
        let none = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        assert_eq!(none["builder"]["loupe"], Value::Null, "no finger, no loupe");
        let stroke: Vec<Value> = (0..=10).map(|i| json!([{ "id": 1, "x": 300.0 + 10.0 * i as f32, "y": 300.0 }])).collect();
        let held = ask(&mut server, &tx, &mut s, "builder_touch", json!({ "frames": stroke, "hold": true })).unwrap();
        assert_eq!(held["held"], 1, "{held}");
        assert_eq!(server.held_touches().len(), 1);
        let status = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        let loupe = &status["builder"]["loupe"];
        // The canvas at FIT on an arena: a bitmap pixel a world pixel.
        let under = crate::map::world_to_cell(frame.to_canvas(crate::math::Vec2::new(400.0, 300.0)));
        assert_eq!(loupe["cell"], json!([under.0, under.1]), "over the cell under the finger: {loupe}");
        assert!(loupe["y"].as_f64().unwrap() + loupe["h"].as_f64().unwrap() < 300.0, "above the finger: {loupe}");
        let lifted = ask(&mut server, &tx, &mut s, "builder_touch", json!({ "frames": [[]] })).unwrap();
        assert_eq!(lifted["held"], 0, "{lifted}");
        assert_eq!(lifted["undo_depth"], base + 1, "the stroke is one step: {lifted}");
        let status = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        assert_eq!(status["builder"]["loupe"], Value::Null, "lifted, gone");
        // Held fingers are build mode's alone.
        let stroke: Vec<Value> = (0..=10).map(|i| json!([{ "id": 2, "x": 300.0 + 10.0 * i as f32, "y": 400.0 }])).collect();
        ask(&mut server, &tx, &mut s, "builder_touch", json!({ "frames": stroke, "hold": true })).unwrap();
        assert_eq!(server.held_touches().len(), 1);
        ask(&mut server, &tx, &mut s, "play", json!({})).unwrap();
        assert!(server.held_touches().is_empty(), "play lets them go");
    }

    /// The hints follow the input a tool stands in for: a `click {touch}`
    /// and a `builder_touch` are taps, a `key` a key press, a plain
    /// `click` neither. A server with no window lays its chrome out with
    /// the last (`status.ui.hints`); a window takes each once
    /// (`take_hints`) and lays its own out with it.
    #[test]
    fn the_input_tools_turn_the_hints() {
        use crate::hud::Hints;
        let (mut server, tx) = DevServer::headless();
        let mut s = game(46);
        let hints = |server: &mut DevServer, s: &mut Session| ask(server, &tx, s, "status", json!({ "sections": ["all"] })).unwrap()["ui"]["hints"].clone();
        assert_eq!(hints(&mut server, &mut s), "keys", "a window with a keyboard opens on the keys");
        ask(&mut server, &tx, &mut s, "click", json!({ "x": 600.0, "y": 300.0, "touch": true })).unwrap();
        assert_eq!(hints(&mut server, &mut s), "touch");
        ask(&mut server, &tx, &mut s, "click", json!({ "x": 600.0, "y": 300.0 })).unwrap();
        assert_eq!(hints(&mut server, &mut s), "touch", "a mouse press is no key");
        ask(&mut server, &tx, &mut s, "key", json!({ "key": "tab" })).unwrap();
        assert_eq!(hints(&mut server, &mut s), "keys");
        assert_eq!(server.take_hints(), Some(Hints::Keys));
        assert_eq!(server.take_hints(), None, "taken once");
        ask(&mut server, &tx, &mut s, "key", json!({ "key": "escape" })).unwrap();
        enter_build(&mut server, &tx, &mut s);
        ask(&mut server, &tx, &mut s, "builder_touch", json!({ "frames": [[]] })).unwrap();
        assert_eq!(hints(&mut server, &mut s), "touch");
        assert_eq!(server.take_hints(), Some(Hints::Touch));
    }

    /// The builder's navigator is one hit test that `click` and
    /// `builder_touch` both reach: `status.builder.navigator` says where it
    /// stands (nowhere at FIT on an arena), a click on it puts the view's
    /// middle on the point under it, a drag carries the view along, a
    /// finger does what the mouse does, and none of it paints.
    #[test]
    fn the_navigator_is_reached_by_click_and_by_a_finger() {
        const FIELD_MAP: &str = r#"
version = 1
size = [80, 45]
cells."5,5" = { kind = "start" }
cells."40,30" = { kind = "bong" }
"#;
        let (mut server, tx) = DevServer::headless();
        let mut s = game(44);
        ask(&mut server, &tx, &mut s, "restart", json!({ "map_toml": INLINE_MAP, "seed": 1 })).unwrap();
        enter_build(&mut server, &tx, &mut s);
        let arena = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        assert_eq!(arena["builder"]["navigator"], Value::Null, "none at FIT on an arena");

        ask(&mut server, &tx, &mut s, "restart", json!({ "map_toml": FIELD_MAP, "seed": 1 })).unwrap();
        enter_build(&mut server, &tx, &mut s);
        let status = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        let nav = status["builder"]["navigator"].clone();
        assert!(nav.is_object() && nav["ui"].is_object(), "a field map's navigator stands at FIT too: {status}");
        let n = |v: &Value, k: &str| v[k].as_f64().unwrap() as f32;
        let at = |r: &Value, fx: f32, fy: f32| (n(r, "x") + n(r, "w") * fx, n(r, "y") + n(r, "h") * fy);
        let centre = |s: &Session| s.builder.camera().center(&s.builder.viewport());
        let undo = s.builder.history().undo_depth();
        let middle = Vec2::new(40.0 * 32.0, 22.5 * 32.0);

        let (x, y) = at(&nav, 0.6, 0.6);
        let clicked = ask(&mut server, &tx, &mut s, "click", json!({ "x": x, "y": y })).unwrap();
        assert!(clicked.get("cell").is_none(), "a press on the navigator is not on the canvas: {clicked}");
        assert!(!s.builder.camera().is_fit(), "a press from FIT zooms in on the point");
        let after_click = centre(&s);
        assert!(after_click.x > middle.x && after_click.y > middle.y, "the view went right and down: {after_click:?}");

        let (x2, y2) = at(&nav, 0.2, 0.3);
        ask(&mut server, &tx, &mut s, "click", json!({ "x": x, "y": y, "drag_to": [x2, y2] })).unwrap();
        let after_drag = centre(&s);
        assert!(after_drag.x < after_click.x && after_drag.y < after_click.y, "the drag carried it up and left: {after_drag:?}");

        // A finger, in the same window coordinates.
        let (tx0, ty0) = at(&nav, 0.5, 0.5);
        let (tx1, ty1) = at(&nav, 0.8, 0.8);
        let frames = json!([[{ "id": 1, "x": tx0, "y": ty0 }], [{ "id": 1, "x": tx1, "y": ty1 }]]);
        let touched = ask(&mut server, &tx, &mut s, "builder_touch", json!({ "frames": frames })).unwrap();
        let after_touch = centre(&s);
        assert!(after_touch.x > after_drag.x && after_touch.y > after_drag.y, "the finger moved it: {after_touch:?}");
        assert_eq!(touched["dirty"], false, "{touched}");
        assert_eq!(s.builder.history().undo_depth(), undo, "the navigator never paints");
    }

    /// `builder_settings {size, anchor}` resizes the canvas about the
    /// anchor as one undo step and reports the size and the anchor;
    /// `builder_undo` brings the dropped cells back.
    #[test]
    fn builder_settings_resizes_the_map_about_its_anchor() {
        let (mut server, tx) = DevServer::headless();
        let mut s = game(44);
        ask(&mut server, &tx, &mut s, "restart", json!({ "map_toml": INLINE_MAP, "seed": 1 })).unwrap();
        enter_build(&mut server, &tx, &mut s);
        let before = s.builder.map().clone();
        let depth = s.builder.history().undo_depth() as u64;
        let r = ask(&mut server, &tx, &mut s, "builder_settings", json!({ "anchor": "bottom_right", "size": [24, 12] })).unwrap();
        assert_eq!((r["size"].clone(), r["anchor"].clone()), (json!([24.0, 12.0]), json!("bottom_right")), "{r}");
        // Bottom-right: the map moves 10 left and 5 up; the start at 5,5
        // falls off, the bong at 30,15 lands at 20,10.
        assert_eq!(s.builder.map().cell(20, 10), Some(&crate::map::CellObject::Bong));
        assert_eq!(s.builder.map().start_cell(), None);
        assert_eq!(s.builder.history().undo_depth() as u64, depth + 1);
        let m = ask(&mut server, &tx, &mut s, "builder_map", json!({})).unwrap();
        assert!(m["diff"]["settings"].as_array().unwrap().contains(&json!("size")), "{m}");
        ask(&mut server, &tx, &mut s, "builder_undo", json!({})).unwrap();
        assert_eq!(s.builder.map(), &before);
        assert!(ask(&mut server, &tx, &mut s, "builder_settings", json!({ "anchor": "nowhere" })).unwrap_err().contains("top_left"));
        assert!(ask(&mut server, &tx, &mut s, "builder_settings", json!({ "size": [24] })).unwrap_err().contains("size:"));
    }

    #[test]
    fn click_and_key_take_the_same_paths_as_the_mouse_and_keyboard() {
        let (mut server, tx) = DevServer::headless();
        let mut s = game(36);
        ask(&mut server, &tx, &mut s, "restart", json!({ "map_toml": INLINE_MAP, "seed": 1 })).unwrap();
        let centre = |r: crate::math::Rectangle| (r.x + r.width / 2.0, r.y + r.height / 2.0);
        // A click on BUILD enters the builder like `build`, and a click on
        // BACK, where `status.builder.buttons` puts it, comes back.
        let build = corner_button(&mut server, &tx, &mut s, "build");
        let m = ask(&mut server, &tx, &mut s, "click", build).unwrap();
        assert_eq!(m["mode"], "build", "{m}");
        let st = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        let b = &st["builder"]["buttons"]["back"];
        let back = json!({ "x": b["x"].as_f64().unwrap() + b["w"].as_f64().unwrap() / 2.0, "y": b["y"].as_f64().unwrap() + b["h"].as_f64().unwrap() / 2.0 });
        let m = ask(&mut server, &tx, &mut s, "click", back).unwrap();
        assert_eq!(m["mode"], "play", "{m}");
        // Tab enters it; Escape there with nothing to take back is BACK.
        let m = ask(&mut server, &tx, &mut s, "key", json!({ "key": "tab" })).unwrap();
        assert_eq!(m["mode"], "build", "{m}");
        let m = ask(&mut server, &tx, &mut s, "key", json!({ "key": "escape" })).unwrap();
        assert_eq!(m["mode"], "play", "{m}");
        assert!(ask(&mut server, &tx, &mut s, "key", json!({ "key": "f1" })).is_err());
        assert!(ask(&mut server, &tx, &mut s, "key", json!({})).is_err());
        // A left click on the field in play mode is not an order; the
        // round is untouched by it.
        let m = ask(&mut server, &tx, &mut s, "click", json!({ "x": 640.0, "y": 32.0 + 400.0 })).unwrap();
        assert_eq!(m["mode"], "play", "{m}");
        let m = ask(&mut server, &tx, &mut s, "key", json!({ "key": "tab" })).unwrap();
        assert_eq!(m["mode"], "build", "{m}");

        // Build mode: a click on a field cell paints with the active brush,
        // a drag crosses every cell, undo is a key.
        ask(&mut server, &tx, &mut s, "builder_tool", json!({ "tool": "iron" })).unwrap();
        // The builder's canvas fills a window a bar's height taller than it
        // while none is drawn, letterboxed in it. A cell's world position
        // is its centre (`map::cell_to_world`).
        let builder = crate::editor::BuilderFrame::headless(s.builder.map().field_size(), s.builder.map().class());
        let cell_centre = |c: i32, r: i32| (c as f32 * 32.0, builder.view.offset.y + r as f32 * 32.0);
        let base = s.builder.history().undo_depth() as u64;
        let (cx, cy) = cell_centre(10, 5);
        let m = ask(&mut server, &tx, &mut s, "click", json!({ "x": cx, "y": cy })).unwrap();
        assert_eq!(m["mode"], "build", "{m}");
        assert_eq!(m["undo_depth"], base + 1);
        let iron = crate::map::CellObject::Wall { material: crate::obstacle::Material::Iron };
        assert_eq!(s.builder.map().cell(10, 5), Some(&iron));
        let (ex, ey) = cell_centre(10, 6);
        let (dx, dy) = cell_centre(14, 6);
        let m = ask(&mut server, &tx, &mut s, "click", json!({ "x": ex, "y": ey, "drag_to": [dx, dy] })).unwrap();
        assert_eq!(m["undo_depth"], base + 2, "one step for the whole drag: {m}");
        assert!((10..=14).all(|c| s.builder.map().cell(c, 6) == Some(&iron)), "the drag crossed every cell");
        assert!(s.builder.map().cell(12, 5).is_none() && s.builder.map().cell(12, 7).is_none(), "and stayed on row 6");
        // A right click erases; Ctrl+Z undoes it.
        let m = ask(&mut server, &tx, &mut s, "click", json!({ "x": cx, "y": cy, "button": "right" })).unwrap();
        assert_eq!(m["undo_depth"], base + 3, "{m}");
        assert!(s.builder.map().cell(10, 5).is_none());
        let m = ask(&mut server, &tx, &mut s, "key", json!({ "key": "undo" })).unwrap();
        assert_eq!(m["undo_depth"], base + 2, "{m}");
        assert_eq!(m["redo_depth"], 1);
        assert_eq!(s.builder.map().cell(10, 5), Some(&iron));
        assert!(ask(&mut server, &tx, &mut s, "click", json!({ "x": 1.0, "y": 1.0, "drag_to": [1.0] })).is_err());
        // PLAY from its plate starts the round frozen, like `play` - on the
        // builder's own bitmap, its whole canvas at its own size.
        server.lockstep = false;
        let (px, py) = centre(builder.ui.rect_to_window(builder.bar().play));
        let m = ask(&mut server, &tx, &mut s, "click", json!({ "x": px, "y": py })).unwrap();
        assert_eq!(m["mode"], "play", "{m}");
        assert!(server.lockstep());
        assert_eq!(s.game.map.cell(12, 6), Some(&iron));
        // Tab in build mode is PLAY too.
        enter_build(&mut server, &tx, &mut s);
        server.lockstep = false;
        let m = ask(&mut server, &tx, &mut s, "key", json!({ "key": "tab" })).unwrap();
        assert_eq!(m["mode"], "play", "{m}");
        assert!(server.lockstep());
        assert_eq!(s.game.frame(), 0);
    }

    #[test]
    fn restart_in_build_mode_returns_to_play_and_replaces_the_builder_map() {
        let (mut server, tx) = DevServer::headless();
        let mut s = game(37);
        enter_build(&mut server, &tx, &mut s);
        let free = (0..40).flat_map(|c| (0..23).map(move |r| (c, r))).find(|&(c, r)| s.builder.map().cell(c, r).is_none()).unwrap();
        ask(&mut server, &tx, &mut s, "builder_paint", json!({ "cells": [[free.0, free.1]], "tool": "barrel" })).unwrap();
        assert!(s.builder.dirty());
        let status = ask(&mut server, &tx, &mut s, "restart", json!({ "map_toml": INLINE_MAP })).unwrap();
        assert_eq!(status["mode"], "play", "{status}");
        assert_eq!(status["map"]["cells"], 3);
        assert_eq!(s.builder.map().cells.len(), 3, "the builder's canvas is the new map");
        assert!(!s.builder.dirty(), "and its baseline");
        assert_eq!(s.builder.map().cell(free.0, free.1), None);
        // A bare restart in build mode returns to play on the same maps.
        enter_build(&mut server, &tx, &mut s);
        let status = ask(&mut server, &tx, &mut s, "restart", json!({})).unwrap();
        assert_eq!(status["mode"], "play");
        assert_eq!(s.builder.map().cells.len(), 3);
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

    /// `step` on the session's own field size (a `restart {map_toml}` may
    /// have changed it from the harness's).
    fn step(server: &mut DevServer, tx: &mpsc::Sender<Request>, s: &mut Session, params: Value) -> Value {
        let (w, h) = s.field_size();
        let rx = call(tx, "step", params);
        server.before_frame(s, w, h);
        server.advance(&mut s.game, Input::default(), 1, w, h, &mut |_| {});
        rx.recv().unwrap().unwrap()
    }

    /// Player 2 is a player: never an enemy in the aggregates, never a
    /// clustering neighbour, and every row says which it is.
    #[test]
    fn history_aggregates_treat_player_2_as_a_player() {
        let (mut server, tx) = DevServer::headless();
        let mut s = game(14);
        ask(&mut server, &tx, &mut s, "restart", json!({ "seed": 14, "players": 2 })).unwrap();
        step(&mut server, &tx, &mut s, json!({ "frames": 60, "snapshot": false }));
        let h = ask(&mut server, &tx, &mut s, "history", json!({ "last": 60, "every": 60 })).unwrap();
        let rows = h["rows"].as_array().unwrap();
        assert!(rows.iter().any(|r| r["slot"] == 1 && r["is_player"] == true), "{rows:?}");
        assert!(rows.iter().any(|r| r["slot"] == 2 && r["is_player"] == false), "{rows:?}");
        assert!(rows.iter().all(|r| r["rotation"].is_number() && r["turret"].is_number()), "{rows:?}");
        let p2 = &h["tanks"]["1"];
        assert_eq!(p2["frames"], 60, "{p2}");
        assert_eq!(p2["no_ring_frames"], 0, "{p2}");
        assert_eq!(p2["cluster_frames"], 0, "{p2}");
        assert!(p2["round"]["turns"].is_number(), "{p2}");
    }

    #[test]
    fn restart_accepts_chassis_names() {
        let (mut server, tx) = DevServer::headless();
        let mut s = game(2);
        ask(&mut server, &tx, &mut s, "restart", json!({ "tank": "titan", "tank2": "scout", "players": 2, "seed": 2 })).unwrap();
        let snap = ask(&mut server, &tx, &mut s, "snapshot", json!({})).unwrap();
        assert_eq!(snap["tanks"][0]["chassis"], "titan", "{}", snap["tanks"][0]);
        assert_eq!(snap["tanks"][1]["chassis"], "scout", "{}", snap["tanks"][1]);
        let err = ask(&mut server, &tx, &mut s, "restart", json!({ "tank": "bogus" })).unwrap_err();
        assert!(err.contains("scout") && err.contains("leviathan"), "{err}");
        let err = ask(&mut server, &tx, &mut s, "restart", json!({ "tank": "titan", "tank_row": 3 })).unwrap_err();
        assert!(err.contains("not both"), "{err}");
        // The schema's enum is the chassis list, so the const string cannot drift.
        let spec = TOOLS.iter().find(|t| t.name == "restart").unwrap();
        let schema = spec.schema();
        let names: Vec<&str> = TankKind::ALL.iter().map(|k| k.name()).collect();
        for key in ["tank", "tank2"] {
            let listed: Vec<&str> = schema["properties"][key]["enum"].as_array().unwrap().iter().map(|v| v.as_str().unwrap()).collect();
            assert_eq!(listed, names, "{key}");
        }
    }

    #[test]
    fn lint_reads_the_round_in_play_mode_and_the_canvas_in_build_mode() {
        let (mut server, tx) = DevServer::headless();
        let mut s = game(7);
        let l = ask(&mut server, &tx, &mut s, "lint", json!({})).unwrap();
        assert_eq!(l["source"], "round", "{l}");
        assert_eq!(l["errors"], 0, "the shipped map lints clean: {l}");
        assert_eq!(l["players"], 1);
        assert_eq!(l["map"]["cells"], s.game.map.cells.len());
        for f in l["findings"].as_array().unwrap() {
            assert!(f["severity"].is_string() && f["kind"].is_string() && f["message"].is_string(), "{f}");
        }
        assert!(ask(&mut server, &tx, &mut s, "lint", json!({ "source": "canvas" })).is_err());
        // A pickup walled into a 9 x 9 iron block on the canvas (the reach
        // rule allows a tank radius plus 96 px of slack, so a thinner ring
        // still counts as approachable): an error the round's map does not
        // have.
        enter_build(&mut server, &tx, &mut s);
        let mut sealed = format!("{INLINE_MAP}\ncells.\"14,8\" = {{ kind = \"pickup\", pickup = \"health\" }}\n");
        for col in 10..=18 {
            for row in 4..=12 {
                if (col, row) != (14, 8) {
                    sealed.push_str(&format!("cells.\"{col},{row}\" = {{ kind = \"wall\", material = \"iron\" }}\n"));
                }
            }
        }
        ask(&mut server, &tx, &mut s, "builder_map", json!({ "map_toml": sealed })).unwrap();
        let l = ask(&mut server, &tx, &mut s, "lint", json!({})).unwrap();
        assert_eq!(l["source"], "builder", "{l}");
        let kinds: Vec<&str> = l["findings"].as_array().unwrap().iter().map(|f| f["kind"].as_str().unwrap()).collect();
        assert!(kinds.iter().any(|k| k.contains("pickup")), "{l}");
        assert!(l["errors"].as_u64().unwrap() >= 1, "{l}");
        assert_eq!(l["map"]["cells"], 3 + 81);
        let l = ask(&mut server, &tx, &mut s, "lint", json!({ "source": "round" })).unwrap();
        assert_eq!(l["source"], "round");
        assert_eq!(l["errors"], 0, "{l}");
        assert_eq!(l["map"]["cells"], s.game.map.cells.len(), "the round's map, not the canvas");
    }

    #[test]
    fn terrain_lists_live_tiles_by_cell() {
        let (mut server, tx) = DevServer::headless();
        let mut s = game(3);
        let map = format!("{INLINE_MAP}\ncells.\"10,10\" = {{ kind = \"barrel\", drum = \"oil\" }}\ncells.\"12,10\" = {{ kind = \"wall\", material = \"wood\" }}\n");
        ask(&mut server, &tx, &mut s, "restart", json!({ "map_toml": map, "seed": 3 })).unwrap();
        let t = ask(&mut server, &tx, &mut s, "terrain", json!({})).unwrap();
        let snap = ask(&mut server, &tx, &mut s, "snapshot", json!({})).unwrap();
        assert_eq!(t["count"], snap["obstacles_alive"], "{t}");
        assert_eq!(t["total"], 3, "{t}");
        assert_eq!(t["truncated"], false);
        let tiles = t["tiles"].as_array().unwrap();
        let iron = tiles.iter().find(|x| x["cell"] == json!([20, 8])).unwrap_or_else(|| panic!("{t}"));
        assert_eq!(iron["material"], "iron");
        assert_eq!(iron["hp"], iron["max_hp"]);
        assert!(iron.get("drum").is_none() && iron.get("burning").is_none() && iron.get("fuse").is_none(), "{iron}");
        let drum = tiles.iter().find(|x| x["cell"] == json!([10, 10])).unwrap();
        assert_eq!(drum["material"], "barrel");
        assert_eq!(drum["drum"], "oil");
        let keys: Vec<(i64, i64)> = tiles.iter().map(|x| (x["cell"][1].as_i64().unwrap(), x["cell"][0].as_i64().unwrap())).collect();
        assert!(keys.windows(2).all(|w| w[0] <= w[1]), "sorted by row then col: {keys:?}");
        assert!(t["fires"].as_array().unwrap().is_empty() && t["fused"].as_array().unwrap().is_empty(), "{t}");
        assert_eq!(t["burning_tiles"], 0);
        let t = ask(&mut server, &tx, &mut s, "terrain", json!({ "only": "burning" })).unwrap();
        assert_eq!(t["count"], 0);
        assert_eq!(t["total"], 3, "total is the live count before filtering");
        let t = ask(&mut server, &tx, &mut s, "terrain", json!({ "materials": ["iron"] })).unwrap();
        assert_eq!(t["count"], 1, "{t}");
        assert!(ask(&mut server, &tx, &mut s, "terrain", json!({ "only": "hot" })).is_err());
        enter_build(&mut server, &tx, &mut s);
        assert!(ask(&mut server, &tx, &mut s, "terrain", json!({})).is_err(), "game-only");
    }

    /// Driving the player round a square is one spin by the probe's rule;
    /// a back-and-forth is reversals, not a spin.
    #[test]
    fn history_counts_turns_reversals_and_spins_for_the_player() {
        let (mut server, tx) = DevServer::headless();
        let mut s = game(1);
        ask(&mut server, &tx, &mut s, "restart", json!({ "map_toml": INLINE_MAP, "enemies": 0, "seed": 1 })).unwrap();
        let round = |server: &mut DevServer, s: &mut Session| {
            ask(server, &tx, s, "history", json!({ "slot": 0, "every": 50 })).unwrap()["tanks"]["0"]["round"].clone()
        };
        step(&mut server, &tx, &mut s, json!({ "frames": 10, "move_dir": "up", "snapshot": false }));
        let base = round(&mut server, &mut s);
        let base_turns = base["turns"].as_u64().unwrap();
        assert_eq!(base["spins"], 0, "{base}");
        // Turning right while still carrying upward momentum: the hull
        // faces 90 but the real heading sits between up and right - the
        // difference `heading` exists to show.
        let player = step(&mut server, &tx, &mut s, json!({ "frames": 10, "move_dir": "right", "snapshot": true }))["snapshot"]["tanks"][0].clone();
        assert_eq!(player["facing"], "right", "{player}");
        assert_eq!(player["rotation"], 90.0);
        assert!(player["speed"].as_f64().unwrap() > 10.0, "{player}");
        let heading = player["heading"].as_f64().unwrap();
        assert!(heading > 0.0 && heading < 90.0, "{player}");
        assert!(player["vx"].as_f64().unwrap() > 0.0 && player["vy"].as_f64().unwrap() < 0.0, "{player}");
        for dir in ["down", "left", "up"] {
            step(&mut server, &tx, &mut s, json!({ "frames": 10, "move_dir": dir, "snapshot": false }));
        }
        let r = round(&mut server, &mut s);
        assert_eq!(r["turns"], base_turns + 4, "{r}");
        assert_eq!(r["spins"], 1, "{r}");
        assert_eq!(r["reversals"], 0, "{r}");
        assert_eq!(r["max_spin_deg"], 360.0, "{r}");
        assert!(r["turret_deg"].as_f64().unwrap() > 90.0, "{r}");
        assert_eq!(r["last_turn_frame"], 41, "{r}");
        let st = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        assert_eq!(st["turns"]["spins"], 1, "{}", st["turns"]);
        // Up, down, up: two U-turns and one A->B->A reversal, no spin.
        for dir in ["down", "up"] {
            step(&mut server, &tx, &mut s, json!({ "frames": 10, "move_dir": dir, "snapshot": false }));
        }
        let r = round(&mut server, &mut s);
        assert_eq!(r["turns"], base_turns + 6, "{r}");
        assert_eq!(r["u_turns"].as_u64().unwrap(), base["u_turns"].as_u64().unwrap() + 2, "{r}");
        assert_eq!(r["reversals"], 1, "{r}");
        assert_eq!(r["spins"], 1, "{r}");
        // A restart zeroes the counters.
        ask(&mut server, &tx, &mut s, "restart", json!({ "seed": 1 })).unwrap();
        let st = ask(&mut server, &tx, &mut s, "status", json!({ "sections": ["all"] })).unwrap();
        assert_eq!(st["turns"]["turns"], 0, "{}", st["turns"]);
        assert_eq!(st["turns"]["spins"], 0);
    }
}
