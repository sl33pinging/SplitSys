// =============================================================================
// array.sv -- 8x8 output-stationary systolic array, FULL / STRIP (SPEC sec. 3.2)
// -----------------------------------------------------------------------------
// Contains: mode register, A-source mux (rows 4-7), A skew, W skew, the 8x8 PE
// grid with the row-4 W boundary mux, the drain read mux, the in-bounds mask
// and the per-quadrant useful-MAC popcounts.
//
//            a_in[r] -> A-src mux -> A skew (D = r) -> PE(r,0) -> ... -> PE(r,7)
//            w_in[c], c<8  -> W skew (D = c)   -> top of column c
//            w_in[8+c]     -> W skew (D = 4+c) -> row-4 boundary mux (STRIP)
//
// Quadrants: Q0 rows 0-3 x cols 0-3, Q1 rows 0-3 x cols 4-7,
//            Q2 rows 4-7 x cols 0-3, Q3 rows 4-7 x cols 4-7.
//
// Parameters
//   ENABLE_STRIP  1: STRIP supported. 0: every STRIP mux, the W lanes 8-15 skew
//                 and the lower-half drain lanes are compiled out (area-cost
//                 build); mode_strip, w_in[15:8], col_inb[15:8] are ignored.
//
// Sequencer-facing interface (all inputs sampled on posedge clk)
//   mode_strip      0 = FULL (8x8 tile), 1 = STRIP (4x16 tile). Registered
//                   internally (strip_q). Change it only while the array is idle
//                   (no MAC in flight) and >= 2 cycles before the tile's first
//                   a_vld; hold it until the tile's drain is complete.
//   a_in[r]         int8 activation for tile row r (packed lane r = a_in[r]),
//                   UNskewed: the k-th word of the tile is presented in one
//                   cycle on all lanes. FULL: lanes 0-7 = tile rows 0-7.
//                   STRIP: lanes 0-3 = tile rows 0-3 (lanes 4-7 ignored; the
//                   A-source mux copies lanes 0-3 into rows 4-7).
//   a_vld           the word on a_in/w_in is the k-th operand pair of the tile
//   a_first         the word is k = 0 (restarts every accumulator it reaches)
//   w_in[c]         int8 weight for tile column c, unskewed, same cycle as a_in.
//                   FULL: lanes 0-7 = tile cols 0-7 (lanes 8-15 ignored).
//                   STRIP: lanes 0-15 = tile cols 0-15.
//   row_inb[r]      tile row r holds a real output row (r < M remaining).
//                   STRIP uses bits 0-3.
//   col_inb[c]      tile column c holds a real output column. FULL uses 0-7.
//                   Masks only affect the counters (padding is zero). Same
//                   timing rule as mode_strip (registered once, then held).
//   drain_row       row to read. FULL: 0-7. STRIP: 0-3 (bit 2 ignored).
//
// Outputs
//   drain_data[j]   signed int32 lane j (two's complement in a packed bus),
//                   registered: valid 1 cycle after drain_row (L_DRAIN_MUX = 1).
//                   Lane j always = tile column j:
//                     FULL : lane c   = PE(r, c) = C[r, c]        (lanes 8-15 = 0)
//                     STRIP: lane c   = PE(i, c)   = C[i, c]
//                            lane 8+c = PE(4+i, c) = C[i, 8+c]
//   mac_cnt_q[q]    number of PEs in quadrant q that perform an in-bounds valid
//                   MAC (PE operand-register stage), registered.
//   mac_cnt         sum of the four, same cycle. Over a tile, each in-bounds PE
//                   counts exactly K, so the sum over a job is M*N*K.
//
// Timing (FULL and STRIP identical): word k presented at the array port in
// cycle t0+k reaches the port of PE(r,c) in cycle t0+k+r+c, so the last MAC
// of PE(7,7) enters in t0+(K-1)+14 and its acc is final from
// t0+(K-1)+14+L_PE. In STRIP, PE(4+i,c) gets A[i,k] (A lane i, delay 4+i) and
// W[k,8+c] (W lane 8+c, delay 4+c, then i rows down) in cycle t0+k+4+i+c.
// mac_cnt* lag the PE operand stage by 1 cycle, i.e. the count for the MAC
// entering PE(r,c) in cycle t appears on mac_cnt* in cycle t+2.
//
// Invariants
//   * Every mode mux is a 2:1 LUT whose output lands directly in a register
//     (skew stage or the PE operand register). Nothing unregistered sits
//     between a mux and a DSP multiplier input.
//   * vld/first travel with A through the same registers as the data.
// Critical paths (estimates, 100 MHz):
//   * row-4 W mux: PE(3,c).w_q (FF) -> LUT3 -> PE(4,c) B register. 1 LUT.
//   * drain mux: P reg -> 8:1 mux (LUT6+MUXF7+MUXF8) -> FF. ~1.5 LUT levels
//     plus routing across the array; registered.
//   * popcount: vld_q & inb_q -> popcount16 (~3 LUT) -> 4-way add (~2) -> FF.
// =============================================================================
module array
    import splitsys_pkg::*;
