//! `bbmcp` - the MCP adapter for the game's dev server (src/devserver.rs,
//! docs/dev-server-design.md). Claude Code launches it from `.mcp.json`
//! as a stdio MCP server; every `tools/call` becomes one request line to
//! the running game on `127.0.0.1:$BONGBONG_DEV_PORT` (default 4747) and
//! the reply comes back as the tool result. The tool list is
//! `bongbong::devserver::TOOLS`, so the game and the adapter can't drift.
//!
//! Also a CLI: `bbmcp call <tool> ['{json params}'] [--raw]` prints one
//! result, for shell scripts and `just mcp-call`.
//!
//! Only stdout carries protocol frames; everything else goes to stderr.
//! Native only - the wasm dev build compiles every bin, hence the stub.

#[cfg(target_os = "emscripten")]
fn main() {}

#[cfg(not(target_os = "emscripten"))]
fn main() {
    native::main();
}

#[cfg(not(target_os = "emscripten"))]
mod native {
    use std::io::{self, BufRead, BufReader, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::time::Duration;

    use bongbong::devserver::{DEFAULT_PORT, REPLY_TIMEOUT, ROOM_TOOLS, ROOMS_DEV_PORT, TOOLS, ToolSpec};
    use serde_json::{Value, json};

    /// Which dev server this adapter is pointed at. One binary, one
    /// protocol, two tool tables: the game's window (`just run-dev`) and
    /// the room server (`just run-server --dev-tools`). They are separate
    /// MCP entries rather than one merged table so the names cannot
    /// collide and both can be attached at once - a co-op bug is usually
    /// read from both ends at the same time.
    struct Target {
        /// The MCP `serverInfo.name`, and the `mcp__<name>__*` prefix.
        server_name: &'static str,
        default_port: u16,
        /// The environment variable that moves the port.
        port_var: &'static str,
        tools: &'static [ToolSpec],
        instructions: &'static str,
        /// What to tell the model when nothing is listening.
        start_hint: &'static str,
    }

    const GAME: &Target = &Target {
        server_name: "bongbong",
        default_port: DEFAULT_PORT,
        port_var: "BONGBONG_DEV_PORT",
        tools: TOOLS,
        instructions: GAME_INSTRUCTIONS,
        start_hint: "start it with `just run-dev` (cargo run --features dev-tools)",
    };

    const ROOMS: &Target = &Target {
        server_name: "bongbong-rooms",
        default_port: ROOMS_DEV_PORT,
        port_var: "BONGBONG_ROOMS_DEV_PORT",
        tools: ROOM_TOOLS,
        instructions: ROOM_INSTRUCTIONS,
        start_hint: "start it with `just run-server-dev` (cargo run -p bongbong-server --features dev-tools)",
    };

    const ROOM_INSTRUCTIONS: &str = "Tools drive the online co-op room server running with `just run-server-dev` (--features dev-tools). This is the authoritative side: the rooms here own the `Game` a round is actually simulated in, while the game's own MCP server (`mcp__bongbong__*`) drives one window, which in an online round holds a replica it never simulates. Call `server_status` first, then `rooms` for the codes every other tool takes. `room_open` makes a room with no client and starts it, `seat_intent` posts a seat's input the way that seat's client would, and `room_step` freezes the room and advances it deterministically at 1/60 s a tick - so a co-op scenario replays from a pinned seed with no window, no browser and no second machine. `room` carries each seat's mailbox (depth, acked, starvations): read it when inputs feel lost. Shells are edge-triggered - one shell per press, a held trigger never re-arms them - so tap with `fire_every`. Owner slots: the seats first (0..seats-1), enemies after. Positions are field pixels, y down, rotation 0 = up.";

    /// The MCP protocol revisions this adapter speaks, newest first. An
    /// `initialize` naming one of them gets it back; anything else gets
    /// the newest, which is what the spec says a server should offer.
    const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];
    const GAME_INSTRUCTIONS: &str = "Tools drive the bongbong game running with `just run-dev` (native, --features dev-tools). Call `status` (or the cheaper `mode`) first: \
the window is in play mode (the round) or build mode (the map builder), and the tools that read or drive the round refuse in build mode until `play`. \
`step` freezes the game in lockstep and advances it deterministically at 1/60 s per frame; `resume` lets it run in real time. \
`screenshot` returns the last rendered frame (the state after the latest step). Owner slots: 0 = player, enemies from 1; in a two-player round (`players {count: 2}`) slot 1 is player 2 and enemies count from 2. \
Positions are field pixels (the map's own `size` in 32 px cells; the shipped default is 34 x 17 = 1088x544), y down, rotation 0 = up.";

