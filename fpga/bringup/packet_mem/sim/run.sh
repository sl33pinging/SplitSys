#!/usr/bin/env bash
# Smoke test for the packet_mem bring-up top (Verilator --binary --timing, WSL).
#   From Git Bash:  scripts/wsl 'bash fpga/bringup/packet_mem/sim/run.sh'
#   From WSL:       bash fpga/bringup/packet_mem/sim/run.sh
# (Verilator, not iverilog: iverilog 12 does not support constant part-selects
# inside always_* blocks, which rtl/packet.sv uses.)
# Exit 0 only if the testbench prints "SMOKE PASS".
set -euo pipefail
here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
top="$(cd "$here/.." && pwd)"
root="$(cd "$top/../../.." && pwd)"
out="${TMPDIR:-/tmp}/splitsys_sim_packet_mem"
mkdir -p "$out"
srcs=()
while read -r f; do
  f="${f%%#*}"; f="${f//[[:space:]]/}"
  [ -n "$f" ] && srcs+=("$root/$f")
done < "$top/sources.f"
verilator --binary --timing -j 0 -Wno-fatal -Wno-lint -Wno-style \
  --top-module tb_packet_mem -Mdir "$out/obj" -o sim \
  -I"$root/rtl" -I"$root/rtl/gen" \
  "${srcs[@]}" "$top/packet_mem.sv" "$here/tb_packet_mem.sv" > "$out/build.log" 2>&1 \
  || { tail -40 "$out/build.log"; exit 1; }
"$out/obj/sim" | tee "$out/sim.log"
grep -q "^SMOKE PASS" "$out/sim.log"
