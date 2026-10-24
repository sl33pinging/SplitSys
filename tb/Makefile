# SplitSys testbenches (owner: verification-engineer). Run inside WSL with
# scripts/env.sh sourced, e.g. from Windows:  scripts/wsl 'make -C tb units'
#
#   make -C tb units        every unit + level-2 suite present (SEED=1)
#   make -C tb signoff      same, but refuses to start unless tb/ and the Rust reference
#                           crates are clean (committed) and RTL is a commit
#   make -C tb quick        smoke: every suite at SCALE=0.1
#   make -C tb all          every suite at SCALE=4 for SEEDS (default 1 2 3)
#   make -C tb <suite>      one suite, e.g. make -C tb pe SEED=7 ; make -C tb core
#   make -C tb system       delegates to tb/system/Makefile (verification instance 2)
#   make -C tb cosim        delegates to tb/cosim/Makefile  (verification instance 2)
#   make -C tb cosim-check  tb/cosim `check` target
#   make -C tb oracle | provenance | clean
# Knobs: SEED, SCALE, WAVES=1 (FST, see common/sim.mk), COCOTB_TEST_FILTER=<test>,
#        RTL_REV (HEAD | <rev> | worktree), SPLITSYS_CACHE_TAG (per-agent namespace, see
#        common/provenance.mk). Parallel: make -C tb -j8 units

TB      := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))
TB_ROOT := $(TB)
REPO    := $(abspath $(TB)/..)

