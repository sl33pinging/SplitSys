# Shared run conventions for every tb/ Makefile (owner: verification-engineer).
# Include it near the top:   include $(TB_ROOT)/common/provenance.mk   (needs TB_ROOT)
#
# 1. Namespaces. Concurrent agents must not share build dirs, logs, or tool binaries.
#    Set SPLITSYS_CACHE_TAG (e.g. ve1, ve2, redteam) in the environment; everything a run
#    writes goes under $(TB_NS) = $(SPLITSYS_CACHE)/tb/<tag> (default tag: "default").
#      $(TB_NS)/sim_build/...        Verilator/cocotb build dirs
#      $(TB_NS)/logs/$(RUN_ID)/...   one directory per run (never overwritten)
#      $(TB_NS)/oracle-target        tb/oracle build (golden from this checkout)
#      $(TB_NS)/sw-target            splitsys-cycle / splitsys-vec builds
#    Content-addressed caches stay shared but are written atomically (tmp dir + mv):
#      $(SPLITSYS_CACHE)/rtl_rev/<commit>/rtl   git-archive of committed rtl/
#      $(SPLITSYS_CACHE)/vec/sw-<tree>/seed<S>  sw/crates/vec output
# 2. Provenance. Every run prints (and writes $(RUN_LOG_DIR)/provenance.txt) the HEAD
#    commit, the tree hashes of rtl (tested commit), tb/, and each Rust crate that produces
#    expected values (golden, vec, cycle, spec) plus spec/, each with "+dirty" when the
#    working tree differs from HEAD. A run is SIGN-OFF ELIGIBLE only when none is dirty;
#    `make -C tb signoff` refuses to start otherwise.

SPLITSYS_CACHE     ?= $(HOME)/.cache/splitsys
SPLITSYS_CACHE_TAG ?= default
TB_NS              := $(SPLITSYS_CACHE)/tb/$(SPLITSYS_CACHE_TAG)
ifndef RUN_ID
RUN_ID := $(shell date +%Y%m%d-%H%M%S)-$(shell echo $$PPID)
endif
export RUN_ID
RUN_LOG_DIR        := $(TB_NS)/logs/$(RUN_ID)
TB_SIM_BUILD       := $(TB_NS)/sim_build

_REPO := $(abspath $(TB_ROOT)/..)
_git   = git -C $(_REPO)
# tree hash of HEAD:<path> and a +dirty suffix if the working tree differs
_tree  = $(shell $(_git) rev-parse --short HEAD:$(1) 2>/dev/null || echo none)$(if $(shell $(_git) status --porcelain -- $(1) 2>/dev/null),+dirty,)
PROV_HEAD   := $(shell $(_git) rev-parse --short HEAD)
PROV_TB     := $(call _tree,tb)
PROV_GOLDEN := $(call _tree,sw/crates/golden)
PROV_VEC    := $(call _tree,sw/crates/vec)
PROV_CYCLE  := $(call _tree,sw/crates/cycle)
PROV_SPECRS := $(call _tree,sw/crates/spec)
PROV_SPEC   := $(call _tree,spec)
PROV_DIRTY  := $(findstring +dirty,$(PROV_TB) $(PROV_GOLDEN) $(PROV_VEC) $(PROV_CYCLE) $(PROV_SPECRS) $(PROV_SPEC))
PROV_LINE    = head=$(PROV_HEAD) rtl=$(RTL_SHA) tb=$(PROV_TB) golden=$(PROV_GOLDEN) vec=$(PROV_VEC) cycle=$(PROV_CYCLE) spec_rs=$(PROV_SPECRS) spec=$(PROV_SPEC) run=$(RUN_ID) ns=$(SPLITSYS_CACHE_TAG)
PROV_ELIGIBLE = $(if $(or $(PROV_DIRTY),$(filter worktree,$(RTL_SHA))),NO (dirty tb/ or Rust reference crates, or RTL_REV=worktree),YES)

.PHONY: provenance
provenance:
	@mkdir -p $(RUN_LOG_DIR)
	@echo "== provenance: $(PROV_LINE)" | tee -a $(RUN_LOG_DIR)/provenance.txt
	@echo "== sign-off eligible: $(PROV_ELIGIBLE)" | tee -a $(RUN_LOG_DIR)/provenance.txt
