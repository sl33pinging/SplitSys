// =============================================================================
// top.sv — SplitSys top level for the RealDigital Urbana (SPEC §2, §3.8, §3.9)
// -----------------------------------------------------------------------------
// Pins (port names = fpga/xdc contract): CLK_100MHZ, BTN[3:0], LED[15:0],
//   D0_AN[3:0], D1_AN[3:0], D0_SEG[7:0], D1_SEG[7:0], UART_RXD (FPGA input),
//   UART_TXD (FPGA output).
//
// Structure
//   board_io  reset root (POR + debounced BTN1 -> `rst`), BTN0 -> self-test
//             request, LEDs, heartbeat, 7-segment display.
//   uart_rx / uart_tx / packet   host link (packet owns the memory bus and the
//             run interface unless the self-test is running).
//   selftest  ROM-driven FULL + STRIP check; owns the bus and run interface
//             while st_busy (packet answers BUSY to memory/run opcodes then).
//   counters  48-bit performance counters; start = core.ctr_start (the accept
//             cycle of every job, host or self-test), so each GEMM clears them.
//   core      rtl-designer A: buffers + sequencer + array + requant.
//
// Bus arbitration: a combinational 2:1 mux on st_busy in front of core. Both
//   masters' outputs are flops. BTN0 can only start the self-test while packet
//   is idle (hunting for a frame), so it never interrupts a WRITE_MEM/READ_MEM
//   in progress; packet's own SELFTEST opcode is always forwarded (packet has
//   already checked busy).
//
// Generics
//   BUILD_ID, ENABLE_STRIP     as in SPEC; ENABLE_STRIP = 0 is the cost build.
//   UART_DIV_OVERRIDE          0 in every hardware build (sim-only speed-up).
//   RX_TIMEOUT_CYC             packet inter-byte timeout (package default).
//   ROM_FILE                   self-test ROM image for $readmemh.
//   DEB_CYC, HB_HALF_CYC, ACT_CYC, SCAN_CYC, BTN_ACTIVE_LOW, SEG_ACTIVE_LOW,
//   AN_ACTIVE_LOW, LEFT_IS_D1, AN0_IS_RIGHT   passed to board_io.
//   BTN[3:2] are unused.
// =============================================================================
module top #(
  parameter logic [31:0] BUILD_ID          = 32'h0000_0000,
  parameter bit          ENABLE_STRIP      = splitsys_pkg::ENABLE_STRIP,
  parameter int unsigned UART_DIV_OVERRIDE = 0,
  parameter int unsigned RX_TIMEOUT_CYC    = splitsys_pkg::RX_TIMEOUT,
  parameter string       ROM_FILE          = "sw/crates/vec/out/selftest_rom.mem",
  parameter int unsigned DEB_CYC           = splitsys_pkg::CLK_HZ / 100,
  parameter int unsigned HB_HALF_CYC       = splitsys_pkg::CLK_HZ / 2,
  parameter int unsigned ACT_CYC           = splitsys_pkg::CLK_HZ / 20,
  parameter int unsigned SCAN_CYC          = splitsys_pkg::CLK_HZ / 1000,
  parameter bit          BTN_ACTIVE_LOW    = 1'b0,
  parameter bit          SEG_ACTIVE_LOW    = 1'b1,
  parameter bit          AN_ACTIVE_LOW     = 1'b1,
  parameter bit          LEFT_IS_D1        = 1'b1,
  parameter bit          AN0_IS_RIGHT      = 1'b1
) (
  input  logic        CLK_100MHZ,
  input  logic [3:0]  BTN,
  output logic [15:0] LED,
  output logic [3:0]  D0_AN,
  output logic [3:0]  D1_AN,
  output logic [7:0]  D0_SEG,
  output logic [7:0]  D1_SEG,
  input  logic        UART_RXD,
  output logic        UART_TXD
);

  import splitsys_pkg::N_REGIONS;
  import splitsys_pkg::WBUF_AW;
  import splitsys_pkg::WBUF_WORD_BITS;
  import splitsys_pkg::WBUF_WORD_BYTES;
  import splitsys_pkg::DESC_BYTES;
  import splitsys_pkg::N_COUNTERS;
  import splitsys_pkg::COUNTER_BITS;
  import splitsys_pkg::N_PES;
  import splitsys_pkg::N_QUADS;

  localparam int unsigned RW = $clog2(N_REGIONS);

  logic clk;
  logic rst;
  assign clk = CLK_100MHZ;

  // ------------------------------------------------------------------ wires
  // UART
  logic [7:0] rx_data, tx_data;
  logic       rx_valid, rx_ferr, rx_active, tx_valid, tx_ready, tx_busy;
  // packet master
  logic [RW-1:0]              p_region;
  logic [WBUF_AW-1:0]         p_addr;
  logic [WBUF_WORD_BITS-1:0]  p_wdata;
  logic [WBUF_WORD_BYTES-1:0] p_be;
  logic                       p_we, p_re;
  logic [DESC_BYTES*8-1:0]    p_desc;
  logic                       p_run_valid, p_run_accept;
  logic [4:0]                 pkt_state;
  // selftest master
  logic [RW-1:0]              s_region;
  logic [WBUF_AW-1:0]         s_addr;
  logic [WBUF_WORD_BITS-1:0]  s_wdata;
  logic [WBUF_WORD_BYTES-1:0] s_be;
  logic                       s_we, s_re;
  logic [DESC_BYTES*8-1:0]    s_desc;
  logic                       s_run_valid, s_run_accept;
  logic [4:0]                 st_state;
  // muxed master -> core
  logic [RW-1:0]              c_region;
  logic [WBUF_AW-1:0]         c_addr;
  logic [WBUF_WORD_BITS-1:0]  c_wdata;
  logic [WBUF_WORD_BYTES-1:0] c_be;
  logic                       c_we, c_re;
  logic [WBUF_WORD_BITS-1:0]  c_rdata;
  logic [DESC_BYTES*8-1:0]    c_desc;
  logic                       c_run_valid, c_run_accept;
  logic                       seq_busy, seq_done, seq_error;
  // counters
  logic                       ctr_snap;
  logic [$clog2(N_COUNTERS)-1:0] ctr_idx;
  logic [COUNTER_BITS-1:0]    ctr_data;
  logic                       ctr_running;
  logic                       tap_done, tap_setup, tap_stream, tap_flush, tap_drain, tap_stall, tap_tile;
  logic [$clog2(N_PES+1)-1:0] tap_mac;
  logic [N_QUADS*$clog2(N_PES/N_QUADS+1)-1:0] tap_mac_q;
  // self-test
  logic        st_start_pkt, st_start_btn, st_busy_w, st_done_w;
  logic        st_pass_full_w, st_pass_strip_w;
  logic [31:0] st_fl_full_w, st_fl_strip_w;
  // display
  logic [15:0] disp_util;
  logic [7:0]  disp_mode;
  logic        disp_valid;
  logic [63:0] seg_img;

  // ---------------------------------------------------------------- board
  board_io #(
    .DEB_CYC(DEB_CYC), .HB_HALF_CYC(HB_HALF_CYC), .ACT_CYC(ACT_CYC), .SCAN_CYC(SCAN_CYC),
    .BTN_ACTIVE_LOW(BTN_ACTIVE_LOW), .SEG_ACTIVE_LOW(SEG_ACTIVE_LOW), .AN_ACTIVE_LOW(AN_ACTIVE_LOW),
    .LEFT_IS_D1(LEFT_IS_D1), .AN0_IS_RIGHT(AN0_IS_RIGHT)
  ) u_board (
    .clk, .btn(BTN[1:0]), .ext_rst_req(1'b0), .rst, .selftest_req(st_start_btn),
    .pass_full(st_pass_full_w), .pass_strip(st_pass_strip_w), .busy(seq_busy || st_busy_w),
    .uart_act(rx_active || tx_busy), .led(LED),
    .disp_util, .disp_mode, .disp_valid,
    .d0_an(D0_AN), .d0_seg(D0_SEG), .d1_an(D1_AN), .d1_seg(D1_SEG), .seg_img
  );

  // ------------------------------------------------------------ host link
  uart_rx #(.DIV_OVERRIDE(UART_DIV_OVERRIDE)) u_rx (
    .clk, .rst, .rxd(UART_RXD), .rx_data, .rx_valid, .rx_frame_err(rx_ferr), .rx_active
  );
  uart_tx #(.DIV_OVERRIDE(UART_DIV_OVERRIDE)) u_tx (
    .clk, .rst, .tx_data, .tx_valid, .tx_ready, .txd(UART_TXD), .tx_busy
  );

  packet #(.BUILD_ID(BUILD_ID), .STRIP_EN(ENABLE_STRIP), .RX_TIMEOUT_CYC(RX_TIMEOUT_CYC)) u_packet (
    .clk, .rst, .rx_data, .rx_valid, .tx_data, .tx_valid, .tx_ready,
    .mem_region(p_region), .mem_addr(p_addr), .mem_wdata(p_wdata), .mem_be(p_be),
    .mem_we(p_we), .mem_re(p_re), .mem_rdata(c_rdata),
    .run_desc(p_desc), .run_valid(p_run_valid), .run_accept(p_run_accept),
    .seq_busy, .seq_done, .seq_error,
    .ctr_snap, .ctr_idx, .ctr_data,
    .disp_util, .disp_mode, .disp_valid,
    .st_start(st_start_pkt), .st_busy(st_busy_w), .st_done(st_done_w),
    .st_pass_full(st_pass_full_w), .st_pass_strip(st_pass_strip_w),
    .st_fl_full(st_fl_full_w), .st_fl_strip(st_fl_strip_w),
    .dbg_state(pkt_state)
  );

  // ------------------------------------------------------------- self-test
  // BTN0 starts the self-test only while packet hunts for a frame (state 0).
  selftest #(.ROM_FILE(ROM_FILE)) u_selftest (
    .clk, .rst, .start(st_start_pkt || (st_start_btn && (pkt_state == 5'd0))),
    .busy(st_busy_w), .done(st_done_w),
    .pass_full(st_pass_full_w), .pass_strip(st_pass_strip_w),
    .fl_full(st_fl_full_w), .fl_strip(st_fl_strip_w),
    .mem_region(s_region), .mem_addr(s_addr), .mem_wdata(s_wdata), .mem_be(s_be),
    .mem_we(s_we), .mem_re(s_re), .mem_rdata(c_rdata),
    .run_desc(s_desc), .run_valid(s_run_valid), .run_accept(s_run_accept),
    .seq_busy, .seq_error, .dbg_state(st_state)
  );

  // ------------------------------------------------------------ arbitration
  assign c_region     = st_busy_w ? s_region    : p_region;
  assign c_addr       = st_busy_w ? s_addr      : p_addr;
  assign c_wdata      = st_busy_w ? s_wdata     : p_wdata;
  assign c_be         = st_busy_w ? s_be        : p_be;
  assign c_we         = st_busy_w ? s_we        : p_we;
  assign c_re         = st_busy_w ? s_re        : p_re;
  assign c_desc       = st_busy_w ? s_desc      : p_desc;
  assign c_run_valid  = st_busy_w ? s_run_valid : p_run_valid;
  assign s_run_accept =  st_busy_w && c_run_accept;
  assign p_run_accept = !st_busy_w && c_run_accept;

  // --------------------------------------------------------------- counters
  counters u_counters (
    .clk, .rst, .start(tap_start), .done(tap_done),
    .tap_setup, .tap_stream, .tap_flush, .tap_drain, .tap_stall,
    .mac_cnt(tap_mac), .mac_q_cnt(tap_mac_q), .tile_done(tap_tile),
    .snap(ctr_snap), .rd_idx(ctr_idx), .rd_data(ctr_data), .running(ctr_running)
  );

  // ------------------------------------------------------------------- core
  logic [2:0] core_state;
  logic       core_strip;
  logic       tap_start;

  core #(.STRIP_EN(ENABLE_STRIP)) u_core (
    .clk, .rst,
    .mem_region(c_region), .mem_addr(c_addr), .mem_wdata(c_wdata), .mem_be(c_be),
    .mem_we(c_we), .mem_re(c_re), .mem_rdata(c_rdata),
    .run_desc(c_desc), .run_valid(c_run_valid), .run_accept(c_run_accept),
    .seq_busy, .seq_done, .seq_error,
    .ctr_start(tap_start), .ctr_done(tap_done),
    .tap_setup, .tap_stream, .tap_flush, .tap_drain, .tap_stall,
    .mac_cnt(tap_mac), .mac_q_cnt(tap_mac_q), .tile_done(tap_tile),
    .dbg_state(core_state), .dbg_strip(core_strip)
  );

  // Probe-only signals (testbenches read them hierarchically) and unused pins.
  // Lint skips signals named unused_* (the default -unused-regexp of the linter).
  logic unused_probe;
  assign unused_probe = ^{BTN[3:2], rx_ferr, st_state, ctr_running, seg_img, core_state, core_strip};

endmodule