# RTL under test. Default: the COMMITTED rtl/ at HEAD, exported to the cache, so a
# sign-off names an exact git hash and never picks up someone's uncommitted edits.
#   RTL_REV=<rev>        test rtl/ as of that commit
#   RTL_REV=worktree     test the working tree (designers' pre-commit check; never a sign-off)
RTL_REV ?= HEAD
ifeq ($(RTL_REV),worktree)
  RTL_SHA   := worktree
  RTL_DIR   := $(REPO)/rtl
  RTL_FILES := $(notdir $(wildcard $(RTL_DIR)/*.sv))
else
  RTL_SHA   := $(shell git -C $(REPO) rev-parse --short $(RTL_REV))
  RTL_DIR    = $(SPLITSYS_CACHE)/rtl_rev/$(RTL_SHA)/rtl
  RTL_FILES := $(notdir $(shell git -C $(REPO) ls-tree --name-only $(RTL_SHA) rtl/))
endif

include $(TB)/common/provenance.mk
export RTL_DIR

ORACLE_TARGET := $(TB_NS)/oracle-target
ORACLE_BIN    := $(ORACLE_TARGET)/release/splitsys-tb-oracle
export SPLITSYS_ORACLE := $(ORACLE_BIN)
SW_TARGET := $(TB_NS)/sw-target
export SPLITSYS_CYCLE_BIN := $(SW_TARGET)/release/splitsys-cycle
VEC_BIN   := $(SW_TARGET)/release/splitsys-vec

SEED   ?= 1
SCALE  ?= 1
SEEDS  ?= 1 2 3

# Vectors from sw/crates/vec, per SEED, keyed by the sw/ tree hash (meta records git_dirty).
SW_TREE := $(shell git -C $(REPO) rev-parse --short HEAD:sw)
VEC_DIR := $(SPLITSYS_CACHE)/vec/sw-$(SW_TREE)$(if $(PROV_DIRTY),-dirty-$(SPLITSYS_CACHE_TAG),)/seed$(SEED)
export VEC_DIR

# Every suite we know about; only those whose RTL exists at the tested commit are run.
ALL_UNITS := pe skew uart counters buffers requant packet packet_ns array core core_ns
rtl_of_pe        := pe.sv
rtl_of_skew      := skew.sv
rtl_of_uart      := uart_rx.sv uart_tx.sv
rtl_of_counters  := counters.sv
rtl_of_buffers   := buffers.sv
rtl_of_requant   := requant.sv
rtl_of_packet    := packet.sv
rtl_of_packet_ns := packet.sv
rtl_of_array     := array.sv pe.sv skew.sv
rtl_of_core      := core.sv sequencer.sv buffers.sv array.sv pe.sv skew.sv requant.sv counters.sv
rtl_of_core_ns   := core.sv sequencer.sv buffers.sv array.sv pe.sv skew.sv requant.sv counters.sv
present = $(and $(wildcard $(TB)/$(1)/Makefile),$(if $(strip $(foreach f,$(rtl_of_$(1)),$(if $(filter $(f),$(RTL_FILES)),,missing))),,yes))
UNITS := $(foreach u,$(ALL_UNITS),$(if $(call present,$(u)),$(u)))

.PHONY: units signoff quick all oracle clean rtl_export vec tools system cosim cosim-check $(ALL_UNITS)

units: $(UNITS)
	@echo "== suites passed: $(UNITS) (SEED=$(SEED) SCALE=$(SCALE))"
	@echo "== skipped (RTL or tb missing): $(filter-out $(UNITS),$(ALL_UNITS))"
	@echo "== provenance: $(PROV_LINE)" | tee -a $(RUN_LOG_DIR)/provenance.txt
	@echo "== sign-off eligible: $(PROV_ELIGIBLE)" | tee -a $(RUN_LOG_DIR)/provenance.txt
	@echo "== logs: $(RUN_LOG_DIR)"

signoff:
	@if [ -n "$(PROV_DIRTY)" ] || [ "$(RTL_SHA)" = worktree ]; then \
	  echo "== REFUSING sign-off: $(PROV_LINE)"; \
	  git -C $(REPO) status --porcelain -- tb sw/crates/golden sw/crates/vec sw/crates/cycle sw/crates/spec spec; exit 1; fi
	$(MAKE) -C $(TB) units RUN_ID=$(RUN_ID)

quick:
	$(MAKE) -C $(TB) units SCALE=0.1 RTL_REV=$(RTL_REV) RUN_ID=$(RUN_ID)

all: tools
	@set -e; for s in $(SEEDS); do $(MAKE) -C $(TB) units SEED=$$s SCALE=4 RTL_REV=$(RTL_REV) RUN_ID=$(RUN_ID)-s$$s; done
	@echo "== all passed for SEEDS=$(SEEDS) at SCALE=4"

oracle: $(ORACLE_BIN)
$(ORACLE_BIN): $(TB)/oracle/src/main.rs $(TB)/oracle/Cargo.toml $(wildcard $(REPO)/sw/crates/golden/src/*.rs) $(wildcard $(REPO)/sw/crates/spec/src/*.rs)
	CARGO_TARGET_DIR=$(ORACLE_TARGET) cargo build --quiet --release --manifest-path $(TB)/oracle/Cargo.toml
	@touch $@

# Cycle model + vector generator, built from this checkout into the namespace.
tools: oracle
	cd $(REPO)/sw && CARGO_TARGET_DIR=$(SW_TARGET) cargo build -q --release -p splitsys-cycle -p splitsys-vec

# Committed rtl/ export (atomic: concurrent runs never see a half-written tree).
rtl_export:
ifneq ($(RTL_REV),worktree)
	@if [ ! -f $(RTL_DIR)/.exported ]; then \
	  tmp=$(SPLITSYS_CACHE)/rtl_rev/.tmp-$(RTL_SHA)-$$$$; rm -rf $$tmp; mkdir -p $$tmp; \
	  git -C $(REPO) archive $(RTL_SHA) rtl | tar -x -C $$tmp && touch $$tmp/rtl/.exported && \
	  mkdir -p $(SPLITSYS_CACHE)/rtl_rev/$(RTL_SHA) && \
	  { mv -T $$tmp/rtl $(RTL_DIR) 2>/dev/null || true; }; rm -rf $$tmp; fi
	@test -f $(RTL_DIR)/.exported
endif

# Vectors (atomic per file set).
vec: tools
	@set -e; mkdir -p $(VEC_DIR); \
	if [ ! -f $(VEC_DIR)/unit/requant.json ]; then tmp=$(VEC_DIR)/.tmp-$$$$; \
	  $(VEC_BIN) unit requant --seed $(SEED) --n 100000 --out $$tmp >/dev/null; \
	  mkdir -p $(VEC_DIR)/unit; mv -f $$tmp/unit/requant.json $(VEC_DIR)/unit/requant.json; rm -rf $$tmp; fi; \
	for m in full strip; do if [ ! -f $(VEC_DIR)/gemm/core_$$m.json ]; then tmp=$(VEC_DIR)/.tmp-$$$$-$$m; \
	  $(VEC_BIN) gemm --seed $(SEED) --n 20 --mode $$m --max-m 32 --max-k 96 --max-n 64 --out $$tmp >/dev/null; \
	  mkdir -p $(VEC_DIR)/gemm; mv -f $$tmp/gemm/*.json $(VEC_DIR)/gemm/core_$$m.json; rm -rf $$tmp; fi; done

# One suite: fresh simulator run forced, per-run log, refuses a pass without results.
$(ALL_UNITS): tools rtl_export vec provenance
	@echo "== [$@] SEED=$(SEED) SCALE=$(SCALE) RTL=$(RTL_SHA) tb=$(PROV_TB) golden=$(PROV_GOLDEN)"
	@mkdir -p $(RUN_LOG_DIR); log=$(RUN_LOG_DIR)/$@.log; start=$$(date +%s); \
	res=$(TB_SIM_BUILD)/$@-$(RTL_SHA)$(if $(filter 1,$(WAVES)),-waves)/results.xml; rm -f $$res; \
	echo "provenance: $(PROV_LINE)" > $$log; \
	$(MAKE) -j1 -C $(TB)/$@ sim SEED=$(SEED) SCALE=$(SCALE) RTL_DIR=$(RTL_DIR) VEC_DIR=$(VEC_DIR) \
	  SPLITSYS_SIM_BUILD=$(TB_SIM_BUILD) SIM_BUILD_TAG=-$(RTL_SHA) >> $$log 2>&1; rc=$$?; \
	if [ $$rc -eq 0 ] && { [ ! -f $$res ] || ! grep -q "<testcase" $$res; }; then \
	  echo "== [$@] ERROR: simulation did not run (no fresh $$res); refusing to report a pass"; rc=1; fi; \
	end=$$(date +%s); \
	grep -E "TESTS=|PASS |FAIL " $$log | tail -40; \
	echo "== [$@] rc=$$rc wall=$$((end-start))s log=$$log"; \
	if [ $$rc -ne 0 ]; then tail -60 $$log; fi; \
	exit $$rc

# Delegation to the second verification instance's suites (their Makefiles own the details).
system: oracle
	$(MAKE) -C $(TB)/system RTL_REV=$(RTL_REV) SEED=$(SEED)
cosim:
	$(MAKE) -C $(TB)/cosim RTL_REV=$(RTL_REV)
cosim-check:
	$(MAKE) -C $(TB)/cosim check RTL_REV=$(RTL_REV)

clean:
	rm -rf $(TB_NS)
