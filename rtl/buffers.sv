// =============================================================================
// buffers.sv -- true dual-port block RAM buffer (SPEC sec. 3.4)
// -----------------------------------------------------------------------------
// One generic module, instantiated once per region by the top level:
//
//   region   WORDS                       WORD_BYTES                     port B user
//   WBUF     splitsys_pkg::WBUF_WORDS    splitsys_pkg::WBUF_WORD_BYTES  sequencer read (W)
//   ABUF     splitsys_pkg::ABUF_WORDS    splitsys_pkg::ABUF_WORD_BYTES  sequencer read (A)
//   OBUF     splitsys_pkg::OBUF_WORDS    splitsys_pkg::OBUF_WORD_BYTES  writeback write
//   RQPARAM  splitsys_pkg::RQ_ENTRIES    splitsys_pkg::RQ_WORD_BYTES    TILE_SETUP read
//
// The memory follows the UG901 byte-write true-dual-port template (read-first
// on both ports, single clock), so Vivado infers RAMB36E1/RAMB18E1 with the
// optional output register (DOA_REG = DOB_REG = 1).
//
// Port A -- host side, byte granular (packet layer, self-test)
//   h_en      access this cycle
//   h_we      1 = write h_wdata to byte h_addr, 0 = read byte h_addr
//   h_addr    BYTE address within the region: word = h_addr / WORD_BYTES,
//             lane = h_addr % WORD_BYTES (little-endian, SPEC sec. 1.3).
//             Out-of-range checks belong to the packet layer (NACK_RANGE).
//   h_wdata   byte to write (replicated to every lane, byte enable = lane)
//   h_rdata   byte read: valid exactly L_RD = 2 cycles after an h_en & !h_we
//             cycle and held until the next host read reaches the output.
//             Write cycles do not disturb h_rdata.
//   h_rvalid  1-cycle pulse, L_RD = 2 cycles after each host read: h_rdata
//             is that read's byte in this cycle.
//
// Port B -- array side, full word
//   b_en      access this cycle
//   b_we      per-byte write enables (all 0 = read). Lane i = b_wdata[8*i +: 8].
//   b_addr    WORD address
//   b_wdata   word to write (OBUF writeback; FULL int8 uses b_we = 16'h00FF etc.)
//   b_rdata   word read: valid exactly L_RD = 2 cycles after an enabled cycle
//             with b_we = 0 (read-first: a write cycle also returns the OLD word,
//             which the user simply ignores). Held while b_en = 0.
//
// Latency: L_RD = 2 = BRAM read latch (cycle 1) + output register (cycle 2),
//          equal to splitsys_pkg::L_RD.
// Collisions: both ports are read-first on one clock. A port reading the
//          address the other port writes in the same cycle returns the OLD
//          data (UG473 read-first). Two writes to the same byte in the same
//          cycle are undefined; the packet layer refuses host memory access
//          while a job runs (BUSY), so a host/sequencer write collision
//          cannot occur.
// Reset:  the output registers are synchronously reset. The memory array and
//          the BRAM-internal read latch are not (they are not fabric flops and
//          a reset would block inference); the memory powers up as zero
//          (BRAM INIT = 0, mirrored by the initial block for simulation).
// Critical path: BRAM clock-to-out with DO_REG = 1 (~0.9 ns) -> consumer;
//          host byte mux (WORD_BYTES:1 x 8 bits, 1-2 LUT levels) after the
//          output register -> packet layer register.
// =============================================================================
module buffers #(
    parameter int WORDS      = splitsys_pkg::WBUF_WORDS,
    parameter int WORD_BYTES = splitsys_pkg::WBUF_WORD_BYTES
) (
    input  logic                                  clk,
    input  logic                                  rst,
    // port A: host, byte granular
    input  logic                                  h_en,
    input  logic                                  h_we,
    input  logic [$clog2(WORDS*WORD_BYTES)-1:0]   h_addr,
    input  logic [7:0]                            h_wdata,
    output logic [7:0]                            h_rdata,
    output logic                                  h_rvalid,
    // port B: array / sequencer, full word
    input  logic                                  b_en,
    input  logic [WORD_BYTES-1:0]                 b_we,
    input  logic [$clog2(WORDS)-1:0]              b_addr,
    input  logic [8*WORD_BYTES-1:0]               b_wdata,
    output logic [8*WORD_BYTES-1:0]               b_rdata
);
    localparam int W  = 8 * WORD_BYTES;
    localparam int AW = $clog2(WORDS);
    localparam int LB = (WORD_BYTES > 1) ? $clog2(WORD_BYTES) : 1;   // lane bits

    // ---- host byte -> word access
    logic [AW-1:0]         a_addr;
    logic [LB-1:0]         a_lane;
    logic [WORD_BYTES-1:0] a_we;
    logic [W-1:0]          a_wdata;

    if (WORD_BYTES > 1) begin : g_lanes
        assign a_addr = h_addr[$clog2(WORDS*WORD_BYTES)-1 -: AW];
        assign a_lane = h_addr[LB-1:0];
    end else begin : g_one_lane
        assign a_addr = h_addr;
        assign a_lane = '0;
    end

    always_comb begin
        for (int i = 0; i < WORD_BYTES; i++) a_we[i] = h_we && (a_lane == LB'(i));
    end
    assign a_wdata = {WORD_BYTES{h_wdata}};

    // ---- memory (UG901 bytewrite true dual port, read-first)
    // Two processes write the array (one per port) exactly as the vendor
    // template requires; this is the documented BRAM idiom, not a multi-driver.
    (* ram_style = "block" *) logic [W-1:0] mem [WORDS];
    logic [W-1:0] a_q, b_q;           // BRAM read latches

    initial begin
        for (int i = 0; i < WORDS; i++) mem[i] = '0;
    end

    /* verilator lint_off MULTIDRIVEN */
    always @(posedge clk) begin : p_port_a
        if (h_en) begin
            for (int i = 0; i < WORD_BYTES; i++) begin
                if (a_we[i]) mem[a_addr][8*i +: 8] <= a_wdata[8*i +: 8];
            end
            a_q <= mem[a_addr];
        end
    end

    always @(posedge clk) begin : p_port_b
        if (b_en) begin
            for (int i = 0; i < WORD_BYTES; i++) begin
                if (b_we[i]) mem[b_addr][8*i +: 8] <= b_wdata[8*i +: 8];
            end
            b_q <= mem[b_addr];
        end
    end
    /* verilator lint_on MULTIDRIVEN */

    // ---- output registers (DOA_REG / DOB_REG), L_RD = 2
    logic [W-1:0]  a_out_q;
    logic [LB-1:0] lane_q1, lane_q2;   // host lane select travels with the read
    logic          a_rd_q1, a_rd_q2;   // host read in flight (writes keep h_rdata)

    always_ff @(posedge clk) begin
        if (rst) begin
            a_rd_q1 <= 1'b0;
            a_rd_q2 <= 1'b0;
            lane_q1 <= '0;
            lane_q2 <= '0;
        end else begin
            a_rd_q1 <= h_en && !h_we;
            a_rd_q2 <= a_rd_q1;
            if (h_en && !h_we) lane_q1 <= a_lane;
            if (a_rd_q1)       lane_q2 <= lane_q1;
        end
    end

    always_ff @(posedge clk) begin
        if (rst)          a_out_q <= '0;
        else if (a_rd_q1) a_out_q <= a_q;
    end

    always_ff @(posedge clk) begin
        if (rst) b_rdata <= '0;
        else     b_rdata <= b_q;
    end

    assign h_rdata  = a_out_q[8*lane_q2 +: 8];
    assign h_rvalid = a_rd_q2;

endmodule
