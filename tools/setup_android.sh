#!/usr/bin/env bash
# One-time: the Android SDK pieces the port needs (command-line tools, the
# NDK, a platform, an arm64 system image, the `bongbong` virtual device),
# cargo-ndk, and raylib 6.0 (the tree vendored in the sola-raylib-sys crate)
# built for Android - PLATFORM=Android, OpenGL ES 2.0, the NDK's
# native_app_glue compiled in - installed under $BONGBONG_ANDROID_LIBS.
# android/build.rs links the game against that prefix. See
# docs/android-port-prd.md and CLAUDE.md's Android section.
#
# Idempotent: every step checks for its result first. Downloads are several
# gigabytes the first time. Android Studio's SDK Manager can install the same
# packages by hand; the script then only builds raylib.
set -euo pipefail
cd "$(dirname "$0")/.."
source tools/android/env.sh

# Any command-line tools release can bootstrap sdkmanager, which then installs
# the current `cmdline-tools;latest` on top of itself.
CMDLINE_TOOLS_BOOTSTRAP="${CMDLINE_TOOLS_BOOTSTRAP:-commandlinetools-mac-9862592_latest.zip}"
ROOT="${XDG_DATA_HOME:-$HOME/.local/share}/bongbong-android"
SRC="$ROOT/src"
BUILD="$ROOT/build"
mkdir -p "$SRC" "$BUILD" "$BONGBONG_ANDROID_LIBS"

[[ -x "$JAVA_HOME/bin/java" ]] || { echo "[setup-android] no JDK at $JAVA_HOME (Android Studio's JBR): install Android Studio or set JAVA_HOME" >&2; exit 1; }
for tool in git cmake curl unzip; do
    command -v "$tool" >/dev/null 2>&1 || { echo "[setup-android] $tool not found" >&2; exit 1; }
done

# --- command-line tools (sdkmanager, avdmanager) ------------------------------
if [[ ! -x "$ANDROID_HOME/cmdline-tools/latest/bin/sdkmanager" ]]; then
    echo "[setup-android] installing command-line tools"
    tmp="$(mktemp -d)"
    curl -fsSL "https://dl.google.com/android/repository/$CMDLINE_TOOLS_BOOTSTRAP" -o "$tmp/tools.zip"
    unzip -q "$tmp/tools.zip" -d "$tmp"
    mkdir -p "$ANDROID_HOME/cmdline-tools"
    rm -rf "$ANDROID_HOME/cmdline-tools/latest"
    mv "$tmp/cmdline-tools" "$ANDROID_HOME/cmdline-tools/latest"
    rm -rf "$tmp"
    yes | sdkmanager --licenses >/dev/null 2>&1 || true
    sdkmanager "cmdline-tools;latest" >/dev/null
fi
# sdkmanager cannot replace the directory it runs from, so the current tools
# land in cmdline-tools/latest-2 next to the bootstrap copy; promote them.
if [[ -x "$ANDROID_HOME/cmdline-tools/latest-2/bin/sdkmanager" ]]; then
    rm -rf "$ANDROID_HOME/cmdline-tools/latest"
    mv "$ANDROID_HOME/cmdline-tools/latest-2" "$ANDROID_HOME/cmdline-tools/latest"
fi
yes | sdkmanager --licenses >/dev/null 2>&1 || true

