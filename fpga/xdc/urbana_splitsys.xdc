# =============================================================================
# urbana_splitsys.xdc - SplitSys project constraints (owner: fpga-integrator)
# -----------------------------------------------------------------------------
# Every pin and IOSTANDARD below is copied verbatim from the RealDigital master
# XDC in this directory (fpga/xdc/urbana_master.xdc, header "URBANA BOARD
# CONSTRAINTS V2I1 1/3/2023"; origin recorded in docs/ENV.md). Port names are
# kept identical to the master file so any line can be checked by grep, with
# ONE deliberate exception: the two USB-UART pins (urbana_uart*.xdc).
# "master:<line>" gives the line number in urbana_master.xdc; "sch p<N>" is a
# page of the Urbana schematic Urbana_V2I1_SCH_2023-01-03.PDF (docs/ENV.md).
#
# Top-level port contract for every SplitSys top that uses this file
# (bring-up tops and rtl/top.sv):
#   input  CLK_100MHZ
#   input  [3:0] BTN          output [15:0] LED
#   output [3:0] D0_AN        output [7:0]  D0_SEG
#   output [3:0] D1_AN        output [7:0]  D1_SEG
#   input  UART_RXD           output UART_TXD   (FPGA's point of view)
# A top with a different port set gets its own XDC in fpga/bringup/<top>/.
# =============================================================================

# ---- Clock: 100 MHz oscillator (master:2 "clk input is from the 100 MHz
# oscillator on Urbana board"; master:3 has the commented create_clock with
# period 10.000). Pin: master:4. Schematic sch p11: oscillator IC16
# ASEM1-100.000MHZ -> buffer IC15 -> net CLK_100MHZ -> FPGA N15.
set_property -dict {PACKAGE_PIN N15 IOSTANDARD LVCMOS33} [get_ports {CLK_100MHZ}]
create_clock -period 10.000 -name clk_100 [get_ports {CLK_100MHZ}]

# ---- Configuration bank voltage (master:7-8) and bitstream options
# (master:9, master:465-466).
set_property CFGBVS VCCO [current_design]
set_property CONFIG_VOLTAGE 3.3 [current_design]
set_property BITSTREAM.Config.SPI_buswidth 4 [current_design]
# UNUSEDPIN: PULLDOWN, deliberately NOT the master's PULLUP (master:465).
# Phase H H-1 (2026-10-24, build 52a66216): with PULLUP the unused RGB0/RGB1
# pins (C9 A9 A10 / A11 C10 B11, master:54-59) are pulled high and both RGB LEDs
# glow at full brightness. sch p4: each RGBn_x net drives the IN pin of a
# low-side switch (Q5-Q10, GND/IN/OUT) on a VCCp5V0 LED, so IN high = lit.
# Why PULLDOWN is safe (it is also the Vivado default for this property):
#  - Dedicated config/JTAG pins (CCLK, DONE, INIT_B, PROGRAM_B, M0-M2, TCK/TMS/
#    TDI/TDO, CFGBVS) are not user I/O, so UNUSEDPIN does not apply to them.
#  - The pull applies only after configuration (DONE high). During any later
#    configuration (power-on QSPI boot or JTAG reprogram) the pins are under
#    PUDC_B / config control, so this setting cannot affect booting.
#  - Multi-function bank-14 pins that become user I/O after config do get the
#    pull-down: QSPI FCS_B M13, D00-D03 K17/K18/L14/M15, QSPI_RESET_B N13,
#    PUDC_B L15. sch p11: QSPI_CS_B (R168, 4.7k), DQ2 (R165), DQ3 (R166) and
#    QSPI_RESET_B (R167, 4.7k) have external pull-ups to VCCp3V3 (same net as
#    the IC14 SN74AVC1T45 supply pins). A k-ohm external pull-up overrides the
#    FPGA's weak (tens of uA) pull-down, so the flash stays deselected and out
#    of reset. D00/D01 have no pull-up; they are don't-care while CS_B is high.
#  - Other unused peripherals only see a weak pull to 0: USB_RESET_B V13 low
#    holds the unused MAX3421E USB host in reset; DDR3 (bank 34) RESET_B/CKE low
#    keep the unused DDR3 in reset/power-down; SPKL/SPKR low = silent. No pin
#    in the master XDC is an active-low enable or a power-switch control
#    (checked by name; the schematic was read from extracted text, not drawn).
# Takes effect only in bitstreams built after this commit; every bitstream at
# 52a66216 (incl. KG1) still has PULLUP and still lights the RGB LEDs.
set_property BITSTREAM.CONFIG.UNUSEDPIN PULLDOWN [current_design]
set_property BITSTREAM.GENERAL.COMPRESS TRUE [current_design]

