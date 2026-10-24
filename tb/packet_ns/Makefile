# packet.sv built with STRIP_EN = 0 (the ENABLE_STRIP = 0 cost build): RUN_GEMM with
# mode STRIP must be NACK_RANGE (SPEC §3.5 rule 1). Reuses tb/packet/test_packet.py.
MODULE_NAME         := packet_ns
COCOTB_TOPLEVEL     := packet
COCOTB_TEST_MODULES := test_packet
COCOTB_TEST_FILTER  ?= test_run_gemm_validation
DUT_SOURCES          = $(RTL_DIR)/packet.sv
PKT_RX_TIMEOUT := 2000
PKT_BUILD_ID   := 1592594996
PKT_STRIP_EN   := 0
export PKT_RX_TIMEOUT PKT_BUILD_ID PKT_STRIP_EN COCOTB_TEST_FILTER
EXTRA_ARGS += -GRX_TIMEOUT_CYC=$(PKT_RX_TIMEOUT) -GBUILD_ID=$(PKT_BUILD_ID) -GSTRIP_EN=0
include ../common/sim.mk
export PYTHONPATH := $(TB_DIR)/packet:$(PYTHONPATH)
