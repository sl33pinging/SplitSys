# =============================================================================
# urbana_uart.xdc - USB-UART pins (owner: fpga-integrator)
# Read by fpga/tcl/build.tcl together with urbana_splitsys.xdc; exactly one of
# urbana_uart.xdc / urbana_uart_swap.xdc is used (build option UART_SWAP).
# Port names are from the FPGA's side: UART_RXD = FPGA input, UART_TXD = output.
# =============================================================================
# DEFAULT (UART_SWAP=0).
# ---- USB-UART (master:140-141, "UART signals"; the BLE_UART_* pins on
# master:136-139 are the Bluetooth module and are not used).
# The master XDC names are from the USB BRIDGE's point of view, not the FPGA's:
#   sch p12: FPGA B16 = net UART_TXD, FPGA A16 = net UART_RXD.
#   sch p10: net UART_TXD = FT2232HQ (IC6) pin 38 = BDBUS0; net UART_RXD = pin 39
#            = BDBUS1. Channel B is the UART (channel A pins 16-19 are JTAG
#            FT_TCK/TDI/TDO/TMS). In FT2232H async-UART mode BDBUS0 is TXD (FTDI
#            output) and BDBUS1 is RXD (FTDI input) -- FTDI DS_FT2232H pin table.
# So the FPGA RECEIVES on B16 and TRANSMITS on A16. Our ports are named from
# the FPGA's side (UART_RXD = FPGA input), so these two lines intentionally
# differ from master:140-141:
#   master "UART_TXD" (B16)  ->  our port UART_RXD (FPGA input)
#   master "UART_RXD" (A16)  ->  our port UART_TXD (FPGA output)
set_property -dict {PACKAGE_PIN B16 IOSTANDARD LVCMOS33} [get_ports {UART_RXD}]
set_property -dict {PACKAGE_PIN A16 IOSTANDARD LVCMOS33} [get_ports {UART_TXD}]
