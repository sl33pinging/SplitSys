// =============================================================================
// packet_mem.sv - bring-up stage 2 top (owner: fpga-integrator)
// -----------------------------------------------------------------------------
// Production modules: uart_rx, uart_tx, packet, buffers (x4: WBUF, ABUF, OBUF,
// RQPARAM), board_io. Everything else here is bring-up glue:
//   * the port-A memory mux (packet's memory bus -> the four buffers), the same
//     job rtl/core.sv does in the full design;
//   * a SEQUENCER STUB: RUN_GEMM is accepted immediately (run_accept =
//     run_valid), nothing is computed, and POLL reports DONE afterwards;
//   * a SELF-TEST STUB: st_done pulses one cycle after st_start with
//     pass_full = pass_strip = 0 and both Fletcher-32 values 0 (it is not a
//     self-test; the host must not treat it as one);
//   * COUNTERS STUB: READ_COUNTERS returns all zeros.
// Port B of every buffer is idle (the array is not present).
//
// What the board shows (docs/HW_CHECKLIST.md H-3): board_io's LEDs (LED15
// heartbeat, LED3 UART activity, LED0-2 off) and its 7-segment display ('-'
// until a SET_DISPLAY, then the mode letter and utilization). BTN1 = reset.
//
// Parameters: defaults are the hardware values. DIV_OVERRIDE and the board_io
// timing parameters are for simulation only; build.tcl never overrides them
// and refuses a nonzero DIV_OVERRIDE.
// =============================================================================
module packet_mem
  import splitsys_pkg::*;
