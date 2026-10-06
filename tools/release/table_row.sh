#!/usr/bin/env bash
# Add a file to the download table of a GitHub Release's notes. Usage:
#   tools/release/table_row.sh <tag> <asset name> <platform>
# e.g. tools/release/table_row.sh v0.2.8 bongbong-macos.dmg "macOS (Apple Silicon and Intel)"
# cargo-dist writes the release body - the "Download bongbong X.Y.Z" table,
# one row per archive - when it creates the release, before
# android-release.yml and macos-release.yml have uploaded the APK and the
# dmg, so those sit in the assets list under the table but not in it. This
# appends a row after the table's last one, in the same shape (file,
# platform, checksum - the asset's `.sha256` beside it) and idempotently: a
# body that already names the asset is left alone. The notes are read and
# written whole, so the workflows that call it share one concurrency group
# per tag (`release-notes-<tag>`) and never overwrite each other's row.
# Needs `gh` logged in with write access to the repo's releases.
set -euo pipefail
TAG="$1"
ASSET="$2"
PLATFORM="$3"
REPO="${GH_REPO:-$(gh repo view --json nameWithOwner -q .nameWithOwner)}"
BASE="https://github.com/$REPO/releases/download/$TAG"
ROW="| [$ASSET]($BASE/$ASSET) | $PLATFORM | [checksum]($BASE/$ASSET.sha256) |"
BODY="$(gh release view "$TAG" --repo "$REPO" --json body -q .body)"
if grep -qF "$ASSET" <<<"$BODY"; then
    echo "[release-table] $TAG already lists $ASSET"
    exit 0
fi
# The last table row is the last line that starts a `| [` link cell; the
# new row goes right after it. A body with no table gets the row appended.
NEW="$(ROW="$ROW" awk '
    { lines[NR] = $0; if ($0 ~ /^\| \[/) last = NR }
    END {
        for (i = 1; i <= NR; i++) { print lines[i]; if (i == last) print ENVIRON["ROW"] }
        if (!last) print ENVIRON["ROW"]
    }' <<<"$BODY")"
gh release edit "$TAG" --repo "$REPO" --notes-file - <<<"$NEW" >/dev/null
echo "[release-table] $TAG: added $ASSET to the download table"
