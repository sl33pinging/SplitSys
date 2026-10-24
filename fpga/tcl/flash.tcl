# =============================================================================
# flash.tcl - write a bitstream to the Urbana's QSPI configuration flash
#             (owner: fpga-integrator)
# -----------------------------------------------------------------------------
# Usage (repo root; board attached to Windows, NOT to WSL via usbipd):
#   scripts/vivado -mode batch -source fpga/tcl/flash.tcl -tclargs <bit> [<cfgmem_part>] [<mode>]
#     <bit>          a .bit file, or a pointer file holding a repo-relative .bit
#                    path (e.g. fpga/known_good/CURRENT)
#     <cfgmem_part>  default mt25ql128-spi-x1_x2_x4 (see "Flash part" below)
#     <mode>         --image-only  write fpga/build/flash/<name>.{bin,prm} only;
#                                  never opens the hardware manager
#                    --program     (default) image + erase + blank-check +
#                                  program + verify, then boot_hw_device and
#                                  check DONE (= the FPGA booted from flash)
#                    --verify-only image + read the flash back against it; no
#                                  erase, no write
#                    --erase       erase the flash range only (failure branch)
# Image name: <bit dir name>.bin, e.g. fpga/build/top/top.bit -> top.bin,
# fpga/known_good/KG2/top.bit -> KG2.bin.
#
# Exit codes / last line:
#   0 FLASH_OK / FLASH_IMAGE_OK / FLASH_VERIFY_OK / FLASH_ERASE_OK
#   1 FLASH_FAIL: <step>: <reason>  (image, JTAG, cfgmem, erase/program/verify)
#   2 usage error
#   4 FLASH_BOOT_FAIL: the flash was written and verified, but after
#     boot_hw_device DONE stayed low. Most likely cause: mode jumper J14 is not
#     installed (the board strapped for JTAG mode). The FPGA is now unconfigured;
#     re-program over JTAG: scripts/vivado -mode batch -source
#     fpga/tcl/program.tcl -tclargs <bit>
#
# Flash part (from vendor documents, not memory):
#   Urbana schematic Urbana_V2I1_SCH_2023-01-03.PDF, sheet 11 ("FPGA BANK 0,
#   XADC and Clock"): IC10 = MT25QL128ABA8ESF (Micron MT25QL, 3 V, 128 Mbit =
#   16 MB), pins DQ0..DQ3/SCK/CS/RESET# on nets QSPI_DQ0..3, QSPI_SCK (CCLK_0
#   C8 via R174), QSPI_CS_B, QSPI_RESET_B. Sheet 12: QSPI_DQ0..3 = K17/K18/
#   L14/M15 (IO_L1P_T0_D00_MOSI_14 .. IO_L2N_T0_D03_14), QSPI_CS_B = M13
#   (IO_L6P_T0_FCS_B_14). Master XDC lines 143-149 agree (QSPI_RESET_B = N13).
#   Vivado 2025.2.1 get_cfgmem_parts *mt25ql128*: the SPI (not Zynq "qspi")
#   entry whose COMPATIBLE_PARTS includes spartan7 is mt25ql128-spi-x1_x2_x4
#   (MEM_MANUFACTURER Micron, MEM_DENSITY 128, MEM_TYPE spi).
# Bitstream: fpga/xdc/urbana_splitsys.xdc already sets SPI_buswidth 4,
#   CONFIG_VOLTAGE 3.3, CFGBVS VCCO, COMPRESS TRUE; CONFIGRATE is left at the
#   default. So the image uses -interface SPIx4 and the reviewed .bit is used
#   byte-for-byte unchanged (no rebuild).
# Mode pins (sheet 11 table "Configuration Mode M[2:0]: Master SPI 001,
#   JTAG 101 Default"): M0 pulled up (R169 1k), M1 to GND (R171 100R), M2 pulled
#   up (R170 1k) and wired to J14 pin 1; J14 pin 2 = GND. J14 jumper ON = 001 =
#   boot from QSPI at power-up; OFF = 101 = JTAG only. JTAG programming (this
#   script, program.tcl) works in either position.
# =============================================================================

proc fail {step msg} {
    puts "FLASH_FAIL: $step: $msg"
    catch {close_hw_manager}
    exit 1
}