    pub fn main() {
        let mut args: Vec<String> = std::env::args().skip(1).collect();
        // `bbmcp rooms ...` points the whole adapter at the room server;
        // everything after it reads exactly as it does for the game.
        let target = match args.first().map(String::as_str) {
            Some("rooms") => {
                args.remove(0);
                ROOMS
            }
            _ => GAME,
        };
        let port = std::env::var(target.port_var)
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(target.default_port);
        match args.first().map(String::as_str) {
            None => serve_stdio(target, port),
            Some("call") => {
                let code = cli_call(target, port, &args[1..]);
                std::process::exit(code);
            }
            Some("-h" | "--help") => {
                eprintln!("{}", usage(target));
            }
            Some(other) => {
                eprintln!("bbmcp: unknown argument {other:?}\n{}", usage(target));
                std::process::exit(2);
            }
        }
    }

    fn usage(target: &Target) -> String {
        let tools: Vec<&str> = target.tools.iter().map(|t| t.name).collect();
        let (var, port) = (target.port_var, target.default_port);
        format!(
            "usage:\n  bbmcp [rooms]                          serve MCP over stdio (what .mcp.json runs)\n  bbmcp [rooms] call <tool> [json] [--raw]   call one tool\n\n`rooms` targets the room server instead of the game window.\n{var} overrides the port (default {port}).\ntools: {}",
            tools.join(", ")
        )
    }

