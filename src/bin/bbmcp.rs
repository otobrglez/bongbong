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

    use bongbong::devserver::{DEFAULT_PORT, REPLY_TIMEOUT, TOOLS};
    use serde_json::{Value, json};

    const SERVER_NAME: &str = "bongbong";
    /// The MCP protocol revisions this adapter speaks, newest first. An
    /// `initialize` naming one of them gets it back; anything else gets
    /// the newest, which is what the spec says a server should offer.
    const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];
    const INSTRUCTIONS: &str = "Tools drive the bongbong game running with `just run-dev` (native, --features dev-tools). Call `status` (or the cheaper `mode`) first: \
the window is in play mode (the round) or build mode (the map builder), and the tools that read or drive the round refuse in build mode until `play`. \
`step` freezes the game in lockstep and advances it deterministically at 1/60 s per frame; `resume` lets it run in real time. \
`screenshot` returns the last rendered frame (the state after the latest step). Owner slots: 0 = player, enemies from 1; in a two-player round (`players {count: 2}`) slot 1 is player 2 and enemies count from 2. \
Positions are field pixels (the map's own `size` in 32 px cells; the shipped default is 34 x 17 = 1088x544), y down, rotation 0 = up.";

    pub fn main() {
        let args: Vec<String> = std::env::args().skip(1).collect();
        let port = std::env::var("BONGBONG_DEV_PORT")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(DEFAULT_PORT);
        match args.first().map(String::as_str) {
            None => serve_stdio(port),
            Some("call") => {
                let code = cli_call(port, &args[1..]);
                std::process::exit(code);
            }
            Some("-h" | "--help") => {
                eprintln!("{}", usage());
            }
            Some(other) => {
                eprintln!("bbmcp: unknown argument {other:?}\n{}", usage());
                std::process::exit(2);
            }
        }
    }

    fn usage() -> String {
        let tools: Vec<&str> = TOOLS.iter().map(|t| t.name).collect();
        format!(
            "usage:\n  bbmcp                      serve MCP over stdio (what .mcp.json runs)\n  bbmcp call <tool> [json] [--raw]   call one tool on the running game\n\nBONGBONG_DEV_PORT overrides the port (default {DEFAULT_PORT}).\ntools: {}",
            tools.join(", ")
        )
    }

    /// One request line to the game, one reply line back.
    fn call_game(port: u16, method: &str, params: &Value) -> Result<Value, String> {
        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let stream = TcpStream::connect_timeout(&addr, Duration::from_secs(1)).map_err(|e| {
            format!("game not reachable on 127.0.0.1:{port} ({e}) - start it with `just run-dev` (cargo run --features dev-tools)")
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

    fn cli_call(port: u16, args: &[String]) -> i32 {
        let raw = args.iter().any(|a| a == "--raw");
        let args: Vec<&String> = args.iter().filter(|a| *a != "--raw").collect();
        let Some(tool) = args.first() else {
            eprintln!("bbmcp call: missing tool name\n{}", usage());
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
        match call_game(port, tool, &params) {
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

    fn tools_json() -> Value {
        Value::Array(
            TOOLS
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "inputSchema": serde_json::from_str::<Value>(t.schema).expect("TOOLS schemas are valid JSON (unit-tested)"),
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
    fn tool_call(call: GameCall, params: &Value) -> Value {
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
        if !TOOLS.iter().any(|t| t.name == name) {
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
    fn handle_line(line: &str, call: GameCall) -> Option<Value> {
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
                "serverInfo": { "name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION") },
                "instructions": INSTRUCTIONS,
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools_json() })),
            "tools/call" => Ok(tool_call(call, &params)),
            other => Err((-32601, format!("method not found: {other}"))),
        };
        Some(match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
        })
    }

    /// Newline-delimited JSON-RPC 2.0 on stdin/stdout until stdin closes.
    fn serve_stdio(port: u16) {
        let stdin = io::stdin();
        let stdout = io::stdout();
        let mut out = stdout.lock();
        let call = |method: &str, params: &Value| call_game(port, method, params);
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            let Some(frame) = handle_line(&line, &call) else { continue };
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
            handle_line(line, call).expect("a request gets a frame")
        }

        #[test]
        fn initialize_negotiates_a_version_this_adapter_speaks() {
            let ok = game(Ok(json!({})));
            let f = frame(r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#, &ok);
            assert_eq!(f["result"]["protocolVersion"], "2025-03-26", "{f}");
            assert_eq!(f["result"]["serverInfo"]["name"], SERVER_NAME);
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
            assert!(handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#, &ok).is_none());
            assert!(handle_line(r#"{"jsonrpc":"2.0","id":1,"result":{}}"#, &ok).is_none(), "a response is not answered");
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
        fn a_closed_port_is_reported_as_the_game_not_running() {
            // Port 1 has nothing listening on loopback; the refusal is immediate.
            let err = call_game(1, "status", &json!({})).unwrap_err();
            assert!(err.contains("just run-dev"), "{err}");
        }
    }
}
