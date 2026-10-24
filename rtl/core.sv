// =============================================================================
// core.sv -- SplitSys compute core: buffers + sequencer + array + requant
// -----------------------------------------------------------------------------
//   mem bus --> region decode --> WBUF / ABUF / OBUF / RQPARAM   (port A)
//                                   | port B
//   run_* --> sequencer --> ABUF/WBUF reads --> lane select --> array (skews inside)
//                |          RQPARAM reads --> requant bank
//                +- drain_row --> array drain --> requant --> OBUF writes
//
// Not included (instantiated by top.sv): packet, counters, uart_*, board_io,
// selftest. Self-test arbitration: top muxes the memory bus AND the run
// interface between packet.sv and selftest.sv in front of this core (e.g.
// select = selftest busy). Both requesters use exactly the same protocol, and
// the core never sees two requesters at once.
//
// Parameters
//   ENABLE_STRIP   1 = FULL + STRIP; 0 = FULL only, STRIP logic compiled out
//                  (packet.sv must then NACK mode STRIP: its STRIP_EN = 0).
//
// Ports (all synchronous to clk, rst synchronous active-high)
//   Memory bus (packet.sv contract; buffers port A):
//     mem_region   REGION_WBUF/ABUF/OBUF/RQPARAM (low 2 bits of region_e)
//     mem_addr     word address in that region's word size (low bits used)
//     mem_wdata    byte lane i at [8*i +: 8]; narrow regions use the low lanes
//     mem_be       byte enables for mem_we
//     mem_we       1-cycle write strobe (writes only the enabled bytes)
//     mem_re       1-cycle read strobe
//     mem_rdata    the word read, valid exactly L_RD = 2 cycles after mem_re
//                  (narrow regions zero-extended); held until the next read.
//     The bus must not be used while seq_busy (packet.sv answers BUSY).
//   Run interface (packet.sv contract):
//     run_desc     24-byte descriptor, byte i at [8*i +: 8], already validated
//     run_valid    request; held until run_accept
//     run_accept   1 while the sequencer is idle (job taken when both are 1)
//     seq_busy     job running (accept cycle excluded, DONE cycle included)
//     seq_done     level: last job finished; cleared by the next accept
//     seq_error    0 (MVP)
//   Counter taps (counters.sv contract, see sequencer.sv for the exact rules):
//     ctr_start    1-cycle pulse in the accept cycle    -> counters.start
//     ctr_done     1-cycle pulse in the DONE cycle      -> counters.done
//     tap_setup / tap_stream / tap_flush / tap_drain / tap_stall
//     mac_cnt      in-bounds MACs this cycle (0..64)    -> counters.mac_cnt
//     mac_q_cnt    quadrant q at [5*q +: 5] (0..16)     -> counters.mac_q_cnt
//     tile_done    1-cycle pulse per completed tile     -> counters.tile_done
//   Observability:
//     dbg_state    sequencer state (sequencer.sv state_e)
//     dbg_strip    current job is STRIP
//
// Latencies (equal to splitsys_pkg; checked at elaboration in sequencer.sv):
//   L_PE = 3, L_RD = 2, L_SETUP = 16, L_RQ = 7 (1 drain register + 6 requant),
//   L_WB = 1, L_ACCEPT = 1, L_DONE = 1. Per tile:
//   T_tile = L_SETUP + (L_RD + K) + 17 + T_DRAIN(mode, flags) + L_RQ + L_WB.
//
// Resources (expected): 64 + 32 DSP48E1; BRAM36: WBUF 16, ABUF 4, OBUF 4,
//   RQPARAM 1.
// =============================================================================
module core
    import splitsys_pkg::*;