#(
    parameter bit ENABLE_STRIP = splitsys_pkg::ENABLE_STRIP
) (
    input  logic                                   clk,
    input  logic                                   rst,
    input  logic                                   mode_strip,
    input  logic [ROWS-1:0][7:0]                   a_in,
    input  logic                                   a_vld,
    input  logic                                   a_first,
    input  logic [W_LANES-1:0][7:0]                w_in,
    input  logic [ROWS-1:0]                        row_inb,
    input  logic [STRIP_TILE_N-1:0]                col_inb,
    input  logic [$clog2(ROWS)-1:0]                drain_row,
    output logic [STRIP_TILE_N-1:0][31:0]          drain_data,
    output logic [N_QUADS-1:0][$clog2(QUAD*QUAD):0] mac_cnt_q,
    output logic [$clog2(N_PES):0]                 mac_cnt
);
    // Geometry. The STRIP wiring assumes the array splits exactly in half at
    // row H, that the halves side by side are 2*C wide, and H is a power of 2.
    localparam int R  = ROWS;          // 8
    localparam int C  = COLS;          // 8
    localparam int H  = QUAD;          // 4: row-H boundary, quadrant edge
    localparam int RB = $clog2(ROWS);  // drain_row width
    localparam int QB = $clog2(QUAD*QUAD) + 1;   // per-quadrant count width

    // -------------------------------------------------------------------------
    // Mode register
    // -------------------------------------------------------------------------
    logic strip_q;
    if (ENABLE_STRIP) begin : g_mode
        always_ff @(posedge clk) begin
            if (rst) strip_q <= 1'b0;
            else     strip_q <= mode_strip;
        end
    end else begin : g_mode_off
        assign strip_q = 1'b0;
        /* verilator lint_off UNUSEDSIGNAL */
        logic unused_strip;
        assign unused_strip = ^{mode_strip, w_in[W_LANES-1:C], col_inb[STRIP_TILE_N-1:C]};
        /* verilator lint_on UNUSEDSIGNAL */
    end

    // -------------------------------------------------------------------------
    // A-source mux (before the A skew, so lane 4+i keeps its 4+i delay)
    // -------------------------------------------------------------------------
    logic [R-1:0][7:0] a_src;
    for (genvar r = 0; r < R; r++) begin : g_asrc
        if (ENABLE_STRIP && r >= H) begin : g_mux
            assign a_src[r] = strip_q ? a_in[r-H] : a_in[r];
        end else begin : g_thru
            assign a_src[r] = a_in[r];
        end
    end

    // -------------------------------------------------------------------------
    // Skew units
    // -------------------------------------------------------------------------
    logic [R-1:0][7:0] a_sk;
    logic [R-1:0]      a_sk_vld, a_sk_first;

    skew #(.LANES(R), .WIDTH(8), .BASE('0), .FLAGS(1'b1)) u_askew (
        .clk, .rst,
        .in_data (a_src),
        .in_vld  ({R{a_vld}}),
        .in_first({R{a_first}}),
        .out_data(a_sk),
        .out_vld (a_sk_vld),
        .out_first(a_sk_first)
    );

    // W skew. Two instances so that the in-instance lane index equals the
    // column c at the edge where the lane enters:
    //   u_wskew_top : W lanes 0..C-1   -> top edge,        delay c      (BASE 0)
    //   u_wskew_mid : W lanes C..2C-1  -> row-H boundary,  delay H + c  (BASE H)
    // (A single 16-lane instance with BASE[j]+j would delay lane C+c by
    // H + C + c, which is NOT the SPEC sec. 3.2 alignment.)
    logic [C-1:0][7:0] w_sk_top;      // enters PE(0, c)
    logic [C-1:0][7:0] w_sk_mid;      // enters PE(H, c) in STRIP
    /* verilator lint_off UNUSEDSIGNAL */
    logic [C-1:0] w_top_vld_nc, w_top_first_nc;   // FLAGS = 0: tied 0
    /* verilator lint_on UNUSEDSIGNAL */

    skew #(.LANES(C), .WIDTH(8), .BASE('0), .FLAGS(1'b0)) u_wskew_top (
        .clk, .rst,
        .in_data (w_in[C-1:0]),
        .in_vld  ('0),
        .in_first('0),
        .out_data(w_sk_top),
        .out_vld (w_top_vld_nc),
        .out_first(w_top_first_nc)
    );

    if (ENABLE_STRIP) begin : g_wmid
        // Per-lane base for lanes C..2C-1 from the package (= H for each lane).
        localparam logic [8*C-1:0] MID_BASE = W_SKEW_BASE_STRIP[8*W_LANES-1 : 8*C];
        /* verilator lint_off UNUSEDSIGNAL */
        logic [C-1:0] w_mid_vld_nc, w_mid_first_nc;
        /* verilator lint_on UNUSEDSIGNAL */
        skew #(.LANES(C), .WIDTH(8), .BASE(MID_BASE), .FLAGS(1'b0)) u_wskew_mid (
            .clk, .rst,
            .in_data (w_in[W_LANES-1:C]),
            .in_vld  ('0),
            .in_first('0),
            .out_data(w_sk_mid),
            .out_vld (w_mid_vld_nc),
            .out_first(w_mid_first_nc)
        );
    end else begin : g_wmid_off
        assign w_sk_mid = '0;
    end

    // -------------------------------------------------------------------------
    // PE grid
    //   a_h[r][c] / v_h / f_h : horizontal operand entering PE(r,c); column C is
    //                           the (unused) right-edge output.
    //   w_v[r][c]             : weight leaving PE(r-1,c) downward (row 0 = skew).
    //   w_pe[r][c]            : weight entering PE(r,c) (row-H mux applied).
    // -------------------------------------------------------------------------
    /* verilator lint_off UNUSEDSIGNAL */
    logic [R-1:0][C:0][7:0] a_h;      // [r][C] right-edge outputs unused
    logic [R-1:0][C:0]      v_h, f_h; // f_h[r][C] unused
    logic [R:0][C-1:0][7:0] w_v;      // w_v[R] bottom-edge outputs unused
    /* verilator lint_on UNUSEDSIGNAL */
    logic [R-1:0][C-1:0][7:0]  w_pe;
    // acc of PE(r,c) at acc[32*(r*C+c) +: 32] (flat so it can be indexed by a
    // variable row in every simulator)
    logic [R*C*32-1:0]         acc;

    for (genvar r = 0; r < R; r++) begin : g_left
        assign a_h[r][0] = a_sk[r];
        assign v_h[r][0] = a_sk_vld[r];
        assign f_h[r][0] = a_sk_first[r];
    end
    for (genvar c = 0; c < C; c++) begin : g_top
        assign w_v[0][c] = w_sk_top[c];
    end

    for (genvar r = 0; r < R; r++) begin : g_row
        for (genvar c = 0; c < C; c++) begin : g_col
            if (ENABLE_STRIP && r == H) begin : g_wmux
                // row-H boundary: STRIP takes W lane C+c (delayed H+c)
                assign w_pe[r][c] = strip_q ? w_sk_mid[c] : w_v[r][c];
            end else begin : g_wthru
                assign w_pe[r][c] = w_v[r][c];
            end

            pe u_pe (
                .clk, .rst,
                .a_in       (a_h[r][c]),
                .a_vld_in   (v_h[r][c]),
                .a_first_in (f_h[r][c]),
                .w_in       (w_pe[r][c]),
                .a_out      (a_h[r][c+1]),
                .a_vld_out  (v_h[r][c+1]),
                .a_first_out(f_h[r][c+1]),
                .w_out      (w_v[r+1][c]),
                .acc_out    (acc[32*(r*C+c) +: 32])
            );
        end
    end

    // -------------------------------------------------------------------------
    // In-bounds mask (registered, physical PE coordinates)
    //   FULL : PE(r,c)   in-bounds = row_inb[r]   & col_inb[c]
    //   STRIP: PE(i,c)   in-bounds = row_inb[i]   & col_inb[c]
    //          PE(H+i,c) in-bounds = row_inb[i]   & col_inb[C+c]
    // -------------------------------------------------------------------------
    logic [R-1:0][C-1:0] inb_d, inb_q;
    for (genvar r = 0; r < R; r++) begin : g_inb_r
        for (genvar c = 0; c < C; c++) begin : g_inb_c
            if (ENABLE_STRIP && r >= H) begin : g_split
                assign inb_d[r][c] = strip_q ? (row_inb[r-H] & col_inb[C+c])
                                             : (row_inb[r]   & col_inb[c]);
            end else begin : g_plain
                assign inb_d[r][c] = row_inb[r] & col_inb[c];
            end
        end
    end

    always_ff @(posedge clk) begin
        if (rst) inb_q <= '0;
        else     inb_q <= inb_d;
    end

    // -------------------------------------------------------------------------
    // Useful-MAC popcounts. PE(r,c) performs a MAC while its operand register
    // holds a valid pair: v_h[r][c+1] (= PE(r,c).a_vld_out).
    // -------------------------------------------------------------------------
    function automatic logic [QB-1:0] popcount_q(input logic [H*H-1:0] x);
        logic [QB-1:0] n;
        n = '0;
        for (int i = 0; i < H*H; i++) n = n + QB'(x[i]);
        return n;
    endfunction

    logic [3:0][H*H-1:0] mac_bits;
    for (genvar q = 0; q < 4; q++) begin : g_quad
        for (genvar i = 0; i < H; i++) begin : g_qr
            for (genvar j = 0; j < H; j++) begin : g_qc
                // quadrant q: row offset (q/2)*H, col offset (q%2)*H
                assign mac_bits[q][i*H + j] =
                    v_h[(q/2)*H + i][(q%2)*H + j + 1] & inb_q[(q/2)*H + i][(q%2)*H + j];
            end
        end
    end

    logic [3:0][QB-1:0] pc;
    for (genvar q = 0; q < 4; q++) begin : g_pc
        assign pc[q] = popcount_q(mac_bits[q]);
    end

    always_ff @(posedge clk) begin
        if (rst) begin
            mac_cnt_q <= '0;
            mac_cnt   <= '0;
        end else begin
            mac_cnt_q <= pc;
            mac_cnt   <= ({2'b0, pc[0]} + {2'b0, pc[1]}) + ({2'b0, pc[2]} + {2'b0, pc[3]});
            // (QB+2 = 7 bits = $clog2(N_PES)+1: holds 0..64)
        end
    end

    // -------------------------------------------------------------------------
    // Drain read mux (registered). Lane j = tile column j in both modes.
    // -------------------------------------------------------------------------
    // Written as a compare-select over rows (an 8:1 mux per lane bit after
    // synthesis), which every simulator accepts.
    logic [STRIP_TILE_N-1:0][31:0] drain_d;
    logic [RB-1:0] row_up;    // physical row feeding lanes 0..C-1
    assign row_up = strip_q ? (drain_row & RB'(H-1)) : drain_row;

    always_comb begin
        drain_d = '0;
        for (int r = 0; r < R; r++) begin
            if (row_up == RB'(r)) begin
                for (int c = 0; c < C; c++) drain_d[c] = acc[32*(r*C+c) +: 32];
            end
        end
        // STRIP: physical row H+i feeds lanes C..2C-1 (compiled out without STRIP)
        if (ENABLE_STRIP && strip_q) begin
            for (int i = 0; i < H; i++) begin
                if (drain_row[RB-2:0] == (RB-1)'(i)) begin
                    for (int c = 0; c < C; c++) drain_d[C+c] = acc[32*((H+i)*C+c) +: 32];
                end
            end
        end
    end

    always_ff @(posedge clk) begin
        if (rst) drain_data <= '0;
        else     drain_data <= drain_d;
    end

endmodule
