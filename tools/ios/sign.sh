#!/usr/bin/env bash
# Sign target/ios-device/BongBong.app for the connected iPhone.
#
# A device build needs an Apple Development certificate and a provisioning
# profile for the bundle id that lists the phone. Both come from Xcode's
# automatic signing, so this script builds the placeholder project in
# tools/ios/sign/ (one empty main.m, the game's bundle id, the Personal
# Team) with -allowProvisioningUpdates: Xcode registers the connected phone,
# mints the certificate and the profile, and installs the profile under
# ~/Library/Developer/Xcode/UserData/Provisioning Profiles/. The script then
# signs the real bundle with that identity, the profile embedded and the
# entitlements Xcode used. The phone must be plugged in, unlocked, trusted,
# and in Developer Mode (Settings > Privacy & Security). Personal Team
# profiles last seven days; rerun to renew.
set -euo pipefail
cd "$(dirname "$0")/../.."
export IOS_SLICE=ios
source tools/ios/env.sh
APP=target/ios-device/BongBong.app
BUNDLE_ID=com.otobrglez.bongbong
TEAM="${BONGBONG_IOS_TEAM:-PFET672VQB}"
[[ -d "$APP" ]] || { echo "[sign-ios] $APP missing: run 'just build-ios-device' first" >&2; exit 1; }

UDID="${BONGBONG_IOS_UDID:-$(xcrun devicectl list devices --json-output /tmp/bb-devices.json >/dev/null 2>&1; python3 -c '
import json,sys
d=json.load(open("/tmp/bb-devices.json"))
for dev in d["result"]["devices"]:
    if dev.get("connectionProperties",{}).get("transportType") not in (None,"None"):
        print(dev["hardwareProperties"]["udid"]); break
')}"
[[ -n "$UDID" ]] || { echo "[sign-ios] no connected iPhone (xcrun devicectl list devices): plug it in, unlock it, trust this Mac" >&2; exit 1; }
echo "[sign-ios] device $UDID"
mkdir -p target/ios-device && printf '%s' "$UDID" > target/ios-device/udid

DD=target/ios-device/sign-derived
# A clean environment: the devenv shell exports CC/CXX/LD for nix's
# toolchain and xcodebuild treats those as build-setting overrides (its
# link step then runs `ld` with clang flags and fails).
env -i HOME="$HOME" USER="${USER:-}" PATH=/usr/bin:/bin:/usr/sbin:/sbin DEVELOPER_DIR="$DEVELOPER_DIR" \
    xcodebuild -project tools/ios/sign/BongBongSign.xcodeproj -scheme BongBongSign -configuration Debug \
    -sdk iphoneos -destination "id=$UDID" -derivedDataPath "$DD" \
    -allowProvisioningUpdates -allowProvisioningDeviceRegistration \
    DEVELOPMENT_TEAM="$TEAM" build 2>&1 | grep -E "error:|Signing Identity|Provisioning Profile|BUILD" || true
STUB=$(ls -d "$DD"/Build/Products/Debug-iphoneos/BongBongSign.app 2>/dev/null | head -1)
[[ -d "$STUB" ]] || { echo "[sign-ios] Xcode did not produce a signed stub; see the errors above" >&2; exit 1; }

IDENTITY=$(security find-identity -v -p codesigning | sed -n 's/.*"\(Apple Development: [^"]*\)".*/\1/p' | head -1)
[[ -n "$IDENTITY" ]] || { echo "[sign-ios] no Apple Development identity in the keychain" >&2; exit 1; }
cp "$STUB/embedded.mobileprovision" "$APP/embedded.mobileprovision"
ENT=target/ios-device/entitlements.plist
codesign -d --entitlements :- "$STUB" > "$ENT" 2>/dev/null
plutil -lint "$ENT" >/dev/null
codesign --force --sign "$IDENTITY" --entitlements "$ENT" --timestamp=none "$APP"
codesign --verify --verbose=2 "$APP"
echo "[sign-ios] signed $APP as '$IDENTITY' for device $UDID"
