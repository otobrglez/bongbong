# Native dev loop: watch, rebuild, and run on source changes.
watch:
    cargo watch -x "run"

# Standing gameplay-health sweep on the shipped default map: 30 fresh
# seeded rounds (the base seed prints in the header, so any flagged round
# is replayable) plus per-kind anomaly heatmaps. See
# docs/gameplay-verification-design.md and CLAUDE.md's probe bullets.
# Pinned to a Protect band round: the shipped map's own level tables may
# say otherwise (the probe refuses --enemies under a waves plan).
# offbox-fire is the sight-box rule (docs/large-maps-follow-camera.md
# section 5): no enemy fires at a seat from outside the seat's box, ever.
probe-sweep:
    cargo run --bin probe -- --scenario afk --mission protect --spawn band --enemies 4 --frames 1800 --rounds 30 --heatmap --budget offbox-fire=0

# Waves spawn plan health check: the maps/missions/ waves fixture, Destroy
# mission (no frog, so an AFK player only loses to gunfire), 30 seeded
# rounds. Rolling-in tanks are exempt from the anomaly checks until they
# arrive. See docs/maps-to-levels.md.
probe-waves:
    cargo run --bin probe -- --map maps/missions/waves-basic.toml --scenario afk --frames 3600 --rounds 30 --seed 2000 --heatmap --budget offbox-fire=0

