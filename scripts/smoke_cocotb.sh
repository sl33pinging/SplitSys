#!/usr/bin/env bash
# cocotb + Verilator smoke test (toolchain-engineer). Writes a throwaway counter
# DUT and test into a temp dir under /tmp (never the repo) and runs it.
#   scripts/smoke_cocotb.sh        -> exit 0 and "SMOKE_COCOTB_OK" on success
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "$HERE/env.sh"
D="$(mktemp -d /tmp/splitsys_cocotb_smoke.XXXXXX)"
trap 'rm -rf "$D"' EXIT

cat > "$D/counter.sv" <<'EOF'
module counter #(parameter int W = 8) (
    input  logic         clk,
    input  logic         rst,
    output logic [W-1:0] count
);
    always_ff @(posedge clk) begin
        if (rst) count <= '0;
        else     count <= count + 1'b1;
    end
endmodule
EOF

cat > "$D/test_counter.py" <<'EOF'
import cocotb
from cocotb.clock import Clock
from cocotb.triggers import ClockCycles, RisingEdge, ReadOnly


@cocotb.test()
async def counts_up(dut):
    cocotb.start_soon(Clock(dut.clk, 10, unit="ns").start())
    dut.rst.value = 1
    await ClockCycles(dut.clk, 2)
    dut.rst.value = 0
    await RisingEdge(dut.clk)
    await ReadOnly()
    start = int(dut.count.value)
    for i in range(1, 300):
        await RisingEdge(dut.clk)
        await ReadOnly()
        assert int(dut.count.value) == (start + i) % 256, f"cycle {i}"
EOF

cat > "$D/Makefile" <<EOF
SIM ?= verilator
TOPLEVEL_LANG = verilog
VERILOG_SOURCES = $D/counter.sv
COCOTB_TOPLEVEL = counter
COCOTB_TEST_MODULES = test_counter
SIM_BUILD = $D/sim_build
include \$(shell cocotb-config --makefiles)/Makefile.sim
EOF

( cd "$D" && make -s > "$D/run.log" 2>&1 ) || { tail -40 "$D/run.log"; echo "SMOKE_COCOTB_FAIL"; exit 1; }
grep -E "TESTS=|PASS|FAIL" "$D/run.log" | tail -5
if grep -q "FAIL=0" "$D/run.log" && grep -q "PASS=1" "$D/run.log"; then
    echo "SMOKE_COCOTB_OK (cocotb $(python -c 'import cocotb;print(cocotb.__version__)'), $(verilator --version | cut -d' ' -f1-2))"
else
    tail -40 "$D/run.log"; echo "SMOKE_COCOTB_FAIL"; exit 1
fi
