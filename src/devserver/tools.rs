//! The dev tools' tables: every tool the game's dev server (`TOOLS`) and
//! the room server's (`ROOM_TOOLS`) answer, with the parameter struct each
//! one takes (docs/dev-server-design.md section 4).
//!
//! A tool is one [`ToolSpec`] row. Its input schema is derived from its
//! parameter struct ([`Params::of`]), and the same struct is what a call's
//! parameters are checked against before the tool runs ([`ToolSpec::check`]):
//! a key the struct does not have is refused by name, and a value of the
//! wrong type is refused with the field it belongs to. The row also says
//! where the tool may run (`in_build`, `online`) and how the adapter
//! presents its reply (`present`), so nothing about a tool is kept in a
//! list apart from its row.
//!
//! `description` is the line or two an MCP client lists; `help` is the
//! long form - reply shapes, edge cases - which the `help` tool serves on
//! demand, so listing the tools stays cheap.

use std::collections::BTreeMap;

use schemars::JsonSchema;
use serde::Deserialize;
use serde::de::DeserializeOwned;
use serde_json::{Value, json};

use crate::simulation::debug::TankPatch;

/// One tool: its wire/MCP name, what a client reads about it, the
/// parameters it takes, and where it may run.
pub struct ToolSpec {
    pub name: &'static str,
    /// What the tool is for, in a line or two: the MCP `description`.
    pub description: &'static str,
    /// The long form: the reply's shape and the edge cases. `help {tool}`
    /// returns it; the MCP tool list does not carry it.
    pub help: &'static str,
    /// The parameter struct: the input schema and the check.
    pub params: Params,
    /// Reads without changing the round, the builder, tuning or lockstep
    /// (MCP `readOnlyHint`). `screenshot` is not: it writes a file and
    /// takes an overlay patch.
    pub read_only: bool,
    /// Throws something away a client may want (MCP `destructiveHint`):
    /// the round (`restart`, `play`, a `players` count change, `kill`),
    /// every tuning edit (`tuning_reset`) or a map file (`builder_save`).
    /// Emitted explicitly because the MCP default is *true*.
    pub destructive: bool,
    /// Runs while the builder is live. A tool that reads or drives the
    /// round does not (docs/game-editor-fusion.md section 11): it would
    /// touch a round the builder has frozen.
    pub in_build: bool,
    /// Runs while the window holds a seat in a room
    /// (docs/online-coop-prd.md section 4.5). A tool that drives the local
    /// round or the builder does not: an online round is the server's to
    /// simulate and the replica on screen is a picture of it, so a write
    /// here would change the picture and reach nobody. The reading tools
    /// describe the online round instead (`Session::shown`).
    pub online: bool,
    /// How the adapter hands the reply to an MCP client.
    pub present: Present,
}

/// How a tool's reply becomes MCP content.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Present {
    /// The reply as compact JSON text, and as `structuredContent`.
    Json,
    /// One string field of the reply as bare text (`nav_grid`'s grid).
    Text(&'static str),
    /// The reply's `png_base64` as an image block, with a caption.
    Image,
}

/// A tool's parameter struct, as the two things the table needs of it.
pub struct Params {
    schema: fn() -> Value,
    parse: fn(&Value) -> Result<(), String>,
}

impl Params {
    /// The parameters `P` describes.
    pub const fn of<P: DeserializeOwned + JsonSchema>() -> Params {
        Params { schema: schema_of::<P>, parse: parses_as::<P> }
    }
}

/// `P`'s JSON schema as an MCP `inputSchema`: an `object` schema with every
/// nested type inlined (no `$defs`), and without the generator's own
/// `$schema` and `title`.
fn schema_of<P: JsonSchema>() -> Value {
    let settings = schemars::generate::SchemaSettings::draft2020_12().with(|s| s.inline_subschemas = true);
    let mut schema = settings.into_generator().into_root_schema_for::<P>().to_value();
    if let Some(object) = schema.as_object_mut() {
        object.remove("$schema");
        object.remove("title");
        object.entry("properties").or_insert_with(|| json!({}));
    }
    schema
}

/// Whether `params` deserialise as `P`. serde's error says what was wrong
/// but not where, so the parameter at fault is found and named: the one
/// without which the rest parses, or is all that is missing.
fn parses_as<P: DeserializeOwned>(params: &Value) -> Result<(), String> {
    let Err(error) = P::deserialize(params) else { return Ok(()) };
    if let Some(object) = params.as_object() {
        for key in object.keys() {
            let mut rest = object.clone();
            rest.remove(key);
            let without = P::deserialize(Value::Object(rest)).map(|_| ()).map_err(|e| e.to_string());
            if without.is_ok() || without.is_err_and(|e| e == format!("missing field `{key}`")) {
                return Err(format!("{key}: {error}"));
            }
        }
    }
    Err(error.to_string())
}

/// `params` as the parameter struct `P`. A tool's handler reads its
/// parameters through this, after `ToolSpec::check` has passed them.
pub fn parse<P: DeserializeOwned>(params: &Value) -> Result<P, String> {
    P::deserialize(params).map_err(|e| e.to_string())
}

impl ToolSpec {
    /// The tool's MCP `inputSchema`.
    pub fn schema(&self) -> Value {
        (self.params.schema)()
    }

    /// The MCP tool annotations the adapter advertises for this tool.
    pub fn annotations(&self) -> Value {
        json!({
            "readOnlyHint": self.read_only,
            "destructiveHint": self.destructive,
            "idempotentHint": self.read_only,
            "openWorldHint": false,
        })
    }

    /// Whether `params` are ones this tool takes: an object (or nothing),
    /// no key its parameter struct lacks, and every value of its field's
    /// type. The error names the tool, the offending key and the keys it
    /// does take, so a misspelt parameter is never a silent no-op.
    pub fn check(&self, params: &Value) -> Result<(), String> {
        let name = self.name;
        let object = match params {
            Value::Null => return (self.params.parse)(&json!({})).map_err(|e| format!("{name}: {e}")),
            Value::Object(object) => object,
            other => return Err(format!("{name}: params must be an object, got {other}")),
        };
        let schema = self.schema();
        let takes = schema.get("properties").and_then(Value::as_object);
        if let Some(takes) = takes
            && let Some(unknown) = object.keys().find(|k| !takes.contains_key(*k))
        {
            let known: Vec<&str> = takes.keys().map(String::as_str).collect();
            return Err(if known.is_empty() {
                format!("{name}: unknown parameter {unknown:?}; it takes none")
            } else {
                format!("{name}: unknown parameter {unknown:?}; it takes {}", known.join(", "))
            });
        }
        (self.params.parse)(params).map_err(|e| format!("{name}: {e}"))
    }
}

/// The row called `name` in `tools`.
pub fn find<'a>(tools: &'a [ToolSpec], name: &str) -> Option<&'a ToolSpec> {
    tools.iter().find(|t| t.name == name)
}

/// The `help` tool, which the adapter answers itself for either table: a
/// tool's long form, or with no `tool` every tool's name and description.
pub const HELP: ToolSpec = ToolSpec {
    name: "help",
    description: "A tool's full documentation - its reply's shape, its edge cases - or, with no `tool`, the list of tools. The tool list carries only a summary of each; read this before relying on a reply field.",
    help: "With `tool`: {tool, description, help, input_schema, read_only, destructive, in_build, online}. Without: {tools: [{name, description}]}. Answered by the adapter, so it works with nothing running.",
    params: Params::of::<HelpParams>(),
    read_only: true,
    destructive: false,
    in_build: true,
    online: true,
    present: Present::Json,
};

/// `attach`: which game the game tools drive. The adapter's own.
pub const ATTACH: ToolSpec = ToolSpec {
    name: "attach",
    description: "Which game the tools drive: a window (`just run-dev`) or the adapter's own headless round. With no parameters it reports the target; `target` switches it.",
    help: "By default the adapter drives a window only when one started from this checkout is listening on the dev port (its `mode.host.cwd` is the adapter's working directory); otherwise it hosts a headless round in-process - a private, deterministic round with no window and no port, which moves only by `step` and whose `screenshot` is the field painted on the CPU. `attach {target: \"window\"}` drives the window on `port` (default the dev port) whichever checkout started it - say so only when that window is yours to drive; `attach {target: \"headless\"}` goes back to the adapter's own round (a fresh one if none is held). Reply: {target: headless|window, port, host: the target's `mode.host`, window: what listens on the dev port ({cwd, pid, same_checkout} or null), adapter: {exe, cwd, stale}}. `adapter.stale` is true when the source is newer than the adapter's binary, in which case the headless round runs old code: call `reload`.",
    params: Params::of::<AttachParams>(),
    read_only: false,
    destructive: false,
    in_build: true,
    online: true,
    present: Present::Json,
};

/// `batch`: several tools in one call. The adapter's own.
pub const BATCH: ToolSpec = ToolSpec {
    name: "batch",
    description: "Run several tools in order in one call - a whole scenario: restart, set up, step, read - and get every reply. Stops at the first error unless `keep_going`.",
    help: "`calls` is [{tool, params}], run in order against the current target. Reply: {results: [{tool, result} | {tool, error}], completed, failed}. A `screenshot` inside a batch replies with its file's path, not the image. `batch` and `reload` cannot be nested. Example - a seeded round, a tank hurt, stepped until it dies: {calls: [{tool: \"restart\", params: {seed: 7, enemies: 2}}, {tool: \"set_tank\", params: {slot: 1, damage: 0.9}}, {tool: \"step\", params: {frames: 600, fire: true, fire_every: 20, until: [\"tank_destroyed\"]}}]}.",
    params: Params::of::<BatchParams>(),
    read_only: false,
    destructive: true,
    in_build: true,
    online: true,
    present: Present::Json,
};

/// `reload`: rebuild the adapter and restart it in place. The adapter's own.
pub const RELOAD: ToolSpec = ToolSpec {
    name: "reload",
    description: "Rebuild the adapter from this checkout's source and restart it in place, so the headless round and the tool list run the code as it is now. The headless round starts over.",
    help: "Runs `cargo build --no-default-features --features dev-tools --bin bbmcp` in the adapter's working directory (a minute or two after a large change), then replaces the adapter's process with the new binary and tells the client the tool list changed. A build that fails changes nothing and replies with the compiler's last lines. A window is its own process: rebuild and restart that with `just run-dev`. Reply: {reloaded, version, exe}.",
    params: Params::of::<NoParams>(),
    read_only: false,
    destructive: true,
    in_build: true,
    online: true,
    present: Present::Json,
};

/// `probe`: the headless probe as a tool. The adapter's own.
pub const PROBE: ToolSpec = ToolSpec {
    name: "probe",
    description: "Run the headless probe - many seeded rounds of one scenario - and get its ANOMALY lines: the default way to check a mechanic or an AI change across seeds.",
    help: "Runs `cargo run --no-default-features --features dev-tools --bin probe -- --scenario <scenario> <args...>` in the adapter's working directory and waits for it (`timeout_seconds`, default 300). Reply: {ok, exit_code, anomalies: the `ANOMALY round= seed= frame= kind=` lines (first 200), anomaly_count, tail: the last lines of its output, command}. Every anomaly's seed is a replay recipe: `restart {seed}` then `step`. `args` are the probe's own flags, e.g. [\"--rounds\", \"20\", \"--seed\", \"7\", \"--map\", \"maps/test/maze.toml\"]; [\"--help\"] lists them.",
    params: Params::of::<ProbeParams>(),
    read_only: true,
    destructive: false,
    in_build: true,
    online: true,
    present: Present::Json,
};

/// `help`'s reply for `params` over `tools`.
pub fn help(tools: &[&ToolSpec], params: &Value) -> Result<Value, String> {
    HELP.check(params)?;
    let asked: HelpParams = parse(params)?;
    match asked.tool {
        None => Ok(json!({
            "tools": tools.iter().map(|t| json!({ "name": t.name, "description": t.description })).collect::<Vec<_>>(),
        })),
        Some(name) => {
            let tool = tools.iter().find(|t| t.name == name).ok_or_else(|| {
                format!("help: no tool is called {name:?}; there are {}", tools.iter().map(|t| t.name).collect::<Vec<_>>().join(", "))
            })?;
            Ok(json!({
                "tool": tool.name,
                "description": tool.description,
                "help": tool.help,
                "input_schema": tool.schema(),
                "read_only": tool.read_only,
                "destructive": tool.destructive,
                "in_build": tool.in_build,
                "online": tool.online,
            }))
        }
    }
}

/// The `key` tool's key names.
pub const KEY_NAMES: &[&str] =
    &["tab", "escape", "enter", "undo", "redo", "copy", "cut", "paste", "delete", "backspace", "1", "2", "left", "right", "up", "down", "zoom_in", "zoom_out"];

