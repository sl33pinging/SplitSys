# Vivado batch smoke test (toolchain-engineer): scripts/vivado -mode batch -source scripts/vivado_smoke.tcl
puts "SMOKE_VERSION: [version -short]"
puts "SMOKE_PARTS: [lsort [get_parts xc7s50*]]"
if {[llength [get_parts xc7s50csga324-2]] == 1} { puts "SMOKE_OK xc7s50csga324-2" ; exit 0 } else { puts "SMOKE_FAIL" ; exit 1 }
