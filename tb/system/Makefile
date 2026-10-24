# tb/system -- system testbench: simulated `top` driven through its real UART pins
# (owner: verification-engineer). Run inside WSL with scripts/env.sh sourced:
#     scripts/wsl 'make -C tb/system'                      # all suites, SEED=1
#     scripts/wsl 'make -C tb/system SUITES=full N_FULL=20'
#
# Every worker is one tb/cosim harness process (Verilator `top` + TCP<->UART pins);
# system_test.py talks SPEC §3.8 frames to it. Expected values: frames from tb/oracle
# (splitsys_golden::packet), outputs from splitsys-vec JSON, counters from
# splitsys-cycle predict, self-test from sw/crates/vec/out/selftest_expected.json.
#
# Knobs: SEED (1), N_FULL (200), N_STRIP (100), N_SWITCH (6 F/S pairs per chain),
#        SWITCH_CHAINS (4), WORKERS (16), SUITES (full,strip,directed,modeswitch,selftest),
#        RTL_REV (HEAD | <rev> | worktree), RAND_RESET=1 (random X-initial state),
#        ONLY=<substring> (only jobs whose name contains it, e.g. ONLY=gemm_full_s1_17),
#        WAVES=1 (FST build; one dump per board in $(WORK)/waves/<board>.fst; use with ONLY=)
# Summary JSON: $(SPLITSYS_CACHE)/system/seed$(SEED)/summary.json

TB_ROOT   := $(abspath $(dir $(lastword $(MAKEFILE_LIST)))/..)
REPO_ROOT := $(abspath $(TB_ROOT)/..)
SPLITSYS_CACHE ?= $(HOME)/.cache/splitsys
CARGO_TARGET_DIR ?= $(SPLITSYS_CACHE)/target

SEED          ?= 1
N_FULL        ?= 200
N_STRIP       ?= 100
N_SWITCH      ?= 6
SWITCH_CHAINS ?= 4
WORKERS       ?= 16
SUITES        ?= full,strip,directed,modeswitch,selftest
RTL_REV       ?= HEAD
RAND_RESET    ?= 0
WAVES         ?= 0
ONLY          ?=

WORK := $(SPLITSYS_CACHE)/system/seed$(SEED)
VEC  := $(CARGO_TARGET_DIR)/release/splitsys-vec
CYC  := $(CARGO_TARGET_DIR)/release/splitsys-cycle
HARNESS = $(shell $(MAKE) --no-print-directory -s -C $(TB_ROOT)/cosim print-bin RTL_REV=$(RTL_REV) WAVES=$(WAVES))
export SPLITSYS_ORACLE := $(SPLITSYS_CACHE)/tb-oracle-target/release/splitsys-tb-oracle

.PHONY: all tools harness
all: tools harness
	@mkdir -p $(WORK)
	@start=$$(date +%s); \
	python3 $(TB_ROOT)/system/system_test.py --harness $(HARNESS) --vec $(VEC) --cycle $(CYC) \
	  --work $(WORK) --seed $(SEED) --n-full $(N_FULL) --n-strip $(N_STRIP) --n-switch $(N_SWITCH) \
	  --switch-chains $(SWITCH_CHAINS) --workers $(WORKERS) --suites $(SUITES) \
	  $(if $(filter 1,$(RAND_RESET)),--rand-reset) $(if $(ONLY),--only $(ONLY)) \
	  $(if $(filter 1,$(WAVES)),--trace-dir $(WORK)/waves) --summary $(WORK)/summary.json; rc=$$?; \
	echo "== [system] rc=$$rc wall=$$(( $$(date +%s) - start ))s RTL_REV=$(RTL_REV) SEED=$(SEED) summary=$(WORK)/summary.json"; \
	exit $$rc

tools:
	@cd $(REPO_ROOT)/sw && cargo build -q --release -p splitsys-vec -p splitsys-cycle
	@$(MAKE) --no-print-directory -s -C $(TB_ROOT) oracle

harness:
	@$(MAKE) --no-print-directory -C $(TB_ROOT)/cosim build RTL_REV=$(RTL_REV) WAVES=$(WAVES)
