# Environment for every Android build step (sourced by tools/setup_android.sh,
# tools/android/*.sh, the `android-*` just recipes and the android-release
# workflow). See CLAUDE.md's Android section and docs/android-port-prd.md.
#
# Works on a Mac with Android Studio's SDK and on a Linux runner with the
# SDK GitHub's ubuntu images preinstall: the SDK root, the NDK's host tag
# and the JDK are the three things that differ, and each is read from the
# environment first.

case "$(uname -s)" in
    Darwin) _bb_host_tag="darwin-x86_64"; _bb_sdk_default="$HOME/Library/Android/sdk" ;;
    *)      _bb_host_tag="linux-x86_64";  _bb_sdk_default="${ANDROID_SDK_ROOT:-$HOME/Android/Sdk}" ;;
esac
export ANDROID_HOME="${ANDROID_HOME:-$_bb_sdk_default}"
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
# The NDK ships one prebuilt toolchain per host; Apple silicon runs the
# x86_64 one under Rosetta and there is no arm64 directory to prefer.
export NDK_TOOLCHAIN="$ANDROID_NDK_HOME/toolchains/llvm/prebuilt/$_bb_host_tag"

# sdkmanager, apksigner, keytool and the emulator are Java programs. On a
# Mac Android Studio ships the JDK they need; a Linux runner has JAVA_HOME
# set already, and a bare Linux box falls back to whatever `java` is on
# the PATH.
if [[ -z "${JAVA_HOME:-}" ]]; then
    if [[ -d "/Applications/Android Studio.app/Contents/jbr/Contents/Home" ]]; then
        export JAVA_HOME="/Applications/Android Studio.app/Contents/jbr/Contents/Home"
    elif command -v java >/dev/null 2>&1; then
        export JAVA_HOME="$(cd "$(dirname "$(readlink -f "$(command -v java)")")/.." && pwd)"
    fi
fi

BUILD_TOOLS_DIR="$(ls -d "$ANDROID_HOME"/build-tools/*/ 2>/dev/null | sort -V | tail -1)"
export ANDROID_BUILD_TOOLS="${BUILD_TOOLS_DIR%/}"
export PATH="$ANDROID_HOME/cmdline-tools/latest/bin:$ANDROID_HOME/platform-tools:$ANDROID_HOME/emulator:$ANDROID_BUILD_TOOLS:$NDK_TOOLCHAIN/bin:${JAVA_HOME:+$JAVA_HOME/bin:}$PATH"

# Where tools/setup_android.sh installs the prebuilt raylib (PLATFORM=Android,
# OpenGL ES 2.0, native_app_glue compiled in) and where android/build.rs
# links it from.
export BONGBONG_ANDROID_LIBS="${BONGBONG_ANDROID_LIBS:-${XDG_DATA_HOME:-$HOME/.local/share}/bongbong-android}"

# The key tools/android/package.sh signs with. Unset, it signs with the
# debug keystore (minted on first use), which installs on any device but
# can never be updated over by a release build - Android ties an app's
# identity to its signing key, so the released APK is signed with one key
# for the app's whole life (the android-release workflow's secrets). The
# password variables carry apksigner's `pass:` spelling implicitly.
export BONGBONG_ANDROID_KEYSTORE="${BONGBONG_ANDROID_KEYSTORE:-}"
export BONGBONG_ANDROID_KEYSTORE_PASS="${BONGBONG_ANDROID_KEYSTORE_PASS:-}"
export BONGBONG_ANDROID_KEY_ALIAS="${BONGBONG_ANDROID_KEY_ALIAS:-}"

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
unset _bb_host_tag _bb_sdk_default
