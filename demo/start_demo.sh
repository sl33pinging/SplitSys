#!/usr/bin/env bash
# SplitSys dashboard from WSL (development / co-sim; the booth uses start_demo.ps1).
#   demo/start_demo.sh [transport] [port] [baud]   e.g. demo/start_demo.sh mock 8080
#                                                   demo/start_demo.sh tcp:127.0.0.1:5555
#                                                   demo/start_demo.sh serial:/dev/ttyUSB1 8080 115200
# baud (serial only): auto (default: 1562500 = KG3+, then 115200 = KG2, probed at PING) or a rate.
# Owner: host-dashboard.
set -eu
root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=/dev/null
source "$root/scripts/env.sh"
transport="${1:-mock}"
port="${2:-8080}"
baud="${3:-auto}"
cd "$root/sw" && cargo build -q --locked -p splitsys-host
exe="$CARGO_TARGET_DIR/debug/splitsys-host"
cd "$root"
echo "dashboard: http://127.0.0.1:$port/  (open it from Windows; WSL2 forwards localhost)"
exec "$exe" --transport "$transport" --baud "$baud" serve --port "$port"
