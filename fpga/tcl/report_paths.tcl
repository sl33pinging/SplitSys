# report_paths.tcl - post-route analysis of a build checkpoint (owner: fpga-integrator)
#   scripts/vivado -mode batch -source fpga/tcl/report_paths.tcl -tclargs <tag>
# Writes fpga/reports/<tag>/paths.rpt: the 10 worst setup paths (one per
# endpoint group) and the reset registers (*rst*_reg*, incl. Vivado replicas)
# with the fan-out of the net each one drives.
set ROOT [file normalize [file join [file dirname [info script]] .. ..]]
set tag [lindex $argv 0]
set_param board.repoPaths [list]
open_checkpoint [file join $ROOT fpga build $tag post_route.dcp]
set out [file join $ROOT fpga reports $tag paths.rpt]
report_timing -setup -max_paths 10 -nworst 1 -unique_pins -file $out
set fh [open $out a]
puts $fh "\n==== reset registers (name, driven-net fan-out) ===="
foreach c [lsort [get_cells -hierarchical -filter {NAME =~ *rst*_reg* && IS_SEQUENTIAL}]] {
    set n [get_nets -quiet -of_objects [get_pins -quiet $c/Q]]
    set fo [expr {[llength $n] ? [get_property FLAT_PIN_COUNT $n] - 1 : 0}]
    puts $fh [format "%-60s fanout %6d" $c $fo]
}
close $fh
puts "PATHS_OK $out"
exit 0
