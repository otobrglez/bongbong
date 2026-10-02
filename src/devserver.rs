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
use crate::editor::{BuilderInput, Category, CellChange, MapEditor, Tool, parse_mission, parse_spawn, parse_tank, parse_tier};
use crate::hud::{leave_dialog_rects, players_dialog_rects, CornerButton, CornerShape, Corners, UiFrame};
use crate::map::MapFile;
use crate::maplint::LintSeverity;
use crate::mode::{Driver, Session};
use crate::net::client::Phase;
use crate::net::round::AnyRound;
use crate::obstacle::Obstacle;
use crate::simulation::debug::{CLUSTER_RADIUS_PX, Detail, FieldTarget, JITTER_WINDOW_FRAMES, SPIN_FULL_CIRCLE_DEG, SPIN_NET_MAX, SPIN_WINDOW_FRAMES, TankPatch, TrackRow, r1, signed_quarter_turn};
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

/// The tools that read or drive the round and are refused in build mode
/// (docs/game-editor-fusion.md section 11) rather than touching a round
/// the builder has frozen.
pub const GAME_ONLY_TOOLS: &[&str] = &[
    "snapshot", "events", "step", "input", "pause", "resume", "history", "nav_grid", "field", "terrain", "teleport",
    "set_tank", "kill", "spawn_enemy", "players", "weather",
];

/// The tools that drive the *local* round or the builder, refused while
/// the window holds a seat in a room (docs/online-coop-prd.md §4.5): an
/// online round is the server's to simulate and the replica on screen is
/// a picture of it, so a write here would change the picture and reach
/// nobody. Everything that only reads - `status`, `snapshot`, `terrain`,
/// `events`, `history`, `nav_grid`, `field`, `map_get`, `lint`,
/// `overlays`, `screenshot`, `mode`, `builder_files` and the `tuning_*`
/// tools - describes the online round instead (`Session::shown`), and
/// `key {escape}` gives the seat up, as does a `click` on the corners'
/// `LEAVE` button - the one thing a click has to press in this mode.
pub const ONLINE_REFUSED_TOOLS: &[&str] = &[
    "step", "input", "pause", "resume", "restart", "teleport", "set_tank", "kill", "spawn_enemy", "players", "play",
    "build", "builder_tool", "builder_paint", "builder_undo", "builder_redo", "builder_settings",
    "builder_map", "builder_save", "builder_touch",
];

/// Tiles one `terrain` reply lists at most (the standard 34 x 17 field
/// has 578 cells; a size-study map can have more).
const TERRAIN_MAX_TILES: usize = 800;

/// The `key` tool's key names.
const KEY_NAMES: &[&str] = &["tab", "escape", "enter", "undo", "redo", "backspace", "1", "2", "left", "right", "up", "down", "zoom_in", "zoom_out"];

/// One tool: its wire/MCP name, the description the model reads, and its
/// input JSON schema (an `object` schema, as a string so this table can be
/// a `const`).
pub struct ToolSpec {
    pub name: &'static str,
    pub description: &'static str,
    pub schema: &'static str,
    /// Reads without changing the round, the builder, tuning or lockstep
    /// (MCP `readOnlyHint`). `screenshot` is not: it writes a file and
    /// takes an overlay patch.
    pub read_only: bool,
    /// Throws something away a client may want (MCP `destructiveHint`):
    /// the round (`restart`, `play`, a `players` count change, `kill`),
    /// every tuning edit (`tuning_reset`) or a map file (`builder_save`).
    /// Emitted explicitly because the MCP default is *true*.
    pub destructive: bool,
}

impl ToolSpec {
    /// The MCP tool annotations the adapter advertises for this tool.
    pub fn annotations(&self) -> Value {
        json!({
            "readOnlyHint": self.read_only,
            "destructiveHint": self.destructive,
            "idempotentHint": self.read_only,
            "openWorldHint": false,
        })
    }
}

const NO_PARAMS: &str = r#"{"type":"object","properties":{}}"#;
/// Just a room code: what most of `ROOM_TOOLS` takes.
const CODE_ONLY: &str = r#"{"type":"object","required":["code"],"properties":{"code":{"type":"string"}}}"#;
const SLOT_PARAMS: &str = r#"{"type":"object","properties":{"slot":{"type":"integer","description":"Owner slot: 0 = player, enemies from 1 (see snapshot.tanks[].slot)"}},"required":["slot"]}"#;

