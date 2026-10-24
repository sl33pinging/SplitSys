# SplitSys Phase H hardware checklist

Owner: `fpga-integrator`. Other agents contribute rows through the lead.
Phase H starts only after `red-team-reviewer` passes S4 (docs/PLAN.md).

**Rules for the session.** One physical action per message. Before each step the lead states the exact command and what the human should see. On any mismatch: **stop**, capture the evidence named in the row, and hand off for simulation reproduction (`verification-engineer`) **before** any rebuild or reprogram. Never overwrite `fpga/known_good/`.

All commands run from the repo root `C:\Users\danie\FPGA_Projects\Calhack2026` in **Git Bash** unless marked otherwise.

---

## Pre-flight (do once, in order)

| # | Check | Command / action | Expected |
| --- | --- | --- | --- |
| P-1 | USB strategy | Read `docs/ENV.md` "USB strategy". Default **(b)**: host runs natively on Windows on the board's COM port; Windows Vivado programs over JTAG. | Strategy (b) confirmed by the human. |
| P-2 | Board not attached to WSL | PowerShell: `usbipd list` | The Urbana's USB device is **not** "Attached". If it is: `usbipd detach --busid <id>`. |
| P-3 | Board connected | Human: plug the micro-USB cable into the Urbana and the laptop; set the power switch on. | Board power LED on. |
| P-4 | Windows sees the COM port | PowerShell: `Get-CimInstance Win32_SerialPort \| Select-Object DeviceID,Description` (or Device Manager → Ports) | One new COM port appears when the board is plugged in. Record it here: `COM__`. |
| P-5 | Vivado sees the device | `scripts/vivado -mode batch -source fpga/tcl/program.tcl -tclargs <bit>` is the first real use; if in doubt, open Vivado Hardware Manager → Open Target → Auto Connect. | One device `xc7s50_0`. |
| P-6 | Bitstreams present and current | `cat fpga/reports/<tag>/summary.json` for each tag used below; `git log --oneline -1` | `"pass":true`, `"git_dirty":"false"`, `wns_ns` ≥ 0. The `build_id` you will see on the board is the `build_id` field. If `fpga/build/<tag>/top.bit` is missing (it is gitignored), rebuild with the command in the row; the build_id then changes to the new HEAD hash. |

---

## H-1 — Heartbeat, LEDs, 7-segment lamp test

| Field | Value |
| --- | --- |
| Bitstream | `fpga/build/heartbeat_uart_echo/top.bit` |
| Build | `scripts/vivado -mode batch -source fpga/tcl/build.tcl -tclargs heartbeat_uart_echo` (Phase S result: `fpga/reports/heartbeat_uart_echo/summary.json`) |
| build_id | `0xAA1549F7` (git `aa1549f7`; bit md5 `3fcfc1da8a703f00a9f1df00f6eb51d4`). Re-read `summary.json` if rebuilt. |
| Program command | `scripts/vivado -mode batch -source fpga/tcl/program.tcl -tclargs fpga/build/heartbeat_uart_echo/top.bit` |
| Physical action | None beyond pre-flight. After programming: **watch the board for 5 seconds**, then **press and hold BTN0** for 2 s, release, and do the same for BTN1, BTN2, BTN3 (one at a time). |
| Host command | none |
| Expected observation | 1. Console prints `PROGRAM_OK` and `DONE pin = '1'`.<br>2. For the first **2 s** every segment and every decimal point of all **8** digits is lit (lamp test).<br>3. Then the display shows the build_id in hex: `AA15 49F7` with a decimal point lit after exactly one digit (the D0 AN[0] marker).<br>4. **LED15 blinks at 1 Hz** (0.5 s on, 0.5 s off: count 10 blinks in ~10 s).<br>5. **LED14 is steadily on.** All other LEDs off (LED3 may flash once if the COM port is opened).<br>6. While a button BTNn is held, LED(8+n) changes state; it returns when released. |
| Record (these resolve open XDC ambiguities) | (a) the exact 8-character string read left to right, and where the lit DP is. The design assumes D1 = left display and AN[3] = leftmost digit, which gives `AA15 49F7` with the DP after the rightmost `7`. Any permutation is a display-mapping fact for `rtl/board_io.sv`, **not** an H-1 failure.<br>(b) whether LED8–11 are **off when released and on when pressed** (buttons active-high) or the reverse (active-low). `rtl-designer` needs this for BTN1 reset and BTN0 self-test in `top`.<br>(c) whether LED14 is lit (confirms LED logic 1 = on). |
| Failure branch | Capture: the full program console output, a photo of the board (LEDs + display), and `usbipd list`. Likely causes, in order: (1) no JTAG target → cable/power, or device attached to WSL (P-2); (2) `PROGRAM_FAIL: DONE` → wrong part or corrupt bit, rebuild; (3) programmed but nothing lit → check LED14: if LED14 off but some other LEDs on, LED polarity is inverted (unexpected; master XDC does not state it); (4) LED15 wrong rate → clock is not 100 MHz (check schematic oscillator; SPEC CLK_HZ must change via `spec-steward`); (5) 7-seg blank but LEDs fine → anode/segment polarity (master XDC says active-low for both). |

