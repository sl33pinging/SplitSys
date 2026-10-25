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
#                            Hardware builds always use 0 (package BAUD_DIV = 4 at 1,562,500 baud, D-020;
#                            KG2 fallback: 54 at 115200). UART_DIV=0 builds exactly that: no override,
#                            the RTL uses the package BAUD_DIV (64 clk/bit, 640 clk/byte) and the harness
#                            serializer takes its bit time from the same package (HARNESS_DIV below).
#   BTN_ACTIVE_LOW=0         must match the hardware build's board_io polarity choice.
#
# Sim-only top overrides (documented; hardware uses the package defaults):
#   UART_DIV_OVERRIDE=$(UART_DIV)  -> 160 clocks per byte instead of 640 (real divisor 4)
#   DEB_CYC=16, HB_HALF_CYC=500, ACT_CYC=64, SCAN_CYC=64 (board_io timers, so BTN0/BTN1
#   presses and the display scan take tens of clocks, not 10^6)
#   RX_TIMEOUT_CYC = package default (1,000,000): the harness freezes time while idle,
#   so a frame split across TCP segments never times out.
#   BUILD_ID = 0x0<7-hex git hash of the RTL under test> (PING / run records show it).

SHELL := /bin/bash
TB_ROOT   := $(abspath $(dir $(lastword $(MAKEFILE_LIST)))/..)
REPO_ROOT := $(abspath $(TB_ROOT)/..)
SPLITSYS_CACHE     ?= $(HOME)/.cache/splitsys
# Per-agent namespace (tb/common/provenance.mk): this lane defaults to ve2; red-team sets its own.
SPLITSYS_CACHE_TAG ?= ve2
export SPLITSYS_CACHE_TAG

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

.DEFAULT_GOAL := run
include $(TB_ROOT)/common/provenance.mk
SPLITSYS_SIM_BUILD := $(TB_SIM_BUILD)
SW_TARGET          := $(TB_NS)/sw-target

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

# Bit divisor of the harness serializer/decoder: UART_DIV, or the package BAUD_DIV of the RTL under
# test when UART_DIV=0 (read from the same commit, so harness and RTL can never disagree).
PKG_BAUD_DIV := $(shell $(if $(filter worktree,$(RTL_REV)),cat $(REPO_ROOT)/rtl/gen/splitsys_pkg.sv,git -C $(REPO_ROOT) show $(RTL_SHA):rtl/gen/splitsys_pkg.sv) 2>/dev/null | grep -o 'unsigned BAUD_DIV *= *[0-9][0-9]*' | grep -o '[0-9][0-9]*$$')
HARNESS_DIV  := $(if $(filter 0,$(UART_DIV)),$(PKG_BAUD_DIV),$(UART_DIV))
ifeq ($(strip $(HARNESS_DIV)),)
  $(error tb/cosim: UART_DIV=0 but BAUD_DIV not found in rtl/gen/splitsys_pkg.sv at $(RTL_SHA))
endif

TAG := cosim-$(RTL_SHA)$(TAG_SUFFIX)-d$(UART_DIV)-b$(BTN_ACTIVE_LOW)$(if $(filter 1,$(WAVES)),-waves)
OBJ := $(SPLITSYS_SIM_BUILD)/$(TAG)
BIN := $(OBJ)/Vtop

VFLAGS := --cc --exe --build -j 16 --top-module top -O3 --x-assign fast \
          -Wno-fatal -Wno-WIDTHEXPAND -Wno-WIDTHTRUNC -Wno-lint -Wno-style \
          -I$(RTL_DIR) -I$(RTL_DIR)/gen \
          -GUART_DIV_OVERRIDE=$(UART_DIV) -GBTN_ACTIVE_LOW=$(BTN_ACTIVE_LOW) \
          -GDEB_CYC=16 -GHB_HALF_CYC=500 -GACT_CYC=64 -GSCAN_CYC=64 \
          -GBUILD_ID=32\'h$(BUILD_ID_HEX) -GROM_FILE=\"$(ROM_FILE)\" \
          $(VEXTRA) -CFLAGS "$(COPT) -DUART_DIV=$(HARNESS_DIV) -DBTN_ACTIVE_LOW_V=$(BTN_ACTIVE_LOW)"
