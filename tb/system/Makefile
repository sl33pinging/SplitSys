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
#        SWITCH_CHAINS (4), WORKERS (16), SUITES (full,strip,directed,modeswitch,selftest),
#        RTL_REV (HEAD | <rev> | worktree), RAND_RESET=1 (random X-initial state),
#        ONLY=<substring> (only jobs whose name contains it, e.g. ONLY=gemm_full_s1_17),
#        WAVES=1 (FST build; one dump per board in $(WORK)/waves/<board>.fst; use with ONLY=)
#        SIGNOFF=1 refuse (exit 2) if RTL_REV=worktree or any expected-value input is dirty:
#                  tb/system tb/cosim tb/common tb/oracle tb/Makefile sw/crates/{golden,vec,cycle,spec} spec
# Isolation: every run gets its own directory $(SPLITSYS_CACHE)/system/sys-<RUN_ID>/ (vectors,
# predictions, a private copy of the harness binary, log, summary.json with rtl/tb provenance);
# the harness build dir is cosim-<rtl>-sys-... and is built under flock.

SHELL := /bin/bash
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
SIGNOFF       ?= 0

RUN_ID := $(shell date +%Y%m%dT%H%M%S)-s$(SEED)-$(shell od -An -N2 -tu2 /dev/urandom | tr -d ' ')
WORK   := $(SPLITSYS_CACHE)/system/sys-$(RUN_ID)
VEC    := $(CARGO_TARGET_DIR)/release/splitsys-vec
CYC    := $(CARGO_TARGET_DIR)/release/splitsys-cycle
COSIM_ARGS := RTL_REV=$(RTL_REV) WAVES=$(WAVES) TAG_SUFFIX=-sys
HARNESS = $(shell $(MAKE) --no-print-directory -s -C $(TB_ROOT)/cosim print-bin $(COSIM_ARGS))
export SPLITSYS_ORACLE := $(SPLITSYS_CACHE)/tb-oracle-target/release/splitsys-tb-oracle

# provenance
INPUTS   := tb/system tb/cosim tb/common tb/oracle tb/Makefile sw/crates/golden sw/crates/vec sw/crates/cycle sw/crates/spec spec
RTL_SHA  := $(if $(filter worktree,$(RTL_REV)),worktree,$(shell git -C $(REPO_ROOT) rev-parse --short $(RTL_REV)))
RTL_TREE := $(if $(filter worktree,$(RTL_REV)),worktree,$(shell git -C $(REPO_ROOT) log -1 --format=%h $(RTL_REV) -- rtl))
TB_HASH  := $(shell git -C $(REPO_ROOT) log -1 --format=%h -- tb/system tb/cosim tb/common tb/oracle)
DIRTY    := $(shell git -C $(REPO_ROOT) status --porcelain -- $(INPUTS) | awk '{print $$2}' | tr '\n' ' ')
META     := {"rtl_rev":"$(RTL_SHA)","rtl_last_commit":"$(RTL_TREE)","tb_hash":"$(TB_HASH)","dirty_inputs":"$(strip $(DIRTY))","signoff":$(SIGNOFF),"run_id":"$(RUN_ID)"}

.PHONY: all tools harness guard
all: guard tools harness
	@mkdir -p $(WORK)
	@cp $(HARNESS) $(WORK)/Vtop
	@echo "== [system] run $(RUN_ID): rtl $(RTL_SHA) (last rtl commit $(RTL_TREE)), tb $(TB_HASH), dirty inputs: [$(strip $(DIRTY))]"
	@start=$$(date +%s); \
	python3 $(TB_ROOT)/system/system_test.py --harness $(WORK)/Vtop --vec $(VEC) --cycle $(CYC) \
	  --work $(WORK) --seed $(SEED) --n-full $(N_FULL) --n-strip $(N_STRIP) --n-switch $(N_SWITCH) \
	  --switch-chains $(SWITCH_CHAINS) --workers $(WORKERS) --suites $(SUITES) \
	  $(if $(filter 1,$(RAND_RESET)),--rand-reset) $(if $(ONLY),--only $(ONLY)) \
	  $(if $(filter 1,$(WAVES)),--trace-dir $(WORK)/waves) --meta '$(META)' \
	  --summary $(WORK)/summary.json 2>&1 | tee $(WORK)/system.log; rc=$${PIPESTATUS[0]}; \
	echo "== [system] rc=$$rc wall=$$(( $$(date +%s) - start ))s rtl=$(RTL_SHA) tb=$(TB_HASH) SEED=$(SEED) dirty=[$(strip $(DIRTY))] summary=$(WORK)/summary.json"; \
	exit $$rc

guard:
ifeq ($(SIGNOFF),1)
	@if [ "$(RTL_REV)" = worktree ]; then echo "== SIGNOFF refused: RTL_REV=worktree"; exit 2; fi
	@if [ -n "$(strip $(DIRTY))" ]; then echo "== SIGNOFF refused: dirty inputs: $(DIRTY)"; exit 2; fi
endif

tools:
	@cd $(REPO_ROOT)/sw && cargo build -q --release -p splitsys-vec -p splitsys-cycle
	@$(MAKE) --no-print-directory -s -C $(TB_ROOT) oracle

harness:
	@$(MAKE) --no-print-directory -C $(TB_ROOT)/cosim build $(COSIM_ARGS)
