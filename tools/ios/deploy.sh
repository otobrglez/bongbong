#!/usr/bin/env bash
# Sign, install, launch and verify target/ios-device/BongBong.app on a wired
# iPad or iPhone (`just run-ios-ipad`, `just run-ios-device`).
#
#   tools/ios/deploy.sh [iPad|iPhone|any|<udid>] [--no-console]
#
# The device: a UDID (or BONGBONG_IOS_UDID) is used as given; a device type
# picks the first *wired* device of that type in `xcrun devicectl list
# devices`, so a phone paired over Wi-Fi never receives an iPad deploy. The
# device must be paired (Trust on the device) and in Developer Mode; both
# are checked with the reason named before anything is built or signed.
#
# Then tools/ios/sign.sh (a profile listing this device), a devicectl
# install, and a console-attached launch that replaces a running instance.
# The verification, within DEPLOY_SETTLE seconds (default 20): the console
# shows raylib's "DISPLAY: Device initialized successfully" (the bundle was
# accepted and the GL context is up), the assets are loaded and both
# render targets exist (the last "FBO" line - a missing asset aborts before
# it), and the process is still in the device's process list afterwards (a
# crash at startup exits within the first second). Prints "DEPLOY OK".
#
# The console: with it attached, Ctrl-C quits the app too (devicectl
# forwards the signal). --no-console detaches after the check and leaves
# the app running; the log is always in target/ios-device/console.log.
# A launch the device refuses because the developer profile has not been
# trusted yet names the Settings path.
set -euo pipefail
cd "$(dirname "$0")/../.."
export IOS_SLICE=ios
source tools/ios/env.sh
OUT=target/ios-device
APP="$OUT/BongBong.app"
BUNDLE_ID=com.otobrglez.bongbong
SETTLE="${DEPLOY_SETTLE:-20}"
WANT=any
CONSOLE=1
for arg in "$@"; do
    case "$arg" in
        --no-console) CONSOLE=0 ;;
        -*) echo "[deploy-ios] unknown option $arg" >&2; exit 2 ;;
        *) WANT="$arg" ;;
    esac
done
[[ -d "$APP" ]] || { echo "[deploy-ios] $APP missing: run 'just build-ios-device' first" >&2; exit 1; }
log() { echo "[deploy-ios] $*"; }
fail() { echo "[deploy-ios] FAIL: $*" >&2; exit 1; }
# devicectl prints "warning: unhandled Platform key" lines on every call.
quiet() { grep -v '^warning: unhandled Platform key' || true; }

# --- 1. The device ---------------------------------------------------------
DEVICES="$OUT/devices.json"
xcrun devicectl list devices --json-output "$DEVICES" >/dev/null 2>&1 || fail "'xcrun devicectl list devices' failed"
pick_device() {
    python3 - "$DEVICES" "$1" <<'PY'
import json, sys
devices = json.load(open(sys.argv[1]))["result"]["devices"]
want = sys.argv[2].lower()
for d in devices:
    hw, cp, dp = d["hardwareProperties"], d["connectionProperties"], d["deviceProperties"]
    udid = hw.get("udid", "")
    kind = hw.get("deviceType", "?")
    wired = cp.get("transportType") == "wired"
    if want == udid.lower() or (wired and want in ("any", kind.lower())):
        print(udid, cp.get("pairingState", "?"), dp.get("developerModeStatus") or "unknown", kind, dp.get("name", "?"))
        break
PY
}
read -r UDID PAIRING DEVMODE KIND NAME < <(pick_device "${BONGBONG_IOS_UDID:-$WANT}") || true
[[ -n "${UDID:-}" ]] || fail "no wired $WANT in 'xcrun devicectl list devices': plug it in with a data cable, unlock it and tap Trust"
log "$KIND '$NAME' $UDID (pairing: $PAIRING, developer mode: $DEVMODE)"
[[ "$PAIRING" == "paired" ]] || fail "'$NAME' is not paired: unlock it and tap Trust, or run 'xcrun devicectl manage pair --device $UDID'"
[[ "$DEVMODE" == "enabled" ]] || fail "'$NAME' has Developer Mode off: Settings > Privacy & Security > Developer Mode (the device restarts), then rerun"

