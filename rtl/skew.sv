// =============================================================================
// skew.sv -- per-lane delay line for systolic operand skew (SPEC sec. 3.3)
// -----------------------------------------------------------------------------
// Lane j is delayed by exactly  D(j) = BASE[j] + j  clock cycles. Data, vld and
// first of a lane travel through the same registers, so they stay together.
//
// Parameters
//   LANES   number of lanes
//   WIDTH   data bits per lane
//   BASE    per-lane extra delay, packed 8 bits per lane: BASE[8*j +: 8].
//           A skew:                        BASE = 0 per lane  (D = r)
//           W skew, top edge  (W lanes 0-7):  BASE = 0 per lane  (D = c)
//           W skew, row-4 entry (W lanes 8-15, STRIP only; a separate
//             8-lane instance, so j = c there): BASE = 4 per lane (D = 4 + c)
//   FLAGS   1: carry vld/first per lane. 0: no flag registers are built and
//           out_vld/out_first are tied to 0 (used for the W path, whose
//           validity travels with the activations, SPEC sec. 3.1).
//
// Ports (lane j of a packed bus is bus[j])
//   in_data/in_vld/in_first    lane inputs, sampled every cycle
//   out_data/out_vld/out_first lane outputs; a lane with D = 0 is a wire
//
// Latency: lane j output in cycle t+D(j) equals its input in cycle t.
// Invariants: synchronous reset clears every stage (out_vld = 0 after reset).
// Critical path: none (register to register, no logic).
// =============================================================================
module skew #(
    parameter int                 LANES = 8,
    parameter int                 WIDTH = 8,
    parameter logic [8*LANES-1:0] BASE  = '0,
    parameter bit                 FLAGS = 1'b1
) (
    input  logic                        clk,
    input  logic                        rst,
    input  logic [LANES-1:0][WIDTH-1:0] in_data,
    input  logic [LANES-1:0]            in_vld,
    input  logic [LANES-1:0]            in_first,
    output logic [LANES-1:0][WIDTH-1:0] out_data,
    output logic [LANES-1:0]            out_vld,
    output logic [LANES-1:0]            out_first
);
    for (genvar j = 0; j < LANES; j++) begin : g_lane
        localparam int D = int'(BASE[8*j +: 8]) + j;

        if (D == 0) begin : g_wire
            assign out_data[j] = in_data[j];
            if (FLAGS) begin : g_f
                assign out_vld[j]   = in_vld[j];
                assign out_first[j] = in_first[j];
            end else begin : g_nf
                assign out_vld[j]   = 1'b0;
                assign out_first[j] = 1'b0;
            end
        end else begin : g_dly
            // One shift register per lane; flags (if any) ride in the top bits.
            localparam int SW = WIDTH + (FLAGS ? 2 : 0);
            logic [SW-1:0] sr [D];
            logic [SW-1:0] sr_in;

            if (FLAGS) begin : g_f
                assign sr_in        = {in_first[j], in_vld[j], in_data[j]};
                assign out_vld[j]   = sr[D-1][WIDTH];
                assign out_first[j] = sr[D-1][WIDTH+1];
            end else begin : g_nf
                assign sr_in        = in_data[j];
                assign out_vld[j]   = 1'b0;
                assign out_first[j] = 1'b0;
            end
            assign out_data[j] = sr[D-1][WIDTH-1:0];

            always_ff @(posedge clk) begin
                if (rst) begin
                    for (int s = 0; s < D; s++) sr[s] <= '0;
                end else begin
                    sr[0] <= sr_in;
                    for (int s = 1; s < D; s++) sr[s] <= sr[s-1];
                end
            end
        end
    end

endmodule
