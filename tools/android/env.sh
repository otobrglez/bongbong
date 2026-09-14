# Environment for every Android build step (sourced by tools/setup_android.sh,
# tools/android/*.sh and the `android-*` just recipes). See CLAUDE.md's
# Android section and docs/android-port-prd.md.

export ANDROID_HOME="${ANDROID_HOME:-$HOME/Library/Android/sdk}"
export ANDROID_SDK_ROOT="$ANDROID_HOME"

# One pin per component; tools/setup_android.sh installs exactly these.
export ANDROID_NDK_VERSION="${ANDROID_NDK_VERSION:-29.0.14206865}"
export ANDROID_PLATFORM_VERSION="${ANDROID_PLATFORM_VERSION:-36}"
export ANDROID_API_MIN="${ANDROID_API_MIN:-29}"
export ANDROID_API_TARGET="${ANDROID_API_TARGET:-36}"
export ANDROID_SYSTEM_IMAGE="system-images;android-${ANDROID_PLATFORM_VERSION};google_apis;arm64-v8a"
export ANDROID_ABI="${ANDROID_ABI:-arm64-v8a}"
export BONGBONG_AVD="${BONGBONG_AVD:-bongbong}"

export ANDROID_NDK_HOME="$ANDROID_HOME/ndk/$ANDROID_NDK_VERSION"
export ANDROID_NDK_ROOT="$ANDROID_NDK_HOME"
export NDK_TOOLCHAIN="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/darwin-x86_64"

# sdkmanager, apksigner and the emulator are Java programs; Android Studio
# ships the JDK they need.
export JAVA_HOME="${JAVA_HOME:-/Applications/Android Studio.app/Contents/jbr/Contents/Home}"

BUILD_TOOLS_DIR="$(ls -d "$ANDROID_HOME"/build-tools/*/ 2>/dev/null | sort -V | tail -1)"
export ANDROID_BUILD_TOOLS="${BUILD_TOOLS_DIR%/}"
export PATH="$ANDROID_HOME/cmdline-tools/latest/bin:$ANDROID_HOME/platform-tools:$ANDROID_HOME/emulator:$ANDROID_BUILD_TOOLS:$NDK_TOOLCHAIN/bin:$JAVA_HOME/bin:$PATH"

# Where tools/setup_android.sh installs the prebuilt raylib (PLATFORM=Android,
# OpenGL ES 2.0, native_app_glue compiled in) and where android/build.rs
# links it from.
export BONGBONG_ANDROID_LIBS="${BONGBONG_ANDROID_LIBS:-${XDG_DATA_HOME:-$HOME/.local/share}/bongbong-android}"

# The NDK compiler for cargo, cc-rs and bindgen. The devenv shell exports
# nix's CC/LD; these target-specific variables win, the same fix the
# emscripten and iOS targets need. cargo-ndk sets the same set when it
# drives cargo; exporting them here keeps a plain `cargo build --target`
# working too.
export CARGO_TARGET_AARCH64_LINUX_ANDROID_LINKER="$NDK_TOOLCHAIN/bin/aarch64-linux-android${ANDROID_API_MIN}-clang"
export CC_aarch64_linux_android="$NDK_TOOLCHAIN/bin/aarch64-linux-android${ANDROID_API_MIN}-clang"
export CXX_aarch64_linux_android="$NDK_TOOLCHAIN/bin/aarch64-linux-android${ANDROID_API_MIN}-clang++"
export AR_aarch64_linux_android="$NDK_TOOLCHAIN/bin/llvm-ar"
export RANLIB_aarch64_linux_android="$NDK_TOOLCHAIN/bin/llvm-ranlib"
# sola-raylib-sys runs bindgen even under `nobuild`; it adds the target
# itself but never the NDK sysroot.
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_linux_android="--sysroot=$NDK_TOOLCHAIN/sysroot"