/// Every tool the server answers, in the order the adapter lists them.
pub const TOOLS: &[ToolSpec] = &[
    ToolSpec {
        name: "status",
        description: "Where the running game is: seed, frame, time, outcome, mission and the resolved spawn plan (`wave` while waves run), paused/lockstep, tank counts, overlay flags, the loaded map, `camera` (the view the window last drew - `whole` for an arena or the builder, `follow` for a field map, `pinned` for the `camera` tool's - with its world `rect`, `scale` (bitmap px per world px), scene `target` and `window_field` (the field area on the window, in points); a followed view adds the `seat` it follows and its `focus` (seat|shared|split|spectating|nobody), whether it `cut` this frame, its `lead` and sub-block `offset`, the `split` of a couch pair apart (null for one view: the divider's `line` - a point and the unit normal into the second half in the followed bitmap's pixels - and `window_line`, how far `apart` the halves' views stand and each half's `seat`, `rect`, `offset`, `cut` and `in_view`), the `establishing` shot's `phase` (whole|zoom|follow) and `progress` (view `establishing` while it plays), the `seating` (local|room), the `framing` - `visible_cells`, `device_scale` (device px per world px), `point_scale`, `block_px`, whether the zoom `snapped` to whole blocks, `tank_points`, `tank_mm` and the `bars` past the aspect clamp - and the `sight_box` it keeps: `half`, the `room` left for the look-ahead and whether it is `in_view`; and `motion`, the one motion switch - whether motion is `reduced` (no shake, no whole-screen ripple, the establishing shot cut rather than zoomed), the `reduce_motion` row's `setting` (platform|off|on) and what the `platform` said at startup, null where it says nothing), `ui` (the UI scale - window units per point -, the window and the safe area the chrome keeps to in points, whether it is laid out for `touch`, in play and online the corners' `buttons` and `clusters` and the `minimap` picture under the right cluster (`null` where none is drawn - an arena shown whole, a phone; a press there does nothing), and the `screen_buttons` of whatever stands over the round - the level select's open tiles (`level_N`) and `back`, a dialog's `one`/`two` or `leave`/`stay`, a level's end screen's `levels`/`again`/`next`, the lobby's live buttons (`host`, `join`, `key_a`, `confirm`, `ready`, `start`, `kick_1`, ...) - all in window coordinates, which is what `click` takes), `mode` (play|build|online) with the dialogs and the builder's state (`builder.navigator`: the navigator's picture in window coordinates for `click`, with its `bitmap` rect for `builder_touch`; `null` at FIT on an arena), and `turns` (heading turns/reversals/spins summed over the live tanks this round - a non-zero `spins` is a tank rotating in place; see `history`). `round` says which round all of this describes: `local`, or `online` with the room code, the seat, `buffer_ms` (how far ahead of the picture the newest snapshot is), `rtt` (the measured round trip - median, p95, floor - and server-minus-local from ping/pong probes), the server's tick, the phase, `interpolation` (the delay in force and its target, the link's jitter, the measured cadence, frames drawn on extrapolation, lateness p50/p95, stalls, the playout rate, corrections and their p95 in px, stale events dropped) and `prediction` (the stage-2 counters: corrections ignored/nudged/snapped, the error histogram `error_buckets` at 0.25/0.5/2/8/48 px and past, `max_error_px`, shots drawn/refused/on screen, inputs `in_flight`, the local fire gate, the lead's `lead_up`/`lead_down` adjustments with the smoothed mailbox `lead_depth`, and decision 9's instrument: `crossings` - provisional shots the picture stopped against a drawn tank or frog -, `crossings_hit` - their paired room copy bursting within 40 px (`HIT_MATCH_PX`) of that stop - and `crossings_missed` - their copy flying on past it (`MISS_MARGIN_PX`) or bursting anywhere else) - in an online round every reading tool describes the room's replica and the tools that would write to it refuse, because only the server simulates it. Cheap; call first.",
        schema: NO_PARAMS,
        read_only: true,
        destructive: false,
    },
    ToolSpec {
        name: "snapshot",
        description: "World state as JSON: every tank (position with its grid `cell`, `rotation` - the sim-side heading - with `facing` as a name and the two drawn angles `hull`/`turret` that ease toward it, real velocity with `speed` and `heading` - the direction it is actually moving, which differs from `rotation` when it is being shoved - damage/hp, ammo, weapon, shield/boost, `ring` - the health ring's opacity 0..1, nearest_ally_px; enemies also `role` - player/hunter/guard - and dist_to_player), projectiles, pickups, `frogs` (a list with `side` player/enemy: the player's frog first, then the enemy frog in a hunt round; `facing` left/right is which way the sprite is drawn - the art is authored facing right and mirrored for the other way, so it says whether a hop or a bite reads correctly), `portals` with `portals_active` (the map's portal anchors; each tank's `portal_cooldown` says when it may enter one again), `engage` (the engagement rings: per enemy its status - engaged/wreck/fleeing/retreating/out_of_range - the ring slot it holds and its target point, on the ring around the player or, for a hunter, the one around the player's frog; an engaged enemy with ring=null steers at its target directly, the pile-up case) and `clusters` (groups of live enemies within 90 px of each other). detail=full adds each enemy's AI memory (role, waypoint, committed heading, last behaviour-tree action, stuck timer, intent), the per-enemy slot rejection tally (claimed/off_map/unreachable/no_los), the player ring's 16-slot table (point, line of sight, who holds it) and `command`, the enemy command layer's last decision (orders by slot, the skipped-conflict tally, the blackboard; `enabled` false while the `c2_enabled` knob is off).",
        schema: r#"{"type":"object","properties":{"detail":{"type":"string","enum":["compact","full"],"default":"compact"}}}"#,
        read_only: true,
        destructive: false,
    },
    ToolSpec {
        name: "events",
        description: "Gameplay events recorded since `since` (a seq number; 0 = everything kept, up to 4096): fired, hit, wreck, ram, deflected (off a shield), ricochet (a shot off iron or a barrel, with the heading it flies on along), laser_beam (a laser's beam from the muzzle to where it stopped, with its variant), shells_collided, frog_bite (with the biting frog's side), pickup_collected, pickup_respawned, obstacle_destroyed, blast, drum_launched, fire_started, ignited (the flamethrower lit `what`: ground, oil, wood, tree, drum, or collapsed a sandbag/fence), teleported (a tank went through a portal: slot, from x/y, to to_x/to_y), round_started, round_ended, plus AI decisions - ai_action (behaviour-tree action changed), engage_slot (ring slot changed; null = steering at its target - the player, or a hunter's frog - directly), stuck_escape, breach (dir, or null when it ends), retreat (on/off), alert (shared last-known player position on/off), retarget (rounds with more than one seat: the enemy switched to fighting seat `player`). Each carries the frame it happened on. `kinds` keeps only those event names, `exclude` drops them.",
        schema: r#"{"type":"object","properties":{"since":{"type":"integer","default":0,"description":"Return events with seq > since"},"limit":{"type":"integer","default":200},"kinds":{"type":"array","items":{"type":"string"},"description":"Only these event names"},"exclude":{"type":"array","items":{"type":"string"},"description":"Drop these event names"}}}"#,
        read_only: true,
        destructive: false,
    },
    ToolSpec {
        name: "step",
        description: "Freeze the game in lockstep and advance exactly `frames` simulation frames at the fixed 1/60 s timestep (all in one rendered frame, so it is fast and deterministic). Optional player input is held for those frames (`move_dir`/`face`/`fire` for player 1, `p2_move_dir`/`p2_face`/`p2_fire` for player 2 in a two-player round); shells/plasma fire once per press, so set fire_every=N to tap the trigger every N frames instead of holding it. Replies with the events of the step (first 256; `kinds`/`exclude` filter by event name, see `events`) and, by default, a compact snapshot. Use `resume` to let the game run in real time again.",
        schema: r#"{"type":"object","properties":{"frames":{"type":"integer","default":1,"minimum":1,"maximum":100000},"move_dir":{"type":"string","enum":["up","down","left","right"]},"face":{"type":"string","enum":["up","down","left","right"]},"fire":{"type":"boolean"},"p2_move_dir":{"type":"string","enum":["up","down","left","right"]},"p2_face":{"type":"string","enum":["up","down","left","right"]},"p2_fire":{"type":"boolean"},"fire_every":{"type":"integer","minimum":1,"description":"With fire=true: press the trigger on frames 0, N, 2N... and release in between (both players)"},"snapshot":{"type":"boolean","default":true},"detail":{"type":"string","enum":["compact","full"],"default":"compact"},"kinds":{"type":"array","items":{"type":"string"}},"exclude":{"type":"array","items":{"type":"string"}}}}"#,
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "input",
        description: "Override a player's input for the next `frames` real-time frames (keyboard is ignored meanwhile): `move_dir`/`face`/`fire` for player 1, `p2_move_dir`/`p2_face`/`p2_fire` for player 2 in a two-player round. Works while the game runs; in lockstep prefer step's own input fields. `cycle_overlays: true` presses the I key once: cycles the overlay presets off -> inspect -> all -> off (with no move_dir/face/fire it leaves the keyboard alone).",
        schema: r#"{"type":"object","properties":{"move_dir":{"type":"string","enum":["up","down","left","right"]},"face":{"type":"string","enum":["up","down","left","right"]},"fire":{"type":"boolean"},"p2_move_dir":{"type":"string","enum":["up","down","left","right"]},"p2_face":{"type":"string","enum":["up","down","left","right"]},"p2_fire":{"type":"boolean"},"frames":{"type":"integer","default":1},"cycle_overlays":{"type":"boolean","default":false}}}"#,
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "pause",
        description: "Enter lockstep: the game stops advancing (no PAUSED overlay, so screenshots stay clean) until `step` or `resume`.",
        schema: NO_PARAMS,
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "resume",
        description: "Leave lockstep and clear the P-key pause: the game runs in real time with wall-clock dt again.",
        schema: NO_PARAMS,
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "restart",
        description: "Start a fresh round, frozen in lockstep (call `resume` to let it run in real time). Optional seed (number or 0x-hex string; pinned for later restarts too), enemy count, player 1's chassis as `tank` (a name - scout, assault, ..., leviathan - the spelling maps and `--tank` use) or `tank_row` (0-11, the sheet order), `players` (1 to 8 - how many seats the round holds, kept for later restarts; the keyboard drives the first two and the rest stand idle in a local round, and `tank2`/`tank2_row` pin seat 1's chassis the way `tank` pins seat 0's), the map: `map` (a path to a TOML under maps/) or `map_toml` (the map's TOML text inline - see `map_get` for the format; the round keeps its current map when neither is given), and the level: `mission` (protect|hunt|destroy), `spawn` (band|waves) with `waves`/`wave_size`/`wave_growth`/`tier_start`/`tier_end` (light|medium|heavy|super) - each pinned for later restarts too, overriding the map's own [mission]/[spawn] tables. `intro: true` starts the round frozen behind the mission banner (off by default so `step` counts play frames). Same seed + same steps replays bit-for-bit.",
        schema: r#"{"type":"object","properties":{"seed":{"type":["integer","string"]},"enemies":{"type":"integer","minimum":0,"maximum":31,"description":"Band plan enemy count; 0 is a sandbox round that never ends by wreck count"},"tank":{"type":"string","enum":["scout","assault","breaker","longbow","flak","wraith","warden","ravager","glacier","obelisk","titan","leviathan"],"description":"Player 1's chassis by name (or tank_row)"},"tank_row":{"type":"integer","minimum":0,"maximum":11},"players":{"type":"integer","minimum":1,"maximum":8,"description":"Seats the round holds; enemies count from the slot after them"},"tank2":{"type":"string","enum":["scout","assault","breaker","longbow","flak","wraith","warden","ravager","glacier","obelisk","titan","leviathan"],"description":"Player 2's chassis by name (or tank2_row)"},"tank2_row":{"type":"integer","minimum":0,"maximum":11},"map":{"type":"string","description":"Path to a map .toml, relative to the game's working directory"},"map_toml":{"type":"string","description":"Map TOML text, e.g. `version = 1\ntanks = 4\ncells.\"20,8\" = { kind = \"wall\", material = \"iron\" }`"},"mission":{"type":"string","enum":["protect","hunt","destroy"]},"spawn":{"type":"string","enum":["band","waves"]},"waves":{"type":"integer","minimum":1},"wave_size":{"type":"integer","minimum":1},"wave_growth":{"type":"integer","minimum":0},"tier_start":{"type":"string","enum":["light","medium","heavy","super"]},"tier_end":{"type":"string","enum":["light","medium","heavy","super"]},"intro":{"type":"boolean"}}}"#,
        read_only: false,
        destructive: true,
    },
    ToolSpec {
        name: "map_get",
        description: "The current map as TOML text (plus name, cell count, default tank count) - edit it and hand it back through `restart {map_toml}`. Format: `version = 1`, optional `tanks = N` (default enemy count), optional `tank = \"titan\"` / `tank2 = \"scout\"` (the players' chassis), optional `theme = \"grass\"|\"desert\"` (the look - ground tileset and tall-grass sheet, grass when absent), optional `weather = \"night\"` (the sky, presentation only: clear, night, dusk, rain, storm, fog, sandstorm, snow or heat_haze; clear when absent), and one `cells.\"col,row\"` entry per occupied 32 px grid cell (col/row from 0 at the top-left; the field is the map's optional `size = [cols, rows]`, 34 x 17 = 1088x544 when absent): `{ kind = \"wall\", material = \"brick\"|\"iron\"|\"wood\"|\"glass\" }`, `{ kind = \"sandbag\" }` / `{ kind = \"barrel\" }` / `{ kind = \"fence\" }` (destructible props: shots sometimes pass over sandbags, barrels explode and chain, fences snap; tanks ram all three), `{ kind = \"barrel\", drum = \"oil\"|\"fuel\" }` (a pinned drum kind: oil leaves a burning pool, fuel goes off harder and launches when another blast sets it off; without `drum` the kind is rolled), `{ kind = \"oil\" }` (an oil trail cell: not solid, a fuse on the ground - a blast or a burning neighbour lights it and the fire runs along it, setting off any drum it reaches), `{ kind = \"tree\" }` / `{ kind = \"pine\" }` (destructible trees, solid like a prop but drawn larger than their cell; they often catch fire when killed and a tank can flatten one by driving into it), `{ kind = \"tall_grass\" }` (not solid - cover a tank hides in, enemies cannot shoot what is standing in it), `{ kind = \"road\" }`, `{ kind = \"water\" }` (a river where it is one cell wide, a lake where it is wider; a lake's open middle is deep - hulls cannot enter, shots fly over - and every other water cell is a ford that slows a hull and, in a north-south stream, carries it downstream; fire never lights on water, frogs hop toward it), `{ kind = \"frog\" }` (one), `{ kind = \"start\" }` (player 1, one), `{ kind = \"start2\" }` (player 2, one, optional - placed beside player 1 when absent, as every seat past the second always is), `{ kind = \"pickup\", pickup = \"health\"|\"ammo\"|\"laser\"|\"minigun\"|\"plasma\"|\"missiles\"|\"speedup\"|\"shield\"|\"flamethrower\"|\"frog_health\" }` (missiles are a four-tube pod firing two salvos of four seeker missiles per pull that climb, lock onto the nearest opposing tank and dive on it over any wall; the flamethrower is player-only: enemies drive over its fuel tank; the frog health pack fully heals the collector's own frog and is left on the ground by a tank whose frog is already at full health). Iron is indestructible, the rest can be shot away. Border walls and enemy spawns are added by the game on top.",
        schema: NO_PARAMS,
        read_only: true,
        destructive: false,
    },
    ToolSpec {
        name: "lint",
        description: "Run the static map linter (src/maplint.rs, the check CI runs on every shipped map) and reply with its findings. `source: builder` lints the builder's canvas as it stands (the default in build mode - validate a map authored with `builder_paint` before `play`); `source: round` lints the map the current round was built from, fresh (the default in play mode - not the round's current, partly shot-away terrain). The map is set up as a headless round with the session's seed, player count and CLI/restart overrides, so the check sees what PLAY would run. Each finding has `severity` (error|warning|info), `kind` (a tag such as gated-pickup, spawn-band-too-tight, gate-blocked, player2-unreachable) and `message`; `errors`/`warnings` count them. Two limits: the spawn-band check reads the map's own `spawn` table (not a `restart {spawn}` override), and the player-2 kinds appear only in a session with more than one seat.",
        schema: r#"{"type":"object","properties":{"source":{"type":"string","enum":["builder","round"],"description":"builder = the canvas (default in build mode); round = the round's map (default in play mode)"}}}"#,
        read_only: true,
        destructive: false,
    },
    ToolSpec {
        name: "terrain",
        description: "The battlefield's tiles and its fire layer as JSON - the numeric view of props, walls and flames that `snapshot` (tanks only) lacks: every live obstacle tile by grid `cell` with material, `hp`/`max_hp`, and when set `drum` (oil|fuel), `burning`/`burn_elapsed`, `fuse` {left, total} (an armed barrel), `heat` (flame exposure), `scorched` (blast-sooted faces, N E S W as bits 0..3), `ram_timer`, `flammable`; plus `fires` (burning ground cells: left, total, pool), `fused` (armed drums' cells), `flames` (this frame's flamethrower jets: shooter slot, origin, direction, range, reach), `burning_tanks`/`burning_wrecks`, and counts of burning tiles, flying drums, oil cells, grass cells and heated cells, plus `portals` (the map's portal anchors by cell) and `portals_active`. `only` keeps just the damaged (hurt, burning, fused, sooted, heated or rammed), burning or fused tiles; `materials` keeps the listed ones. At most 800 tiles (`truncated`).",
        schema: r#"{"type":"object","properties":{"only":{"type":"string","enum":["all","damaged","burning","fused"],"default":"all"},"materials":{"type":"array","items":{"type":"string","enum":["brick","iron","wood","glass","sandbag","barrel","fence","tree","pine"]},"description":"Only tiles of these materials"}}}"#,
        read_only: true,
        destructive: false,
    },
    ToolSpec {
        name: "history",
        description: "Per-tank rows recorded every frame (last 60 s, cleared on restart): position, heading (`rotation`) and the drawn turret angle, behaviour-tree action, ring slot, stuck, touching terrain, is_player. Replies with every N-th frame's rows (`every`) over the last `last` frames, optionally one `slot`, plus per-tank aggregates over the whole window: frames seen, distance travelled, net displacement, cluster_frames (2+ other live enemies within 90 px), stuck_frames, no_ring_frames (engaged without a slot), touching_frames (static terrain), tank_touching_frames (another tank's hull - the jam signal, which unlike a ram count does not saturate), and `round`, the tank's heading-turn counters since the round began: turns (heading changes), u_turns (180 flips), reversals (A->B->A within 2 s, the probe's jitter unit), spins (a full same-direction circle of quarter-turns within 2 s while drifting under 60 px - the probe's spin rule; the turret visibly rotating in place), max_spin_deg, turret_deg (the turret sprite's total sweep) and last_turn_frame. The live-game counterpart of the probe's per-round stats.",
        schema: r#"{"type":"object","properties":{"slot":{"type":"integer","description":"Only this tank's rows (aggregates still cover every tank)"},"last":{"type":"integer","default":600,"minimum":1,"maximum":3600,"description":"Window in frames, ending at the latest recorded one"},"every":{"type":"integer","default":10,"minimum":1,"description":"Row sampling stride in frames"}}}"#,
        read_only: true,
        destructive: false,
    },
    ToolSpec {
        name: "screenshot",
        description: "Capture the current frame (the state after the latest step) as a PNG: returned inline and saved under target/devshots/. scale 0.5 (default) halves it; use 1.0 to read overlay text. Optionally set overlay flags in the same call (same as the `overlays` tool). source=scene skips the HUD and overlays.",
        schema: r#"{"type":"object","properties":{"scale":{"type":"number","default":0.5,"minimum":0.1,"maximum":1},"source":{"type":"string","enum":["screen","scene"],"default":"screen"},"overlays":{"type":"object","properties":{"nav_grid":{"type":"boolean"},"ai":{"type":"boolean"},"projectiles":{"type":"boolean"},"engage":{"type":"boolean"},"pickups":{"type":"boolean"},"hitboxes":{"type":"boolean"},"stats":{"type":"boolean"}}}}}"#,
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "overlays",
        description: "Set persistent debug overlays drawn on top of the game (visible to the human too), one flag at a time: nav_grid (blocked pathfinding cells), ai (each enemy's waypoint, heading, last behaviour-tree action), projectiles (hit boxes + velocity), engage (engagement-ring targets), pickups (collect radius), hitboxes (each tank's hull and turret damage boxes and its rounded movement collider), stats (each tank's readout card: ammo, weapon, hp, speed, velocity, collider size, an enemy's retreat/fire state). Omitted flags keep their value, an unknown flag is an error; replies with the current flags. The I key in the game window cycles presets instead (off -> inspect = hitboxes + stats -> all); `input {cycle_overlays: true}` presses it.",
        schema: r#"{"type":"object","properties":{"nav_grid":{"type":"boolean"},"ai":{"type":"boolean"},"projectiles":{"type":"boolean"},"engage":{"type":"boolean"},"pickups":{"type":"boolean"},"hitboxes":{"type":"boolean"},"stats":{"type":"boolean"}}}"#,
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "camera",
        description: "Pin the part of the world the window draws, for screenshots (docs/large-maps-follow-camera.md): `x`/`y` a world point in field pixels at the view's centre and `zoom` 1 or more - the view shows the field's size divided by the zoom, kept inside the field and snapped to whole 2 px blocks, and fills the field area under the HUD bar, which is drawn as ever. A field left out keeps the pin's own (the field's centre, zoom 1, for a first pin). A pin outranks a field map's follow camera. `reset: true` lets the pin go: the view is the map's again - the whole field for an arena, the follow camera for a field map; no parameters only report. The pin holds across restarts and map changes, clamped to each field, and changes only the picture - the round, the AI and the builder's canvas never see it. Replies like `status.camera`: `view` (whole|follow|pinned), the visible world `rect`, `scale` (bitmap px per world px), the scene `target` size in texels and a pin's `center` and `zoom` (a followed view's position is the one the window last drew, unknown until it has drawn one).",
        schema: r#"{"type":"object","properties":{"x":{"type":"number","description":"World x at the view's centre"},"y":{"type":"number","description":"World y at the view's centre"},"zoom":{"type":"number","minimum":1,"description":"How many times the field is magnified"},"reset":{"type":"boolean","default":false,"description":"Show the whole field again"}}}"#,
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "nav_grid",
        description: "The AI's pathfinding grid as text (# blocked, . open) with tanks (P player, digits enemies, x wrecks), the frog (F) and pickups (*) marked - the cheapest way to reason about the layout.",
        schema: NO_PARAMS,
        read_only: true,
        destructive: false,
    },
    ToolSpec {
        name: "field",
        description: "The flow field enemies follow toward a shared target this frame: `arrows` (one line per row: ^ v < > the neighbour each cell steps into, G the goal, # blocked, . unreachable) and `costs` (the cost to the goal per cell, -1 blocked/unreachable, so a priced firing lane or crowd cell shows as a jump). target: player (default), player2 or frog. Errors when that target is not in the round.",
        schema: r#"{"type":"object","properties":{"target":{"type":"string","enum":["player","player2","frog"],"default":"player"}}}"#,
        read_only: true,
        destructive: false,
    },
    ToolSpec {
        name: "teleport",
        description: "Move a tank to (x, y) in field pixels (velocity zeroed) and optionally snap its facing.",
        schema: r#"{"type":"object","properties":{"slot":{"type":"integer"},"x":{"type":"number"},"y":{"type":"number"},"facing":{"type":"string","enum":["up","down","left","right"]}},"required":["slot","x","y"]}"#,
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "set_tank",
        description: "Overwrite a tank's damage (0 = pristine, 100 = wreck), ammo counts (setting a special weapon's stock above 0 also arms it, like its pickup would), shield_hp (rainbow-shield absorption left in damage points, not seconds), the speed-boost timer and portal_cooldown (seconds before it may enter a portal again). Omitted fields are untouched.",
        schema: r#"{"type":"object","properties":{"slot":{"type":"integer"},"damage":{"type":"number"},"shells_ammo":{"type":"integer"},"minigun_ammo":{"type":"integer"},"missile_ammo":{"type":"integer"},"plasma_ammo":{"type":"integer"},"laser_charges":{"type":"integer"},"flame_fuel":{"type":"number"},"shield_hp":{"type":"number"},"speed_boost_timer":{"type":"number"},"portal_cooldown":{"type":"number"}},"required":["slot"]}"#,
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "kill",
        description: "Destroy a tank on the next simulated frame through the normal kill path (explosion, shockwave, wreck event, round end).",
        schema: SLOT_PARAMS,
        read_only: false,
        destructive: true,
    },
    ToolSpec {
        name: "spawn_enemy",
        description: "Add an enemy at (x, y) with an optional chassis row (0-11) and AI role (player - fights the player, the default; hunter - drives at and shoots the player's frog; guard - stays leashed to the enemy frog). Draws from the round RNG, so the round stops being the seeded replay afterwards. Returns the new slot.",
        schema: r#"{"type":"object","properties":{"x":{"type":"number"},"y":{"type":"number"},"row":{"type":"integer","minimum":0,"maximum":11},"role":{"type":"string","enum":["player","hunter","guard"]}},"required":["x","y"]}"#,
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "tuning_get",
        description: "Current tuning knobs as {name: value}; diff_only=true returns just the knobs that differ from the compiled defaults.",
        schema: r#"{"type":"object","properties":{"diff_only":{"type":"boolean","default":false}}}"#,
        read_only: true,
        destructive: false,
    },
    ToolSpec {
        name: "tuning_set",
        description: "Apply a tuning patch {knob: value} (array knobs as name.label or a full array) at this frame boundary. Range-checked; the whole patch is rejected on any bad key.",
        schema: r#"{"type":"object","properties":{"patch":{"type":"object","additionalProperties":{"type":["number","boolean","array"]}}},"required":["patch"]}"#,
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "tuning_reset",
        description: "Restore every tuning knob to its compiled default.",
        schema: NO_PARAMS,
        read_only: false,
        destructive: true,
    },
    ToolSpec {
        name: "tuning_schema",
        description: "The knob table (name, group, type, doc, range, default, when it applies). Several hundred rows, so filter by group and/or a name substring.",
        schema: r#"{"type":"object","properties":{"group":{"type":"string"},"name_contains":{"type":"string"}}}"#,
        read_only: true,
        destructive: false,
    },
    // ----- the two modes and the map builder (docs/game-editor-fusion.md section 11) -----
    ToolSpec {
        name: "mode",
        description: "Which mode the window is in - play (the round) or build (the map builder in the same window) - plus whether the leave-round dialog is open and the builder's state: dirty (edited since it was loaded), map name, active tool and its category, undo/redo depth. Cheap; `status` carries `mode` too.",
        schema: NO_PARAMS,
        read_only: true,
        destructive: false,
    },
    ToolSpec {
        name: "lang",
        description: "The language the window draws its text in (docs/localization-prd.md). Without `tag` reports the one in force and every shipped tag; with `tag` (`en`, `sl`) switches to it at this frame boundary - a tag the game has not got is answered with English. Works in every mode; nothing about the round changes.",
        schema: r#"{"type":"object","properties":{"tag":{"type":"string","description":"A shipped language tag to switch to"}}}"#,
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "weather",
        description: "The sky over the round on screen (docs/weather.md): drawn, and part of the rules - shorter enemy sight at night, in a storm and in fog, less grip in the rain, the water frozen in the snow, gusts in a sandstorm. A sky is settled when a round starts. Without `name` reports `in_force` (the round's sky - never `random`: a random weather is the sky the round's seed picks, the same for the same seed), `map` (the round's map's own `weather` key), `override` (the `weather_override` tuning knob's, null when it follows the map - `--weather` and the web page's `?weather=` set it; it applies from the next round), `rules` (`on`, `enemy_sight_px`, `grip`, `frozen`, `gust_on_player` - player 1's wind in px/s -, `gust_front` - the sandstorm gust crossing the field, its `start` in round seconds and its `dir`), `without_shaders` (the window draws its skies without the weather's shaders - they would not compile here, or the `weather_without_shaders` knob asks: the light map blended in, the snow, fog, sand, rain and falling snow as plain blocks) and every name. With `name` (clear, night, dusk, rain, storm, fog, sandstorm, snow, heat_haze, random) puts that key on the round's map and starts the round over on its own seed, frozen like `restart` leaves it; the key lasts through `restart`s on that map, and the override knob still outranks it. An online window only reports: a room's round is fought under its map's sky. The builder's WEATHER row is `builder_settings {weather}`; `map_get`/`restart {map_toml}` carry the key as `weather = \"night\"`.",
        schema: r#"{"type":"object","properties":{"name":{"type":"string","enum":["clear","night","dusk","rain","storm","fog","sandstorm","snow","heat_haze","random"],"description":"The sky to draw the round under; random is picked by the round's seed"}}}"#,
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "build",
        description: "Press BUILD in play mode: opens the 'Leave this round?' dialog while a round is in progress (the round is frozen until it is answered) and switches to the builder at once on the end screen. `answer: leave` confirms an open dialog and enters build mode; `answer: stay` closes it and keeps playing. A no-op in build mode. Replies like `mode`.",
        schema: r#"{"type":"object","properties":{"answer":{"type":"string","enum":["leave","stay"],"description":"Answer the open leave dialog instead of pressing the button"}}}"#,
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "players",
        description: "The players button in play mode (docs/two-players.md). Without `count`: press it - opens the 'How many players?' dialog (the round is frozen until it is answered) or closes an open one. With `count` (1 to 8): answer it - a different count restarts the round at once with that many seats, frozen in lockstep like `restart`; the current count just closes the dialog. The dialog itself only offers one and two - the couch counts; the rest are a room's, and reach the round through this tool or `restart`. The count sticks for the session (later `restart`s, PLAY from the builder). Two seats: slot 1 is player 2 and enemies count from 2; `step`/`input` take `p2_*` fields for it. Past two, the extra seats stand idle and the bar keeps the two-player readout. Replies like `mode`.",
        schema: r#"{"type":"object","properties":{"count":{"type":"integer","minimum":1,"maximum":2,"description":"Answer the dialog with this player count"}}}"#,
        read_only: false,
        destructive: true,
    },
    ToolSpec {
        name: "play",
        description: "Press PLAY in build mode: the builder's (edited) map becomes the round's map and a fresh round starts, frozen in lockstep like `restart` (so `step` counts play frames; `resume` for real time). A seed pinned by an earlier `restart` stays pinned. `intro: true` starts the round frozen behind the mission banner. Fails in play mode - `restart` starts a round there. Replies with `status`.",
        schema: r#"{"type":"object","properties":{"intro":{"type":"boolean","default":false}}}"#,
        read_only: false,
        destructive: true,
    },
    ToolSpec {
        name: "builder_tool",
        description: "Select the builder's brush by name - brick, iron, wood, glass (WALL); sandbag, barrel, oil_drum, fuel_drum, fence, tesla, tesla_enemy, gun_tower, gun_tower_enemy, bio_slush, bio_slush_enemy (PROP); road, water, tall_grass, tree, pine, oil_trail, gate, portal (GROUND); start, start2 (player 2's start), frog, enemy_frog (ACTOR); health, ammo, laser, minigun, plasma, missiles, speedup, shield, flamethrower, frog_health, tower_pack (PICKUP); or eraser - through the category's own selection path, so the bar's category button updates as well. Without `tool`, only reports the active tool and every category's current tool and full list (the authoritative spelling of every brush).",
        schema: r#"{"type":"object","properties":{"tool":{"type":"string","description":"A tool name (see the description) or eraser"}}}"#,
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "builder_paint",
        description: "One stroke on the builder's canvas: a press on cells[0], a drag through the rest, a release - so the toggle-erase rule (a press on a cell that already holds exactly the brush's object erases it, and paint-or-erase is decided on the first cell for the whole stroke), singleton moves (start/start2/frog/enemy_frog) and one-undo-step-per-stroke apply exactly as for a mouse. Cells are [col, row] on the 32 px grid (the map's `size`, 34 x 17 when absent, from the top-left). `tool` selects a brush first (see `builder_tool`); `button: right` erases whatever the brush. Replies with every changed cell's object before and after (in the map's own shape, null = empty) and the undo depth.",
        schema: r#"{"type":"object","properties":{"cells":{"type":"array","items":{"type":"array","items":{"type":"integer"},"minItems":2,"maxItems":2},"minItems":1,"description":"[[col, row], ...] in stroke order"},"tool":{"type":"string"},"button":{"type":"string","enum":["left","right"],"default":"left"}},"required":["cells"]}"#,
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "builder_undo",
        description: "Undo the builder's last `steps` edits (default 1; a whole stroke, one settings field, a load or a reset is one step each). Replies with how many were undone, the depth left on each side and the cells the last undone step changed.",
        schema: r#"{"type":"object","properties":{"steps":{"type":"integer","minimum":1,"default":1}}}"#,
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "builder_redo",
        description: "Redo the builder's last `steps` undone edits (default 1). Replies with how many were redone, the depth left on each side and the cells the last redone step changed.",
        schema: r#"{"type":"object","properties":{"steps":{"type":"integer","minimum":1,"default":1}}}"#,
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "builder_settings",
        description: "The builder's MAP settings - the map file's own level keys: tanks (enemy count 0-31), tank (player 1's chassis name), tank2 (player 2's, two-player rounds), mission (protect|hunt|destroy), spawn (band|waves), waves (1-20), wave_size (1-31), wave_growth (0-10), tier_start/tier_end (light|medium|heavy|super), theme (grass|desert - the look: ground tileset and tall-grass sheet; the canvas redraws in it at once), weather (clear|night|dusk|rain|storm|fog|sandstorm|snow|heat_haze - the sky the round is drawn under, docs/weather.md; the canvas itself stays clear to edit on), anchor (where the old map sits when the size changes: top_left|top|top_right|left|center|right|bottom_left|bottom|bottom_right - the panel's ANCHOR, kept for the session), size ([cols, rows] cells, 16 x 9 up to 250 a side: the map resized with the old one placed by the anchor, cells that land past the new edge dropped - one undo step, which brings them back - and the canvas made again on the new field). A field left out is untouched; a field set to null goes back to auto (unset: the game's own roll or the `waves` tuning group; mission/spawn back to protect/band). Each changed field is one undo step, in the order listed. `reset: true` then reverts cells and settings to the baseline (one undoable step). Replies with the current values (null = auto, `size` in cells) and `cli_overrides`: which of them a command-line flag (-e, --tank, --mission, ...) or an earlier `restart` parameter overrides at PLAY, so the map's value is not what the round will use.",
        schema: r#"{"type":"object","properties":{"tanks":{"type":["integer","null"],"minimum":0,"maximum":31},"tank":{"type":["string","null"],"description":"A chassis name, e.g. titan"},"tank2":{"type":["string","null"],"description":"Player 2's chassis name"},"mission":{"type":["string","null"],"enum":["protect","hunt","destroy",null]},"spawn":{"type":["string","null"],"enum":["band","waves",null]},"waves":{"type":["integer","null"],"minimum":1,"maximum":20},"wave_size":{"type":["integer","null"],"minimum":1,"maximum":31},"wave_growth":{"type":["integer","null"],"minimum":0,"maximum":10},"tier_start":{"type":["string","null"],"enum":["light","medium","heavy","super",null]},"tier_end":{"type":["string","null"],"enum":["light","medium","heavy","super",null]},"theme":{"type":["string","null"],"enum":["grass","desert",null],"description":"null = grass, the default"},"weather":{"type":["string","null"],"enum":["clear","night","dusk","rain","storm","fog","sandstorm","snow","heat_haze","random",null],"description":"null = clear, the default; random = a sky picked by each round's seed"},"anchor":{"type":"string","enum":["top_left","top","top_right","left","center","right","bottom_left","bottom","bottom_right"],"description":"Where the old map sits when the size changes"},"size":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2,"description":"[cols, rows] in cells, 16 x 9 to 250 x 250"},"reset":{"type":"boolean","default":false,"description":"Revert cells and settings to the baseline"}}}"#,
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "builder_map",
        description: "Without parameters: the builder's map as TOML (`toml`), its name, `dirty` and `diff` - cells added/removed/changed and the settings fields that differ from the baseline (the map as loaded). With `name` (a Load-list name from `builder_files`), `map` (a path under maps/) or `map_toml` (inline TOML, the `map_get` format): loads that map into the canvas as one undo step and makes it the new baseline - the FILE > LOAD path; the round keeps its map until `play`. With `clear`: FILE > CLEAR MAP - empties the canvas of every placed object as one undo step, keeping the settings, size, theme and name; not a new baseline, so `dirty` is set. `map_get` keeps answering with the map the current round was built from, which differs from this once the builder is dirty.",
        schema: r#"{"type":"object","properties":{"name":{"type":"string","description":"A name from builder_files"},"map_toml":{"type":"string","description":"Map TOML text to load into the builder"},"map":{"type":"string","description":"Path to a map .toml, relative to the game's working directory"},"clear":{"type":"boolean","default":false,"description":"Empty the canvas of every placed object (FILE > CLEAR MAP)"}}}"#,
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "builder_files",
        description: "What FILE > LOAD offers: every map the builder can load by name - the files under maps/ on native (`on_disk`) plus the maps shipped inside the binary (default, hunt-basic, waves-basic; all the web build has) - and `can_save`, whether this build writes maps to disk (native yes, web no: web edits live in memory for the session).",
        schema: r#"{"type":"object","properties":{}}"#,
        read_only: true,
        destructive: false,
    },
    ToolSpec {
        name: "builder_camera",
        description: "The builder's own camera over its canvas (docs/large-maps-follow-camera.md section 9), the way to frame a screenshot of the builder: `x`/`y` a world point in field pixels to put in the middle of the canvas and `zoom` how many times FIT (1 is FIT, the whole map); a field left out keeps the camera's own. `fit: true` goes back to the whole canvas, as the bar's FIT button does. The zoom is kept between FIT and a cell of `builder_zoom_max_cell_pt` points and the view inside the field, centred on an axis the map does not fill. Independent of the round's `camera` pin. No parameters only reports. Replies like `status.builder.camera`: `fit`, the world `rect` the canvas shows, `scale` (bitmap px per world px), `zoom` (times FIT), `fit_scale`, `device_scale` (device px per world px - whole blocks on the glass when it is a multiple of 0.5), `cell_mm` (a cell's width on a touch screen's glass, what the paint threshold reads) and the canvas `area` (bitmap px, under the bar).",
        schema: r#"{"type":"object","properties":{"x":{"type":"number","description":"World x (field px) for the middle of the canvas"},"y":{"type":"number","description":"World y (field px) for the middle of the canvas"},"zoom":{"type":"number","minimum":1,"description":"Times FIT; 1 is the whole map"},"fit":{"type":"boolean","default":false,"description":"Back to the whole canvas"}}}"#,
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "builder_touch",
        description: "Drive the builder with raw touch frames, the way a touch screen does - the multi-finger input `--touch-from-mouse` cannot make (src/editor/gesture.rs, docs/large-maps-follow-camera.md section 9): one finger paints once it moves past the touch slop (`builder_touch_slop_pt`; a resting finger paints nothing) and a quick one-finger tap paints a cell; two fingers pan and pinch-zoom about their middle, a second finger landing on a stroke takes it back, and a coarse screen's zoom settles on whole blocks when they part; a two-finger tap undoes and a three-finger tap redoes (`builder_tap_seconds`). Where a cell is under `builder_paint_min_cell_mm` on the glass (`status.builder.camera.cell_mm`) a one-finger tap zooms in to `builder_tap_zoom_cell_mm` instead and a drag pans. `frames` is a list of frames, each the touch points down that frame as {id, x, y} in bitmap pixels (the 32 px bar included, as for `click`); a frame with no points lifts every finger, and the tool lifts every finger at its end. Each frame is `dt` seconds (default 1/60). Build mode only. Replies like `mode`, with `camera` (as `status.builder.camera`).",
        schema: r#"{"type":"object","properties":{"frames":{"type":"array","items":{"type":"array","items":{"type":"object","properties":{"id":{"type":"integer"},"x":{"type":"number"},"y":{"type":"number"}},"required":["id","x","y"]}},"description":"Frames of touch points, first to last"},"dt":{"type":"number","minimum":0,"description":"Seconds per frame (default 1/60)"}},"required":["frames"]}"#,
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "builder_save",
        description: "FILE > SAVE / SAVE AS: write the builder's map to maps/<name>.toml (native only; `name` defaults to the map's own name and becomes it) and make the saved state the baseline, so `dirty` clears. Letters, digits, - and _ only. Replies like `builder_map`.",
        schema: r#"{"type":"object","properties":{"name":{"type":"string"}}}"#,
        read_only: false,
        destructive: true,
    },
    ToolSpec {
        name: "click",
        description: "A raw press at a window position - the window's own coordinates, which `status.ui.buttons` and `status.camera.window_field` give (with no window, the live mode's bitmap at its own size: the builder's under its 32 px bar, play's field alone) - in either mode, on the same hit-tests a mouse or a finger uses: in play mode the corners' buttons (BUILD, the players button, ONLINE, RESTART on a keyboard-less build, and the level button on a level - `status.ui.buttons`), either dialog's buttons (a press outside a dialog closes it), a level's end-screen buttons (LEVELS, PLAY AGAIN, the way on) and the level select's tiles and BACK (a press outside it closes it) - `status.ui.screen_buttons` - and a press on the field itself does nothing in play mode; the lobby's buttons (`status.ui.screen_buttons`); online the corners' LEAVE; in build mode the bar's buttons (PLAY starts the round like `play`), a dropdown row, a settings stepper or a field cell. With `drag_to`, a press, a straight drag to that point and a release, crossing every cell on the way. Replies like `mode`, with `world` - the world point the press landed on, through the camera - for a press on the field outside the builder. This tests the UI; `build`/`play`/`builder_*` address the model directly.",
        schema: r#"{"type":"object","properties":{"x":{"type":"number"},"y":{"type":"number"},"button":{"type":"string","enum":["left","right"],"default":"left"},"drag_to":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2,"description":"[x, y] to drag to before releasing"}},"required":["x","y"]}"#,
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "key",
        description: "Press one key for one frame: tab (BUILD/PLAY - in play mode it opens the leave dialog, or closes an open one; in build mode it starts the round like `play`), escape (keep playing / close a dialog or popup; in play mode with no dialog, open or close the level select), enter (leave the round; in the players dialog, switch to the other count; on a level's end screen, the way on or PLAY AGAIN; in the level select, start the level under the focus; confirm a popup), left / right / up / down (move the level select's focus over the open levels; in the builder, pan the canvas for one frame the way the arrow points), 1 / 2 (answer the players dialog), undo, redo (Ctrl+Z / Ctrl+Y in the builder), zoom_in / zoom_out (`+` / `-` in the builder: one zoom step about the canvas's middle), backspace; `text` types characters into an open builder prompt. Replies like `mode` (`levels_open`, `levels_focus`).",
        schema: r#"{"type":"object","properties":{"key":{"type":"string","enum":["tab","escape","enter","undo","redo","backspace","1","2","left","right","up","down","zoom_in","zoom_out"]},"text":{"type":"string","description":"Characters to type this frame (build mode)"}}}"#,
        read_only: false,
        destructive: false,
    },
];

