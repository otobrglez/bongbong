#!/usr/bin/env bash
# The app's two version numbers, as tools/ios/bundle.sh writes them into
# Info.plist: prints "VERSION BUILD" (CFBundleShortVersionString, then
# CFBundleVersion).
#
#   tools/ios/version.sh [Cargo.toml]
#
# A release is Cargo.toml's version, built at the UTC minute:
# 0.2.6 (20261001.1006). A pull request's build (BONGBONG_IOS_PR=<N>, set by
# .github/workflows/testflight.yml) gets a version of its own:
#
#   MAJOR.MINOR.<PR_TRAIN_BASE + N> (YYYYMMDD.HHMM.<N>)    0.2.10068 (20261002.0441.68)
#
# App Store Connect groups builds by version (a "train") and TestFlight
# lists them that way, so a PR built on Cargo.toml's version would land in
# that release's train, newer than the release's own build. A version per PR
# keeps each PR in a train of its own and leaves the releases' alone. Apple
# takes only digits in a version, at most three numbers (ITMS-90060), so the
# PR cannot be a suffix: it is the patch, offset by PR_TRAIN_BASE so it can
# never be a release's patch. Keeping MAJOR.MINOR keeps the PR's version
# above the release it was built on, since App Store Connect refuses a
# version below one it has approved for the App Store (ITMS-90062). The
# build number keeps the PR as its third part, which is how the workflow's
# `expire` job finds a closed PR's builds.
#
# BONGBONG_IOS_BUILD overrides the build number (one to three numbers).
set -euo pipefail
PR_TRAIN_BASE=10000
fail() { echo "[ios-version] $*" >&2; exit 1; }

CARGO_TOML="${1:-$(dirname "$0")/../../Cargo.toml}"
[[ -f "$CARGO_TOML" ]] || fail "no $CARGO_TOML"
CARGO_VER="$(sed -n 's/^version = "\(.*\)"/\1/p' "$CARGO_TOML" | head -1)"
[[ -n "$CARGO_VER" ]] || fail "no version in $CARGO_TOML"

PR="${BONGBONG_IOS_PR:-}"
if [[ -n "$PR" ]]; then
    [[ "$PR" =~ ^[1-9][0-9]*$ ]] || fail "BONGBONG_IOS_PR '$PR' is not a pull request number"
    [[ "$CARGO_VER" =~ ^(0|[1-9][0-9]*)\.(0|[1-9][0-9]*)\. ]] \
        || fail "$CARGO_TOML's version '$CARGO_VER' does not start MAJOR.MINOR."
    VERSION="${BASH_REMATCH[1]}.${BASH_REMATCH[2]}.$((PR_TRAIN_BASE + PR))"
    SUFFIX=".$PR"
else
    VERSION="$CARGO_VER"
    SUFFIX=""
fi

BUILD="${BONGBONG_IOS_BUILD:-$(date -u +%Y%m%d.%H%M)$SUFFIX}"
[[ "$BUILD" =~ ^[0-9]+(\.[0-9]+){0,2}$ ]] \
    || fail "build number '$BUILD' is not one to three numbers separated by periods"

echo "$VERSION $BUILD"