#(
    parameter bit STRIP_EN = splitsys_pkg::ENABLE_STRIP
) (
    input  logic                          clk,
    input  logic                          rst,
    // memory bus
    input  logic [$clog2(N_REGIONS)-1:0]  mem_region,
    input  logic [WBUF_AW-1:0]            mem_addr,
    input  logic [WBUF_WORD_BITS-1:0]     mem_wdata,
    input  logic [WBUF_WORD_BYTES-1:0]    mem_be,
    input  logic                          mem_we,
    input  logic                          mem_re,
    output logic [WBUF_WORD_BITS-1:0]     mem_rdata,
    // run interface
    input  logic [DESC_BYTES*8-1:0]       run_desc,
    input  logic                          run_valid,
    output logic                          run_accept,
    output logic                          seq_busy,
    output logic                          seq_done,
    output logic                          seq_error,
    // counter taps
    output logic                          ctr_start,
    output logic                          ctr_done,
    output logic                          tap_setup,
    output logic                          tap_stream,
    output logic                          tap_flush,
    output logic                          tap_drain,
    output logic                          tap_stall,
    output logic [$clog2(N_PES+1)-1:0]    mac_cnt,
    output logic [N_QUADS*5-1:0]          mac_q_cnt,
    output logic                          tile_done,
    // observability
    output logic [2:0]                    dbg_state,
    output logic                          dbg_strip
);
    localparam int RW = $clog2(N_REGIONS);

    // ------------------------------------------------------ region decode
    logic sel_w, sel_a, sel_o, sel_r;
    assign sel_w = (mem_region == RW'(REGION_WBUF));
    assign sel_a = (mem_region == RW'(REGION_ABUF));
    assign sel_o = (mem_region == RW'(REGION_OBUF));
    assign sel_r = (mem_region == RW'(REGION_RQPARAM));

    logic [RW-1:0] rd_region_q1, rd_region_q2;   // region of the read in flight
    always_ff @(posedge clk) begin
        if (rst) begin
            rd_region_q1 <= '0;
            rd_region_q2 <= '0;
        end else begin
            if (mem_re) rd_region_q1 <= mem_region;
            rd_region_q2 <= rd_region_q1;
        end
    end

    // ------------------------------------------------------ buffers
    logic [WBUF_WORD_BITS-1:0] w_a_rdata, w_b_rdata;
    logic [ABUF_WORD_BITS-1:0] a_a_rdata, a_b_rdata;
    logic [OBUF_WORD_BITS-1:0] o_a_rdata;
    logic [RQ_WORD_BITS-1:0]   r_a_rdata, r_b_rdata;
    /* verilator lint_off UNUSEDSIGNAL */
    logic [OBUF_WORD_BITS-1:0] o_b_rdata;           // OBUF port B only writes
    logic w_rv, a_rv, o_rv, r_rv;                   // a_rvalid: bus uses L_RD timing
    /* verilator lint_on UNUSEDSIGNAL */

    logic                       ab_en, wb_en, rq_en, ob_en;
    logic [ABUF_AW-1:0]         ab_addr;
    logic [WBUF_AW-1:0]         wb_addr;
    logic [RQ_AW-1:0]           rq_addr;
    logic [OBUF_AW-1:0]         ob_addr;
    logic [OBUF_WORD_BYTES-1:0] ob_we;
    logic [OBUF_WORD_BITS-1:0]  ob_wdata;

    buffers #(.WORDS(WBUF_WORDS), .WORD_BYTES(WBUF_WORD_BYTES)) u_wbuf (
        .clk, .rst,
        .a_en   (sel_w && (mem_we || mem_re)),
        .a_we   (mem_we ? mem_be : '0),
        .a_addr (mem_addr[WBUF_AW-1:0]),
        .a_wdata(mem_wdata),
        .a_rdata(w_a_rdata),
        .a_rvalid(w_rv),
        .b_en   (wb_en),
        .b_we   ('0),
        .b_addr (wb_addr),
        .b_wdata('0),
        .b_rdata(w_b_rdata)
    );

    buffers #(.WORDS(ABUF_WORDS), .WORD_BYTES(ABUF_WORD_BYTES)) u_abuf (
        .clk, .rst,
        .a_en   (sel_a && (mem_we || mem_re)),
        .a_we   (mem_we ? mem_be[ABUF_WORD_BYTES-1:0] : '0),
        .a_addr (mem_addr[ABUF_AW-1:0]),
        .a_wdata(mem_wdata[ABUF_WORD_BITS-1:0]),
        .a_rdata(a_a_rdata),
        .a_rvalid(a_rv),
        .b_en   (ab_en),
        .b_we   ('0),
        .b_addr (ab_addr),
        .b_wdata('0),
        .b_rdata(a_b_rdata)
    );

    buffers #(.WORDS(OBUF_WORDS), .WORD_BYTES(OBUF_WORD_BYTES)) u_obuf (
        .clk, .rst,
        .a_en   (sel_o && (mem_we || mem_re)),
        .a_we   (mem_we ? mem_be[OBUF_WORD_BYTES-1:0] : '0),
        .a_addr (mem_addr[OBUF_AW-1:0]),
        .a_wdata(mem_wdata[OBUF_WORD_BITS-1:0]),
        .a_rdata(o_a_rdata),
        .a_rvalid(o_rv),
        .b_en   (ob_en),
        .b_we   (ob_we),
        .b_addr (ob_addr),
        .b_wdata(ob_wdata),
        .b_rdata(o_b_rdata)
    );

    buffers #(.WORDS(RQ_ENTRIES), .WORD_BYTES(RQ_WORD_BYTES)) u_rqparam (
        .clk, .rst,
        .a_en   (sel_r && (mem_we || mem_re)),
        .a_we   (mem_we ? mem_be[RQ_WORD_BYTES-1:0] : '0),
        .a_addr (mem_addr[RQ_AW-1:0]),
        .a_wdata(mem_wdata[RQ_WORD_BITS-1:0]),
        .a_rdata(r_a_rdata),
        .a_rvalid(r_rv),
        .b_en   (rq_en),
        .b_we   ('0),
        .b_addr (rq_addr),
        .b_wdata('0),
        .b_rdata(r_b_rdata)
    );

    /* verilator lint_off UNUSEDSIGNAL */
    logic unused_bus;   // upper address/data bits unused by the narrower regions
    assign unused_bus = ^{mem_addr, mem_wdata, mem_be};
    /* verilator lint_on UNUSEDSIGNAL */

    always_comb begin
        case (rd_region_q2)
            RW'(REGION_WBUF): mem_rdata = w_a_rdata;
            RW'(REGION_ABUF): mem_rdata = WBUF_WORD_BITS'(a_a_rdata);
            RW'(REGION_OBUF): mem_rdata = WBUF_WORD_BITS'(o_a_rdata);
            default:          mem_rdata = WBUF_WORD_BITS'(r_a_rdata);
        endcase
    end

    // ------------------------------------------------------ sequencer
    logic                          arr_mode_strip, arr_a_vld, arr_a_first;
    logic [ROWS-1:0][7:0]          arr_a;
    logic [W_LANES-1:0][7:0]       arr_w;
    logic [ROWS-1:0]               arr_row_inb;
    logic [STRIP_TILE_N-1:0]       arr_col_inb;
    logic [$clog2(ROWS)-1:0]       arr_drain_row;
    logic [STRIP_TILE_N-1:0][31:0] drain_data;
    logic                          prm_we, rq_bypass, rq_in_vld, rq_out_vld;
    logic [3:0]                    prm_lane;
    logic [31:0]                   prm_word;
    logic [STRIP_TILE_N-1:0][31:0] rq_out;

    sequencer #(.STRIP_EN(STRIP_EN)) u_seq (
        .clk, .rst,
        .run_desc, .run_valid, .run_accept, .seq_busy, .seq_done, .seq_error,
        .ctr_start, .ctr_done, .tap_setup, .tap_stream, .tap_flush, .tap_drain,
        .tap_stall, .tile_done,
        .ab_en, .ab_addr, .ab_rdata(a_b_rdata),
        .wb_en, .wb_addr, .wb_rdata(w_b_rdata),
        .rq_en, .rq_addr, .rq_rdata(r_b_rdata),
        .ob_en, .ob_we, .ob_addr, .ob_wdata,
        .arr_mode_strip, .arr_a, .arr_a_vld, .arr_a_first, .arr_w,
        .arr_row_inb, .arr_col_inb, .arr_drain_row,
        .rq_prm_we(prm_we), .rq_prm_lane(prm_lane), .rq_prm_word(prm_word),
        .rq_bypass, .rq_in_vld, .rq_out_vld, .rq_out,
        .dbg_state
    );
    assign dbg_strip = arr_mode_strip;

    // ------------------------------------------------------ array + requant
    logic [N_QUADS-1:0][4:0] mac_q;

    array #(.ENABLE_STRIP(STRIP_EN)) u_array (
        .clk, .rst,
        .mode_strip(arr_mode_strip),
        .a_in      (arr_a),
        .a_vld     (arr_a_vld),
        .a_first   (arr_a_first),
        .w_in      (arr_w),
        .row_inb   (arr_row_inb),
        .col_inb   (arr_col_inb),
        .drain_row (arr_drain_row),
        .drain_data(drain_data),
        .mac_cnt_q (mac_q),
        .mac_cnt   (mac_cnt)
    );
    assign mac_q_cnt = mac_q;

    requant #(.LANES(STRIP_TILE_N)) u_rq (
        .clk, .rst,
        .prm_we, .prm_lane, .prm_word,
        .bypass  (rq_bypass),
        .in_vld  (rq_in_vld),
        .in_acc  (drain_data),
        .out_vld (rq_out_vld),
        .out_data(rq_out)
    );

endmodule