# ---- LEDs (master:30-45, "On-board LEDs"). ACTIVE HIGH: sch p4, each net LDn
# drives the anode through a 330R resistor (R41...), all cathodes go to GND.
set_property -dict {PACKAGE_PIN C13 IOSTANDARD LVCMOS33} [get_ports {LED[0]}]
set_property -dict {PACKAGE_PIN C14 IOSTANDARD LVCMOS33} [get_ports {LED[1]}]
set_property -dict {PACKAGE_PIN D14 IOSTANDARD LVCMOS33} [get_ports {LED[2]}]
set_property -dict {PACKAGE_PIN D15 IOSTANDARD LVCMOS33} [get_ports {LED[3]}]
set_property -dict {PACKAGE_PIN D16 IOSTANDARD LVCMOS33} [get_ports {LED[4]}]
set_property -dict {PACKAGE_PIN F18 IOSTANDARD LVCMOS33} [get_ports {LED[5]}]
set_property -dict {PACKAGE_PIN E17 IOSTANDARD LVCMOS33} [get_ports {LED[6]}]
set_property -dict {PACKAGE_PIN D17 IOSTANDARD LVCMOS33} [get_ports {LED[7]}]
set_property -dict {PACKAGE_PIN C17 IOSTANDARD LVCMOS33} [get_ports {LED[8]}]
set_property -dict {PACKAGE_PIN B18 IOSTANDARD LVCMOS33} [get_ports {LED[9]}]
set_property -dict {PACKAGE_PIN A17 IOSTANDARD LVCMOS33} [get_ports {LED[10]}]
set_property -dict {PACKAGE_PIN B17 IOSTANDARD LVCMOS33} [get_ports {LED[11]}]
set_property -dict {PACKAGE_PIN C18 IOSTANDARD LVCMOS33} [get_ports {LED[12]}]
set_property -dict {PACKAGE_PIN D18 IOSTANDARD LVCMOS33} [get_ports {LED[13]}]
set_property -dict {PACKAGE_PIN E18 IOSTANDARD LVCMOS33} [get_ports {LED[14]}]
set_property -dict {PACKAGE_PIN G17 IOSTANDARD LVCMOS33} [get_ports {LED[15]}]

# ---- Push buttons (master:48-51, "On-board Buttons"). ACTIVE HIGH (pressed = 1):
# sch p4, each button connects VCCp2V5 to a node pulled to GND by 10k (R60-R63),
# which reaches the FPGA net BTNn through a 10k series resistor (R43/R47/R52/R57).
# The bring-up top mirrors the debounced levels on LED8-11 (confirmed at H-1).
set_property -dict {PACKAGE_PIN J2 IOSTANDARD LVCMOS25} [get_ports {BTN[0]}]
set_property -dict {PACKAGE_PIN J1 IOSTANDARD LVCMOS25} [get_ports {BTN[1]}]
set_property -dict {PACKAGE_PIN G2 IOSTANDARD LVCMOS25} [get_ports {BTN[2]}]
set_property -dict {PACKAGE_PIN H2 IOSTANDARD LVCMOS25} [get_ports {BTN[3]}]