    /// One request line to the game, one reply line back.
    fn call_game(target: &Target, port: u16, method: &str, params: &Value) -> Result<Value, String> {
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(1)).map_err(|e| {
            let (what, hint) = (target.server_name, target.start_hint);
            format!("{what} not reachable on 127.0.0.1:{port} ({e}) - {hint}")
        })?;
        // Outlast the game's own reply timeout so its message wins.
        stream.set_read_timeout(Some(REPLY_TIMEOUT + Duration::from_secs(10))).map_err(|e| e.to_string())?;
        let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
        let request = json!({ "id": 1, "method": method, "params": params });
        writeln!(writer, "{request}").map_err(|e| e.to_string())?;
        writer.flush().map_err(|e| e.to_string())?;
        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).map_err(|e| format!("reading the game's reply: {e}"))?;
        if line.trim().is_empty() {
            return Err("the game closed the connection without replying".to_string());
        }
        let reply: Value = serde_json::from_str(&line).map_err(|e| format!("bad reply from the game: {e}"))?;
        if let Some(err) = reply.get("error") {
            return Err(err.as_str().map(str::to_string).unwrap_or_else(|| err.to_string()));
        }
        Ok(reply.get("result").cloned().unwrap_or(Value::Null))
    }

    fn cli_call(target: &Target, port: u16, args: &[String]) -> i32 {
        let raw = args.iter().any(|a| a == "--raw");
        let args: Vec<&String> = args.iter().filter(|a| *a != "--raw").collect();
        let Some(tool) = args.first() else {
            eprintln!("bbmcp call: missing tool name\n{}", usage(target));
            return 2;
        };
        let params: Value = match args.get(1) {
            None => json!({}),
            Some(text) => match serde_json::from_str(text) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("bbmcp call: params are not JSON: {e}");
                    return 2;
                }
            },
        };
        match call_game(target, port, tool, &params) {
            Ok(mut result) => {
                if tool.as_str() == "nav_grid"
                    && let Some(grid) = result.get("grid").and_then(Value::as_str)
                {
                    return print_out(grid);
                }
                if !raw && let Some(obj) = result.as_object_mut() {
                    obj.remove("png_base64");
                }
                print_out(&serde_json::to_string_pretty(&result).unwrap_or_default())
            }
            Err(e) => {
                eprintln!("bbmcp call {tool}: {e}");
                1
            }
        }
    }

    /// Print a CLI result; a closed pipe (`| head`) is not an error.
    fn print_out(text: &str) -> i32 {
        let mut out = io::stdout().lock();
        let _ = writeln!(out, "{text}");
        let _ = out.flush();
        0
    }

    fn tools_json(target: &Target) -> Value {
        Value::Array(
            target
                .tools
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "inputSchema": serde_json::from_str::<Value>(t.schema).expect("every tool table's schemas are valid JSON (unit-tested)"),
                        "annotations": t.annotations(),
                    })
                })
                .collect(),
        )
    }

    /// The protocol revision to answer `initialize` with - see `PROTOCOL_VERSIONS`.
    fn negotiate_version(requested: Option<&str>) -> &'static str {
        PROTOCOL_VERSIONS.iter().copied().find(|v| Some(*v) == requested).unwrap_or(PROTOCOL_VERSIONS[0])
    }

    fn text_content(text: String, is_error: bool) -> Value {
        json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
    }

    /// How a tool call reaches the game: `(method, params)` in, the reply's
    /// `result` out, or the error message the model should read. The stdio
    /// server uses `call_game`; the tests substitute a closure.
    type GameCall<'a> = &'a dyn Fn(&str, &Value) -> Result<Value, String>;

    /// Run one MCP `tools/call`; failures become `isError` results (the
    /// model sees the message) rather than JSON-RPC errors.
    fn tool_call(target: &Target, call: GameCall, params: &Value) -> Value {
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
        if !target.tools.iter().any(|t| t.name == name) {
            return text_content(format!("unknown tool {name:?}"), true);
        }
        match call(name, &arguments) {
            Err(e) => text_content(e, true),
            Ok(result) => match name {
                "screenshot" => {
                    let data = result.get("png_base64").and_then(Value::as_str).unwrap_or("");
                    let text = format!(
                        "saved {} ({}x{}, frame {})",
                        result["path"].as_str().unwrap_or("?"),
                        result["width"],
                        result["height"],
                        result["frame"]
                    );
                    json!({ "content": [
                        { "type": "image", "data": data, "mimeType": "image/png" },
                        { "type": "text", "text": text },
                    ] })
                }
                "nav_grid" => text_content(result["grid"].as_str().unwrap_or("").to_string(), false),
                _ => text_content(serde_json::to_string_pretty(&result).unwrap_or_default(), false),
            },
        }
    }

    /// One JSON-RPC 2.0 line in, at most one frame out: `None` for a
    /// notification (no id) or a response (no method), which get no reply;
    /// a parse error answers with id null, as the spec says.
    fn handle_line(target: &Target, line: &str, call: GameCall) -> Option<Value> {
        let msg: Value = match serde_json::from_str(line) {
            Ok(v) => v,
            Err(e) => {
                return Some(json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("parse error: {e}") } }));
            }
        };
        let (Some(id), Some(method)) = (msg.get("id").cloned(), msg.get("method").and_then(Value::as_str)) else {
            return None;
        };
        let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
        let result: Result<Value, (i64, String)> = match method {
            "initialize" => Ok(json!({
                "protocolVersion": negotiate_version(params.get("protocolVersion").and_then(Value::as_str)),
                "capabilities": { "tools": {} },
                "serverInfo": { "name": target.server_name, "version": env!("CARGO_PKG_VERSION") },
                "instructions": target.instructions,
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools_json(target) })),
            "tools/call" => Ok(tool_call(target, call, &params)),
            other => Err((-32601, format!("method not found: {other}"))),
        };
        Some(match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
        })
    }

    /// Newline-delimited JSON-RPC 2.0 on stdin/stdout until stdin closes.
    fn serve_stdio(target: &Target, port: u16) {
        let stdin = io::stdin();
        let stdout = io::stdout();
        let mut out = stdout.lock();
        let call = |method: &str, params: &Value| call_game(target, port, method, params);
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            let Some(frame) = handle_line(target, &line, &call) else { continue };
            if writeln!(out, "{frame}").and_then(|()| out.flush()).is_err() {
                break;
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// A game that answers every call with `reply`.
        fn game(reply: Result<Value, String>) -> impl Fn(&str, &Value) -> Result<Value, String> {
            move |_, _| reply.clone()
        }

        fn frame(line: &str, call: GameCall) -> Value {
            handle_line(GAME, line, call).expect("a request gets a frame")
        }

        #[test]
        fn initialize_negotiates_a_version_this_adapter_speaks() {
            let ok = game(Ok(json!({})));
            let f = frame(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#, &ok);
            assert_eq!(f["result"]["protocolVersion"], "2025-03-26", "{f}");
            assert_eq!(f["result"]["serverInfo"]["name"], GAME.server_name);
            assert!(f["result"]["instructions"].as_str().unwrap().contains("status"));
            assert!(f["result"]["capabilities"]["tools"].is_object());
            let f = frame(r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#, &ok);
            assert_eq!(f["result"]["protocolVersion"], PROTOCOL_VERSIONS[0], "an unknown revision gets the newest");
            let f = frame(r#"{"jsonrpc":"2.0","id":3,"method":"initialize"}"#, &ok);
            assert_eq!(f["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);
            assert_eq!(f["id"], 3);
        }

        #[test]
        fn tools_list_carries_every_tool_with_its_schema_and_annotations() {
            let f = frame(r#"{"jsonrpc":"2.0","id":"a","method":"tools/list"}"#, &game(Ok(json!({}))));
            let tools = f["result"]["tools"].as_array().unwrap();
            assert_eq!(tools.len(), TOOLS.len());
            for (t, spec) in tools.iter().zip(TOOLS) {
                assert_eq!(t["name"], spec.name);
                assert_eq!(t["inputSchema"]["type"], "object", "{}", spec.name);
                assert!(t["annotations"]["readOnlyHint"].is_boolean(), "{}", spec.name);
                assert!(t["annotations"]["destructiveHint"].is_boolean(), "{}", spec.name);
            }
            let status = tools.iter().find(|t| t["name"] == "status").unwrap();
            assert_eq!(status["annotations"]["readOnlyHint"], true);
            assert_eq!(status["annotations"]["destructiveHint"], false);
            let save = tools.iter().find(|t| t["name"] == "builder_save").unwrap();
            assert_eq!(save["annotations"]["destructiveHint"], true);
        }

        #[test]
        fn tool_call_results_are_content_blocks_and_failures_are_is_error() {
            let f = frame(r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"status","arguments":{}}}"#, &game(Ok(json!({ "frame": 7 }))));
            let content = &f["result"]["content"][0];
            assert_eq!(content["type"], "text");
            assert!(content["text"].as_str().unwrap().contains("\"frame\": 7"), "{f}");
            assert_eq!(f["result"]["isError"], false);
            // An unreachable game: the message names the fix, as an isError result.
            let down = game(Err("game not reachable - start it with `just run-dev`".to_string()));
            let f = frame(r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"status"}}"#, &down);
            assert_eq!(f["result"]["isError"], true, "{f}");
            assert!(f["result"]["content"][0]["text"].as_str().unwrap().contains("just run-dev"));
            // A tool the game does not have never reaches it.
            let f = frame(r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"explode"}}"#, &game(Ok(json!({}))));
            assert_eq!(f["result"]["isError"], true);
            assert!(f["result"]["content"][0]["text"].as_str().unwrap().contains("unknown tool"));
            // A screenshot is an image block plus a caption.
            let shot = game(Ok(json!({ "png_base64": "Zm9v", "path": "target/devshots/1.png", "width": 544, "height": 288, "frame": 9 })));
            let f = frame(r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"screenshot"}}"#, &shot);
            assert_eq!(f["result"]["content"][0]["type"], "image");
            assert_eq!(f["result"]["content"][0]["data"], "Zm9v");
            assert_eq!(f["result"]["content"][0]["mimeType"], "image/png");
            assert!(f["result"]["content"][1]["text"].as_str().unwrap().contains("544x288"));
            // The nav grid is the bare text.
            let grid = game(Ok(json!({ "grid": "#.#\n" })));
            let f = frame(r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"nav_grid"}}"#, &grid);
            assert_eq!(f["result"]["content"][0]["text"], "#.#\n");
        }

        #[test]
        fn notifications_get_no_frame_and_bad_input_gets_a_json_rpc_error() {
            let ok = game(Ok(json!({})));
            assert!(handle_line(GAME, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#, &ok).is_none());
            assert!(handle_line(GAME, r#"{"jsonrpc":"2.0","id":1,"result":{}}"#, &ok).is_none(), "a response is not answered");
            let f = frame("{not json", &ok);
            assert_eq!(f["error"]["code"], -32700);
            assert!(f["id"].is_null());
            let f = frame(r#"{"jsonrpc":"2.0","id":9,"method":"resources/list"}"#, &ok);
            assert_eq!(f["error"]["code"], -32601);
            assert_eq!(f["id"], 9);
            let f = frame(r#"{"jsonrpc":"2.0","id":10,"method":"ping"}"#, &ok);
            assert!(f["result"].is_object());
        }

        #[test]
        fn a_closed_port_is_reported_as_the_thing_not_running() {
            // Port 1 has nothing listening on loopback; the refusal is immediate.
            let err = call_game(GAME, 1, "status", &json!({})).unwrap_err();
            assert!(err.contains("just run-dev"), "{err}");
            // And the room server's refusal names its own recipe, not the
            // game's - the two are started differently.
            let err = call_game(ROOMS, 1, "rooms", &json!({})).unwrap_err();
            assert!(err.contains("just run-server-dev"), "{err}");
            assert!(err.contains("bongbong-rooms"), "{err}");
        }

        /// One adapter, two tables: `bbmcp rooms` has to advertise the
        /// room server's tools and *only* those, or a model would call a
        /// game tool the room server has never heard of.
        #[test]
        fn the_rooms_target_advertises_the_room_tools_alone() {
            let ok = game(Ok(json!({})));
            let f = handle_line(ROOMS, r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#, &ok)
                .expect("a request gets a frame");
            let tools = f["result"]["tools"].as_array().unwrap();
            assert_eq!(tools.len(), ROOM_TOOLS.len());
            let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
            assert!(names.contains(&"room_open"), "{names:?}");
            assert!(!names.contains(&"screenshot"), "a game tool leaked into the rooms table: {names:?}");
            // And a game tool called against the rooms target is refused
            // here rather than sent to a server that cannot answer it.
            let refused = tool_call(ROOMS, &ok, &json!({"name": "screenshot", "arguments": {}}));
            assert_eq!(refused["isError"], true, "{refused}");
        }
    }
}
