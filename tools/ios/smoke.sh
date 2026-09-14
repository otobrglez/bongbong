#!/usr/bin/env bash
# Build tools/ios/smoke.c against the simulator SDL3 from tools/setup_ios.sh,
# wrap it in target/ios-sim/Smoke.app and run it in the simulator with the
# console attached. Pass: "GL_VERSION=OpenGL ES 2.0 ...", "glGetError=0x0"
# lines and "SMOKE OK"; `just ios-screenshot` during the run shows an orange
# triangle on blue.
set -euo pipefail
cd "$(dirname "$0")/../.."
source tools/ios/env.sh
PC="$IOS_PREFIX/lib/pkgconfig/sdl3.pc"
[[ -f "$PC" ]] || { echo "[ios-smoke] $PC missing: run 'just ios-setup' first" >&2; exit 1; }
SDK="$(xcrun --sdk iphonesimulator --show-sdk-path)"
APP=target/ios-sim/Smoke.app
BUNDLE_ID=com.otobrglez.bongbong.smoke
rm -rf "$APP" && mkdir -p "$APP"
# The .pc's Libs line is "-lSDL3 -Wl,-framework,X ..." - the authoritative
# framework set for this SDL build.
PC_LIBS="$(sed -n 's/^Libs: //p' "$PC" | sed 's/-L[^ ]*//g')"
# shellcheck disable=SC2086
xcrun --sdk iphonesimulator clang -target "arm64-apple-ios${IOS_MIN}-simulator" -isysroot "$SDK" \
    -I"$IOS_PREFIX/include" tools/ios/smoke.c -o "$APP/smoke" \
    -L"$IOS_PREFIX/lib" $PC_LIBS -framework OpenGLES -ObjC
sed -e 's/__EXECUTABLE__/smoke/' -e "s/__BUNDLE_ID__/$BUNDLE_ID/" -e 's/__NAME__/Smoke/' \
    -e 's/__VERSION__/0.0.1/' -e "s/__MIN_OS__/$IOS_MIN/" -e 's/__PLATFORM__/iPhoneSimulator/' -e 's/__DTPLATFORM__/iphonesimulator/' tools/ios/Info.plist > "$APP/Info.plist"
plutil -lint "$APP/Info.plist" >/dev/null
xcrun simctl boot "$IOS_DEVICE" >/dev/null 2>&1 || true
open -a Simulator
xcrun simctl install booted "$APP"
xcrun simctl launch --console-pty --terminate-running-process booted "$BUNDLE_ID"
