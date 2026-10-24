# report_hier.tcl - hierarchical utilization from an existing build checkpoint
# (owner: fpga-integrator). Same report build.tcl writes for new builds.
#   scripts/vivado -mode batch -source fpga/tcl/report_hier.tcl -tclargs <tag>
# Reads fpga/build/<tag>/post_route.dcp, writes fpga/reports/<tag>/utilization_hier.rpt.
set ROOT [file normalize [file join [file dirname [info script]] .. ..]]
set tag [lindex $argv 0]
set_param board.repoPaths [list]
open_checkpoint [file join $ROOT fpga build $tag post_route.dcp]
report_utilization -hierarchical -hierarchical_depth 4 -file [file join $ROOT fpga reports $tag utilization_hier.rpt]
exit 0
