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
use crate::editor::{BuilderInput, Category, CellChange, MapEditor, Tool, parse_mission, parse_spawn, parse_tank, parse_tier};
use crate::hud::{leave_dialog_rects, mode_button_rect, players_button_rect, players_dialog_rects};
use crate::map::MapFile;
use crate::mode::{Driver, Session};
use crate::simulation::debug::{CLUSTER_RADIUS_PX, Detail, TankPatch, TrackRow};
use crate::simulation::{Event, Game, Input, Overlays, PlayerCount};
use crate::tank::{Dir, TankKind};
use crate::tuning;
use crate::level::{Mission, SpawnKind, Tier};
use crate::{Layout, PHYSICS_FIXED_DT, Position, parse_seed};

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
/// How far apart the points of a `click {drag_to}` drag are sampled: well
/// under a 32 px cell, so the stroke crosses every cell on the line.
const CLICK_DRAG_STEP_PX: f32 = 8.0;

/// The tools that read or drive the round and are refused in build mode
/// (docs/game-editor-fusion.md section 11) rather than touching a round
/// the builder has frozen.
pub const GAME_ONLY_TOOLS: &[&str] = &[
    "snapshot", "events", "step", "input", "pause", "resume", "history", "nav_grid", "teleport",
    "set_tank", "kill", "spawn_enemy", "players",
];

