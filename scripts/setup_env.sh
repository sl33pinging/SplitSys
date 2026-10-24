#!/usr/bin/env bash
# Idempotent, user-level (no sudo) setup of the SplitSys toolchain in WSL.
#
#   scripts/setup_env.sh               # everything below, skipping what exists
#   scripts/setup_env.sh rust|autotools|verilator|venv   # one step
#
# Never upgrades a tool that already works. Never edits rc files.
# System packages that need sudo are NOT installed here; see docs/ENV.md.
# Owner: toolchain-engineer.
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=/dev/null
source "$HERE/env.sh"

PREFIX="$HOME/.local"
SRC="$HOME/src"
JOBS="$(nproc)"
mkdir -p "$PREFIX/bin" "$SRC"

# Pinned versions (see docs/ENV.md "Versions").
VERILATOR_TAG="${VERILATOR_TAG:-v5.052}"   # latest stable tag at setup time (>= cocotb min 5.036)
AUTOCONF_VER="2.72"
HELP2MAN_VER="1.49.3"
GNU_MIRROR="https://ftp.gnu.org/gnu"

log() { printf '[setup_env] %s\n' "$*"; }

step_rust() {
    if command -v rustup >/dev/null 2>&1; then
        log "rustup present: $(rustc --version)"
    else
        log "installing rustup stable (user-level)"
        curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs \
            | sh -s -- -y --no-modify-path --profile minimal --default-toolchain stable
    fi
    rustup component add clippy rustfmt >/dev/null
    log "rust: $(rustc --version); $(cargo clippy --version); $(rustfmt --version)"
}

# autoconf/help2man are needed to build Verilator from git. Prefer the apt
# packages (sudo request in docs/ENV.md); otherwise build GNU tarballs into ~/.local.
gnu_build() { # name version
    local name="$1" ver="$2" dir="$SRC/$1-$2"
    log "building $name $ver into $PREFIX"
    if [ ! -d "$dir" ]; then
        curl -fsSL "$GNU_MIRROR/$name/$name-$ver.tar.xz" -o "$SRC/$name-$ver.tar.xz"
        tar -C "$SRC" -xf "$SRC/$name-$ver.tar.xz"
    fi
    ( cd "$dir" && ./configure --prefix="$PREFIX" >/dev/null && make -j"$JOBS" >/dev/null && make install >/dev/null )
}
step_autotools() {
    command -v autoconf >/dev/null 2>&1 && log "autoconf present: $(command -v autoconf)" || gnu_build autoconf "$AUTOCONF_VER"
    command -v help2man >/dev/null 2>&1 && log "help2man present: $(command -v help2man)" || gnu_build help2man "$HELP2MAN_VER"
}

step_verilator() {
    local want="${VERILATOR_TAG#v}"
    if command -v verilator >/dev/null 2>&1 && verilator --version | grep -q "Verilator $want "; then
        log "verilator present: $(verilator --version)"
        return 0
    fi
    step_autotools
    local dir="$SRC/verilator"
    if [ ! -d "$dir/.git" ]; then
        git clone --quiet https://github.com/verilator/verilator "$dir"
    fi
    ( cd "$dir"
      git fetch --quiet --tags
      git checkout --quiet "$VERILATOR_TAG"
      unset VERILATOR_ROOT
      autoconf
      ./configure --prefix="$PREFIX"
      make -j"$JOBS"
      make install )
    hash -r
    log "verilator installed: $(verilator --version)"
}

step_venv() {
    if [ ! -x "$SPLITSYS_VENV/bin/python" ]; then
        log "creating venv at $SPLITSYS_VENV"
        python3 -m venv "$SPLITSYS_VENV"
    fi
    "$SPLITSYS_VENV/bin/python" -m pip install --quiet -r "$HERE/requirements.txt"
    log "venv: $("$SPLITSYS_VENV/bin/python" -c 'import cocotb,pytest;print("cocotb",cocotb.__version__,"pytest",pytest.__version__)')"
}

if [ $# -eq 0 ]; then
    set -- rust verilator venv
fi
for s in "$@"; do
    "step_$s"
done
log "done: $*"