set usage "usage: flash.tcl <bit> \[<cfgmem_part>\] \[--image-only|--program|--verify-only|--erase\]"
set mode program
set pos {}
foreach a $argv {
    switch -- $a {
        --image-only  { set mode image }
        --program     { set mode program }
        --verify-only { set mode verify }
        --erase       { set mode erase }
        default {
            if {[string match --* $a]} { puts $usage; exit 2 }
            lappend pos $a
        }
    }
}
if {[llength $pos] < 1 || [llength $pos] > 2} { puts $usage; exit 2 }

set ROOT [file normalize [file join [file dirname [info script]] .. ..]]
set bit [lindex $pos 0]
set part_name [expr {[llength $pos] == 2 ? [lindex $pos 1] : "mt25ql128-spi-x1_x2_x4"}]
set FLASH_MB 16   ;# MT25QL128 = 128 Mbit (schematic IC10; Vivado MEM_DENSITY 128)

if {[file pathtype $bit] ne "absolute"} { set bit [file join $ROOT $bit] }
if {[file isfile $bit] && [file extension $bit] ne ".bit"} {
    set fh [open $bit]; set rel [string trim [read $fh]]; close $fh
    set bit [file join $ROOT $rel]
}
if {![file isfile $bit]} { fail args "bitfile not found: $bit" }
set bit [file normalize $bit]

# Ignore the WSL board repository from the user's Vivado_init.tcl.
set_param board.repoPaths [list]

# ---- pick the cfgmem part (names repeat per family; keep a spartan7 entry)
set mem ""
foreach p [get_cfgmem_parts -quiet $part_name] {
    if {[lsearch -exact [get_property COMPATIBLE_PARTS $p] spartan7] >= 0} { set mem $p; break }
}
if {$mem eq ""} { fail cfgmem_part "no cfgmem part '$part_name' compatible with spartan7 (try: get_cfgmem_parts *mt25ql128*)" }
puts "FLASH: cfgmem part [get_property NAME $mem] ([get_property MEM_MANUFACTURER $mem], [get_property MEM_DENSITY $mem] Mbit, [get_property MEM_TYPE $mem] [get_property DATA_WIDTH $mem])"

# ---- md5 helper (tcllib md5 ships with Vivado)
package require md5
proc file_md5 {f} { return [string tolower [::md5::md5 -hex -file $f]] }

# ---- 1. flash image (always; a file only, harmless)
set name [file tail [file dirname $bit]]
set out_dir [file join $ROOT fpga build flash]
file mkdir $out_dir
set bin [file join $out_dir $name.bin]
set prm [file join $out_dir $name.prm]
puts "FLASH: bit $bit md5 [file_md5 $bit]"
if {[catch {
    write_cfgmem -force -format bin -interface SPIx4 -size $FLASH_MB \
        -loadbit [list up 0x00000000 $bit] -file $bin
} e]} { fail write_cfgmem $e }
if {![file isfile $bin] || ![file isfile $prm]} { fail write_cfgmem "no $bin / $prm written" }
set bin_md5 [file_md5 $bin]
set info [file join $out_dir $name.txt]
set fh [open $info w]
puts $fh "bit      [string map [list $ROOT/ {}] $bit]"
puts $fh "bit_md5  [file_md5 $bit]"
puts $fh "bin      [string map [list $ROOT/ {}] $bin]"
puts $fh "bin_md5  $bin_md5"
puts $fh "bin_size [file size $bin]"
puts $fh "cfgmem   [get_property NAME $mem]"
puts $fh "args     write_cfgmem -format bin -interface SPIx4 -size $FLASH_MB -loadbit {up 0x00000000 <bit>}"
puts $fh "date     [clock format [clock seconds] -format {%Y-%m-%dT%H:%M:%S%z}]"
close $fh
puts "FLASH: image $bin ([file size $bin] bytes) md5 $bin_md5"
if {$mode eq "image"} { puts "FLASH_IMAGE_OK: $bin"; exit 0 }