# Sweep every maps/test/ adversarial fixture at a pinned seed and hold it
# to the recorded baseline: each ceiling is the observed maximum across all
# six fixtures - deterministic under the pinned seed, so any exceedance is
# a real behavior change, not noise. After a deliberate AI/map/tuning
# change shifts the numbers: rerun, read the new totals, and re-baseline
# consciously - never bump a ceiling just to go green. Zero-ceilings
# (stale-start, stall, wall-grind, bump-rate, low-progress, never-arrived,
# invariant, tank-grind, offbox-fire) are kinds no fixture currently
# produces at all.
# tank-grind and pile-up joined the budgets 2026-09-14 with the enemy
# command & control instrumentation (docs/enemy-command-and-control-prd.md).
# They are the first two kinds that measure a tank against *another tank*
# rather than against the map, and were added as regression insurance, not
# a currently-failing gate (pile-up's real reading came later, below). The
# number that work actually has to move is the ram tally the sweep now
# prints beside the totals, which is not budgeted: a ram is not a failure,
# and once C2 can order one, budgeting it would budget the feature.
# Re-measured 2026-09-04, twice. First after the Protect mission's hunter
# roll (`enemy_hunter_share_protect`, one RNG draw per enemy in
# `Game::init`) shifted every stream: with the share zeroed the previous
# totals came back exactly, so those differences were the stream, not the
# hunters. Then after hunters learned to shoot through destructible walls
# (line of fire), got a snipe cooldown and a ring slot even when alone:
# jitter 2 -> 6 and churn 7 -> 10. frog-block carries the jitter=6 (and
# churn=3, clustering=4) and every one of them is a hunter - the totals are
# 0/0/4 with the share zeroed; traced frame by frame they are the normal
# diagonal-slot approach (right/down legs each held the full commitment
# hold) plus the turn to snipe the player and back, not a stall or a
# spin. maze holds churn=10 and clustering=8 at this seed; border-stuck=1
# is the maze at seed 0x3ea and pockets at 0x3eb (plain player-role
# tanks: one wedging at the map corner for ~1.5 s while routing around
# the maze's edge, one holding an aligned firing line on the player 26 px
# from the bottom wall). See docs/gameplay-verification-design.md.
# Re-measured 2026-10-01 for the sight-box rule (docs/large-maps-follow-
# camera.md section 5): an enemy fires at a seat only from inside the
# seat's +-11.5 x +-7.5 cell box, and offbox-fire=0 holds every fixture to
# it (before the rule choke, pockets, props and towers fired 10, 20, 42 and
# 10 shots from outside it). The same change made the sweep totals sum
# every kind: they used to drop tank-grind and pile-up, so those two zero
# ceilings had never been read off a sweep. pile-up=2 is maze's, with or
# without the rule (round 1, 0x3e9: two flags in the maze's middle); choke
# reads 1 (round 8, 0x3f0: three tanks on the player's row at the gap's
# mouth, the funnel this fixture provokes, on a timeline the rule moved -
# its north tank now fires from 224 px rather than 283). Every other
# fixture's totals are unchanged but props (border-stuck 0 -> 1, churn
# 4 -> 5) and towers (spin 0 -> 1, clustering 1 -> 0), all inside their
# ceilings.
# The probe is built once and the fixtures run side by side, one per core,
# each into its own log, printed in fixture order once all are done; every
# fixture runs even after one fails, and the recipe fails if any did. CI
# runs this recipe (.github/workflows/ci.yml), so these are the only copy
# of the ceilings.
probe-fixtures:
    #!/usr/bin/env bash
    set -euo pipefail
    budgets="--budget stale-start=0 --budget stall=0 --budget border-stuck=1 --budget jitter=6 --budget spin=1 --budget churn=10 --budget clustering=9 --budget wall-grind=0 --budget bump-rate=0 --budget low-progress=0 --budget never-arrived=0 --budget invariant=0 --budget tank-grind=0 --budget pile-up=2 --budget offbox-fire=0"
    cargo build --bin probe
    probe="${CARGO_TARGET_DIR:-target}/debug/probe"
    logs=$(mktemp -d)
    trap 'rm -rf "$logs"' EXIT
    printf '%s\n' maps/test/*.toml | xargs -P "$(getconf _NPROCESSORS_ONLN)" -I{} sh -c \
        '"$1" --map "$2" --frames 1800 --rounds 10 --seed 1000 $3 > "$4/$(basename "$2").log" 2>&1 || touch "$4/$(basename "$2").failed"' \
        _ "$probe" {} "$budgets" "$logs"
    failed=0
    for m in maps/test/*.toml; do
        cat "$logs/$(basename "$m").log"
        if [ -e "$logs/$(basename "$m").failed" ]; then
            echo "probe-fixtures: $m is over its ceilings" >&2
            failed=1
        fi
    done
    exit "$failed"

# Field maps (docs/large-maps-follow-camera.md section 12): the rules only
# a map the camera follows plays by (`simulation::field`) - alerts chained
# from neighbour to neighbour with leashes home, far enemies thinking every
# few ticks or asleep, band spawns and wave gates out of every seat's
# sight box and about a 15 s walk out where the map has one, a wave called
# to the fight, a fallen seat back through the gate nearest its team. The
# 96 x 54 study map, longwater (the shipped 112 x 63 free-play field) and
# five levels 56 wide, AFK, at a pinned seed; the maps/test/ fixtures
# stay arenas (`view = "whole"`), so this is where a change to the
# field-map AI shows. Ceilings are each kind's maximum over the seven maps,
# recorded 2026-10-02 and re-baselined the same day for lanes (a hull turns
# where its slide ends on the centre line of the lane it turns into,
# docs/large-maps-follow-camera.md section 12) - re-baseline consciously,
# never to go green. Lanes re-time every round, and over eight 30-round
# sweeps of each 40-wide level (seeds 1000 and 2000, AFK and advancing, one
# seat and two) every kind's total held or fell - jitter -4 %, spin -3 %,
# stall -14 %, low-progress -45 %, clustering level - but for hedge-maze's
# jitter (+9 %, in its one-lane gaps) and harbor-lights' clustering and
# pile-ups (+23 %, +26 %, at its seat's north firing slot, which the fight
# reaches sooner); these ten rounds read, before -> after: jitter=28 is
# harbor-lights (23 -> 28; 18 -> 27 on hedge-maze); spin=8 is archipelago
# (5 -> 8, tanks circling its islands' shores); clustering=10 is hedge-maze
# (5 -> 10; the old ceiling, 6, was archipelago's, now 3); stall=1 is
# archipelago (round 7, 0x3ef: a hunter standing to fire at the players'
# frog, a hold the probe mutes only for a seat); low-progress=1 is
# archipelago (round 7 again: a tank pressed for three seconds against
# another's hull, on a route of its own the margin still steers, before its
# stuck escape). border-stuck=3 (hedge-maze, 3 -> 3: enemies spawned in the
# maze's lanes along its top and bottom edge, which they drive for their
# first seconds), churn=20 and pile-up=6 stand. The study map reads
# border-stuck=1 jitter=13 churn=8 and meets the fight about 12 s in, in
# all ten rounds (it was 13 s, in nine); longwater reads jitter=4 spin=1
# churn=3 and meets it about 11 s in, in all ten (in round 0x3f1 two of the
# first wave were still out at 60 s, riding the edge of a nav row past
# their turning). Re-baselined a second time the same day for the
# margin's window (a hull whose switch-margin window is no wider than the
# ground it covers between two thinks can cross it unseen, pass after
# pass, so its searched route's turn is a lane turn too -
# `ai::margin_never_turns`, docs/large-maps-follow-camera.md section 12):
# about a third of the sweeps' rounds re-time, and over sixteen 30-round
# sweeps of each 40-wide level (seeds 1000 to 4000) clustering and
# pile-ups held, churn fell 4 % and jitter rose 2 %, 5 % on hedge-maze,
# whose maze routes are a search's staircases a hull on that sliver used
# to drive past. These ten rounds read: jitter=30 is hedge-maze (27 -> 30:
# 0x3e8, 0x3e9, 0x3ec, 0x3ef and 0x3f1 one more each, 0x3ed two fewer -
# collision dodges in the maze's lanes on a re-timed first contact, and in
# 0x3ec and 0x3ef a staircase turn the old margin let the hull drive past;
# harbor-lights stays at 28). stall=1 and low-progress=1 stand, archipelago
# no longer reading them (round 7 re-times: 1 -> 0 each); spin=8 stands
# (archipelago 8 -> 6), as do border-stuck=3 (hedge-maze 3 -> 3),
# clustering=10 (hedge-maze 10 -> 10), churn=20 and pile-up=6 (hedge-maze
# 6 -> 6). The study map reads border-stuck=1 jitter=13 churn=6 pile-up=2
# (churn 8 -> 6, pile-up 0 -> 2) and still meets the fight about 12 s in;
# longwater reads as before, black-gold and castle-moat but for a point of
# churn. Re-baselined again when the shipped maps grew by 40 % a side
# (34 x 17 to 48 x 24, 40 x 20 to 56 x 28, longwater 80 x 45 to 112 x 63;
# the study map is as it was): border-stuck=8 is castle-moat (0 -> 8: the
# south gate's tanks drive the strip between the moat and the field's edge,
# behind a line of fences, to the bridge - the strip they always took, a
# third again as long); spin=9 and churn=39 are hedge-maze (2 -> 9 and
# 12 -> 39: its walks run longer, and a tank with nobody in sight wanders
# more of them); wall-grind=1 is harbor-lights (0 -> 1: in round 0x3ef a
# tank pressed toward the frog against the pier beside it on the beach).
# Every other ceiling stands: jitter 24 on hedge-maze (from 30) and 20 on
# harbor-lights (from 28), clustering=10 on both, pile-up 5 on hedge-maze
# (from 6). The fight comes later on the larger fields: longwater about
# 13.5 s in (10.4), black-gold 14.9 s (5.6), hedge-maze 9.7 s (2.6). Prints
# first contact and ms per tick beside the anomalies. Re-baselined once more
# for the play-test pass (players' armour 0.77, a shield of 70 on a 6 s clock,
# a rocket pickup one salvo of four), which re-times the rounds: spin=12,
# churn=43 and clustering=11 are hedge-maze (9, 39, 10), pile-up=8 is
# archipelago (5 -> 8), tank-grind=1 is hedge-maze and archipelago (0 -> 1);
# with the old armour and shield hedge-maze reads its old ceilings exactly.
# Not in CI: well over two minutes in a debug build, the study map alone
# more than one.
probe-fields:
    for m in maps/study/frontier.toml maps/longwater.toml maps/hedge-maze.toml maps/archipelago.toml maps/black-gold.toml maps/harbor-lights.toml maps/castle-moat.toml; do cargo run --bin probe -- --map $m --frames 3600 --rounds 10 --seed 1000 --budget stale-start=0 --budget stall=1 --budget border-stuck=8 --budget jitter=30 --budget spin=12 --budget churn=43 --budget clustering=11 --budget wall-grind=1 --budget bump-rate=0 --budget low-progress=1 --budget never-arrived=0 --budget invariant=0 --budget tank-grind=1 --budget pile-up=8 --budget offbox-fire=0 || exit 1; done

# The perfect defence (docs/large-maps-follow-camera.md section 12): the
# probe's `defend` scenario - every enemy destroyed the moment it comes
# within 400 px of a live seat or the players' frog, so nobody is ever
# attacked - on the two maps that need the follow camera, ten seven-minute
# rounds each at a pinned seed. A round lasts exactly as long as its last
# straggler keeps it waiting, and never-arrived counts one still out. Every
# round is won; a tank's walk to the defence has a median of 15 s on
# longwater and 10 s on the study map, the longest 37 s and 54 s, and the
# rounds last 149 to 169 s on longwater and 118 to 199 s on the study map,
# whose rounds 0x3ec and 0x3ee each roll two stragglers in again through a
# nearer gate (`probe: rerolls:`) - 0x3ec's last a wave tank the portals
# and the pickups beyond them led off across the map. On the 80 x 45
# longwater, before the shipped maps grew by 40 % a side, the walk's
# median was 11 s, its longest 24 s and the rounds 117 to 133 s; the
# figures before that are the 80 x 45 field's too. Before the margin's
# window (docs/large-maps-follow-camera.md section 12) re-timed them, the
# longest walks were 40 and 48 s and the rounds lasted 122 to 138 and 118
# to 167 s; before the pacing director shortened the calm breathers and
# the stragglers were rolled in again, 51 and 86 s, 123 to 152 and 123 to
# 259 s. Before lanes, 8 of longwater's ten rounds and 7 of the study
# map's still had a tank out at seven minutes, the longest walks 147 and
# 149 s, and tanks still walking after 339 and 378. Release: in a debug
# build the twenty rounds take the better part of an hour.
probe-defend:
    for m in maps/longwater.toml maps/study/frontier.toml; do cargo run --release --bin probe -- --map $m --scenario defend --frames 25200 --rounds 10 --seed 1000 --budget never-arrived=0 || exit 1; done

run:
    cargo run

# Map thumbnails (docs/mapshot-prd.md): every map under maps/ rendered to a
# field-only PNG of its first frame, into target/thumbnails/ with the same
# relative paths. The CPU renderer needs no window, so this also runs on a
# server with no display; `mapshot --help` lists the knobs (--scale, --seed,
# --players, --tank, --no-tanks, --plain).
thumbnails:
    cargo run --bin mapshot -- --out-dir target/thumbnails maps

# The same batch through the game's own renderer in a hidden window - the
# cross-check that the CPU output is what the game draws.
thumbnails-gpu:
    cargo run --bin mapshot -- --renderer gpu --out-dir target/thumbnails-gpu maps

# GPU against CPU on the shipped maps: renders each both ways and prints the
# mean channel difference and the share of pixels off; exits 1 beyond the
# tolerance in thumbnail.rs. Opens a hidden window, hence a recipe rather
# than a `cargo test` case (macOS creates windows on the main thread only).
mapshot-compare:
    cargo run --bin mapshot -- --check --out-dir target/thumbnails-check maps/default.toml maps/portals.toml maps/missions maps/test/props.toml

# Palette guards on the generated sheets: every opaque pixel on the Puny
# Palette, and no green on anything drawn over the ground layer (walls,
# props, blasts) - see tools/check_sheets.py and docs/PALETTE.md.
check-sheets:
    nix-shell -p "python3.withPackages (ps: [ps.pillow])" --run "python3 tools/check_sheets.py"

# One-time: install the emsdk toolchain (pinned version, see
# tools/setup_emscripten.sh) and the site/'s JS dependencies (Astro).
# The wasm32-unknown-emscripten rustup target and node/yarn are already
# provided by devenv.nix (languages.rust.targets, languages.javascript).
setup-web:
    ./tools/setup_emscripten.sh
    cd site && yarn install

# Build the release wasm binary, stage bongbong.{wasm,js,data} into
# site/public/game/ (gitignored - see site/README.md), then build the Astro
# site (site/dist/) around it. Auto-sources emsdk_env.sh from
# ~/.local/share/emsdk if emcc is not on PATH.
build-web: (_build-web "")

# Same as build-web but with the `dev-tools` cargo feature: the wasm exports
# the tuning C API (src/capi.rs) and the page's tuning panel appears under
# the canvas (docs/runtime-tuning-design.md). This is what PR previews ship;
# production (cloudflare-deploy.yml) uses plain build-web.
build-web-dev: (_build-web "--features dev-tools")

_build-web features:
    bash -c 'set -e; \
        command -v emcc >/dev/null 2>&1 \
            || source ~/.local/share/emsdk/emsdk_env.sh >/dev/null 2>&1 \
            || { echo "[build-web] emcc not on PATH and no emsdk at ~/.local/share/emsdk/. Run just setup-web first." >&2; exit 1; }; \
        cargo build --release --target wasm32-unknown-emscripten --bin bongbong {{features}}'
    mkdir -p site/public/game
    rm -f site/public/game/*
    cp target/wasm32-unknown-emscripten/release/bongbong.wasm site/public/game/
    cp target/wasm32-unknown-emscripten/release/bongbong.js site/public/game/
    cp target/wasm32-unknown-emscripten/release/deps/bongbong.data site/public/game/
    cd site && yarn build
    @echo "[build-web] site/dist/ ready. Run 'just serve-web' to open."

# Build (see build-web) then preview site/dist/ at http://localhost:4321.
serve-web: build-web
    cd site && yarn preview --port ${PORT:=4321}

# Dev-tools build (see build-web-dev) then preview it at http://localhost:4321.
serve-web-dev: build-web-dev
    cd site && yarn preview --port ${PORT:=4321}

# Preview whatever site/dist/ currently holds, without rebuilding - so a
# `just build-web-dev` isn't silently overwritten by serve-web's own
# (feature-less) build-web dependency. Fails if nothing has been built yet.
preview-web:
    test -f site/dist/index.html || { echo "[preview-web] nothing built yet - run just build-web or just build-web-dev first" >&2; exit 1; }
    cd site && yarn preview --port ${PORT:=4321}

# Native dev-tools build with the embedded dev server listening on
# 127.0.0.1:4747 (docs/dev-server-design.md): what the `bongbong` MCP
# server in .mcp.json talks to, so Claude Code (or `just mcp-call`) can
# step, inspect and screenshot the running game. Extra args pass through
# (`just run-dev --seed 0xB0B5 --enemies 4`).
run-dev *ARGS:
    cargo run --features dev-tools -- {{ARGS}}

# The same dev-tools build started in Build mode - the in-game map builder
# (docs/game-editor-fusion.md), with the dev server attached so the
# `builder_*` tools can drive it. `--editor` works in every build, and any
# native build loads and saves maps/*.toml through FILE. Extra args pass
# through (`just run-editor --map maps/test/maze.toml`).
run-editor *ARGS:
    cargo run --features dev-tools -- --editor {{ARGS}}

# `watch` with the dev server: rebuild and relaunch on every source change.
# The MCP adapter reconnects per call, so a relaunch only costs the
# in-flight request.
watch-dev:
    cargo watch -x "run --features dev-tools"

# The online co-op room server on loopback (docs/online-coop-prd.md §4.13,
# CLAUDE.md's room server section): plain ws:// on 127.0.0.1:4848, and
# `/health`, `/ready` and `/metrics` on the admin port 127.0.0.1:4850.
# One instance holds every room.
# Headless - no raylib in its graph. Extra args pass through
# (`just run-server --max-rooms 10`).
run-server *ARGS:
    cargo run -p bongbong-server -- --listen 127.0.0.1:4848 --insecure {{ARGS}}

# The same server with its dev tools (server/src/devserver.rs): a
# loopback JSON socket on 4849 that `bbmcp rooms` drives, so the
# `mcp__bongbong-rooms__*` tools can open a room with no client, post a
# seat's intents and step the round deterministically. Dev only - the
# release image builds without the feature and has no listener at all.
run-server-dev *ARGS:
    cargo run -p bongbong-server --features dev-tools -- --listen 127.0.0.1:4848 --insecure {{ARGS}}

# One room-server tool from a shell, the way `just mcp-call` drives the
# game (`just rooms-mcp-call rooms`, `just rooms-mcp-call room_open '{"seats":2}'`).
rooms-mcp-call TOOL *PARAMS:
    cargo run -q --features dev-tools --bin bbmcp -- rooms call {{TOOL}} {{PARAMS}}

# netlab (netlab/README.md, docs/online-coop-prd.md §4.16): how far
# networked play is from local play, measured - the room server in-process
# behind a TCP-modelling impairment proxy, two headless clients running the
# window's own `OnlineRound`, and a local twin fed the same scripts. Release
# build, because its frame loops run in real time.
# `just netlab run --profile typical --scenario shoot`,
# `just netlab run --remote wss://rooms.bongbong.io/pr-48 --scenario duel`.
netlab *ARGS:
    cargo run --release -p netlab -- {{ARGS}}

# Profiles x scenarios x client-hull modes as one markdown table, each run
# in its own process (`just netlab-suite --quick`, a few minutes;
# `--remote URL` to sweep a deployed server).
netlab-suite *ARGS:
    cargo run --release -p netlab -- suite {{ARGS}}

# --- The room server's image and its deploy (docs/online-coop-prd.md §4.8) ---
#
# The registry and the cluster are reached over Tailscale, so these need
# the tailnet up (`tailscale status`). The image tag is `git describe`,
# the same one .github/workflows/deploy-rooms.yml stamps.

rooms_registry := "registry.folk-decibel.ts.net"
rooms_image := rooms_registry / "bongbong/bongbong-server"

# linux/amd64 is what the cluster runs; on an Apple silicon machine this
# is an emulated build - slow, but it is what makes the artifact the same
# one the cluster gets.
# Build the room server's image locally.
rooms-image *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail
    TAG=$(git describe --tags --always)
    echo "Building {{rooms_image}}:$TAG"
    docker buildx build --platform linux/amd64 \
      -t "{{rooms_image}}:latest" -t "{{rooms_image}}:$TAG" \
      --load {{ARGS}} .

# `--insecure` is added because nothing terminates TLS in front of it here.
# Run the image the cluster would run, on loopback.
rooms-image-run PORT='4848':
    docker run --rm -p {{PORT}}:4848 -p 127.0.0.1:4850:4850 {{rooms_image}}:latest \
      --listen 0.0.0.0:4848 --admin-listen 0.0.0.0:4850 --insecure

# deploy-rooms.yml does exactly this; this is the hand path for a one-off.
# Build the image, push it, and point the kustomization at the new tag.
rooms-push *ARGS:
    #!/usr/bin/env bash
    set -euo pipefail
    TAG=$(git describe --tags --always)
    echo "Building and pushing {{rooms_image}}:$TAG"
    docker buildx build --platform linux/amd64 \
      -t "{{rooms_image}}:latest" -t "{{rooms_image}}:$TAG" \
      --push {{ARGS}} .
    echo "Updating kustomization.yaml with tag: $TAG"
    cd k8s/base && kustomize edit set image "{{rooms_image}}:$TAG"

# A deploy with rounds in progress waits for them: `Recreate` plus a drain
# of up to 30 minutes, which is the trade for ending nobody's round.
# Apply the manifests and wait the rollout out.
rooms-deploy:
    #!/usr/bin/env bash
    set -euo pipefail
    kubectl apply -k k8s/base/
    kubectl rollout status deployment/rooms -n bongbong-prod --timeout=35m

# What the cluster would apply, without applying it.
rooms-manifests:
    kubectl kustomize k8s/base/

# The room server's own logs, followed.
rooms-logs *ARGS:
    kubectl logs -n bongbong-prod deployment/rooms --follow --tail=100 {{ARGS}}

# Call one dev-server tool from the shell, e.g.
# `just mcp-call step '{"frames":120,"move_dir":"up"}'` or `just mcp-call nav_grid`.
# Same tools the MCP server exposes (src/devserver.rs's TOOLS).
mcp-call TOOL ARGS='{}':
    cargo run -q --features dev-tools --bin bbmcp -- call {{TOOL}} '{{ARGS}}'

# --- iOS simulator (docs/ios-native-port-prd.md, CLAUDE.md's iOS section) ---
# Every recipe runs through tools/ios/env.sh: Xcode as DEVELOPER_DIR (the
# devenv shell points it at nix's apple-sdk), one deployment target, the
# library prefix build.rs links from, and bindgen's simulator sysroot.

# One-time: SDL3 (static) and raylib (SDL backend, OpenGL ES 2.0) for the
# simulator into ~/.local/share/bongbong-ios/sim (tools/setup_ios.sh,
# pinned SDL tag).
ios-setup:
    ./tools/setup_ios.sh

# Gate 0: an SDL3 + GL ES 2 glDrawElements program in the simulator
# (tools/ios/smoke.c). Apple Silicon simulators have had a crash in that
# call; rerun after every Xcode or runtime update. Pass = "SMOKE OK".
ios-smoke:
    ./tools/ios/smoke.sh

# Build the game for the simulator (plain build, no dev-tools) and stage
# target/ios-sim/BongBong.app. Extra args go to cargo (`--release`).
build-ios-sim *ARGS:
    bash -c 'set -e; source tools/ios/env.sh; cargo build --target aarch64-apple-ios-sim --bin bongbong {{ARGS}}'
    bash -c 'set -e; source tools/ios/env.sh; ./tools/ios/bundle.sh debug'

# Build, boot the simulator (BONGBONG_IOS_DEVICE, default "iPhone 17"),
# install the bundle and launch it with the console attached (Ctrl-C
# detaches; the app keeps running). Rotate the simulator to landscape with
# Cmd+Left if it comes up portrait.
run-ios-sim *ARGS: (build-ios-sim ARGS)
    bash -c 'set -e; source tools/ios/env.sh; \
        xcrun simctl boot "$IOS_DEVICE" >/dev/null 2>&1 || true; open -a Simulator; \
        xcrun simctl install booted target/ios-sim/BongBong.app; \
        xcrun simctl launch --console-pty --terminate-running-process booted com.otobrglez.bongbong'

# Screenshot the booted simulator (default target/ios-sim/shot.png).
ios-screenshot OUT="target/ios-sim/shot.png":
    bash -c 'source tools/ios/env.sh; xcrun simctl io booted screenshot {{OUT}}'

# --- iPhone (device) ---
# One-time: the device slice of SDL3 + raylib into ~/.local/share/bongbong-ios/ios.
ios-setup-device:
    SLICE=ios ./tools/setup_ios.sh

# Build for the phone and stage target/ios-device/BongBong.app (unsigned).
build-ios-device *ARGS:
    bash -c 'set -e; export IOS_SLICE=ios; source tools/ios/env.sh; cargo build --target aarch64-apple-ios --bin bongbong {{ARGS}}'
    bash -c 'set -e; export IOS_SLICE=ios; source tools/ios/env.sh; ./tools/ios/bundle.sh debug'

# Build, then tools/ios/deploy.sh: sign for the wired iPhone (tools/ios/sign.sh:
# Xcode's automatic signing on the placeholder project mints the certificate
# and profile), install, launch with the console attached and verify the
# round came up (raylib's window, the render targets, the process still
# alive - "DEPLOY OK"). Refusals name the fix: pairing (Trust), Developer
# Mode, or the first run's profile trust under Settings > General > VPN &
# Device Management. Ctrl-C quits the app too; `deploy.sh --no-console`
# detaches instead. Extra args go to cargo (`--features dev-tools` for the
# frame-time log and the dev server, reachable from the Mac through
# `iproxy 4747:4747`).
run-ios-device *ARGS: (build-ios-device ARGS)
    ./tools/ios/deploy.sh iPhone

# The same for the wired iPad (the phone, even when paired over Wi-Fi, is
# never picked). BONGBONG_IOS_UDID pins a device explicitly.
run-ios-ipad *ARGS: (build-ios-device ARGS)
    ./tools/ios/deploy.sh iPad

# tools/ios/testflight.sh needs BONGBONG_IOS_TEAM and the ASC_* API key in
# .envrc; `--export` stops at a signed .ipa instead of uploading.
# Build (release), sign for the App Store and upload to TestFlight.
ios-testflight *ARGS:
    ./tools/ios/testflight.sh {{ARGS}}

# tools/ios/gen_app_icon.py; --row N picks another chassis.
# Regenerate the App Store icon.
ios-icon *ARGS:
    nix-shell -p "python3.withPackages (ps: [ps.pillow])" --run "python3 tools/ios/gen_app_icon.py {{ARGS}}"

# --- Android (docs/android-port-prd.md, CLAUDE.md's Android section) ---
# Every recipe sources tools/android/env.sh: the SDK, NDK and JDK paths,
# the API pins, the prebuilt raylib prefix and the NDK compiler for cargo.

# One-time: command-line tools, NDK, platform, arm64 system image, the
# `bongbong` AVD, and raylib built for Android (tools/setup_android.sh).
android-setup:
    ./tools/setup_android.sh

# Gate 0: raylib's Android platform in a NativeActivity (tools/android/smoke.c)
# on the AVD - proves toolchain, packaging, GL ES 2, assets and touch before
# any Rust. Pass = "SMOKE OK" on screen, a non-zero asset size and touch lines in logcat.
android-smoke:
    ./tools/android/smoke.sh

# Build libbongbong_android.so (a plain cargo build for aarch64-linux-android;
# tools/android/env.sh points cargo, cc-rs and bindgen at the NDK) and stage
# target/android/BongBong.apk (tools/android/package.sh: assets/static, the
# .so, the icon, the INTERNET permission the rooms need; signed with the
# debug key unless BONGBONG_ANDROID_KEYSTORE names the release one).
build-android *ARGS:
    bash -c 'set -e; source tools/android/env.sh; cargo build --release --target aarch64-linux-android -p bongbong-android {{ARGS}}; \
        tools/android/package.sh target/aarch64-linux-android/release/libbongbong_android.so bongbong_android com.otobrglez.bongbong BongBong target/android/BongBong.apk static/ \
        "    <uses-permission android:name=\"android.permission.INTERNET\" />" bongbong_on_create'

# tools/android/gen_app_icon.py; --row N picks another chassis.
# Regenerate the launcher icon's layers under tools/android/res/.
android-icon *ARGS:
    nix-shell -p "python3.withPackages (ps: [ps.pillow])" --run "python3 tools/android/gen_app_icon.py {{ARGS}}"

# Build, boot the AVD if needed, install and launch the game, then follow logcat (Ctrl-C detaches).
run-android *ARGS: (build-android ARGS)
    bash -c 'set -e; source tools/android/env.sh; tools/android/emulator.sh; \
        adb install -r target/android/BongBong.apk; adb logcat -c || true; \
        adb shell am start -n com.otobrglez.bongbong/android.app.NativeActivity; \
        adb logcat -s raylib:V bongbong:V'

# Screenshot the running emulator (default target/android/shot.png).
android-screenshot OUT="target/android/shot.png":
    bash -c 'source tools/android/env.sh; adb exec-out screencap -p > {{OUT}} && echo {{OUT}}'

# Inject a tap (`just android-tap 600 400`) or a swipe (`just android-swipe 1800 600 1800 300`) in screen pixels.
android-tap X Y:
    bash -c 'source tools/android/env.sh; adb shell input tap {{X}} {{Y}}'
android-swipe X1 Y1 X2 Y2 MS="300":
    bash -c 'source tools/android/env.sh; adb shell input swipe {{X1}} {{Y1}} {{X2}} {{Y2}} {{MS}}'

# Conference demos (ntk/): isolated raylib-with-Rust examples, one file per
# demo under ntk/src/bin/. `just ntk 01_hello`.
ntk NAME *ARGS:
    cargo run -p ntk-demos --bin {{NAME}} -- {{ARGS}}