ifeq ($(WAVES),1)
  VFLAGS += --trace-fst -CFLAGS -DTRACE
endif

.PHONY: run build rtl_export print-bin check host_src
# S3 acceptance: real splitsys-host vs this harness (ping, selftest, run both modes, bench smoke).
# Records go to RESULTS/sim (default <repo>/results/sim, source "sim").
#   HOST_REV=HEAD (default) | <rev>: splitsys-host is built from a COMMITTED tree (a local
#            `git clone --shared` at that commit in $(SPLITSYS_CACHE)/host_rev/<sha>), so other
#            agents' uncommitted sw/ or demo/ edits never leak into a sign-off; its run records
#            carry git_hash <sha> (no -dirty).
#   HOST_REV=worktree: build from this checkout (host developers' pre-commit check).
RESULTS  ?= $(REPO_ROOT)/results
SEED     ?= 1
HOST_REV ?= HEAD
ifeq ($(HOST_REV),worktree)
  HOST_SRC    := $(REPO_ROOT)
  HOST_TARGET := $(SW_TARGET)
else
  HOST_SHA    := $(shell git -C $(REPO_ROOT) rev-parse --short $(HOST_REV))
  HOST_SRC    := $(SPLITSYS_CACHE)/host_rev/$(HOST_SHA)
  HOST_TARGET := $(TB_NS)/host-target
endif
HOST_BIN := $(HOST_TARGET)/release/splitsys-host

host_src:
ifneq ($(HOST_REV),worktree)
	@mkdir -p $(SPLITSYS_CACHE)/host_rev; \
	flock $(HOST_SRC).lock bash -c '[ -f $(HOST_SRC)/.ok ] || { rm -rf $(HOST_SRC) && \
	  git clone -q --shared --no-checkout $(REPO_ROOT) $(HOST_SRC) && \
	  git -C $(HOST_SRC) checkout -q --detach $(HOST_SHA) && touch $(HOST_SRC)/.ok; }'
	@echo "== cosim-check host: splitsys-host from committed $(HOST_SHA) ($(HOST_SRC))"
endif

check: provenance build host_src
	@cd $(HOST_SRC)/sw && CARGO_TARGET_DIR=$(HOST_TARGET) cargo build -q --release -p splitsys-host
	@mkdir -p $(RUN_LOG_DIR); log=$(RUN_LOG_DIR)/cosim-check.log; \
	echo "provenance: $(PROV_LINE) host=$(HOST_REV)$(if $(HOST_SHA),:$(HOST_SHA))" > $$log; \
	echo "sign-off eligible: $(PROV_ELIGIBLE)" >> $$log; \
	TMPDIR=$(RUN_LOG_DIR) bash $(TB_ROOT)/cosim/host_check.sh $(BIN) $(HOST_BIN) $(RESULTS) $(SEED) 2>&1 | tee -a $$log; \
	rc=$${PIPESTATUS[0]}; echo "== [cosim-check] rc=$$rc $(PROV_LINE) host=$(HOST_REV)$(if $(HOST_SHA),:$(HOST_SHA)) log=$$log"; exit $$rc

run: build
	@echo "== cosim: RTL=$(RTL_SHA) BUILD_ID=0x$(BUILD_ID_HEX) bit=$$(( $(HARNESS_DIV) * 16 )) clocks (UART_DIV_OVERRIDE=$(UART_DIV)); Ctrl-C to stop"
	$(BIN) --port $(PORT) --ctrl-port $(CTRL_PORT) $(ARGS)

rtl_export:
ifneq ($(RTL_REV),worktree)
	@$(MAKE) --no-print-directory -C $(TB_ROOT) rtl_export RTL_REV=$(RTL_REV) >/dev/null
endif

build: rtl_export
	@mkdir -p $(OBJ)
	@echo "== cosim build: $(TAG) -> $(BIN)"
	@cd $(OBJ) && flock $(OBJ).lock verilator $(VFLAGS) -Mdir $(OBJ) \
	  $(RTL_DIR)/gen/splitsys_pkg.sv $(RTL_DIR)/*.sv \
	  $(TB_ROOT)/cosim/harness.cpp > $(OBJ)/build.log 2>&1 || (tail -40 $(OBJ)/build.log; exit 1)
	@echo "== cosim build ok: $(BIN)"

print-bin:
	@echo $(BIN)