# ---- 7-segment display 0 (master:62-73). Anodes and segments ACTIVE LOW, as the
# master XDC comments say; sch p4 agrees: each AN net drives the base of a PNP
# (MBT3906) high-side switch through 1.5k, and each SEG net drives a cathode of
# the common-anode KW4-281ASB through 100R. SEG[0..7] = CA, CB, CC, CD, CE, CF,
# CG, DP (sch p4 net-to-pin check: SEG_0->pin11 CA ... SEG_7->pin3 DP).
# Which display is physically on the left, and which AN is the leftmost digit,
# can't be read from the schematic (layout); H-1 records it.
set_property -dict {PACKAGE_PIN G6 IOSTANDARD LVCMOS25} [get_ports {D0_AN[0]}]
set_property -dict {PACKAGE_PIN H6 IOSTANDARD LVCMOS25} [get_ports {D0_AN[1]}]
set_property -dict {PACKAGE_PIN C3 IOSTANDARD LVCMOS25} [get_ports {D0_AN[2]}]
set_property -dict {PACKAGE_PIN B3 IOSTANDARD LVCMOS25} [get_ports {D0_AN[3]}]
set_property -dict {PACKAGE_PIN E6 IOSTANDARD LVCMOS25} [get_ports {D0_SEG[0]}]
set_property -dict {PACKAGE_PIN B4 IOSTANDARD LVCMOS25} [get_ports {D0_SEG[1]}]
set_property -dict {PACKAGE_PIN D5 IOSTANDARD LVCMOS25} [get_ports {D0_SEG[2]}]
set_property -dict {PACKAGE_PIN C5 IOSTANDARD LVCMOS25} [get_ports {D0_SEG[3]}]
set_property -dict {PACKAGE_PIN D7 IOSTANDARD LVCMOS25} [get_ports {D0_SEG[4]}]
set_property -dict {PACKAGE_PIN D6 IOSTANDARD LVCMOS25} [get_ports {D0_SEG[5]}]
set_property -dict {PACKAGE_PIN C4 IOSTANDARD LVCMOS25} [get_ports {D0_SEG[6]}]
set_property -dict {PACKAGE_PIN B5 IOSTANDARD LVCMOS25} [get_ports {D0_SEG[7]}]

# ---- 7-segment display 1 (master:76-87). Same polarity and segment order.
set_property -dict {PACKAGE_PIN E4 IOSTANDARD LVCMOS25} [get_ports {D1_AN[0]}]
set_property -dict {PACKAGE_PIN E3 IOSTANDARD LVCMOS25} [get_ports {D1_AN[1]}]
set_property -dict {PACKAGE_PIN F5 IOSTANDARD LVCMOS25} [get_ports {D1_AN[2]}]
set_property -dict {PACKAGE_PIN H5 IOSTANDARD LVCMOS25} [get_ports {D1_AN[3]}]
set_property -dict {PACKAGE_PIN F3 IOSTANDARD LVCMOS25} [get_ports {D1_SEG[0]}]
set_property -dict {PACKAGE_PIN G5 IOSTANDARD LVCMOS25} [get_ports {D1_SEG[1]}]
set_property -dict {PACKAGE_PIN J3 IOSTANDARD LVCMOS25} [get_ports {D1_SEG[2]}]
set_property -dict {PACKAGE_PIN H4 IOSTANDARD LVCMOS25} [get_ports {D1_SEG[3]}]
set_property -dict {PACKAGE_PIN F4 IOSTANDARD LVCMOS25} [get_ports {D1_SEG[4]}]
set_property -dict {PACKAGE_PIN H3 IOSTANDARD LVCMOS25} [get_ports {D1_SEG[5]}]
set_property -dict {PACKAGE_PIN E5 IOSTANDARD LVCMOS25} [get_ports {D1_SEG[6]}]
set_property -dict {PACKAGE_PIN J4 IOSTANDARD LVCMOS25} [get_ports {D1_SEG[7]}]

# ---- USB-UART pins are NOT in this file. fpga/tcl/build.tcl reads exactly one of:
#   urbana_uart.xdc       (default) FPGA RX = B16, TX = A16, per the schematic
#   urbana_uart_swap.xdc  (UART_SWAP=1) FPGA RX = A16, TX = B16: was the fallback if H-2
#                         echoed nothing. NOT NEEDED: H-2 (2026-10-24) confirmed the default.

# No set_input_delay/set_output_delay: BTN and UART_RXD are asynchronous and
# 2-FF synchronized in RTL; LED/7-seg/UART_TXD are driven from flops and are
# timing-insensitive at human/115200-baud rates. No timing exceptions here.