#(
  parameter logic [31:0] BUILD_ID       = 32'h0000_0000,
  parameter int unsigned DIV_OVERRIDE   = 0,                 // SIM ONLY
  parameter int unsigned RX_TIMEOUT_CYC = RX_TIMEOUT,        // sim may lower
  parameter int unsigned DEB_CYC        = CLK_HZ / 100,
  parameter int unsigned HB_HALF_CYC    = CLK_HZ / 2,
  parameter int unsigned ACT_CYC        = CLK_HZ / 20,
  parameter int unsigned SCAN_CYC       = CLK_HZ / 1000
) (
  input  logic        CLK_100MHZ,
  input  logic [3:0]  BTN,
  output logic [15:0] LED,
  output logic [3:0]  D0_AN,
  output logic [7:0]  D0_SEG,
  output logic [3:0]  D1_AN,
  output logic [7:0]  D1_SEG,
  input  logic        UART_RXD,
  output logic        UART_TXD
);

  logic clk, rst;
  assign clk = CLK_100MHZ;

  // ------------------------------------------------------------------ UART
  logic [7:0] rx_data, tx_data;
  logic       rx_valid, rx_frame_err, rx_active;
  logic       tx_valid, tx_ready, tx_busy;

  uart_rx #(.DIV_OVERRIDE(DIV_OVERRIDE)) u_rx (
    .clk(clk), .rst(rst), .rxd(UART_RXD),
    .rx_data(rx_data), .rx_valid(rx_valid), .rx_frame_err(rx_frame_err), .rx_active(rx_active)
  );

  uart_tx #(.DIV_OVERRIDE(DIV_OVERRIDE)) u_tx (
    .clk(clk), .rst(rst), .tx_data(tx_data), .tx_valid(tx_valid), .tx_ready(tx_ready),
    .txd(UART_TXD), .tx_busy(tx_busy)
  );

  // ---------------------------------------------------------------- packet
  localparam int unsigned RW = $clog2(N_REGIONS);
  logic [RW-1:0]                  mem_region;
  logic [WBUF_AW-1:0]             mem_addr;
  logic [WBUF_WORD_BITS-1:0]      mem_wdata, mem_rdata;
  logic [WBUF_WORD_BYTES-1:0]     mem_be;
  logic                           mem_we, mem_re;
  logic [DESC_BYTES*8-1:0]        run_desc;
  logic                           run_valid, run_accept;
  logic                           seq_done;
  logic                           ctr_snap;
  logic [$clog2(N_COUNTERS)-1:0]  ctr_idx;
  logic [15:0]                    disp_util;
  logic [7:0]                     disp_mode;
  logic                           disp_valid;
  logic                           st_start, st_done;
  logic [4:0]                     dbg_state;

  packet #(.BUILD_ID(BUILD_ID), .RX_TIMEOUT_CYC(RX_TIMEOUT_CYC)) u_packet (
    .clk(clk), .rst(rst),
    .rx_data(rx_data), .rx_valid(rx_valid),
    .tx_data(tx_data), .tx_valid(tx_valid), .tx_ready(tx_ready),
    .mem_region(mem_region), .mem_addr(mem_addr), .mem_wdata(mem_wdata), .mem_be(mem_be),
    .mem_we(mem_we), .mem_re(mem_re), .mem_rdata(mem_rdata),
    .run_desc(run_desc), .run_valid(run_valid), .run_accept(run_accept),
    .seq_busy(1'b0), .seq_done(seq_done), .seq_error(1'b0),
    .ctr_snap(ctr_snap), .ctr_idx(ctr_idx), .ctr_data('0),
    .disp_util(disp_util), .disp_mode(disp_mode), .disp_valid(disp_valid),
    .st_start(st_start), .st_busy(1'b0), .st_done(st_done),
    .st_pass_full(1'b0), .st_pass_strip(1'b0), .st_fl_full(32'h0), .st_fl_strip(32'h0),
    .dbg_state(dbg_state)
  );

  // ---------------------------------------------------- sequencer stub (glue)
  assign run_accept = run_valid;
  always_ff @(posedge clk) begin
    if (rst)                          seq_done <= 1'b0;
    else if (run_valid && run_accept) seq_done <= 1'b1;
  end

  // ---------------------------------------------------- self-test stub (glue)
  always_ff @(posedge clk) begin
    if (rst) st_done <= 1'b0;
    else     st_done <= st_start;
  end

  // Unconnected outputs below (b_rdata, selftest_req, seg_img) are intentional.
  /* verilator lint_off PINCONNECTEMPTY */

  // ------------------------------------------- port-A memory mux (glue)
  // packet holds mem_region constant for a whole command, so the read-data mux
  // may use it directly (packet.sv header).
  logic [WBUF_WORD_BITS-1:0] wbuf_rd;
  logic [ABUF_WORD_BITS-1:0] abuf_rd;
  logic [OBUF_WORD_BITS-1:0] obuf_rd;
  logic [RQ_WORD_BITS-1:0]   rq_rd;
  logic [N_REGIONS-1:0]      sel;
  logic                      a_rvalid_unused [N_REGIONS];

  for (genvar r = 0; r < N_REGIONS; r++) begin : g_sel
    assign sel[r] = (mem_region == RW'(r));
  end

  buffers #(.WORDS(WBUF_WORDS), .WORD_BYTES(WBUF_WORD_BYTES)) u_wbuf (
    .clk(clk), .rst(rst),
    .a_en((mem_we || mem_re) && sel[RW'(REGION_WBUF)]),
    .a_we(mem_we ? mem_be[WBUF_WORD_BYTES-1:0] : '0),
    .a_addr(mem_addr[WBUF_AW-1:0]), .a_wdata(mem_wdata[WBUF_WORD_BITS-1:0]),
    .a_rdata(wbuf_rd), .a_rvalid(a_rvalid_unused[0]),
    .b_en(1'b0), .b_we('0), .b_addr('0), .b_wdata('0), .b_rdata()
  );

  buffers #(.WORDS(ABUF_WORDS), .WORD_BYTES(ABUF_WORD_BYTES)) u_abuf (
    .clk(clk), .rst(rst),
    .a_en((mem_we || mem_re) && sel[RW'(REGION_ABUF)]),
    .a_we(mem_we ? mem_be[ABUF_WORD_BYTES-1:0] : '0),
    .a_addr(mem_addr[ABUF_AW-1:0]), .a_wdata(mem_wdata[ABUF_WORD_BITS-1:0]),
    .a_rdata(abuf_rd), .a_rvalid(a_rvalid_unused[1]),
    .b_en(1'b0), .b_we('0), .b_addr('0), .b_wdata('0), .b_rdata()
  );

  buffers #(.WORDS(OBUF_WORDS), .WORD_BYTES(OBUF_WORD_BYTES)) u_obuf (
    .clk(clk), .rst(rst),
    .a_en((mem_we || mem_re) && sel[RW'(REGION_OBUF)]),
    .a_we(mem_we ? mem_be[OBUF_WORD_BYTES-1:0] : '0),
    .a_addr(mem_addr[OBUF_AW-1:0]), .a_wdata(mem_wdata[OBUF_WORD_BITS-1:0]),
    .a_rdata(obuf_rd), .a_rvalid(a_rvalid_unused[2]),
    .b_en(1'b0), .b_we('0), .b_addr('0), .b_wdata('0), .b_rdata()
  );

  buffers #(.WORDS(RQ_ENTRIES), .WORD_BYTES(RQ_WORD_BYTES)) u_rq (
    .clk(clk), .rst(rst),
    .a_en((mem_we || mem_re) && sel[RW'(REGION_RQPARAM)]),
    .a_we(mem_we ? mem_be[RQ_WORD_BYTES-1:0] : '0),
    .a_addr(mem_addr[RQ_AW-1:0]), .a_wdata(mem_wdata[RQ_WORD_BITS-1:0]),
    .a_rdata(rq_rd), .a_rvalid(a_rvalid_unused[3]),
    .b_en(1'b0), .b_we('0), .b_addr('0), .b_wdata('0), .b_rdata()
  );

  always_comb begin
    case (mem_region)
      RW'(REGION_WBUF): mem_rdata = wbuf_rd;
      RW'(REGION_ABUF): mem_rdata = WBUF_WORD_BITS'(abuf_rd);
      RW'(REGION_OBUF): mem_rdata = WBUF_WORD_BITS'(obuf_rd);
      default:          mem_rdata = WBUF_WORD_BITS'(rq_rd);
    endcase
  end

  // -------------------------------------------------------------- board I/O
  board_io #(
    .DEB_CYC(DEB_CYC), .HB_HALF_CYC(HB_HALF_CYC), .ACT_CYC(ACT_CYC), .SCAN_CYC(SCAN_CYC)
  ) u_board (
    .clk(clk), .btn(BTN[1:0]), .ext_rst_req(1'b0), .rst(rst), .selftest_req(),
    .pass_full(1'b0), .pass_strip(1'b0), .busy(1'b0), .uart_act(rx_active || tx_busy),
    .led(LED),
    .disp_util(disp_util), .disp_mode(disp_mode), .disp_valid(disp_valid),
    .d0_an(D0_AN), .d0_seg(D0_SEG), .d1_an(D1_AN), .d1_seg(D1_SEG), .seg_img()
  );

  /* verilator lint_on PINCONNECTEMPTY */

  // Outputs of production modules that this bring-up top does not need.
  /* verilator lint_off UNUSEDSIGNAL */
  logic unused_ok;
  assign unused_ok = ^{1'b0, run_desc, ctr_snap, ctr_idx, dbg_state, rx_frame_err, BTN[3:2],
                       a_rvalid_unused[0], a_rvalid_unused[1], a_rvalid_unused[2],
                       a_rvalid_unused[3]};
  /* verilator lint_on UNUSEDSIGNAL */

endmodule
