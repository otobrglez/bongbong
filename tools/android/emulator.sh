#!/usr/bin/env bash
# Make sure the bongbong AVD is booted and adb sees it; start it headless
# in the background if not (the window is available through Android
# Studio's Device Manager or `emulator -avd bongbong` by hand).
set -euo pipefail
cd "$(dirname "$0")/../.."
source tools/android/env.sh
if adb devices | grep -qE "^emulator-[0-9]+\s+device"; then
    exit 0
fi
emulator -list-avds | grep -qx "$BONGBONG_AVD" || { echo "[android] AVD $BONGBONG_AVD missing: run 'just android-setup'" >&2; exit 1; }
echo "[android] booting AVD $BONGBONG_AVD"
nohup emulator -avd "$BONGBONG_AVD" -gpu host -no-boot-anim -no-snapshot-save ${BONGBONG_EMULATOR_ARGS:-} > "${TMPDIR:-/tmp}/bongbong-emulator.log" 2>&1 &
adb wait-for-device
until [[ "$(adb shell getprop sys.boot_completed 2>/dev/null | tr -d '\r')" == "1" ]]; do sleep 2; done
adb shell settings put global window_animation_scale 0 >/dev/null 2>&1 || true
echo "[android] $BONGBONG_AVD is up"
