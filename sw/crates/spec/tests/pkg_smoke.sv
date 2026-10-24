// Smoke top for compiling rtl/gen/splitsys_pkg.sv standalone (owner: spec-steward).
// iverilog -g2012 -o /tmp/pkg.vvp rtl/gen/splitsys_pkg.sv sw/crates/spec/tests/pkg_smoke.sv && vvp /tmp/pkg.vvp
module pkg_smoke;
  import splitsys_pkg::*;
  opcode_e op;
  status_e st;
  mode_e md;
  initial begin
    op = OP_RUN_GEMM;
    st = ST_NACK_RANGE;
    md = MODE_STRIP;
    if (op != 8'h20 || st != 8'h03 || md != 8'h01) $fatal(1, "enum encodings");
    if (ROWS * COLS != N_PES || COLS % QUAD != 0) $fatal(1, "array dims");
    if (int'(W_SKEW_BASE_STRIP[8*8 +: 8]) != STRIP_W_SKEW_BASE || W_SKEW_BASE_STRIP[8*7 +: 8] != 8'd0)
      $fatal(1, "strip skew base");
    if (WBUF_WORD_BITS != 8 * W_LANES || (1 << OBUF_AW) != OBUF_WORDS) $fatal(1, "buffers");
    if (CTR_TILES != N_COUNTERS - 1 || COUNTER_BITS != 48) $fatal(1, "counters");
    if (PING_MAGIC_LE[7:0] != "S" || PING_MAGIC_LE[31:24] != "T") $fatal(1, "ping magic");
    $display("PKG_SMOKE_OK");
    $finish;
  end
endmodule
