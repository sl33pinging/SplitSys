# SplitSys testbenches (owner: verification-engineer). Run inside WSL with
# scripts/env.sh sourced, e.g. from Windows:  scripts/wsl 'make -C tb units'
#
#   make -C tb units        every unit suite present (S1 sign-off level, SEED=1)
#   make -C tb quick        smoke: every unit suite at SCALE=0.1
#   make -C tb all          sign-off: unit suites at SCALE=4 for SEEDS (default 1 2 3)
#   make -C tb <module>     one suite, e.g. make -C tb pe SEED=7
#   make -C tb oracle       build tb/oracle (thin CLI over splitsys_golden)
#   make -C tb clean
# Knobs: SEED, SCALE, WAVES=1 (FST, see common/sim.mk), COCOTB_TEST_FILTER=<test>.
# Parallel: make -C tb -j8 units

TB := $(abspath $(dir $(lastword $(MAKEFILE_LIST))))
SPLITSYS_CACHE ?= $(HOME)/.cache/splitsys
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
present = $(and $(wildcard $(TB)/$(1)/Makefile),$(if $(strip $(foreach f,$(rtl_of_$(1)),$(if $(wildcard $(TB)/../rtl/$(f)),,missing))),,yes))
UNITS := $(foreach u,$(ALL_UNITS),$(if $(call present,$(u)),$(u)))

SEED   ?= 1
SCALE  ?= 1
SEEDS  ?= 1 2 3

.PHONY: units quick all oracle clean $(ALL_UNITS)

units: $(UNITS)
	@echo "== units passed: $(UNITS) (SEED=$(SEED) SCALE=$(SCALE))"
	@echo "== skipped (RTL or tb missing): $(filter-out $(UNITS),$(ALL_UNITS))"

quick:
	$(MAKE) -C $(TB) units SCALE=0.1

all: oracle
	@set -e; for s in $(SEEDS); do $(MAKE) -C $(TB) units SEED=$$s SCALE=4; done
	@echo "== all passed for SEEDS=$(SEEDS) at SCALE=4"

oracle: $(ORACLE_BIN)
$(ORACLE_BIN): $(TB)/oracle/src/main.rs $(TB)/oracle/Cargo.toml $(wildcard $(TB)/../sw/crates/golden/src/*.rs)
	CARGO_TARGET_DIR=$(ORACLE_TARGET) cargo build --quiet --release --manifest-path $(TB)/oracle/Cargo.toml
	@touch $@

# Each suite runs in its own directory; time is printed for the < 2 min budget.
$(ALL_UNITS): oracle
	@echo "== [$@] SEED=$(SEED) SCALE=$(SCALE)"
	@start=$$(date +%s); \
	$(MAKE) -C $(TB)/$@ SEED=$(SEED) SCALE=$(SCALE) > $(SPLITSYS_CACHE)/tb-$@.log 2>&1; rc=$$?; \
	end=$$(date +%s); \
	grep -E "TESTS=|PASS |FAIL |\*\* " $(SPLITSYS_CACHE)/tb-$@.log | grep -vE "^\s*$$" | tail -40; \
	echo "== [$@] rc=$$rc wall=$$((end-start))s log=$(SPLITSYS_CACHE)/tb-$@.log"; \
	if [ $$rc -ne 0 ]; then tail -60 $(SPLITSYS_CACHE)/tb-$@.log; fi; \
	exit $$rc

clean:
	rm -rf $(SPLITSYS_CACHE)/sim_build $(SPLITSYS_CACHE)/tb-*.log
