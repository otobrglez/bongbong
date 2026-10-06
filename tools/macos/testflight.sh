#!/usr/bin/env bash
# Build the Mac app, sign it for the App Store and upload it to App Store
# Connect, where it appears in TestFlight under macOS once processed
# (`just macos-testflight`) - the twin of tools/ios/testflight.sh.
#
#   tools/macos/testflight.sh [--export]
#
# --export stops at a signed .pkg in target/macos/appstore/export/ instead
# of uploading (to check the signing without spending a build number).
#
# TestFlight and the Mac App Store take only a sandboxed app, so this signs
# a copy of BongBong.app (tools/macos/bundle.sh) with
# tools/macos/AppStore.entitlements: the sandbox, and outgoing connections
# for the online rooms. Inside the sandbox HOME is the app's container, so
# the progress file and saved maps (`levels::data_dir`) land there with no
# change to the game. The dmg (tools/macos/package.sh) is not sandboxed.
#
# Then, as on iOS: an .xcarchive around the app, and `xcodebuild
# -exportArchive` with method app-store-connect, which with
# -allowProvisioningUpdates and an App Store Connect API key creates or
# reuses Apple's cloud-managed distribution and installer certificates and
# the Mac App Store profile for the bundle id, re-signs the app with the
# entitlements it carries, wraps it in a .pkg and uploads it.
#
# CI runs the same script (.github/workflows/testflight.yml's macos job).
# Needs, in .envrc or the environment:
#   BONGBONG_IOS_TEAM    the paid team's ID (the iOS script's; one team)
#   ASC_KEY_ID, ASC_ISSUER_ID, ASC_KEY_PATH
#                        the App Store Connect API key, as tools/ios/testflight.sh
#   MACOS_BUILD          optional; the build number (tools/macos/bundle.sh)
# and, once, on the web: the macOS platform added to the app's record in
# App Store Connect (the bundle id is registered for every platform).
set -euo pipefail
cd "$(dirname "$0")/../.."
DEST=upload
for arg in "$@"; do
    case "$arg" in
        --export) DEST="export" ;;
        *) echo "[testflight-macos] unknown argument $arg" >&2; exit 2 ;;
    esac
done
log() { echo "[testflight-macos] $*"; }
fail() { echo "[testflight-macos] FAIL: $*" >&2; exit 1; }

BUNDLE_ID=com.otobrglez.bongbong
TEAM="${BONGBONG_IOS_TEAM:-}"
[[ -n "$TEAM" ]] || fail "set BONGBONG_IOS_TEAM in .envrc to the team's ID (developer.apple.com > Membership details)"
[[ -n "${ASC_KEY_ID:-}" && -n "${ASC_ISSUER_ID:-}" ]] \
    || fail "set ASC_KEY_ID and ASC_ISSUER_ID in .envrc (App Store Connect > Users and Access > Integrations)"
KEY="${ASC_KEY_PATH:-$HOME/.appstoreconnect/private_keys/AuthKey_$ASC_KEY_ID.p8}"
[[ -f "$KEY" ]] || fail "no API key at $KEY (download the .p8 there, or set ASC_KEY_PATH)"
# xcodebuild is Xcode's, not the devenv shell's SDK (tools/ios/env.sh).
XCODE_DIR="${BONGBONG_DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"

# --- 1. Build, and sign a sandboxed copy -----------------------------------
./tools/macos/bundle.sh
OUT=target/macos/appstore
APP="$OUT/BongBong.app"
rm -rf "$OUT" && mkdir -p "$OUT"
ditto target/macos/BongBong.app "$APP"
PB=/usr/libexec/PlistBuddy
VER=$("$PB" -c "Print :CFBundleShortVersionString" "$APP/Contents/Info.plist")
BUILD=$("$PB" -c "Print :CFBundleVersion" "$APP/Contents/Info.plist")
# Ad hoc: the export re-signs with the distribution certificate and keeps
# the entitlements this signature carries.
codesign --force --sign - --timestamp=none --entitlements tools/macos/AppStore.entitlements "$APP"

# --- 2. The archive ----------------------------------------------------------
ARCHIVE="$OUT/BongBong.xcarchive"
mkdir -p "$ARCHIVE/Products/Applications"
ditto "$APP" "$ARCHIVE/Products/Applications/BongBong.app"
"$PB" -c "Add :ArchiveVersion integer 2" \
    -c "Add :CreationDate date $(date -u '+%a %b %d %H:%M:%S UTC %Y')" \
    -c "Add :Name string BongBong" \
    -c "Add :SchemeName string BongBong" \
    -c "Add :ApplicationProperties dict" \
    -c "Add :ApplicationProperties:ApplicationPath string Applications/BongBong.app" \
    -c "Add :ApplicationProperties:CFBundleIdentifier string $BUNDLE_ID" \
    -c "Add :ApplicationProperties:CFBundleShortVersionString string $VER" \
    -c "Add :ApplicationProperties:CFBundleVersion string $BUILD" \
    -c "Add :ApplicationProperties:Team string $TEAM" \
    -c "Add :ApplicationProperties:Architectures array" \
    "$ARCHIVE/Info.plist" >/dev/null
i=0
for arch in $(lipo -archs "$APP/Contents/MacOS/bongbong"); do
    "$PB" -c "Add :ApplicationProperties:Architectures:$i string $arch" "$ARCHIVE/Info.plist"
    i=$((i + 1))
done

OPTIONS="$OUT/ExportOptions.plist"
"$PB" -c "Add :method string app-store-connect" \
    -c "Add :destination string $DEST" \
    -c "Add :signingStyle string automatic" \
    -c "Add :teamID string $TEAM" \
    -c "Add :uploadSymbols bool false" \
    -c "Add :manageAppVersionAndBuildNumber bool false" \
    "$OPTIONS" >/dev/null

# --- 3. Sign, and upload or export -------------------------------------------
EXPORT="$OUT/export"
log "$BUNDLE_ID $VER ($BUILD, $(lipo -archs "$APP/Contents/MacOS/bongbong")): exportArchive, destination $DEST"
# A clean environment, as on iOS: the devenv shell's CC/CXX/LD read as
# build-setting overrides to xcodebuild.
env -i HOME="$HOME" USER="${USER:-}" PATH=/usr/bin:/bin:/usr/sbin:/sbin DEVELOPER_DIR="$XCODE_DIR" \
    xcodebuild -exportArchive -archivePath "$ARCHIVE" -exportOptionsPlist "$OPTIONS" \
    -exportPath "$EXPORT" -allowProvisioningUpdates \
    -authenticationKeyPath "$KEY" -authenticationKeyID "$ASC_KEY_ID" \
    -authenticationKeyIssuerID "$ASC_ISSUER_ID" 2>&1 | tee "$OUT/export.log" \
    | grep -vE '^warning: unhandled Platform key|^\s*$' || true
grep -q "EXPORT SUCCEEDED\|Upload succeeded\|Uploaded " "$OUT/export.log" \
    || fail "exportArchive did not succeed; the whole log is $OUT/export.log"
if [[ "$DEST" == export ]]; then
    log "signed $(ls "$EXPORT"/*.pkg)"
else
    log "uploaded $VER ($BUILD). App Store Connect processes it for a few minutes, then it is in TestFlight under macOS."
fi
