#!/usr/bin/env bash
# Stage and sign an APK from a NativeActivity shared object, with the
# build-tools alone (aapt2, zipalign, apksigner - no Gradle, no dex, since
# the app has no Java code). Usage:
#   tools/android/package.sh <lib.so> <lib name> <package> <name> <out.apk> [assets dir] [extra permissions xml] [entry function]
# The entry function is the exported symbol NativeActivity calls instead of
# ANativeActivity_onCreate (android.app.func_name); the Rust cdylib needs it.
# The .so is stored uncompressed and page-aligned (zipalign -p), which is
# what Android needs to map it without extracting.
set -euo pipefail
cd "$(dirname "$0")/../.."
source tools/android/env.sh
SO="$1"; LIB_NAME="$2"; PACKAGE="$3"; NAME="$4"; OUT="$5"; ASSETS="${6:-}"; PERMS="${7:-}"; FUNC="${8:-}"
FUNC_META=""
[[ -n "$FUNC" ]] && FUNC_META="            <meta-data android:name=\"android.app.func_name\" android:value=\"$FUNC\" />"
[[ -f "$SO" ]] || { echo "[package-android] $SO missing" >&2; exit 1; }
ANDROID_JAR="$ANDROID_HOME/platforms/android-$ANDROID_PLATFORM_VERSION/android.jar"
[[ -f "$ANDROID_JAR" ]] || ANDROID_JAR="$(ls "$ANDROID_HOME"/platforms/android-*/android.jar | sort -V | tail -1)"
VER="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
# versionCode: 0.0.23 -> 23 (+ 1000*minor + 1000000*major), monotonic per release.
VCODE="$(echo "$VER" | awk -F. '{print $1*1000000 + $2*1000 + $3}')"
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
mkdir -p "$STAGE/lib/$ANDROID_ABI"
cp "$SO" "$STAGE/lib/$ANDROID_ABI/lib$LIB_NAME.so"
sed -e "s/__PACKAGE__/$PACKAGE/g" -e "s/__NAME__/$NAME/g" -e "s/__LIB_NAME__/$LIB_NAME/g" \
    -e "s/__VERSION_CODE__/$VCODE/g" -e "s/__VERSION_NAME__/$VER/g" \
    -e "s/__MIN_SDK__/$ANDROID_API_MIN/g" -e "s/__TARGET_SDK__/$ANDROID_API_TARGET/g" \
    -e "s#__PERMISSIONS__#$PERMS#g" -e "s#__FUNC_NAME__#$FUNC_META#g" tools/android/AndroidManifest.xml > "$STAGE/AndroidManifest.xml"
assets_arg=()
if [[ -n "$ASSETS" ]]; then
    # Keep the directory itself: the game opens "static/x.png", so the APK
    # needs assets/static/x.png.
    name="$(basename "${ASSETS%/}")"
    mkdir -p "$STAGE/assets/$name"
    rsync -a --exclude '_backup/' --exclude '.DS_Store' --exclude '__pycache__/' "${ASSETS%/}/" "$STAGE/assets/$name/"
    assets_arg=(-A "$STAGE/assets")
fi
aapt2 link --manifest "$STAGE/AndroidManifest.xml" -I "$ANDROID_JAR" "${assets_arg[@]}" \
    --min-sdk-version "$ANDROID_API_MIN" --target-sdk-version "$ANDROID_API_TARGET" \
    --version-code "$VCODE" --version-name "$VER" -o "$STAGE/unsigned.apk"
# the native library, stored (-0) so zipalign -p can page-align it
( cd "$STAGE" && zip -q -0 -X unsigned.apk "lib/$ANDROID_ABI/lib$LIB_NAME.so" )
zipalign -p -f 4 "$STAGE/unsigned.apk" "$STAGE/aligned.apk"
mkdir -p "$(dirname "$OUT")"
apksigner sign --ks "$HOME/.android/debug.keystore" --ks-pass pass:android --ks-key-alias androiddebugkey \
    --key-pass pass:android --out "$OUT" "$STAGE/aligned.apk"
echo "[package-android] $OUT ($(du -sh "$OUT" | cut -f1)) - $PACKAGE v$VER ($VCODE), min $ANDROID_API_MIN, target $ANDROID_API_TARGET"
