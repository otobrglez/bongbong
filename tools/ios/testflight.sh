#!/usr/bin/env bash
# Build the device app, sign it for the App Store and upload it to App Store
# Connect, where it appears in TestFlight once processed (`just ios-testflight`).
#
#   tools/ios/testflight.sh [--export]
#
# --export stops at a signed .ipa in target/ios-device/export/ instead of
# uploading (to check the signing without spending a build number).
#
# There is no Xcode project for the game, so the script makes the one thing
# Xcode's distribution step takes as input: an .xcarchive around the bundle
# tools/ios/bundle.sh stages (Products/Applications/BongBong.app plus the
# archive's own Info.plist). `xcodebuild -exportArchive` then does what the
# Organizer's "Distribute App" does: with -allowProvisioningUpdates and an
# App Store Connect API key it creates or reuses the Apple Distribution
# certificate (cloud-managed, so no private key needs to be on this Mac) and
# the App Store profile for the bundle id, re-signs the app, and uploads it.
#
# CI runs the same script (.github/workflows/testflight.yml, on version tags),
# with the key written from repo secrets. Needs, in .envrc or the environment:
#   BONGBONG_IOS_TEAM    the paid team's ID (developer.apple.com > Membership)
#   ASC_KEY_ID           an App Store Connect Team API key (Users and Access >
#   ASC_ISSUER_ID        Integrations), Admin role - cloud-managed signing
#                        needs it
#   ASC_KEY_PATH         optional; default ~/.appstoreconnect/private_keys/AuthKey_<ASC_KEY_ID>.p8
# and, once, on the web: the App ID com.otobrglez.bongbong registered under
# that team and an app record for it in App Store Connect.
set -euo pipefail
cd "$(dirname "$0")/../.."
export IOS_SLICE=ios
source tools/ios/env.sh
DEST=upload
for arg in "$@"; do
    case "$arg" in
        --export) DEST=export ;;
        *) echo "[testflight] unknown argument $arg" >&2; exit 2 ;;
    esac
done
log() { echo "[testflight] $*"; }
fail() { echo "[testflight] FAIL: $*" >&2; exit 1; }

BUNDLE_ID=com.otobrglez.bongbong
TEAM="${BONGBONG_IOS_TEAM:-}"
[[ -n "$TEAM" ]] || fail "set BONGBONG_IOS_TEAM in .envrc to the team's ID (developer.apple.com > Membership details)"
[[ -n "${ASC_KEY_ID:-}" && -n "${ASC_ISSUER_ID:-}" ]] \
    || fail "set ASC_KEY_ID and ASC_ISSUER_ID in .envrc (App Store Connect > Users and Access > Integrations)"
KEY="${ASC_KEY_PATH:-$HOME/.appstoreconnect/private_keys/AuthKey_$ASC_KEY_ID.p8}"
[[ -f "$KEY" ]] || fail "no API key at $KEY (download the .p8 there, or set ASC_KEY_PATH)"

# --- 1. Build and stage ------------------------------------------------------
# release, not dist: dist's LTO internalises std's __isPlatformVersionAtLeast,
# which SDL3's Objective-C objects (@available) reference from outside it,
# and the link fails.
PROFILE=release
log "cargo build --$PROFILE (aarch64-apple-ios)"
cargo build --target aarch64-apple-ios --bin bongbong --release
OUT=target/ios-device
./tools/ios/bundle.sh "$PROFILE"
APP="$OUT/BongBong.app"
PB=/usr/libexec/PlistBuddy
VER=$("$PB" -c "Print :CFBundleShortVersionString" "$APP/Info.plist")
BUILD=$("$PB" -c "Print :CFBundleVersion" "$APP/Info.plist")
# The deploy flow's development profile and signature must not ride along.
rm -f "$APP/embedded.mobileprovision"
codesign --force --sign - --timestamp=none "$APP"

# --- 2. The archive ------------------------------------------------------------
ARCHIVE="$OUT/BongBong.xcarchive"
rm -rf "$ARCHIVE" && mkdir -p "$ARCHIVE/Products/Applications"
cp -R "$APP" "$ARCHIVE/Products/Applications/"
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
    -c "Add :ApplicationProperties:Architectures:0 string arm64" \
    "$ARCHIVE/Info.plist" >/dev/null

OPTIONS="$OUT/ExportOptions.plist"
rm -f "$OPTIONS"
"$PB" -c "Add :method string app-store-connect" \
    -c "Add :destination string $DEST" \
    -c "Add :signingStyle string automatic" \
    -c "Add :teamID string $TEAM" \
    -c "Add :uploadSymbols bool false" \
    -c "Add :manageAppVersionAndBuildNumber bool false" \
    "$OPTIONS" >/dev/null

# --- 3. Sign, and upload or export ---------------------------------------------
EXPORT="$OUT/export"
rm -rf "$EXPORT"
log "$BUNDLE_ID $VER ($BUILD): exportArchive, destination $DEST"
# A clean environment, as in sign.sh: the devenv shell's CC/CXX/LD read as
# build-setting overrides to xcodebuild.
env -i HOME="$HOME" USER="${USER:-}" PATH=/usr/bin:/bin:/usr/sbin:/sbin DEVELOPER_DIR="$DEVELOPER_DIR" \
    xcodebuild -exportArchive -archivePath "$ARCHIVE" -exportOptionsPlist "$OPTIONS" \
    -exportPath "$EXPORT" -allowProvisioningUpdates \
    -authenticationKeyPath "$KEY" -authenticationKeyID "$ASC_KEY_ID" \
    -authenticationKeyIssuerID "$ASC_ISSUER_ID" 2>&1 | tee "$OUT/export.log" \
    | grep -vE '^warning: unhandled Platform key|^\s*$' || true
grep -q "EXPORT SUCCEEDED\|Upload succeeded\|Uploaded " "$OUT/export.log" \
    || fail "exportArchive did not succeed; the whole log is $OUT/export.log"
if [[ "$DEST" == export ]]; then
    log "signed $(ls "$EXPORT"/*.ipa)"
else
    log "uploaded $VER ($BUILD). App Store Connect processes it for a few minutes, then it is in TestFlight."
fi
