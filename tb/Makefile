# SplitSys testbenches (owner: verification-engineer). Run inside WSL with
# scripts/env.sh sourced, e.g. from Windows:  scripts/wsl 'make -C tb units'
#
#   make -C tb units        every unit suite present (S1 sign-off level, SEED=1)
#   make -C tb quick        smoke: every unit suite at SCALE=0.1
#   make -C tb all          sign-off: unit suites at SCALE=4 for SEEDS (default 1 2 3)
#   make -C tb <module>     one suite, e.g. make -C tb pe SEED=7
#   make -C tb oracle       build tb/oracle (thin CLI over splitsys_golden)
#   make -C tb clean
# RTL under test defaults to the committed HEAD (see RTL_REV below).
# Knobs: SEED, SCALE, WAVES=1 (FST, see common/sim.mk), COCOTB_TEST_FILTER=<test>.
# Parallel: make -C tb -j8 units

TB := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))
SPLITSYS_CACHE ?= $(HOME)/.cache/splitsys
SPLITSYS_SIM_BUILD ?= $(SPLITSYS_CACHE)/sim_build
ORACLE_TARGET  := $(SPLITSYS_CACHE)/tb-oracle-target
ORACLE_BIN     := $(ORACLE_TARGET)/release/splitsys-tb-oracle
export SPLITSYS_ORACLE := $(ORACLE_BIN)

# Every unit suite we know about; only those whose RTL exists are run.
ALL_UNITS := pe skew uart counters buffers requant packet array
rtl_of_pe       := pe.sv
rtl_of_skew     := skew.sv
rtl_of_uart     := uart_rx.sv uart_tx.sv
rtl_of_counters := counters.sv
rtl_of_buffers  := buffers.sv
rtl_of_requant  := requant.sv
rtl_of_packet   := packet.sv
rtl_of_array    := array.sv pe.sv skew.sv
# RTL under test. Default: the COMMITTED rtl/ at HEAD, exported to the cache, so a
# sign-off names an exact git hash and never picks up someone's uncommitted edits.
#   RTL_REV=<rev>        test rtl/ as of that commit
#   RTL_REV=worktree     test the working tree (designers' pre-commit check)
RTL_REV ?= HEAD
ifeq ($(RTL_REV),worktree)
  RTL_SHA  := worktree
  RTL_DIR  := $(abspath $(TB)/../rtl)
  RTL_FILES := $(notdir $(wildcard $(RTL_DIR)/*.sv))
else
  RTL_SHA  := $(shell git -C $(TB)/.. rev-parse --short $(RTL_REV))
  RTL_DIR  := $(SPLITSYS_CACHE)/rtl_rev/$(RTL_SHA)/rtl
  RTL_FILES := $(notdir $(shell git -C $(TB)/.. ls-tree --name-only $(RTL_SHA) rtl/))
endif
export RTL_DIR

# Unit vectors from sw/crates/vec, generated per SEED into the cache, keyed by the
# committed sw/ tree hash (the JSON meta records git_hash and git_dirty; tests log it).
SW_TREE := $(shell git -C $(TB)/.. rev-parse --short HEAD:sw)
VEC_DIR := $(SPLITSYS_CACHE)/vec/sw-$(SW_TREE)/seed$(SEED)
export VEC_DIR
VEC_UNITS := requant
present = $(and $(wildcard $(TB)/$(1)/Makefile),$(if $(strip $(foreach f,$(rtl_of_$(1)),$(if $(filter $(f),$(RTL_FILES)),,missing))),,yes))
UNITS := $(foreach u,$(ALL_UNITS),$(if $(call present,$(u)),$(u)))

SEED   ?= 1
SCALE  ?= 1
SEEDS  ?= 1 2 3

.PHONY: units quick all oracle clean rtl_export vec cycle $(ALL_UNITS)

units: $(UNITS)
	@echo "== units passed: $(UNITS) (SEED=$(SEED) SCALE=$(SCALE) RTL=$(RTL_SHA))"
	@echo "== skipped (RTL or tb missing): $(filter-out $(UNITS),$(ALL_UNITS))"

quick:
	$(MAKE) -C $(TB) units SCALE=0.1 RTL_REV=$(RTL_REV)

all: oracle
	@set -e; for s in $(SEEDS); do $(MAKE) -C $(TB) units SEED=$$s SCALE=4 RTL_REV=$(RTL_REV); done
	@echo "== all passed for SEEDS=$(SEEDS) at SCALE=4"

oracle: $(ORACLE_BIN)
$(ORACLE_BIN): $(TB)/oracle/src/main.rs $(TB)/oracle/Cargo.toml $(wildcard $(TB)/../sw/crates/golden/src/*.rs)
	CARGO_TARGET_DIR=$(ORACLE_TARGET) cargo build --quiet --release --manifest-path $(TB)/oracle/Cargo.toml
	@touch $@

# Each suite runs in its own directory; time is printed for the < 2 min budget.
rtl_export:
ifneq ($(RTL_REV),worktree)
	@if [ ! -f $(RTL_DIR)/.exported ]; then rm -rf $(RTL_DIR); mkdir -p $(dir $(RTL_DIR)); 	  git -C $(TB)/.. archive $(RTL_SHA) rtl | tar -x -C $(dir $(RTL_DIR)) && touch $(RTL_DIR)/.exported; fi
endif

# Cycle model binary (counter expectations), built from the working sw/ tree.
CARGO_TARGET_DIR ?= $(SPLITSYS_CACHE)/target
export SPLITSYS_CYCLE_BIN := $(CARGO_TARGET_DIR)/release/splitsys-cycle
cycle:
	cd $(TB)/../sw && cargo build -q --release -p splitsys-cycle

vec:
	@for u in $(VEC_UNITS); do [ -f $(VEC_DIR)/unit/$$u.json ] || 	  (cd $(TB)/../sw && cargo run -q --release -p splitsys-vec -- unit $$u --seed $(SEED) --n 100000 --out $(VEC_DIR)) || exit 1; done

$(ALL_UNITS): oracle rtl_export vec cycle
	@echo "== [$@] SEED=$(SEED) SCALE=$(SCALE) RTL=$(RTL_SHA) ($(RTL_DIR))"
	@start=$$(date +%s); res=$(SPLITSYS_SIM_BUILD)/$@-$(RTL_SHA)$(if $(filter 1,$(WAVES)),-waves)/results.xml; \
	rm -f $$res; \
	$(MAKE) -j1 -C $(TB)/$@ sim SEED=$(SEED) SCALE=$(SCALE) RTL_DIR=$(RTL_DIR) VEC_DIR=$(VEC_DIR) SIM_BUILD_TAG=-$(RTL_SHA) > $(SPLITSYS_CACHE)/tb-$@.log 2>&1; rc=$$?; \
	if [ $$rc -eq 0 ] && { [ ! -f $$res ] || ! grep -q "<testcase" $$res; }; then \
	  echo "== [$@] ERROR: simulation did not run (no fresh $$res); refusing to report a pass"; rc=1; fi; \
	end=$$(date +%s); \
	grep -E "TESTS=|PASS |FAIL |\*\* " $(SPLITSYS_CACHE)/tb-$@.log | grep -vE "^\s*$$" | tail -40; \
	echo "== [$@] rc=$$rc wall=$$((end-start))s log=$(SPLITSYS_CACHE)/tb-$@.log"; \
	if [ $$rc -ne 0 ]; then tail -60 $(SPLITSYS_CACHE)/tb-$@.log; fi; \
	exit $$rc

clean:
	rm -rf $(SPLITSYS_CACHE)/sim_build $(SPLITSYS_CACHE)/tb-*.log
