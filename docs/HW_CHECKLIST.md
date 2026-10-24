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

## Board facts (verified in Phase S from the vendor files)

Sources: master XDC `fpga/xdc/urbana_master.xdc` ("URBANA BOARD CONSTRAINTS V2I1 1/3/2023") and the schematic `C:\Users\danie\Downloads\Urbana_V2I1_SCH_2023-01-03.PDF` (19 pages, V2I1). Schematic facts were read from the PDF's embedded net/pin annotations (text extraction with pypdf; the pages were not rendered).

| Fact | Value | Evidence |
| --- | --- | --- |
| Clock | 100 MHz on FPGA N15 | master:2–4; sch p11: oscillator IC16 `ASEM1-100.000MHZ` → buffer IC15 → net `CLK_100MHZ` → N15 |
| USB-UART direction | **FPGA receives on B16, transmits on A16.** The master-XDC names are from the FTDI's side. | sch p12: B16 = net `UART_TXD`, A16 = net `UART_RXD`. sch p10: `UART_TXD` = FT2232HQ (IC6) pin 38 = BDBUS0, `UART_RXD` = pin 39 = BDBUS1; channel A pins 16–19 are JTAG (FT_TCK/TDI/TDO/TMS), so channel B is the UART, where BDBUS0 = TXD (FTDI output) and BDBUS1 = RXD (FTDI input), per the FTDI DS_FT2232H pin table (vendor datasheet, from knowledge; not in the repo). Project XDC: port `UART_RXD` → B16, `UART_TXD` → A16. |
| Buttons BTN0–3 | **Active high** (pressed = 1), external 10k pull-down | sch p4: each button ties `VCCp2V5` to a node with 10k to GND (R60–R63); that node reaches net BTNn through 10k (R43/R47/R52/R57). FPGA pins J2/J1/G2/H2 (sch p13, master:48–51). |
| LEDs LD0–15 | **Active high** | sch p4: net LDn → 330R → LED anode; all LED cathodes on GND. |
| 7-seg anodes D0/D1_AN | **Active low** | sch p4: each AN net drives a PNP (MBT3906DW1) high-side switch base through 1.5k. Master XDC also says "Active LOW". |
| 7-seg segments D0/D1_SEG | **Active low**, SEG[0..7] = A, B, C, D, E, F, G, DP | sch p4: SEG nets → 100R → cathode pins of the common-anode KW4-281ASB (SEG_0→pin 11 CA, SEG_1→7 CB, SEG_2→4 CC, SEG_3→2 CD, SEG_4→1 CE, SEG_5→10 CF, SEG_6→5 CG, SEG_7→3 DP). |
| Which display is left; which AN is the leftmost digit | **Unknown**: this depends on board layout, not on the schematic netlist | Measured at H-1 (the design assumes D1 = left, AN[3] = leftmost: `rtl/board_io.sv` `LEFT_IS_D1=1`, `AN0_IS_RIGHT=1`). |

---

## H-1 — Heartbeat, LEDs, 7-segment lamp test

