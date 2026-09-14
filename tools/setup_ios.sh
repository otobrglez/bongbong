#!/usr/bin/env bash
# One-time: build the iOS *simulator* slice of SDL3 (static) and of raylib
# 6.0 (the tree vendored in the sola-raylib-sys crate) with raylib's SDL
# backend and the OpenGL ES 2.0 renderer, and install both under
# $IOS_PREFIX (default ~/.local/share/bongbong-ios/sim). build.rs links the
# game against that prefix when the target is aarch64-apple-ios-sim. See
# docs/ios-native-port-prd.md and CLAUDE.md's iOS section.
#
# Idempotent: SDL3 is skipped when its marker for the pinned tag exists;
# raylib is cheap and is always rebuilt (its source is the registry crate,
# so a `cargo update` of sola-raylib is picked up).
set -euo pipefail
cd "$(dirname "$0")/.."
# SLICE=sim (default) builds for the simulator, SLICE=ios for a device;
# each goes to its own prefix (…/bongbong-ios/sim, …/bongbong-ios/ios).
SLICE="${SLICE:-sim}"
export IOS_SLICE="$SLICE"
source tools/ios/env.sh
case "$SLICE" in
    sim) SYSROOT=iphonesimulator ;;
    ios) SYSROOT=iphoneos ;;
    *) echo "[setup-ios] SLICE must be sim or ios, got '$SLICE'" >&2; exit 1 ;;
esac

# Pinned SDL tag. 3.4.x is the current stable line and carries the Xcode 26
# fixes; raylib's SDL3 detection needs only SDL_MINOR_VERSION >= 1, and the
# SDL2 names it uses come from SDL_oldnames.h in both 3.2 and 3.4. Override
# with SDL_TAG=release-3.2.30 if a 3.4 change ever trips it.
SDL_TAG="${SDL_TAG:-release-3.4.16}"

ROOT="${XDG_DATA_HOME:-$HOME/.local/share}/bongbong-ios"
SRC="$ROOT/src"
BUILD="$ROOT/build"
mkdir -p "$SRC" "$BUILD" "$IOS_PREFIX"

for tool in git cmake xcrun lipo; do
    command -v "$tool" >/dev/null 2>&1 || { echo "[setup-ios] $tool not found" >&2; exit 1; }
done
xcrun --sdk "$SYSROOT" --show-sdk-path >/dev/null 2>&1 \
    || { echo "[setup-ios] no $SYSROOT SDK: is Xcode installed at $DEVELOPER_DIR?" >&2; exit 1; }

# CMake >= 3.24 is what SDL's Apple branch requires (LINK_LIBRARY:FRAMEWORK).
IOS_CMAKE=(
    -G "Unix Makefiles"
    -DCMAKE_SYSTEM_NAME=iOS
    -DCMAKE_OSX_SYSROOT="$SYSROOT"
    -DCMAKE_OSX_ARCHITECTURES=arm64
    -DCMAKE_OSX_DEPLOYMENT_TARGET="$IOS_MIN"
    -DCMAKE_BUILD_TYPE=Release
    -DCMAKE_INSTALL_PREFIX="$IOS_PREFIX"
)
JOBS="$(sysctl -n hw.ncpu)"

# --- SDL3, static -----------------------------------------------------------
if [[ -f "$IOS_PREFIX/.sdl3-$SDL_TAG" && -f "$IOS_PREFIX/lib/libSDL3.a" ]]; then
    echo "[setup-ios] SDL3 $SDL_TAG already installed in $IOS_PREFIX"
