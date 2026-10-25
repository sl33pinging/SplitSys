# tb/system -- system testbench: simulated `top` driven through its real UART pins
# (owner: verification-engineer). Run inside WSL with scripts/env.sh sourced:
#     scripts/wsl 'make -C tb/system'                      # all suites, SEED=1
#     scripts/wsl 'make -C tb/system SIGNOFF=1'            # same, refuses unless the inputs are clean
#     scripts/wsl 'make -C tb/system SUITES=full N_FULL=20'
#
# Every worker is one tb/cosim harness process (Verilator `top` + TCP<->UART pins);
# system_test.py talks SPEC §3.8 frames to it. Expected values: frames from tb/oracle
# (splitsys_golden::packet), outputs from splitsys-vec JSON, counters from
# splitsys-cycle predict, self-test from sw/crates/vec/out/selftest_expected.json.
#
# Knobs: SEED (1), N_FULL (200), N_STRIP (100), N_SWITCH (6 F/S pairs per chain),
#        SWITCH_CHAINS (4), WORKERS (16), SUITES (full,strip,directed,modeswitch,selftest,display),
#        RTL_REV (HEAD | <rev> | worktree), RAND_RESET=1 (random X-initial state),
#        ONLY=<substring> (only jobs whose name contains it, e.g. ONLY=gemm_full_s1_17),
#        WAVES=1 (FST build; one dump per board in $(WORK)/waves/<board>.fst; use with ONLY=)
#        UART_DIV (1): sim-only top.UART_DIV_OVERRIDE of the harness (16 clk/bit, for speed);
#                  UART_DIV=0 = no override, the real package divisor (BAUD_DIV 4: 64 clk/bit).
#                  At UART_DIV=0 selftest_busy cannot meet its own precondition (its 8 probe
#                  frames need ~151k cycles of UART time, more than the 78,594-cycle max-shape
#                  job) and reports "test window too long"; it is covered at UART_DIV=1.
#        SIGNOFF=1 refuse (exit 2) unless tb/common/provenance.mk says "sign-off eligible: YES"
#                  (tb/, sw/crates/{golden,vec,cycle,spec}, spec/ clean and RTL_REV != worktree)
# Run conventions: tb/common/provenance.mk (see tb/README.md). Namespace SPLITSYS_CACHE_TAG
# (default ve2 for this lane): oracle in $(TB_NS)/oracle-target, splitsys-vec/-cycle in
# $(TB_NS)/sw-target (both built from this checkout via `make -C tb tools`), harness build in
# $(TB_NS)/sim_build (flock'd), and one directory per run: $(TB_NS)/logs/<RUN_ID>/system/
# (vectors, predictions, private harness copy, system.log, summary.json with the provenance line).

SHELL := /bin/bash
TB_ROOT   := $(abspath $(dir $(lastword $(MAKEFILE_LIST)))/..)
REPO_ROOT := $(abspath $(TB_ROOT)/..)
SPLITSYS_CACHE     ?= $(HOME)/.cache/splitsys
SPLITSYS_CACHE_TAG ?= ve2
export SPLITSYS_CACHE_TAG

SEED          ?= 1
N_FULL        ?= 200
N_STRIP       ?= 100
N_SWITCH      ?= 6
SWITCH_CHAINS ?= 4
WORKERS       ?= 16
SUITES        ?= full,strip,directed,modeswitch,selftest,display
RTL_REV       ?= HEAD
RAND_RESET    ?= 0
WAVES         ?= 0
ONLY          ?=
SIGNOFF       ?= 0
UART_DIV      ?= 1

RTL_SHA  := $(if $(filter worktree,$(RTL_REV)),worktree,$(shell git -C $(REPO_ROOT) rev-parse --short $(RTL_REV)))
.DEFAULT_GOAL := all
include $(TB_ROOT)/common/provenance.mk

RTL_TREE := $(if $(filter worktree,$(RTL_REV)),worktree,$(shell git -C $(REPO_ROOT) log -1 --format=%h $(RTL_REV) -- rtl))
WORK   := $(RUN_LOG_DIR)/system
VEC    := $(TB_NS)/sw-target/release/splitsys-vec
CYC    := $(TB_NS)/sw-target/release/splitsys-cycle
export SPLITSYS_ORACLE := $(TB_NS)/oracle-target/release/splitsys-tb-oracle
COSIM_ARGS := RTL_REV=$(RTL_REV) WAVES=$(WAVES) TAG_SUFFIX=-sys UART_DIV=$(UART_DIV)
HARNESS = $(shell $(MAKE) --no-print-directory -s -C $(TB_ROOT)/cosim print-bin $(COSIM_ARGS))
META := {"provenance":"$(PROV_LINE)","signoff_eligible":"$(PROV_ELIGIBLE)","rtl_rev":"$(RTL_SHA)","rtl_last_commit":"$(RTL_TREE)","signoff":$(SIGNOFF),"run_id":"$(RUN_ID)"}

.PHONY: all tools harness guard
all: guard provenance tools harness
	@mkdir -p $(WORK)
	@cp $(HARNESS) $(WORK)/Vtop
	@echo "provenance: $(PROV_LINE)" > $(WORK)/system.log
	@start=$$(date +%s); \
	python3 $(TB_ROOT)/system/system_test.py --harness $(WORK)/Vtop --vec $(VEC) --cycle $(CYC) \
	  --work $(WORK) --seed $(SEED) --n-full $(N_FULL) --n-strip $(N_STRIP) --n-switch $(N_SWITCH) \
	  --switch-chains $(SWITCH_CHAINS) --workers $(WORKERS) --suites $(SUITES) \
	  $(if $(filter 1,$(RAND_RESET)),--rand-reset) $(if $(ONLY),--only $(ONLY)) \
	  $(if $(filter 1,$(WAVES)),--trace-dir $(WORK)/waves) --meta '$(META)' \
	  --summary $(WORK)/summary.json 2>&1 | tee -a $(WORK)/system.log; rc=$${PIPESTATUS[0]}; \
	echo "== [system] rc=$$rc wall=$$(( $$(date +%s) - start ))s SEED=$(SEED) last-rtl-commit=$(RTL_TREE) eligible=$(PROV_ELIGIBLE)"; \
	echo "== [system] $(PROV_LINE)"; \
	echo "== [system] log=$(WORK)/system.log summary=$(WORK)/summary.json"; \
	exit $$rc

guard:
ifeq ($(SIGNOFF),1)
	@if [ "$(PROV_ELIGIBLE)" != YES ]; then echo "== SIGNOFF refused: $(PROV_ELIGIBLE) -- $(PROV_LINE)"; exit 2; fi
endif

tools:
	@$(MAKE) --no-print-directory -s -C $(TB_ROOT) tools >/dev/null

harness:
	@$(MAKE) --no-print-directory -C $(TB_ROOT)/cosim build $(COSIM_ARGS)
