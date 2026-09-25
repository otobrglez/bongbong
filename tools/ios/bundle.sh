#!/usr/bin/env bash
# Stage target/ios-sim/BongBong.app (or target/ios-device/ under
# IOS_SLICE=ios) from a cargo build: the binary, Info.plist from the
# template, the icon compiled from tools/ios/Assets.xcassets, the privacy
# manifest, and the assets by the same relative paths the game opens
# (static/ and maps/ - the iOS entry chdirs into the bundle, which is flat,
# so "static/..." resolves as on desktop). No signing: tools/ios/sign.sh
# signs for a wired device, tools/ios/testflight.sh for the App Store.
#
# The bundle carries what App Store Connect checks on an upload and Xcode
# would otherwise write: the DT* keys naming the SDK and Xcode it was built
# with (an unknown or too-old toolchain is refused), CFBundleIconName and
# the Assets.car actool makes from the one 1024 px icon.
# CFBundleShortVersionString is Cargo.toml's version; CFBundleVersion, the
# build number every upload must raise, is BONGBONG_IOS_BUILD or the UTC
# time (YYYYMMDD.HHMM).
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
BUILD="${BONGBONG_IOS_BUILD:-$(date -u +%Y%m%d.%H%M)}"
rm -rf "$APP" && mkdir -p "$APP"
cp "$BIN" "$APP/bongbong"
sed -e 's/__EXECUTABLE__/bongbong/' -e 's/__BUNDLE_ID__/com.otobrglez.bongbong/' -e 's/__NAME__/BongBong/' \
    -e "s/__VERSION__/$VER/" -e "s/__BUILD__/$BUILD/" -e "s/__MIN_OS__/${IOS_MIN:-15.0}/" \
    -e "s/__PLATFORM__/$PLATFORM/" -e "s/__DTPLATFORM__/$DTPLATFORM/" tools/ios/Info.plist > "$APP/Info.plist"
PB=/usr/libexec/PlistBuddy
# xcrun prints "warning: unhandled Platform key" lines on every call.
xcrun() { command xcrun "$@" 2> >(grep -v '^warning: unhandled Platform key' >&2); }
SDK="$DTPLATFORM"
SDK_BUILD="$(xcrun --sdk "$SDK" --show-sdk-build-version)"
XCODE="$(xcodebuild -version)"
XCODE_VER="$(sed -n 's/^Xcode //p' <<<"$XCODE")"   # 26.3 -> DTXcode 2630
IFS=. read -r XMAJ XMIN XPATCH <<<"$XCODE_VER"
"$PB" -c "Add :DTCompiler string com.apple.compilers.llvm.clang.1_0" \
    -c "Add :DTPlatformBuild string $SDK_BUILD" \
    -c "Add :DTPlatformVersion string $(xcrun --sdk "$SDK" --show-sdk-platform-version)" \
    -c "Add :DTSDKBuild string $SDK_BUILD" \
    -c "Add :DTSDKName string $SDK$(xcrun --sdk "$SDK" --show-sdk-version)" \
    -c "Add :DTXcode string $(printf '%02d%d%d' "$XMAJ" "${XMIN:-0}" "${XPATCH:-0}")" \
    -c "Add :DTXcodeBuild string $(sed -n 's/^Build version //p' <<<"$XCODE")" \
    -c "Add :BuildMachineOSBuild string $(sw_vers -buildVersion)" "$APP/Info.plist"
[[ "$SDK" == iphoneos ]] && "$PB" -c "Add :UIRequiredDeviceCapabilities array" \
    -c "Add :UIRequiredDeviceCapabilities:0 string arm64" "$APP/Info.plist"
ICON_PLIST="$OUT/icon-info.plist"
xcrun actool tools/ios/Assets.xcassets --compile "$APP" --platform "$SDK" \
    --minimum-deployment-target "${IOS_MIN:-15.0}" --app-icon AppIcon \
    --target-device iphone --target-device ipad \
    --output-partial-info-plist "$ICON_PLIST" --errors --warnings >/dev/null
"$PB" -c "Merge $ICON_PLIST" "$APP/Info.plist" >/dev/null
cp tools/ios/PrivacyInfo.xcprivacy "$APP/"
plutil -lint "$APP/Info.plist" >/dev/null
rsync -a --exclude '_backup/' --exclude '.DS_Store' --exclude '__pycache__/' static/ "$APP/static/"
rsync -a --exclude '.DS_Store' maps/ "$APP/maps/"
echo "[bundle-ios] $APP $VER ($BUILD), $(du -sh "$APP" | cut -f1)"
