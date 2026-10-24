MODULE_NAME         := packet
COCOTB_TOPLEVEL     := packet
COCOTB_TEST_MODULES := test_packet
DUT_SOURCES          = $(RTL_DIR)/packet.sv
# Sim-only overrides: short inter-byte timeout (SPEC §3.8 allows testbenches to
# lower RX_TIMEOUT through the module parameter) and a known BUILD_ID for PING.
# Keep these in sync with PKT_RX_TIMEOUT / PKT_BUILD_ID (exported to the test).
PKT_RX_TIMEOUT := 2000
PKT_BUILD_ID   := 1592594996
export PKT_RX_TIMEOUT PKT_BUILD_ID
EXTRA_ARGS += -GRX_TIMEOUT_CYC=$(PKT_RX_TIMEOUT) -GBUILD_ID=$(PKT_BUILD_ID)
include ../common/sim.mk