| Field | Value |
| --- | --- |
| Bitstream | `fpga/build/heartbeat_uart_echo/top.bit` |
| Build | `scripts/vivado -mode batch -source fpga/tcl/build.tcl -tclargs heartbeat_uart_echo` (Phase S result: `fpga/reports/heartbeat_uart_echo/summary.json`) |
| build_id | `0x2148101A` (git `2148101a`; bit md5 `c537e86cc6e33acb4561748e3a75ec98`). Re-read `summary.json` if rebuilt. |
| Program command | `scripts/vivado -mode batch -source fpga/tcl/program.tcl -tclargs fpga/build/heartbeat_uart_echo/top.bit` |
| Physical action | None beyond pre-flight. After programming: **watch the board for 5 seconds**, then **press and hold BTN0** for 2 s, release, and do the same for BTN1, BTN2, BTN3 (one at a time). |
| Host command | none |
| Expected observation | 1. Console prints `PROGRAM_OK` and `DONE pin = '1'`.<br>2. For the first **2 s** every segment and every decimal point of all **8** digits is lit (lamp test).<br>3. Then the display shows the build_id in hex: `2148 101A` with a decimal point lit after exactly one digit (the D0 AN[0] marker).<br>4. **LED15 blinks at 1 Hz** (0.5 s on, 0.5 s off: count 10 blinks in ~10 s).<br>5. **LED14 is steadily on.** All other LEDs off (LED3 may flash once if the COM port is opened).<br>6. While a button BTNn is held, LED(8+n) is **on**; it goes off when released (the schematic says the buttons are active high). |
| Record (these resolve open XDC ambiguities) | (a) the exact 8-character string read left to right, and where the lit DP is. The design assumes D1 = left display and AN[3] = leftmost digit, which gives `2148 101A` with the DP after the rightmost `A`. Any permutation is a display-mapping fact for `rtl/board_io.sv`, **not** an H-1 failure.<br>(b) confirm LED8–11 are off when released and on when pressed (active high, sch p4). If they are reversed, `top` needs `BTN_ACTIVE_LOW=1`; tell `rtl-designer`. Until then BTN1 would hold `top` in reset.<br>(c) LED14 lit (LED logic 1 = on, sch p4). |
| Failure branch | Capture: the full program console output, a photo of the board (LEDs + display), and `usbipd list`. Likely causes, in order: (1) no JTAG target → cable/power, or device attached to WSL (P-2); (2) `PROGRAM_FAIL: DONE` → wrong part or corrupt bit, rebuild; (3) programmed but nothing lit → check LED14 (active high per sch p4; all-dark means the design is not running: check DONE, the clock); (4) LED15 wrong rate → clock is not 100 MHz (check schematic oscillator; SPEC CLK_HZ must change via `spec-steward`); (5) 7-seg blank but LEDs fine → anode/segment polarity (active-low per master XDC and sch p4). |

## H-2 — UART echo, 10,000 bytes

| Field | Value |
| --- | --- |
| Bitstream | same as H-1 (`fpga/build/heartbeat_uart_echo/top.bit`, build_id `0x2148101A`); no reprogram needed if H-1 just passed. |
| Program command | (only if power-cycled) `scripts/vivado -mode batch -source fpga/tcl/program.tcl -tclargs fpga/build/heartbeat_uart_echo/top.bit` |
| Physical action | none |
| Host command | Windows (strategy b): `splitsys-host --transport serial:COM__ echo --bytes 10000` (the subcommand exists in `sw/crates/host/src/main.rs`; default baud 115200 from the spec; the Windows build of the host is pending, see S4). Link settings: 115200 baud, 8N1, no flow control. <br>Fallback sanity check if the host is not ready: PuTTY (Serial, COM__, 115200, 8N1, flow control None); type characters. |
| Expected observation | Host prints 10,000 bytes sent, 10,000 received, **0 mismatches, 0 missing** (and the measured throughput, ~11.5 kB/s at 115200). LED3 lit during the transfer. LED4 (frame error) and LED5 (FIFO overflow) stay **off**. In PuTTY: each typed character appears once. |
| Board facts used | FPGA **receives on B16** (schematic net `UART_TXD` = FTDI TXD) and **transmits on A16** (net `UART_RXD` = FTDI RXD); see Board facts (sch p10, p12). Effective baud = 100 MHz / (54 × 16) = 115,741 (+0.47 %); the echo top has a 64-byte FIFO, and RX tolerates about ±4 %. |
| Failure branch | Capture: host console output (`> h2_echo.log`), first 64 bytes sent vs received in hex, LED3/4/5 states, a photo. Likely causes, in order: (1) nothing received and LED3 **never** lights while sending → wrong COM port, or the port is held by another program. A pin swap is unlikely, since the schematic settles the direction, but as a last resort swap the two `PACKAGE_PIN`s in `fpga/xdc/urbana_splitsys.xdc`, rebuild, retest, and record the result in docs/ENV.md; (2) LED3 lights but nothing comes back → TX pin wrong (same swap) ; (3) garbage bytes → baud mismatch (host not at 115200, or clock not 100 MHz: compare H-1 heartbeat rate); (4) occasional drops with LED5 on → host sending far faster than 115200 or the FIFO too small (report to lead); (5) LED4 on after opening the port only → a glitch on port open; press and release BTN1 (clears LED4/LED5) and retry. |
| Then | If the plan raises the baud (SPEC §3.8: only after 10,000 packets at 115200 pass), that is a later H-3 decision, not part of this bitstream. |

