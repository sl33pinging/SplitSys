# =============================================================================
# build.tcl - SplitSys non-project Vivado build (owner: fpga-integrator)
# -----------------------------------------------------------------------------
# Usage (repo root, Git Bash or WSL):
#   scripts/vivado -mode batch -source fpga/tcl/build.tcl -tclargs <tag> [top] [NAME=VALUE ...]
# Examples:
#   ... -tclargs heartbeat_uart_echo                 (top = heartbeat_uart_echo)
#   ... -tclargs top                                 (full design, ENABLE_STRIP=1)
#   ... -tclargs top_nostrip top ENABLE_STRIP=0      (cost build)
#
# Arguments
#   tag    report/build directory name: fpga/reports/<tag>/, fpga/build/<tag>/
#   top    top module. Default: <tag> if fpga/bringup/<tag>/ exists, else "top".
#   NAME=VALUE  extra top-level generics (Verilog literal syntax, e.g.
#          ENABLE_STRIP=0, FOO=32'h1). BUILD_ID is always set from git and may
#          not be overridden. Any *DIV_OVERRIDE* generic other than 0 is refused
#          (it is a simulation-only UART speed-up).
#          Full top only: ENABLE_STRIP defaults to 1, and ROM_FILE (the self-test
#          ROM image for $readmemh) defaults to sw/crates/vec/out/selftest_rom.mem;
#          a repo-relative ROM_FILE=<path> may be given. It is passed to synthesis
#          as an absolute path, must exist, counts toward git_dirty, and a
#          $readmemh file that cannot be opened (Synth 8-4445) is an error.
#   UART_SWAP=1  build option, not a generic: use fpga/xdc/urbana_uart_swap.xdc
#          (FPGA RX = A16, TX = B16) instead of fpga/xdc/urbana_uart.xdc (RX =
#          B16, TX = A16, the schematic reading). Fallback for H-2; give the
#          build its own tag, e.g. ... -tclargs top_uartswap top UART_SWAP=1
#
# Sources
#   Bring-up top (fpga/bringup/<top>/ exists): the repo-relative files listed in
#   fpga/bringup/<top>/sources.f, then fpga/bringup/<top>/*.sv (not sim/).
#   Otherwise: rtl/gen/*.sv then rtl/*.sv.
#   Constraints: fpga/bringup/<top>/*.xdc if any (UART_SWAP not supported then),
#   else fpga/xdc/urbana_splitsys.xdc + fpga/xdc/urbana_uart[_swap].xdc.
#
# Outputs
#   fpga/reports/<tag>/ utilization.rpt timing_summary.rpt drc.rpt power.rpt
#                       io.rpt post_synth_utilization.rpt summary.json (one line)
#   fpga/build/<tag>/   top.bit, post_route.dcp      (gitignored)
#
# Exit status: 0 only if WNS >= 0, WHS >= 0, no DRC errors, no latches, and
# (full top only: top == "top") DSP >= 64. summary.json is written either way,
# with "pass" and "failures". The bitstream is written only on pass.
# =============================================================================

set PART      xc7s50csga324-2
set CLK_NAME  clk_100
set MIN_DSP_FULL 64

set ROOT [file normalize [file join [file dirname [info script]] .. ..]]
cd $ROOT   ;# relative paths inside RTL (e.g. $readmemh defaults) resolve from the repo root
set DEFAULT_ROM_FILE sw/crates/vec/out/selftest_rom.mem

proc die {msg} { puts "BUILD_ERROR: $msg"; exit 2 }

