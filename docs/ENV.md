# ENV — machine facts for SplitSys (verified 2026-10-24, toolchain-engineer)

Every fact here was observed on this laptop with the commands shown, not taken from memory.
Re-check everything with:

```
scripts/wsl 'scripts/check_env.sh'            # from Git Bash (or: scripts/check_env.sh, it re-execs in WSL)
source scripts/env.sh && scripts/check_env.sh # from a WSL shell at the repo root
scripts/check_env.sh --quick                  # skip launching Vivado (~30 s)
scripts/check_env.sh --smoke                  # also run the cocotb+Verilator smoke test
```

Last result: `ALL REQUIRED OK`, exit 0. Optional WARNs: socat, ccache, mold (wait on the sudo command below), Windows-side cargo (pre-Phase-H item).

## How to run things (read this first)

| From | Linux tools (cargo, verilator, make, cocotb) | Vivado |
| --- | --- | --- |
| Git Bash (agents' Bash tool) | `scripts/wsl '<command>'` | `scripts/vivado -mode batch -source fpga/tcl/x.tcl -tclargs ...` (or via `scripts/wsl 'vivado ...'`) |
| WSL shell at repo root | `source scripts/env.sh` once, then run normally | `vivado ...` (resolves to `scripts/vivado` after `env.sh`) |

- **`scripts/wsl '<cmd>'`** runs `<cmd>` with `bash` in WSL, `cd` to the repo root (`/mnt/c/Users/danie/FPGA_Projects/Calhack2026`), with `scripts/env.sh` sourced. The command string is base64-encoded on the Windows side, so quotes, `$`, and `/mnt/...` paths reach WSL unchanged (no `MSYS_NO_PATHCONV` needed). The exit status is preserved (`scripts/wsl 'exit 7'` returns 7); stdin is forwarded. It also works when called from inside WSL.
- **`scripts/env.sh`** (source it; idempotent, never touches rc files) exports:
  - `SPLITSYS_ROOT` (repo root), `SPLITSYS_CACHE=~/.cache/splitsys`
  - `CARGO_TARGET_DIR=~/.cache/splitsys/target`: Rust builds live on ext4, **not** in `sw/target`. Binaries are at `$CARGO_TARGET_DIR/{debug,release}/<bin>`.
  - `SPLITSYS_VENV=~/.cache/splitsys/venv`, and activates it (`VIRTUAL_ENV`, PATH)
  - `SPLITSYS_SIM_BUILD=~/.cache/splitsys/sim_build`: the suggested root for cocotb/Verilator build directories (`SIM_BUILD=$SPLITSYS_SIM_BUILD/<module>` in tb Makefiles). This keeps the slow C++ compile off `/mnt/c`.
  - `SPLITSYS_VIVADO_VERSION=2025.2.1`
  - `OBJCACHE=ccache` and `CCACHE_DIR` when ccache is installed (Verilator picks this up automatically)
  - PATH order: venv/bin, ~/.local/bin, ~/.cargo/bin, `$SPLITSYS_ROOT/scripts`, then the system PATH. The WSL PATH also carries the Windows PATH (interop `appendWindowsPath`), so `vivado` resolves to the **2025.1** install unless `env.sh` is sourced. Always source `env.sh`.
- **`scripts/screenshot <url> <out.png> [W,H]`**: a headless Windows Edge screenshot. See "Dashboard screenshots".

## Inventory

**Host and WSL**
- Laptop: ASUS ROG Zephyrus G14 (2025), Windows 11 Home 10.0.26200.
- WSL 2.6.1.0, kernel `6.6.87.2-microsoft-standard-WSL2`, WSLg 1.0.66. Distro **Ubuntu 24.04.3 LTS**, user `qwer`, `/etc/wsl.conf` has only `[boot] systemd=true`. No `C:\Users\danie\.wslconfig`, so networking is the default NAT with localhost forwarding.
- 24 CPUs (`nproc`), 15 GiB RAM, WSL root ext4 1007 GB with 937 GB free. `C:` (`/mnt/c`) has 160 GB free of 875 GB.

**Tools on PATH before any install** (`command -v` / `--version`)
- apt: build-essential (gcc/g++ 13.3.0), GNU Make 4.3, git 2.43.0, flex 2.6.4, bison 3.8.2, perl 5.38.2, m4, python3 3.12.3 (+venv, dev, pip), libfl-dev, zlib1g-dev, cmake, pkg-config, iverilog 12.0, gtkwave 3.3.116, node v18.19.1 / npm 9.2.0, curl, wget.
- Pre-existing under `$HOME` (not installed by us): `~/.local/bin/uv` 0.9.21, `~/.local/bin/z3` 5.1.0 (Verilator's configure picked it up as its constraint solver), `~/.local/bin/python3.12`.
- Rust: rustup stable 1.99.1 with clippy and rustfmt in `~/.cargo` (installed earlier by the lead; verified, not reinstalled).
- `/usr/local/bin`: `openFPGALoader`, `magic`, `netgen`, `xschem`, `ext2sim`, `ext2spice`, `rawtovcd`. There is **no `/usr/local/bin/vivado` wrapper** (CLAUDE.md assumed one; `scripts/vivado` replaces it). `/opt` is empty.
- `~/oss-cad-suite` (2026-08-18 bundle, **not on PATH**) has yosys, nextpnr-*, iverilog, and a *devel* Verilator 5.051 (`rev v5.050-196`). Keep it off PATH: it would shadow our pinned Verilator.
- Missing: autoconf, automake, help2man, ccache, mold, libgoogle-perftools-dev, numactl, socat, unzip, any Linux Chromium or Playwright. apt's verilator candidate is only 5.020, which is too old for cocotb 2.1.0.

**Other `$HOME` contents (existence noted only, not opened; hackathon rule):** `~/6205_python`, `~/eda`, `~/workspace`, `~/downloads`, `~/src` (ours, see below), `~/openFPGALoader` (source checkout), `~/vivado-boards` (`new/board_files/zedboard`, `old/board_parts`; no Urbana board files). Under `C:\Users\danie\FPGA_Projects` there are old projects (`18240`, `lab01`..`lab04`, `Mini_GPU`, `Simple_CPU`, `Syetemverilog_CPU`, `Projects`, a few loose `.sv`/`.bit` files). **None of their code was opened or copied.**

**Windows side (read-only)**
- `C:\Xilinx`: `2025.1`, `2025.2.1`, `DocNav`, `Downloads`, `xic`.
- The Vivado user init script `C:\Users\danie\AppData\Roaming\Xilinx\Vivado\Vivado_init.tcl` runs on **every** Vivado launch. Its single line is `set_param board.repoPaths [list "//wsl.localhost/Ubuntu-24.04/home/qwer/vivado-boards/new/board_files"]`. This is harmless (it only adds a board repo; there is no Urbana board file there). Not modified.
- Microsoft Edge: `C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe`. Chrome: `C:\Program Files\Google\Chrome\Application\chrome.exe`.
- usbipd-win 5.3.0 at `C:\Program Files\usbipd-win\usbipd.exe`.
- Windows Rust: `where.exe cargo rustup` finds nothing. **Not installed** (see USB strategy).
- Windows Python 3.13 and nvm/node exist on the Windows PATH (not used by the project).

## Installed (what and how). All user-level, no sudo.

| What | How | Where |
| --- | --- | --- |
| autoconf 2.72 | `scripts/setup_env.sh autotools` (GNU tarball, `./configure --prefix=~/.local`) | `~/.local/bin/autoconf`, source in `~/src/autoconf-2.72` |
| help2man 1.49.3 | same | `~/.local/bin/help2man` |
| **Verilator 5.052** (tag `v5.052`, 2026-09-05, the latest stable tag on GitHub at setup time) | `scripts/setup_env.sh verilator`: git clone, checkout tag, `autoconf && ./configure --prefix=$HOME/.local && make -j24 && make install`. Build log: `~/.cache/splitsys/verilator_build.log` | `~/.local/bin/verilator`, `~/.local/share/verilator`; source `~/src/verilator` |
| Python venv (python 3.12.3) with cocotb 2.1.0, pytest 9.1.1 | `scripts/setup_env.sh venv` (`pip install -r scripts/requirements.txt`) | `~/.cache/splitsys/venv` |

**Venv decision:** the venv is at `~/.cache/splitsys/venv` (ext4), not `.venv/` on `/mnt/c`, for import and build speed. `env.sh` activates it. `.venv/` is unused (and gitignored).

Verilator was configured before the optional apt packages arrived, so it has no tcmalloc and no mold. That costs only some speed. Do **not** rebuild it mid-event (it works). ccache *is* used at simulation time once installed, via `OBJCACHE`.

Recreate everything: `scripts/setup_env.sh` (steps `rust verilator venv`; each skips what already exists).

## Versions (pinned)

| Tool | Version | Pin location |
| --- | --- | --- |
| rustc / cargo | 1.99.1 stable (clippy 0.1.99, rustfmt 1.10.0) | rustup default `stable-x86_64-unknown-linux-gnu`. Do not `rustup update` mid-event. |
| Verilator | 5.052 | `VERILATOR_TAG` in `scripts/setup_env.sh` |
| cocotb | 2.1.0 | `scripts/requirements.txt` |
| cocotb's minimum Verilator | **5.036** ("cocotb supports Verilator 5.036+", https://docs.cocotb.org/en/v2.1.0/simulator_support.html) | enforced by `check_env.sh` |
| pytest | 9.1.1 | `scripts/requirements.txt` |
| Python | 3.12.3 | system python3 (venv base) |
| Vivado | **2025.2.1** (64-bit), batch mode verified | `SPLITSYS_VIVADO_VERSION` in `scripts/env.sh` |
| iverilog | 12.0 (apt, optional) | |

cocotb 2.x notes for testbench authors: use `COCOTB_TOPLEVEL` / `COCOTB_TEST_MODULES` in Makefiles (the 1.x names `TOPLEVEL`/`MODULE` are deprecated), `Clock(dut.clk, 10, unit="ns")` (`unit`, not `units`), and `cocotb.start_soon(...)`. `scripts/smoke_cocotb.sh` is a minimal working example of all three.

## Vivado path rules

**Chosen: Vivado 2025.2.1** at `C:\Xilinx\2025.2.1\Vivado\bin\vivado.bat`. It is the version CLAUDE.md names, it runs in batch mode, and it has the target part. 2025.1 is also installed and is a fallback (`SPLITSYS_VIVADO_VERSION=2025.1 scripts/vivado ...`), but it was not smoke-tested.

**Smoke test** (`scripts/vivado -mode batch -source scripts/vivado_smoke.tcl`, about 30 s, exit 0):
```
SMOKE_VERSION: 2025.2.1
SMOKE_PARTS: xc7s50csga324-1 xc7s50csga324-1IL xc7s50csga324-1Q xc7s50csga324-2 xc7s50fgga484-1 ... xc7s50ftgb196-2
SMOKE_OK xc7s50csga324-2
```
So `xc7s50csga324-2` exists. The csga324 speed grades available are -1, -1IL, -1Q, and -2. The grade on the physical board still has to be confirmed against its marking or the RealDigital docs in Phase H. CLAUDE.md says -2.

**`scripts/vivado` rules** (it works the same from Git Bash and WSL):
1. Vivado is a Windows program, started as `cmd.exe /d /c C:\Xilinx\2025.2.1\Vivado\bin\vivado.bat <args>`. Its working directory must be on `C:`. If you call it from a WSL-only directory (`/tmp`, `~`), the script `cd`s to the repo root first.
2. **Prefer repo-relative paths with forward slashes** (`fpga/tcl/build.tcl`). Vivado's Tcl accepts `/` on Windows.
3. Absolute `/mnt/c/...` (WSL) or `/c/...` (Git Bash) arguments are rewritten to `C:/...`. Absolute paths that exist only in WSL (`/home/...`, `/tmp/...`) are **rejected** (exit 2). Vivado on `\\wsl.localhost\...` UNC paths is unreliable, so keep every Vivado input and output under the repo. Inside Tcl, derive paths from `[file dirname [info script]]`, not from WSL paths.
4. No spaces or cmd metacharacters (`& | < > ^ %`) in any argument (rejected, exit 2), because cmd.exe re-parses the command line.
5. Unless you pass `-log/-journal/-nolog/-nojournal`, logs go to `fpga/build/vivado_logs/vivado_<stamp>.{log,jou}` (gitignored), not the cwd. `-version` gets no log flags.
6. The exit code propagates: Tcl `exit 3` gives `scripts/vivado` exit 3 (tested from WSL with `-tclargs foo bar`, and `$argv` arrived as `foo bar`).
7. `scripts/vivado --which` prints the `.bat` that would run.
8. Inside WSL after `source scripts/env.sh`, plain `vivado` is `scripts/vivado`, so the canonical commands in CLAUDE.md (`vivado -mode batch -source fpga/tcl/build.tcl -tclargs <tag>`) work as written from the repo root.

## Board files and XDC

**Found:** the RealDigital Urbana master XDC.
- **Origin:** `C:\Users\danie\Downloads\Urbana.xdc` (WSL: `/mnt/c/Users/danie/Downloads/Urbana.xdc`), 24993 bytes, 468 lines, mtime 2026-03-04, md5 `efdc408b5aab32e8ff03770e47cd6c47` (CRLF original). Header: `# URBANA BOARD CONSTRAINTS V2I1 1/3/2023`. It covers every board resource (100 MHz clock, switches, LEDs, buttons, RGB LEDs, two 7-segment displays, HDMI, USB host, SD, audio, analog, UART + BLE UART, servo, PMODs, DDR3).
- **Copied verbatim** to `fpga/xdc/urbana_master.xdc`. Git normalizes it to LF on commit (`.gitattributes`); the content is otherwise unchanged.
- The same folder holds the schematic `C:\Users\danie\Downloads\Urbana_V2I1_SCH_2023-01-03.PDF` (plus two duplicate downloads). It was not copied; read it in place when needed.
- Facts read from the file (for `fpga-integrator`, who should still read the file itself): clock `CLK_100MHZ` on **N15** LVCMOS33 (its `create_clock` line is commented out and names port `clk_100MHz`). USB-UART: **`UART_TXD` B16, `UART_RXD` A16** (LVCMOS33). Switches use LVCMOS25, LEDs LVCMOS33. `CFGBVS VCCO`, `CONFIG_VOLTAGE 3.3`, `SPI_buswidth 4`.
- **Vendor-file quirks (do not "fix" the master; handle them in the project XDC):** every `set_property` line is *active* (318 of them, none commented out), so a project XDC must use only the needed lines. HDMI lines use the misspelled IOSTANDARD `TDMS_33` (Vivado's name is `TMDS_33`). There is a port typo `HDMI_DO_P` (letter O) and a port name with a hyphen, `HDMI_I2C-SDA`. The header says V2I1. Confirm against the board silkscreen in Phase H.

**Searches run (bounded, read-only):**
```
find $HOME -maxdepth 4 (prune .cache .cargo .rustup src) \( -iname '*urbana*' -o -iname '*.xdc' -o -iname '*board*' \)
find ~/vivado-boards -maxdepth 6 \( -iname '*urbana*' -o -iname '*.xdc' \)
find /mnt/c/Xilinx -maxdepth 6 \( -iname '*urbana*' -o -iname '*realdigital*' \)
find /mnt/c/Users/*/{Downloads,Documents,Desktop} /mnt/c/Users/*/OneDrive/{Documents,Desktop} -maxdepth 4 \( -iname '*urbana*' -o -iname '*.xdc' \)
find /mnt/c/Users/danie/FPGA_Projects -maxdepth 3 (prune Calhack2026) \( -iname '*urbana*' -o -iname '*.xdc' \)
```
Other candidates found and **not used**: `Downloads/Boolean240.xdc` and `Downloads/boolean_original.xdc` (Boolean board, a different board), `Downloads/pwm_servo_control.xdc` (a project file), openFPGALoader `spiOverJtag/constr_*.xdc` (generic), and `FPGA_Projects/{lab01,lab02}/xdc/top_level.xdc` (coursework derived from the Urbana file; not opened beyond line 1, not copied). There are no Urbana Vivado board files in `/mnt/c/Xilinx` or `~/vivado-boards`. They are not needed, because the project uses the part number plus the XDC.

## Dashboard screenshots

Use **Windows-side headless Edge**. No Linux browser and no sudo needed.
```
scripts/screenshot http://127.0.0.1:8080/ results/plots/scene1.png 1920,1080
```
- Verified: `python3 -m http.server 8765 --bind 127.0.0.1` in WSL gives HTTP 200 from Windows `curl` (WSL2 localhost forwarding works). Edge rendered it to a 1920x1080 PNG, from both Git Bash and WSL.
- Raw command, if needed: `"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe" --headless=new --disable-gpu --hide-scrollbars --no-first-run --user-data-dir=%TEMP%\splitsys_edge_headless --window-size=1920,1080 --virtual-time-budget=5000 --screenshot=C:\path\out.png http://127.0.0.1:8080/`
- Gotchas: `--user-data-dir` is required (without it, Edge hands off to the running browser instance and writes nothing). `msedge.exe` can return before the PNG is flushed, so the script waits up to 30 s for the file. The output must be on `C:` (any repo path works). `--virtual-time-budget=5000` gives page JS 5 s of virtual time to render.
- Playwright and Linux Chromium were **not** installed. They would need sudo for system libraries.

## USB strategy

**Default: (b), pending the human's confirmation in Phase H.** Build and run `splitsys-host` natively on Windows against the board's COM port. Windows Vivado programs over JTAG. Nothing moves between WSL and Windows during the demo.
- Windows-side Rust is **not installed** (`where.exe cargo rustup` finds nothing). **Pre-Phase-H checklist item (human, Windows approval):** install rustup-init.exe (stable, MSVC host). That also needs the MSVC build tools / Windows SDK if they are absent; check `where.exe link` then. Then run `cargo build --release -p splitsys-host` from `C:\Users\danie\FPGA_Projects\Calhack2026\sw` in a Windows shell. Do not use the WSL `CARGO_TARGET_DIR` for this.
- Strategy (a) fallback: usbipd-win 5.3.0 is installed (`usbipd.exe` present), so `usbipd attach --wsl` is available if (b) fails.

## Sudo request (one command, optional)

The lead has already sent this. Nothing *required* waits on it:
```
sudo apt-get update && sudo apt-get install -y autoconf automake help2man ccache mold libgoogle-perftools-dev numactl socat unzip
```
It provides: `socat` (handy for cosim debugging; `check_env` marks it OPTIONAL), `ccache` (faster repeated Verilator C++ builds, auto-enabled by `env.sh`), and `unzip`. autoconf and help2man are already user-built in `~/.local`. No other sudo packages are needed.

## Known issues

- **WSL PATH includes Windows PATH.** Without `source scripts/env.sh`, `vivado` is Vivado **2025.1**, and Windows tools (`make` from ezwinports, Git's `usr/bin`) sit at the end of PATH. Always source `env.sh` (`scripts/wsl` does it for you).
- **Builds on `/mnt/c` are slow** (drvfs). Keep build outputs under `~/.cache/splitsys` (`CARGO_TARGET_DIR`, `SPLITSYS_SIM_BUILD`). Sources stay in the repo on `C:` so Windows Vivado can read them.
- A cocotb+Verilator smoke run takes about 30 s, mostly Verilator's C++ compile. ccache will help once installed.
- Vivado batch startup is about 30 s per launch. Batch several steps into one Tcl run.
- cmd.exe prints `UNC paths are not supported` if a Windows program is started from a WSL-only cwd. `scripts/vivado` and `scripts/screenshot` avoid this by using a `C:` cwd.
- Twice, early in setup, stdout from a `scripts/wsl` call was missing from the agent tool output when that call was chained with other Windows commands in the same Bash invocation. It did not reproduce in isolation (10+ reruns). If output looks truncated, rerun the `scripts/wsl` call alone and trust its exit code.
- `C:` has 160 GB free. Vivado runs under `fpga/build/` are fine, but do not keep many full runs.