## H-3 — Packet layer and memory round trips

| Field | Value |
| --- | --- |
| Bitstream | `fpga/build/packet_mem/top.bit` (top `fpga/bringup/packet_mem/packet_mem.sv`: production uart_rx/tx + packet + 4× buffers + board_io; sequencer, self-test and counters are **stubs**) |
| Build | `scripts/vivado -mode batch -source fpga/tcl/build.tcl -tclargs packet_mem` (Phase S result: `fpga/reports/packet_mem/summary.json`: WNS +2.692 ns, 1316 LUT, 1216 FF, 25 BRAM) |
| build_id | `0x9FD6577E` (git `9fd6577e`; bit md5 `a3d969551ee39c018312fc2b23ad7bd5`). Re-read `summary.json` if rebuilt. |
| Program command | `scripts/vivado -mode batch -source fpga/tcl/program.tcl -tclargs fpga/build/packet_mem/top.bit` |
| Physical action | none |
| Host command | 1. `splitsys-host --transport serial:COM__ ping`<br>2. Memory round trip, 10,000 frames over all four regions: **PENDING: `host-dashboard` to add a `memtest --frames N` subcommand** (the driver already has `write_mem`/`read_mem` in `sw/crates/host/src/client.rs`). It should do random region, random unaligned address and random length (1..1019 bytes, within the region), WRITE_MEM then READ_MEM, compare, and print the counts. |
| Expected observation | 1. `PING ok: build_id 9fd6577e  proto v1`.<br>2. memtest: 10,000 frames, 0 mismatches, 0 NACK/timeouts, every region exercised. LED15 1 Hz heartbeat, LED3 flickers during traffic. The 7-segment display shows `-` on every digit (board_io before the first SET_DISPLAY). |
| Do NOT use with this bitstream | `selftest` (the stub answers pass = 0 with zero checksums), `run` / `bench` (RUN_GEMM is accepted but nothing is computed), `READ_COUNTERS` (all zeros). |
| Failure branch | Capture: host console with `RUST_LOG`/verbose output if available, the failing frame (region, address, length, first differing byte), a photo. Likely causes, in order: (1) PING times out but H-2 echo passed → packet-layer bug: reproduce with `bash fpga/bringup/packet_mem/sim/run.sh` and the same bytes (`verification-engineer`); (2) NACK_CRC bursts → link noise or baud mismatch: rerun H-2; (3) data mismatches in one region only → buffers or mux for that region; (4) mismatches at word boundaries → byte-enable handling. |

## H-4 — Self-test (BTN0 and SELFTEST opcode)

TODO (owner: `fpga-integrator` with `rtl-designer`). Bitstream `top` (ENABLE_STRIP=1). Check: BTN0 → LED0 (FULL) and LED1 (STRIP) on; `splitsys-host selftest` agrees (pass bytes and both Fletcher-32 values). Depends on the button polarity recorded at H-1.

## H-5 — Random GEMMs, both modes

TODO (owner: `fpga-integrator` with `verification-engineer`, `perf-modeler`). Bitstream `top`. Check: 100 random GEMMs per mode bit-exact; counters equal the cycle model (or one calibration of provisional latencies, then freeze).

## H-6 — Benchmarks and KG1

TODO (owner: `fpga-integrator` with `host-dashboard`, `perf-modeler`). Bitstream `top`. Check: full benchmark suite and M-sweep → `results/hw/`; then archive **KG1** in `fpga/known_good/KG1/` and update `fpga/known_good/CURRENT`.

## H-7 — Dashboard cold start

TODO (owner: `host-dashboard` with `fpga-integrator`). Bitstream `fpga/known_good/CURRENT`. Check: dashboard on the real monitor with HW data; cold-start drill (power-cycle → program → dashboard live) under 60 s.
