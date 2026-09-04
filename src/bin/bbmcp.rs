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

    use bongbong::devserver::{DEFAULT_PORT, TOOLS};
    use serde_json::{Value, json};

    const SERVER_NAME: &str = "bongbong";
    const INSTRUCTIONS: &str = "Tools drive the bongbong game running with `just run-dev` (native, --features dev-tools). \
`step` freezes the game in lockstep and advances it deterministically at 1/60 s per frame; `resume` lets it run in real time. \
`screenshot` returns the last rendered frame (the state after the latest step). Owner slots: 0 = player, enemies from 1. \
Positions are screen pixels, 1280x720 by default, y down, rotation 0 = up.";

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
        stream.set_read_timeout(Some(Duration::from_secs(120))).map_err(|e| e.to_string())?;
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
                    })
                })
                .collect(),
        )
    }

    fn text_content(text: String, is_error: bool) -> Value {
        json!({ "content": [{ "type": "text", "text": text }], "isError": is_error })
    }

    /// Run one MCP `tools/call`; failures become `isError` results (the
    /// model sees the message) rather than JSON-RPC errors.
    fn tool_call(port: u16, params: &Value) -> Value {
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
        if !TOOLS.iter().any(|t| t.name == name) {
            return text_content(format!("unknown tool {name:?}"), true);
        }
        match call_game(port, name, &arguments) {
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

    /// Newline-delimited JSON-RPC 2.0 on stdin/stdout until stdin closes.
    fn serve_stdio(port: u16) {
        let stdin = io::stdin();
        let stdout = io::stdout();
        let mut out = stdout.lock();
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            let msg: Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(e) => {
                    let _ = writeln!(out, "{}", json!({ "jsonrpc": "2.0", "id": null, "error": { "code": -32700, "message": format!("parse error: {e}") } }));
                    let _ = out.flush();
                    continue;
                }
            };
            // Notifications (no id) and responses (no method) get no reply.
            let (Some(id), Some(method)) = (msg.get("id").cloned(), msg.get("method").and_then(Value::as_str)) else {
                continue;
            };
            let params = msg.get("params").cloned().unwrap_or_else(|| json!({}));
            let result: Result<Value, (i64, String)> = match method {
                "initialize" => Ok(json!({
                    "protocolVersion": params.get("protocolVersion").cloned().unwrap_or_else(|| json!("2025-06-18")),
                    "capabilities": { "tools": {} },
                    "serverInfo": { "name": SERVER_NAME, "version": env!("CARGO_PKG_VERSION") },
                    "instructions": INSTRUCTIONS,
                })),
                "ping" => Ok(json!({})),
                "tools/list" => Ok(json!({ "tools": tools_json() })),
                "tools/call" => Ok(tool_call(port, &params)),
                other => Err((-32601, format!("method not found: {other}"))),
            };
            let frame = match result {
                Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
                Err((code, message)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
            };
            if writeln!(out, "{frame}").and_then(|()| out.flush()).is_err() {
                break;
            }
        }
    }
}
