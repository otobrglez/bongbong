#!/usr/bin/env bash
# Print one release's entry from CHANGELOG.md - its heading and everything
# up to the next release's heading. Usage: tools/release/entry.sh X.Y.Z
# Exits 1 when the changelog has no entry for that version, which is how a
# release would go out with only its download table.
set -euo pipefail
VERSION="${1#v}"
FILE="${2:-CHANGELOG.md}"
awk -v v="$VERSION" '
    /^## / { if (on) exit; if ($2 == v) on = 1 }
    on { print }
    END { exit !on }
' "$FILE"
