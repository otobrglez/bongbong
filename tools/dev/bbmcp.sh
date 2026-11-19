#!/usr/bin/env bash
# Start the MCP adapter (src/bin/bbmcp.rs) the way .mcp.json needs it: as a
# binary that is already built, so the client's connect timeout is never
# spent inside cargo. Arguments pass through (`rooms` for the room server).
#
# The binary is this checkout's `target/debug/bbmcp` (`just build-mcp`). A
# worktree that has not built one yet borrows the main checkout's; the
# adapter then reports itself stale and its `reload` tool builds this
# checkout's own. With no binary anywhere it is built here, which can take
# longer than the client waits: run `just build-mcp` and reconnect.
set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$root"

exe="target/debug/bbmcp"
if [ ! -x "$exe" ]; then
    # In a worktree the common git directory is the main checkout's .git.
    common="$(git rev-parse --path-format=absolute --git-common-dir 2>/dev/null || true)"
    borrowed="${common%/.git}/target/debug/bbmcp"
    if [ -n "$common" ] && [ "${common%/.git}" != "$root" ] && [ -x "$borrowed" ]; then
        echo "bbmcp: no adapter built in $root - running $borrowed (call \`reload\`, or \`just build-mcp\`, for this checkout's own)" >&2
        exec "$borrowed" "$@"
    fi
    echo "bbmcp: no adapter built yet - building it (the client may time out first; \`just build-mcp\`, then reconnect)" >&2
    cargo build --no-default-features --features dev-tools --bin bbmcp >&2
fi
exec "$exe" "$@"
