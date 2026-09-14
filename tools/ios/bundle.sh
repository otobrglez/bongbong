#!/usr/bin/env bash
# Stage target/ios-sim/BongBong.app from a cargo build for
# aarch64-apple-ios-sim: the binary, Info.plist from the template, and the
# assets by the same relative paths the game opens (static/ and maps/ - the
# iOS entry chdirs into the bundle, which is flat, so "static/..." resolves
# as on desktop). No signing: the simulator does not need it.
set -euo pipefail
cd "$(dirname "$0")/../.."
PROFILE="${1:-debug}"
# IOS_SLICE (tools/ios/env.sh): sim -> the simulator build, ios -> the device build.
case "${IOS_SLICE:-sim}" in
    sim) TARGET=aarch64-apple-ios-sim; OUT=target/ios-sim; PLATFORM=iPhoneSimulator; DTPLATFORM=iphonesimulator ;;
    ios) TARGET=aarch64-apple-ios;     OUT=target/ios-device; PLATFORM=iPhoneOS; DTPLATFORM=iphoneos ;;
    *) echo "[bundle-ios] IOS_SLICE must be sim or ios" >&2; exit 1 ;;
esac
BIN="target/$TARGET/$PROFILE/bongbong"
APP="$OUT/BongBong.app"
[[ -x "$BIN" ]] || { echo "[bundle-ios] $BIN missing: run 'just build-ios-sim' (or build-ios-device) first" >&2; exit 1; }
VER="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
rm -rf "$APP" && mkdir -p "$APP"
cp "$BIN" "$APP/bongbong"
sed -e 's/__EXECUTABLE__/bongbong/' -e 's/__BUNDLE_ID__/com.otobrglez.bongbong/' -e 's/__NAME__/BongBong/' \
    -e "s/__VERSION__/$VER/" -e "s/__MIN_OS__/${IOS_MIN:-15.0}/" \
    -e "s/__PLATFORM__/$PLATFORM/" -e "s/__DTPLATFORM__/$DTPLATFORM/" tools/ios/Info.plist > "$APP/Info.plist"
plutil -lint "$APP/Info.plist" >/dev/null
rsync -a --exclude '_backup/' --exclude '.DS_Store' --exclude '__pycache__/' static/ "$APP/static/"
rsync -a --exclude '.DS_Store' maps/ "$APP/maps/"
echo "[bundle-ios] $APP ($(du -sh "$APP" | cut -f1))"
