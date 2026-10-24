# =============================================================================
# program.tcl - program the Urbana's XC7S50 over JTAG (owner: fpga-integrator)
# -----------------------------------------------------------------------------
# Usage (Phase H only, board attached to Windows, NOT to WSL via usbipd):
#   scripts/vivado -mode batch -source fpga/tcl/program.tcl -tclargs <bitfile>
#   e.g. ... -tclargs fpga/build/heartbeat_uart_echo/top.bit
#        ... -tclargs fpga/known_good/CURRENT      (a file containing a .bit path)
# Volatile programming only (SRAM); nothing is written to the QSPI flash.
# Prints PROGRAM_OK and exits 0 if DONE is high afterwards; exits non-zero
# otherwise (PROGRAM_FAIL: <reason>).
# =============================================================================

proc fail {msg} { puts "PROGRAM_FAIL: $msg"; catch {close_hw_manager}; exit 1 }

if {[llength $argv] != 1} { puts "usage: program.tcl <bitfile>"; exit 2 }
set ROOT [file normalize [file join [file dirname [info script]] .. ..]]
set bit [lindex $argv 0]
if {[file pathtype $bit] ne "absolute"} { set bit [file join $ROOT $bit] }
# A pointer file (e.g. fpga/known_good/CURRENT) holds a repo-relative .bit path.
if {[file isfile $bit] && [file extension $bit] ne ".bit"} {
    set fh [open $bit]; set rel [string trim [read $fh]]; close $fh
    set bit [file join $ROOT $rel]
}
if {![file isfile $bit]} { fail "bitfile not found: $bit" }
puts "PROGRAM: $bit"

# Ignore the WSL board repository set by the user's Vivado_init.tcl (its
# board.xml files fail to parse); programming needs no board files.
set_param board.repoPaths [list]

open_hw_manager
if {[catch {connect_hw_server -allow_non_jtag} e]} { fail "connect_hw_server: $e" }
set targets [get_hw_targets -quiet]
if {[llength $targets] == 0} {
    fail "no JTAG target. Is the micro-USB plugged in, the board powered, and the device NOT attached to WSL (usbipd detach)?"
}
if {[llength $targets] > 1} { puts "PROGRAM: several targets, using the first: $targets" }
if {[catch {open_hw_target [lindex $targets 0]} e]} { fail "open_hw_target: $e" }

set devs [get_hw_devices -quiet xc7s50*]
if {[llength $devs] != 1} { fail "expected one xc7s50 device, found: [get_hw_devices -quiet]" }
set dev [lindex $devs 0]
current_hw_device $dev
refresh_hw_device -update_hw_probes false $dev

set_property PROGRAM.FILE $bit $dev
set_property PROBES.FILE {} $dev
if {[catch {program_hw_devices $dev} e]} { fail "program_hw_devices: $e" }
refresh_hw_device -update_hw_probes false $dev

set done ""
catch {set done [get_property REGISTER.CONFIG_STATUS.BIT14_DONE_PIN $dev]}
puts "PROGRAM: DONE pin = '$done'"
if {$done ne "1"} { fail "DONE pin not high after programming" }

close_hw_target
disconnect_hw_server
close_hw_manager
puts "PROGRAM_OK: $bit"
exit 0
