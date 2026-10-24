# Shared cocotb + Verilator settings. Each tb/<module>/Makefile sets
#   MODULE_NAME, COCOTB_TOPLEVEL, COCOTB_TEST_MODULES, DUT_SOURCES
# (and optionally EXTRA_ARGS / SIM_BUILD_TAG) and then includes this file.
#
# Knobs (environment or make variables):
#   SEED=<n>          seed for every randomized test (printed by each test). Default 1.
#   SCALE=<f>         random-suite size multiplier (quick: 0.1, units: 1, all: 4).
#   WAVES=1           build with FST tracing; dump.fst lands in $(SIM_BUILD).
#                     Use with COCOTB_TEST_FILTER=<failing test> for a single-test wave.
#   SPLITSYS_SIM_BUILD  root for Verilator build dirs (WSL ext4; set by scripts/env.sh).

TB_DIR    := $(abspath $(dir $(lastword $(MAKEFILE_LIST)))/..)
REPO_ROOT := $(abspath $(TB_DIR)/..)
RTL_DIR   ?= $(REPO_ROOT)/rtl

SIM           ?= verilator
TOPLEVEL_LANG ?= verilog
SEED          ?= 1
SCALE         ?= 1
WAVES         ?= 0
export SEED SCALE

SPLITSYS_SIM_BUILD ?= $(HOME)/.cache/splitsys/sim_build
SIM_BUILD_TAG ?=
ifeq ($(WAVES),1)
  SIM_BUILD := $(SPLITSYS_SIM_BUILD)/$(MODULE_NAME)$(SIM_BUILD_TAG)-waves
  EXTRA_ARGS += --trace-fst --trace-structs
  SIM_ARGS += --trace
else
  SIM_BUILD := $(SPLITSYS_SIM_BUILD)/$(MODULE_NAME)$(SIM_BUILD_TAG)
endif
COCOTB_RESULTS_FILE := $(SIM_BUILD)/results.xml

# The generated package always comes first.
VERILOG_SOURCES := $(RTL_DIR)/gen/splitsys_pkg.sv $(DUT_SOURCES)

# -Wall lint is the RTL's job; the testbench build must not fail on style
# warnings, but it must fail on real errors (Verilator default).
EXTRA_ARGS += -Wno-fatal -Wno-WIDTHEXPAND -Wno-WIDTHTRUNC -I$(RTL_DIR) -I$(RTL_DIR)/gen
BUILD_ARGS += -j 8

# Python: tb/common on the path for splitsys_tb helpers.
export PYTHONPATH := $(TB_DIR)/common:$(PYTHONPATH)
export SPLITSYS_REPO_ROOT := $(REPO_ROOT)
export SPLITSYS_RTL_DIR := $(RTL_DIR)

include $(shell cocotb-config --makefiles)/Makefile.sim
