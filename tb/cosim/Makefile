# tb/cosim -- simulated `top` behind a TCP socket (owner: verification-engineer).
# Run inside WSL with scripts/env.sh sourced, e.g. from Windows:
#     scripts/wsl 'make -C tb/cosim'                 # build + start, prints the port
#     splitsys-host --transport tcp:127.0.0.1:5555 ping      (from another shell)
#
#   make -C tb/cosim             build, then run the bridge in the foreground (PORT, default 5555)
#   make -C tb/cosim build       build only; prints the harness path
#   make -C tb/cosim PORT=0      ephemeral port (printed as "COSIM_PORT <n>")
#   make -C tb/cosim check       S3 acceptance in one shot: harness on an ephemeral port + real
#                                splitsys-host ping/selftest/run (both modes, int32)/bench smoke;
#                                records -> RESULTS/sim (default <repo>/results/sim, source "sim")
# Knobs:
#   RTL_REV=<rev>|worktree   RTL under test (default HEAD: committed rtl/, exported by tb/Makefile rtl_export)
#   WAVES=1                  FST tracing build; run with ARGS='--trace /path/x.fst'
#   UART_DIV=1               sim-only baud divisor -> top.UART_DIV_OVERRIDE (bit = UART_DIV*16 clocks).
#                            Hardware builds always use 0 (package BAUD_DIV = 54 at 115200).
#   BTN_ACTIVE_LOW=0         must match the hardware build's board_io polarity choice.
#
# Sim-only top overrides (documented; hardware uses the package defaults):
#   UART_DIV_OVERRIDE=$(UART_DIV)  -> 160 clocks per byte instead of 8,680
#   DEB_CYC=16, HB_HALF_CYC=500, ACT_CYC=64, SCAN_CYC=64 (board_io timers, so BTN0/BTN1
#   presses and the display scan take tens of clocks, not 10^6)
#   RX_TIMEOUT_CYC = package default (1,000,000): the harness freezes time while idle,
#   so a frame split across TCP segments never times out.
#   BUILD_ID = 0x0<7-hex git hash of the RTL under test> (PING / run records show it).

TB_ROOT   := $(abspath $(dir $(lastword $(MAKEFILE_LIST)))/..)
REPO_ROOT := $(abspath $(TB_ROOT)/..)
SPLITSYS_CACHE     ?= $(HOME)/.cache/splitsys
SPLITSYS_SIM_BUILD ?= $(SPLITSYS_CACHE)/sim_build
CARGO_TARGET_DIR ?= $(SPLITSYS_CACHE)/target

RTL_REV ?= HEAD
ifeq ($(RTL_REV),worktree)
  RTL_SHA := worktree
  RTL_DIR := $(REPO_ROOT)/rtl
  BUILD_HASH := $(shell git -C $(REPO_ROOT) rev-parse --short=7 HEAD)
else
  RTL_SHA := $(shell git -C $(REPO_ROOT) rev-parse --short $(RTL_REV))
  RTL_DIR := $(SPLITSYS_CACHE)/rtl_rev/$(RTL_SHA)/rtl
  BUILD_HASH := $(shell git -C $(REPO_ROOT) rev-parse --short=7 $(RTL_REV))
endif

UART_DIV       ?= 1
BTN_ACTIVE_LOW ?= 0
WAVES          ?= 0
PORT           ?= 5555
CTRL_PORT      ?= 0
ARGS           ?=
COPT           ?= -O3 -march=native
VEXTRA         ?=
BUILD_ID_HEX   ?= 0$(BUILD_HASH)
ROM_FILE       ?= $(REPO_ROOT)/sw/crates/vec/out/selftest_rom.mem

TAG := cosim-$(RTL_SHA)$(TAG_SUFFIX)-d$(UART_DIV)-b$(BTN_ACTIVE_LOW)$(if $(filter 1,$(WAVES)),-waves)
OBJ := $(SPLITSYS_SIM_BUILD)/$(TAG)
BIN := $(OBJ)/Vtop

VFLAGS := --cc --exe --build -j 16 --top-module top -O3 --x-assign fast \
          -Wno-fatal -Wno-WIDTHEXPAND -Wno-WIDTHTRUNC -Wno-lint -Wno-style \
          -I$(RTL_DIR) -I$(RTL_DIR)/gen \
          -GUART_DIV_OVERRIDE=$(UART_DIV) -GBTN_ACTIVE_LOW=$(BTN_ACTIVE_LOW) \
          -GDEB_CYC=16 -GHB_HALF_CYC=500 -GACT_CYC=64 -GSCAN_CYC=64 \
          -GBUILD_ID=32\'h$(BUILD_ID_HEX) -GROM_FILE=\"$(ROM_FILE)\" \
          $(VEXTRA) -CFLAGS "$(COPT) -DUART_DIV=$(UART_DIV) -DBTN_ACTIVE_LOW_V=$(BTN_ACTIVE_LOW)"
ifeq ($(WAVES),1)
  VFLAGS += --trace-fst -CFLAGS -DTRACE
endif

.PHONY: run build rtl_export print-bin check
# S3 acceptance: real splitsys-host vs this harness (ping, selftest, run both modes, bench smoke).
# Records go to RESULTS/sim (default <repo>/results/sim, source "sim").
RESULTS ?= $(REPO_ROOT)/results
SEED    ?= 1
check: build
	@cd $(REPO_ROOT)/sw && cargo build -q --release -p splitsys-host
	bash $(TB_ROOT)/cosim/host_check.sh $(BIN) $(CARGO_TARGET_DIR)/release/splitsys-host $(RESULTS) $(SEED)

run: build
	@echo "== cosim: RTL=$(RTL_SHA) BUILD_ID=0x$(BUILD_ID_HEX) bit=$$(( $(UART_DIV) * 16 )) clocks; Ctrl-C to stop"
	$(BIN) --port $(PORT) --ctrl-port $(CTRL_PORT) $(ARGS)

rtl_export:
ifneq ($(RTL_REV),worktree)
	@$(MAKE) --no-print-directory -C $(TB_ROOT) rtl_export RTL_REV=$(RTL_REV) >/dev/null
endif

build: rtl_export
	@mkdir -p $(OBJ)
	@echo "== cosim build: $(TAG) -> $(BIN)"
	@cd $(OBJ) && verilator $(VFLAGS) -Mdir $(OBJ) \
	  $(RTL_DIR)/gen/splitsys_pkg.sv $(RTL_DIR)/*.sv \
	  $(TB_ROOT)/cosim/harness.cpp > $(OBJ)/build.log 2>&1 || (tail -40 $(OBJ)/build.log; exit 1)
	@echo "== cosim build ok: $(BIN)"

print-bin:
	@echo $(BIN)