# --- 2. Sign for it ---------------------------------------------------------
BONGBONG_IOS_UDID="$UDID" ./tools/ios/sign.sh

# --- 3. Install ---------------------------------------------------------------
log "installing on '$NAME'"
if ! xcrun devicectl device install app --device "$UDID" "$APP" > "$OUT/install.log" 2>&1; then
    quiet < "$OUT/install.log" >&2
    fail "install refused (target/ios-device/install.log)"
fi
grep -E "installationURL" "$OUT/install.log" | sed 's/^/[deploy-ios] /' || true

# --- 4. Launch with the console attached, verify -------------------------------
CONSOLE_LOG="$OUT/console.log"
: > "$CONSOLE_LOG"
xcrun devicectl device process launch --console --terminate-existing --device "$UDID" "$BUNDLE_ID" > "$CONSOLE_LOG" 2>&1 &
LAUNCH_PID=$!
# SIGKILL: devicectl forwards SIGINT/SIGTERM to the app, a kill it cannot
# catch leaves the app running on the device.
detach() { kill -KILL "$LAUNCH_PID" 2>/dev/null || true; }
trap detach EXIT

READY=""
for ((i = 0; i < SETTLE * 2; i++)); do
    if grep -q "failed to launch" "$CONSOLE_LOG"; then
        quiet < "$CONSOLE_LOG" | grep -E "Unable to launch|failed to launch" | head -2 >&2
        if grep -q "not been explicitly trusted" "$CONSOLE_LOG"; then
            fail "'$NAME' has not trusted the developer profile yet: Settings > General > VPN & Device Management > Developer App > Trust, then rerun"
        fi
        fail "launch refused (target/ios-device/console.log)"
    fi
    if grep -q "FBO: \[ID 3\] Framebuffer object created" "$CONSOLE_LOG"; then READY=1; break; fi
    if ! kill -0 "$LAUNCH_PID" 2>/dev/null; then break; fi
    sleep 0.5
done
if [[ -z "$READY" ]]; then
    quiet < "$CONSOLE_LOG" | tail -15 >&2
    if grep -q "DISPLAY: Device initialized successfully" "$CONSOLE_LOG"; then
        fail "the window came up but the round did not finish loading within ${SETTLE}s (console tail above)"
    fi
    fail "no window within ${SETTLE}s: is '$NAME' unlocked? (console tail above)"
fi
sleep 1
alive() {
    xcrun devicectl device info processes --device "$UDID" --json-output "$OUT/processes.json" >/dev/null 2>&1 || return 1
    python3 - "$OUT/processes.json" <<'PY'
import json, sys
for p in json.load(open(sys.argv[1]))["result"]["runningProcesses"]:
    if p.get("executable", "").endswith("/BongBong.app/bongbong"):
        print(p["processIdentifier"]); break
PY
}
PID=$(alive)
[[ -n "$PID" ]] || { quiet < "$CONSOLE_LOG" | tail -15 >&2; fail "the app exited right after loading (console tail above)"; }
grep -E "^bongbong: iOS window" "$CONSOLE_LOG" | sed 's/^/[deploy-ios] /' || true
log "DEPLOY OK: $KIND '$NAME' runs $BUNDLE_ID (pid $PID)"

if [[ "$CONSOLE" == 1 ]]; then
    log "console attached, Ctrl-C quits the app (--no-console to leave it running)"
    trap 'kill "$TAIL_PID" 2>/dev/null; kill -INT "$LAUNCH_PID" 2>/dev/null; wait "$LAUNCH_PID" 2>/dev/null; exit 0' INT TERM
    tail -n +1 -f "$CONSOLE_LOG" | grep --line-buffered -v '^warning: unhandled Platform key' &
    TAIL_PID=$!
    wait "$LAUNCH_PID" || true
    kill "$TAIL_PID" 2>/dev/null || true
    trap - EXIT
fi
