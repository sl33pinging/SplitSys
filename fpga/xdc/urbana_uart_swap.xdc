# =============================================================================
# urbana_uart_swap.xdc - USB-UART pins (owner: fpga-integrator)
# Read by fpga/tcl/build.tcl together with urbana_splitsys.xdc; exactly one of
# urbana_uart.xdc / urbana_uart_swap.xdc is used (build option UART_SWAP).
# Port names are from the FPGA's side: UART_RXD = FPGA input, UART_TXD = output.
# =============================================================================
# FALLBACK (UART_SWAP=1): the master-XDC naming taken from the FPGA's side,
# i.e. the opposite of the schematic reading in urbana_uart.xdc. Use only if
# H-2 echoes 0 bytes with the default bitstream (docs/HW_CHECKLIST.md H-2).
#   master:140 "UART_TXD" B16 -> our port UART_TXD (FPGA output)
#   master:141 "UART_RXD" A16 -> our port UART_RXD (FPGA input)
set_property -dict {PACKAGE_PIN A16 IOSTANDARD LVCMOS33} [get_ports {UART_RXD}]
set_property -dict {PACKAGE_PIN B16 IOSTANDARD LVCMOS33} [get_ports {UART_TXD}]
