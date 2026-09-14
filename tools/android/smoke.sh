#!/usr/bin/env bash
# Build tools/android/smoke.c against the prebuilt raylib, package it as a
# NativeActivity APK with tools/android/package.sh, install it on the AVD
# and show its logcat lines. Pass: "screen WxH", a non-zero asset size,
# "SMOKE OK" in `just android-screenshot`, and "touch" lines after
# `adb shell input tap`.
set -euo pipefail
cd "$(dirname "$0")/../.."
source tools/android/env.sh
PREFIX="$BONGBONG_ANDROID_LIBS/$ANDROID_ABI"
[[ -f "$PREFIX/lib/libraylib.a" ]] || { echo "[android-smoke] $PREFIX/lib/libraylib.a missing: run 'just android-setup' first" >&2; exit 1; }
OUT=target/android/smoke
mkdir -p "$OUT"
"$NDK_TOOLCHAIN/bin/aarch64-linux-android${ANDROID_API_MIN}-clang" -shared -fPIC -O2 \
    -I"$PREFIX/include" tools/android/smoke.c -o "$OUT/libsmoke.so" \
    -L"$PREFIX/lib" -lraylib -llog -landroid -lEGL -lGLESv2 -lOpenSLES -lm \
    -Wl,--wrap=fopen -Wl,-u,ANativeActivity_onCreate -Wl,-z,max-page-size=16384
tools/android/package.sh "$OUT/libsmoke.so" smoke com.otobrglez.bongbong.smoke Smoke "$OUT/Smoke.apk" static/ ""
tools/android/emulator.sh
adb install -r "$OUT/Smoke.apk" >/dev/null
adb logcat -c || true
adb shell am start -n com.otobrglez.bongbong.smoke/android.app.NativeActivity >/dev/null
sleep 4
adb shell input tap 600 400 || true
sleep 1
adb logcat -d -s raylib:V smoke:V | grep -E "smoke|DISPLAY|Display size|Screen size|GL:|PLATFORM" | tail -25