/// The `key` tool's key names.
const KEY_NAMES: &[&str] = &["tab", "escape", "enter", "undo", "redo", "backspace", "1", "2"];

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
        description: "World state as JSON: every tank (position, velocity, damage/hp, ammo, weapon, shield/boost, `ring` - the health ring's opacity 0..1, nearest_ally_px; enemies also `role` - player/hunter/guard), projectiles, pickups, `frogs` (a list with `side` player/enemy: the player's frog first, then the enemy frog in a hunt round), `engage` (the engagement rings: per enemy its status - engaged/wreck/fleeing/retreating/out_of_range - the ring slot it holds and its target point, on the ring around the player or, for a hunter, the one around the player's frog; an engaged enemy with ring=null steers at its target directly, the pile-up case) and `clusters` (groups of live enemies within 90 px of each other). detail=full adds each enemy's AI memory (role, waypoint, committed heading, last behaviour-tree action, stuck timer, intent), the per-enemy slot rejection tally (claimed/off_map/unreachable/no_los) and the player ring's 16-slot table (point, line of sight, who holds it).",
        schema: r#"{"type":"object","properties":{"detail":{"type":"string","enum":["compact","full"],"default":"compact"}}}"#,
    },
    ToolSpec {
        name: "events",
        description: "Gameplay events recorded since `since` (a seq number; 0 = everything kept, up to 4096): fired, hit, wreck, ram, deflected (off a shield), shells_collided, frog_bite (with the biting frog's side), pickup_collected, pickup_respawned, round_started, round_ended, plus AI decisions - ai_action (behaviour-tree action changed), engage_slot (ring slot changed; null = steering at its target - the player, or a hunter's frog - directly), stuck_escape, breach (dir, or null when it ends), retreat (on/off), alert (shared last-known player position on/off), retarget (two-player rounds: the enemy switched to fighting `player` 0 or 1). Each carries the frame it happened on. `kinds` keeps only those event names, `exclude` drops them.",
        schema: r#"{"type":"object","properties":{"since":{"type":"integer","default":0,"description":"Return events with seq > since"},"limit":{"type":"integer","default":200},"kinds":{"type":"array","items":{"type":"string"},"description":"Only these event names"},"exclude":{"type":"array","items":{"type":"string"},"description":"Drop these event names"}}}"#,
    },
    ToolSpec {
        name: "step",
        description: "Freeze the game in lockstep and advance exactly `frames` simulation frames at the fixed 1/60 s timestep (all in one rendered frame, so it is fast and deterministic). Optional player input is held for those frames (`move_dir`/`face`/`fire` for player 1, `p2_move_dir`/`p2_face`/`p2_fire` for player 2 in a two-player round); shells/plasma fire once per press, so set fire_every=N to tap the trigger every N frames instead of holding it. Replies with the events of the step (first 256; `kinds`/`exclude` filter by event name, see `events`) and, by default, a compact snapshot. Use `resume` to let the game run in real time again.",
        schema: r#"{"type":"object","properties":{"frames":{"type":"integer","default":1,"minimum":1,"maximum":100000},"move_dir":{"type":"string","enum":["up","down","left","right"]},"face":{"type":"string","enum":["up","down","left","right"]},"fire":{"type":"boolean"},"p2_move_dir":{"type":"string","enum":["up","down","left","right"]},"p2_face":{"type":"string","enum":["up","down","left","right"]},"p2_fire":{"type":"boolean"},"fire_every":{"type":"integer","minimum":1,"description":"With fire=true: press the trigger on frames 0, N, 2N... and release in between (both players)"},"snapshot":{"type":"boolean","default":true},"detail":{"type":"string","enum":["compact","full"],"default":"compact"},"kinds":{"type":"array","items":{"type":"string"}},"exclude":{"type":"array","items":{"type":"string"}}}}"#,
    },
    ToolSpec {
        name: "input",
        description: "Override a player's input for the next `frames` real-time frames (keyboard is ignored meanwhile): `move_dir`/`face`/`fire` for player 1, `p2_move_dir`/`p2_face`/`p2_fire` for player 2 in a two-player round. Works while the game runs; in lockstep prefer step's own input fields. `cycle_overlays: true` presses the I key once: cycles the overlay presets off -> inspect -> all -> off (with no move_dir/face/fire it leaves the keyboard alone).",
        schema: r#"{"type":"object","properties":{"move_dir":{"type":"string","enum":["up","down","left","right"]},"face":{"type":"string","enum":["up","down","left","right"]},"fire":{"type":"boolean"},"p2_move_dir":{"type":"string","enum":["up","down","left","right"]},"p2_face":{"type":"string","enum":["up","down","left","right"]},"p2_fire":{"type":"boolean"},"frames":{"type":"integer","default":1},"cycle_overlays":{"type":"boolean","default":false}}}"#,
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
        description: "Start a fresh round, frozen in lockstep (call `resume` to let it run in real time). Optional seed (number or 0x-hex string; pinned for later restarts too), enemy count, player chassis row (0-11), `players` (1 or 2 - the session's player count, kept for later restarts; `tank2_row` pins player 2's chassis), the map: `map` (a path to a TOML under maps/) or `map_toml` (the map's TOML text inline - see `map_get` for the format; the round keeps its current map when neither is given), and the level: `mission` (protect|hunt|destroy), `spawn` (band|waves) with `waves`/`wave_size`/`wave_growth`/`tier_start`/`tier_end` (light|medium|heavy|super) - each pinned for later restarts too, overriding the map's own [mission]/[spawn] tables. `intro: true` starts the round frozen behind the mission banner (off by default so `step` counts play frames). Same seed + same steps replays bit-for-bit.",
        schema: r#"{"type":"object","properties":{"seed":{"type":["integer","string"]},"enemies":{"type":"integer","minimum":0,"maximum":31,"description":"Band plan enemy count; 0 is a sandbox round that never ends by wreck count"},"tank_row":{"type":"integer","minimum":0,"maximum":11},"players":{"type":"integer","minimum":1,"maximum":2},"tank2_row":{"type":"integer","minimum":0,"maximum":11},"map":{"type":"string","description":"Path to a map .toml, relative to the game's working directory"},"map_toml":{"type":"string","description":"Map TOML text, e.g. `version = 1\ntanks = 4\ncells.\"20,8\" = { kind = \"wall\", material = \"iron\" }`"},"mission":{"type":"string","enum":["protect","hunt","destroy"]},"spawn":{"type":"string","enum":["band","waves"]},"waves":{"type":"integer","minimum":1},"wave_size":{"type":"integer","minimum":1},"wave_growth":{"type":"integer","minimum":0},"tier_start":{"type":"string","enum":["light","medium","heavy","super"]},"tier_end":{"type":"string","enum":["light","medium","heavy","super"]},"intro":{"type":"boolean"}}}"#,
    },
    ToolSpec {
        name: "map_get",
        description: "The current map as TOML text (plus name, cell count, default tank count) - edit it and hand it back through `restart {map_toml}`. Format: `version = 1`, optional `tanks = N` (default enemy count), optional `tank = \"titan\"` / `tank2 = \"scout\"` (the players' chassis), and one `cells.\"col,row\"` entry per occupied 32 px grid cell (40 columns x 23 rows at 1280x720, col/row from 0 at the top-left): `{ kind = \"wall\", material = \"brick\"|\"iron\"|\"wood\"|\"glass\" }`, `{ kind = \"sandbag\" }` / `{ kind = \"barrel\" }` / `{ kind = \"fence\" }` (destructible props: shots sometimes pass over sandbags, barrels explode and chain, fences snap; tanks ram all three), `{ kind = \"barrel\", drum = \"oil\"|\"fuel\" }` (a pinned drum kind: oil leaves a burning pool, fuel goes off harder and launches when another blast sets it off; without `drum` the kind is rolled), `{ kind = \"oil\" }` (an oil trail cell: not solid, a fuse on the ground - a blast or a burning neighbour lights it and the fire runs along it, setting off any drum it reaches), `{ kind = \"tree\" }` / `{ kind = \"pine\" }` (destructible trees, solid like a prop but drawn larger than their cell; they often catch fire when killed and a tank can flatten one by driving into it), `{ kind = \"tall_grass\" }` (not solid - cover a tank hides in, enemies cannot shoot what is standing in it), `{ kind = \"road\" }`, `{ kind = \"frog\" }` (one), `{ kind = \"start\" }` (player 1, one), `{ kind = \"start2\" }` (player 2 in a two-player round, one, optional - placed beside player 1 when absent), `{ kind = \"pickup\", pickup = \"health\"|\"ammo\"|\"laser\"|\"minigun\"|\"plasma\"|\"speedup\"|\"shield\" }`. Iron is indestructible, the rest can be shot away. Border walls and enemy spawns are added by the game on top.",
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
        schema: r#"{"type":"object","properties":{"scale":{"type":"number","default":0.5,"minimum":0.1,"maximum":1},"source":{"type":"string","enum":["screen","scene"],"default":"screen"},"overlays":{"type":"object","properties":{"nav_grid":{"type":"boolean"},"ai":{"type":"boolean"},"projectiles":{"type":"boolean"},"engage":{"type":"boolean"},"pickups":{"type":"boolean"},"inspect":{"type":"boolean"},"orders":{"type":"boolean"}}}}}"#,
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
        description: "Add an enemy at (x, y) with an optional chassis row (0-11) and AI role (player - fights the player, the default; hunter - drives at and shoots the player's frog; guard - stays leashed to the enemy frog). Draws from the round RNG, so the round stops being the seeded replay afterwards. Returns the new slot.",
        schema: r#"{"type":"object","properties":{"x":{"type":"number"},"y":{"type":"number"},"row":{"type":"integer","minimum":0,"maximum":11},"role":{"type":"string","enum":["player","hunter","guard"]}},"required":["x","y"]}"#,
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
    // ----- the two modes and the map builder (docs/game-editor-fusion.md section 11) -----
    ToolSpec {
        name: "mode",
        description: "Which mode the window is in - play (the round) or build (the map builder in the same window) - plus whether the leave-round dialog is open and the builder's state: dirty (edited since it was loaded), map name, active tool and its category, undo/redo depth. Cheap; `status` carries `mode` too.",
        schema: NO_PARAMS,
    },
    ToolSpec {
        name: "build",
        description: "Press BUILD in play mode: opens the 'Leave this round?' dialog while a round is in progress (the round is frozen until it is answered) and switches to the builder at once on the end screen. `answer: leave` confirms an open dialog and enters build mode; `answer: stay` closes it and keeps playing. A no-op in build mode. Replies like `mode`.",
        schema: r#"{"type":"object","properties":{"answer":{"type":"string","enum":["leave","stay"],"description":"Answer the open leave dialog instead of pressing the button"}}}"#,
    },
    ToolSpec {
        name: "players",
        description: "The players button in play mode (docs/two-players.md). Without `count`: press it - opens the 'How many players?' dialog (the round is frozen until it is answered) or closes an open one. With `count` (1 or 2): answer it - a different count restarts the round at once in that mode, frozen in lockstep like `restart`; the current count just closes the dialog. The count sticks for the session (later `restart`s, PLAY from the builder). Two players: slot 1 is player 2 and enemies count from 2; `step`/`input` take `p2_*` fields for it; `tap` is refused. Replies like `mode`.",
        schema: r#"{"type":"object","properties":{"count":{"type":"integer","minimum":1,"maximum":2,"description":"Answer the dialog with this player count"}}}"#,
    },
    ToolSpec {
        name: "play",
        description: "Press PLAY in build mode: the builder's (edited) map becomes the round's map and a fresh round starts, frozen in lockstep like `restart` (so `step` counts play frames; `resume` for real time). A seed pinned by an earlier `restart` stays pinned. `intro: true` starts the round frozen behind the mission banner. Fails in play mode - `restart` starts a round there. Replies with `status`.",
        schema: r#"{"type":"object","properties":{"intro":{"type":"boolean","default":false}}}"#,
    },
    ToolSpec {
        name: "builder_tool",
        description: "Select the builder's brush by name - brick, iron, wood, glass (WALL); sandbag, barrel, oil_drum, fuel_drum, fence, tree, pine (PROP); road, tall_grass, oil_trail, gate (GROUND); start, start2 (player 2's start), frog, enemy_frog (ACTOR); health, ammo, laser, minigun, plasma, speedup, shield (PICKUP); or eraser - through the category's own selection path, so the bar's category button updates as well. Without `tool`, only reports the active tool and every category's current tool and list.",
        schema: r#"{"type":"object","properties":{"tool":{"type":"string","description":"A tool name (see the description) or eraser"}}}"#,
    },
    ToolSpec {
        name: "builder_paint",
        description: "One stroke on the builder's canvas: a press on cells[0], a drag through the rest, a release - so the toggle-erase rule (a press on a cell that already holds exactly the brush's object erases it, and paint-or-erase is decided on the first cell for the whole stroke), singleton moves (start/start2/frog/enemy_frog) and one-undo-step-per-stroke apply exactly as for a mouse. Cells are [col, row] on the 32 px grid (40 x 23 at 1280x720, from the top-left). `tool` selects a brush first (see `builder_tool`); `button: right` erases whatever the brush. Replies with every changed cell's object before and after (in the map's own shape, null = empty) and the undo depth.",
        schema: r#"{"type":"object","properties":{"cells":{"type":"array","items":{"type":"array","items":{"type":"integer"},"minItems":2,"maxItems":2},"minItems":1,"description":"[[col, row], ...] in stroke order"},"tool":{"type":"string"},"button":{"type":"string","enum":["left","right"],"default":"left"}},"required":["cells"]}"#,
    },
    ToolSpec {
        name: "builder_undo",
        description: "Undo the builder's last `steps` edits (default 1; a whole stroke, one settings field, a load or a reset is one step each). Replies with how many were undone, the depth left on each side and the cells the last undone step changed.",
        schema: r#"{"type":"object","properties":{"steps":{"type":"integer","minimum":1,"default":1}}}"#,
    },
    ToolSpec {
        name: "builder_redo",
        description: "Redo the builder's last `steps` undone edits (default 1). Replies with how many were redone, the depth left on each side and the cells the last redone step changed.",
        schema: r#"{"type":"object","properties":{"steps":{"type":"integer","minimum":1,"default":1}}}"#,
    },
    ToolSpec {
        name: "builder_settings",
        description: "The builder's MAP settings - the map file's own level keys: tanks (enemy count 0-31), tank (player 1's chassis name), tank2 (player 2's, two-player rounds), mission (protect|hunt|destroy), spawn (band|waves), waves (1-20), wave_size (1-31), wave_growth (0-10), tier_start/tier_end (light|medium|heavy|super). A field left out is untouched; a field set to null goes back to auto (unset: the game's own roll or the `waves` tuning group; mission/spawn back to protect/band). Each changed field is one undo step, in the order listed. `reset: true` then reverts cells and settings to the baseline (one undoable step). Replies with the current values (null = auto) and `cli_overrides`: which of them a command-line flag (-e, --tank, --mission, ...) or an earlier `restart` parameter overrides at PLAY, so the map's value is not what the round will use.",
        schema: r#"{"type":"object","properties":{"tanks":{"type":["integer","null"],"minimum":0,"maximum":31},"tank":{"type":["string","null"],"description":"A chassis name, e.g. titan"},"tank2":{"type":["string","null"],"description":"Player 2's chassis name"},"mission":{"type":["string","null"],"enum":["protect","hunt","destroy",null]},"spawn":{"type":["string","null"],"enum":["band","waves",null]},"waves":{"type":["integer","null"],"minimum":1,"maximum":20},"wave_size":{"type":["integer","null"],"minimum":1,"maximum":31},"wave_growth":{"type":["integer","null"],"minimum":0,"maximum":10},"tier_start":{"type":["string","null"],"enum":["light","medium","heavy","super",null]},"tier_end":{"type":["string","null"],"enum":["light","medium","heavy","super",null]},"reset":{"type":"boolean","default":false,"description":"Revert cells and settings to the baseline"}}}"#,
    },
    ToolSpec {
        name: "builder_map",
        description: "Without parameters: the builder's map as TOML (`toml`), its name, `dirty` and `diff` - cells added/removed/changed and the settings fields that differ from the baseline (the map as loaded). With `name` (a Load-list name from `builder_files`), `map` (a path under maps/) or `map_toml` (inline TOML, the `map_get` format): loads that map into the canvas as one undo step and makes it the new baseline - the FILE > LOAD path; the round keeps its map until `play`. `map_get` keeps answering with the map the current round was built from, which differs from this once the builder is dirty.",
        schema: r#"{"type":"object","properties":{"name":{"type":"string","description":"A name from builder_files"},"map_toml":{"type":"string","description":"Map TOML text to load into the builder"},"map":{"type":"string","description":"Path to a map .toml, relative to the game's working directory"}}}"#,
    },
    ToolSpec {
        name: "builder_files",
        description: "What FILE > LOAD offers: every map the builder can load by name - the files under maps/ on native (`on_disk`) plus the maps shipped inside the binary (default, hunt-basic, waves-basic; all the web build has) - and `can_save`, whether this build writes maps to disk (native yes, web no: web edits live in memory for the session).",
        schema: r#"{"type":"object","properties":{}}"#,
    },
    ToolSpec {
        name: "builder_save",
        description: "FILE > SAVE / SAVE AS: write the builder's map to maps/<name>.toml (native only; `name` defaults to the map's own name and becomes it) and make the saved state the baseline, so `dirty` clears. Letters, digits, - and _ only. Replies like `builder_map`.",
        schema: r#"{"type":"object","properties":{"name":{"type":"string"}}}"#,
    },
    ToolSpec {
        name: "click",
        description: "A raw press at a window position (pixels, the 32 px HUD bar included: the field starts at y = 32), in either mode, on the same hit-tests a mouse or a finger uses: in play mode the BUILD button (right end of the bar), the players button beside it, either dialog's buttons (a press outside a dialog closes it) - a press on the field itself does nothing in play mode; in build mode the bar's buttons (PLAY starts the round like `play`), a dropdown row, a settings stepper or a field cell. With `drag_to`, a press, a straight drag to that point and a release, crossing every cell on the way. Replies like `mode`. This tests the UI; `build`/`play`/`builder_*` address the model directly.",
        schema: r#"{"type":"object","properties":{"x":{"type":"number"},"y":{"type":"number"},"button":{"type":"string","enum":["left","right"],"default":"left"},"drag_to":{"type":"array","items":{"type":"number"},"minItems":2,"maxItems":2,"description":"[x, y] to drag to before releasing"}},"required":["x","y"]}"#,
    },
    ToolSpec {
        name: "key",
        description: "Press one key for one frame: tab (BUILD/PLAY - in play mode it opens the leave dialog, or closes an open one; in build mode it starts the round like `play`), escape (keep playing / close a dialog or popup), enter (leave the round; in the players dialog, switch to the other count; confirm a popup), 1 / 2 (answer the players dialog), undo, redo (Ctrl+Z / Ctrl+Y in the builder), backspace; `text` types characters into an open builder prompt. Replies like `mode`.",
        schema: r#"{"type":"object","properties":{"key":{"type":"string","enum":["tab","escape","enter","undo","redo","backspace","1","2"]},"text":{"type":"string","description":"Characters to type this frame (build mode)"}}}"#,
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
            injected2: None,
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
    pub fn before_frame(&mut self, session: &mut Session, width: f32, height: f32) {
        // The AI-decision events exist for this server's `events` feed.
        session.game.trace_ai = true;
        while let Ok(req) = self.rx.try_recv() {
            self.dispatch(session, req, width, height);
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
        if let Some((intent, left)) = self.injected2 {
            self.injected2 = (left > 1).then_some((intent, left - 1));
            input.player2_intent = intent;
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
                let mut player2_intent = step.intent2.unwrap_or(input.player2_intent);
                if let Some(n) = step.fire_every {
                    player_intent.fire = player_intent.fire && i % n == 0;
                    player2_intent.fire = player2_intent.fire && i % n == 0;
                }
                let before = game.frame();
                game.update(Input { player_intent, player2_intent, ..Input::default() }, PHYSICS_FIXED_DT, width, height);
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

    fn status(&self, session: &Session, width: f32, height: f32) -> Value {
        let game = &session.game;
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
            "mode": session.mode().name(),
            "dialog_open": session.dialog,
            "players_dialog_open": session.players_dialog,
            "builder": { "dirty": session.builder.dirty(), "tool": session.builder.tool().name() },
            "events_kept": self.events.len(),
            "next_event_seq": self.next_seq,
            "history_frames": self.history.len(),
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
        // The tools that work on the whole session - the mode switch and
        // the builder - before the ones that only see the round.
        if let Some(result) = self.dispatch_session(session, &method, &params, width, height) {
            let _ = reply.send(result);
            return;
        }
        let game = &mut session.game;
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

    fn restart(&mut self, session: &mut Session, params: &Value, width: f32, height: f32) -> Result<Value, String> {
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
        if let Some(row) = params.get("tank_row") {
            game.player_row_override = Some(row.as_i64().ok_or("tank_row must be an integer")? as i32);
        }
        if let Some(row) = params.get("tank2_row") {
            game.player2_row_override = Some(row.as_i64().ok_or("tank2_row must be an integer")? as i32);
        }
        if let Some(n) = params.get("players") {
            game.players = n
                .as_u64()
                .and_then(|n| PlayerCount::from_count(n as usize))
                .ok_or_else(|| format!("players must be 1 or 2, got {n}"))?;
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
            session.replace_map(map, width, height);
        }
        session.driver = Driver::Play;
        session.dialog = false;
        session.players_dialog = false;
        session.game.init(width, height);
        self.round_started(session);
        Ok(self.status(session, width, height))
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
        let layout = Layout::for_field(width, height);
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
            "restart" => self.restart(session, params, width, height),
            "mode" => Ok(mode_json(session)),
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
                        session.answer_players(count, width, height);
                        if session.game.players != before {
                            self.round_started(session);
                        }
                        Ok(mode_json(session))
                    }
                    None => Err(format!("count must be 1 or 2, got {v}")),
                },
            },
            "play" => {
                if session.mode() == Driver::Play {
                    Err("already in play mode: `restart` starts a fresh round here, `build` enters the builder".to_string())
                } else {
                    session.game.show_intro = params.get("intro").and_then(Value::as_bool).unwrap_or(false);
                    session.play(width, height);
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
                    let changes = session.builder.stroke(&cells, right, width, height);
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
                    let step = if undo { session.builder.undo(width, height) } else { session.builder.redo(width, height) };
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
            "builder_settings" => builder_settings(session, params, width, height),
            "builder_map" => {
                let by_name = match params.get("name") {
                    None | Some(Value::Null) => Ok(None),
                    Some(Value::String(n)) => Ok(Some(n.clone())),
                    Some(other) => Err(format!("name must be a string, got {other}")),
                };
                by_name.and_then(|name| match name {
                    Some(name) => {
                        if params.get("map").is_some() || params.get("map_toml").is_some() {
                            return Err("give one of name, map or map_toml".to_string());
                        }
                        session.builder.load_named(&name, width, height)?;
                        builder_map_json(&session.builder)
                    }
                    None => map_param(params).and_then(|map| {
                        if let Some(map) = map {
                            session.builder.load(map, width, height);
                        }
                        builder_map_json(&session.builder)
                    }),
                })
            }
            "builder_files" => Ok(json!({
                "maps": crate::map::available_maps(),
                "can_save": crate::map::saving_available(),
            })),
            "builder_save" => {
                let name = params.get("name").and_then(Value::as_str);
                session.builder.save(name).and_then(|_| builder_map_json(&session.builder))
            }
            "click" => self.click(session, params, &layout),
            "key" => self.key(session, params, &layout),
            _ => return None,
        };
        Some(result)
    }

    /// `click`: one press (and optionally a drag) at a window position,
    /// through the same hit-tests `main.rs` runs on the mouse.
    fn click(&mut self, session: &mut Session, params: &Value, layout: &Layout) -> Result<Value, String> {
        let (Some(x), Some(y)) = (f32_param(params, "x"), f32_param(params, "y")) else {
            return Err("click needs numeric x and y (window pixels, the bar included)".to_string());
        };
        let right = button_param(params)?;
        let drag_to = match params.get("drag_to") {
            None | Some(Value::Null) => None,
            Some(v) => match v.as_array().map(Vec::as_slice) {
                Some([dx, dy]) => match (dx.as_f64(), dy.as_f64()) {
                    (Some(dx), Some(dy)) => Some(Position::new(dx as f32, dy as f32)),
                    _ => return Err(format!("drag_to must be [x, y] numbers, got {v}")),
                },
                _ => return Err(format!("drag_to must be [x, y], got {v}")),
            },
        };
        let point = Position::new(x, y);
        match session.mode() {
            Driver::Play => {
                // The same order as `main.rs`: an open dialog eats every
                // press while it is up, then the two bar buttons.
                if session.players_dialog {
                    let rects = players_dialog_rects(layout.field);
                    let p = layout.to_field(point);
                    let before = session.game.players;
                    if rects.one.check_collision_point_rec(p) {
                        session.answer_players(PlayerCount::One, layout.field.w, layout.field.h);
                    } else if rects.two.check_collision_point_rec(p) {
                        session.answer_players(PlayerCount::Two, layout.field.w, layout.field.h);
                    } else if !rects.panel.check_collision_point_rec(p) {
                        session.close_players_dialog();
                    }
                    if session.game.players != before {
                        self.round_started(session);
                    }
                } else if session.dialog {
                    let rects = leave_dialog_rects(layout.field);
                    let p = layout.to_field(point);
                    if rects.leave.check_collision_point_rec(p) {
                        session.answer_dialog(true);
                    } else if rects.stay.check_collision_point_rec(p) || !rects.panel.check_collision_point_rec(p) {
                        session.answer_dialog(false);
                    }
                } else if mode_button_rect(layout.panel).check_collision_point_rec(point) {
                    session.press_build();
                } else if players_button_rect(layout.panel).check_collision_point_rec(point) {
                    session.press_players();
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
                session.update_builder(&press, layout);
                let mut last = point;
                if let Some(to) = drag_to {
                    let steps = (point.distance_to(to) / CLICK_DRAG_STEP_PX).ceil().max(1.0) as usize;
                    for i in 1..=steps {
                        let t = i as f32 / steps as f32;
                        last = Position::new(point.x + (to.x - point.x) * t, point.y + (to.y - point.y) * t);
                        let held = BuilderInput { pointer: Some(last), held: !right, right_held: right, ..BuilderInput::default() };
                        session.update_builder(&held, layout);
                    }
                }
                session.update_builder(&BuilderInput { pointer: Some(last), ..BuilderInput::default() }, layout);
                // The press may have been PLAY.
                if session.mode() == Driver::Play {
                    self.round_started(session);
                }
            }
        }
        Ok(mode_json(session))
    }

    /// `key`: one key for one frame, or typed text, through the same
    /// paths `main.rs` takes for the keyboard.
    fn key(&mut self, session: &mut Session, params: &Value, layout: &Layout) -> Result<Value, String> {
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
            Driver::Play if session.players_dialog => {
                let (w, h) = (layout.field.w, layout.field.h);
                let before = session.game.players;
                match key {
                    Some("1") => {
                        session.answer_players(PlayerCount::One, w, h);
                    }
                    Some("2") => {
                        session.answer_players(PlayerCount::Two, w, h);
                    }
                    Some("enter") => {
                        let other = match before {
                            PlayerCount::One => PlayerCount::Two,
                            PlayerCount::Two => PlayerCount::One,
                        };
                        session.answer_players(other, w, h);
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
                Some("escape") => {
                    session.answer_dialog(false);
                }
                Some("enter") => {
                    session.answer_dialog(true);
                }
                // undo/redo/backspace, 1/2 and typed text mean nothing in play.
                _ => {}
            },
            Driver::Build => {
                if key == Some("tab") {
                    session.toggle(layout.field.w, layout.field.h);
                    if session.mode() == Driver::Play {
                        self.round_started(session);
                    }
                } else {
                    let input = BuilderInput {
                        escape: key == Some("escape"),
                        enter: key == Some("enter"),
                        backspace: key == Some("backspace"),
                        undo: key == Some("undo"),
                        redo: key == Some("redo"),
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

/// `mode`'s reply: the session's mode and the builder's state in one look.
fn mode_json(session: &Session) -> Value {
    let b = &session.builder;
    json!({
        "mode": session.mode().name(),
        "dialog_open": session.dialog,
        "players_dialog_open": session.players_dialog,
        "players": session.game.players.count(),
        "dirty": b.dirty(),
        "map_name": b.name(),
        "tool": b.tool().name(),
        "category": b.active_category().map(category_name),
        "open_menu": b.open_menu(),
        "undo_depth": b.history().undo_depth(),
        "redo_depth": b.history().redo_depth(),
    })
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
fn builder_settings(session: &mut Session, params: &Value, width: f32, height: f32) -> Result<Value, String> {
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
    }
    if params.get("reset").and_then(Value::as_bool).unwrap_or(false) {
        b.reset(width, height);
    }
    Ok(settings_json(session))
}

/// The builder's settings as the tools spell them (null = auto) plus
/// which of them the round would override (`Game`'s CLI/restart values
/// outrank the map's, exactly as they outrank a file's).
fn settings_json(session: &Session) -> Value {
    let s = session.builder.settings();
    let g = &session.game;
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
        Session::new(game, W, H)
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
        assert_eq!(status["players"], 1);
        assert_eq!(status["players_dialog_open"], false);
    }

    /// The players tool and the button/keys behind it: the dialog freezes
    /// the round, a new count restarts in that mode with player 2 in slot
    /// 1, `step` drives player 2 through `p2_*`, and `restart {players: 1}`
    /// goes back.
    #[test]
    fn players_tool_switches_mode_and_the_button_opens_the_dialog() {
        let (mut server, tx) = DevServer::headless();
        let mut s = game(41);
        let layout = Layout::for_field(W, H);
        // The button opens it and the round freezes; a press outside closes it.
        let m = ask(&mut server, &tx, &mut s, "players", json!({})).unwrap();
        assert_eq!(m["players_dialog_open"], true, "{m}");
        assert!(!s.playing());
        let m = ask(&mut server, &tx, &mut s, "click", json!({ "x": 10.0, "y": 100.0 })).unwrap();
        assert_eq!(m["players_dialog_open"], false, "{m}");
        let b = players_button_rect(layout.panel);
        let m = ask(&mut server, &tx, &mut s, "click", json!({ "x": b.x + b.width / 2.0, "y": b.y + b.height / 2.0 })).unwrap();
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
        assert!(s.game.player2.is_some());
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
            server.advance(&mut s.game, Input::default(), 0.016, W, H);
            rx.recv().unwrap().unwrap();
        }
        ask(&mut server, &tx, &mut s, "players", json!({ "count": 2 })).unwrap();
        assert_eq!(s.game.frame(), 3);
        // `p2_*` drives player 2 and nobody else.
        let before: Vec<_> = s.game.tank_snapshots().into_iter().filter(|t| t.is_player).map(|t| (t.player, t.position)).collect();
        let rx = call(&tx, "step", json!({ "frames": 30, "p2_move_dir": "down", "snapshot": false }));
        server.before_frame(&mut s, W, H);
        server.advance(&mut s.game, Input::default(), 0.016, W, H);
        rx.recv().unwrap().unwrap();
        let after: Vec<_> = s.game.tank_snapshots().into_iter().filter(|t| t.is_player).map(|t| (t.player, t.position)).collect();
        assert!((after[0].1.y - before[0].1.y).abs() < 1.0, "player 1 stayed put");
        assert!(after[1].1.y > before[1].1.y + 20.0, "player 2 drove down");
        // The keys answer the dialog; Enter means the other count.
        ask(&mut server, &tx, &mut s, "players", json!({})).unwrap();
        let m = ask(&mut server, &tx, &mut s, "key", json!({ "key": "enter" })).unwrap();
        assert_eq!(m["players"], 1, "{m}");
        assert!(s.game.player2.is_none());
        ask(&mut server, &tx, &mut s, "players", json!({})).unwrap();
        let m = ask(&mut server, &tx, &mut s, "key", json!({ "key": "2" })).unwrap();
        assert_eq!(m["players"], 2, "{m}");
        assert!(ask(&mut server, &tx, &mut s, "players", json!({ "count": 3 })).is_err());
        // restart takes the count too, and clears an open dialog.
        ask(&mut server, &tx, &mut s, "players", json!({})).unwrap();
        let st = ask(&mut server, &tx, &mut s, "restart", json!({ "players": 1, "seed": 3 })).unwrap();
        assert_eq!(st["players"], 1, "{st}");
        assert_eq!(st["players_dialog_open"], false);
        assert!(s.game.player2.is_none());
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
        server.advance(&mut game, Input::default(), 0.016, W, H);
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

    // ----- the two modes and the builder -----

    /// One request, answered at the next frame boundary.
    fn ask(server: &mut DevServer, tx: &mpsc::Sender<Request>, session: &mut Session, method: &str, params: Value) -> Result<Value, String> {
        let rx = call(tx, method, params);
        server.before_frame(session, W, H);
        rx.recv().unwrap()
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
            ("overlays", json!({ "inspect": true })),
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
        assert_eq!(cats[4]["tools"].as_array().unwrap().len(), 7, "{}", cats[4]);
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
        server.advance(&mut s.game, Input::default(), 0.016, W, H);
        assert_eq!(step.recv().unwrap().unwrap()["frame"], 2);
        // The edit survives the round trip back into the builder.
        enter_build(&mut server, &tx, &mut s);
        assert_eq!(s.builder.map().cell(10, 5), Some(&crate::map::CellObject::Wall { material: crate::obstacle::Material::Iron }));
        let status = ask(&mut server, &tx, &mut s, "play", json!({ "intro": true })).unwrap();
        assert!(status["intro_seconds_left"].as_f64().unwrap() > 0.0, "{status}");
    }

    #[test]
    fn click_and_key_take_the_same_paths_as_the_mouse_and_keyboard() {
        let (mut server, tx) = DevServer::headless();
        let mut s = game(36);
        ask(&mut server, &tx, &mut s, "restart", json!({ "map_toml": INLINE_MAP, "seed": 1 })).unwrap();
        let layout = Layout::for_field(W, H);
        let button = mode_button_rect(layout.panel);
        let centre = |r: sola_raylib::prelude::Rectangle| (r.x + r.width / 2.0, r.y + r.height / 2.0);
        // A click on BUILD opens the dialog like `build`.
        let (bx, by) = centre(button);
        let m = ask(&mut server, &tx, &mut s, "click", json!({ "x": bx, "y": by })).unwrap();
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
        // The dialog's own buttons.
        ask(&mut server, &tx, &mut s, "key", json!({ "key": "tab" })).unwrap();
        let rects = leave_dialog_rects(layout.field);
        let (sx, sy) = centre(rects.stay);
        let m = ask(&mut server, &tx, &mut s, "click", json!({ "x": sx, "y": sy + layout.field.y })).unwrap();
        assert_eq!(m["dialog_open"], false, "{m}");
        assert_eq!(m["mode"], "play");
        ask(&mut server, &tx, &mut s, "key", json!({ "key": "tab" })).unwrap();
        let (lx, ly) = centre(rects.leave);
        let m = ask(&mut server, &tx, &mut s, "click", json!({ "x": lx, "y": ly + layout.field.y })).unwrap();
        assert_eq!(m["mode"], "build", "{m}");

        // Build mode: a click on a field cell paints with the active brush,
        // a drag crosses every cell, undo is a key.
        ask(&mut server, &tx, &mut s, "builder_tool", json!({ "tool": "iron" })).unwrap();
        // A cell's world position is its centre (`map::cell_to_world`).
        let cell_centre = |c: i32, r: i32| (c as f32 * 32.0, layout.field.y + r as f32 * 32.0);
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
        // PLAY from the bar starts the round frozen, like `play`.
        server.lockstep = false;
        let (px, py) = centre(button);
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
}