/// The sections `status` adds to its core on request (`status {sections}`).
pub const STATUS_SECTIONS: &[&str] = &["camera", "ui", "builder", "frame_stages", "weather", "stats", "turns", "overlays"];

// ---------------------------------------------------------------------
// Parameter structs. A field's doc comment is its schema description.
// Names that a handler parses itself (a chassis, a direction, a tool) are
// strings here, with their values listed in the schema, so the handler's
// own error - which names the valid spellings - is the one a caller reads.
// ---------------------------------------------------------------------

/// A tool that takes no parameters.
#[derive(Deserialize, JsonSchema)]
pub struct NoParams {}

#[derive(Deserialize, JsonSchema)]
pub struct HelpParams {
    /// The tool to document; leave out for the list of tools.
    pub tool: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct AttachParams {
    /// headless (the adapter's own round) or window (a running game).
    #[schemars(extend("enum" = ["headless", "window"]))]
    pub target: Option<String>,
    /// The window's dev port (default: the adapter's, 4747 unless
    /// BONGBONG_DEV_PORT moves it).
    pub port: Option<u16>,
}

/// One call of a `batch`.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BatchCall {
    pub tool: String,
    /// The tool's parameters (default {}).
    pub params: Option<Value>,
}

#[derive(Deserialize, JsonSchema)]
pub struct BatchParams {
    /// The calls, run in order.
    pub calls: Vec<BatchCall>,
    /// Run the rest after a call fails (default false).
    pub keep_going: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ProbeParams {
    /// The scenario every round plays.
    #[schemars(extend("enum" = ["afk", "advance", "brake", "survive", "defend", "circle"]))]
    pub scenario: String,
    /// The probe's own flags, e.g. ["--rounds", "20", "--seed", "7"].
    pub args: Option<Vec<String>>,
    /// How long to wait for it (default 300).
    pub timeout_seconds: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct StatusParams {
    /// Parts to add to the core reply: camera, ui, builder, frame_stages,
    /// weather, stats, turns, overlays - or all.
    #[schemars(extend("items" = {"type": "string", "enum": ["all", "camera", "ui", "builder", "frame_stages", "weather", "stats", "turns", "overlays"]}))]
    pub sections: Option<Vec<String>>,
}

#[derive(Deserialize, JsonSchema)]
pub struct SnapshotParams {
    /// compact (default) or full, which adds each enemy's AI memory.
    #[schemars(extend("enum" = ["compact", "full"]))]
    pub detail: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct EventsParams {
    /// Return events with seq > since (default 0).
    pub since: Option<u64>,
    /// At most this many (default 200).
    pub limit: Option<u64>,
    /// Only these event names.
    pub kinds: Option<Vec<String>>,
    /// Drop these event names.
    pub exclude: Option<Vec<String>>,
}

/// The held input `step` and `input` share: player 1's, and player 2's
/// under `p2_` in a two-player round.
#[derive(Deserialize, JsonSchema)]
pub struct IntentParams {
    #[schemars(extend("enum" = ["up", "down", "left", "right"]))]
    pub move_dir: Option<String>,
    #[schemars(extend("enum" = ["up", "down", "left", "right"]))]
    pub face: Option<String>,
    pub fire: Option<bool>,
    /// Set one lantern down, on the press.
    pub lamp: Option<bool>,
    #[schemars(extend("enum" = ["up", "down", "left", "right"]))]
    pub p2_move_dir: Option<String>,
    #[schemars(extend("enum" = ["up", "down", "left", "right"]))]
    pub p2_face: Option<String>,
    pub p2_fire: Option<bool>,
    pub p2_lamp: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct StepParams {
    /// Frames to advance (default 1, at most 100000).
    #[schemars(range(min = 1, max = 100000))]
    pub frames: Option<u64>,
    #[serde(flatten)]
    pub intent: IntentParams,
    /// With fire=true: press the trigger on frames 0, N, 2N... and release
    /// in between (both players).
    #[schemars(range(min = 1))]
    pub fire_every: Option<u64>,
    /// Stop early, after the frame one of these events happens on (event
    /// names as `events` gives them); `frames` is then the most to run.
    pub until: Option<Vec<String>>,
    /// Stop early, after the frame the round is won or lost on.
    pub until_outcome: Option<bool>,
    /// Add a snapshot of the world after the step (default false).
    pub snapshot: Option<bool>,
    /// The snapshot's detail: compact (default) or full.
    #[schemars(extend("enum" = ["compact", "full"]))]
    pub detail: Option<String>,
    /// Only these event names.
    pub kinds: Option<Vec<String>>,
    /// Drop these event names.
    pub exclude: Option<Vec<String>>,
}

#[derive(Deserialize, JsonSchema)]
pub struct InputParams {
    #[serde(flatten)]
    pub intent: IntentParams,
    /// Frames to hold the input for (default 1).
    pub frames: Option<u64>,
    /// Press the I key once: the overlay cycle.
    pub cycle_overlays: Option<bool>,
}

/// A seed as a number or as text (decimal or 0x-hex).
#[derive(Deserialize, JsonSchema)]
#[serde(untagged)]
pub enum SeedParam {
    Number(u64),
    Text(String),
}

#[derive(Deserialize, JsonSchema)]
pub struct RestartParams {
    /// The round's seed: a number, or text in decimal or 0x-hex.
    pub seed: Option<SeedParam>,
    /// Band plan enemy count; 0 is a sandbox round that never ends by
    /// wreck count.
    #[schemars(range(min = 0, max = 31))]
    pub enemies: Option<u64>,
    /// Player 1's chassis by name (or tank_row).
    #[schemars(extend("enum" = ["scout", "assault", "breaker", "longbow", "flak", "wraith", "warden", "ravager", "glacier", "obelisk", "titan", "leviathan"]))]
    pub tank: Option<String>,
    #[schemars(range(min = 0, max = 11))]
    pub tank_row: Option<i64>,
    /// Seats the round holds; enemies count from the slot after them.
    #[schemars(range(min = 1, max = 8))]
    pub players: Option<u64>,
    /// Player 2's chassis by name (or tank2_row).
    #[schemars(extend("enum" = ["scout", "assault", "breaker", "longbow", "flak", "wraith", "warden", "ravager", "glacier", "obelisk", "titan", "leviathan"]))]
    pub tank2: Option<String>,
    #[schemars(range(min = 0, max = 11))]
    pub tank2_row: Option<i64>,
    /// Path to a map .toml, relative to the game's working directory.
    pub map: Option<String>,
    /// Map TOML text, e.g. `version = 1\ntanks = 4\ncells."20,8" = { kind = "wall", material = "iron" }`.
    pub map_toml: Option<String>,
    #[schemars(extend("enum" = ["protect", "hunt", "destroy"]))]
    pub mission: Option<String>,
    #[schemars(extend("enum" = ["band", "waves"]))]
    pub spawn: Option<String>,
    #[schemars(range(min = 1))]
    pub waves: Option<u32>,
    #[schemars(range(min = 1))]
    pub wave_size: Option<u32>,
    pub wave_growth: Option<u32>,
    #[schemars(extend("enum" = ["light", "medium", "heavy", "super"]))]
    pub tier_start: Option<String>,
    #[schemars(extend("enum" = ["light", "medium", "heavy", "super"]))]
    pub tier_end: Option<String>,
    /// Keep the frozen intro banner (default false, so `step` counts are
    /// play frames).
    pub intro: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct LintParams {
    /// builder = the canvas (default in build mode); round = the round's
    /// map (default in play mode).
    #[schemars(extend("enum" = ["builder", "round"]))]
    pub source: Option<String>,
    /// Lint this map file instead (a path relative to the game's working
    /// directory); it is loaded nowhere.
    pub map: Option<String>,
    /// Lint this map TOML text instead; it is loaded nowhere.
    pub map_toml: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct TerrainParams {
    /// all (default), damaged, burning or fused.
    #[schemars(extend("enum" = ["all", "damaged", "burning", "fused"]))]
    pub only: Option<String>,
    /// Only tiles of these materials (brick, iron, wood, glass, concrete,
    /// sandbag, barrel, fence, target, tree and the tree species).
    pub materials: Option<Vec<String>>,
}

#[derive(Deserialize, JsonSchema)]
pub struct HistoryParams {
    /// Only this tank's rows (aggregates still cover every tank).
    pub slot: Option<u64>,
    /// Window in frames, ending at the latest recorded one (default 600,
    /// at most 3600).
    #[schemars(range(min = 1, max = 3600))]
    pub last: Option<u64>,
    /// Row sampling stride in frames (default 10).
    #[schemars(range(min = 1))]
    pub every: Option<u64>,
}

/// The debug overlay flags, one per `simulation::Overlays` field and the
/// dev server's own `camera`; a flag left out keeps its value.
#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct OverlayFlags {
    pub nav_grid: Option<bool>,
    pub ai: Option<bool>,
    pub projectiles: Option<bool>,
    pub engage: Option<bool>,
    pub pickups: Option<bool>,
    pub hitboxes: Option<bool>,
    pub stats: Option<bool>,
    /// The follow camera's reasoning over a followed view: the dead zone,
    /// the sight boxes, the look-ahead, the committed heading, the fight's
    /// pull and the spring's goal (`follow::CameraOverlay`). Kept by the
    /// dev server, not the round, so the I key's presets leave it alone.
    pub camera: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ScreenshotParams {
    /// Output scale, 0.1 to 1 (default 0.5).
    #[schemars(range(min = 0.1, max = 1.0))]
    pub scale: Option<f64>,
    /// screen (default: the window as presented) or scene (the field's
    /// render texture).
    #[schemars(extend("enum" = ["screen", "scene"]))]
    pub source: Option<String>,
    /// Overlay flags to set before the capture.
    pub overlays: Option<OverlayFlags>,
}

#[derive(Deserialize, JsonSchema)]
pub struct CameraParams {
    /// World x at the view's centre.
    pub x: Option<f64>,
    /// World y at the view's centre.
    pub y: Option<f64>,
    /// How many times the field is magnified (at least 1).
    pub zoom: Option<f64>,
    /// Show the whole field again.
    pub reset: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct FieldParams {
    /// player (default), player2 or bong.
    #[schemars(extend("enum" = ["player", "player2", "bong"]))]
    pub target: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct SlotParams {
    /// Owner slot: 0 = player, enemies from 1 (see snapshot.tanks[].slot).
    pub slot: u64,
}

#[derive(Deserialize, JsonSchema)]
pub struct TeleportParams {
    pub slot: u64,
    pub x: f64,
    pub y: f64,
    #[schemars(extend("enum" = ["up", "down", "left", "right"]))]
    pub facing: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct SetTankParams {
    pub slot: u64,
    #[serde(flatten)]
    pub patch: TankPatch,
}

#[derive(Deserialize, JsonSchema)]
pub struct SpawnEnemyParams {
    pub x: f64,
    pub y: f64,
    /// Chassis row, 0-11.
    #[schemars(range(min = 0, max = 11))]
    pub row: Option<i64>,
    #[schemars(extend("enum" = ["player", "hunter", "guard"]))]
    pub role: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct SpawnPickupParams {
    /// A pickup's name (see `help spawn_pickup`).
    pub kind: String,
    pub x: f64,
    pub y: f64,
}

/// A point strike: `rod_call` and `well_at`.
#[derive(Deserialize, JsonSchema)]
pub struct StrikeParams {
    pub x: f64,
    pub y: f64,
    /// The enemy's side calls it (default false: player 1's).
    pub enemy: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct TuningGetParams {
    pub diff_only: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct TuningSetParams {
    /// {knob: value}; a value is a number, a boolean or an array.
    pub patch: BTreeMap<String, Value>,
}

#[derive(Deserialize, JsonSchema)]
pub struct TuningSchemaParams {
    pub group: Option<String>,
    pub name_contains: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct LangParams {
    /// A shipped language tag to switch to.
    pub tag: Option<String>,
}

/// One weather's name or a list of them.
fn skies_schema(_: &mut schemars::SchemaGenerator) -> schemars::Schema {
    schemars::json_schema!({
        "type": ["string", "array", "null"],
        "items": { "type": "string", "enum": ["clear", "night", "dusk", "rain", "storm", "fog", "sandstorm", "snow", "heat_haze", "random"] },
    })
}

#[derive(Deserialize, JsonSchema)]
pub struct WeatherParams {
    /// The sky to draw the round under, or a list of skies the round's
    /// seed picks one of; random is every sky.
    #[schemars(schema_with = "skies_schema")]
    pub name: Option<Value>,
    /// Start the round over under the new sky rather than changing it
    /// mid-round (default false).
    pub restart: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct PlayersParams {
    /// Answer the dialog with this player count.
    #[schemars(range(min = 1, max = 2))]
    pub count: Option<u64>,
}

#[derive(Deserialize, JsonSchema)]
pub struct PlayParams {
    /// Keep the frozen intro banner (default false).
    pub intro: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct BuilderToolParams {
    /// A tool name (see `help builder_tool`) or eraser.
    pub tool: Option<String>,
    /// How the brush paints.
    #[schemars(extend("enum" = ["pen", "rect", "fill", "scatter"]))]
    pub shape: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct BuilderPaintParams {
    /// [[col, row], ...] in stroke order.
    #[schemars(length(min = 1))]
    pub cells: Vec<[i64; 2]>,
    pub tool: Option<String>,
    #[schemars(extend("enum" = ["pen", "rect", "fill", "scatter"]))]
    pub shape: Option<String>,
    /// left (default) or right.
    #[schemars(extend("enum" = ["left", "right"]))]
    pub button: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct StepsParams {
    /// How many (default 1).
    #[schemars(range(min = 1))]
    pub steps: Option<u64>,
}

/// A field that tells `null` from left out: `Some(None)` and `None`.
fn nullable<'de, T: Deserialize<'de>, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Option<T>>, D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

/// `builder_settings`: a setting left out is left as it is, and `null`
/// sets it back to auto - `None` and `Some(None)` of a nullable field.
#[derive(Deserialize, JsonSchema)]
pub struct BuilderSettingsParams {
    #[schemars(range(min = 0, max = 31))]
    #[serde(default, deserialize_with = "nullable")]
    pub tanks: Option<Option<u32>>,
    /// A chassis name, e.g. titan.
    #[serde(default, deserialize_with = "nullable")]
    pub tank: Option<Option<String>>,
    /// Player 2's chassis name.
    #[serde(default, deserialize_with = "nullable")]
    pub tank2: Option<Option<String>>,
    /// The players' bong; null = spike.
    #[schemars(extend("enum" = ["spike", "oggle", "gloop", null]))]
    #[serde(default, deserialize_with = "nullable")]
    pub bong: Option<Option<String>>,
    /// The enemy's bong in a Hunt round; null = blip.
    #[schemars(extend("enum" = ["blip", "puff", "capi", "fuzz", null]))]
    #[serde(default, deserialize_with = "nullable")]
    pub enemy_bong: Option<Option<String>>,
    #[schemars(extend("enum" = ["protect", "hunt", "destroy", null]))]
    #[serde(default, deserialize_with = "nullable")]
    pub mission: Option<Option<String>>,
    #[schemars(extend("enum" = ["band", "waves", null]))]
    #[serde(default, deserialize_with = "nullable")]
    pub spawn: Option<Option<String>>,
    #[schemars(range(min = 1, max = 20))]
    #[serde(default, deserialize_with = "nullable")]
    pub waves: Option<Option<u32>>,
    #[schemars(range(min = 1, max = 31))]
    #[serde(default, deserialize_with = "nullable")]
    pub wave_size: Option<Option<u32>>,
    #[schemars(range(min = 0, max = 10))]
    #[serde(default, deserialize_with = "nullable")]
    pub wave_growth: Option<Option<u32>>,
    #[schemars(extend("enum" = ["light", "medium", "heavy", "super", null]))]
    #[serde(default, deserialize_with = "nullable")]
    pub tier_start: Option<Option<String>>,
    #[schemars(extend("enum" = ["light", "medium", "heavy", "super", null]))]
    #[serde(default, deserialize_with = "nullable")]
    pub tier_end: Option<Option<String>>,
    /// null = grass, the default.
    #[schemars(extend("enum" = ["grass", "desert", null]))]
    #[serde(default, deserialize_with = "nullable")]
    pub theme: Option<Option<String>>,
    /// The kind of place, one made for the map's theme; null = none, the
    /// plain tiles.
    #[schemars(extend("enum" = ["valley", "floodplain", "karst", null]))]
    #[serde(default, deserialize_with = "nullable")]
    pub biome: Option<Option<String>>,
    /// A weather's name or a list of them (each round's seed picks one);
    /// null or [] = clear, the default; random = every sky.
    #[schemars(schema_with = "skies_schema")]
    #[serde(default, deserialize_with = "nullable")]
    pub weather: Option<Option<Value>>,
    /// Where the old map sits when the size changes.
    #[schemars(extend("enum" = ["top_left", "top", "top_right", "left", "center", "right", "bottom_left", "bottom", "bottom_right"]))]
    pub anchor: Option<String>,
    /// [cols, rows] in cells, 16 x 9 to 250 x 250.
    pub size: Option<[f64; 2]>,
    /// Revert cells and settings to the baseline.
    pub reset: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct BuilderMapParams {
    /// A name from builder_files.
    pub name: Option<String>,
    /// Map TOML text to load into the builder.
    pub map_toml: Option<String>,
    /// Path to a map .toml, relative to the game's working directory.
    pub map: Option<String>,
    /// Empty the canvas of every placed object (FILE > CLEAR MAP).
    pub clear: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct BuilderCameraParams {
    /// World x (field px) for the middle of the canvas.
    pub x: Option<f64>,
    /// World y (field px) for the middle of the canvas.
    pub y: Option<f64>,
    /// Times FIT; 1 is the whole map.
    pub zoom: Option<f64>,
    /// Back to the whole canvas.
    pub fit: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct TouchPointParams {
    pub id: u64,
    pub x: f64,
    pub y: f64,
}

#[derive(Deserialize, JsonSchema)]
pub struct BuilderTouchParams {
    /// Frames of touch points, first to last.
    pub frames: Vec<Vec<TouchPointParams>>,
    /// Seconds per frame (default 1/60).
    pub dt: Option<f64>,
    /// Leave the last frame's fingers down at the end, for a screenshot
    /// mid-gesture (default false).
    pub hold: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct BuilderSelectParams {
    /// [col0, row0, col1, row1]: opposite corners, inclusive.
    pub rect: Option<[i64; 4]>,
    pub clear: Option<bool>,
    /// [dx, dy] in cells.
    pub move_by: Option<[i64; 2]>,
    #[schemars(extend("enum" = ["copy", "cut", "delete", "flip_h", "flip_v", "stamp", "paste", "place", "cancel"]))]
    pub action: Option<String>,
    /// [col, row]: the ghost's middle for paste, its top-left for place.
    pub at: Option<[i64; 2]>,
}

#[derive(Deserialize, JsonSchema)]
pub struct BuilderStampParams {
    /// A stamp's key.
    pub name: Option<String>,
    /// [col, row]: the ghost's top-left.
    pub at: Option<[i64; 2]>,
    /// Put the ghost down at once.
    pub place: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct BuilderSaveParams {
    /// The file's name; leave out to save under the map's own.
    pub name: Option<String>,
}

#[derive(Deserialize, JsonSchema)]
pub struct ClickParams {
    /// Window x, as `status {sections: [ui, camera]}` gives positions.
    pub x: f64,
    pub y: f64,
    /// left (default) or right.
    #[schemars(extend("enum" = ["left", "right"]))]
    pub button: Option<String>,
    /// [x, y] to drag to before releasing.
    pub drag_to: Option<[f64; 2]>,
    /// A tap rather than a mouse press: the hints turn to taps.
    pub touch: Option<bool>,
}

#[derive(Deserialize, JsonSchema)]
pub struct KeyParams {
    #[schemars(extend("enum" = ["tab", "escape", "enter", "undo", "redo", "copy", "cut", "paste", "delete", "backspace", "1", "2", "left", "right", "up", "down", "zoom_in", "zoom_out"]))]
    pub key: Option<String>,
    /// Characters to type this frame (build mode).
    pub text: Option<String>,
}

/// The room server's parameter structs (`ROOM_TOOLS`). Public, since the
/// room server is another crate: its handlers read a call through these.
pub mod room {
    use std::collections::BTreeMap;

    use schemars::JsonSchema;
    use serde::Deserialize;
    use serde_json::Value;

    use crate::simulation::debug::TankPatch;

    /// Just a room code: what most of `ROOM_TOOLS` takes.
    #[derive(Deserialize, JsonSchema)]
    pub struct CodeParams {
        /// The room's code, from `rooms` or `room_open`.
        pub code: String,
    }

    #[derive(Deserialize, JsonSchema)]
    pub struct RoomOpenParams {
        /// A shipped map by name (default: default).
        pub map: Option<String>,
        /// A whole map, instead of a shipped one by name.
        pub map_toml: Option<String>,
        #[schemars(extend("enum" = ["protect", "hunt", "destroy"]))]
        pub mission: Option<String>,
        pub seed: Option<u64>,
        /// Seats the room holds (default 1).
        #[schemars(range(min = 1, max = 8))]
        pub seats: Option<u64>,
        /// This room's own tuning rows {knob: value}, as `room_tuning` takes
        /// them.
        pub tuning: Option<BTreeMap<String, Value>>,
    }

    #[derive(Deserialize, JsonSchema)]
    pub struct RoomStepParams {
        pub code: String,
        /// Ticks to advance (default 1, at most 100000).
        #[schemars(range(min = 1, max = 100000))]
        pub ticks: Option<u64>,
        /// Add a snapshot of the world after the step (default false).
        pub snapshot: Option<bool>,
        pub kinds: Option<Vec<String>>,
        pub exclude: Option<Vec<String>>,
    }

    #[derive(Deserialize, JsonSchema)]
    pub struct SeatIntentParams {
        pub code: String,
        #[schemars(range(min = 0, max = 7))]
        pub seat: u64,
        /// Ticks to hold the intent for (default 1).
        #[schemars(range(min = 1, max = 100000))]
        pub ticks: Option<u64>,
        #[schemars(extend("enum" = ["up", "down", "left", "right"]))]
        pub move_dir: Option<String>,
        #[schemars(extend("enum" = ["up", "down", "left", "right"]))]
        pub face: Option<String>,
        pub fire: Option<bool>,
        /// With fire=true: press on ticks 0, N, 2N... and release in between.
        #[schemars(range(min = 1))]
        pub fire_every: Option<u64>,
    }

    #[derive(Deserialize, JsonSchema)]
    pub struct RoomSetTankParams {
        pub code: String,
        pub slot: u64,
        #[serde(flatten)]
        pub patch: TankPatch,
    }

    #[derive(Deserialize, JsonSchema)]
    pub struct RoomSpawnPickupParams {
        pub code: String,
        pub kind: String,
        pub x: f64,
        pub y: f64,
    }

    #[derive(Deserialize, JsonSchema)]
    pub struct RoomTuningParams {
        pub code: String,
        /// {knob: value} rows to set on this room.
        pub patch: Option<BTreeMap<String, Value>>,
        /// Drop every row this room set.
        pub reset: Option<bool>,
    }

    #[derive(Deserialize, JsonSchema)]
    pub struct RoomSnapshotParams {
        pub code: String,
        #[schemars(extend("enum" = ["compact", "full"]))]
        pub detail: Option<String>,
    }

    #[derive(Deserialize, JsonSchema)]
    pub struct RoomEventsParams {
        pub code: String,
        pub since: Option<u64>,
        pub limit: Option<u64>,
        pub kinds: Option<Vec<String>>,
        pub exclude: Option<Vec<String>>,
    }
}

/// Every tool the server answers, in the order the adapter lists them.
pub const TOOLS: &[ToolSpec] = &[
    ToolSpec {
        name: "status",
        description: "Where the game is: mode, seed, frame, outcome, mission, spawn plan, lockstep, tank counts, map. `sections` adds the heavy parts: camera, ui, builder, frame_stages, weather, stats, turns, overlays, or all.",
        help: "Where the running game is: seed, frame, time, outcome, mission and the resolved spawn plan (`wave` while waves run), paused/lockstep, tank counts, overlay flags, the loaded map, `frame_stages` (ms a frame by stage averaged over half a second, the `sim.*` groups inside `sim`; `draws`, what the last whole frame drew through the canvas - blits, rects, shapes, texture_switches - and the blend/shader/target scopes it opened; `pace`, the worst frame and the 99th percentile over the last `frames` frames and how many ran 0..4 `steps`), `camera` (the view the window last drew - `whole` for an arena or the builder, `follow` for a field map, `pinned` for the `camera` tool's - with its world `rect`, `scale` (bitmap px per world px), scene `target` and `window_field` (the field area on the window, in points); a followed view adds the `seat` it follows and its `focus` (seat|shared|split|spectating|nobody), whether it `cut` this frame, how far a `glide` from the wreck to a restarted round's start has come (0..1, eased; null with none - a round starting over on the map on screen glides, its first plays the establishing shot), its `lead` and sub-block `offset`, the `split` of a couch pair apart (null for one view: the divider's `line` - a point and the unit normal into the second half in the followed bitmap's pixels - and `window_line`, how far `apart` the halves' views stand and each half's `seat`, `rect`, `offset`, `cut` and `in_view`), the `establishing` shot's `phase` (whole|zoom|follow) and `progress` (view `establishing` while it plays), the `seating` (local|room), the `framing` - `visible_cells`, `device_scale` (device px per world px), `point_scale`, `block_px`, whether the zoom `snapped` to whole blocks, `tank_points`, `tank_mm` and the `bars` past the aspect clamp - and the `sight_box` it keeps: `half`, the `room` left for the look-ahead and whether it is `in_view`; and `motion`, the one motion switch - whether motion is `reduced` (no shake, no whole-screen ripple, the establishing shot cut rather than zoomed), the `reduce_motion` row's `setting` (platform|off|on) and what the `platform` said at startup, null where it says nothing), `ui` (the UI scale - window units per point -, the window and the safe area the chrome keeps to in points, whether it is laid out for `touch`, the input its `hints` name - `keys` or `touch`, the last one used: a touch landing turns them to taps and a key press back -, in play and online the corners' `buttons` and `clusters` and the `minimap` picture under the right cluster (`null` where none is drawn - an arena shown whole, a phone; a press there does nothing), and the `screen_buttons` of whatever stands over the round - the level select's open tiles (`level_N`) and `back`, a dialog's `one`/`two` or `leave`/`stay`, a level's end screen's `levels`/`again`/`next`, the lobby's live buttons (`host`, `join`, `key_a`, `confirm`, `ready`, `start`, `kick_1`, ...) - all in window coordinates, which is what `click` takes), `mode` (play|build|online) with the dialogs and the builder's state (the builder's chrome is laid out on the window in UI points like play's corners - plates in the top corners of the safe area and a tray along its bottom, floating over the canvas, which fills the window as play's field does; the popups under the top plates and rising from the tray -, and its rects come in window coordinates, what `click` and `builder_touch` take, each with its `ui` points; `builder.navigator`: the navigator's picture, `null` at FIT on an arena and while an open popup reaches over its corner; `builder.buttons`: the builder's buttons by name - the plates' `back`, `day` (while the canvas's sky is dark), `play`, `play_here`, `check`, `clear`, `fit`, `map`, `file`, `erase`, `undo`, `redo`, each category's `category_<name>` (its icon half) and `list_<name>` (its list half) and `brush` (BRUSH: the brush's shape, the select tool, the stamps), or the one `tools` button a phone's tray folds the six and BRUSH into; with the select tool the live buttons of the strip under the top plates - `sel_copy`, `sel_cut`, `sel_paste`, `sel_flip_h`, `sel_flip_v`, `sel_delete`, `sel_stamp` (keep the selection as a stamp), `sel_stamps` (the STAMPS list), or with a paste ghost `sel_place`, `sel_flip_h`, `sel_flip_v`, `sel_cancel`; while a popup is open its own - a list's or the palette's `tool_<name>`, BRUSH's list's and the palette's brush row's `shape_<name>`, `tool_select` and `brush_stamps`, the FILE menu's `load`, `save`, `save_as` and `clear_map`, the Load list's `map_<name>`, the STAMPS list's `stamp_<key>`, the MAP panel's - `tab_round`/`tab_tanks`/`tab_field`/`tab_sky` where a short room puts its groups behind a rail of tabs, `reset`, a choice's `<field>_<option>` (`mission_hunt`, `spawn_waves`, `theme_desert`), a stepper's `<field>_dec`/`<field>_inc` (`tanks`, `waves`, `size`, `growth`, `tier_start`, `tier_end`, `tank`, `tank2`, `width`, `height`, `anchor`) and a sky's tile `weather_<sky>`, which puts it in or takes it out -, the CHECK panel's `finding_N` rows and `fix_N` buttons - and a paged popup's `page_back`/`page_next`; `builder.sky`: the sky the canvas is drawn under (its map's, as the paused round's seed picked it, or `clear` while DAY is down) and `builder.day` whether DAY is down; `builder.shape`: how the brush paints (pen, rect, fill or scatter); `builder.rect`: the rectangle a RECT drag is drawing, filled on its release (null with none); `builder.message`: the status line's feedback - a save, a load, a FILL refused for its size - or null; `builder.selection` and `builder.ghost`: the select tool's rectangle and the paste ghost as {col, row, cols, rows} (the ghost with its `cells`), `null` for none; `builder.clipboard`: what COPY and CUT took ({cols, rows, cells}); `builder.stamps`: how many stamps the STAMPS list offers; `builder.check`: the CHECK panel's last report; `builder.clear`: the clear check - the canvas's `revision`, whether it is `cleared` (won from plain PLAY with no edit since) and its `par` in seconds, and the revision PLAY started the local round on (`attempt`); `builder.loupe`: the loupe over a painting finger - where it stands, the `world` it shows, its `device_scale` and the `cell` the stroke paints -, `null` without one; `builder.thumbnails`: while FILE > LOAD's list is open, every map whose thumbnail its pages have made so far, a picture a frame - its `name`, its size in `cells` and whether it has a `picture` (a map that does not parse has none) -, empty while it is closed), and `turns` (heading turns/reversals/spins summed over the live tanks this round - a non-zero `spins` is a tank rotating in place; see `history`). `round` says which round all of this describes: `local`, or `online` with the room code, the seat, `buffer_ms` (how far ahead of the picture the newest snapshot is), `rtt` (the measured round trip - median, p95, floor - and server-minus-local from ping/pong probes), the server's tick, the phase, `interpolation` (the delay in force and its target, the link's jitter, the measured cadence, frames drawn on extrapolation, lateness p50/p95, stalls, the playout rate, corrections and their p95 in px, stale events dropped) and `prediction` (the stage-2 counters: corrections ignored/nudged/snapped, the error histogram `error_buckets` at 0.25/0.5/2/8/48 px and past, `max_error_px`, shots drawn/refused/on screen, inputs `in_flight`, the local fire gate, the lead's `lead_up`/`lead_down` adjustments with the smoothed mailbox `lead_depth`, and decision 9's instrument: `crossings` - provisional shots the picture stopped against a drawn tank or bong -, `crossings_hit` - their paired room copy bursting within 40 px (`HIT_MATCH_PX`) of that stop - and `crossings_missed` - their copy flying on past it (`MISS_MARGIN_PX`) or bursting anywhere else) - in an online round every reading tool describes the room's replica and the tools that would write to it refuse, because only the server simulates it. Cheap; call first.",
        params: Params::of::<StatusParams>(),
        read_only: true,
        destructive: false,
        in_build: true,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "snapshot",
        description: "World state as JSON: every tank, projectile, pickup, bong, portal and zone, and air support's aircraft and calls. `detail: full` adds each enemy's AI memory.",
        help: "World state as JSON: every tank (position with its grid `cell`, `rotation` - the sim-side heading - with `facing` as a name and the two drawn angles `hull`/`turret` that ease toward it, real velocity with `speed` and `heading` - the direction it is actually moving, which differs from `rotation` when it is being shoved - damage/hp, ammo, weapon, shield/boost, `ring` - the health ring's opacity 0..1, nearest_ally_px; enemies also `role` - player/hunter/guard - and dist_to_player), projectiles, pickups, `bongs` (a list with `side` player/enemy: the player's bong first, then the enemy bong in a hunt round; `facing` left/right is which way the sprite is drawn - the art is authored facing right and mirrored for the other way, so it says whether a hop or a bite reads correctly), `portals` with `portals_active` (the map's portal anchors; each tank's `portal_cooldown` says when it may enter one again), `engage` (the engagement rings: per enemy its status - engaged/wreck/fleeing/retreating/out_of_range - the ring slot it holds and its target point, on the ring around the player or, for a hunter, the one around the player's bong; an engaged enemy with ring=null steers at its target directly, the pile-up case) and `clusters` (groups of live enemies within 90 px of each other); while there are any, `aircraft` (air support in the air, by id: `seat`, `sortie` heli/jet, `stage` inbound/attack/egress/escort/loiter, the ground point `x`/`y` and the `height` over it, `heading_deg` - 0 = up, clockwise -, `speed` in px/s, `bank` - -1 full left to 1 full right - and `ammo`, the helicopter's salvos or the jet's passes left; online they are the replica's, off the wire, whose `speed` is the sortie's full one since no velocity travels) and `air_calls` (the calls still to arrive, by id: `seat`, `sortie` and `seconds_left`, null while one waits behind the seat's sortie before it). detail=full adds each enemy's AI memory (role, waypoint, committed heading, last behaviour-tree action, stuck timer, intent), the per-enemy slot rejection tally (claimed/off_map/unreachable/no_los), the player ring's 16-slot table (point, line of sight, who holds it) and `command`, the enemy command layer's last decision (orders by slot, the skipped-conflict tally, the blackboard; `enabled` false while the `c2_enabled` knob is off).",
        params: Params::of::<SnapshotParams>(),
        read_only: true,
        destructive: false,
        in_build: false,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "events",
        description: "Simulation events after the cursor `since` (pass the reply's `next` back), filtered by `kinds`/`exclude`. AI-decision events are included.",
        help: "Gameplay events recorded since `since` (a seq number; 0 = everything kept, up to 4096): fired, hit, wreck, ram, deflected (off a shield), ricochet (a shot off iron or a barrel, with the heading it flies on along), laser_beam (a laser's beam from the muzzle to where it stopped, with its variant), shells_collided, bong_bite (with the biting bong's side), pickup_collected, pickup_respawned, obstacle_destroyed, blast, drum_launched, fire_started, ignited (the flamethrower lit `what`: ground, oil, wood, tree, drum, or collapsed a sandbag/fence), oil_printed (a hull carrying oil out of a pool printed `cell`, going `dir`, with `fade` eighths left; `oil_print_cells` above 0), teleported (a tank went through a portal: slot, from x/y, to to_x/to_y), grass_trampled (hulls drove into the grass `cell` `grass_trample_passes` times: it hides nobody for the round), round_started, round_ended, plus AI decisions - ai_action (behaviour-tree action changed), engage_slot (ring slot changed; null = steering at its target - the player, or a hunter's bong - directly), stuck_escape, breach (dir, or null when it ends), retreat (on/off), alert (shared last-known player position on/off), retarget (rounds with more than one seat: the enemy switched to fighting seat `player`). Each carries the frame it happened on. `kinds` keeps only those event names, `exclude` drops them.",
        params: Params::of::<EventsParams>(),
        read_only: true,
        destructive: false,
        in_build: false,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "step",
        description: "Freeze the game in lockstep and advance exactly `frames` frames at 1/60 s, holding optional player input. Replies with the step's events; `snapshot: true` adds the world.",
        help: "Freeze the game in lockstep and advance exactly `frames` simulation frames at the fixed 1/60 s timestep (all in one rendered frame, so it is fast and deterministic). Optional player input is held for those frames (`move_dir`/`face`/`fire`/`lamp` for player 1, `p2_move_dir`/`p2_face`/`p2_fire`/`p2_lamp` for player 2 in a two-player round; a held `lamp` sets one lantern down, on the press - docs/volcano.md); shells/plasma fire once per press, so set fire_every=N to tap the trigger every N frames instead of holding it. Replies with the events of the step (first 256; `kinds`/`exclude` filter by event name, see `events`) and, by default, a compact snapshot. Use `resume` to let the game run in real time again.",
        params: Params::of::<StepParams>(),
        read_only: false,
        destructive: false,
        in_build: false,
        online: false,
        present: Present::Json,
    },
    ToolSpec {
        name: "input",
        description: "Hold player input for `frames` real-time frames in place of the keyboard. It does not freeze the game; `step` is the deterministic way.",
        help: "Override a player's input for the next `frames` real-time frames (keyboard is ignored meanwhile): `move_dir`/`face`/`fire`/`lamp` for player 1, `p2_move_dir`/`p2_face`/`p2_fire`/`p2_lamp` for player 2 in a two-player round. Works while the game runs; in lockstep prefer step's own input fields. `cycle_overlays: true` presses the I key once: cycles the overlay presets off -> inspect -> all -> off (with no move_dir/face/fire it leaves the keyboard alone). **In an online round** it drives this window's seat in the room the way the keyboard does - sent at the tick rate, predicted and drawn as a player's press is - so a weapon can be fired as the rig's or a room's seat and the picture screenshotted; the player-2 fields and `cycle_overlays` are refused there, since they would reach nobody (`overlays` sets the replica's overlays). A shell fires once per press, so to fire again send `fire: false` for a frame, then `fire: true`.",
        params: Params::of::<InputParams>(),
        read_only: false,
        destructive: false,
        in_build: false,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "pause",
        description: "Freeze the game in lockstep, so only `step` advances it.",
        help: "Enter lockstep: the game stops advancing (no PAUSED overlay, so screenshots stay clean) until `step` or `resume`.",
        params: Params::of::<NoParams>(),
        read_only: false,
        destructive: false,
        in_build: false,
        online: false,
        present: Present::Json,
    },
    ToolSpec {
        name: "resume",
        description: "Leave lockstep: the game runs in real time again.",
        help: "Leave lockstep and clear the P-key pause: the game runs in real time with wall-clock dt again.",
        params: Params::of::<NoParams>(),
        read_only: false,
        destructive: false,
        in_build: false,
        online: false,
        present: Present::Json,
    },
    ToolSpec {
        name: "restart",
        description: "Start a fresh round, optionally with a seed, a map (a path or TOML text), mission, spawn plan, chassis and player count. Leaves it frozen in lockstep at frame 0.",
        help: "Start a fresh round, frozen in lockstep (call `resume` to let it run in real time). Optional seed (number or 0x-hex string; pinned for later restarts too), enemy count, player 1's chassis as `tank` (a name - scout, assault, ..., leviathan - the spelling maps and `--tank` use) or `tank_row` (0-11, the sheet order), `players` (1 to 8 - how many seats the round holds, kept for later restarts; the keyboard drives the first two and the rest stand idle in a local round, and `tank2`/`tank2_row` pin seat 1's chassis the way `tank` pins seat 0's), the map: `map` (a path to a TOML under maps/) or `map_toml` (the map's TOML text inline - see `map_get` for the format; the round keeps its current map when neither is given), and the level: `mission` (protect|hunt|destroy), `spawn` (band|waves) with `waves`/`wave_size`/`wave_growth`/`tier_start`/`tier_end` (light|medium|heavy|super) - each pinned for later restarts too, overriding the map's own [mission]/[spawn] tables. `intro: true` starts the round frozen behind the mission banner (off by default so `step` counts play frames). Same seed + same steps replays bit-for-bit.",
        params: Params::of::<RestartParams>(),
        read_only: false,
        destructive: true,
        in_build: true,
        online: false,
        present: Present::Json,
    },
    ToolSpec {
        name: "map_get",
        description: "The round's map: its identity and its full TOML text, which `restart {map_toml}` takes back.",
        help: "The current map as TOML text (plus name, cell count, default tank count) - edit it and hand it back through `restart {map_toml}`. Format: `version = 1`, optional `tanks = N` (default enemy count), optional `tank = \"titan\"` / `tank2 = \"scout\"` (the players' chassis), optional `bong = \"oggle\"` (the players' bong: spike, oggle or gloop; spike when absent) / `enemy_bong = \"capi\"` (the enemy's in a Hunt round: blip, puff, capi or fuzz; blip when absent), optional `theme = \"grass\"|\"desert\"` (the look - ground tileset and tall-grass sheet, grass when absent), optional `weather = \"night\"` (the sky: clear, night, dusk, rain, storm, fog, sandstorm, snow, heat_haze or random - every sky; or a list, `weather = [\"rain\", \"snow\"]`, of which each round's seed picks one; clear when absent), and one `cells.\"col,row\"` entry per occupied 32 px grid cell (col/row from 0 at the top-left; the field is the map's optional `size = [cols, rows]`, 34 x 17 = 1088x544 when absent): `{ kind = \"wall\", material = \"brick\"|\"iron\"|\"wood\"|\"glass\" }`, `{ kind = \"sandbag\" }` / `{ kind = \"barrel\" }` / `{ kind = \"fence\" }` (destructible props: shots sometimes pass over sandbags, barrels explode and chain, fences snap; tanks ram all three), `{ kind = \"barrel\", drum = \"oil\"|\"fuel\" }` (a pinned drum kind: oil leaves a burning pool, fuel goes off harder and launches when another blast sets it off; without `drum` the kind is rolled), `{ kind = \"oil\" }` (an oil trail cell: not solid, a fuse on the ground - a blast or a burning neighbour lights it and the fire runs along it, setting off any drum it reaches), `{ kind = \"tree\" }` / `{ kind = \"pine\" }` / `\"spruce\"` / `\"scots\"` / `\"fir\"` / `\"birch\"` / `\"willow\"` / `\"palm\"` / `\"snag\"` (destructible trees, solid like a prop but drawn larger than their cell; they often catch fire when killed and a tank can flatten one by driving into it; a fir sapling, a palm and a dead snag hide nothing behind them; on a desert map every tree but the palm is dry), `{ kind = \"tall_grass\" }` (not solid - cover a tank hides in, enemies cannot shoot what is standing in it), `{ kind = \"bush\" }` / `\"berry_bush\"` / `\"juniper\"` / `\"fern\"` / `\"autumn_bush\"` / `\"reeds\"` (soft cover like tall grass, one bush to a cell), `{ kind = \"road\" }`, `{ kind = \"water\" }` (a river where it is one cell wide, a lake where it is wider; a lake's open middle is deep - hulls cannot enter, shots fly over - and every other water cell is a ford that slows a hull and, in a north-south stream, carries it downstream; fire never lights on water, bongs hop toward it), `{ kind = \"bong\" }` (one), `{ kind = \"start\" }` (player 1, one), `{ kind = \"start2\" }` (player 2, one, optional - placed beside player 1 when absent, as every seat past the second always is), `{ kind = \"pickup\", pickup = \"health\"|\"ammo\"|\"laser\"|\"minigun\"|\"plasma\"|\"missiles\"|\"speedup\"|\"shield\"|\"flamethrower\"|\"bong_health\" }` (missiles are a four-tube pod firing a salvo of four seeker missiles per pull, one pull a pickup, that climb, lock onto the nearest opposing tank and dive on it over any wall; the flamethrower is player-only: enemies drive over its fuel tank; the bong health pack fully heals the collector's own bong and is left on the ground by a tank whose bong is already at full health). Iron is indestructible, the rest can be shot away. Border walls and enemy spawns are added by the game on top.",
        params: Params::of::<NoParams>(),
        read_only: true,
        destructive: false,
        in_build: true,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "lint",
        description: "Run the map linter on the builder's canvas or the round's map: findings with severity, code and cells.",
        help: "Run the static map linter (src/maplint.rs, the check CI runs on every shipped map) and reply with its findings. `source: builder` lints the builder's canvas as it stands (the default in build mode - validate a map authored with `builder_paint` before `play`); `source: round` lints the map the current round was built from, fresh (the default in play mode - not the round's current, partly shot-away terrain). The map is set up as a headless round with the session's seed, player count and CLI/restart overrides, so the check sees what PLAY would run. Each finding has `severity` (error|warning|info), `kind` (a tag such as gated-pickup, spawn-band-too-tight, gate-blocked, player2-unreachable) and `message`; `errors`/`warnings` count them. Two limits: the spawn-band check reads the map's own `spawn` table (not a `restart {spawn}` override), and the player-2 kinds appear only in a session with more than one seat.",
        params: Params::of::<LintParams>(),
        read_only: true,
        destructive: false,
        in_build: true,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "terrain",
        description: "Live tiles by cell: material, health, burning and fused state. Filter with `only` and `materials`.",
        help: "The battlefield's tiles and its fire layer as JSON - the numeric view of props, walls and flames that `snapshot` (tanks only) lacks: every live obstacle tile by grid `cell` with material, `hp`/`max_hp`, and when set `drum` (oil|fuel), `burning`/`burn_elapsed`, `fuse` {left, total} (an armed barrel), `heat` (flame exposure), `scorched` (blast-sooted faces, N E S W as bits 0..3), `ram_timer`, `flammable`; plus `fires` (burning ground cells: left, total, pool), `fused` (armed drums' cells), `flames` (this frame's flamethrower jets: shooter slot, origin, direction, range, reach), `burning_tanks`/`burning_wrecks`, and counts of burning tiles, flying drums, oil cells, grass cells and heated cells, `grass_trampled` (the grass cells trampled for the round, as [col, row]; `grass_trample_passes`), plus `portals` (the map's portal anchors by cell) and `portals_active`. `only` keeps just the damaged (hurt, burning, fused, sooted, heated or rammed), burning or fused tiles; `materials` keeps the listed ones. At most 800 tiles (`truncated`).",
        params: Params::of::<TerrainParams>(),
        read_only: true,
        destructive: false,
        in_build: false,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "history",
        description: "Per-tank track rows over the last frames, with aggregates: jitter, spins, clustering, turn counts.",
        help: "Per-tank rows recorded every frame (last 60 s, cleared on restart): position, heading (`rotation`) and the drawn turret angle, behaviour-tree action, ring slot, stuck, touching terrain, is_player. Replies with every N-th frame's rows (`every`) over the last `last` frames, optionally one `slot`, plus per-tank aggregates over the whole window: frames seen, distance travelled, net displacement, cluster_frames (2+ other live enemies within 90 px), stuck_frames, no_ring_frames (engaged without a slot), touching_frames (static terrain), tank_touching_frames (another tank's hull - the jam signal, which unlike a ram count does not saturate), and `round`, the tank's heading-turn counters since the round began: turns (heading changes), u_turns (180 flips), reversals (A->B->A within 2 s, the probe's jitter unit), spins (a full same-direction circle of quarter-turns within 2 s while drifting under 60 px - the probe's spin rule; the turret visibly rotating in place), max_spin_deg, turret_deg (the turret sprite's total sweep) and last_turn_frame. The live-game counterpart of the probe's per-round stats.",
        params: Params::of::<HistoryParams>(),
        read_only: true,
        destructive: false,
        in_build: false,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "screenshot",
        description: "The last rendered frame as a PNG (the state after the latest step). `overlays` sets debug overlays for it.",
        help: "Capture the current frame (the state after the latest step) as a PNG: returned inline and saved under target/devshots/. scale 0.5 (default) halves it; use 1.0 to read overlay text. Optionally set overlay flags in the same call (same as the `overlays` tool). source=scene skips the HUD and overlays.",
        params: Params::of::<ScreenshotParams>(),
        read_only: false,
        destructive: false,
        in_build: true,
        online: true,
        present: Present::Image,
    },
    ToolSpec {
        name: "overlays",
        description: "Read or set the debug overlay flags drawn over the round.",
        help: "Set persistent debug overlays drawn on top of the game (visible to the human too), one flag at a time: nav_grid (blocked pathfinding cells), ai (each enemy's waypoint, heading, last behaviour-tree action), projectiles (hit boxes + velocity), engage (engagement-ring targets), pickups (collect radius), hitboxes (each tank's hull and turret damage boxes and its rounded movement collider), stats (each tank's readout card: ammo, weapon, hp, speed, velocity, collider size, an enemy's retreat/fire state), camera (a followed view's camera reasoning: the sight box round each seat it keeps in orange, the dead zone round its base in yellow, the look-ahead in cyan and its cap, the fight's pull in magenta, the committed heading in green and one being turned to in pale green, the spring's goal as a yellow cross and the spring as a white ring, a shot's kick and a blow's shove as a red line four times its length, and the numbers along the view's bottom edge; the same numbers are `status.camera.trace`). Omitted flags keep their value, an unknown flag is an error; replies with the current flags. The I key in the game window cycles presets instead (off -> inspect = hitboxes + stats -> all; never `camera`); `input {cycle_overlays: true}` presses it.",
        params: Params::of::<OverlayFlags>(),
        read_only: false,
        destructive: false,
        in_build: true,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "camera",
        description: "Pin the view to a world point and zoom, or `reset` to give it back to the map.",
        help: "Pin the part of the world the window draws, for screenshots (docs/large-maps-follow-camera.md): `x`/`y` a world point in field pixels at the view's centre and `zoom` 1 or more - the view shows the field's size divided by the zoom, kept inside the field and snapped to whole 2 px blocks, and fills the field area under the HUD bar, which is drawn as ever. A field left out keeps the pin's own (the field's centre, zoom 1, for a first pin). A pin outranks a field map's follow camera. `reset: true` lets the pin go: the view is the map's again - the whole field for an arena, the follow camera for a field map; no parameters only report. The pin holds across restarts and map changes, clamped to each field, and changes only the picture - the round, the AI and the builder's canvas never see it. Replies like `status.camera`: `view` (whole|follow|pinned), the visible world `rect`, `scale` (bitmap px per world px), the scene `target` size in texels and a pin's `center` and `zoom` (a followed view's position is the one the window last drew, unknown until it has drawn one).",
        params: Params::of::<CameraParams>(),
        read_only: false,
        destructive: false,
        in_build: true,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "nav_grid",
        description: "The routing grid as ASCII: blocked cells, tanks, the bong.",
        help: "The AI's pathfinding grid as text (# blocked, . open) with tanks (P player, digits enemies, x wrecks), the bong (F) and pickups (*) marked - the cheapest way to reason about the layout.",
        params: Params::of::<NoParams>(),
        read_only: true,
        destructive: false,
        in_build: false,
        online: true,
        present: Present::Text("grid"),
    },
    ToolSpec {
        name: "field",
        description: "The flow field toward a target (player, player2 or bong): direction and cost per cell.",
        help: "The flow field enemies follow toward a shared target this frame: `arrows` (one line per row: ^ v < > the neighbour each cell steps into, G the goal, # blocked, . unreachable) and `costs` (the cost to the goal per cell, -1 blocked/unreachable, so a priced firing lane or crowd cell shows as a jump). target: player (default), player2 or bong. Errors when that target is not in the round.",
        params: Params::of::<FieldParams>(),
        read_only: true,
        destructive: false,
        in_build: false,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "teleport",
        description: "Move a tank to (x, y), optionally facing a direction.",
        help: "Move a tank to (x, y) in field pixels (velocity zeroed) and optionally snap its facing.",
        params: Params::of::<TeleportParams>(),
        read_only: false,
        destructive: false,
        in_build: false,
        online: false,
        present: Present::Json,
    },
    ToolSpec {
        name: "set_tank",
        description: "Patch a tank's state: damage, ammo, special weapon charges, shield, timers. Replies with the tank.",
        help: "Overwrite a tank's damage (0 = pristine, 100 = wreck), ammo counts (setting a special weapon's stock above 0 also arms it in place of the one special the tank carries, like its pickup would), shield_hp (rainbow-shield absorption left in damage points, not seconds), the speed-boost timer, portal_cooldown (seconds before it may enter a portal again) and charge (seconds a charge weapon's trigger has been held - a gauss rail's charge or a rod's reticle put on a tank carrying it; 0 takes it off). Omitted fields are untouched.",
        params: Params::of::<SetTankParams>(),
        read_only: false,
        destructive: false,
        in_build: false,
        online: false,
        present: Present::Json,
    },
    ToolSpec {
        name: "kill",
        description: "Destroy a tank through the normal explosion path (applied on the next frame).",
        help: "Destroy a tank on the next simulated frame through the normal kill path (explosion, shockwave, wreck event, round end).",
        params: Params::of::<SlotParams>(),
        read_only: false,
        destructive: true,
        in_build: false,
        online: false,
        present: Present::Json,
    },
    ToolSpec {
        name: "spawn_enemy",
        description: "Add an enemy at (x, y), optionally by chassis row and role.",
        help: "Add an enemy at (x, y) with an optional chassis row (0-11) and AI role (player - fights the player, the default; hunter - drives at and shoots the player's bong; guard - stays leashed to the enemy bong). Draws from the round RNG, so the round stops being the seeded replay afterwards. Returns the new slot.",
        params: Params::of::<SpawnEnemyParams>(),
        read_only: false,
        destructive: false,
        in_build: false,
        online: false,
        present: Present::Json,
    },
    ToolSpec {
        name: "spawn_pickup",
        description: "Drop a supply crate of `kind` at (x, y).",
        help: "Put a crate of `kind` (a map's pickup spelling: health, ammo, laser, minigun, plasma, missiles, speedup, shield, flamethrower, bong_health, tower_pack, heat_shield, grenades, sonic_hammer, emp_burst, gauss_rail, fpv_swarm, rod_from_god, gravity_well, air_heli, air_jet) down at the map cell nearest (x, y), in its air drop. Not a slot: it never respawns. Refused outside the field, on a solid tile and where a pickup already stands. Draws no RNG. Returns the crate's position.",
        params: Params::of::<SpawnPickupParams>(),
        read_only: false,
        destructive: false,
        in_build: false,
        online: false,
        present: Present::Json,
    },
    ToolSpec {
        name: "rod_call",
        description: "Call a rod from god onto (x, y).",
        help: "Call a rod from god onto the map cell nearest (x, y) at once (docs/rod-from-god.md): player 1's call, the kills credited to it, or with enemy=true an enemy's. It stands for rod_countdown_seconds and lands like any other. Refused outside the field. Draws no RNG. Returns the call's id, its cell and the round time it lands at.",
        params: Params::of::<StrikeParams>(),
        read_only: false,
        destructive: false,
        in_build: false,
        online: false,
        present: Present::Json,
    },
    ToolSpec {
        name: "well_at",
        description: "Open a gravity well at (x, y).",
        help: "Anchor a gravity well at (x, y) at once (docs/gravity-well.md): player 1's, or with enemy=true an enemy's; it forms for well_form_seconds, pulls for well_pull_seconds and collapses like any other. Refused off the field or inside a solid cell. Draws no RNG. Returns the well's id, its centre and the round time its pull starts.",
        params: Params::of::<StrikeParams>(),
        read_only: false,
        destructive: false,
        in_build: false,
        online: false,
        present: Present::Json,
    },
    ToolSpec {
        name: "tuning_get",
        description: "Current tuning knobs as {name: value}; `diff_only` returns just the ones that differ from the compiled defaults.",
        help: "Current tuning knobs as {name: value}; diff_only=true returns just the knobs that differ from the compiled defaults.",
        params: Params::of::<TuningGetParams>(),
        read_only: true,
        destructive: false,
        in_build: true,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "tuning_set",
        description: "Stage a tuning patch {knob: value}; it applies at the next frame boundary.",
        help: "Apply a tuning patch {knob: value} (array knobs as name.label or a full array) at this frame boundary. Range-checked; the whole patch is rejected on any bad key.",
        params: Params::of::<TuningSetParams>(),
        read_only: false,
        destructive: false,
        in_build: true,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "tuning_reset",
        description: "Reset every tuning knob to its compiled default.",
        help: "Restore every tuning knob to its compiled default.",
        params: Params::of::<NoParams>(),
        read_only: false,
        destructive: true,
        in_build: true,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "tuning_schema",
        description: "The tuning knobs' rows (group, range, default, doc), filtered by `group` or `name_contains`.",
        help: "The knob table (name, group, type, doc, range, default, when it applies). Several hundred rows, so filter by group and/or a name substring.",
        params: Params::of::<TuningSchemaParams>(),
        read_only: true,
        destructive: false,
        in_build: true,
        online: true,
        present: Present::Json,
    },
    // ----- the two modes and the map builder (docs/game-editor-fusion.md section 11) -----
    ToolSpec {
        name: "mode",
        description: "Cheap: which mode the window is in (play, build, online) and what stands over the round. Call first.",
        help: "Which mode the window is in - play (the round) or build (the map builder in the same window) - plus whether the leave-round dialog is open and the builder's state: dirty (edited since it was loaded), map name, active tool and its category, undo/redo depth. Cheap; `status` carries `mode` too.",
        params: Params::of::<NoParams>(),
        read_only: true,
        destructive: false,
        in_build: true,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "lang",
        description: "Read the UI language, or switch it with `tag`.",
        help: "The language the window draws its text in (docs/localization-prd.md). Without `tag` reports the one in force and every shipped tag; with `tag` (`en`, `sl`) switches to it at this frame boundary - a tag the game has not got is answered with English. Works in every mode; nothing about the round changes.",
        params: Params::of::<LangParams>(),
        read_only: false,
        destructive: false,
        in_build: true,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "weather",
        description: "The sky in force and what it does to the round; `name` changes it, `restart: true` starts the round over under it.",
        help: "The sky over the round on screen (docs/weather.md): drawn, and part of the rules - shorter enemy sight at night, in a storm and in fog, less grip in the rain, the water frozen in the snow, gusts in a sandstorm. A sky is settled when a round starts. Without `name` reports `in_force` (the round's sky - never `random`: a random weather is the sky the round's seed picks, the same for the same seed), `map` (the round's map's own `weather` key: a name, or the list of skies the seed picks among), `override` (the `weather_override` tuning knob's, null when it follows the map - `--weather` and the web page's `?weather=` set it; it applies from the next round), `rules` (`on`, `enemy_sight_px`, `grip`, `frozen`, `gust_on_player` - player 1's wind in px/s -, `gust_front` - the sandstorm gust crossing the field, its `start` in round seconds and its `dir`), `without_shaders` (the window draws its skies without the weather's shaders - they would not compile here, or the `weather_without_shaders` knob asks: the light map blended in, the snow, fog, sand, rain and falling snow as plain blocks) and every name. With `name` (clear, night, dusk, rain, storm, fog, sandstorm, snow, heat_haze, random, or a list of them, the round's seed picking one) puts that key on the round's map and changes the sky mid-round, the round running on (`Game::change_weather`: snow ices the water over, but ice stays ice until the next round); with `restart: true` it starts the round over on its own seed instead, frozen like `restart` leaves it, so the water is settled from the new sky. The key lasts through `restart`s on that map, and the override knob still outranks it. An online window only reports: a room's round is fought under its map's sky. The builder's WEATHER checklist is `builder_settings {weather}`; `map_get`/`restart {map_toml}` carry the key as `weather = \"night\"` or `weather = [\"rain\", \"snow\"]`.",
        params: Params::of::<WeatherParams>(),
        read_only: false,
        destructive: false,
        in_build: false,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "build",
        description: "Enter the map builder (the BUILD button); the round pauses behind it.",
        help: "Press BUILD in play mode: enters the builder at once, mid-round and on the end screen alike, the round paused behind it for `back` to come back to (only `play` puts a new round in its place). With the level select or the players dialog up it closes that instead. A no-op in build mode. Replies like `mode`, whose `back` is then `{puts_away}`.",
        params: Params::of::<NoParams>(),
        read_only: false,
        destructive: false,
        in_build: true,
        online: false,
        present: Present::Json,
    },
    ToolSpec {
        name: "back",
        description: "The builder's BACK: return to the round it paused.",
        help: "Press BACK in build mode (or Escape there with nothing on the canvas to take back): the round BUILD paused comes back exactly where it stood, at once, still frozen in lockstep if it was. Edits made since BUILD are put away - the canvas goes back to the round's map as one undo step, which `builder_undo` (UNDO in the next BUILD) takes back - and `status.ui`'s left cluster carries a line saying so for `ui_put_away_seconds`. `mode.back.puts_away` says beforehand whether it will. A no-op in play mode and where there is no paused round (`mode.back` null). Replies like `mode`.",
        params: Params::of::<NoParams>(),
        read_only: false,
        destructive: false,
        in_build: true,
        online: false,
        present: Present::Json,
    },
    ToolSpec {
        name: "players",
        description: "Open the players dialog, or answer it with `count` (a changed count restarts the round).",
        help: "The players button in play mode (docs/two-players.md). Without `count`: press it - opens the 'How many players?' dialog (the round is frozen until it is answered) or closes an open one. With `count` (1 to 8): answer it - a different count restarts the round at once with that many seats, frozen in lockstep like `restart`; the current count just closes the dialog. The dialog itself only offers one and two - the couch counts; the rest are a room's, and reach the round through this tool or `restart`. The count sticks for the session (later `restart`s, PLAY from the builder). Two seats: slot 1 is player 2 and enemies count from 2; `step`/`input` take `p2_*` fields for it. Past two, the extra seats stand idle and the bar keeps the two-player readout. Replies like `mode`.",
        params: Params::of::<PlayersParams>(),
        read_only: false,
        destructive: true,
        in_build: false,
        online: false,
        present: Present::Json,
    },
    ToolSpec {
        name: "play",
        description: "Start a round on the builder's map (PLAY) and leave it frozen in lockstep.",
        help: "Press PLAY in build mode: the builder's (edited) map becomes the round's map and a fresh round starts, frozen in lockstep like `restart` (so `step` counts play frames; `resume` for real time). A seed pinned by an earlier `restart` stays pinned. `intro: true` starts the round frozen behind the mission banner. Fails in play mode - `restart` starts a round there. Replies with `status`.",
        params: Params::of::<PlayParams>(),
        read_only: false,
        destructive: true,
        in_build: true,
        online: false,
        present: Present::Json,
    },
    ToolSpec {
        name: "builder_tool",
        description: "Read or pick the builder's tool and brush shape.",
        help: "Select the builder's brush by name - brick, iron, wood, glass, concrete (WALL); sandbag, barrel, oil_drum, fuel_drum, fence, tesla, tesla_enemy, gun_tower, gun_tower_enemy, bio_slush, bio_slush_enemy (PROP); road, water, lava, tall_grass, tree, pine, oil_trail, gate, portal, volcano, lamp (GROUND); start, start2 (player 2's start), bong, enemy_bong (ACTOR); health, ammo, laser, minigun, plasma, missiles, speedup, shield, flamethrower, bong_health, tower_pack, heat_shield (PICKUP); eraser; or select, the rectangle select tool (BRUSH's list; `builder_select` drives it) - through the category's own selection path, so the bar's category button updates as well. Any tool but select lets the selection go and takes a paste ghost away. `shape` sets how the brush paints, BRUSH's list's shapes (docs/large-maps-follow-camera.md section 9): pen (a stroke, under the toggle-erase rule), rect (a rectangle from the press to the release, filled on the release; the eraser or the right button clears it), fill (a flood from the pressed cell over the cells joined to it edge to edge that hold exactly what it holds, at most `builder_fill_max_cells` - past it nothing changes and `status.builder.message` says why -, under the toggle-erase rule) or scatter (a hashed share, `builder_scatter_density`, of the cells within `builder_scatter_radius_cells` of each cell the stroke crosses, laid on empty cells only; erasing, it thins the brush's object, or anything under the eraser). Picking a shape takes a painting brush back from the select tool; a singleton (start, start2, bong, enemy_bong) paints with the pen whatever the shape. Without `tool` or `shape`, only reports the active tool, the brush's `shape` and every category's current tool and full list (the authoritative spelling of every brush).",
        params: Params::of::<BuilderToolParams>(),
        read_only: false,
        destructive: false,
        in_build: true,
        online: false,
        present: Present::Json,
    },
    ToolSpec {
        name: "builder_paint",
        description: "Paint a stroke of cells with the current (or given) tool and shape, as one undo step.",
        help: "One stroke on the builder's canvas: a press on cells[0], a drag through the rest, a release - so the toggle-erase rule (a press on a cell that already holds exactly the brush's object erases it, and paint-or-erase is decided on the first cell for the whole stroke), singleton moves (start/start2/bong/enemy_bong) and one-undo-step-per-stroke apply exactly as for a mouse. Cells are [col, row] on the 32 px grid (the map's `size`, 34 x 17 when absent, from the top-left). `tool` selects a brush first and `shape` how it paints (see `builder_tool`): with rect the stroke's first and last cells are the rectangle's corners, with fill only cells[0] counts, with scatter every cell's footprint is laid. `button: right` erases whatever the brush. Replies with every changed cell's object before and after (in the map's own shape, null = empty), the undo depth and the status line's `message` (a FILL refused for its size says so there).",
        params: Params::of::<BuilderPaintParams>(),
        read_only: false,
        destructive: false,
        in_build: true,
        online: false,
        present: Present::Json,
    },
    ToolSpec {
        name: "builder_undo",
        description: "Undo the builder's last `steps` edits.",
        help: "Undo the builder's last `steps` edits (default 1; a whole stroke, one settings field, a load or a reset is one step each). Replies with how many were undone, the depth left on each side and the cells the last undone step changed.",
        params: Params::of::<StepsParams>(),
        read_only: false,
        destructive: false,
        in_build: true,
        online: false,
        present: Present::Json,
    },
    ToolSpec {
        name: "builder_redo",
        description: "Redo the builder's last `steps` undone edits.",
        help: "Redo the builder's last `steps` undone edits (default 1). Replies with how many were redone, the depth left on each side and the cells the last redone step changed.",
        params: Params::of::<StepsParams>(),
        read_only: false,
        destructive: false,
        in_build: true,
        online: false,
        present: Present::Json,
    },
    ToolSpec {
        name: "builder_settings",
        description: "Read or set the builder map's settings: size, tank count, chassis, bongs, mission, spawn plan, theme, biome, weather. null means auto.",
        help: "The builder's MAP settings - the map file's own level keys: tanks (enemy count 0-31), tank (player 1's chassis name), tank2 (player 2's, two-player rounds), bong (the players' bong: spike|oggle|gloop, null = spike), enemy_bong (the enemy's in a Hunt round: blip|puff|capi|fuzz, null = blip), mission (protect|hunt|destroy), spawn (band|waves), waves (1-20), wave_size (1-31), wave_growth (0-10), tier_start/tier_end (light|medium|heavy|super), theme (grass|desert - the look: ground tileset and tall-grass sheet; the canvas redraws in it at once; a biome made for the other theme comes off with it), biome (valley|floodplain|karst, null = none - the kind of place, docs/biomes.md, the panel's LOOK: one made for the map's theme, valley and floodplain for grass, karst for desert, any other refused; the canvas redraws in it at once), weather (clear|night|dusk|rain|storm|fog|sandstorm|snow|heat_haze|random, or a list of them - the skies a round may be fought under, each round's seed picking one, docs/weather.md; the panel's WEATHER checklist; the canvas itself stays clear to edit on), anchor (where the old map sits when the size changes: top_left|top|top_right|left|center|right|bottom_left|bottom|bottom_right - the panel's ANCHOR, kept for the session), size ([cols, rows] cells, 16 x 9 up to 250 a side: the map resized with the old one placed by the anchor, cells that land past the new edge dropped - one undo step, which brings them back - and the canvas made again on the new field). A field left out is untouched; a field set to null goes back to auto (unset: the game's own roll or the `waves` tuning group; mission/spawn back to protect/band). Each changed field is one undo step, in the order listed. `reset: true` then reverts cells and settings to the baseline (one undoable step). Replies with the current values (null = auto, `size` in cells) and `cli_overrides`: which of them a command-line flag (-e, --tank, --mission, ...) or an earlier `restart` parameter overrides at PLAY, so the map's value is not what the round will use.",
        params: Params::of::<BuilderSettingsParams>(),
        read_only: false,
        destructive: false,
        in_build: true,
        online: false,
        present: Present::Json,
    },
    ToolSpec {
        name: "builder_map",
        description: "The builder's map as TOML with its diff from the baseline; or load one by name, path or TOML text, or clear the canvas.",
        help: "Without parameters: the builder's map as TOML (`toml`), its name, `dirty` and `diff` - cells added/removed/changed and the settings fields that differ from the baseline (the map as loaded). With `name` (a Load-list name from `builder_files`), `map` (a path under maps/) or `map_toml` (inline TOML, the `map_get` format): loads that map into the canvas as one undo step and makes it the new baseline - the FILE > LOAD path; the round keeps its map until `play`. With `clear`: FILE > CLEAR MAP - empties the canvas of every placed object as one undo step, keeping the settings, size, theme and name; not a new baseline, so `dirty` is set. `map_get` keeps answering with the map the current round was built from, which differs from this once the builder is dirty.",
        params: Params::of::<BuilderMapParams>(),
        read_only: false,
        destructive: false,
        in_build: true,
        online: false,
        present: Present::Json,
    },
    ToolSpec {
        name: "builder_files",
        description: "The maps the builder can load, and whether saving is available.",
        help: "What FILE > LOAD offers: every map the builder can load by name - the files under maps/ on native (`on_disk`) plus the maps shipped inside the binary (default, hunt-basic, waves-basic; all the web build has) - and `can_save`, whether this build keeps maps (with map modding on, every build, in the player's store; off, native only). With modding on, a shipped map the player changed is marked `modified` and opens as their copy.",
        params: Params::of::<NoParams>(),
        read_only: true,
        destructive: false,
        in_build: true,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "builder_camera",
        description: "Read or frame the builder's canvas camera.",
        help: "The builder's own camera over its canvas (docs/large-maps-follow-camera.md section 9), the way to frame a screenshot of the builder: `x`/`y` a world point in field pixels to put in the middle of the canvas and `zoom` how many times FIT (1 is FIT, the whole map); a field left out keeps the camera's own. `fit: true` goes back to the whole canvas, as the FIT button does. The zoom is kept between FIT and a cell of `builder_zoom_max_cell_pt` points and the view inside the field, centred on an axis the map does not fill. Independent of the round's `camera` pin. No parameters only reports. Replies like `status.builder.camera`: `fit`, the world `rect` the canvas shows, `scale` (bitmap px per world px), `zoom` (times FIT), `fit_scale`, `device_scale` (device px per world px - whole blocks on the glass when it is a multiple of 0.5), `cell_mm` (a cell's width on a touch screen's glass, what the paint threshold reads) and the canvas `area` (bitmap px).",
        params: Params::of::<BuilderCameraParams>(),
        read_only: false,
        destructive: false,
        in_build: true,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "builder_touch",
        description: "Drive the builder with touch points frame by frame: pan, pinch, long-press.",
        help: "Drive the builder with raw touch frames, the way a touch screen does - the multi-finger input `--touch-from-mouse` cannot make (src/editor/gesture.rs, docs/large-maps-follow-camera.md section 9): one finger paints once it moves past the touch slop (`builder_touch_slop_pt`; a resting finger paints nothing) and a quick one-finger tap paints a cell; two fingers pan and pinch-zoom about their middle, a second finger landing on a stroke takes it back, and a coarse screen's zoom settles on whole blocks when they part; a two-finger tap undoes and a three-finger tap redoes (`builder_tap_seconds`). Where a cell is under `builder_paint_min_cell_mm` on the glass (`status.builder.camera.cell_mm`) a one-finger tap zooms in to `builder_tap_zoom_cell_mm` instead and a drag pans - except where the view shows the whole map (FIT), where a drag paints. `frames` is a list of frames, each the touch points down that frame as {id, x, y} in window coordinates, as for `click` (`status.builder.buttons` and `status.camera.window_field` give them); a frame with no points lifts every finger, and the tool lifts every finger at its end - unless `hold` is true, which leaves the last frame's fingers down: the window's frames keep them there (a stroke goes on painting under a still finger, the loupe stands over it - `status.builder.loupe`) until the next `builder_touch` carries on from them or lifts them. Each frame is `dt` seconds (default 1/60). Build mode only. Fingers on the glass turn the hints to taps (`status.ui.hints`). Replies like `mode`, with `camera` (as `status.builder.camera`) and `held` (how many fingers are left down).",
        params: Params::of::<BuilderTouchParams>(),
        read_only: false,
        destructive: false,
        in_build: true,
        online: false,
        present: Present::Json,
    },
    ToolSpec {
        name: "builder_select",
        description: "Drive the builder's select tool: set a rect, move it, copy, cut, paste, flip, stamp.",
        help: "The builder's select tool (docs/large-maps-patterns.md, \"Selection, copy and stamps\") as a drag and the strip under the top plates drive it: `rect` [col0, row0, col1, row1] selects those cells (two opposite corners, inclusive; the part on the field) and takes the select tool, as a drag across them does; `clear` lets the selection go and takes a paste ghost away; `move_by` [dx, dy] carries the selection's cells that many cells, kept on the field, as a drag from inside it does; `action` acts as the strip's buttons and the keys do - copy, cut (a copy and a delete), delete, flip_h, flip_v (mirror the selection in place - or the paste ghost while one stands), stamp (keep the selection, trimmed to its cells, as a stamp for this session: `builder_stamp`), paste (the clipboard as the paste ghost with its middle on `at` [col, row], else in the middle of the view), place (put the ghost down where it stands, or with its top-left on `at`), cancel (take the ghost away). Every move, flip, cut, delete and place is one undo step. A clip is transparent: its empty cells leave the map as it was. A start, player 2's start or a bong moves with its cells; a paste puts one down only where the map holds none of it by then - never two. Replies with the `selection` and the `ghost` ({col, row, cols, rows}, null for none), the `clipboard` ({cols, rows, cells}, null), what the call `changes`d (as `builder_paint`) and `undo_depth`.",
        params: Params::of::<BuilderSelectParams>(),
        read_only: false,
        destructive: false,
        in_build: true,
        online: false,
        present: Present::Json,
    },
    ToolSpec {
        name: "builder_stamp",
        description: "List the builder's stamps, or pick one as a ghost and place it.",
        help: "The builder's STAMPS list: without `name`, every stamp it offers - the shipped ones (maps/stamps/: fort, bunker, river-bend) then the ones kept this session (`saved_N`, `builder_select {action: stamp}`) - each with its `key`, its `name` in the language on screen, its size in cells and how many cells it holds. With `name` (a key): that stamp becomes the paste ghost in the middle of the view, with the select tool, as a press on its row does; `at` [col, row] stands the ghost's top-left there instead, and `place: true` puts it down at once (one undo step). Replies with the `stamps`, the `ghost` and, with `place`, the `changes`.",
        params: Params::of::<BuilderStampParams>(),
        read_only: false,
        destructive: false,
        in_build: true,
        online: false,
        present: Present::Json,
    },
    ToolSpec {
        name: "builder_save",
        description: "Save the builder's map to a file (FILE > SAVE).",
        help: "FILE > SAVE / SAVE AS: write the builder's map to maps/<name>.toml (native only; `name` defaults to the map's own name and becomes it) and make the saved state the baseline, so `dirty` clears. Letters, digits, - and _ only. Replies like `builder_map`.",
        params: Params::of::<BuilderSaveParams>(),
        read_only: false,
        destructive: true,
        in_build: true,
        online: false,
        present: Present::Json,
    },
    ToolSpec {
        name: "click",
        description: "One mouse press (optionally a drag, or a tap) at a window position, through the game's own hit tests.",
        help: "A raw press at a window position - the window's own coordinates, which `status.ui.buttons` and `status.camera.window_field` give (with no window, the live mode's bitmap at its own size: the builder's in a window 40 px taller than its canvas, play's field alone) - in either mode, on the same hit-tests a mouse or a finger uses: in play mode the corners' buttons (BUILD, the pause button left of it, the players button, ONLINE, RESTART on a keyboard-less build, and the level button on a level - `status.ui.buttons`), either dialog's buttons (a press outside a dialog closes it), a level's end-screen buttons (LEVELS, PLAY AGAIN, the way on) and the level select's tiles and BACK (a press outside it closes it) - `status.ui.screen_buttons` - and a press on the field itself does nothing in play mode; the lobby's buttons (`status.ui.screen_buttons`); online the corners' LEAVE; in build mode the bar's buttons (PLAY starts the round like `play`), an open popup's rows, steppers and pager - `status.builder.buttons` - or a field cell. With `drag_to`, a press, a straight drag to that point and a release, crossing every cell on the way. Replies like `mode`, with `world` - the world point the press landed on, through the camera - for a press on the field outside the builder. With `touch`, the press is a tap: the hints turn to taps, as a finger landing turns them (`status.ui.hints`). This tests the UI; `build`/`play`/`builder_*` address the model directly.",
        params: Params::of::<ClickParams>(),
        read_only: false,
        destructive: false,
        in_build: true,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "key",
        description: "Press one key, or type text, through the game's own key handling.",
        help: "Press one key for one frame: tab (BUILD/PLAY - in play mode it opens the leave dialog, or closes an open one; in build mode it starts the round like `play`), escape (keep playing / close a dialog or popup; in play mode with no dialog, open or close the level select), enter (leave the round; in the players dialog, switch to the other count; on a level's end screen, the way on or PLAY AGAIN; in the level select, start the level under the focus; confirm a popup), left / right / up / down (move the level select's focus over the open levels; in the builder, pan the canvas for one frame the way the arrow points), 1 / 2 (answer the players dialog), undo, redo (Ctrl+Z / Ctrl+Y in the builder), copy, cut, paste (Ctrl+C / Ctrl+X / Ctrl+V in the builder: the selection copied or cut, the clipboard pasted as a ghost in the middle of the view - `builder_select {action: paste, at}` stands it elsewhere), delete (the selection's cells cleared; so does backspace outside the Save prompt; in the builder escape also takes a paste ghost away, then the selection), zoom_in / zoom_out (`+` / `-` in the builder: one zoom step about the canvas's middle), backspace; `text` types characters into an open builder prompt. A key press turns the hints back to the keys (`status.ui.hints`). Replies like `mode` (`levels_open`, `levels_focus`).",
        params: Params::of::<KeyParams>(),
        read_only: false,
        destructive: false,
        in_build: true,
        online: true,
        present: Present::Json,
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
        description: "The room server as a whole: rooms held and the cap, draining, uptime, protocol version, tick counters. Cheap; call first.",
        help: "The server as a whole: how many rooms it holds and its cap, whether it is draining, its uptime, the build's `protocol_version`, and the counters `/metrics` publishes (rooms by phase, seats, tick p50/p99 in microseconds, tick overruns, snapshot bytes/s, reconnects, intent starvations). Cheap; call first.",
        params: Params::of::<NoParams>(),
        read_only: true,
        destructive: false,
        in_build: true,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "rooms",
        description: "Every room the server holds: code, phase, seats, tick.",
        help: "Every room this server holds: `code`, `phase` (waiting|playing|paused|ended), the seat count and how many are connected, the round's `tick`, the map, the seed and how long the room has been alive. The `code` is what every other tool here takes.",
        params: Params::of::<NoParams>(),
        read_only: true,
        destructive: false,
        in_build: true,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "room",
        description: "One room in detail: its seats, each seat's mailbox, the round's status.",
        help: "One room in full: its lifecycle and how long until the TTL that would end it, the map, seed, mission and resolved spawn plan, the round's tick and outcome, the `tuning_patch` it is being fought under and every `Welcome` carries (a room of two or more scales the waves, a room of one is the empty patch, and `room_tuning`'s rows - `dev_tuning` - go on top), and a row per seat - nick, whether it holds the room, ready, connected, its chassis, and its mailbox. `owns_hull` and `pose_refusals` say whether the seat's client owns its hull (stage 3) and how many of its poses the validator refused. **The mailbox row is the one to read when inputs feel lost**: a `depth` pinned at `BUFFER_MAX` means the client is running ahead and the oldest intents are being dropped, while a climbing `starvations` means it is not stamping far enough ahead and the tick is repeating its last intent.",
        params: Params::of::<room::CodeParams>(),
        read_only: true,
        destructive: false,
        in_build: true,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "room_open",
        description: "Open a room with no client and start its round: map, mission, seed, seats, tuning.",
        help: "Open a room with no client at all and start its round, so a scenario needs no window and no socket: `seats` bot seats are taken (1..=8, each with a mailbox `seat_intent` drives), and the round begins at once rather than waiting for a host to press START. Takes the setup a hosting client takes - a shipped map by name or `map_toml` whole, the mission, and a pinned `seed` so the round replays - plus `tuning`, rows of this room's own (`room_tuning`) the round is set up and fought under from its first tick, `Restart` rows included: `{\"enemy_special_weapon_chance\": 1, \"enemy_special_weapon_emp_share\": 1}` starts it with every enemy carrying an EMP. A bad row refuses the whole call. Replies with the code.",
        params: Params::of::<room::RoomOpenParams>(),
        read_only: false,
        destructive: false,
        in_build: true,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "room_step",
        description: "Freeze a room and advance it `ticks` ticks at 1/60 s. Replies with the events; `snapshot: true` adds the world.",
        help: "Freeze a room's round and advance exactly `ticks` at the fixed 1/60 s timestep, the way the game's `step` drives the local round - so an online round replays deterministically from a pinned seed instead of racing the wall clock. The room stops ticking on real time until `room_resume`; connected clients still get their snapshots, so a window watching it simply sees the round advance in steps. Replies with the events of the step and, by default, a compact snapshot.",
        params: Params::of::<room::RoomStepParams>(),
        read_only: false,
        destructive: false,
        in_build: true,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "room_resume",
        description: "Let a frozen room run on the wall clock again.",
        help: "Let a frozen room tick on real time again (the opposite of `room_step`).",
        params: Params::of::<room::CodeParams>(),
        read_only: false,
        destructive: false,
        in_build: true,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "seat_intent",
        description: "Post a seat's input for `ticks` ticks, the way that seat's client would.",
        help: "Post an intent into one seat's mailbox for the next `ticks` ticks, exactly as that seat's client would - so a bot seat can be driven, or a real seat's input stood in for. `move_dir`/`face`/`fire` are the human-settable fields the wire carries; nothing else travels. A shell fires once per press and a held trigger can never re-arm it, so set `fire_every=N` to tap every N ticks rather than holding. Posted at the client tick that seat's mailbox is expecting, so the server's `acked` and a client's own replay stay in step.",
        params: Params::of::<room::SeatIntentParams>(),
        read_only: false,
        destructive: false,
        in_build: true,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "room_set_tank",
        description: "Patch a tank in a room's round: damage, ammo, special weapon charges, shield, timers.",
        help: "The game's `set_tank` on the room's authoritative round: overwrite a tank's damage (0 = pristine, 100 = wreck), ammo counts (a special weapon's stock above 0 arms it in place of the one special the tank carries, as its crate would - `sonic_ammo`, `emp_charges`, `gauss_slugs`, `fpv_drones`, `rods`, `wells`, `grenade_ammo`, `missile_ammo`, `plasma_ammo`, `laser_charges`, `minigun_ammo`, `flame_fuel` in seconds), `disabled`/`special_offline` seconds, `charge` (seconds a gauss rail's trigger has been held), `shield_hp`, the speed-boost timer and `portal_cooldown`. `slot` is an owner slot: the seats first (0..seats-1), enemies after, so it arms a seat's hands or an enemy's. Omitted fields are untouched. It reaches every client in the next snapshot - the seat's own window predicts and draws its presses with the weapon from then on. Replies with the tank's row of `room_snapshot`.",
        params: Params::of::<room::RoomSetTankParams>(),
        read_only: false,
        destructive: false,
        in_build: true,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "room_spawn_pickup",
        description: "Drop a supply crate of `kind` at (x, y) in a room's round.",
        help: "The game's `spawn_pickup` on the room's authoritative round: a crate of `kind` (a map's pickup spelling: health, ammo, laser, minigun, plasma, missiles, speedup, shield, flamethrower, bong_health, tower_pack, heat_shield, grenades, sonic_hammer, emp_burst, gauss_rail, fpv_swarm, rod_from_god, gravity_well, air_heli, air_jet) down at the map cell nearest (x, y), in its air drop, for a seat or an enemy to drive over. Not a slot: it never respawns. Refused outside the field, on a solid tile and where a pickup already stands. Draws no RNG. Returns the crate's position.",
        params: Params::of::<room::RoomSpawnPickupParams>(),
        read_only: false,
        destructive: false,
        in_build: true,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "room_tuning",
        description: "Read, patch or reset a room's own tuning rows.",
        help: "This room's own tuning rows, which no other room on the server sees: the room is set up and ticked under the server's table, its seat-count patch and these on top, and every `Welcome` carries them, so a window seated in it predicts and draws with the same numbers. Without `patch` or `reset` it reports `rows` (this room's) and `tuning_patch` (the whole patch its round is fought under). `patch` ({knob: value}, array knobs as `name.label` or the whole array; range-checked, refused whole on any bad row) adds rows, `reset: true` drops them all first. A change in a round is in force from the next tick, each row where the game next reads it - a `live` row at once, a tank's kit (the enemies' special weapons) when the next tank spawns, the wave plan at the next round, so a round's setup goes through `room_open {tuning}` - and every connected window is welcomed again on the live world to take it, its seat and mailbox kept.",
        params: Params::of::<room::RoomTuningParams>(),
        read_only: false,
        destructive: false,
        in_build: true,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "room_snapshot",
        description: "A room's authoritative world state, as `snapshot` gives the window's.",
        help: "The room's authoritative world as JSON - the reading the game's `snapshot` gives, on the round the server is simulating rather than on a client's replica: every tank with position, rotation, velocity, health, ammo, weapon and (for enemies) role and target, plus projectiles, pickups, bongs, portals and the engagement rings, and while there are any `aircraft` (air support in the air: `seat`, `sortie`, `stage`, `x`/`y`, `height`, `heading_deg`, `speed`, `bank`, `ammo`) and `air_calls` (`seat`, `sortie`, `seconds_left` - null while waiting). `detail=full` adds each enemy's AI memory. This is the truth a replica is checked against.",
        params: Params::of::<room::RoomSnapshotParams>(),
        read_only: true,
        destructive: false,
        in_build: true,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "room_events",
        description: "A room's simulation events after the cursor `since`, filtered by `kinds`/`exclude`.",
        help: "The room's gameplay events since `since` (0 = everything kept): fired, hit, wreck, ram, pickups, obstacle_destroyed, teleported, round_started, round_ended and the rest - the vocabulary the game's `events` speaks, recorded on the authoritative round. `kinds` keeps only those names, `exclude` drops them. **The way to tell an input that never arrived from a shot the server refused**: no `fired` for a seat that pulled the trigger is the first, a `fired` with nothing after it the second.",
        params: Params::of::<room::RoomEventsParams>(),
        read_only: true,
        destructive: false,
        in_build: true,
        online: true,
        present: Present::Json,
    },
    ToolSpec {
        name: "room_close",
        description: "Close a room and drop its round.",
        help: "End a room now: its round stops, its seats are let go and the code stops resolving. For clearing up after a scenario; a room `room_open` made also ages out on its own TTL.",
        params: Params::of::<room::CodeParams>(),
        read_only: false,
        destructive: true,
        in_build: true,
        online: true,
        present: Present::Json,
    },
];
