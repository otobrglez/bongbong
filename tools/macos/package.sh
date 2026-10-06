#!/usr/bin/env bash
# Sign, notarize and package target/macos/BongBong.app (tools/macos/bundle.sh)
# as target/macos/bongbong-macos.dmg - what a Mac downloads from a release:
# drag the app to Applications, double-click it, and Gatekeeper lets it
# open. Usage: tools/macos/package.sh [--no-notarize]
#
# 1. The app is signed with the Developer ID Application certificate in
#    the keychain, under the hardened runtime and with a secure timestamp
#    (all three are what notarization asks for).
# 2. It goes to Apple's notary service and the ticket is stapled to it, so
#    it opens even offline.
# 3. The dmg (the app and an Applications link) is signed, notarized and
#    stapled the same way.
#
#   MACOS_SIGN_IDENTITY  the certificate to sign with (default: the first
#                        "Developer ID Application" identity in the keychain)
#   ASC_KEY_ID, ASC_ISSUER_ID, ASC_KEY_PATH
#                        the App Store Connect API key notarytool signs in
#                        with - the one tools/ios/testflight.sh uses; the
#                        key defaults to
#                        ~/.appstoreconnect/private_keys/AuthKey_<ASC_KEY_ID>.p8
#
# With no Developer ID certificate the app is signed ad hoc and nothing is
# notarized: a dmg that runs on this Mac, and that Gatekeeper blocks on any
# other. --no-notarize signs with the certificate but skips Apple.
set -euo pipefail
cd "$(dirname "$0")/../.."
# The devenv shell points DEVELOPER_DIR at nix's SDK, which has no
# notarytool or stapler: use Xcode's, as tools/ios/env.sh does.
if ! xcrun -f notarytool >/dev/null 2>&1; then
    export DEVELOPER_DIR="${BONGBONG_DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"
    unset SDKROOT
fi
OUT=target/macos
APP="$OUT/BongBong.app"
DMG="$OUT/bongbong-macos.dmg"
NOTARIZE=1
[[ "${1:-}" == "--no-notarize" ]] && NOTARIZE=0
fail() { echo "[package-macos] $*" >&2; exit 1; }
[[ -d "$APP" ]] || fail "$APP missing: run tools/macos/bundle.sh first"

IDENTITY="${MACOS_SIGN_IDENTITY:-$(security find-identity -v -p codesigning \
    | sed -n 's/.*"\(Developer ID Application: .*\)"$/\1/p' | head -1)}"
if [[ -z "$IDENTITY" ]]; then
    echo "[package-macos] no Developer ID Application certificate: signing ad hoc, not notarizing" >&2
    IDENTITY=-
    NOTARIZE=0
fi

if (( NOTARIZE )); then
    [[ -n "${ASC_KEY_ID:-}" && -n "${ASC_ISSUER_ID:-}" ]] || fail "set ASC_KEY_ID and ASC_ISSUER_ID (or pass --no-notarize)"
    KEY="${ASC_KEY_PATH:-$HOME/.appstoreconnect/private_keys/AuthKey_$ASC_KEY_ID.p8}"
    [[ -f "$KEY" ]] || fail "no API key at $KEY (or set ASC_KEY_PATH)"
fi

# Submit a file to the notary service and staple its ticket to `$2`,
# printing Apple's log when it is refused.
notarize() {
    local file="$1" staple="$2" result id
    echo "[package-macos] notarizing $(basename "$file")"
    result="$(xcrun notarytool submit "$file" --key "$KEY" --key-id "$ASC_KEY_ID" \
        --issuer "$ASC_ISSUER_ID" --wait --timeout 30m --output-format json)"
    id="$(plutil -extract id raw - <<<"$result")"
    if [[ "$(plutil -extract status raw - <<<"$result")" != Accepted ]]; then
        xcrun notarytool log "$id" --key "$KEY" --key-id "$ASC_KEY_ID" --issuer "$ASC_ISSUER_ID" >&2 || true
        fail "the notary service refused $(basename "$file") (submission $id)"
    fi
    xcrun stapler staple "$staple"
}

# The dmg takes a timestamped signature; the app the hardened runtime too.
dmg_flags=(--force --sign "$IDENTITY")
[[ "$IDENTITY" != - ]] && dmg_flags+=(--timestamp)
app_flags=("${dmg_flags[@]}")
[[ "$IDENTITY" != - ]] && app_flags+=(--options runtime)

echo "[package-macos] signing BongBong.app as ${IDENTITY}"
codesign "${app_flags[@]}" "$APP"
codesign --verify --strict --verbose=2 "$APP"
if (( NOTARIZE )); then
    ditto -c -k --keepParent "$APP" "$OUT/BongBong.zip"
    notarize "$OUT/BongBong.zip" "$APP"
    rm -f "$OUT/BongBong.zip"
fi

# hdiutil's verbs are deprecated in favour of diskutil's on newer macOS,
# which the runners do not all have yet; the warning is noise.
hdi() { hdiutil "$@" 2> >(grep -v 'is deprecated' >&2) >/dev/null; }

# The disk: the app, an Applications link to drag it onto, and the app's
# icon as the volume's (a .VolumeIcon.icns at its root, and the root's
# Finder flags saying it has a custom icon - set on a writable copy, which
# is then compressed).
STAGE="$OUT/dmg"
RW="$OUT/rw.dmg"
MNT="$OUT/mnt"
rm -rf "$STAGE" "$DMG" "$RW" "$MNT" && mkdir -p "$STAGE" "$MNT"
ditto "$APP" "$STAGE/BongBong.app"
ln -s /Applications "$STAGE/Applications"
cp "$APP/Contents/Resources/AppIcon.icns" "$STAGE/.VolumeIcon.icns"
hdi create -volname BongBong -srcfolder "$STAGE" -fs HFS+ -format UDRW -ov "$RW"
hdi attach "$RW" -nobrowse -noautoopen -mountpoint "$MNT"
# kHasCustomIcon, 0x0400 of the Finder flags: the ninth byte of the 32.
xattr -wx com.apple.FinderInfo 0000000000000000040000000000000000000000000000000000000000000000 "$MNT"
hdi detach "$MNT"
hdi convert "$RW" -format UDZO -o "$DMG"
rm -rf "$STAGE" "$RW" "$MNT"
codesign "${dmg_flags[@]}" "$DMG"
# A download that does not mount is the worst way for a release to fail:
# check the image's checksums before it goes anywhere.
hdi verify "$DMG" || fail "$DMG does not verify"
if (( NOTARIZE )); then
    notarize "$DMG" "$DMG"
fi

shasum -a 256 "$DMG" | sed "s#  $OUT/#  #" > "$DMG.sha256"
if [[ "$IDENTITY" == - ]]; then how="signed ad hoc, not notarized"
elif (( NOTARIZE )); then how="signed and notarized"
else how="signed, not notarized"; fi
echo "[package-macos] $DMG ($(du -h "$DMG" | cut -f1)): $how"
