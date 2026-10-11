#!/usr/bin/env bash
# Show the booted simulator in a window (`just run-ios-sim`, `just ios-smoke`),
# opened from the Xcode tools/ios/env.sh picks, so the window belongs to the
# Xcode whose simctl booted the device: Simulator.app up to Xcode 26 (in
# Contents/Developer/Applications), DeviceHub.app from Xcode 27 (in
# Contents/Applications). Neither there, or `open` refused: it says so and
# exits 0 - simctl runs the device headless and `just ios-screenshot` still
# sees it; only touches need the window.
set -euo pipefail
cd "$(dirname "$0")/../.."
source tools/ios/env.sh
CONTENTS="$(dirname "$DEVELOPER_DIR")"
for app in "$DEVELOPER_DIR/Applications/Simulator.app" "$CONTENTS/Applications/DeviceHub.app"; do
    [[ -d "$app" ]] || continue
    open "$app" && exit 0
    echo "[ios-sim] could not open $app: the simulator runs headless" >&2
    exit 0
done
echo "[ios-sim] no Simulator.app or DeviceHub.app under $CONTENTS: the simulator runs headless" >&2
