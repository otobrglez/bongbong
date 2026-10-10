# Platforms and shipping reference

The room server, the web, iOS and Android builds and the releases in full. CLAUDE.md keeps the
commands and the gotchas; this is the detail. Update it with the build.

## Room server (`server/`, docs/online-coop-prd.md §4.7-4.10, §4.13)

`bongbong-server` is the online co-op room server: a workspace member on the `bongbong` library with
`default-features = false` (headless - `cargo tree -p bongbong-server -e normal | grep sola` is
empty), tokio + axum, one binary. **One instance holds every room** - no sharding, no routing,
nothing derived from a code (docs/online-coop-prd.md §4.8 keeps the several-instance design as
deferred work). `just run-server` (`--listen 127.0.0.1:4848 --insecure`; `--insecure` only declares
that plain `ws://` is expected and is logged, `--max-rooms` caps the server); `GET /ws`, which
carries every message - there is no HTTP `POST` -, alone on `--listen`; the operator's routes on
**the admin listener** (`--admin-listen`, 127.0.0.1:4850; a deployment opens it to the cluster and
no Ingress routes it): `GET /health` (liveness - 200 whenever the server answers, drain or not,
reading the room list on the way), `GET /ready` (readiness - 503 from the start of the drain) and
`GET /metrics`, Prometheus text: rooms by phase and the `--max-rooms` cap, rooms in play and rounds
started/ended by map (`room::map_label`: a shipped name or `custom`) and mission, clients by build
(`net::wire::ClientInfo` - version, protocol, platform - which every client sends in its
`Create`/`Join`, cleaned by `metrics::ClientLabel::of` and capped at `CLIENT_LABELS_MAX`), seats,
tick p50/p99 µs, snapshot bytes/s, reconnects. `hub.rs` holds the rooms by code (`code.rs`:
`LETTERS` symbols from `ALPHABET`, both re-exported from `net::rooms` so a minted code is always one
the lobby's grid can type; anything else is `Malformed`) and the SIGTERM drain (no new rooms or
rematches; `begin_drain` wakes every room through a `watch` and `room::Lifecycle::drain` closes a
waiting or paused room at once and an ended one after `DRAIN_ENDED_TTL`, so only a round in play is
waited for; exit when the last room ends or after `DRAIN_MAX`; Ctrl-C exits at once). `room.rs` is
one task per room owning a `Game`: `Lifecycle` (waiting → playing at 60 Hz → paused when nobody is
connected → ended; `WAITING_TTL`/`EMPTY_TTL`/`ENDED_TTL`/`ROUND_MAX`/`SEAT_GRACE` are server policy,
not tuning), the tick (each seat's mailbox - `net::mailbox`, the game crate's, shared with the rig -
sampled into `Input::seats`. A server-driven seat's is an **ordered jitter buffer**, oldest first
one per tick, capped at `BUFFER_MAX`, so a burst is spread over the ticks that follow rather than
collapsed to its newest. A client-owned seat's (`IntentMsg::owned`) is **played out on the room's
clock**: a play point moves one client tick per room tick and each read takes every intent at or
before it - pose, stick and view from the newest, the trigger merged press for press so no tap is
lost (shells fire on the edge, so a press whose release hid in an earlier merge is delivered up then
down) - which keeps the room's copy of the hull, the one every other seat draws, on the client's own
track however the packets bunched; a once-a-second controller holds the point a tick after
`PLAYOUT_WIDEN_MISSES` late ticks and moves it two after a window with `PLAYOUT_NARROW_AHEAD` ticks
to spare, and `pose_reach_ticks` - how far the validator believes the pose - is the driving since
the last pose, held to the room's own ticks since it. `press_tick` names the intent a press came on,
which `authority::stamp_presses` writes into `Fired::input_tick`. A starved tick repeats the last
intent (an owned pose dead-reckoned along its velocity for `DEAD_RECKON_TICKS`, then held; its
trigger the client's own last, never a fresh press) and counts a starvation for `/metrics`, a stale
one coasts `INTENT_COAST` then reads as none. `acked_tick` is the tick **applied**, not the newest
received, because that is what a stage-2 client measures its replay from, and `wire_state` - the
depth after the read plus a starved bit - goes out as `Snapshot::mailbox`, the reading the client's
lead is steered by; a packet carrying a pose (`IntentMsg::owned`, stage 3) goes through
`net::authority::take_pose` before the tick, a refusal or a tick that moved the hull past
`PLACED_PX` (`authority::moved_since`) puts a `WireEvent::Placed` on the next snapshot, and the dev
`room` tool shows `owns_hull`/`pose_refusals` per seat), **the end screen** (ticked and sent like
any other frame, then `end_round` on the tick before the one `Game::update` would call `init` on -
`Game::restart_countdown` is the reading - so a room never starts a round nobody asked for), the
snapshots (`net::encode::snapshot` stamped with the room's clock, the events of the two ticks
between banked through `encode::wire_events` and prepended; a delta every `SNAPSHOT_EVERY` ticks
encoded once for everyone, a full one for a seat that just joined or skipped; a `Welcome` in a round
is `encode::welcome` on the live world - holes and all - with the round's tuning patch), the lobby
(`Create`/`Join`/`Ready`/`Start`/`Leave`/`Kick`/`Chat` in,
`RoomCreated`/`Roster`/`Started`/`Ended`/`Said`/`Error` out - the reply variants live in
`net::wire::Lobby`), seats owned by device token for the room's life, never by name (a join under a
name a seat already has is numbered, `distinct_nick`: `oto 2`; a reconnect reclaims the seat with a
fresh `Welcome`, plus an `Ended` where the round is already over; `SEATS_PLAYABLE` is `MAX_SEATS`
and a join past it is refused by name, the round starting with a player per seat up to the highest
occupied one). **Difficulty follows the seat count** (docs/online-coop-prd.md §4.11,
docs/maps-to-levels.md "Difficulty by seat count"): `tuning_patch(seats)` is the round's whole
tuning diff - `wave_size_scale = 1 + (seats - 1) * online_wave_size_per_seat` and `wave_tier_step =
(seats - 1) / online_wave_tier_seats_per_step`, the two dials read from the build's table - and one
seat is the empty patch, so a room of one plays the offline round down to the bytes. `init_under`
puts it on the process-wide table on top of `Tuning::DEFAULT` and calls `Game::init` under
`ROUND_TUNING`, a mutex: both rows are `Restart` rows read only where the spawn plan is resolved, so
the lock has to cover nothing else and a room already playing reads nothing that moves. The same
patch rides every `Welcome`, so a replica resolves the plan its room is fought under; `net::round`
puts the window's own table back when the seat is given up. A `Start` while `Ended` is the rematch:
a fresh `init`, a new seed unless one is pinned, and every seat's ready cleared. **The tick is on
the wall clock** (`net::authority::TickClock`, shared with the rig): each tick is due one period
after the last was *due*, up to `CATCH_UP_TICKS` owed ticks run back to back and more restart the
schedule from now (counted as dropped), the schedule stops while the room is not ticking so a resume
is never paid back as a burst, and the tick arm is polled before commands - with one command served
between back-to-back late ticks, so a flood of lobby or dev traffic can neither delay a tick nor be
starved by an overloaded room. `/metrics` adds tick lateness (p50/p99), late, dropped and overrun
ticks and intent starvations. `conn.rs` is one task per socket with a bounded `Outbox`: a slow
client skips snapshots, never queues them. **Its keep-alive** (`hub::KeepAlive`, set before serving
through `Server::keep_alive`): the writer sends a WebSocket ping every `ping_every` (2 s), which a
browser answers at the protocol level even in a hidden tab, and a connection that sends nothing at
all - not even that pong - for `silent_after` (10 s) is closed, so a seat on a dead path starts its
grace and an empty round pauses instead of holding the seat forever. `cargo test -p bongbong-server`
runs the unit tests and `tests/round.rs`, which binds an ephemeral port, plays a round through two
headless `tokio-tungstenite` clients and measures the cadence, fills a room to its cap, plays a
four-seat round with a replica per client held to the wire, and - through whole
`net::round::OnlineRound`s rather than a bare `RoomClient` - taps the trigger on one seat and on
both seats of a co-op room and counts the shells back (the intersection of `OnlineRound`'s real-time
pacing and the ordered mailbox is the one lane neither `Lockstep` nor `net::rig` covers, since both
call `RoomClient::send_intent` directly and feed the rig's mailbox from the same thread).

**Its dev surface** (`server/src/devserver.rs`, feature `dev-tools`, `just run-server-dev`):
newline-delimited JSON on `127.0.0.1:4849` (`--dev-port`), the same framing and the same
`bongbong::devserver::ToolSpec` rows the game's dev server speaks, so **one adapter drives both** -
`bbmcp` for the window, `bbmcp rooms` for this, registered in `.mcp.json` as
`mcp__bongbong-rooms__*` beside `mcp__bongbong__*`. The *table* is
`bongbong::devserver::ROOM_TOOLS`, in the game crate because the dependency runs the other way (a
table in the server could not be read by a `bbmcp` that lives in the lib); the dispatch is the
server's and `every_advertised_tool_has_an_arm` holds the two together. Tools: `server_status`,
`rooms`, `room` (seats **with their mailboxes** - `depth`, `acked`, `starvations`, the reading that
says whether input was lost on the way in), `room_open` (a room and a started round with no client
at all, `seats` bots, `tuning` rows of its own), `seat_intent` (drive a seat; a standing script fed
**one intent per tick**, because the mailbox is a jitter buffer capped at `BUFFER_MAX` and a batch
posted at once keeps only the last eight), `room_set_tank`/`room_spawn_pickup` (the game's
`set_tank`/`spawn_pickup` on the authority: a weapon in a seat's or an enemy's hands, a crate down),
`room_tuning` (**a room's own rows**: the room keeps `dev_rows`, folded with the seat patch into
`tuning_json` and an `own_table` - the server's table under that patch - and everything the room
does (`handle`, `tick`, `on_deadline`, `dev`) runs under it through `tuning::in_force` (dev-tools
only: a table `tuning()` reads on this thread alone for a synchronous stretch, never across an
`.await`; writes and `current()` stay the live table's), so no other room and not the process's
table see them; a change mid-round welcomes every connected window again on the live world, and the
client puts its own table back before each welcome's patch), `room_step`/`room_resume` (freeze the
room and advance it deterministically, the game's `step` for a room), `room_snapshot`,
`room_events`, `room_close`. Discipline is the game's: a socket task only queues a `Command::Dev`
and the room task answers it between ticks, so nothing reads a world mid-update. **Dev-only by
construction** - the feature is off by default so the release image builds none of it, and the
listener is loopback and never on the axum router. `server/tests/devtools.rs` plays a two-seat round
through it with no client and pins the deterministic replay.

**Shipping it** (`Dockerfile`, `k8s/`, `.github/workflows/deploy-rooms.yml`; docs/online-coop-prd.md
§4.8, k8s/README.md): the pipeline is boo-run's, which is what this Hetzner cluster already runs -
the private registry `registry.folk-decibel.ts.net` and the API server are both on a tailnet, so CI
connects with a `tag:ci` Tailscale OAuth client first, then `docker buildx build --platform
linux/amd64 --push` (tags `latest` and `git describe`; the cleanup after it keeps the newest three
release tags and never touches `latest` or a PR's images), then `kustomize edit set image` +
`kubectl apply -k k8s/base/`. **The trigger is the version tag `cloudflare-deploy.yml` uses**, so
the wasm client and the image ship together - a client and server that disagree about
`net::PROTOCOL_VERSION` refuse each other by name. The image is built on `rust:1.98.1-bookworm` in
cargo-chef stages - the dependencies a layer of their own, rebuilt only when a manifest or the
lockfile changes - then copied onto `gcr.io/distroless/cc-debian12`, ~53 MB. In CI the layers are
kept in GitHub's Actions cache (scope `rooms-server`): ci.yml's `rooms-cache` job writes it on every
push to master, the PR and release builds read it, and `crazy-max/ghaction-github-runtime` gives the
plain `docker buildx build` step the token it needs (without it `type=gha` silently does nothing);
the registry is reached only through a Tailscale relay, too slow to pull a cache from; headlessness
comes from `server/Cargo.toml`'s `default-features = false` rather than a build flag, and the build
fails if `cargo tree` ever finds `sola` in the server's graph. The workload is one Deployment of one
replica in `bongbong-prod` with `strategy: Recreate` (two replicas would take rooms the other cannot
see), `terminationGracePeriodSeconds` 1830 to outlast `DRAIN_MAX`, nginx WebSocket timeouts at an
hour, and no secrets or volumes of its own - though the namespace does need a `regcred` pull secret
on its default ServiceAccount, which is how `boo-prod` does it and why no Deployment here names
`imagePullSecrets`. TLS is Cloudflare's at the edge (the cluster runs no cert-manager), so the
Ingress carries no `tls` block, exactly like boo.run's. **Two things are easy to get wrong**:
liveness is `/health`, never `/ready` - `/ready` answers 503 for the whole drain, and a liveness
probe on it would restart the pod seconds into every deploy (the admin listener stays up through the
drain for the same reason); and there is no CPU *limit*, only a request, because a CFS quota would
throttle the 60 Hz tick at the period boundary and stutter every room together. `just
rooms-image`/`rooms-image-run`/`rooms-push`/`rooms-deploy`/`rooms-manifests`/`rooms-logs` are the
hand paths. **Every open PR also gets its own server** (`.github/workflows/pr-server.yml`,
`k8s/preview/rooms-preview.yaml`): namespace `bongbong-pr-<N>`, reachable at
`wss://rooms.bongbong.io/pr-<N>/ws` - a *path* on production's host, rewritten to `/ws` by the
ingress, so a preview needs no DNS record, no certificate and no client change (`--rooms
wss://rooms.bongbong.io/pr-<N>` already builds that URL). The web preview's sticky comment links it
at this server (`<preview>/?rooms=...`). One namespace holds the lot, so closing the PR deletes it;
previews run at 100m CPU with `--max-rooms 3` and a 30 s grace, since `delete namespace` waits the
grace out. **Monitoring** (k8s/README.md): production and every preview carry a ServiceMonitor, and
the Grafana dashboard "bongbong room servers" is `k8s/base/grafana/bongbong-rooms.json`, shipped as
a `grafana_dashboard` ConfigMap by the kustomization - read-only in the UI, edited as JSON;
`GRAFANA_URL`/`GRAFANA_USER`/`GRAFANA_PASSWORD` in `.envrc` reach its API.

## Web / wasm build

Target `wasm32-unknown-emscripten`, run through `sola-raylib`'s `game_loop::run`
(`../sola-raylib/book/src/web.md`).
- Setup `just setup-web` (emsdk pinned in `tools/setup_emscripten.sh`, which says why - a bump is a
  build plus a play of the page; `cd site && yarn install`). `just build-web` builds the release
  wasm, copies `bongbong.{wasm,js,data}` into `site/public/game/` (gitignored; Astro copies
  `public/` byte-for-byte, which the glue's fixed relative `bongbong.data` path needs) and runs
  `yarn build` → `site/dist/`. `just build-web-dev`/`serve-web-dev` add `--features dev-tools`: the
  page's **Tuning** panel appears, rendered from the exported schema (no site change per knob),
  persisted in `localStorage`, with "Copy JSON" / "Copy as Rust". `serve-web` depends on `build-web`
  and silently drops the feature - use `serve-web-dev` or `just preview-web`. `wrangler.toml`
  deploys `site/dist/`.
- **`game_loop::run`'s fps means different things**: native `SetTargetFPS`; on web any `fps > 0`
  selects a setTimeout driver and only `0` selects rAF (timers are unsynced, throttled and judder),
  so `app.rs` passes 0 on emscripten. Physics runs on its own fixed-dt accumulator, so a lower loop
  rate is safe.
- `.cargo/config.toml` carries the emscripten link flags (GLFW3, memory growth, `--preload-file
  static@/static` so `static/...` paths resolve, **no `-sASYNCIFY`** (it instruments every function
  and halved the web client; docs/online-coop-prd.md decision 19), and **`-sSTACK_SIZE=4194304`**:
  emscripten's default shadow stack is 64 KB, `app::run` alone holds a ~20 KB frame, and an overflow
  is *silent* in a release build - it corrupts the static data just below the stack and surfaces as
  stb_image failing to decode a valid PNG ("IMAGE: Failed to load image data"); dev-tools builds
  link `-sSTACK_OVERFLOW_CHECK=2` through build.rs so an overflow aborts naming the limits instead)
  and an `[env]` block pinning `CC_/CXX_/AR_wasm32_unknown_emscripten` to the real emcc tools -
  **required under nix**, whose global `CC` injects flags emcc rejects. Not in sola-raylib's book.
- **The favicons** (`site/public/favicon.svg`, `favicon.ico`, `apple-touch-icon.png`) are the app
  icon's tank, generated by `tools/web/gen_favicon.py` beside the iOS and macOS icon scripts -
  regenerate them, never edit them by hand.
- **The page's JavaScript lives in `site/src/scripts/`** (entry `main.ts`; `runtime.ts` defines
  `window.Module`, `loading-panel.ts`, `input.ts`, `fullscreen.ts`, `tuning-panel.ts`, `strings.ts` -
  the page's own dozen words in each shipped language, picked by the game's rule (`?lang=`, then
  `navigator.languages`) and written into `data-i18n` elements before the runtime starts, `<html
  lang>` with them; the tuning panel stays English), bundled and minified by Astro - never inline in
  the `.astro` page. The Emscripten glue is a classic script reading the global `Module`, so
  `index.astro` loads it with `defer` after the module script (spec-ordered).
- **The canvas fills its box** (docs/large-maps-follow-camera.md §10): `.stage` takes the space it
  is given at any shape - the column's width and `100dvh` less a 72 px hints strip (none under 540
  px tall) and the safe-area insets; the on-screen viewport inside the safe-area bands in
  `.immersive` and `:fullscreen` - and the canvas fills it exactly; never `aspect-ratio` or
  `object-fit` on it, because raylib maps touches and GLFW clicks by dividing by the CSS box. The
  window is the canvas's drawing buffer: `app::web::follow_canvas` reads the box
  (`emscripten_get_element_css_size`) and `devicePixelRatio` at the top of every frame and
  `SetWindowSize`s the buffer to the box in device pixels (`view::canvas_buffer`, scaled down evenly
  past `CANVAS_MAX_SIDE`, 8192), asking twice because emscripten's GLFW takes any full-screen
  element for its canvas; the window opens at that size and `FLAG_WINDOW_RESIZABLE` stays off on the
  web (it would size the canvas to the tab). The web window is in device pixels like Android's:
  `window_units_per_point` is `web::units_per_point`, `screen()` is the box in CSS pixels, and
  `view_max_scale` caps an arena in points (`max_scale x units`). A phone held upright (portrait,
  under 720 px, `hover: none`, `pointer: coarse`) gets the `.turn` card over the stage while the
  round runs on behind it; real full screen locks landscape where the browser can (`fullscreen.ts`).
  `touch-action: none` because the builder drags. The overlay controls and the loading panel are
  positioned in `.stage`, so they follow it wherever it lands. Full screen has three outcomes:
  `requestFullscreen` where it exists, the `.immersive` class elsewhere, and on iPhone Safari only
  Add to Home Screen removes the URL bar (`apple-mobile-web-app-capable`; not advertised on the
  canvas). The API's rejection is async (refresh the label in the promise handlers) and old WebKit's
  prefixed call returns `undefined`. In `.immersive` and `:fullscreen` the `.game` box is the whole
  viewport with the safe-area bands (`viewport-fit=cover`) as *border-box* padding and the stage
  filling the inside; `.immersive` is placed on the on-screen part of the viewport (`--vv-*`, set on
  the root by `fullscreen.ts` from `window.visualViewport`), not `inset: 0`, because a phone lays a
  fixed box out in the largest viewport, the one with the browser's bars collapsed.
- **The level progress** (docs/levels.md) is the one thing the game keeps in the page: `app.rs`
  reads `localStorage['bongbong.level']` at startup through `page_string` and writes it through
  `emscripten_run_script` the frame a level is won, both wrapped so a storage that throws only
  forgets.
- **What the page publishes on `window`**, the only way the browser tells the game anything the API
  cannot: `app.rs` reads each through `emscripten_run_script*`, and nothing else in the tree does.
  `window.bbShift` (`site/src/scripts/input.ts`), read once a frame by `left_shift_down`, because
  emscripten's GLFW maps both Shift keys to left (`event.code` is what tells them apart, and Left
  Shift is player 2's fire key). `window.bbOverlay` (`site/src/scripts/overlay.ts`: `top x y w h` in
  CSS pixels), read once a frame by `app::web::follow_canvas` into `hud::PageOverlay`: on a touch
  screen (`(hover: none) and (pointer: coarse)`) the page's Full screen and Subscribe controls stand
  in a band along the canvas's top, centred - the one strip a thumb does not reach in a landscape
  grip - and the game keeps its chrome below it as below a safe area (the band is the `UiFrame`'s
  top inset, so the builder's bar and its canvas stand under it too); with a mouse they keep to the
  bottom-left and take no band, and the off-screen arrows keep off their rectangle wherever it is.
  `window.bbInvite`, `window.bbToken` and `window.bbLang` (`site/src/scripts/room.ts`; the last is
  `navigator.languages`, comma-joined), `window.bbMotion` (`site/src/scripts/motion.ts`, from
  `prefers-reduced-motion`, which seeds the motion switch) and `window.bbTouch` (`overlay.ts`, the
  `(hover: none) and (pointer: coarse)` query: a touch screen's page is framed as the app on that
  device), read once at startup by `page_string` - the web build's whole command line, `?lang=` and
  `?weather=` included (`text::lang_from_url`, `weather::weather_from_url`).
- **Online co-op in the browser** (docs/online-coop-prd.md §4.10): `ONLINE_AVAILABLE` is the
  `online` feature, so the bar carries the ONLINE button here too and the lobby hosts and joins over
  `net::web`'s socket. The room comes off the page's own URL: `room.ts` publishes `location.href`
  and `net::rooms::Invite::parse` reads it - `/j/CODE` (the invite link `join_url` writes) or
  `?join=CODE` (a page serving no such route, which is what the Astro preview is), plus the
  `?rooms=` override a dev QR carries, completed to `ws`/`wss` by the page's own scheme. A code
  sends the window straight into the lobby on that room. Identity is `room.ts`'s too: a device id in
  `localStorage` crossed with a tab id in `sessionStorage`, so a reload reclaims the seat and a
  second tab is a second player (`Session::token`; storage that throws falls back to a per-page id).
  **The invite path is `site/public/_redirects`**, one line: the site is static and has no page at
  `/j/CODE`, so `/j/*` is redirected to `/?join=:splat`, which `Invite::parse` reads the same way -
  a test in `net::rooms` reads that file so the rule and `join_url` cannot drift. **An invite points
  back at the page that minted it** (`net::rooms::SiteBase`, carried on `Session::site`): the
  deployed site for a build with no page of its own, otherwise the page's own origin, because a PR
  preview's QR has to reach *that* preview - production is a different build dialling a different
  server, where the code names nothing. The redirect replaces the query rather than merging it,
  which is why `join_url` writes the *query* form directly for an overridden host
  (`?join=CODE&rooms=...`) and keeps the pretty path only for the deployed one. Locally: `just
  run-server`, `just serve-web-dev`, then `http://localhost:4321/?rooms=ws://127.0.0.1:4848` to host
  and `...&join=CODE` in a second tab.
- **Shaders need GLSL ES 100 ports**: `static/web/*.fs` are hand-ported twins of `static/*.fs`
  (`#version 100`, `precision mediump float`, `varying`, `texture2D`, `gl_FragColor`);
  `shader_path()` picks the directory. Change both by hand.
- Audio: none yet. Adding `InitAudioDevice` needs `-sASYNCIFY=1` back in the rustflags, and with it
  the speed cost decision 19 removed.
- **CI**: `.github/actions/build-web/action.yml` is the shared build (version pins as input defaults -
  keep in sync with `devenv.nix` and `tools/setup_emscripten.sh`); it builds `--bin bongbong` alone,
  and ci.yml's `web-cache` job builds it on every push to master, because a run reads only caches
  saved on its own ref or on master - without it a PR's first preview and a release tag compile
  everything. `cloudflare-deploy.yml` deploys production on version tags (the `bongbong` Worker,
  bongbong.io). `pr-preview.yml` deploys every same-repo PR with `--features dev-tools` to an
  ephemeral Worker `bongbong-pr-<N>` on its workers.dev subdomain (`wrangler deploy --name`, no
  custom domain), posts a sticky comment, and deletes the worker on close. **PR previews are the QA
  surface with the tuning panel; production never gets it.** Fork PRs get no secrets by design -
  never "fix" that with `pull_request_target`. The token needs account Workers Scripts:Edit.

## iOS build

Native app on raylib's **SDL backend with SDL3** and **OpenGL ES 2.0**, hand-bundled, no Xcode
project (docs/ios-native-port-prd.md). Every recipe sources `tools/ios/env.sh` (Xcode as
`DEVELOPER_DIR` - the devenv shell points it and `SDKROOT` at nix's apple-sdk -, one deployment
target `IOS_MIN` 15.0 for rustc, cc-rs, cmake and the plist, `IOS_SLICE` sim|ios, the library prefix
`BONGBONG_IOS_LIBS`, bindgen's sysroot, and `tools/ios/bin` first on `PATH`, whose `cc` - rustc's
default linker for the build scripts and proc-macros - is Xcode's clang: with `DEVELOPER_DIR` at
Xcode, nix's wrapper would link them against Xcode's macOS SDK, whose `.tbd` stubs nix's `ld`
cannot read from Xcode 27 on. A `PATH` entry, not `CARGO_TARGET_AARCH64_APPLE_DARWIN_LINKER`:
cargo fingerprints a configured linker, and the desktop build shares a few host units with an iOS
build, so every switch would recompile them and their dependents). `.cargo/config.toml` sets
`linker = "/usr/bin/clang"` and the `CC_/CXX_/AR_` pins for both iOS targets.
- **Setup**: `just ios-setup` (`tools/setup_ios.sh`: static SDL3 at a pinned tag and raylib from the
  tree vendored in the `sola-raylib-sys` registry crate (the version `Cargo.lock` pins) into
  `~/.local/share/bongbong-ios/sim`; SDL3 is built with `-DSDL_CAMERA=OFF -DSDL_HIDAPI=OFF` because
  App Store Connect refuses a binary that references `AVCaptureDevice` or CoreBluetooth without a
  usage string, and the options are part of its install marker so a change rebuilds (the script
  hands cmake Xcode's clang: nix's wrapper, first on the devenv PATH, adds `-mmacos-version-min` and
  fails the iOS compiler check); raylib gets `-DMA_NO_COREAUDIO` because miniaudio's CoreAudio
  backend needs Objective-C - when audio arrives, compile `raudio.c` as ObjC). `just ios-smoke`
  (`tools/ios/smoke.c`, SDL3 + GL ES 2 `glDrawElements`) is the standing check after any
  Xcode/runtime update. Device slice: `just ios-setup-device`.
- **Run**: `just run-ios-sim` (`build-ios-sim` + `tools/ios/bundle.sh` →
  `target/ios-sim/BongBong.app`: binary, `Info.plist` from the template - `UILaunchScreen` required
  or iOS runs it at a compatibility size, landscape only -, `static/` and `maps/` flat since the
  entry chdirs into the bundle), `just ios-screenshot`. `simctl` cannot inject touches - use the
  Simulator window. **iPhone / iPad**: `just run-ios-device` / `just run-ios-ipad` build
  `target/ios-device/BongBong.app`, then `tools/ios/deploy.sh <iPhone|iPad|udid>` picks the *wired*
  device of that type (a phone paired over Wi-Fi never takes an iPad deploy), runs
  `tools/ios/sign.sh` (Xcode's automatic signing on the placeholder project `tools/ios/sign/` mints
  the certificate and a profile listing that device - Personal Team, seven-day profiles; a failed
  Xcode build never falls through to an older stub), installs via `devicectl`, launches with the
  console attached and verifies the round came up (window, render targets, process alive → "DEPLOY
  OK"; refusals name the fix: Trust, Developer Mode, the first run's profile trust under Settings >
  General > VPN & Device Management). devicectl forwards Ctrl-C to the app; `--no-console` detaches.
  A dev-tools build's dev server is reachable from the Mac through `iproxy 4747:4747`
  (libimobiledevice), so the `mcp__bongbong__*` tools and `screenshot` work against the device.
- **TestFlight**: `just ios-testflight` (`tools/ios/testflight.sh`) builds `--release` (not `dist`:
  its LTO drops the `__isPlatformVersionAtLeast` SDL3 links against), stages the bundle, wraps it in
  a hand-made `.xcarchive` and runs `xcodebuild -exportArchive` (method `app-store-connect`,
  automatic cloud-managed signing, `destination upload`) authenticated by an App Store Connect API
  key - `BONGBONG_IOS_TEAM`, `ASC_KEY_ID`, `ASC_ISSUER_ID` in `.envrc`, the `.p8` under
  `~/.appstoreconnect/private_keys/`; `--export` stops at a signed `.ipa`.
  `.github/workflows/testflight.yml` runs the same script on `macos-26` (Xcode 26.3 pinned through
  `BONGBONG_DEVELOPER_DIR`) on the version tag the other release workflows use, by hand, or for a
  same-repo PR labelled `ios` (on the label and every push while it is on; the merge ref the PR's
  previews build, build number `YYYYMMDD.HHMM.<PR>`, a sticky PR comment naming it; closing the PR
  expires its builds, the `expire` job through the App Store Connect API); secrets `ASC_KEY_ID`,
  `ASC_ISSUER_ID`, `ASC_KEY_P8`, SDL3 + raylib cached on the setup script's hash. `bundle.sh` writes
  what an upload is checked for on every slice: the `DT*` toolchain keys, `CFBundleVersion` as a
  build number (`BONGBONG_IOS_BUILD`, else UTC `YYYYMMDD.HHMM`), the icon compiled by `actool` from
  `tools/ios/Assets.xcassets` (`just ios-icon` regenerates it from the tank sheet),
  `PrivacyInfo.xcprivacy` (the required-reason APIs the binary imports - recheck with `nm -u` after
  a new dependency) and `ITSAppUsesNonExemptEncryption` false.
- **Linking**: `Cargo.toml` gives `sola-raylib` `nobuild` + `sdl` for `cfg(target_os = "ios")`
  (resolver 3 scopes it); `build.rs`'s `ios_link` emits `libraylib.a`, `libSDL3.a`, every framework
  from SDL's `sdl3.pc`, `OpenGLES` and `-ObjC`. Never enable `opengl_es_20` (links `-lGLESv2
  -lGLdispatch`, which do not exist on Apple).
- **What differs on iOS** (`app/ios.rs`; `bongbong::EMBEDDED`): `SDL_RunApp` → `app_main` with
  default `Args`; the window is created at the screen's point size, `fullscreen` + `highdpi`. **The
  raylib build carries `tools/ios/raylib-sdl-highdpi.patch`** (SDL backend renders into the full
  high-density drawable and `SetupViewport` projects in points; raylib's `screenScale` matrix was
  tried and breaks the HUD inside render textures). **`route_default_framebuffer`** is real platform
  glue: iOS has no framebuffer 0 - SDL draws through its own FBO/renderbuffer - so glad's
  `glBindFramebuffer`/`glBindRenderbuffer` are wrapped to map 0 onto SDL's objects, or every frame
  after the first render texture goes into the void. `set_hints` sets `SDL_IOS_HIDE_HOME_INDICATOR`
  "2". `app::ios::reduce_motion` (`UIAccessibilityIsReduceMotionEnabled`) seeds the motion switch at
  startup. `target_fps` 0 on a device, 60 on the simulator (`target_abi = "sim"` ignores the swap
  interval). GLSL ES 100 shaders, no map saving, lower particle budget. A dev-tools build logs
  `FrameStats` every five seconds (the only frame-time readout on a phone). App-lifecycle events are
  not handled yet. The simulator's GL is a software renderer - its frame times say nothing about a
  phone.
- **No keyboard** (`KEYBOARD_AVAILABLE` false on iOS/Android): the right corner cluster carries a
  RESTART button (`hud::CornerButton::Restart`, pressed through `tuning::request_restart`) and hides
  the players button; two-player mode is unavailable.

## Android build

raylib's own Android platform: a `NativeActivity` through `android_native_app_glue`, GL ES 2 over
EGL, one shared object, no Java (docs/android-port-prd.md). Every recipe sources
`tools/android/env.sh` (SDK/NDK/JDK paths, API pins, the prebuilt raylib prefix
`BONGBONG_ANDROID_LIBS`, the NDK linker and `CC_`/`AR_` for cc-rs, bindgen's sysroot; cargo-ndk does
not build on the nix toolchain).
- **The `android/` crate** (`crate-type = ["cdylib"]`, `libbongbong_android.so`) exports the C
  `main` raylib's `android_main` calls, which runs `app::android::app_main`. The symbol must be in
  the cdylib's own objects - an rlib-side `#[no_mangle]` is an unreferenced archive member the
  linker drops. The root package stays the default workspace member.
- **Setup**: `just android-setup` (`tools/setup_android.sh`: command-line tools, pinned NDK,
  platform, arm64 image, the `bongbong` AVD with `hw.keyboard=yes`, raylib from the tree vendored in
  the `sola-raylib-sys` registry crate (the version `Cargo.lock` pins), with
  `tools/android/raylib-android-relaunch.patch` (raylib master's fix for a relaunch hang after a
  destroy while paused) on top, built with `-DPLATFORM=Android` into
  `~/.local/share/bongbong-android/<abi>`; `BONGBONG_ANDROID_NO_EMULATOR=1` is the build-machine cut -
  no image, emulator or AVD - and the script runs on Linux too, since `env.sh` picks the NDK's host
  tag, the SDK root and the JDK by OS). `just android-smoke` is the standing check after any
  SDK/NDK/emulator update.
- **Build and run**: `just build-android` (`cargo build --release --target aarch64-linux-android -p
  bongbong-android`, then `tools/android/package.sh`: `aapt2 link` of the manifest template -
  `hasCode="false"`, `android.app.lib_name`, `sensorLandscape` with `appCategory="game"` (which
  keeps that lock on large screens from API 36), the `INTERNET` permission the rooms need -,
  `assets/static/**`, the adaptive launcher icon compiled from `tools/android/res/` (`just
  android-icon` redraws its layers from the tank sheet, `tools/android/gen_app_icon.py`, the iOS
  icon's twin), the `.so` stored page-aligned, `apksigner`; no Gradle, no dex). **Signing**:
  `BONGBONG_ANDROID_KEYSTORE`/`_KEYSTORE_PASS`/`_KEY_ALIAS` name the release key, else the debug
  keystore signs (minted with `keytool` on first use). Android ties an app to its key for life - a
  phone refuses an update signed with another - so the release key is minted once, kept off the repo
  and lives in the secrets the workflow reads; a debug-signed install has to be uninstalled before a
  release one goes on. `just run-android` boots the AVD headless if needed, installs, launches
  `com.otobrglez.bongbong/android.app.NativeActivity` and follows logcat. `just android-screenshot`,
  `android-tap X Y`, `android-swipe` drive it from the shell; `adb shell input keyevent` gives only
  a down+up, fine for R/Space, useless for driving (held keys need `sendevent` or a hand on the
  window).
- **Why `nobuild` again**: `sola-raylib-sys`'s Android cmake branch mis-parses the API level and
  emits no link args. `android/build.rs` emits the prebuilt `libraylib.a`, `log android EGL GLESv2
  OpenSLES c m`, and as `rustc-cdylib-link-arg` **`-Wl,--wrap=fopen`** (raylib routes asset reads
  into the APK through `__wrap_fopen`; without it every `LoadTexture` fails silently),
  **`-Wl,-u,ANativeActivity_onCreate`** (keeps the activity entry from the static archive) and
  `-Wl,-z,max-page-size=16384`.
- **Shipping it** (`.github/workflows/android-release.yml`): runs when the Release workflow finishes
  on a version tag (`workflow_run`, since dist's `gh release create` fails on an existing release,
  so the APK has to go second), builds on `ubuntu-22.04` with the image's SDK plus the pinned NDK
  and platform, signs with the secrets
  `ANDROID_KEYSTORE_B64`/`ANDROID_KEYSTORE_PASS`/`ANDROID_KEY_ALIAS` and attaches
  `bongbong-aarch64-linux-android.apk` and its `.sha256` to the release beside dist's archives, then
  its `release-notes` job adds the row to the notes' download table through
  `tools/release/table_row.sh`, which dist wrote before the APK existed (idempotent; not in dist's
  `sha256.sum`, written before too; the job shares the `release-notes-<tag>` concurrency group with
  macos-release.yml's, since each rewrites the notes whole). A manual run takes a `tag`, and with
  `upload` off keeps the APK as a workflow artifact only; a PR touching the Android build (`pr-gate`
  matches the paths) runs it the same way as a check, debug-signed from a fork; a same-repo PR
  labelled `android` builds on the label and every push, keeps the APK as an unzipped artifact
  `bongbong-pr-<N>.apk` (a GitHub login downloads it) linked from a sticky PR comment, and
  `pr-cleanup` deletes those on close - GitHub dispatches only workflows on the default branch, so
  the PR is how a change to the pipeline itself is tried. arm64 only: another ABI is a raylib build
  per ABI (`ANDROID_ABI`) and one more `lib/<abi>/` in `package.sh`. Mint the key with `keytool
  -genkeypair -keystore bongbong-release.jks -alias bongbong -keyalg RSA -keysize 4096 -validity
  10000` and store `base64 < bongbong-release.jks` as the first secret.
- **What differs on Android**: the level progress file lives in the activity's `internalDataPath`
  (`app::android::data_dir`, through raylib's `GetAndroidApp` and the NDK's stable struct layouts);
  window size `(0, 0)` (raylib sizes the framebuffer from the display; a non-zero request is
  upscaled by the compositor), none of the iOS framebuffer glue, `target_fps` 0, GLSL ES 100
  shaders, no map saving, lower particle budget, RESTART button, no two-player mode. `app_main`
  pipes stderr into logcat (tag `bongbong`), installs a logging panic hook and runs under
  `catch_unwind`. raylib eats the Back key and ignores `SetExitKey`. With the `INTERNET` permission
  and `adb forward tcp:4747 tcp:4747` the dev server is reachable from the Mac.

## Releases (cargo-dist)

Native archives (Linux x86_64, Windows x86_64; macOS, the web, iOS and Android have their own paths)
are built by `.github/workflows/release.yml`, generated from `dist-workspace.toml` (`[profile.dist]`
in `Cargo.toml`). Trigger: bump `Cargo.toml`'s `version`, tag `v{X}.{Y}.{Z}`, push the tag.
- **Release notes are `CHANGELOG.md`**: dist puts the entry whose heading is `## X.Y.Z - YYYY-MM-DD`
  above the download table and makes that heading the release's title. `just release-prepare
  [VERSION]` runs `prepare-release.yml`: `tools/release/gather.sh` collects the PRs merged since the
  last tag (merge and squash commits, stacked PRs marked, direct commits), Claude writes the entry
  by `tools/release/notes-prompt.md` (players and testers are the readers), and a `Release X.Y.Z` PR
  is opened on `release/vX.Y.Z` (a version given bumps `Cargo.toml` and `Cargo.lock` there too);
  review and edit the notes in it, merge, then `just release-tag` (refuses without the entry,
  `tools/release/entry.sh`). `just release-notes` writes the same entry locally. Edit the prompt,
  not the workflow, to change how notes read.
- **macOS is `bongbong-macos.dmg`, not a dist archive** (`.github/workflows/macos-release.yml`,
  `tools/macos/`): a bare binary is one Gatekeeper refuses to open and Finder cannot start.
  `tools/macos/bundle.sh` (`just macos-app`) builds `BongBong.app` - one binary for both
  architectures (a release build per target, `lipo`, `MACOSX_DEPLOYMENT_TARGET` 11.0; nix's libiconv
  repointed at the system's, anything else outside `/System` and `/usr/lib` an error), `Info.plist`
  from `tools/macos/Info.plist`, `AppIcon.icns` from `tools/macos/AppIcon.png` (`just macos-icon`,
  the iOS icon's picture on Apple's macOS grid) and `static/` in `Resources` less `web/`, `_backup/`
  and `_original/` (maps, stamps and levels are compiled in). `tools/macos/package.sh` (`just
  macos-dmg`) signs it with the Developer ID Application certificate (hardened runtime, timestamp),
  notarizes and staples it, wraps it with an Applications link in a dmg whose volume carries the
  app's icon (`.VolumeIcon.icns` and the root's custom-icon Finder flag, set on a writable copy
  before it is compressed), signs, verifies, notarizes and staples that; with no certificate it
  signs ad hoc and skips Apple, and `--no-notarize` skips Apple. The workflow runs after Release on
  a version tag (like android-release.yml), on a manual run (`tag`, `upload`) and on a PR touching
  the packaging (signed, not notarized); secrets `MACOS_CERT_P12_B64`/`MACOS_CERT_PASSWORD` (the
  certificate as base64 .p12) and testflight.yml's `ASC_KEY_ID`/`ASC_ISSUER_ID`/`ASC_KEY_P8`;
  `spctl` and `stapler validate` check the result before the upload. **TestFlight** is the other Mac
  route (`tools/macos/testflight.sh`, `just macos-testflight`, testflight.yml's `upload-macos` job
  on the iOS job's triggers with a PR label `macos`): the same app signed ad hoc with
  `tools/macos/AppStore.entitlements` (the sandbox, which TestFlight and the Mac App Store require,
  plus `network.client` for the rooms - HOME becomes the app's container, so the progress file and
  saved maps follow with no code), wrapped in a hand-made `.xcarchive` and exported as on iOS
  (`app-store-connect`, Apple's cloud-managed certificates through the ASC key, `--export` stops at
  a `.pkg`); the bundle ID is UNIVERSAL and the app record carries the macOS platform. The dmg is
  not sandboxed, so a tester with both keeps two progresses.
- **`static/` ships in every archive** (`include = ["static"]`), not in the binary: assets load
  relative to the working directory, so a release only runs from its extracted folder. If a PATH
  install is ever wanted, resolve assets against `current_exe()` instead.
- `installers = []` - a windowed game, not a CLI. **`[dist.binaries]` lists `bongbong` and `probe`
  per target** and must exclude `bbmcp` (a `required-features` bin; dist builds without features and
  fails looking for it); the table opens a TOML subtable, so keep it after the last `[dist]` scalar
  and before `[dist.dependencies.apt]`. `msvc-crt-static = false` because the `cmake` crate builds
  raylib against the dynamic CRT. Linux needs raylib's dev packages (`[dist.dependencies.apt]`, the
  same list as sola-raylib's CI). No `LICENSE` file yet - revisit before any installer that surfaces
  one.
- `pr-run-mode = "skip"`: Release runs on version tags alone (a plan-only PR run woke
  android-release.yml's `workflow_run` for nothing).
- `dist generate --mode ci` regenerates the workflow; in this cargo-dist version (0.32.0)
  `--allow-dirty` silently skips the file write, so run it without the flag, and `mkdir -p
  .github/workflows` first.
