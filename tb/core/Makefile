MODULE_NAME         := core
COCOTB_TOPLEVEL     := core_tb
COCOTB_TEST_MODULES := test_core
DUT_SOURCES          = $(RTL_DIR)/pe.sv $(RTL_DIR)/skew.sv $(RTL_DIR)/array.sv $(RTL_DIR)/buffers.sv \
                       $(RTL_DIR)/requant.sv $(RTL_DIR)/sequencer.sv $(RTL_DIR)/core.sv \
                       $(RTL_DIR)/counters.sv $(TB_DIR)/core/core_tb.sv
include ../common/sim.mk
