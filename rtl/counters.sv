// =============================================================================
// counters.sv — 48-bit performance counters (SPEC §3.7)
// -----------------------------------------------------------------------------
// Counter index = READ_COUNTERS order from spec.toml (CTR_* in splitsys_pkg):
//   0 cycles_total   1 cycles_setup   2 cycles_stream   3 cycles_flush
//   4 cycles_drain   5 cycles_stall   6 mac_useful      7..10 mac_useful_q0..q3
//   11 tiles
//
// Ports (all synchronous to clk)
//   start           1-cycle pulse on RUN_GEMM descriptor accept. Clears every
//                   counter and the add pipeline, and sets `running` from the
//                   next cycle. Taps presented in the start cycle are NOT counted.
//   done            1-cycle pulse from the sequencer when it reaches DONE. Taps
//                   presented in the done cycle ARE counted; `running` drops
//                   after it, so every counter freezes.
//   tap_setup       FSM is in TILE_SETUP this cycle            -> cycles_setup
//   tap_stream      FSM is in STREAM                            -> cycles_stream
//   tap_flush       FSM is in FLUSH                             -> cycles_flush
//   tap_drain       FSM is in DRAIN or WRITEBACK                -> cycles_drain
//   tap_stall       array waiting on buffer/output backpressure -> cycles_stall
//   mac_cnt[6:0]    popcount of in-bounds MACs this cycle, 0..64 -> mac_useful
//   mac_q_cnt       4 x 5-bit popcounts, quadrant q at [5*q +: 5], 0..16 each
//                                                               -> mac_useful_q[q]
//   tile_done       1-cycle pulse per completed tile            -> tiles
//   (cycles_total increments on every cycle with `running` = 1.)
//
//   So cycles_total counts the cycles start+1 .. done inclusive. The sequencer
//   must make cycles_total == setup+stream+flush+drain+stall + L_ACCEPT + L_DONE
//   by asserting exactly one cycle tap in every running cycle except L_ACCEPT
//   cycles right after start and L_DONE cycles ending with the done cycle.
//
//   snap            1-cycle pulse: copy all live counters into the snapshot bank
//                   (atomic; READ_COUNTERS sees one consistent instant).
//   rd_idx          counter index to read from the snapshot bank
//   rd_data[47:0]   snapshot[rd_idx], registered: valid 1 cycle after rd_idx
//                   (read latency 1, <= L_RD as packet.sv requires).
//   running         high from start+1 through done (inclusive)
//
// Latency / invariants
//   Taps of cycle t are registered at t+1 and added into the counter at t+2.
//   After `done` at cycle t the counters are final from cycle t+2 on.
//   A snap taken while running returns an exact prefix of the run.
//   48-bit counters never wrap within any realistic run (2^48 cycles ~ 32 days
//   at 100 MHz).
//
// Critical path: increment register (7 bits) -> 48-bit adder (12 CARRY4) ->
//   counter register. About 2 ns on -2 Spartan-7; registered on both sides.
// =============================================================================
module counters
  import splitsys_pkg::*;
(
  input  logic                        clk,
  input  logic                        rst,
  input  logic                        start,
  input  logic                        done,
  input  logic                        tap_setup,
  input  logic                        tap_stream,
  input  logic                        tap_flush,
  input  logic                        tap_drain,
  input  logic                        tap_stall,
  input  logic [$clog2(N_PES+1)-1:0]   mac_cnt,
  input  logic [N_QUADS*$clog2(N_PES/N_QUADS+1)-1:0] mac_q_cnt,
  input  logic                        tile_done,
  input  logic                        snap,
  input  logic [$clog2(N_COUNTERS)-1:0] rd_idx,
  output logic [COUNTER_BITS-1:0]     rd_data,
  output logic                        running
);

  localparam int unsigned MAC_W  = $clog2(N_PES + 1);            // 7: 0..64
  localparam int unsigned MACQ_W = $clog2(N_PES / N_QUADS + 1);  // 5: 0..16
  localparam int unsigned IDX_W  = $clog2(N_COUNTERS);
  localparam int unsigned INC_W  = MAC_W;                         // widest increment

  // ------------------------------------------------------------- run window
  always_ff @(posedge clk) begin
    if (rst)        running <= 1'b0;
    else if (start) running <= 1'b1;
    else if (done)  running <= 1'b0;
  end

  // ------------------------------------------- stage 1: registered increments
  logic [INC_W-1:0] inc_q [N_COUNTERS];

  always_ff @(posedge clk) begin
    if (rst || start || !running) begin
      for (int i = 0; i < N_COUNTERS; i++) inc_q[i] <= '0;
    end else begin
      inc_q[CTR_CYCLES_TOTAL]  <= INC_W'(1);
      inc_q[CTR_CYCLES_SETUP]  <= INC_W'(tap_setup);
      inc_q[CTR_CYCLES_STREAM] <= INC_W'(tap_stream);
      inc_q[CTR_CYCLES_FLUSH]  <= INC_W'(tap_flush);
      inc_q[CTR_CYCLES_DRAIN]  <= INC_W'(tap_drain);
      inc_q[CTR_CYCLES_STALL]  <= INC_W'(tap_stall);
      inc_q[CTR_MAC_USEFUL]    <= mac_cnt;
      inc_q[CTR_MAC_USEFUL_Q0] <= INC_W'(mac_q_cnt[0*MACQ_W +: MACQ_W]);
      inc_q[CTR_MAC_USEFUL_Q1] <= INC_W'(mac_q_cnt[1*MACQ_W +: MACQ_W]);
      inc_q[CTR_MAC_USEFUL_Q2] <= INC_W'(mac_q_cnt[2*MACQ_W +: MACQ_W]);
      inc_q[CTR_MAC_USEFUL_Q3] <= INC_W'(mac_q_cnt[3*MACQ_W +: MACQ_W]);
      inc_q[CTR_TILES]         <= INC_W'(tile_done);
    end
  end

  // --------------------------------------------------- stage 2: 48-bit adders
  logic [COUNTER_BITS-1:0] cnt [N_COUNTERS];

  always_ff @(posedge clk) begin
    for (int i = 0; i < N_COUNTERS; i++) begin
      if (rst || start) cnt[i] <= '0;
      else              cnt[i] <= cnt[i] + COUNTER_BITS'(inc_q[i]);
    end
  end

  // ------------------------------------------------------- snapshot + read
  logic [COUNTER_BITS-1:0] snap_q [N_COUNTERS];

  always_ff @(posedge clk) begin
    for (int i = 0; i < N_COUNTERS; i++) begin
      if (rst)       snap_q[i] <= '0;
      else if (snap) snap_q[i] <= cnt[i];
    end
  end

  always_ff @(posedge clk) begin
    if (rst)                          rd_data <= '0;
    else if (rd_idx < IDX_W'(N_COUNTERS)) rd_data <= snap_q[rd_idx];
    else                              rd_data <= '0;
  end

endmodule