# ---- 2. JTAG
open_hw_manager
if {[catch {connect_hw_server -allow_non_jtag} e]} { fail connect_hw_server $e }
set targets [get_hw_targets -quiet]
if {[llength $targets] == 0} {
    fail open_hw_target "no JTAG target. Is the micro-USB plugged in, the board powered, and the device NOT attached to WSL (usbipd detach)?"
}
if {[llength $targets] > 1} { puts "FLASH: several targets, using the first: $targets" }
if {[catch {open_hw_target [lindex $targets 0]} e]} { fail open_hw_target $e }
set devs [get_hw_devices -quiet xc7s50*]
if {[llength $devs] != 1} { fail hw_device "expected one xc7s50 device, found: [get_hw_devices -quiet]" }
set dev [lindex $devs 0]
current_hw_device $dev
refresh_hw_device -update_hw_probes false $dev

# ---- 3. cfgmem object + operation flags
if {[catch {create_hw_cfgmem -hw_device $dev $mem} e]} { fail create_hw_cfgmem $e }
set cfg [get_property PROGRAM.HW_CFGMEM $dev]
set_property PROGRAM.ADDRESS_RANGE  {use_file} $cfg
set_property PROGRAM.FILES          [list $bin] $cfg
set_property PROGRAM.PRM_FILE       $prm $cfg
set_property PROGRAM.UNUSED_PIN_TERMINATION {pull-none} $cfg
set_property PROGRAM.CHECKSUM 0 $cfg
switch -- $mode {
    program { set flags {ERASE 1 BLANK_CHECK 1 CFG_PROGRAM 1 VERIFY 1} }
    verify  { set flags {ERASE 0 BLANK_CHECK 0 CFG_PROGRAM 0 VERIFY 1} }
    erase   { set flags {ERASE 1 BLANK_CHECK 1 CFG_PROGRAM 0 VERIFY 0} }
}
foreach {k v} $flags { set_property PROGRAM.$k $v $cfg }
puts "FLASH: mode $mode, operations: $flags"

# ---- 4. load Vivado's SPI-over-JTAG helper bitstream into the FPGA
#         (this replaces whatever design is running until the boot step)
if {[catch {
    create_hw_bitstream -hw_device $dev [get_property PROGRAM.HW_CFGMEM_BITFILE $dev]
    program_hw_devices $dev
    refresh_hw_device -update_hw_probes false $dev
} e]} { fail cfgmem_helper_bitstream $e }

# ---- 5. erase / blank-check / program / verify (Vivado runs them in that
#         order and errors out on the first failing step)
set t0 [clock seconds]
if {[catch {program_hw_cfgmem -hw_cfgmem $cfg} e]} { fail "program_hw_cfgmem ($mode)" $e }
puts "FLASH: program_hw_cfgmem ($mode) ok in [expr {[clock seconds] - $t0}] s"

# ---- 6. boot from flash and check DONE
if {$mode eq "program" || $mode eq "verify"} {
    if {[catch {boot_hw_device $dev} e]} { fail boot_hw_device $e }
    after 3000
    refresh_hw_device -update_hw_probes false $dev
    set done ""
    catch {set done [get_property REGISTER.CONFIG_STATUS.BIT14_DONE_PIN $dev]}
    puts "FLASH: after boot_hw_device DONE pin = '$done'"
    if {$done ne "1"} {
        puts "FLASH_BOOT_FAIL: flash $mode passed, but the FPGA did not configure from flash (DONE low)."
        puts "  Most likely: mode jumper J14 (next to the FPGA, sheet 11, on M2) is not installed."
        puts "  The FPGA is unconfigured now. Restore over JTAG:"
        puts "  scripts/vivado -mode batch -source fpga/tcl/program.tcl -tclargs [string map [list $ROOT/ {}] $bit]"
        catch {close_hw_manager}
        exit 4
    }
} else {
    puts "FLASH: erase only; the FPGA runs the SPI helper design. Re-program over JTAG:"
    puts "  scripts/vivado -mode batch -source fpga/tcl/program.tcl -tclargs [string map [list $ROOT/ {}] $bit]"
}

catch {close_hw_target}
catch {disconnect_hw_server}
close_hw_manager
switch -- $mode {
    program { puts "FLASH_OK: $bin (md5 $bin_md5) written, verified, FPGA booted from flash" }
    verify  { puts "FLASH_VERIFY_OK: flash matches $bin (md5 $bin_md5), FPGA booted from flash" }
    erase   { puts "FLASH_ERASE_OK" }
}
exit 0
