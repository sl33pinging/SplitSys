#!/usr/bin/env bash
# SplitSys dashboard from WSL (development / co-sim; the booth uses start_demo.ps1).
#   demo/start_demo.sh [transport] [port]     e.g. demo/start_demo.sh mock 8080
#                                                   demo/start_demo.sh tcp:127.0.0.1:5555
# Owner: host-dashboard.
set -eu
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=/dev/null
source "$root/scripts/env.sh"
transport="${1:-mock}"
port="${2:-8080}"
cd "$root/sw" && cargo build -q --locked -p splitsys-host
exe="$CARGO_TARGET_DIR/debug/splitsys-host"
cd "$root"
echo "dashboard: http://127.0.0.1:$port/  (open it from Windows; WSL2 forwards localhost)"
exec "$exe" --transport "$transport" serve --port "$port"