else
    if [[ ! -d "$SRC/SDL/.git" ]]; then
        git clone --depth 1 --branch "$SDL_TAG" https://github.com/libsdl-org/SDL.git "$SRC/SDL"
    else
        git -C "$SRC/SDL" fetch --depth 1 origin "refs/tags/$SDL_TAG:refs/tags/$SDL_TAG"
        git -C "$SRC/SDL" checkout -q "$SDL_TAG"
    fi
    rm -rf "$BUILD/sdl3-$SLICE"
    cmake -S "$SRC/SDL" -B "$BUILD/sdl3-$SLICE" "${IOS_CMAKE[@]}" \
        -DSDL_SHARED=OFF -DSDL_STATIC=ON -DSDL_TEST_LIBRARY=OFF -DSDL_TESTS=OFF -DSDL_EXAMPLES=OFF
    cmake --build "$BUILD/sdl3-$SLICE" -j"$JOBS"
    cmake --install "$BUILD/sdl3-$SLICE"
    rm -f "$IOS_PREFIX"/.sdl3-*
    touch "$IOS_PREFIX/.sdl3-$SDL_TAG"
fi

# --- raylib 6.0 from the crate's vendored tree: SDL backend, ES 2.0 ---------
RL_VENDORED="$(ls -d "$HOME"/.cargo/registry/src/*/sola-raylib-sys-6.3.0/raylib 2>/dev/null | head -1 || true)"
[[ -d "$RL_VENDORED" ]] || { echo "[setup-ios] sola-raylib-sys-6.3.0 is not in the cargo registry; run 'cargo fetch' first" >&2; exit 1; }

# A fresh copy of the vendored tree (the registry is never edited) with the
# one patch iOS needs on top: tools/ios/raylib-sdl-highdpi.patch teaches the
# SDL backend to render into the whole high-pixel-density drawable (3x on a
# phone) instead of a 1x corner of it - the GLFW backend already does this
# on high-DPI displays through raylib's screenScale, the SDL one never did.
RL="$SRC/raylib"
rm -rf "$RL"
cp -R "$RL_VENDORED" "$RL"
patch -p1 -d "$RL" --silent < tools/ios/raylib-sdl-highdpi.patch

# -DMA_NO_COREAUDIO: raudio.c compiles miniaudio as plain C, and on iOS
# miniaudio's CoreAudio backend #includes the Objective-C AVFoundation
# umbrella header, which does not compile as C. Without CoreAudio miniaudio
# keeps its null backend; the game plays no audio. When audio arrives,
# raudio.c has to be compiled as Objective-C instead. RAYLIB_EXTRA_CMAKE is
# the escape hatch (e.g. "-DCUSTOMIZE_BUILD=ON -DSUPPORT_MODULE_RAUDIO=OFF").
rm -rf "$BUILD/raylib-$SLICE"
cmake -S "$RL" -B "$BUILD/raylib-$SLICE" "${IOS_CMAKE[@]}" \
    -DPLATFORM=SDL -DOPENGL_VERSION="ES 2.0" -DBUILD_EXAMPLES=OFF \
    -DSDL3_DIR="$IOS_PREFIX/lib/cmake/SDL3" -DCMAKE_FIND_ROOT_PATH="$IOS_PREFIX" \
    -DCMAKE_C_FLAGS="-DMA_NO_COREAUDIO" ${RAYLIB_EXTRA_CMAKE:-}
cmake --build "$BUILD/raylib-$SLICE" -j"$JOBS"
cmake --install "$BUILD/raylib-$SLICE"

# --- report ------------------------------------------------------------------
echo
echo "[setup-ios] $SLICE slice installed in $IOS_PREFIX:"
for lib in libSDL3.a libraylib.a; do
    printf '  %-12s %s\n' "$lib" "$(lipo -info "$IOS_PREFIX/lib/$lib" | sed 's/.*: //')"
done
echo "[setup-ios] SDL3 link line (lib/pkgconfig/sdl3.pc, what build.rs parses):"
grep -E '^Libs' "$IOS_PREFIX/lib/pkgconfig/sdl3.pc" | sed 's/^/  /'
echo
echo "Next: just ios-smoke (GL ES check in the simulator), then just run-ios-sim."