# --- SDK packages ----------------------------------------------------------------
need=()
[[ -d "$ANDROID_NDK_HOME" ]] || need+=("ndk;$ANDROID_NDK_VERSION")
[[ -f "$ANDROID_HOME/platforms/android-$ANDROID_PLATFORM_VERSION/android.jar" ]] || need+=("platforms;android-$ANDROID_PLATFORM_VERSION")
[[ -d "$ANDROID_HOME/system-images/android-$ANDROID_PLATFORM_VERSION/google_apis/arm64-v8a" ]] || need+=("$ANDROID_SYSTEM_IMAGE")
[[ -x "$ANDROID_HOME/platform-tools/adb" ]] || need+=("platform-tools")
[[ -x "$ANDROID_HOME/emulator/emulator" ]] || need+=("emulator")
if ((${#need[@]})); then
    echo "[setup-android] sdkmanager ${need[*]}"
    sdkmanager "${need[@]}"
fi

# --- the virtual device --------------------------------------------------------
if ! emulator -list-avds 2>/dev/null | grep -qx "$BONGBONG_AVD"; then
    echo "[setup-android] creating AVD $BONGBONG_AVD"
    device=pixel_7
    avdmanager list device -c 2>/dev/null | grep -qx "$device" || device="$(avdmanager list device -c 2>/dev/null | grep -E '^pixel' | tail -1)"
    echo no | avdmanager create avd -n "$BONGBONG_AVD" -k "$ANDROID_SYSTEM_IMAGE" -d "$device" >/dev/null
fi
# A hardware keyboard, so the host's keys arrive as a real held key. Without
# it the emulator delivers a press as a down and an up in quick succession,
# and a held arrow only nudges the tank (movement reads the key every frame).
AVD_CONFIG="$HOME/.android/avd/$BONGBONG_AVD.avd/config.ini"
if [[ -f "$AVD_CONFIG" ]] && ! grep -qx "hw.keyboard=yes" "$AVD_CONFIG"; then
    if grep -q "^hw.keyboard=" "$AVD_CONFIG"; then
        sed -i '' 's/^hw.keyboard=.*/hw.keyboard=yes/' "$AVD_CONFIG"
    else
        echo "hw.keyboard=yes" >> "$AVD_CONFIG"
    fi
    echo "[setup-android] enabled the hardware keyboard on $BONGBONG_AVD"
fi

# --- raylib for Android --------------------------------------------------------------
RL_VENDORED="$(ls -d "$HOME"/.cargo/registry/src/*/sola-raylib-sys-6.3.0/raylib 2>/dev/null | head -1 || true)"
[[ -d "$RL_VENDORED" ]] || { echo "[setup-android] sola-raylib-sys-6.3.0 is not in the cargo registry; run 'cargo fetch' first" >&2; exit 1; }
RL="$SRC/raylib"
rm -rf "$RL"
cp -R "$RL_VENDORED" "$RL"
PREFIX="$BONGBONG_ANDROID_LIBS/$ANDROID_ABI"
rm -rf "$BUILD/raylib-$ANDROID_ABI"
cmake -S "$RL" -B "$BUILD/raylib-$ANDROID_ABI" -G "Unix Makefiles" \
    -DCMAKE_TOOLCHAIN_FILE="$ANDROID_NDK_HOME/build/cmake/android.toolchain.cmake" \
    -DANDROID_ABI="$ANDROID_ABI" -DANDROID_PLATFORM="android-$ANDROID_API_MIN" \
    -DPLATFORM=Android -DBUILD_EXAMPLES=OFF -DCMAKE_BUILD_TYPE=Release \
    -DCMAKE_INSTALL_PREFIX="$PREFIX"
cmake --build "$BUILD/raylib-$ANDROID_ABI" -j"$(sysctl -n hw.ncpu)"
cmake --install "$BUILD/raylib-$ANDROID_ABI"

echo
echo "[setup-android] raylib for $ANDROID_ABI in $PREFIX:"
"$NDK_TOOLCHAIN/bin/llvm-nm" "$PREFIX/lib/libraylib.a" 2>/dev/null | grep -E " T (android_main|ANativeActivity_onCreate)$" | sed 's/^/  /'
echo "[setup-android] SDK: ndk $ANDROID_NDK_VERSION, platform android-$ANDROID_PLATFORM_VERSION, image $ANDROID_SYSTEM_IMAGE, AVD $BONGBONG_AVD"
echo "Next: just android-smoke, then just run-android."
