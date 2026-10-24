// =============================================================================
// requant.sv -- 16-lane int32 -> int8 requantization (SPEC sec. 1.2, 3.6)
// -----------------------------------------------------------------------------
// Per lane j (lane j = tile output column j, as delivered by the array drain):
//
//     t = int64(acc) * int64(mult[j]) + (1 << (shift[j] - 1))
//     y = clamp(t >>> shift[j], -128, 127)
//
// Implemented exactly through the identity (s = shift >= 1, x = acc*mult):
//     floor((x + 2^(s-1)) / 2^s) == floor((floor(x / 2^(s-1)) + 1) / 2)
// so the hardware shifts first (q = x >>> (s-1)) and then rounds with a 10-bit
// add, instead of a 48-bit add in front of the shifter. The saturation tests
// are done on q:  y > 127 <=> q >= 255,  y < -128 <=> q <= -258.
//
// Pipeline (one stage per line, LATENCY = 6 cycles, port to port):
//   E1  input registers: acc, mult (or 1 in bypass), s-1 (or 0), vld   [DSP A/B]
//   E2  multiply  (32 x 16 signed -> 48)                               [DSP M]
//   E3  product                                                        [DSP P]
//   E4  product   (2nd register; Vivado cascades the two DSP48E1 that a
//                  32x16 multiply needs and absorbs E1..E4 into them)  [DSP P]
//   E5  q = product >>> (s-1)            (48-bit barrel shift, 5 levels)
//   E6  out = clamp((q + 1) >>> 1)        or, in bypass, q[31:0] = acc
// Two DSP48E1 per lane, 32 in total (SPEC sec. 5 budget: 64 + 32 <= 120).
//
// Ports (lane j of a packed bus is bus[j])
//   prm_we/prm_lane/prm_word   parameter load (sequencer TILE_SETUP): lane
//                       prm_lane gets the RQPARAM word {8'h0, shift[7:0],
//                       mult[15:0]}. Only mult[14:0] and shift[4:0] are used
//                       (SPEC: mult 1..32767, shift 1..31; hardware does not
//                       range-check, out-of-range values give unspecified y).
//                       Takes effect for data entering E1 in the next cycle.
//   bypass              1 = int32 path (flags = FLAGS_INT32): out = acc exactly,
//                       same latency. 0 = int8 requant (flags = FLAGS_INT8).
//                       Sampled with the data at E1 (may change per beat).
//   in_vld, in_acc[j]   one drained row: 16 signed int32 lanes
//   out_vld, out[j]     LATENCY cycles later. int8 mode: out[j] is y sign-
//                       extended to 32 bits (write out[j][7:0]); bypass:
//                       out[j] = acc. Lanes with no in_vld are don't-care but
//                       out_vld is exact.
//
// Invariants: parameters are captured per beat at E1, so the bank may be
//   reloaded as soon as the last beat of a tile has been presented.
// Critical path estimate (100 MHz): E5 barrel shifter, 48 bits x 5 levels
//   = 3 LUT6 levels (~2.5 ns + routing); E6 = 48-bit constant compares
//   (2 LUT levels) in parallel with a 10-bit carry chain, then a 3:1 mux.
//   The multiply is DSP-internal with all pipeline registers enabled.
// =============================================================================
module requant #(
    parameter int LANES = splitsys_pkg::STRIP_TILE_N   // 16
) (
    input  logic                    clk,
    input  logic                    rst,
    // parameter bank load
    input  logic                    prm_we,
    input  logic [$clog2(LANES)-1:0] prm_lane,
    input  logic [31:0]             prm_word,
    // data
    input  logic                    bypass,
    input  logic                    in_vld,
    input  logic [LANES-1:0][31:0]  in_acc,     // signed int32 per lane
    output logic                    out_vld,
    output logic [LANES-1:0][31:0]  out_data    // signed int32 per lane
);
    // ---- parameter bank (one register pair per lane)
    logic [LANES-1:0][14:0] mult_bank;
    logic [LANES-1:0][4:0]  shift_bank;

    always_ff @(posedge clk) begin
        if (rst) begin
            mult_bank  <= '0;
            shift_bank <= '0;
        end else if (prm_we) begin
            mult_bank[prm_lane]  <= prm_word[14:0];
            shift_bank[prm_lane] <= prm_word[20:16];
        end
    end

    /* verilator lint_off UNUSEDSIGNAL */
    logic unused_prm;   // RQPARAM bits outside mult[14:0], shift[4:0] are ignored
    assign unused_prm = ^{prm_word[31:21], prm_word[15]};
    /* verilator lint_on UNUSEDSIGNAL */

    // ---- control pipeline (vld, bypass) E1..E6
    logic [6:1] vld_p;
    logic [5:1] byp_p;

    always_ff @(posedge clk) begin
        if (rst) begin
            vld_p <= '0;
            byp_p <= '0;
        end else begin
            vld_p <= {vld_p[5:1], in_vld};
            byp_p <= {byp_p[4:1], bypass};
        end
    end
    assign out_vld = vld_p[6];

    // ---- datapath, per lane
    for (genvar j = 0; j < LANES; j++) begin : g_lane
        // E1: operand registers
        (* use_dsp = "yes" *) logic signed [31:0] acc_e1;
        (* use_dsp = "yes" *) logic signed [15:0] mult_e1;    // 0..32767, as signed
        logic [4:0] shm1_e1, shm1_e2, shm1_e3, shm1_e4;         // shift - 1

        always_ff @(posedge clk) begin
            if (rst) begin
                acc_e1  <= '0;
                mult_e1 <= '0;
                shm1_e1 <= '0;
            end else begin
                acc_e1  <= in_acc[j];
                mult_e1 <= bypass ? 16'sd1 : {1'b0, mult_bank[j]};
                shm1_e1 <= bypass ? 5'd0   : (shift_bank[j] - 5'd1);
            end
        end

        // E2..E4: multiply with three output registers (DSP M, P, cascade P)
        (* use_dsp = "yes" *) logic signed [47:0] prod_e2, prod_e3, prod_e4;

        always_ff @(posedge clk) begin
            if (rst) begin
                prod_e2 <= '0;
                prod_e3 <= '0;
                prod_e4 <= '0;
                shm1_e2 <= '0;
                shm1_e3 <= '0;
                shm1_e4 <= '0;
            end else begin
                prod_e2 <= acc_e1 * mult_e1;   // |x| < 2^46: exact in 48 bits
                prod_e3 <= prod_e2;
                prod_e4 <= prod_e3;
                shm1_e2 <= shm1_e1;
                shm1_e3 <= shm1_e2;
                shm1_e4 <= shm1_e3;
            end
        end

        // E5: q = x >>> (s-1)
        logic signed [47:0] q_e5;
        always_ff @(posedge clk) begin
            if (rst) q_e5 <= '0;
            else     q_e5 <= prod_e4 >>> shm1_e4;
        end

        // E6: round half up and saturate, or pass the int32 through (bypass)
        logic        sat_hi, sat_lo;
        logic [9:0]  rnd;          // q + 1 on 10 bits; valid when not saturated
        logic [7:0]  y;

        assign sat_hi = (q_e5 > 48'sd254);     // y > 127
        assign sat_lo = (q_e5 < -48'sd257);    // y < -128
        assign rnd    = q_e5[9:0] + 10'd1;     // q in [-257, 254] -> q+1 fits
        assign y      = sat_hi ? 8'h7F : sat_lo ? 8'h80 : rnd[8:1];

        always_ff @(posedge clk) begin
            if (rst)           out_data[j] <= '0;
            else if (byp_p[5]) out_data[j] <= q_e5[31:0];
            else               out_data[j] <= {{24{y[7]}}, y};
        end

        /* verilator lint_off UNUSEDSIGNAL */
        logic unused_lane;   // upper q bits only feed the saturation compares
        assign unused_lane = rnd[9] ^ rnd[0];
        /* verilator lint_on UNUSEDSIGNAL */
    end

endmodule