# ------------------------------------------------------------------ arguments
if {[llength $argv] < 1} {
    die "usage: build.tcl <tag> \[top\] \[NAME=VALUE ...\]"
}
set tag [lindex $argv 0]
if {![regexp {^[A-Za-z0-9_.-]+$} $tag]} { die "bad tag '$tag'" }
set top ""
set user_generics [list]
# vivado.bat goes through cmd.exe, which treats '=' as an argument separator:
# "ENABLE_STRIP=0" arrives as two arguments "ENABLE_STRIP" "0". So an
# all-uppercase identifier (generic names are uppercase; module names are not)
# takes the next argument as its value. "NAME=VALUE" in one argument also works.
set rest [lrange $argv 1 end]
for {set i 0} {$i < [llength $rest]} {incr i} {
    set a [lindex $rest $i]
    if {[regexp {^([A-Za-z_][A-Za-z0-9_]*)=(.+)$} $a -> k v]} {
        lappend user_generics $k $v
    } elseif {[regexp {^[A-Z][A-Z0-9_]*$} $a]} {
        incr i
        if {$i >= [llength $rest]} { die "generic $a has no value" }
        lappend user_generics $a [lindex $rest $i]
    } elseif {$top eq ""} {
        set top $a
    } else {
        die "unexpected argument '$a'"
    }
}
set bringup_dir [file join $ROOT fpga bringup]
if {$top eq ""} {
    if {[file isdirectory [file join $bringup_dir $tag]]} { set top $tag } else { set top top }
}
set is_full_top [expr {$top eq "top"}]
set min_dsp [expr {$is_full_top ? $MIN_DSP_FULL : 0}]

# ------------------------------------------------------------------- build id
proc git_short_hash {root} {
    if {![catch {exec git -C $root rev-parse --short=8 HEAD} h]} {
        set h [string trim $h]
        if {[regexp {^[0-9a-f]{8}$} $h]} { return $h }
    }
    # Fallback: read .git directly (git not on the Windows PATH).
    set gd [file join $root .git]
    set fh [open [file join $gd HEAD]]; set head [string trim [read $fh]]; close $fh
    if {[regexp {^ref: (.+)$} $head -> ref]} {
        set rf [file join $gd $ref]
        if {[file exists $rf]} {
            set fh [open $rf]; set head [string trim [read $fh]]; close $fh
        } else {
            set fh [open [file join $gd packed-refs]]; set pr [read $fh]; close $fh
            if {![regexp -line "^(\[0-9a-f\]{40}) $ref\$" $pr -> head]} {
                die "cannot resolve $ref"
            }
        }
    }
    return [string range $head 0 7]
}
# "true" if any file this build reads (sources, XDC, this script) differs from
# HEAD or is untracked, i.e. BUILD_ID does not fully identify the bitstream.
proc git_dirty {root files} {
    if {[catch {exec git -C $root status --porcelain --untracked-files=all -- {*}$files} s]} {
        return "unknown"
    }
    return [expr {[string trim $s] eq "" ? "false" : "true"}]
}
set git_hash [git_short_hash $ROOT]
set build_id_hex [string toupper $git_hash]

