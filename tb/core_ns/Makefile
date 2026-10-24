# core.sv built with STRIP_EN = 0 (the cost-of-splitting build): every FULL directed job
# and random FULL jobs must be bit-exact with counters == splitsys-cycle. Reuses
# tb/core/test_core.py (CORE_STRIP_EN=0 restricts it to FULL jobs).
MODULE_NAME         := core_ns
COCOTB_TOPLEVEL     := core_tb
COCOTB_TEST_MODULES := test_core
DUT_SOURCES          = $(RTL_DIR)/pe.sv $(RTL_DIR)/skew.sv $(RTL_DIR)/array.sv $(RTL_DIR)/buffers.sv \
                       $(RTL_DIR)/requant.sv $(RTL_DIR)/sequencer.sv $(RTL_DIR)/core.sv \
                       $(RTL_DIR)/counters.sv $(TB_DIR)/core/core_tb.sv
CORE_STRIP_EN := 0
export CORE_STRIP_EN
EXTRA_ARGS += -GSTRIP_EN=0
include ../common/sim.mk
export PYTHONPATH := $(TB_DIR)/core:$(PYTHONPATH)
