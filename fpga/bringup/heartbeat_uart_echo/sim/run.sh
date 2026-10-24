#!/usr/bin/env bash
# Smoke test for the heartbeat_uart_echo bring-up top (iverilog, runs in WSL).
#   From Git Bash:  scripts/wsl 'fpga/bringup/heartbeat_uart_echo/sim/run.sh'
#   From WSL:       fpga/bringup/heartbeat_uart_echo/sim/run.sh
# Exit 0 only if the testbench prints "SMOKE PASS".
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
top="$(cd "$here/.." && pwd)"
root="$(cd "$top/../../.." && pwd)"
out="${TMPDIR:-/tmp}/splitsys_sim_heartbeat_uart_echo"
mkdir -p "$out"
srcs=()
while read -r f; do
  f="${f%%#*}"; f="${f//[[:space:]]/}"
  [ -n "$f" ] && srcs+=("$root/$f")
done < "$top/sources.f"
iverilog -g2012 -Wall -o "$out/sim.vvp" -s tb_heartbeat_uart_echo \
  "${srcs[@]}" "$top/heartbeat_uart_echo.sv" "$here/tb_heartbeat_uart_echo.sv"
vvp -n "$out/sim.vvp" | tee "$out/sim.log"
grep -q "^SMOKE PASS" "$out/sim.log"
