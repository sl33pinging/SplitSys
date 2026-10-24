# shellcheck shell=bash
# SplitSys project environment (WSL side). Source it, do not execute it:
#     source scripts/env.sh
# Idempotent: re-sourcing does not grow PATH. Never edits shell rc files.
# Owner: toolchain-engineer. Facts behind every choice here: docs/ENV.md.

if [ -n "${BASH_SOURCE[0]:-}" ]; then
    _ss_here="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
else
    _ss_here="$(pwd)/scripts"
fi
export SPLITSYS_ROOT="$(cd "$_ss_here/.." && pwd)"
unset _ss_here

# Build products live on WSL ext4 (fast), not on /mnt/c (slow 9p/drvfs).
export SPLITSYS_CACHE="${SPLITSYS_CACHE:-$HOME/.cache/splitsys}"
export CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-$SPLITSYS_CACHE/target}"
export SPLITSYS_VENV="${SPLITSYS_VENV:-$SPLITSYS_CACHE/venv}"
# Suggested root for cocotb/Verilator build dirs (tb/ Makefiles may use
# SIM_BUILD=$SPLITSYS_SIM_BUILD/<module>).
export SPLITSYS_SIM_BUILD="${SPLITSYS_SIM_BUILD:-$SPLITSYS_CACHE/sim_build}"
mkdir -p "$SPLITSYS_CACHE" "$CARGO_TARGET_DIR" "$SPLITSYS_SIM_BUILD" 2>/dev/null || true

# Vivado (Windows install, called through scripts/vivado). See docs/ENV.md.
export SPLITSYS_VIVADO_VERSION="${SPLITSYS_VIVADO_VERSION:-2025.2.1}"

# Prepend a directory to PATH once.
_ss_path_prepend() {
    case ":$PATH:" in
        *":$1:"*) PATH="$1:$(printf '%s' ":$PATH:" | sed "s#:$1:#:#g; s#^:##; s#:\$##")" ;;
        *) PATH="$1:$PATH" ;;
    esac
}
# Order (last prepended wins): scripts/ (vivado entry point) < cargo < local < venv
_ss_path_prepend "$SPLITSYS_ROOT/scripts"
_ss_path_prepend "$HOME/.cargo/bin"
_ss_path_prepend "$HOME/.local/bin"
if [ -x "$SPLITSYS_VENV/bin/python" ]; then
    export VIRTUAL_ENV="$SPLITSYS_VENV"
    _ss_path_prepend "$SPLITSYS_VENV/bin"
fi
unset -f _ss_path_prepend
export PATH

# Verilator: use ccache for the generated C++ if available.
if command -v ccache >/dev/null 2>&1; then
    export OBJCACHE=ccache
    export CCACHE_DIR="${CCACHE_DIR:-$SPLITSYS_CACHE/ccache}"
fi

export PIP_DISABLE_PIP_VERSION_CHECK=1
