// =============================================================================
// pe.sv -- SplitSys processing element (SPEC sec. 3.1)
// -----------------------------------------------------------------------------
// Output-stationary int8 MAC. Activations move right, weights move down, the
// partial sum stays in place. Maps onto exactly one DSP48E1:
//
//   port --> [A/B reg] --> (x) --> [M reg] --> (+) --> [P reg] = acc
//    a_in    a_q, w_q              m_q          ^        |
//    w_in    (also forwarded                    +-- first ? 0 : P (opmode Z mux)
//             to the neighbours)
//
// Ports
//   a_in  / w_in        signed int8 operands (unregistered, from the neighbour
//                       PE's forward register, or from the skew unit / row-4 mux)
//   a_vld_in            this cycle's a_in/w_in pair is a real MAC
//   a_first_in          first MAC of a new output: acc <= a*w (else acc + a*w)
//   a_out / w_out       a_in / w_in delayed by exactly 1 cycle (one register
//   a_vld_out           per PE in each direction). vld/first travel with `a`.
//   a_first_out
//   acc_out             signed int32 accumulator = low 32 bits of the DSP P
//                       register. Holds its value while no valid MAC arrives.
//
// Latency (LATENCY = 3, must equal splitsys_pkg::L_PE)
//   If (a_in, w_in, a_vld_in=1) are presented at the port in cycle t (sampled
//   at the clock edge that ends cycle t), acc_out reflects that MAC from cycle
//   t+3 onwards: edge 1 -> A/B regs, edge 2 -> M reg, edge 3 -> P reg.
//   Back-to-back valid MACs every cycle are supported (fully pipelined).
//
// Invariants
//   * vld/first are delayed through the same register stages as the operands,
//     so a MAC can never pick up a stale `first` (SPEC sec. 8).
//   * No saturation: with K <= 256, |acc| <= 256*128*128 = 2^22 << 2^31.
//   * Synchronous active-high reset clears every register (maps to the DSP's
//     RSTA/RSTB/RSTM/RSTP pins).
// Critical path: DSP-internal (M reg -> ALU -> P reg); fabric only drives the
// A/B inputs from neighbour registers (or a 2:1 mode mux at the row-4 boundary).
// =============================================================================
module pe (
    input  logic               clk,
    input  logic               rst,
    input  logic signed [7:0]  a_in,
    input  logic               a_vld_in,
    input  logic               a_first_in,
    input  logic signed [7:0]  w_in,
    output logic signed [7:0]  a_out,
    output logic               a_vld_out,
    output logic               a_first_out,
    output logic signed [7:0]  w_out,
    output logic signed [31:0] acc_out
);
    localparam int LATENCY = 3;

    // ---- stage 1: operand registers (DSP AREG/BREG; also the systolic forward)
    (* use_dsp = "yes" *) logic signed [7:0] a_q;
    (* use_dsp = "yes" *) logic signed [7:0] w_q;
    logic vld_q, first_q;

    always_ff @(posedge clk) begin
        if (rst) begin
            a_q     <= '0;
            w_q     <= '0;
            vld_q   <= 1'b0;
            first_q <= 1'b0;
        end else begin
            a_q     <= a_in;
            w_q     <= w_in;
            vld_q   <= a_vld_in;
            first_q <= a_first_in;
        end
    end

    // ---- stage 2: product register (DSP MREG)
    (* use_dsp = "yes" *) logic signed [15:0] m_q;
    logic vld_m, first_m;

    always_ff @(posedge clk) begin
        if (rst) begin
            m_q     <= '0;
            vld_m   <= 1'b0;
            first_m <= 1'b0;
        end else begin
            m_q     <= a_q * w_q;          // signed 8x8 -> signed 16, exact
            vld_m   <= vld_q;
            first_m <= first_q;
        end
    end

    // ---- stage 3: accumulator (DSP PREG, CEP = vld_m, opmode Z = first ? 0 : P)
    (* use_dsp = "yes" *) logic signed [31:0] acc_q;
    logic signed [31:0] m_ext;      // explicit sign extension of the product
    logic signed [31:0] acc_base;   // the DSP Z-mux: 0 on `first`, P otherwise

    assign m_ext    = {{16{m_q[15]}}, m_q};
    assign acc_base = first_m ? 32'sd0 : acc_q;

    always_ff @(posedge clk) begin
        if (rst) begin
            acc_q <= '0;
        end else if (vld_m) begin
            acc_q <= acc_base + m_ext;
        end
    end

    assign a_out       = a_q;
    assign w_out       = w_q;
    assign a_vld_out   = vld_q;
    assign a_first_out = first_q;
    assign acc_out     = acc_q;

endmodule