# ------------------------------------------------------------------- generics
set generics [list BUILD_ID "32'h$git_hash"]
set gen_dict [dict create BUILD_ID "32'h$git_hash"]
set uart_swap 0
if {[dict exists $user_generics UART_SWAP]} {
    set uart_swap [dict get $user_generics UART_SWAP]
    set user_generics [dict remove $user_generics UART_SWAP]
    if {$uart_swap ni {0 1}} { die "UART_SWAP must be 0 or 1" }
}
set rom_file ""
if {$is_full_top} {
    if {![dict exists $user_generics ENABLE_STRIP]} { lappend user_generics ENABLE_STRIP 1 }
    if {[dict exists $user_generics ROM_FILE]} {
        set rom_rel [string trim [dict get $user_generics ROM_FILE] "\""]
        set user_generics [dict remove $user_generics ROM_FILE]
    } else {
        set rom_rel $DEFAULT_ROM_FILE
    }
    set rom_file [file normalize [file join $ROOT $rom_rel]]
    if {![file isfile $rom_file]} { die "self-test ROM image not found: $rom_file" }
    lappend user_generics ROM_FILE "\"$rom_file\""
}
foreach {k v} $user_generics {
    if {$k eq "BUILD_ID"} { die "BUILD_ID comes from git and cannot be overridden" }
    if {[string match -nocase *DIV_OVERRIDE* $k]} {
        if {![regexp {^(0+|[0-9]+'[dhb]0+)$} $v]} {
            die "$k=$v refused: DIV_OVERRIDE is simulation-only and must be 0 in hardware builds"
        }
    }
    lappend generics $k $v
    dict set gen_dict $k $v
}
set enable_strip [expr {[dict exists $gen_dict ENABLE_STRIP] ? [dict get $gen_dict ENABLE_STRIP] : "null"}]

# -------------------------------------------------------------------- sources
set srcs [list]
set top_bringup [file join $bringup_dir $top]
if {[file isdirectory $top_bringup]} {
    set sf [file join $top_bringup sources.f]
    if {[file exists $sf]} {
        set fh [open $sf]; set lines [split [read $fh] "\n"]; close $fh
        foreach l $lines {
            set l [string trim [lindex [split $l #] 0]]
            if {$l ne ""} { lappend srcs [file join $ROOT $l] }
        }
    }
    foreach f [lsort [glob -nocomplain -directory $top_bringup *.sv]] { lappend srcs $f }
    set xdcs [lsort [glob -nocomplain -directory $top_bringup *.xdc]]
} else {
    foreach f [lsort [glob -nocomplain -directory [file join $ROOT rtl gen] *.sv]] { lappend srcs $f }
    foreach f [lsort [glob -nocomplain -directory [file join $ROOT rtl] *.sv]]     { lappend srcs $f }
    set xdcs [list]
}
if {[llength $xdcs] == 0} {
    set uart_xdc [expr {$uart_swap ? "urbana_uart_swap.xdc" : "urbana_uart.xdc"}]
    set xdcs [list [file join $ROOT fpga xdc urbana_splitsys.xdc] [file join $ROOT fpga xdc $uart_xdc]]
} elseif {$uart_swap} {
    die "UART_SWAP=1 is not supported for a top with its own XDC in $top_bringup"
}
foreach f [concat $srcs $xdcs] { if {![file exists $f]} { die "missing source $f" } }
set dirty_files [concat $srcs $xdcs [list [file normalize [info script]]]]
if {$rom_file ne ""} { lappend dirty_files $rom_file }
set dirty [git_dirty $ROOT $dirty_files]

set rpt_dir [file join $ROOT fpga reports $tag]
set out_dir [file join $ROOT fpga build $tag]
file mkdir $rpt_dir $out_dir

puts "BUILD: tag=$tag top=$top part=$PART build_id=0x$build_id_hex dirty=$dirty"
puts "BUILD: generics=$generics"
puts "BUILD: sources=[llength $srcs] xdc=$xdcs"

# ---------------------------------------------------------------------- flow
# The user's Vivado_init.tcl points board.repoPaths at a WSL share whose
# board.xml files fail to parse (78 CRITICAL WARNING [Board 49-4] at set_part).
# This flow uses only the part number, never a board file, so clear it for
# this session (the Windows-side init file is not touched).
set_param board.repoPaths [list]
set_part $PART
foreach f $srcs { read_verilog -sv $f }
foreach f $xdcs { read_xdc $f }

set gen_args [list]
foreach {k v} $generics { lappend gen_args -generic "$k=$v" }
# A $readmemh image that cannot be opened must fail the build, not give a zero ROM.
set_msg_config -id {Synth 8-4445} -new_severity ERROR
synth_design -top $top -part $PART {*}$gen_args

# latch check right after synthesis (the netlist primitives are LDCE/LDPE)
set latches [get_cells -hierarchical -quiet -filter {REF_NAME =~ LD*}]
set n_latch [llength $latches]
report_utilization -file [file join $rpt_dir post_synth_utilization.rpt]

opt_design
place_design
phys_opt_design
route_design

report_utilization      -file [file join $rpt_dir utilization.rpt]
report_timing_summary   -max_paths 10 -report_unconstrained -file [file join $rpt_dir timing_summary.rpt]
report_drc              -file [file join $rpt_dir drc.rpt]
report_power            -file [file join $rpt_dir power.rpt]
report_io               -file [file join $rpt_dir io.rpt]
write_checkpoint -force [file join $out_dir post_route.dcp]

# ------------------------------------------------------------------- metrics
proc slack_of {args} {
    set p [get_timing_paths -quiet {*}$args -max_paths 1 -nworst 1]
    if {[llength $p] == 0} { return "" }
    return [get_property SLACK $p]
}
set wns [slack_of -setup]
set whs [slack_of -hold]

set util [report_utilization -return_string]
proc util_num {txt label} {
    if {[regexp -line "^\\|\\s*$label\\s*\\|\\s*(\[0-9.\]+)\\s*\\|" $txt -> n]} { return $n }
    return -1
}
set lut  [util_num $util {Slice LUTs\*?}]
set ff   [util_num $util {Slice Registers}]
set dsp  [util_num $util {DSPs}]
set bram [util_num $util {Block RAM Tile}]

set clk_obj [get_clocks -quiet $CLK_NAME]
if {[llength $clk_obj] == 1 && $wns ne ""} {
    set period [get_property PERIOD $clk_obj]
    set fmax [format %.1f [expr {1000.0 / ($period - $wns)}]]
} else {
    set period "null"; set fmax "null"
}

set n_drc_err 0
foreach v [get_drc_violations -quiet] {
    if {[get_property SEVERITY $v] eq "Error"} { incr n_drc_err }
}

# --------------------------------------------------------------------- checks
set failures [list]
if {[llength $clk_obj] != 1}            { lappend failures "clock $CLK_NAME not found" }
if {$wns eq ""}                         { lappend failures "no constrained setup paths" } \
elseif {$wns < 0}                       { lappend failures "WNS $wns < 0" }
if {$whs ne "" && $whs < 0}             { lappend failures "WHS $whs < 0" }
if {$n_drc_err > 0}                     { lappend failures "$n_drc_err DRC errors" }
if {$n_latch > 0}                       { lappend failures "$n_latch latches: [lrange $latches 0 9]" }
if {$dsp < $min_dsp}                    { lappend failures "DSP $dsp < $min_dsp" }
if {$lut < 0 || $ff < 0 || $dsp < 0 || $bram < 0} { lappend failures "could not parse utilization" }
set pass [expr {[llength $failures] == 0}]

if {$pass} {
    write_bitstream -force [file join $out_dir top.bit]
    set bitfile "fpga/build/$tag/top.bit"
} else {
    set bitfile ""
}

# --------------------------------------------------------------- summary.json
proc jstr {s} { return "\"[string map {\\ \\\\ \" \\\"} $s]\"" }
proc jnum {v} { return [expr {$v eq "" ? "null" : $v}] }
set gj [list]
dict for {k v} $gen_dict { lappend gj "[jstr $k]:[jstr $v]" }
set fj [list]
foreach f $failures { lappend fj [jstr $f] }
set es [expr {$enable_strip eq "null" ? "null" : [jstr $enable_strip]}]
set json "\{\"tag\":[jstr $tag],\"top\":[jstr $top],\"part\":[jstr $PART],\"build_id\":[jstr 0x$build_id_hex],\"git_hash\":[jstr $git_hash],\"git_dirty\":[jstr $dirty],\"vivado\":[jstr [version -short]],\"clk_period_ns\":[jnum $period],\"wns_ns\":[jnum $wns],\"whs_ns\":[jnum $whs],\"lut\":$lut,\"ff\":$ff,\"dsp\":$dsp,\"bram\":$bram,\"fmax_est_mhz\":[jnum $fmax],\"enable_strip\":$es,\"uart_swap\":$uart_swap,\"rom_file\":[jstr [expr {$rom_file eq "" ? "" : [string map [list "$ROOT/" ""] $rom_file]}]],\"min_dsp\":$min_dsp,\"latches\":$n_latch,\"drc_errors\":$n_drc_err,\"generics\":\{[join $gj ,]\},\"bitfile\":[jstr $bitfile],\"pass\":[expr {$pass ? "true" : "false"}],\"failures\":\[[join $fj ,]\]\}"
set fh [open [file join $rpt_dir summary.json] w]
puts $fh $json
close $fh
puts "SUMMARY: $json"

if {!$pass} {
    puts "BUILD_FAIL: [join $failures {; }]"
    exit 1
}
puts "BUILD_PASS: $bitfile"
exit 0