/// **The room server's tool table** (`bongbong-server`, CLAUDE.md's room
/// server section): the same `ToolSpec` rows `TOOLS` is made of, over the
/// same newline-delimited JSON on a socket - `bbmcp rooms` is this
/// adapter pointed at the other port.
///
/// The *table* lives here rather than in `bongbong-server` because the
/// dependency runs the other way: the server crate is built on this one,
/// so a table there could not be read by a `bbmcp` that lives here. Pure
/// data, so it costs a headless build nothing; the dispatch is the
/// server's, and a test there holds every row to an arm so the two
/// cannot drift - `TOOLS`'s own discipline.
///
/// **What it is for.** The game's dev server drives the round in *this*
/// window; in an online round that window holds a replica and every
/// writing tool refuses by name, because only the server simulates it.
/// These are the other end - they read and drive the authoritative round
/// itself, which is the only place a co-op bug can actually be observed.
pub const ROOM_TOOLS: &[ToolSpec] = &[
    ToolSpec {
        name: "server_status",
        description: "The server as a whole: how many rooms it holds and its cap, whether it is draining, its uptime, the build's `protocol_version`, and the counters `/metrics` publishes (rooms by phase, seats, tick p50/p99 in microseconds, tick overruns, snapshot bytes/s, reconnects, intent starvations). Cheap; call first.",
        schema: NO_PARAMS,
        read_only: true,
        destructive: false,
    },
    ToolSpec {
        name: "rooms",
        description: "Every room this server holds: `code`, `phase` (waiting|playing|paused|ended), the seat count and how many are connected, the round's `tick`, the map, the seed and how long the room has been alive. The `code` is what every other tool here takes.",
        schema: NO_PARAMS,
        read_only: true,
        destructive: false,
    },
    ToolSpec {
        name: "room",
        description: "One room in full: its lifecycle and how long until the TTL that would end it, the map, seed, mission and resolved spawn plan, the round's tick and outcome, the `tuning_patch` it is being fought under (a room of two or more scales the waves; a room of one is the empty patch), and a row per seat - nick, whether it holds the room, ready, connected, its chassis, and its mailbox. `owns_hull` and `pose_refusals` say whether the seat's client owns its hull (stage 3) and how many of its poses the validator refused. **The mailbox row is the one to read when inputs feel lost**: a `depth` pinned at `BUFFER_MAX` means the client is running ahead and the oldest intents are being dropped, while a climbing `starvations` means it is not stamping far enough ahead and the tick is repeating its last intent.",
        schema: CODE_ONLY,
        read_only: true,
        destructive: false,
    },
    ToolSpec {
        name: "room_open",
        description: "Open a room with no client at all and start its round, so a scenario needs no window and no socket: `seats` bot seats are taken (1..=8, each with a mailbox `seat_intent` drives), and the round begins at once rather than waiting for a host to press START. Takes the setup a hosting client takes - a shipped map by name or `map_toml` whole, the mission, and a pinned `seed` so the round replays. Replies with the code. It is a real room: a player can join it by code and play alongside the bots.",
        schema: r#"{"type":"object","properties":{"map":{"type":"string","default":"default"},"map_toml":{"type":"string","description":"A whole map, instead of a shipped one by name"},"mission":{"type":"string","enum":["protect","hunt","destroy"]},"seed":{"type":"integer"},"seats":{"type":"integer","default":1,"minimum":1,"maximum":8}}}"#,
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "room_step",
        description: "Freeze a room's round and advance exactly `ticks` at the fixed 1/60 s timestep, the way the game's `step` drives the local round - so an online round replays deterministically from a pinned seed instead of racing the wall clock. The room stops ticking on real time until `room_resume`; connected clients still get their snapshots, so a window watching it simply sees the round advance in steps. Replies with the events of the step and, by default, a compact snapshot.",
        schema: r#"{"type":"object","required":["code"],"properties":{"code":{"type":"string"},"ticks":{"type":"integer","default":1,"minimum":1,"maximum":100000},"snapshot":{"type":"boolean","default":true},"kinds":{"type":"array","items":{"type":"string"}},"exclude":{"type":"array","items":{"type":"string"}}}}"#,
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "room_resume",
        description: "Let a frozen room tick on real time again (the opposite of `room_step`).",
        schema: CODE_ONLY,
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "seat_intent",
        description: "Post an intent into one seat's mailbox for the next `ticks` ticks, exactly as that seat's client would - so a bot seat can be driven, or a real seat's input stood in for. `move_dir`/`face`/`fire` are the human-settable fields the wire carries; nothing else travels. A shell fires once per press and a held trigger can never re-arm it, so set `fire_every=N` to tap every N ticks rather than holding. Posted at the client tick that seat's mailbox is expecting, so the server's `acked` and a client's own replay stay in step.",
        schema: r#"{"type":"object","required":["code","seat"],"properties":{"code":{"type":"string"},"seat":{"type":"integer","minimum":0,"maximum":7},"ticks":{"type":"integer","default":1,"minimum":1,"maximum":100000},"move_dir":{"type":"string","enum":["up","down","left","right"]},"face":{"type":"string","enum":["up","down","left","right"]},"fire":{"type":"boolean"},"fire_every":{"type":"integer","minimum":1,"description":"With fire=true: press on ticks 0, N, 2N... and release in between"}}}"#,
        read_only: false,
        destructive: false,
    },
    ToolSpec {
        name: "room_snapshot",
        description: "The room's authoritative world as JSON - the reading the game's `snapshot` gives, on the round the server is simulating rather than on a client's replica: every tank with position, rotation, velocity, health, ammo, weapon and (for enemies) role and target, plus projectiles, pickups, frogs, portals and the engagement rings. `detail=full` adds each enemy's AI memory. This is the truth a replica is checked against.",
        schema: r#"{"type":"object","required":["code"],"properties":{"code":{"type":"string"},"detail":{"type":"string","enum":["compact","full"],"default":"compact"}}}"#,
        read_only: true,
        destructive: false,
    },
    ToolSpec {
        name: "room_events",
        description: "The room's gameplay events since `since` (0 = everything kept): fired, hit, wreck, ram, pickups, obstacle_destroyed, teleported, round_started, round_ended and the rest - the vocabulary the game's `events` speaks, recorded on the authoritative round. `kinds` keeps only those names, `exclude` drops them. **The way to tell an input that never arrived from a shot the server refused**: no `fired` for a seat that pulled the trigger is the first, a `fired` with nothing after it the second.",
        schema: r#"{"type":"object","required":["code"],"properties":{"code":{"type":"string"},"since":{"type":"integer","default":0},"limit":{"type":"integer","default":200},"kinds":{"type":"array","items":{"type":"string"}},"exclude":{"type":"array","items":{"type":"string"}}}}"#,
        read_only: true,
        destructive: false,
    },
    ToolSpec {
        name: "room_close",
        description: "End a room now: its round stops, its seats are let go and the code stops resolving. For clearing up after a scenario; a room `room_open` made also ages out on its own TTL.",
        schema: CODE_ONLY,
        read_only: false,
        destructive: true,
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
}

/// A view the `camera` tool pinned: the world point at its centre and how
/// many times the field is magnified (`view::Camera::zoomed`, which keeps
/// it inside whichever field is on screen).
#[derive(Clone, Copy, Debug, PartialEq)]
struct CameraPin {
    center: Vec2,
    zoom: f32,
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
        }
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
    /// exactly its size - the builder's map under its bar, or play's field
    /// alone - which is all a server with no window ever has.
    fn click_frame(&self, session: &Session, width: f32, height: f32) -> (Layout, View, UiFrame) {
        let build = session.mode() == Driver::Build;
        match (&self.drawn, self.drawn_ui) {
            (Some(drawn), Some(ui)) if build == (drawn.mode == CameraMode::Build) => (drawn.layout, drawn.view, ui),
            _ => {
                let layout = if build {
                    let (w, h) = session.builder.map().field_size();
                    Layout::for_field(w, h)
                } else {
                    Layout::bare(width, height)
                };
                let (w, h) = layout.window_size();
                let window = (w as f32, h as f32);
                (layout, View::fit(window, window), UiFrame::plain(window))
            }
        }
    }

    /// `status.builder.navigator`: where the builder's navigator stands
    /// (`MapEditor::navigator_rect`) through the frame the window last drew
    /// the builder in - in window coordinates, what `click` takes, and as
    /// `bitmap` the same rect in the builder's bitmap pixels, what
    /// `builder_touch` takes; `null` where there is none (FIT on an arena)
    /// or the window is not in build mode.
    fn navigator_json(&self, session: &Session, width: f32, height: f32) -> Value {
        if session.mode() != Driver::Build {
            return Value::Null;
        }
        let (layout, view, _) = self.click_frame(session, width, height);
        match session.builder.navigator_rect(&layout) {
            Some(r) => {
                let a = view.to_window(Vec2::new(r.x, r.y));
                let b = view.to_window(Vec2::new(r.x + r.width, r.y + r.height));
                json!({
                    "x": a.x, "y": a.y, "w": b.x - a.x, "h": b.y - a.y,
                    "bitmap": { "x": r.x, "y": r.y, "w": r.width, "h": r.height },
                })
            }
            None => Value::Null,
        }
    }

    /// The corners the window lays out for the live mode in `ui`
    /// (`hud::corners`), `None` where it draws none.
    fn corners(session: &Session, ui: &UiFrame) -> Option<Corners> {
        CornerShape::of(&session.play_chrome(), session.shown().players.count()).map(|shape| crate::hud::corners(ui, &shape))
    }

    /// `camera`: pin a view, let it go, or say where it is. A pin's field
    /// left out keeps the one it had (the field's centre at zoom 1 for a
    /// first pin).
    fn pin_camera(&mut self, params: &Value, session: &Session, field: (f32, f32)) -> Result<Value, String> {
        if params.get("reset").and_then(Value::as_bool).unwrap_or(false) {
            self.camera = None;
            return Ok(self.camera_json(session, field));
        }
        let number = |key: &str| -> Result<Option<f32>, String> {
            match params.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(v) => match v.as_f64() {
                    Some(n) if n.is_finite() => Ok(Some(n as f32)),
                    _ => Err(format!("{key} must be a number, got {v}")),
                },
            }
        };
        let (x, y, zoom) = (number("x")?, number("y")?, number("zoom")?);
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
        // The AI-decision events exist for this server's `events` feed.
        session.game.trace_ai = true;
        self.observe_replica(session);
        while let Ok(req) = self.rx.try_recv() {
            self.dispatch(session, req, width, height);
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
                self.drain_events(game, Some((&mut sink, &step.filter)));
                step.events = sink;
                self.record_history(game);
                after_step(game);
            }
            let snapshot = step.want_snapshot.then(|| to_value(game.debug_snapshot(width, height, step.detail)));
            let _ = step.reply.send(Ok(json!({
                "frame": game.frame(),
                "time": r1(game.time),
                "outcome": game.outcome(),
                "restarted": step.restarted,
                "lockstep": true,
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
        json!({
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
            "intro_seconds_left": game.intro_timer,
            "paused": snap.paused,
            "lockstep": self.lockstep,
            "step_pending": self.pending_step.is_some(),
            "tanks": snap.tanks.len(),
            "enemies_alive": snap.tanks.iter().filter(|t| !t.is_player && !t.wreck).count(),
            "width": width,
            "height": height,
            "overlays": overlays_json(game),
            "players": game.players.count(),
            "map": map_json(&game.map),
            "weather": weather_json(game),
            "camera": self.camera_json(session, (width, height)),
            "ui": self.ui_json(session, width, height),
            "mode": session.mode().name(),
            "language": crate::text::language(),
            "dialog_open": session.dialog,
            "players_dialog_open": session.players_dialog,
            "levels_open": session.level_select.is_some(),
            "levels_focus": session.level_select.as_ref().map(|select| select.focus() + 1),
            "level": level_json(session),
            "stats": game.round_stats(),
            "builder": {
                "dirty": session.builder.dirty(),
                "tool": session.builder.tool().name(),
                "camera": builder_camera_json(&session.builder),
                "navigator": self.navigator_json(session, width, height),
            },
            "events_kept": self.events.len(),
            "next_event_seq": self.next_seq,
            "history_frames": self.history.len(),
            "turns": self.turns_summary(),
        })
    }

    fn dispatch(&mut self, session: &mut Session, req: Request, width: f32, height: f32) {
        let Request { method, params, reply } = req;
        // The game-only tools refuse while the builder is live rather than
        // touching a frozen round (docs/game-editor-fusion.md section 11).
        if session.mode() == Driver::Build && GAME_ONLY_TOOLS.contains(&method.as_str()) {
            let _ = reply.send(Err(format!("{method} needs play mode: the builder is live - call `play` first")));
            return;
        }
        // A step while the round stands still behind a question would
        // wait for frames that never run (`app.rs` advances only while
        // `Session::playing`): say what is asking instead.
        if method == "step" && session.mode() == Driver::Play && !session.playing() {
            let what = if session.level_select.is_some() {
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
        if session.mode() == Driver::Online && ONLINE_REFUSED_TOOLS.contains(&method.as_str()) {
            let room = session.online.as_ref().and_then(AnyRound::code).unwrap_or("-----");
            let _ = reply.send(Err(format!(
                "{method} needs a local round: this window holds a seat in room {room}, whose round only the server \
                 simulates - the replica on screen is a picture of it, not the authority. Give the seat up with \
                 `key {{\"key\": \"escape\"}}` to come back to the local round; the reading tools (status, snapshot, \
                 terrain, events, history, nav_grid, field, screenshot) already describe the online one"
            )));
            return;
        }
        // The tools that work on the whole session - the mode switch and
        // the builder - before the ones that only see the round.
        if let Some(result) = self.dispatch_session(session, &method, &params, width, height) {
            let _ = reply.send(result);
            return;
        }
        // The round on screen: the room's replica in an online round, the
        // session's own otherwise. Only `screenshot`/`overlays` write
        // through it, and only drawing flags.
        let game = session.shown_mut();
        let result = match method.as_str() {
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
                    let intents = parse_intent(&params, "").and_then(|p1| parse_intent(&params, "p2_").map(|p2| (p1, p2)));
                    match (frames_param(&params, 1), intents, detail_param(&params), event_filter(&params)) {
                        (Ok(remaining), Ok((intent, intent2)), Ok(detail), Ok(filter)) => {
                            self.lockstep = true;
                            self.pending_step = Some(PendingStep {
                                remaining,
                                intent,
                                intent2,
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
            "input" => match (parse_intent(&params, ""), parse_intent(&params, "p2_"), frames_param(&params, 1)) {
                (Ok(intent), Ok(intent2), Ok(frames)) => {
                    if let Some(intent) = intent {
                        self.injected = Some((intent, frames.min(u32::MAX as u64) as u32));
                    }
                    if let Some(intent) = intent2 {
                        self.injected2 = Some((intent, frames.min(u32::MAX as u64) as u32));
                    }
                    self.cycle_overlays_pending = params.get("cycle_overlays").and_then(Value::as_bool).unwrap_or(false);
                    Ok(json!({ "frames": frames, "cycle_overlays": self.cycle_overlays_pending }))
                }
                (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => Err(e),
            },
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
                                if let Err(e) = apply_overlays(game, flags) {
                                    let _ = reply.send(Err(e));
                                    return;
                                }
                            }
                            self.pending_shot = Some(PendingShot { scale: scale.clamp(0.1, 1.0), source, presented: false, reply });
                            return;
                        }
                        Err(e) => Err(e),
                    }
                }
            }
            "overlays" => apply_overlays(game, &params).map(|()| overlays_json(game)),
            "nav_grid" => Ok(json!({ "grid": game.nav_grid_ascii(width, height) })),
            "field" => {
                let target = match params.get("target").and_then(Value::as_str).unwrap_or("player") {
                    "player" => Ok(FieldTarget::Player(0)),
                    "player2" => Ok(FieldTarget::Player(1)),
                    "frog" => Ok(FieldTarget::Frog),
                    other => Err(format!("unknown field target {other:?}: player, player2 or frog")),
                };
                target.and_then(|target| {
                    game.field_dump(width, height, target)
                        .map(|dump| serde_json::to_value(dump).expect("FieldDump serialises"))
                        .ok_or_else(|| format!("no {target:?} in the round to build a field toward"))
                })
            }
            "terrain" => terrain_json(game, &params),
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
                // `TankPatch` is `serde(default)`, so an unknown key would
                // parse, do nothing, and still report success. Catch the one
                // field that was renamed rather than let a cached schema or
                // an old script silently no-op.
                if params.get("shield_timer").is_some() {
                    return Err("shield_timer is gone: the shield is a pool of absorption, not a timer - use shield_hp (damage points, over shield_capacity)".to_string());
                }
                let patch: TankPatch = serde_json::from_value(params.clone()).map_err(|e| e.to_string())?;
                game.debug_set_tank(slot, &patch)?;
                let snap = game.debug_snapshot(width, height, Detail::Compact);
                Ok(to_value(snap.tanks.into_iter().find(|t| t.slot == slot)))
            }),
            "kill" => slot_param(&params).and_then(|slot| game.debug_kill(slot).map(|()| json!({ "slot": slot, "applied_on_next_frame": true }))),
            "spawn_enemy" => match (f32_param(&params, "x"), f32_param(&params, "y")) {
                (Some(x), Some(y)) => {
                    let row = params.get("row").and_then(Value::as_i64).map(|r| r as i32);
                    let role = match params.get("role") {
                        None | Some(Value::Null) => Ok(None),
                        Some(v) => serde_json::from_value::<crate::ai::Role>(v.clone())
                            .map(Some)
                            .map_err(|_| format!("role: expected player, hunter or guard, got {v}")),
                    };
                    role.and_then(|role| {
                        game.debug_spawn_enemy(Position::new(x, y), row, role).map(|slot| json!({ "slot": slot }))
                    })
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

    fn restart(&mut self, session: &mut Session, params: &Value) -> Result<Value, String> {
        let game = &mut session.game;
        match params.get("seed") {
            None | Some(Value::Null) => {}
            Some(Value::Number(n)) => game.seed_override = Some(n.as_u64().ok_or("seed must be a non-negative integer")?),
            Some(Value::String(s)) => game.seed_override = Some(parse_seed(s)?),
            Some(other) => return Err(format!("seed must be a number or string, got {other}")),
        }
        if let Some(n) = params.get("enemies") {
            game.enemy_count_override = Some(n.as_u64().ok_or("enemies must be an integer")? as usize);
        }
        // A chassis as a row (the sheet order) or as its name (the spelling
        // maps, `builder_settings` and `--tank` use); not both.
        let chassis = |name_key: &str, row_key: &str| -> Result<Option<i32>, String> {
            match (params.get(name_key).filter(|v| !v.is_null()), params.get(row_key).filter(|v| !v.is_null())) {
                (Some(_), Some(_)) => Err(format!("give {name_key} (a name) or {row_key} (0-11), not both")),
                (Some(Value::String(s)), None) => parse_tank(s).map(|k| Some(k.row())).ok_or_else(|| {
                    format!("unknown {name_key} {s:?}; one of {}", TankKind::ALL.iter().map(|k| k.name()).collect::<Vec<_>>().join(", "))
                }),
                (Some(other), None) => Err(format!("{name_key} must be a chassis name, got {other}")),
                (None, Some(row)) => Ok(Some(row.as_i64().ok_or_else(|| format!("{row_key} must be an integer"))? as i32)),
                (None, None) => Ok(None),
            }
        };
        if let Some(row) = chassis("tank", "tank_row")? {
            game.player_row_override = Some(row);
        }
        if let Some(row) = chassis("tank2", "tank2_row")? {
            game.player2_row_override = Some(row);
        }
        if let Some(n) = params.get("players") {
            game.players = n
                .as_u64()
                .and_then(|n| PlayerCount::from_count(n as usize))
                .ok_or_else(|| format!("players must be 1 to {}, got {n}", crate::MAX_SEATS))?;
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
        // A restart is a play-mode thing: a builder session ends here, and
        // a new map replaces the builder's canvas as well as the round's.
        if let Some(map) = map_param(params)? {
            session.replace_map(map);
        }
        session.driver = Driver::Play;
        session.dialog = false;
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
    fn weather(&mut self, session: &mut Session, params: &Value) -> Result<Value, String> {
        if let Some(sky) = weather_name(params)? {
            if session.mode() == Driver::Online {
                return Err("weather: this window holds a seat in a room, whose round is fought under the room's map's sky - \
                            only reporting works here"
                    .into());
            }
            let game = &mut session.game;
            game.map.weather = sky;
            let pinned = game.seed_override.replace(game.round_seed());
            let (width, height) = game.map.field_size();
            game.init(width, height);
            game.seed_override = pinned;
            self.round_started(session);
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
            "status" => Ok(self.status(session, width, height)),
            "pause" => {
                self.lockstep = true;
                Ok(self.status(session, width, height))
            }
            "resume" => {
                self.lockstep = false;
                session.game.paused = false;
                Ok(self.status(session, width, height))
            }
            "restart" => self.restart(session, params),
            "weather" => self.weather(session, params),
            "camera" => self.pin_camera(params, session, (width, height)),
            "lint" => lint_json(session, params.get("source").and_then(Value::as_str)),
            "mode" => Ok(mode_json(session)),
            "lang" => {
                if let Some(tag) = params.get("tag").and_then(Value::as_str) {
                    crate::text::set_language(tag);
                }
                Ok(json!({
                    "language": crate::text::language(),
                    "shipped": crate::text::SHIPPED_LANGS.iter().map(|(tag, _)| *tag).collect::<Vec<_>>(),
                }))
            }
            "build" => match params.get("answer") {
                None | Some(Value::Null) => {
                    session.press_build();
                    Ok(mode_json(session))
                }
                Some(Value::String(s)) if s == "leave" => {
                    session.answer_dialog(true);
                    Ok(mode_json(session))
                }
                Some(Value::String(s)) if s == "stay" => {
                    session.answer_dialog(false);
                    Ok(mode_json(session))
                }
                Some(other) => Err(format!("answer must be leave|stay, got {other}")),
            },
            "players" => match params.get("count") {
                None | Some(Value::Null) => {
                    session.press_players();
                    Ok(mode_json(session))
                }
                Some(v) => match v.as_u64().and_then(|n| PlayerCount::from_count(n as usize)) {
                    Some(count) => {
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
                    None => Err(format!("count must be 1 to {}, got {v}", crate::MAX_SEATS)),
                },
            },
            "play" => {
                if session.mode() == Driver::Play {
                    Err("already in play mode: `restart` starts a fresh round here, `build` enters the builder".to_string())
                } else {
                    session.game.show_intro = params.get("intro").and_then(Value::as_bool).unwrap_or(false);
                    session.play();
                    self.round_started(session);
                    Ok(self.status(session, width, height))
                }
            }
            "builder_tool" => tool_param(params).map(|tool| {
                if let Some(tool) = tool {
                    session.builder.select_tool(tool);
                }
                tool_json(&session.builder)
            }),
            "builder_paint" => match (cells_param(params), button_param(params), tool_param(params)) {
                (Ok(cells), Ok(right), Ok(tool)) => {
                    if let Some(tool) = tool {
                        session.builder.select_tool(tool);
                    }
                    let changes = session.builder.stroke(&cells, right);
                    Ok(json!({
                        "changes": changes_json(&changes),
                        "undo_depth": session.builder.history().undo_depth(),
                    }))
                }
                (Err(e), _, _) | (_, Err(e), _) | (_, _, Err(e)) => Err(e),
            },
            "builder_undo" | "builder_redo" => steps_param(params).map(|steps| {
                let undo = method == "builder_undo";
                let mut done = 0;
                let mut last = None;
                for _ in 0..steps {
                    let step = if undo { session.builder.undo() } else { session.builder.redo() };
                    match step {
                        Some(step) => {
                            done += 1;
                            last = Some(step);
                        }
                        None => break,
                    }
                }
                let history = session.builder.history();
                let mut v = json!({
                    "undo_depth": history.undo_depth(),
                    "redo_depth": history.redo_depth(),
                    "cells": changes_json(&last.map(|s| s.cells()).unwrap_or_default()),
                });
                v[if undo { "undone" } else { "redone" }] = json!(done);
                v
            }),
            "builder_settings" => builder_settings(session, params),
            "builder_map" => {
                let by_name = match params.get("name") {
                    None | Some(Value::Null) => Ok(None),
                    Some(Value::String(n)) => Ok(Some(n.clone())),
                    Some(other) => Err(format!("name must be a string, got {other}")),
                };
                let clear = params.get("clear").and_then(Value::as_bool).unwrap_or(false);
                let loads_one = params.get("map").is_some() || params.get("map_toml").is_some();
                by_name.and_then(|name| match name {
                    _ if clear => {
                        if name.is_some() || loads_one {
                            return Err("clear takes no map: give one of name, map, map_toml or clear".to_string());
                        }
                        session.builder.clear();
                        builder_map_json(&session.builder)
                    }
                    Some(name) => {
                        if loads_one {
                            return Err("give one of name, map or map_toml".to_string());
                        }
                        session.builder.load_named(&name)?;
                        builder_map_json(&session.builder)
                    }
                    None => map_param(params).and_then(|map| {
                        if let Some(map) = map {
                            session.builder.load(map);
                        }
                        builder_map_json(&session.builder)
                    }),
                })
            }
            "builder_camera" => {
                let b = &mut session.builder;
                if params.get("fit").and_then(Value::as_bool).unwrap_or(false) {
                    b.fit_camera();
                } else {
                    let (x, y, zoom) = (f32_param(params, "x"), f32_param(params, "y"), f32_param(params, "zoom"));
                    if x.is_some() || y.is_some() || zoom.is_some() {
                        let vp = b.viewport();
                        let center = b.camera().center(&vp);
                        let zoom = zoom.unwrap_or(b.camera().scale(&vp) / vp.fit_scale());
                        b.frame_camera(Vec2::new(x.unwrap_or(center.x), y.unwrap_or(center.y)), zoom);
                    }
                }
                Ok(builder_camera_json(&session.builder))
            }
            "builder_touch" => {
                if session.mode() != Driver::Build {
                    Err("builder_touch needs build mode - call `build` first".to_string())
                } else {
                    let (layout, _, _) = self.click_frame(session, width, height);
                    touch_frames(params).map(|(frames, dt)| {
                        let mut down = false;
                        for touches in frames.into_iter().chain(std::iter::once(Vec::new())) {
                            let now = !touches.is_empty();
                            let input = BuilderInput {
                                pointer: touches.first().map(|t| t.pos),
                                pressed: now && !down,
                                held: now,
                                touches,
                                dt,
                                ..BuilderInput::default()
                            };
                            session.update_builder(&input, &layout);
                            down = now;
                        }
                        let mut reply = mode_json(session);
                        reply["camera"] = builder_camera_json(&session.builder);
                        reply
                    })
                }
            }
            "builder_files" => Ok(json!({
                "maps": crate::map::available_maps(),
                "can_save": crate::map::saving_available(),
            })),
            "builder_save" => {
                let name = params.get("name").and_then(Value::as_str);
                session.builder.save(name).and_then(|_| builder_map_json(&session.builder))
            }
            "click" => self.click(session, params, width, height),
            "key" => self.key(session, params, width, height),
            _ => return None,
        };
        Some(result)
    }

    /// `click`: one press (and optionally a drag) at a window position,
    /// through the same hit-tests `app.rs` runs on the mouse: the corners'
    /// buttons in UI points, the dialogs and the builder in the bitmap's
    /// pixels, each through the frame the window last drew
    /// (`click_frame`).
    fn click(&mut self, session: &mut Session, params: &Value, width: f32, height: f32) -> Result<Value, String> {
        let (Some(x), Some(y)) = (f32_param(params, "x"), f32_param(params, "y")) else {
            return Err("click needs numeric x and y (window coordinates, as `status.ui` and `status.camera` give them)".to_string());
        };
        let right = button_param(params)?;
        let drag_to = match params.get("drag_to") {
            None | Some(Value::Null) => None,
            Some(v) => match v.as_array().map(Vec::as_slice) {
                Some([dx, dy]) => match (dx.as_f64(), dy.as_f64()) {
                    (Some(dx), Some(dy)) => Some(Vec2::new(dx as f32, dy as f32)),
                    _ => return Err(format!("drag_to must be [x, y] numbers, got {v}")),
                },
                _ => return Err(format!("drag_to must be [x, y], got {v}")),
            },
        };
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
                if session.level_select.is_some() {
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
                } else if session.dialog {
                    let rects = leave_dialog_rects(ui.area);
                    if rects.leave.contains(p) {
                        session.answer_dialog(true);
                    } else if rects.stay.contains(p) || !rects.panel.contains(p) {
                        session.answer_dialog(false);
                    }
                } else if !right && session.press_result(p, ui.area) {
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
                let press = BuilderInput {
                    pointer: Some(point),
                    pressed: !right,
                    held: !right,
                    right_pressed: right,
                    right_held: right,
                    ..BuilderInput::default()
                };
                session.update_builder(&press, &layout);
                let mut last = point;
                if let Some(to) = drag_to.map(|to| view.to_bitmap(to)) {
                    let steps = (point.distance_to(to) / CLICK_DRAG_STEP_PX).ceil().max(1.0) as usize;
                    for i in 1..=steps {
                        let t = i as f32 / steps as f32;
                        last = Vec2::new(point.x + (to.x - point.x) * t, point.y + (to.y - point.y) * t);
                        let held = BuilderInput { pointer: Some(last), held: !right, right_held: right, ..BuilderInput::default() };
                        session.update_builder(&held, &layout);
                    }
                }
                session.update_builder(&BuilderInput { pointer: Some(last), ..BuilderInput::default() }, &layout);
                // The press may have been PLAY.
                if session.mode() == Driver::Play {
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
        if session.mode() == Driver::Build
            && let Some(world) = session.builder.world_at(point, &layout)
        {
            reply["world"] = json!({ "x": world.x, "y": world.y });
            reply["cell"] = json!(session.builder.cell_at(point, &layout).map(|(c, r)| [c, r]));
        }
        Ok(reply)
    }

    /// `key`: one key for one frame, or typed text, through the same
    /// paths `app.rs` takes for the keyboard.
    fn key(&mut self, session: &mut Session, params: &Value, width: f32, height: f32) -> Result<Value, String> {
        let (layout, _, ui) = self.click_frame(session, width, height);
        let layout = &layout;
        let key = match params.get("key") {
            None | Some(Value::Null) => None,
            Some(Value::String(s)) => Some(s.as_str()),
            Some(other) => return Err(format!("key must be one of {}, got {other}", KEY_NAMES.join("|"))),
        };
        if let Some(k) = key
            && !KEY_NAMES.contains(&k)
        {
            return Err(format!("unknown key {k:?}; one of {}", KEY_NAMES.join("|")));
        }
        let text = params.get("text").and_then(Value::as_str).unwrap_or("").to_string();
        if key.is_none() && text.is_empty() {
            return Err(format!("key needs `key` ({}) or `text`", KEY_NAMES.join("|")));
        }
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
                    if session.dialog {
                        session.answer_dialog(false);
                    } else {
                        session.press_build();
                    }
                }
                // Esc answers the leave dialog while it asks, and opens
                // the level select otherwise.
                Some("escape") => {
                    if session.dialog {
                        session.answer_dialog(false);
                    } else {
                        session.press_levels();
                    }
                }
                // The leave dialog's answer while it asks; otherwise a
                // level's end screen takes it, as `app.rs` does.
                Some("enter") => {
                    if session.dialog {
                        session.answer_dialog(true);
                    } else if session.enter_result() {
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
                        zoom_in: key == Some("zoom_in"),
                        zoom_out: key == Some("zoom_out"),
                        pan_keys: Vec2::new(arrow("left", "right"), arrow("up", "down")),
                        dt: crate::PHYSICS_FIXED_DT,
                        typed: text,
                        ..BuilderInput::default()
                    };
                    session.update_builder(&input, layout);
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
fn lint_json(session: &Session, source: Option<&str>) -> Result<Value, String> {
    let source = match source {
        None if session.mode() == Driver::Build => "builder",
        None => "round",
        Some(s @ ("builder" | "round")) => s,
        Some(other) => return Err(format!("source must be builder|round, got {other:?}")),
    };
    // In an online round the map to lint is the room's, as the replica
    // was built from it.
    let live = session.shown();
    let mut game = Game::default();
    game.map = if source == "builder" { session.builder.map().clone() } else { live.map.clone() };
    game.seed_override = Some(live.seed_override.unwrap_or_else(|| live.round_seed()));
    game.players = live.players;
    game.enemy_count_override = live.enemy_count_override;
    game.player_row_override = live.player_row_override;
    game.player2_row_override = live.player2_row_override;
    game.level_overrides = live.level_overrides;
    let (width, height) = game.map.field_size();
    game.init(width, height);
    let findings = crate::maplint::lint(&game, width, height);
    let count = |severity: LintSeverity| findings.iter().filter(|f| f.severity == severity).count();
    Ok(json!({
        "source": source,
        "map": map_json(&game.map),
        "seed": format!("{:#x}", game.round_seed()),
        "players": game.players.count(),
        "errors": count(LintSeverity::Error),
        "warnings": count(LintSeverity::Warning),
        "infos": count(LintSeverity::Info),
        "findings": findings
            .iter()
            .map(|f| json!({ "severity": f.severity.to_string(), "kind": f.kind.tag(), "message": f.message }))
            .collect::<Vec<_>>(),
    }))
}

/// `terrain`: every live tile plus the fire layer - see the tool's
/// description for the shape. Tiles are sorted by row then column, quiet
/// fields left out, so an untouched wall is one short line.
fn terrain_json(game: &Game, params: &Value) -> Result<Value, String> {
    let only = params.get("only").and_then(Value::as_str).unwrap_or("all");
    if !["all", "damaged", "burning", "fused"].contains(&only) {
        return Err(format!("only must be all|damaged|burning|fused, got {only:?}"));
    }
    let materials: Vec<String> = match params.get("materials") {
        None | Some(Value::Null) => Vec::new(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|v| v.as_str().map(str::to_string).ok_or_else(|| "materials must be an array of material names".to_string()))
            .collect::<Result<_, _>>()?,
        Some(other) => Err(format!("materials must be an array of material names, got {other}"))?,
    };
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
        "grass_cells": game.grass_cells.len(),
        "hot_cells": game.heat.len(),
    }))
}

/// `mode`'s reply: the session's mode and the builder's state in one look.
/// The level the local round is (docs/levels.md) - its number, map and
/// title, and the furthest one reached - or `null` in free play.
fn level_json(session: &Session) -> Value {
    let (Some(i), Some(campaign)) = (session.level(), session.campaign.as_ref()) else { return Value::Null };
    let Some(level) = campaign.levels.get(i) else { return Value::Null };
    json!({
        "number": i + 1,
        "count": campaign.levels.len(),
        "map": level.map,
        "title": level.title(),
        "reached": campaign.reached() + 1,
        "last": campaign.is_last(i),
    })
}

fn mode_json(session: &Session) -> Value {
    let b = &session.builder;
    json!({
        "mode": session.mode().name(),
        "dialog_open": session.dialog,
        "players_dialog_open": session.players_dialog,
        "levels_open": session.level_select.is_some(),
        "levels_focus": session.level_select.as_ref().map(|select| select.focus() + 1),
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
    json!({ "tool": b.tool().name(), "category": b.active_category().map(category_name), "categories": categories })
}

/// `builder_touch`'s frames - each a list of `{id, x, y}` touch points in
/// bitmap pixels - and its seconds per frame.
fn touch_frames(params: &Value) -> Result<(Vec<Vec<crate::touch::TouchPoint>>, f32), String> {
    let dt = match params.get("dt") {
        None | Some(Value::Null) => PHYSICS_FIXED_DT,
        Some(v) => match v.as_f64() {
            Some(s) if s.is_finite() && s >= 0.0 => s as f32,
            _ => return Err(format!("dt must be seconds, 0 or more, got {v}")),
        },
    };
    let Some(frames) = params.get("frames").and_then(Value::as_array) else {
        return Err("builder_touch needs `frames`: a list of frames, each a list of {id, x, y}".to_string());
    };
    let point = |p: &Value| -> Result<crate::touch::TouchPoint, String> {
        let (Some(id), Some(x), Some(y)) = (p.get("id").and_then(Value::as_i64), p.get("x").and_then(Value::as_f64), p.get("y").and_then(Value::as_f64)) else {
            return Err(format!("a touch point is {{id, x, y}} (an integer id and two numbers), got {p}"));
        };
        Ok(crate::touch::TouchPoint { id: id as i32, pos: Vec2::new(x as f32, y as f32) })
    };
    let frames = frames
        .iter()
        .map(|f| match f.as_array() {
            Some(points) => points.iter().map(point).collect::<Result<Vec<_>, _>>(),
            None => Err(format!("a frame is a list of touch points, got {f}")),
        })
        .collect::<Result<Vec<_>, _>>()?;
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
fn builder_settings(session: &mut Session, params: &Value) -> Result<Value, String> {
    /// `None` = absent (untouched), `Some(None)` = null (auto), `Some(Some)` = a value.
    fn u32_field(params: &Value, key: &str) -> Result<Option<Option<u32>>, String> {
        match params.get(key) {
            None => Ok(None),
            Some(Value::Null) => Ok(Some(None)),
            Some(v) => v
                .as_u64()
                .map(|n| Some(Some(n.min(u32::MAX as u64) as u32)))
                .ok_or_else(|| format!("{key} must be a non-negative integer or null, got {v}")),
        }
    }
    fn str_field(params: &Value, key: &str) -> Result<Option<Option<String>>, String> {
        match params.get(key) {
            None => Ok(None),
            Some(Value::Null) => Ok(Some(None)),
            Some(Value::String(s)) => Ok(Some(Some(s.clone()))),
            Some(v) => Err(format!("{key} must be a string or null, got {v}")),
        }
    }
    fn parse_or<T>(key: &str, s: &str, parse: impl Fn(&str) -> Option<T>, valid: &[&str]) -> Result<T, String> {
        parse(s).ok_or_else(|| format!("unknown {key} {s:?}; one of {}", valid.join(", ")))
    }
    const MISSIONS: &[&str] = &["protect", "hunt", "destroy"];
    const SPAWNS: &[&str] = &["band", "waves"];
    const TIERS: &[&str] = &["light", "medium", "heavy", "super"];
    const THEMES: &[&str] = &["grass", "desert"];
    let weathers: Vec<&str> = crate::map::Weather::ALL.iter().map(|w| w.name()).collect();
    let tanks: Vec<&str> = TankKind::ALL.iter().map(|k| k.name()).collect();

    let b = &mut session.builder;
    // One `apply_settings` per field so each change is its own step (and
    // the clamped result is re-read before the next).
    let mut s = b.settings();
    if let Some(v) = u32_field(params, "tanks")? {
        s.tanks = v;
        b.apply_settings(s);
        s = b.settings();
    }
    if let Some(v) = str_field(params, "tank")? {
        s.tank = v.map(|n| parse_or("tank", &n, parse_tank, &tanks)).transpose()?;
        b.apply_settings(s);
        s = b.settings();
    }
    if let Some(v) = str_field(params, "tank2")? {
        s.tank2 = v.map(|n| parse_or("tank2", &n, parse_tank, &tanks)).transpose()?;
        b.apply_settings(s);
        s = b.settings();
    }
    if let Some(v) = str_field(params, "mission")? {
        s.mission = v.map(|n| parse_or("mission", &n, parse_mission, MISSIONS)).transpose()?.unwrap_or_default();
        b.apply_settings(s);
        s = b.settings();
    }
    if let Some(v) = str_field(params, "spawn")? {
        s.spawn = v.map(|n| parse_or("spawn", &n, parse_spawn, SPAWNS)).transpose()?.unwrap_or_default();
        b.apply_settings(s);
        s = b.settings();
    }
    if let Some(v) = u32_field(params, "waves")? {
        s.waves = v;
        b.apply_settings(s);
        s = b.settings();
    }
    if let Some(v) = u32_field(params, "wave_size")? {
        s.size = v;
        b.apply_settings(s);
        s = b.settings();
    }
    if let Some(v) = u32_field(params, "wave_growth")? {
        s.growth = v;
        b.apply_settings(s);
        s = b.settings();
    }
    if let Some(v) = str_field(params, "tier_start")? {
        s.tier_start = v.map(|n| parse_or("tier_start", &n, parse_tier, TIERS)).transpose()?;
        b.apply_settings(s);
        s = b.settings();
    }
    if let Some(v) = str_field(params, "tier_end")? {
        s.tier_end = v.map(|n| parse_or("tier_end", &n, parse_tier, TIERS)).transpose()?;
        b.apply_settings(s);
        s = b.settings();
    }
    if let Some(v) = str_field(params, "theme")? {
        s.theme = v
            .map(|n| parse_or("theme", &n, crate::map::Theme::parse, THEMES))
            .transpose()?
            .unwrap_or_default();
        b.apply_settings(s);
        s = b.settings();
    }
    if let Some(v) = str_field(params, "weather")? {
        s.weather = v
            .map(|n| parse_or("weather", &n, crate::map::Weather::parse, &weathers))
            .transpose()?
            .unwrap_or_default();
        b.apply_settings(s);
    }
    // The anchor first, so a size in the same call is placed by it.
    match params.get("anchor") {
        None | Some(Value::Null) => {}
        Some(Value::String(name)) => {
            let names: Vec<&str> = crate::editor::Anchor::ALL.iter().map(|a| a.name()).collect();
            b.set_resize_anchor(parse_or("anchor", name, crate::editor::Anchor::parse, &names)?);
        }
        Some(v) => return Err(format!("anchor must be a string, got {v}")),
    }
    match params.get("size") {
        None | Some(Value::Null) => {}
        Some(v) => match v.as_array().map(Vec::as_slice) {
            Some([cols, rows]) => match (cols.as_f64(), rows.as_f64()) {
                (Some(cols), Some(rows)) if cols.is_finite() && rows.is_finite() => {
                    let anchor = b.resize_anchor();
                    b.resize(cols as f32, rows as f32, anchor);
                }
                _ => return Err(format!("size must be [cols, rows] numbers, got {v}")),
            },
            _ => return Err(format!("size must be [cols, rows], got {v}")),
        },
    }
    if params.get("reset").and_then(Value::as_bool).unwrap_or(false) {
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
        "mission": s.mission.name(),
        "spawn": s.spawn.name(),
        "waves": s.waves,
        "wave_size": s.size,
        "wave_growth": s.growth,
        "tier_start": s.tier_start.map(Tier::name),
        "tier_end": s.tier_end.map(Tier::name),
        "theme": s.theme.name(),
        "weather": s.weather.name(),
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

/// `map`/`map_toml` from `params` (`restart` and `builder_map`): `None`
/// when neither is given, an error for both.
fn map_param(params: &Value) -> Result<Option<MapFile>, String> {
    let map_path = params.get("map").filter(|v| !v.is_null());
    let map_toml = params.get("map_toml").filter(|v| !v.is_null());
    match (map_path, map_toml) {
        (Some(_), Some(_)) => Err("give either map (a path) or map_toml (inline TOML), not both".to_string()),
        (Some(path), None) => {
            let path = path.as_str().ok_or("map must be a path string")?;
            MapFile::load(Path::new(path)).map(Some)
        }
        (None, Some(text)) => {
            let text = text.as_str().ok_or("map_toml must be a string of map TOML")?;
            MapFile::from_toml_str(text).map(Some).map_err(|e| format!("map_toml: {e}"))
        }
        (None, None) => Ok(None),
    }
}

/// `tool` from `params`: `None` when absent, an error naming every valid
/// spelling for an unknown name.
fn tool_param(params: &Value) -> Result<Option<Tool>, String> {
    match params.get("tool") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Tool::parse(s).map(Some).ok_or_else(|| {
            let names: Vec<&str> = crate::editor::TOOLS.iter().map(|t| t.name()).collect();
            format!("unknown tool {s:?}; one of {}", names.join(", "))
        }),
        Some(other) => Err(format!("tool must be a string, got {other}")),
    }
}

/// `cells` from `params`: a non-empty list of `[col, row]` pairs.
fn cells_param(params: &Value) -> Result<Vec<(i32, i32)>, String> {
    let Some(Value::Array(items)) = params.get("cells") else {
        return Err("cells must be an array of [col, row] pairs".to_string());
    };
    if items.is_empty() {
        return Err("cells must hold at least one [col, row] pair".to_string());
    }
    items
        .iter()
        .map(|v| match v.as_array().map(Vec::as_slice) {
            Some([c, r]) => match (c.as_i64(), r.as_i64()) {
                (Some(c), Some(r)) => Ok((c as i32, r as i32)),
                _ => Err(format!("each cell must be [col, row] integers, got {v}")),
            },
            _ => Err(format!("each cell must be [col, row], got {v}")),
        })
        .collect()
}

/// `button` from `params`: `true` for the secondary (right) button.
fn button_param(params: &Value) -> Result<bool, String> {
    match params.get("button") {
        None | Some(Value::Null) => Ok(false),
        Some(Value::String(s)) if s == "left" => Ok(false),
        Some(Value::String(s)) if s == "right" => Ok(true),
        Some(other) => Err(format!("button must be left|right, got {other}")),
    }
}

/// `steps` from `params` (undo/redo): at least 1, default 1.
fn steps_param(params: &Value) -> Result<usize, String> {
    match params.get("steps") {
        None | Some(Value::Null) => Ok(1),
        Some(v) => match v.as_u64() {
            Some(n) if n >= 1 => Ok(n as usize),
            _ => Err(format!("steps must be a positive integer, got {v}")),
        },
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
        "weather": map.weather.name(),
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
        "map": game.map.weather.name(),
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
fn weather_name(params: &Value) -> Result<Option<crate::map::Weather>, String> {
    match params.get("name") {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(name)) => crate::map::Weather::parse(name).map(Some).ok_or_else(|| {
            let names: Vec<&str> = crate::map::Weather::ALL.iter().map(|w| w.name()).collect();
            format!("unknown weather {name:?}; one of {}", names.join(", "))
        }),
        Some(other) => Err(format!("name must be a weather's name, got {other}")),
    }
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

/// The overlay flags a tool may set, one per `Overlays` field.
const OVERLAY_FLAGS: [&str; 7] = ["nav_grid", "ai", "projectiles", "engage", "pickups", "hitboxes", "stats"];

/// Set only the overlay flags present in `flags`. A key that is not an
/// `OVERLAY_FLAGS` entry is an error naming them, and nothing is applied.
fn apply_overlays(game: &mut Game, flags: &Value) -> Result<(), String> {
    if let Some(unknown) = flags.as_object().and_then(|map| map.keys().find(|k| !OVERLAY_FLAGS.contains(&k.as_str()))) {
        return Err(format!("unknown overlay flag {unknown:?}; flags: {}", OVERLAY_FLAGS.join(", ")));
    }
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
    if let Some(b) = flag("hitboxes") {
        o.hitboxes = b;
    }
    if let Some(b) = flag("stats") {
        o.stats = b;
    }
    Ok(())
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
/// One player's intent from `move_dir`/`face`/`fire` under `prefix` (`""`
/// for player 1, `"p2_"` for player 2); `None` when none of the three is
/// present, so the keyboard keeps that player.
fn parse_intent(params: &Value, prefix: &str) -> Result<Option<Intent>, String> {
    let dir = |key: &str| -> Result<Option<Dir>, String> {
        let key = format!("{prefix}{key}");
        match params.get(&key).and_then(Value::as_str) {
            None => Ok(None),
            Some(s) => Dir::parse(s).map(Some).ok_or_else(|| format!("{key} must be up|down|left|right, got {s:?}")),
        }
    };
    let move_dir = dir("move_dir")?;
    let face = dir("face")?;
    let fire = params.get(format!("{prefix}fire")).and_then(Value::as_bool);
    if move_dir.is_none() && face.is_none() && fire.is_none() {
        return Ok(None);
    }
    Ok(Some(Intent { move_dir, face, fire: fire.unwrap_or(false), fire_aim_offset: 0.0, slow: 0.0 }))
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
        let spec = |name: &str| serde_json::from_str::<Value>(TOOLS.iter().find(|t| t.name == name).unwrap().schema).unwrap();
        assert_eq!(keys(&spec("overlays")), fields);
        assert_eq!(keys(&spec("screenshot")["properties"]["overlays"]), fields);
        assert_eq!(OVERLAY_FLAGS.iter().map(|f| f.to_string()).collect::<std::collections::BTreeSet<_>>(), fields);
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
        let rx = call(&tx, "status", json!({}));
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
        let mut s = game(7);
        let field = (W, H);
        let before = s.game.frame();
        let status = ask(&mut server, &tx, &mut s, "status", json!({})).unwrap();
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
        let status = ask(&mut server, &tx, &mut s, "status", json!({})).unwrap();
        assert_eq!(status["camera"], pinned);

        ask(&mut server, &tx, &mut s, "camera", json!({ "zoom": 4.0 })).unwrap();
        assert_eq!(server.camera(field), Camera::zoomed(field, Vec2::new(640.0, 360.0), 4.0), "the centre stays");
        let corner = ask(&mut server, &tx, &mut s, "camera", json!({ "x": 0.0, "y": 0.0 })).unwrap();
        assert_eq!((corner["rect"]["x"].as_f64(), corner["rect"]["y"].as_f64()), (Some(0.0), Some(0.0)), "{corner}");
        assert_eq!(corner["zoom"], 4.0, "the zoom stays");
        let report = ask(&mut server, &tx, &mut s, "camera", json!({})).unwrap();
        assert_eq!(report, corner, "no parameters only report");

        assert!(ask(&mut server, &tx, &mut s, "camera", json!({ "zoom": 0.5 })).unwrap_err().contains("zoom"));
        assert!(ask(&mut server, &tx, &mut s, "camera", json!({ "x": "left" })).unwrap_err().contains("x must be a number"));
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
        let status = at(&mut server, &mut s, "status", json!({})).unwrap();
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
        let status = at(&mut server, &mut s, "status", json!({})).unwrap();
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
        assert_eq!(m["dialog_open"], true, "{m}");
        at(&mut server, &mut s, "key", json!({ "key": "escape" })).unwrap();
        // A press on the field lands in the world through that camera: the
        // window's middle is the followed seat.
        let m = at(&mut server, &mut s, "click", json!({ "x": 960.0, "y": 540.0 })).unwrap();
        assert_eq!(m["world"], json!({ "x": seat.x, "y": seat.y }), "{m}");
        assert_eq!(m["dialog_open"], false);
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
        let status = at(&mut server, &mut s, "status", json!({})).unwrap();
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
        let st = ask(&mut server, &tx, &mut s, "status", json!({})).unwrap();
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
        let rx = call(&tx, "step", json!({ "frames": 1 }));
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
        let rx = call(&tx, "step", json!({ "frames": 1 }));
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
        let rx = call(&tx, "status", json!({}));
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
        // Setting a sky starts the round over under it on its own seed:
        // the rules read it from the start.
        for _ in 0..5 {
            game.game.update(Input::default(), crate::PHYSICS_FIXED_DT, W, H);
        }
        let seed = game.game.round_seed();
        let w = ask(&mut server, &mut game, "weather", json!({ "name": "storm" })).unwrap();
        assert_eq!(w["map"], "storm", "{w}");
        assert_eq!(w["in_force"], "storm", "{w}");
        assert_eq!(game.game.map.weather, crate::map::Weather::Storm);
        assert_eq!(game.game.frame(), 0, "the round started over");
        assert_eq!(game.game.round_seed(), seed, "on the seed it had");
        assert_eq!(game.game.seed_override, Some(6), "and the seed setting is left as it was");
        assert_eq!(w["rules"]["enemy_sight_px"], json!(crate::tuning::tuning().enemy_view_range * crate::tuning::tuning().night_sight_factor), "{w}");
        assert_eq!(w["rules"]["grip"], json!(crate::tuning::tuning().rain_grip_factor), "{w}");
        let err = ask(&mut server, &mut game, "weather", json!({ "name": "hail" })).unwrap_err();
        assert!(err.contains("heat_haze"), "{err}");
        assert_eq!(game.game.map.weather, crate::map::Weather::Storm, "a refused name changes nothing");
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
        let status = ask(&mut server, &mut game, "status", json!({})).unwrap();
        assert_eq!(status["weather"]["in_force"], "storm", "{status}");
        assert_eq!(status["map"]["weather"], "storm", "{status}");
        let map = ask(&mut server, &mut game, "map_get", json!({})).unwrap();
        assert!(map["toml"].as_str().unwrap().contains("weather = \"storm\""), "{map}");
        let settings = ask(&mut server, &mut game, "builder_settings", json!({ "weather": "fog" })).unwrap();
        assert_eq!(settings["weather"], "fog", "{settings}");
        assert_eq!(game.builder.map().weather, crate::map::Weather::Fog);
        ask(&mut server, &mut game, "build", json!({})).unwrap();
        ask(&mut server, &mut game, "build", json!({ "answer": "leave" })).unwrap();
        let err = ask(&mut server, &mut game, "weather", json!({ "name": "rain" })).unwrap_err();
        assert!(err.contains("play"), "{err}");
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

        let rx = call(&tx, "step", json!({ "frames": 120, "detail": "full" }));
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
        assert_eq!(snap["frogs"][0]["x"], 960.0, "{}", snap["frogs"]);
        assert_eq!(snap["frogs"][0]["side"], "player");
        assert_eq!(snap["frogs"].as_array().map(Vec::len), Some(1), "a protect round has one frog");
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
        let rx = call(&tx, "status", json!({}));
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
            let rx = call(&tx, "step", json!({ "frames": 300 }));
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
        let status = ask(server, tx, session, "status", json!({})).unwrap();
        let b = &status["ui"][list][name];
        let at = |k: &str| b[k].as_f64().unwrap_or_else(|| panic!("no {name} in `ui.{list}` of {status}"));
        json!({ "x": at("x") + at("w") / 2.0, "y": at("y") + at("h") / 2.0 })
    }

    /// BUILD, then confirm the dialog: the builder is live afterwards.
    fn enter_build(server: &mut DevServer, tx: &mpsc::Sender<Request>, session: &mut Session) {
        ask(server, tx, session, "build", json!({})).unwrap();
        let m = ask(server, tx, session, "build", json!({ "answer": "leave" })).unwrap();
        assert_eq!(m["mode"], "build", "{m}");
    }

    #[test]
    fn build_asks_mid_round_and_the_game_only_tools_refuse_in_build_mode() {
        let (mut server, tx) = DevServer::headless();
        let mut s = game(31);
        let m = ask(&mut server, &tx, &mut s, "mode", json!({})).unwrap();
        assert_eq!(m["mode"], "play");
        assert_eq!(m["dialog_open"], false);
        assert!(m["open_menu"].is_null());
        assert_eq!(m["tool"], "brick");
        assert_eq!(m["category"], "wall");
        assert_eq!(m["map_name"], "untitled", "an inline map has no file stem");
        // BUILD mid-round asks; the round is frozen meanwhile.
        let m = ask(&mut server, &tx, &mut s, "build", json!({})).unwrap();
        assert_eq!(m["mode"], "play", "{m}");
        assert_eq!(m["dialog_open"], true);
        assert!(!s.playing());
        let m = ask(&mut server, &tx, &mut s, "build", json!({ "answer": "stay" })).unwrap();
        assert_eq!(m["dialog_open"], false);
        assert!(s.playing());
        assert!(ask(&mut server, &tx, &mut s, "build", json!({ "answer": "maybe" })).is_err());
        // Leave: the builder is live and the round tools refuse.
        enter_build(&mut server, &tx, &mut s);
        let err = ask(&mut server, &tx, &mut s, "step", json!({ "frames": 1 })).unwrap_err();
        assert!(err.contains("play"), "{err}");
        for tool in GAME_ONLY_TOOLS {
            assert!(ask(&mut server, &tx, &mut s, tool, json!({})).is_err(), "{tool} must refuse in build mode");
        }
        let status = ask(&mut server, &tx, &mut s, "status", json!({})).unwrap();
        assert_eq!(status["mode"], "build", "{status}");
        assert_eq!(status["dialog_open"], false);
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
        for tool in ["status", "mode", "screenshot", "overlays", "map_get", "tuning_get", "tuning_set", "tuning_reset", "tuning_schema", "restart", "build", "play", "builder_tool", "builder_paint", "builder_undo", "builder_redo", "builder_settings", "builder_map", "builder_files", "builder_save", "click", "key"] {
            assert!(!GAME_ONLY_TOOLS.contains(&tool), "{tool} must not be game-only");
            assert!(TOOLS.iter().any(|t| t.name == tool), "{tool} is advertised");
        }
        // A second BUILD press in build mode is a no-op, and `build` on the
        // end screen skips the dialog.
        let m = ask(&mut server, &tx, &mut s, "build", json!({})).unwrap();
        assert_eq!(m["mode"], "build");
        ask(&mut server, &tx, &mut s, "play", json!({})).unwrap();
        s.game.debug_kill(0).unwrap();
        s.game.update(Input::default(), PHYSICS_FIXED_DT, W, H);
        let m = ask(&mut server, &tx, &mut s, "build", json!({})).unwrap();
        assert_eq!(m["mode"], "build", "{m}");
        assert_eq!(m["dialog_open"], false);
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

        let st = ask(&mut server, &tx, &mut s, "status", json!({})).unwrap();
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
        let st = ask(&mut server, &tx, &mut s, "status", json!({})).unwrap();
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
        for tool in ONLINE_REFUSED_TOOLS {
            let spec = TOOLS.iter().find(|t| t.name == *tool).unwrap_or_else(|| panic!("{tool} is not advertised"));
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
        assert!(ask(&mut server, &tx, &mut s, "nonsense", json!({})).unwrap_err().contains("unknown method"));

        // Esc gives the seat up; the local round is back and answers.
        let m = ask(&mut server, &tx, &mut s, "key", json!({ "key": "escape" })).unwrap();
        assert_eq!(m["mode"], "play", "{m}");
        let st = ask(&mut server, &tx, &mut s, "status", json!({})).unwrap();
        assert_eq!(st["round"]["kind"], "local", "{st}");
        assert!(st["round"]["room"].is_null());
        assert_eq!(st["tanks"], 5, "the local round is the one on screen again: {st}");
        // A `step` is answered from `advance`, one rendered frame later.
        let rx = call(&tx, "step", json!({ "frames": 1 }));
        server.before_frame(&mut s, W, H);
        server.advance(&mut s.game, Input::default(), 1, W, H, &mut |_| {});
        assert_eq!(rx.recv().unwrap().unwrap()["frame"], 1);
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
        let st = ask(&mut server, &tx, &mut s, "status", json!({})).unwrap();
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
        assert_eq!(cats.len(), 5);
        assert_eq!(cats[0]["name"], "wall");
        assert_eq!(cats[0]["current"], "iron");
        assert_eq!(cats[4]["tools"].as_array().unwrap().len(), 11, "{}", cats[4]);
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
        let campaign = crate::levels::Campaign::new(crate::levels::Levels::shipped(), None);
        let mut game = Game::default();
        game.enemy_count_override = Some(1);
        game.seed_override = Some(3);
        game.map = campaign.map(0).expect("level 1 opens");
        let (w, h) = game.map.field_size();
        game.init(w, h);
        let mut s = Session::new(game);
        s.set_campaign(campaign);
        let status = ask(&mut server, &tx, &mut s, "status", json!({})).unwrap();
        assert_eq!(status["level"]["number"], 1, "{status}");
        assert_eq!(status["level"]["map"], "lotus-lagoon");
        assert_eq!(status["stats"]["enemies"], 1);

        let enemy = s.game.world.query::<&crate::tank::Tank>().with::<&crate::ai::Ai>().iter().map(|t| t.owner_slot()).min().unwrap();
        s.game.debug_kill(enemy).unwrap();
        s.game.update(Input::default(), crate::PHYSICS_FIXED_DT, w, h);
        let status = ask(&mut server, &tx, &mut s, "status", json!({})).unwrap();
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
        assert_eq!((m["levels_open"].as_bool(), m["levels_focus"].as_u64()), (Some(true), Some(2)), "{m}");
        let err = ask(&mut server, &tx, &mut s, "step", json!({ "frames": 1 })).unwrap_err();
        assert!(err.contains("level select"), "{err}");

        // Only the open tiles are buttons: level 3 is locked, and a press
        // on it - where the window draws it, the chrome's area of a window
        // the size of play's bitmap - does nothing.
        let status = ask(&mut server, &tx, &mut s, "status", json!({})).unwrap();
        let screen = &status["ui"]["screen_buttons"];
        assert!(screen["level_2"].is_object() && screen["level_3"].is_null() && screen["back"].is_object(), "{screen}");
        let (ww, wh) = Layout::bare(w, h).window_size();
        let r = crate::level_select::tile_rect(UiFrame::plain((ww as f32, wh as f32)).area, 2);
        let locked = json!({ "x": r.x + r.width / 2.0, "y": r.y + r.height / 2.0 });
        let m = ask(&mut server, &tx, &mut s, "click", locked).unwrap();
        assert_eq!((m["levels_open"].as_bool(), m["level"]["number"].as_u64()), (Some(true), Some(2)), "a locked tile: {m}");

        let m = ask(&mut server, &tx, &mut s, "key", json!({ "key": "escape" })).unwrap();
        assert_eq!(m["levels_open"], false, "{m}");
        let m = ask(&mut server, &tx, &mut s, "key", json!({ "key": "escape" })).unwrap();
        assert_eq!(m["levels_open"], true, "Esc opens it over the round: {m}");
        let m = ask(&mut server, &tx, &mut s, "key", json!({ "key": "left" })).unwrap();
        assert_eq!(m["levels_focus"], 1, "{m}");
        let m = ask(&mut server, &tx, &mut s, "key", json!({ "key": "enter" })).unwrap();
        assert_eq!((m["levels_open"].as_bool(), m["level"]["number"].as_u64()), (Some(false), Some(1)), "{m}");
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
        let status = ask(&mut server, &tx, &mut s, "status", json!({})).unwrap();
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
        let screen = crate::editor::CanvasScreen { device_per_px: 3.0, points_per_px: 1.5, coarse: false };
        let layout = Layout::for_field(1088.0, 544.0);
        s.update_builder(&BuilderInput { screen: Some(screen), ..BuilderInput::default() }, &layout);
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
        assert!(ask(&mut server, &tx, &mut s, "builder_touch", json!({ "frames": [[{ "id": 1 }]] })).unwrap_err().contains("{id, x, y}"));
        assert!(ask(&mut server, &tx, &mut s, "builder_touch", json!({})).unwrap_err().contains("frames"));
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
cells."40,30" = { kind = "frog" }
"#;
        let (mut server, tx) = DevServer::headless();
        let mut s = game(44);
        ask(&mut server, &tx, &mut s, "restart", json!({ "map_toml": INLINE_MAP, "seed": 1 })).unwrap();
        enter_build(&mut server, &tx, &mut s);
        let arena = ask(&mut server, &tx, &mut s, "status", json!({})).unwrap();
        assert_eq!(arena["builder"]["navigator"], Value::Null, "none at FIT on an arena");

        ask(&mut server, &tx, &mut s, "restart", json!({ "map_toml": FIELD_MAP, "seed": 1 })).unwrap();
        enter_build(&mut server, &tx, &mut s);
        let status = ask(&mut server, &tx, &mut s, "status", json!({})).unwrap();
        let nav = status["builder"]["navigator"].clone();
        assert!(nav.is_object() && nav["bitmap"].is_object(), "a field map's navigator stands at FIT too: {status}");
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

        let bitmap = &nav["bitmap"];
        let (tx0, ty0) = at(bitmap, 0.5, 0.5);
        let (tx1, ty1) = at(bitmap, 0.8, 0.8);
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
        // falls off, the frog at 30,15 lands at 20,10.
        assert_eq!(s.builder.map().cell(20, 10), Some(&crate::map::CellObject::Frog));
        assert_eq!(s.builder.map().start_cell(), None);
        assert_eq!(s.builder.history().undo_depth() as u64, depth + 1);
        let m = ask(&mut server, &tx, &mut s, "builder_map", json!({})).unwrap();
        assert!(m["diff"]["settings"].as_array().unwrap().contains(&json!("size")), "{m}");
        ask(&mut server, &tx, &mut s, "builder_undo", json!({})).unwrap();
        assert_eq!(s.builder.map(), &before);
        assert!(ask(&mut server, &tx, &mut s, "builder_settings", json!({ "anchor": "nowhere" })).unwrap_err().contains("top_left"));
        assert!(ask(&mut server, &tx, &mut s, "builder_settings", json!({ "size": [24] })).unwrap_err().contains("[cols, rows]"));
    }

    #[test]
    fn click_and_key_take_the_same_paths_as_the_mouse_and_keyboard() {
        let (mut server, tx) = DevServer::headless();
        let mut s = game(36);
        ask(&mut server, &tx, &mut s, "restart", json!({ "map_toml": INLINE_MAP, "seed": 1 })).unwrap();
        // Play's bitmap is the field alone, the builder's the field under
        // its bar.
        let layout = Layout::bare(W, H);
        let build_layout = Layout::for_field(W, H);
        let centre = |r: crate::math::Rectangle| (r.x + r.width / 2.0, r.y + r.height / 2.0);
        // A click on BUILD opens the dialog like `build`.
        let build = corner_button(&mut server, &tx, &mut s, "build");
        let m = ask(&mut server, &tx, &mut s, "click", build).unwrap();
        assert_eq!(m["dialog_open"], true, "{m}");
        assert_eq!(m["mode"], "play");
        // A press outside the dialog keeps playing.
        let m = ask(&mut server, &tx, &mut s, "click", json!({ "x": 10.0, "y": 100.0 })).unwrap();
        assert_eq!(m["dialog_open"], false);
        // Tab opens it; Escape closes it; Enter leaves.
        let m = ask(&mut server, &tx, &mut s, "key", json!({ "key": "tab" })).unwrap();
        assert_eq!(m["dialog_open"], true, "{m}");
        let m = ask(&mut server, &tx, &mut s, "key", json!({ "key": "escape" })).unwrap();
        assert_eq!(m["dialog_open"], false);
        assert!(ask(&mut server, &tx, &mut s, "key", json!({ "key": "f1" })).is_err());
        assert!(ask(&mut server, &tx, &mut s, "key", json!({})).is_err());
        // A left click on the field in play mode is not an order; the
        // round is untouched by it.
        let m = ask(&mut server, &tx, &mut s, "click", json!({ "x": 640.0, "y": 32.0 + 400.0 })).unwrap();
        assert_eq!(m["mode"], "play", "{m}");
        assert_eq!(m["dialog_open"], false);
        // The dialog's own buttons, where `status.ui.screen_buttons` puts
        // them - centred on the window, in the UI's points.
        ask(&mut server, &tx, &mut s, "key", json!({ "key": "tab" })).unwrap();
        let stay = screen_button(&mut server, &tx, &mut s, "stay");
        let (ww, wh) = layout.window_size();
        let rects = leave_dialog_rects(UiFrame::plain((ww as f32, wh as f32)).area);
        let (sx, sy) = centre(rects.stay);
        assert_eq!((stay["x"].as_f64(), stay["y"].as_f64()), (Some(sx as f64), Some(sy as f64)), "the rects the painter reads");
        let m = ask(&mut server, &tx, &mut s, "click", stay).unwrap();
        assert_eq!(m["dialog_open"], false, "{m}");
        assert_eq!(m["mode"], "play");
        ask(&mut server, &tx, &mut s, "key", json!({ "key": "tab" })).unwrap();
        let leave = screen_button(&mut server, &tx, &mut s, "leave");
        let m = ask(&mut server, &tx, &mut s, "click", leave).unwrap();
        assert_eq!(m["mode"], "build", "{m}");

        // Build mode: a click on a field cell paints with the active brush,
        // a drag crosses every cell, undo is a key.
        ask(&mut server, &tx, &mut s, "builder_tool", json!({ "tool": "iron" })).unwrap();
        // A cell's world position is its centre (`map::cell_to_world`).
        let cell_centre = |c: i32, r: i32| (c as f32 * 32.0, build_layout.field.y + r as f32 * 32.0);
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
        // PLAY from the bar starts the round frozen, like `play` - on the
        // builder's own bitmap, its whole canvas under the bar.
        server.lockstep = false;
        let (fw, fh) = s.builder.map().field_size();
        let (px, py) = centre(crate::hud::mode_button_rect(Layout::for_field(fw, fh).panel));
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
        let schema: Value = serde_json::from_str(spec.schema).unwrap();
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
        let player = step(&mut server, &tx, &mut s, json!({ "frames": 10, "move_dir": "right" }))["snapshot"]["tanks"][0].clone();
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
        let st = ask(&mut server, &tx, &mut s, "status", json!({})).unwrap();
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
        let st = ask(&mut server, &tx, &mut s, "status", json!({})).unwrap();
        assert_eq!(st["turns"]["turns"], 0, "{}", st["turns"]);
        assert_eq!(st["turns"]["spins"], 0);
    }
}