## H-2 — UART echo, 10,000 bytes

| Field | Value |
| --- | --- |
| Bitstream | same as H-1 (`fpga/build/heartbeat_uart_echo/top.bit`, build_id `0xAA1549F7`); no reprogram needed if H-1 just passed. |
| Program command | (only if power-cycled) `scripts/vivado -mode batch -source fpga/tcl/program.tcl -tclargs fpga/build/heartbeat_uart_echo/top.bit` |
| Physical action | none |
| Host command | Windows (strategy b): `splitsys-host --port COM__ echo --bytes 10000` — **PENDING: `host-dashboard` has not written the `echo` subcommand yet; exact flags to be confirmed by `host-dashboard` before S4.** Link settings: 115200 baud, 8N1, no flow control. <br>Fallback sanity check if the host is not ready: PuTTY (Serial, COM__, 115200, 8N1, flow control None); type characters. |
| Expected observation | Host prints 10,000 bytes sent, 10,000 received, **0 mismatches, 0 missing** (and the measured throughput, ~11.5 kB/s at 115200). LED3 lit during the transfer. LED4 (frame error) and LED5 (FIFO overflow) stay **off**. In PuTTY: each typed character appears once. |
| Board facts used | FPGA transmits on `UART_TXD` (B16) and receives on `UART_RXD` (A16): an **assumption** about the master-XDC naming convention (it does not state direction). Effective baud = 100 MHz / (54 × 16) = 115,741 (+0.47 %); the echo top has a 64-byte FIFO, and RX tolerates about ±4 %. |
| Failure branch | Capture: host console output (`> h2_echo.log`), first 64 bytes sent vs received in hex, LED3/4/5 states, a photo. Likely causes, in order: (1) nothing received and LED3 **never** lights while sending → wrong COM port, port held by another program, or RX/TX pins swapped (swap the two `PACKAGE_PIN`s in `fpga/xdc/urbana_splitsys.xdc`, rebuild, retest; record the fact in docs/ENV.md); (2) LED3 lights but nothing comes back → TX pin wrong (same swap) ; (3) garbage bytes → baud mismatch (host not at 115200, or clock not 100 MHz: compare H-1 heartbeat rate); (4) occasional drops with LED5 on → host sending far faster than 115200 or the FIFO too small (report to lead); (5) LED4 on after opening the port only → a glitch on port open; press-and-release BTN1 (clears LED4/LED5) and retry. |
| Then | If the plan raises the baud (SPEC §3.8: only after 10,000 packets at 115200 pass), that is a later H-3 decision, not part of this bitstream. |

## H-3 — Packet layer and memory round trips

TODO (owner: `fpga-integrator` with `host-dashboard`). Bitstream `packet_mem` (`fpga/bringup/packet_mem/`, not built yet). Check: `ping` returns the build_id; WRITE/READ_MEM round trip of 10,000 frames on every region (WBUF, ABUF, OBUF, RQPARAM).

## H-4 — Self-test (BTN0 and SELFTEST opcode)

TODO (owner: `fpga-integrator` with `rtl-designer`). Bitstream `top` (ENABLE_STRIP=1). Check: BTN0 → LED0 (FULL) and LED1 (STRIP) on; `splitsys-host selftest` agrees (pass bytes and both Fletcher-32 values). Depends on the button polarity recorded at H-1.

## H-5 — Random GEMMs, both modes

TODO (owner: `fpga-integrator` with `verification-engineer`, `perf-modeler`). Bitstream `top`. Check: 100 random GEMMs per mode bit-exact; counters equal the cycle model (or one calibration of provisional latencies, then freeze).

## H-6 — Benchmarks and KG1

TODO (owner: `fpga-integrator` with `host-dashboard`, `perf-modeler`). Bitstream `top`. Check: full benchmark suite and M-sweep → `results/hw/`; then archive **KG1** in `fpga/known_good/KG1/` and update `fpga/known_good/CURRENT`.

## H-7 — Dashboard cold start

TODO (owner: `host-dashboard` with `fpga-integrator`). Bitstream `fpga/known_good/CURRENT`. Check: dashboard on the real monitor with HW data; cold-start drill (power-cycle → program → dashboard live) under 60 s.
