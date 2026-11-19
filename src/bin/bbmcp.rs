//! `bbmcp` - the MCP adapter for the dev tools (src/devserver.rs,
//! docs/dev-server-design.md). Claude Code launches it from `.mcp.json`
//! (through `tools/dev/bbmcp.sh`, which starts the built binary) as a stdio
//! MCP server. The tool list is `bongbong::devserver::TOOLS`, so the game
//! and the adapter can't drift.
//!
//! A game tool is answered by one of two targets. A **window** - the game
//! running with `just run-dev` - gets each call as one request line on
//! `127.0.0.1:$BONGBONG_DEV_PORT` (default 4747). With no window from this
//! checkout listening there, the adapter hosts a **headless** round itself
//! (`devserver::Headless`): the same tools on a private round, no window
//! and no port. `attach` reports and switches the target. `bbmcp rooms`
//! is the room server's adapter instead, which only ever forwards.
//!
//! The adapter answers a few tools itself: `help` (a tool's long form),
//! `attach`, `batch` (several calls in one), `reload` (rebuild and restart
//! in place) and `probe`.
//!
//! Also a CLI: `bbmcp call <tool> ['{json params}'] [--raw]` prints one
//! result, for shell scripts and `just mcp-call`; `bbmcp tools` prints the
//! MCP tool list.
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
    use std::io::{self, BufRead, BufReader, Read, Write};
    use std::net::{SocketAddr, TcpStream};
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant, SystemTime};

    use bongbong::devserver::{
        ATTACH, AttachParams, BATCH, BatchParams, DEFAULT_PORT, HELP, Headless, PROBE, Present, ProbeParams, RELOAD, REPLY_TIMEOUT, ROOM_TOOLS,
        ROOMS_DEV_PORT, TOOLS, ToolSpec, help, parse,
    };
    use serde_json::{Value, json};

    /// Which dev server this adapter is for. One binary, one protocol, two
    /// tool tables: the game's (`just run-dev`, or a headless round) and
    /// the room server's (`just run-server-dev`). They are separate MCP
    /// entries rather than one merged table so the names cannot collide
    /// and both can be attached at once - a co-op bug is usually read from
    /// both ends at the same time.
    struct Target {
        /// The MCP `serverInfo.name`, and the `mcp__<name>__*` prefix.
        server_name: &'static str,
        default_port: u16,
        /// The environment variable that moves the port.
        port_var: &'static str,
        tools: &'static [ToolSpec],
        /// The tools the adapter answers itself for this target.
        own: &'static [&'static ToolSpec],
        /// Whether the adapter may host a headless round for it.
        headless: bool,
        instructions: &'static str,
        /// What to tell the model when nothing is listening.
        start_hint: &'static str,
    }

    const GAME: &Target = &Target {
        server_name: "bongbong",
        default_port: DEFAULT_PORT,
        port_var: "BONGBONG_DEV_PORT",
        tools: TOOLS,
        own: &[&HELP, &ATTACH, &BATCH, &RELOAD, &PROBE],
        headless: true,
        instructions: GAME_INSTRUCTIONS,
        start_hint: "start it with `just run-dev` (cargo run --features dev-tools), or `attach {\"target\": \"headless\"}` for a round with no window",
    };

    const ROOMS: &Target = &Target {
        server_name: "bongbong-rooms",
        default_port: ROOMS_DEV_PORT,
        port_var: "BONGBONG_ROOMS_DEV_PORT",
        tools: ROOM_TOOLS,
        own: &[&HELP, &BATCH, &RELOAD],
        headless: false,
        instructions: ROOM_INSTRUCTIONS,
        start_hint: "start it with `just run-server-dev` (cargo run -p bongbong-server --features dev-tools)",
    };

    const ROOM_INSTRUCTIONS: &str = "Tools drive the online co-op room server running with `just run-server-dev` (--features dev-tools). This is the authoritative side: the rooms here own the `Game` a round is actually simulated in, while the game's own MCP server (`mcp__bongbong__*`) drives one window, which in an online round holds a replica it never simulates. Call `server_status` first, then `rooms` for the codes every other tool takes. `room_open` makes a room with no client and starts it, `seat_intent` posts a seat's input the way that seat's client would, and `room_step` freezes the room and advances it deterministically at 1/60 s a tick - so a co-op scenario replays from a pinned seed with no window, no browser and no second machine. `room` carries each seat's mailbox (depth, acked, starvations): read it when inputs feel lost. Shells are edge-triggered - one shell per press, a held trigger never re-arms them - so tap with `fire_every`. Owner slots: the seats first (0..seats-1), enemies after. Positions are field pixels, y down, rotation 0 = up. The tool list carries a summary of each tool: `help {tool}` has its reply's shape. A parameter a tool does not take is refused by name.";

    /// The MCP protocol revisions this adapter speaks, newest first. An
    /// `initialize` naming one of them gets it back; anything else gets
    /// the newest, which is what the spec says a server should offer.
    const PROTOCOL_VERSIONS: &[&str] = &["2025-06-18", "2025-03-26", "2024-11-05"];
    const GAME_INSTRUCTIONS: &str = "Tools drive a round of bongbong. The target is a window (the game running with `just run-dev`) when one started from this checkout is listening, and otherwise a headless round the adapter hosts itself: private, deterministic, no window - `attach` reports which and switches. \
Call `mode` first: the game is in play mode (the round) or build mode (the map builder), and the tools that read or drive the round refuse in build mode until `play`. `status` is the round in more detail, and its `sections` add the heavy parts. \
`step` freezes the game in lockstep and advances it deterministically at 1/60 s per frame (`until` stops it at an event); `resume` lets a window run in real time again. A headless round moves only by `step`. \
`screenshot` returns the last rendered frame of a window, or the field painted on the CPU for a headless round. `batch` runs a whole scenario in one call. \
The tool list carries a summary of each tool: `help {tool}` has its reply's shape and edge cases. A parameter a tool does not take is refused by name. \
Owner slots: 0 = player, enemies from 1; in a two-player round (`players {count: 2}`) slot 1 is player 2 and enemies count from 2. \
Positions are field pixels (the map's own `size` in 32 px cells; the shipped default is 34 x 17 = 1088x544), y down, rotation 0 = up.";

    /// The adapter's own build: what `reload` runs and what the launcher
    /// (`tools/dev/bbmcp.sh`) builds when no binary exists.
    const BUILD_ARGS: &[&str] = &["build", "--no-default-features", "--features", "dev-tools", "--bin", "bbmcp"];
    /// The binary that build writes, relative to the working directory.
    const BUILT_EXE: &str = "target/debug/bbmcp";
    /// Set across `reload`'s restart to the JSON-RPC id of the call the new
    /// process has to answer.
    const RESUME_VAR: &str = "BBMCP_RELOAD_ID";
    /// `BBMCP_TARGET`: `headless` or `window` pins the target; unset is the
    /// default described under `attach`.
    const TARGET_VAR: &str = "BBMCP_TARGET";
    /// How long a batch may run from a shell before `probe` gives up, by
    /// default.
    const PROBE_TIMEOUT: Duration = Duration::from_secs(300);
    /// `ANOMALY` lines one `probe` reply carries at most.
    const PROBE_ANOMALY_CAP: usize = 200;

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
        let port = std::env::var(target.port_var).ok().and_then(|p| p.parse().ok()).unwrap_or(target.default_port);
        let mut adapter = Adapter::new(target, port);
        match args.first().map(String::as_str) {
            None => serve_stdio(&mut adapter),
            Some("call") => {
                let code = cli_call(&mut adapter, &args[1..]);
                std::process::exit(code);
            }
            Some("tools") => {
                print_out(&serde_json::to_string_pretty(&tools_json(target)).unwrap_or_default());
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
        let tools: Vec<&str> = all_tools(target).iter().map(|t| t.name).collect();
        let (var, port) = (target.port_var, target.default_port);
        format!(
            "usage:\n  bbmcp [rooms]                          serve MCP over stdio (what .mcp.json runs)\n  bbmcp [rooms] call <tool> [json] [--raw]   call one tool\n  bbmcp [rooms] tools                    print the MCP tool list\n\n`rooms` targets the room server instead of the game.\n{var} overrides the port (default {port}); {TARGET_VAR}=headless|window pins the game's target.\ntools: {}",
            tools.join(", ")
        )
    }

    /// Every tool this target lists: the adapter's own first, then the
    /// server's table.
    fn all_tools(target: &Target) -> Vec<&'static ToolSpec> {
        target.own.iter().copied().chain(target.tools.iter()).collect()
    }

    /// What a game tool is answered by.
    enum Backend {
        /// Not yet settled: the first call settles it (`Adapter::settle`).
        Unsettled,
        /// The game on this loopback port.
        Window(u16),
        /// The adapter's own round.
        Headless(Box<Headless>),
    }

    /// How a call reaches a window or the room server: `(port, method,
    /// params)` in, the reply's `result` out, or the error message the
    /// model should read. The tests substitute a closure.
    type Wire = Box<dyn Fn(u16, &str, &Value) -> Result<Value, String>>;

    struct Adapter {
        target: &'static Target,
        port: u16,
        backend: Backend,
        wire: Wire,
    }

    impl Adapter {
        fn new(target: &'static Target, port: u16) -> Adapter {
            Adapter { target, port, backend: Backend::Unsettled, wire: Box::new(move |port, method, params| call_port(target, port, method, params)) }
        }

        /// What listens on `port`, as its `mode.host` (older builds have
        /// none), or `None` when nothing answers.
        fn window_host(&self, port: u16) -> Option<Value> {
            (self.wire)(port, "mode", &json!({})).ok().map(|mode| mode.get("host").cloned().unwrap_or(Value::Null))
        }

        /// Whether the game on the dev port was started from this
        /// checkout: its working directory is the adapter's.
        fn window_is_ours(host: &Value) -> bool {
            let here = std::env::current_dir().ok().and_then(|dir| dir.canonicalize().ok());
            let there = host.get("cwd").and_then(Value::as_str).and_then(|dir| Path::new(dir).canonicalize().ok());
            here.is_some() && here == there
        }

        /// Settle an unsettled target: `BBMCP_TARGET` if it pins one;
        /// otherwise this checkout's window when one listens, else a
        /// headless round. The room server's is always its port.
        fn settle(&mut self) -> Result<(), String> {
            if !matches!(self.backend, Backend::Unsettled) {
                return Ok(());
            }
            let pinned = std::env::var(TARGET_VAR).ok();
            self.backend = match (self.target.headless, pinned.as_deref()) {
                (false, _) | (true, Some("window")) => Backend::Window(self.port),
                (true, Some("headless")) => Backend::Headless(Box::new(Headless::new()?)),
                (true, Some(other)) => return Err(format!("{TARGET_VAR} must be headless or window, got {other:?}")),
                (true, None) => match self.window_host(self.port) {
                    Some(host) if Self::window_is_ours(&host) => Backend::Window(self.port),
                    _ => Backend::Headless(Box::new(Headless::new()?)),
                },
            };
            Ok(())
        }

        /// One call to the server's own table, on the settled target.
        fn forward(&mut self, method: &str, params: &Value) -> Result<Value, String> {
            self.settle()?;
            match &mut self.backend {
                Backend::Window(port) => (self.wire)(*port, method, params),
                Backend::Headless(round) => round.call(method, params),
                Backend::Unsettled => unreachable!("settled above"),
            }
        }

        /// One tool call - the adapter's own or the server's - as its
        /// reply value.
        fn call(&mut self, name: &str, params: &Value) -> Result<Value, String> {
            let own = |spec: &ToolSpec| self.target.own.iter().any(|t| t.name == spec.name) && spec.name == name;
            if own(&HELP) {
                return help(&all_tools(self.target), params);
            }
            if own(&ATTACH) {
                ATTACH.check(params)?;
                return self.attach(&parse(params)?);
            }
            if own(&BATCH) {
                BATCH.check(params)?;
                return self.batch(&parse(params)?);
            }
            if own(&PROBE) {
                PROBE.check(params)?;
                return probe(&parse(params)?);
            }
            if own(&RELOAD) {
                return Err("reload restarts the adapter, so it runs only as an MCP call of its own - not from the CLI and not inside a batch".to_string());
            }
            if !self.target.tools.iter().any(|t| t.name == name) {
                return Err(format!("unknown tool {name:?}; `help` lists them"));
            }
            self.forward(name, params)
        }

        /// `attach`: switch the target if asked, then report it.
        fn attach(&mut self, asked: &AttachParams) -> Result<Value, String> {
            match asked.target.as_deref() {
                None if asked.port.is_some() => return Err("attach: `port` is the window's - give target: \"window\" with it".to_string()),
                None => self.settle()?,
                Some("headless") => {
                    if !matches!(self.backend, Backend::Headless(_)) {
                        self.backend = Backend::Headless(Box::new(Headless::new()?));
                    }
                }
                Some("window") => {
                    let port = asked.port.unwrap_or(self.port);
                    // Refuse rather than leave the tools pointed at nothing.
                    (self.wire)(port, "mode", &json!({}))?;
                    self.backend = Backend::Window(port);
                }
                Some(other) => return Err(format!("attach: target must be headless or window, got {other:?}")),
            }
            let (kind, port) = match &self.backend {
                Backend::Window(port) => ("window", Some(*port)),
                _ => ("headless", None),
            };
            let host = self.forward("mode", &json!({})).ok().and_then(|mode| mode.get("host").cloned());
            let window = self.window_host(self.port).map(|host| {
                json!({ "port": self.port, "cwd": host.get("cwd"), "pid": host.get("pid"), "same_checkout": Self::window_is_ours(&host) })
            });
            Ok(json!({
                "target": kind,
                "port": port,
                "host": host,
                "window": window,
                "adapter": adapter_json(),
            }))
        }

        /// `batch`: the calls in order, each reply or error kept.
        fn batch(&mut self, asked: &BatchParams) -> Result<Value, String> {
            let keep_going = asked.keep_going.unwrap_or(false);
            let mut results = Vec::new();
            let mut failed = 0;
            for call in &asked.calls {
                let params = call.params.clone().unwrap_or_else(|| json!({}));
                let answer = if call.tool == BATCH.name {
                    Err("batch cannot be nested".to_string())
                } else {
                    self.call(&call.tool, &params)
                };
                match answer {
                    Ok(mut result) => {
                        // An image has no place inside a JSON reply: its
                        // file's path is there.
                        if let Some(object) = result.as_object_mut() {
                            object.remove("png_base64");
                        }
                        results.push(json!({ "tool": call.tool, "result": result }));
                    }
                    Err(error) => {
                        failed += 1;
                        results.push(json!({ "tool": call.tool, "error": error }));
                        if !keep_going {
                            break;
                        }
                    }
                }
            }
            Ok(json!({ "completed": results.len() - failed, "failed": failed, "asked": asked.calls.len(), "results": results }))
        }
    }

    /// `attach.adapter`: the adapter's own binary, and whether the source
    /// it was built from has moved on.
    fn adapter_json() -> Value {
        let exe = std::env::current_exe().ok();
        json!({
            "version": env!("CARGO_PKG_VERSION"),
            "exe": exe.as_ref().map(|p| p.display().to_string()),
            "cwd": std::env::current_dir().ok().map(|dir| dir.display().to_string()),
            "stale": exe.as_deref().and_then(stale),
        })
    }

    /// Whether anything the adapter is built from is newer than `exe`:
    /// `None` when that cannot be told (not run from the repository).
    fn stale(exe: &Path) -> Option<bool> {
        let built = std::fs::metadata(exe).and_then(|m| m.modified()).ok()?;
        if !Path::new("Cargo.toml").is_file() || !Path::new("src").is_dir() {
            return None;
        }
        Some(["Cargo.toml", "Cargo.lock", "levels.toml", "src", "maps", "lang"].iter().any(|root| newer_under(Path::new(root), built)))
    }

    /// Whether `path` - a file, or any file under a directory - was
    /// modified after `than`.
    fn newer_under(path: &Path, than: SystemTime) -> bool {
        let Ok(meta) = std::fs::metadata(path) else { return false };
        if meta.is_file() {
            return meta.modified().is_ok_and(|at| at > than);
        }
        std::fs::read_dir(path).is_ok_and(|entries| entries.flatten().any(|entry| newer_under(&entry.path(), than)))
    }

    /// `probe`: run the probe binary and gather what it found.
    fn probe(asked: &ProbeParams) -> Result<Value, String> {
        let mut args: Vec<String> = ["run", "-q", "--no-default-features", "--features", "dev-tools", "--bin", "probe", "--", "--scenario"].map(String::from).to_vec();
        args.push(asked.scenario.clone());
        args.extend(asked.args.iter().flatten().cloned());
        let timeout = asked.timeout_seconds.map(Duration::from_secs).unwrap_or(PROBE_TIMEOUT);
        let command = format!("cargo {}", args.join(" "));
        let mut child = Command::new("cargo")
            .args(&args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("probe: could not run `{command}`: {e}"))?;
        // Drain both pipes on their own threads, so a chatty probe never
        // blocks on a full one while this waits.
        let drain = |mut pipe: Box<dyn Read + Send>| {
            std::thread::spawn(move || {
                let mut text = String::new();
                let _ = pipe.read_to_string(&mut text);
                text
            })
        };
        let out = drain(Box::new(child.stdout.take().expect("piped above")));
        let err = drain(Box::new(child.stderr.take().expect("piped above")));
        let started = Instant::now();
        let status = loop {
            match child.try_wait().map_err(|e| e.to_string())? {
                Some(status) => break Some(status),
                None if started.elapsed() > timeout => {
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
                None => std::thread::sleep(Duration::from_millis(50)),
            }
        };
        let (out, err) = (out.join().unwrap_or_default(), err.join().unwrap_or_default());
        let anomalies: Vec<&str> = out.lines().chain(err.lines()).filter(|line| line.starts_with("ANOMALY")).collect();
        let tail = |text: &str, n: usize| -> Vec<String> {
            let lines: Vec<&str> = text.lines().filter(|line| !line.starts_with("ANOMALY")).collect();
            lines[lines.len().saturating_sub(n)..].iter().map(|line| line.to_string()).collect()
        };
        Ok(json!({
            "ok": status.is_some_and(|s| s.success()),
            "exit_code": status.and_then(|s| s.code()),
            "timed_out": status.is_none(),
            "anomaly_count": anomalies.len(),
            "anomalies": anomalies.iter().take(PROBE_ANOMALY_CAP).collect::<Vec<_>>(),
            "tail": tail(&out, 25),
            "stderr_tail": tail(&err, 15),
            "command": command,
        }))
    }

    /// One request line to a dev server on loopback, one reply line back.
    fn call_port(target: &Target, port: u16, method: &str, params: &Value) -> Result<Value, String> {
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

    fn cli_call(adapter: &mut Adapter, args: &[String]) -> i32 {
        let raw = args.iter().any(|a| a == "--raw");
        let args: Vec<&String> = args.iter().filter(|a| *a != "--raw").collect();
        let Some(tool) = args.first() else {
            eprintln!("bbmcp call: missing tool name\n{}", usage(adapter.target));
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
        let answer = adapter.call(tool, &params);
        // A shell call that fell to a headless round met a fresh one,
        // which is rarely what a script stepping a game meant.
        if matches!(adapter.backend, Backend::Headless(_)) {
            eprintln!("bbmcp: no game from this checkout on 127.0.0.1:{} - answered by a fresh headless round", adapter.port);
        }
        match answer {
            Ok(mut result) => {
                let present = all_tools(adapter.target).iter().find(|t| t.name == tool.as_str()).map_or(Present::Json, |t| t.present);
                if let Present::Text(field) = present
                    && let Some(text) = result.get(field).and_then(Value::as_str)
                {
                    return print_out(text);
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
            all_tools(target)
                .iter()
                .map(|t| {
                    json!({
                        "name": t.name,
                        "description": t.description,
                        "inputSchema": t.schema(),
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

    /// A tool's reply as MCP content, the way its row says to present it.
    /// JSON goes out compact: a model reads it as well as indented, for
    /// about a third fewer tokens.
    fn present(spec: &ToolSpec, result: &Value) -> Value {
        match spec.present {
            Present::Image => {
                let data = result.get("png_base64").and_then(Value::as_str).unwrap_or("");
                let mut text = format!(
                    "saved {} ({}x{}, frame {})",
                    result["path"].as_str().unwrap_or("?"),
                    result["width"],
                    result["height"],
                    result["frame"]
                );
                if let Some(note) = result.get("note").and_then(Value::as_str) {
                    text.push_str(" - ");
                    text.push_str(note);
                }
                json!({ "content": [
                    { "type": "image", "data": data, "mimeType": "image/png" },
                    { "type": "text", "text": text },
                ] })
            }
            Present::Text(field) => text_content(result[field].as_str().unwrap_or("").to_string(), false),
            Present::Json => text_content(result.to_string(), false),
        }
    }

    /// Run one MCP `tools/call`; failures become `isError` results (the
    /// model sees the message) rather than JSON-RPC errors.
    fn tool_call(adapter: &mut Adapter, params: &Value) -> Value {
        let name = params.get("name").and_then(Value::as_str).unwrap_or("");
        let arguments = params.get("arguments").cloned().unwrap_or_else(|| json!({}));
        let Some(spec) = all_tools(adapter.target).into_iter().find(|t| t.name == name) else {
            return text_content(format!("unknown tool {name:?}; `help` lists them"), true);
        };
        match adapter.call(name, &arguments) {
            Err(e) => text_content(e, true),
            Ok(result) => present(spec, &result),
        }
    }

    /// One JSON-RPC 2.0 line in, at most one frame out: `None` for a
    /// notification (no id) or a response (no method), which get no reply;
    /// a parse error answers with id null, as the spec says.
    fn handle_line(adapter: &mut Adapter, line: &str) -> Option<Value> {
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
                "capabilities": { "tools": { "listChanged": true } },
                "serverInfo": { "name": adapter.target.server_name, "version": env!("CARGO_PKG_VERSION") },
                "instructions": instructions(adapter.target),
            })),
            "ping" => Ok(json!({})),
            "tools/list" => Ok(json!({ "tools": tools_json(adapter.target) })),
            "tools/call" if params.get("name").and_then(Value::as_str) == Some(RELOAD.name) => Ok(reload(&id)),
            "tools/call" => Ok(tool_call(adapter, &params)),
            other => Err((-32601, format!("method not found: {other}"))),
        };
        Some(match result {
            Ok(result) => json!({ "jsonrpc": "2.0", "id": id, "result": result }),
            Err((code, message)) => json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } }),
        })
    }

    /// The target's instructions, with a line on top when the adapter is
    /// older than the source it would run a headless round from.
    fn instructions(target: &Target) -> String {
        let is_stale = std::env::current_exe().ok().as_deref().and_then(stale).unwrap_or(false);
        if is_stale {
            format!("NOTE: this adapter's binary is older than the source in its checkout, so its tool list and any headless round run old code - call `reload` to rebuild and restart it. {}", target.instructions)
        } else {
            target.instructions.to_string()
        }
    }

    /// `reload`: build the adapter, then become the new binary, which
    /// answers this call (`resume`). Returns only when it did not happen:
    /// the reply says why.
    fn reload(id: &Value) -> Value {
        let built = Command::new("cargo").args(BUILD_ARGS).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped()).output();
        let output = match built {
            Ok(output) => output,
            Err(e) => return text_content(format!("reload: could not run cargo: {e}"), true),
        };
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            let lines: Vec<&str> = stderr.lines().collect();
            let tail = lines[lines.len().saturating_sub(40)..].join("\n");
            return text_content(format!("reload: the build failed, so the adapter is unchanged.\n{tail}"), true);
        }
        text_content(format!("reload: {}", restart(id)), true)
    }

    /// Replace this process with the freshly built adapter, same
    /// arguments. Returns only on failure, with the reason.
    #[cfg(unix)]
    fn restart(id: &Value) -> String {
        use std::os::unix::process::CommandExt;
        let exe = PathBuf::from(BUILT_EXE);
        let error = Command::new(&exe).args(std::env::args().skip(1)).env(RESUME_VAR, id.to_string()).exec();
        format!("built, but could not start {}: {error}", exe.display())
    }

    #[cfg(not(unix))]
    fn restart(_id: &Value) -> String {
        "built; restart the MCP server to run it (this platform cannot replace a running process)".to_string()
    }

    /// The frames a process started by `reload` owes: the answer to the
    /// call that restarted it, and the notice that the tool list may have
    /// changed.
    fn resume() -> Vec<Value> {
        let Ok(id) = std::env::var(RESUME_VAR) else { return Vec::new() };
        // SAFETY: the adapter is single-threaded here - nothing else reads
        // the environment while it is changed.
        unsafe { std::env::remove_var(RESUME_VAR) };
        let Ok(id) = serde_json::from_str::<Value>(&id) else { return Vec::new() };
        let reply = json!({
            "reloaded": true,
            "version": env!("CARGO_PKG_VERSION"),
            "exe": std::env::current_exe().ok().map(|p| p.display().to_string()),
            "note": "the adapter restarted: a headless round starts over, and the tool list may have changed",
        });
        vec![
            json!({ "jsonrpc": "2.0", "id": id, "result": text_content(reply.to_string(), false) }),
            json!({ "jsonrpc": "2.0", "method": "notifications/tools/list_changed" }),
        ]
    }

    /// Newline-delimited JSON-RPC 2.0 on stdin/stdout until stdin closes.
    fn serve_stdio(adapter: &mut Adapter) {
        let stdin = io::stdin();
        let stdout = io::stdout();
        let mut out = stdout.lock();
        for frame in resume() {
            if writeln!(out, "{frame}").and_then(|()| out.flush()).is_err() {
                return;
            }
        }
        for line in stdin.lock().lines() {
            let Ok(line) = line else { break };
            if line.trim().is_empty() {
                continue;
            }
            let Some(frame) = handle_line(adapter, &line) else { continue };
            if writeln!(out, "{frame}").and_then(|()| out.flush()).is_err() {
                break;
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        /// The game's adapter pointed at a window that answers every call
        /// with `reply` - never a headless round, never a socket.
        fn window(reply: Result<Value, String>) -> Adapter {
            Adapter { target: GAME, port: 1, backend: Backend::Window(1), wire: Box::new(move |_, _, _| reply.clone()) }
        }

        fn frame(adapter: &mut Adapter, line: &str) -> Value {
            handle_line(adapter, line).expect("a request gets a frame")
        }

        fn text(frame: &Value) -> &str {
            frame["result"]["content"][0]["text"].as_str().unwrap_or_else(|| panic!("no text in {frame}"))
        }

        #[test]
        fn initialize_negotiates_a_version_this_adapter_speaks() {
            let mut ok = window(Ok(json!({})));
            let f = frame(&mut ok, r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#);
            assert_eq!(f["result"]["protocolVersion"], "2025-03-26", "{f}");
            assert_eq!(f["result"]["serverInfo"]["name"], GAME.server_name);
            assert!(f["result"]["instructions"].as_str().unwrap().contains("mode"));
            assert_eq!(f["result"]["capabilities"]["tools"]["listChanged"], true, "`reload` changes the list");
            let f = frame(&mut ok, r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"protocolVersion":"1999-01-01"}}"#);
            assert_eq!(f["result"]["protocolVersion"], PROTOCOL_VERSIONS[0], "an unknown revision gets the newest");
            let f = frame(&mut ok, r#"{"jsonrpc":"2.0","id":3,"method":"initialize"}"#);
            assert_eq!(f["result"]["protocolVersion"], PROTOCOL_VERSIONS[0]);
            assert_eq!(f["id"], 3);
        }

        #[test]
        fn tools_list_carries_every_tool_with_its_schema_and_annotations() {
            let f = frame(&mut window(Ok(json!({}))), r#"{"jsonrpc":"2.0","id":"a","method":"tools/list"}"#);
            let tools = f["result"]["tools"].as_array().unwrap();
            assert_eq!(tools.len(), TOOLS.len() + GAME.own.len());
            for t in tools {
                let name = &t["name"];
                assert_eq!(t["inputSchema"]["type"], "object", "{name}");
                assert!(t["annotations"]["readOnlyHint"].is_boolean(), "{name}");
                assert!(t["annotations"]["destructiveHint"].is_boolean(), "{name}");
            }
            let named = |name: &str| tools.iter().find(|t| t["name"] == name).unwrap_or_else(|| panic!("{name} is not listed"));
            assert_eq!(named("status")["annotations"]["readOnlyHint"], true);
            assert_eq!(named("status")["annotations"]["destructiveHint"], false);
            assert_eq!(named("builder_save")["annotations"]["destructiveHint"], true);
            for own in ["help", "attach", "batch", "reload", "probe"] {
                named(own);
            }
        }

        /// The point of the short descriptions: what a client loads to
        /// list the tools stays small, and the long form is one call away.
        #[test]
        fn the_tool_list_is_light_and_help_has_the_long_form() {
            let list = tools_json(GAME).to_string();
            assert!(list.len() < 40_000, "the game's tool list is {} characters", list.len());
            for tool in all_tools(GAME) {
                assert!(tool.description.len() <= 320, "{}: a description is a line or two ({} characters)", tool.name, tool.description.len());
            }
            let mut adapter = window(Ok(json!({})));
            let about = adapter.call("help", &json!({ "tool": "status" })).unwrap();
            assert!(about["help"].as_str().unwrap().len() > 2000, "status's long form documents its reply");
            assert!(about["input_schema"]["properties"]["sections"].is_object(), "{about}");
            let index = adapter.call("help", &json!({})).unwrap();
            assert_eq!(index["tools"].as_array().unwrap().len(), TOOLS.len() + GAME.own.len());
            assert!(adapter.call("help", &json!({ "tool": "explode" })).unwrap_err().contains("no tool"));
        }

        #[test]
        fn tool_call_results_are_content_blocks_and_failures_are_is_error() {
            let call = |reply: Result<Value, String>, line: &str| frame(&mut window(reply), line);
            let f = call(Ok(json!({ "frame": 7 })), r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"mode","arguments":{}}}"#);
            assert_eq!(f["result"]["content"][0]["type"], "text");
            assert_eq!(text(&f), r#"{"frame":7}"#, "JSON goes out compact");
            assert_eq!(f["result"]["isError"], false);
            // An unreachable game: the message names the fix, as an isError result.
            let f = call(
                Err("game not reachable - start it with `just run-dev`".to_string()),
                r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"mode"}}"#,
            );
            assert_eq!(f["result"]["isError"], true, "{f}");
            assert!(text(&f).contains("just run-dev"));
            // A tool the game does not have never reaches it.
            let f = call(Ok(json!({})), r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"explode"}}"#);
            assert_eq!(f["result"]["isError"], true);
            assert!(text(&f).contains("unknown tool"));
            // A screenshot is an image block plus a caption.
            let shot = json!({ "png_base64": "Zm9v", "path": "target/devshots/1.png", "width": 544, "height": 288, "frame": 9 });
            let f = call(Ok(shot), r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"screenshot"}}"#);
            assert_eq!(f["result"]["content"][0]["type"], "image");
            assert_eq!(f["result"]["content"][0]["data"], "Zm9v");
            assert_eq!(f["result"]["content"][0]["mimeType"], "image/png");
            assert!(f["result"]["content"][1]["text"].as_str().unwrap().contains("544x288"));
            // The nav grid is the bare text.
            let f = call(Ok(json!({ "grid": "#.#\n" })), r#"{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"nav_grid"}}"#);
            assert_eq!(text(&f), "#.#\n");
        }

        #[test]
        fn notifications_get_no_frame_and_bad_input_gets_a_json_rpc_error() {
            let mut ok = window(Ok(json!({})));
            assert!(handle_line(&mut ok, r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#).is_none());
            assert!(handle_line(&mut ok, r#"{"jsonrpc":"2.0","id":1,"result":{}}"#).is_none(), "a response is not answered");
            let f = frame(&mut ok, "{not json");
            assert_eq!(f["error"]["code"], -32700);
            assert!(f["id"].is_null());
            let f = frame(&mut ok, r#"{"jsonrpc":"2.0","id":9,"method":"resources/list"}"#);
            assert_eq!(f["error"]["code"], -32601);
            assert_eq!(f["id"], 9);
            let f = frame(&mut ok, r#"{"jsonrpc":"2.0","id":10,"method":"ping"}"#);
            assert!(f["result"].is_object());
        }

        #[test]
        fn a_closed_port_is_reported_as_the_thing_not_running() {
            // Port 1 has nothing listening on loopback; the refusal is immediate.
            let err = call_port(GAME, 1, "status", &json!({})).unwrap_err();
            assert!(err.contains("just run-dev"), "{err}");
            // And the room server's refusal names its own recipe, not the
            // game's - the two are started differently.
            let err = call_port(ROOMS, 1, "rooms", &json!({})).unwrap_err();
            assert!(err.contains("just run-server-dev"), "{err}");
            assert!(err.contains("bongbong-rooms"), "{err}");
        }

        /// One adapter, two tables: `bbmcp rooms` has to advertise the
        /// room server's tools and *only* those, or a model would call a
        /// game tool the room server has never heard of.
        #[test]
        fn the_rooms_target_advertises_the_room_tools_alone() {
            let mut rooms = Adapter { target: ROOMS, port: 1, backend: Backend::Unsettled, wire: Box::new(|_, _, _| Ok(json!({}))) };
            let f = frame(&mut rooms, r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#);
            let tools = f["result"]["tools"].as_array().unwrap();
            assert_eq!(tools.len(), ROOM_TOOLS.len() + ROOMS.own.len());
            let names: Vec<&str> = tools.iter().map(|t| t["name"].as_str().unwrap()).collect();
            assert!(names.contains(&"room_open"), "{names:?}");
            assert!(!names.contains(&"screenshot"), "a game tool leaked into the rooms table: {names:?}");
            assert!(!names.contains(&"attach") && !names.contains(&"probe"), "the room server has no headless round: {names:?}");
            // And a game tool called against the rooms target is refused
            // here rather than sent to a server that cannot answer it.
            let refused = tool_call(&mut rooms, &json!({"name": "screenshot", "arguments": {}}));
            assert_eq!(refused["isError"], true, "{refused}");
            // The room server's adapter only ever forwards.
            rooms.call("rooms", &json!({})).unwrap();
            assert!(matches!(rooms.backend, Backend::Window(1)));
        }

        /// With no window from this checkout, the game's tools are
        /// answered by a round the adapter hosts - and a window that some
        /// other checkout started is left alone.
        #[test]
        fn with_no_window_of_this_checkout_the_adapter_hosts_a_headless_round() {
            let elsewhere = json!({ "mode": "play", "host": { "kind": "window", "pid": 1, "cwd": "/somewhere/else" } });
            let mut adapter = Adapter { target: GAME, port: 1, backend: Backend::Unsettled, wire: Box::new(move |_, _, _| Ok(elsewhere.clone())) };
            let mode = adapter.call("mode", &json!({})).unwrap();
            assert_eq!(mode["host"]["kind"], "headless", "{mode}");
            let at = adapter.call("attach", &json!({})).unwrap();
            assert_eq!(at["target"], "headless", "{at}");
            assert_eq!(at["window"]["same_checkout"], false, "{at}");
            // Asked for by name, that window is driven.
            let at = adapter.call("attach", &json!({ "target": "window" })).unwrap();
            assert_eq!(at["target"], "window", "{at}");
            assert_eq!(adapter.call("mode", &json!({})).unwrap()["host"]["cwd"], "/somewhere/else");
            assert!(adapter.call("attach", &json!({ "target": "moon" })).unwrap_err().contains("headless or window"));
            assert!(adapter.call("attach", &json!({ "targt": "window" })).unwrap_err().contains("unknown parameter"));
        }

        /// A window from this checkout is the one the tools drive.
        #[test]
        fn a_window_of_this_checkout_is_driven() {
            let here = std::env::current_dir().unwrap().display().to_string();
            let ours = json!({ "mode": "play", "host": { "kind": "window", "pid": 1, "cwd": here } });
            let mut adapter = Adapter { target: GAME, port: 1, backend: Backend::Unsettled, wire: Box::new(move |_, _, _| Ok(ours.clone())) };
            assert_eq!(adapter.call("mode", &json!({})).unwrap()["host"]["kind"], "window");
            assert!(matches!(adapter.backend, Backend::Window(1)));
        }

        /// `batch` is a scenario in one call: every reply in order, and a
        /// failure stops it unless told to keep going.
        #[test]
        fn a_batch_runs_a_scenario_on_the_headless_round() {
            let mut adapter = Adapter { target: GAME, port: 1, backend: Backend::Unsettled, wire: Box::new(|_, _, _| Err("nothing listens".to_string())) };
            let calls = json!({ "calls": [
                { "tool": "restart", "params": { "seed": 7, "enemies": 2 } },
                { "tool": "step", "params": { "frames": 30, "move_dir": "up" } },
                { "tool": "teleport", "params": { "slot": 0 } },
                { "tool": "mode" },
            ] });
            let ran = adapter.call("batch", &calls).unwrap();
            assert_eq!(ran["completed"], 2, "{ran}");
            assert_eq!(ran["failed"], 1, "{ran}");
            assert_eq!(ran["results"][1]["result"]["frame"], 30, "{ran}");
            assert!(ran["results"][2]["error"].as_str().unwrap().contains("missing field `x`"), "{ran}");
            assert_eq!(ran["results"].as_array().unwrap().len(), 3, "the batch stopped at the failure");
            let mut all = calls.clone();
            all["keep_going"] = json!(true);
            let ran = adapter.call("batch", &all).unwrap();
            assert_eq!(ran["results"].as_array().unwrap().len(), 4, "{ran}");
            assert_eq!(ran["results"][3]["result"]["mode"], "play", "{ran}");
            let nested = adapter.call("batch", &json!({ "calls": [{ "tool": "batch", "params": { "calls": [] } }] })).unwrap();
            assert!(nested["results"][0]["error"].as_str().unwrap().contains("nested"), "{nested}");
        }
    }
}
