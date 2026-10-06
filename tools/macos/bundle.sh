#!/usr/bin/env bash
# Build target/macos/BongBong.app: the game for Apple Silicon and Intel in
# one binary (a release build per architecture, joined by lipo), Info.plist
# from the template, AppIcon.icns made from tools/macos/AppIcon.png, and
# static/ in Contents/Resources - the binary moves there at startup
# (src/app/macos.rs), and every map, stamp and level is compiled in. No
# signing: tools/macos/package.sh signs, notarizes and wraps it in a dmg.
#
#   MACOS_ARCHS  the architectures to build (default "aarch64 x86_64"; one
#                builds a thinner app, e.g. MACOS_ARCHS=aarch64 for a quick
#                local try where the x86_64 std is not installed)
#   MACOS_MIN    the oldest macOS it runs on (default 11.0, the first with
#                Apple Silicon), for rustc, cc-rs, cmake and the plist alike
#   MACOS_BUILD  CFBundleVersion, the build number App Store Connect needs
#                raised on every upload (default the UTC time, YYYYMMDD.HHMM)
set -euo pipefail
cd "$(dirname "$0")/../.."
ARCHS="${MACOS_ARCHS:-aarch64 x86_64}"
export MACOSX_DEPLOYMENT_TARGET="${MACOS_MIN:-11.0}"
OUT=target/macos
APP="$OUT/BongBong.app"
VER="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
BUILD="${MACOS_BUILD:-$(date -u +%Y%m%d.%H%M)}"

bins=()
for arch in $ARCHS; do
    target="$arch-apple-darwin"
    echo "[bundle-macos] building $target (macOS $MACOSX_DEPLOYMENT_TARGET+)"
    cargo build --release --locked --bin bongbong --target "$target"
    bins+=("target/$target/release/bongbong")
done

rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
lipo -create -output "$APP/Contents/MacOS/bongbong" "${bins[@]}"
# Every Mac has what the binary links, or it does not start. Built in the
# devenv shell it links nix's libiconv, which has the same interface as the
# system's: point it there. Anything else from outside the system is an
# error.
while read -r lib; do
    case "$lib" in
        /System/*|/usr/lib/*) ;;
        /nix/store/*/libiconv.2.dylib)
            install_name_tool -change "$lib" /usr/lib/libiconv.2.dylib "$APP/Contents/MacOS/bongbong" ;;
        *) echo "[bundle-macos] the binary links $lib, which a Mac does not have" >&2; exit 1 ;;
    esac
done < <(otool -L "$APP/Contents/MacOS/bongbong" | awk 'NR > 1 && !/:$/ { print $1 }' | sort -u)
sed -e "s/__VERSION__/$VER/" -e "s/__BUILD__/$BUILD/" -e "s/__MIN_OS__/$MACOSX_DEPLOYMENT_TARGET/" \
    tools/macos/Info.plist > "$APP/Contents/Info.plist"
plutil -lint "$APP/Contents/Info.plist" >/dev/null

# What the desktop game reads: not the web build's GLSL ES shaders, the
# art backups or the ground tileset's pristine originals.
rsync -a --exclude '_backup' --exclude '_original' --exclude '/static/web' static "$APP/Contents/Resources/"
# The required-reason APIs the binary imports, the same Rust as the iOS app's.
cp tools/ios/PrivacyInfo.xcprivacy "$APP/Contents/Resources/"

ICONSET="$OUT/AppIcon.iconset"
rm -rf "$ICONSET" && mkdir -p "$ICONSET"
for size in 16 32 128 256 512; do
    sips -z "$size" "$size" tools/macos/AppIcon.png --out "$ICONSET/icon_${size}x${size}.png" >/dev/null
    sips -z $((size * 2)) $((size * 2)) tools/macos/AppIcon.png --out "$ICONSET/icon_${size}x${size}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/AppIcon.icns"
rm -rf "$ICONSET"

echo "[bundle-macos] $APP: $VER ($BUILD), $(lipo -archs "$APP/Contents/MacOS/bongbong")"
