# Environment for every iOS build step (sourced by tools/setup_ios.sh,
# tools/ios/*.sh and the `ios-*` just recipes). See CLAUDE.md's iOS section
# and docs/ios-native-port-prd.md.
#
# The devenv shell exports DEVELOPER_DIR and SDKROOT pointing at nix's
# apple-sdk-14.4, which hides Xcode: `xcrun --sdk iphonesimulator` finds no
# SDK and cc-rs/rustc would link iOS objects against MacOSX.sdk. Point the
# tools at Xcode and let xcrun pick the SDK from the target triple.
export DEVELOPER_DIR="${BONGBONG_DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
unset SDKROOT

# One deployment target for SDL3, raylib, the rgui shim (cc-rs), the Rust
# binary (rustc) and the bundle's MinimumOSVersion, so every object in the
# link carries the same LC_BUILD_VERSION minimum.
export IOS_MIN="${IOS_MIN:-15.0}"
export IPHONEOS_DEPLOYMENT_TARGET="$IOS_MIN"

# Which slice: "sim" (the simulator, default) or "ios" (a device). It picks
# the SDK (`xcrun --sdk`), the library prefix tools/setup_ios.sh installs
# into and build.rs links from, and bindgen's sysroot.
export IOS_SLICE="${IOS_SLICE:-sim}"
case "$IOS_SLICE" in
    sim) IOS_SDK_NAME=iphonesimulator ;;
    ios) IOS_SDK_NAME=iphoneos ;;
    *) echo "tools/ios/env.sh: IOS_SLICE must be sim or ios, got '$IOS_SLICE'" >&2; return 1 2>/dev/null || exit 1 ;;
esac
export IOS_SDK_NAME
export IOS_PREFIX="${BONGBONG_IOS_LIBS:-${XDG_DATA_HOME:-$HOME/.local/share}/bongbong-ios/$IOS_SLICE}"
export BONGBONG_IOS_LIBS="$IOS_PREFIX"

# The simulator device the run recipe boots (a name from `xcrun simctl list devices`).
export IOS_DEVICE="${BONGBONG_IOS_DEVICE:-iPhone 17}"

# sola-raylib-sys's build script runs bindgen even under `nobuild`. bindgen
# adds `--target=arm64-apple-ios-simulator` itself but never the SDK, so
# libclang would otherwise resolve <stdlib.h> from the macOS SDK.
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_apple_ios_sim="-isysroot $(xcrun --sdk iphonesimulator --show-sdk-path)"
export BINDGEN_EXTRA_CLANG_ARGS_aarch64_apple_ios="-isysroot $(xcrun --sdk iphoneos --show-sdk-path)"
