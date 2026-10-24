// =============================================================================
// packet.sv — host-link framing, CRC-8, command dispatch, memory mux (SPEC §3.8)
// -----------------------------------------------------------------------------
// Wire format (all multi-byte fields little-endian)
//   request : SYNC_REQ(0xA5) | opcode u8 | len u16 | payload[len] | crc8
//   response: SYNC_RSP(0x5A) | status u8 | len u16 | payload[len] | crc8
//   crc8 = CRC-8 poly CRC8_POLY (0x07), init CRC8_INIT (0x00), MSB first, no
//   reflection, no xorout, over every byte after the sync byte up to and
//   including the payload.
//
// Protocol behaviour
//   * Strict request/response: one request is parsed, executed, and answered
//     before the next one is parsed. Bytes that arrive while a command is
//     executing or its response is being sent are discarded. A byte other than
//     SYNC_REQ while hunting for a frame is ignored.
//   * RX_TIMEOUT_CYC (default: package RX_TIMEOUT; testbenches may lower it)
//     idle cycles between two bytes of one frame reset the parser silently.
//   * len > MAX_PAYLOAD (D-010): the frame is not buffered; the parser drops
//     every byte until the line has been idle for RX_TIMEOUT_CYC cycles, then
//     answers NACK_RANGE.
//   * The whole payload is buffered and the CRC verified BEFORE any side
//     effect: a NACK_CRC frame never writes memory or starts a job.
//   * Error precedence (SPEC §3.8, D-010), all error responses have len 0:
//       bad CRC                                     -> NACK_CRC
//       unknown opcode                              -> NACK_OPCODE
//       busy (RUN_GEMM/SELFTEST/WRITE_MEM/READ_MEM) -> BUSY
//       wrong payload length, or field out of range -> NACK_RANGE
//       otherwise execute                           -> ACK (+ payload)
//   * busy = seq_busy || st_busy || (a RUN_GEMM still waiting for accept).
//     PING, POLL, READ_COUNTERS, SET_DISPLAY are always served.
//   * Payload lengths: PING 0, WRITE_MEM 6..MAX_PAYLOAD (1..1019 data bytes),
//     READ_MEM 7, RUN_GEMM DESC_BYTES, POLL 0, READ_COUNTERS 0, SELFTEST 0,
//     SET_DISPLAY 3.
//   * Ranges: region < N_REGIONS and byte_addr + count <= region bytes for
//     WRITE_MEM/READ_MEM; READ_MEM 1 <= rlen <= MAX_PAYLOAD; SET_DISPLAY
//     util_permille <= 1000 and mode <= 1. RUN_GEMM (SPEC §3.5, D-005), with
//     MB = ceil(M/8), M_pad = 8*MB, NB = ceil(N/16): mode FULL, or STRIP if
//     STRIP_EN; flags == FLAGS_INT8 or FLAGS_INT32; M,K,N in [*_MIN, *_MAX];
//     reserved == 0;
//       a_base   + MB*K                                <= ABUF_WORDS
//       w_base   + NB*K                                <= WBUF_WORDS
//       out_base + NB*M_pad*(int32 ? INT32_WORDS_PER_ROW : 1) <= OBUF_WORDS
//       rq_base  + N                                   <= RQ_ENTRIES
//     All sums are 33-bit, so a u32 base can never wrap.
//
// Interfaces (all synchronous to clk)
//   UART side: rx_data/rx_valid from uart_rx; tx_data/tx_valid/tx_ready to
//   uart_tx (byte taken when tx_valid && tx_ready).
//
//   Memory bus (to buffers port A, muxed by `top`):
//     mem_region  region index (REGION_*); registered; constant for the whole
//                 command, so top may mux mem_rdata combinationally by it.
//     mem_addr    WORD address in units of that region's word size.
//     mem_wdata   byte lane i at [8*i +: 8]; narrow regions use the low lanes
//                 (ABUF lanes 0..7, RQPARAM lanes 0..3).
//     mem_be      byte enables for mem_we; partial and unaligned words are
//                 written with byte enables only (never read-modify-write).
//     mem_we      1-cycle write strobe.
//     mem_re      1-cycle read request; mem_rdata must hold that word in the
//                 cycle exactly L_RD cycles later (L_RD from the package).
//     All mem_* outputs are flops. Writes: at most one per 2 cycles.
//     Reads: at most one outstanding.
//
//   Sequencer:
//     run_desc    the 24 descriptor bytes, byte i at [8*i +: 8]. Registered;
//                 stable from run_valid until the next RUN_GEMM is decided.
//     run_valid   held high until run_accept (valid/ready handshake). Only
//                 raised when not busy and the descriptor is legal.
//     run_accept  sequencer takes the job in a cycle with run_valid && run_accept.
//                 RUN_GEMM is ACKed after the accept.
//     seq_busy / seq_done / seq_error   levels; POLL returns
//                 seq_busy|st_busy -> POLL_BUSY, else error -> POLL_ERROR,
//                 else done -> POLL_DONE, else POLL_IDLE.
//
//   Counters (counters.sv): ctr_snap 1-cycle pulse when READ_COUNTERS is
//     decided; ctr_idx registered counter index; ctr_data must be valid at most
//     L_RD cycles after ctr_idx changes (counters.sv: 1 cycle).
//   Display: disp_util/disp_mode registered, disp_valid 1-cycle pulse on update.
//   Self-test: st_start 1-cycle pulse; the engine MUST answer with a st_done
//     pulse (results valid in that cycle), or the link waits forever.
//   dbg_state: FSM state, for testbench probing.
//
// Latency (requests are UART-bound; these only matter for co-sim speed)
//   RX byte -> state update 1 cycle. Good CRC -> decision 3 cycles (two
//   range-check calc stages). Response bytes: header back-to-back through the
//   uart_tx holding register; each payload byte needs 2 + L_RD cycles of fetch,
//   which overlaps the previous byte's shifting. WRITE_MEM executes at
//   2 cycles per data byte before the ACK.
//
// Critical paths (estimates, -2 Spartan-7)
//   CRC-8 byte update: each output bit is an XOR of <= 8 of the 16 inputs
//     (crc ^ data), 2 LUT levels; registered.
//   Range checks: split over S_CALC1 (small LUT multiplies 4x9, 5x9, 5x7) ->
//     S_CALC2 (33-bit adds, CARRY4 chain) -> S_DECIDE (33-bit compares).
//   Payload byte mux: 16:1 byte lane select from mem_rdata, 1-2 LUT levels.
//
// Resources: one 1 KiB payload buffer (inferred block RAM, 8 bits x
//   MAX_PAYLOAD), ~600 flops.
// =============================================================================
module packet
  import splitsys_pkg::*;
#(
  parameter logic [31:0] BUILD_ID            = 32'h0000_0000,
  parameter bit          STRIP_EN            = ENABLE_STRIP,
  parameter int unsigned RX_TIMEOUT_CYC      = RX_TIMEOUT  // inter-byte timeout; tests may lower it
) (
  input  logic                          clk,
  input  logic                          rst,
  // UART byte streams
  input  logic [7:0]                    rx_data,
  input  logic                          rx_valid,
  output logic [7:0]                    tx_data,
  output logic                          tx_valid,
  input  logic                          tx_ready,
  // memory bus -> buffers port A (via top)
  output logic [$clog2(N_REGIONS)-1:0]  mem_region,
  output logic [WBUF_AW-1:0]            mem_addr,
  output logic [WBUF_WORD_BITS-1:0]     mem_wdata,
  output logic [WBUF_WORD_BYTES-1:0]    mem_be,
  output logic                          mem_we,
  output logic                          mem_re,
  input  logic [WBUF_WORD_BITS-1:0]     mem_rdata,
  // sequencer
  output logic [DESC_BYTES*8-1:0]       run_desc,
  output logic                          run_valid,
  input  logic                          run_accept,
  input  logic                          seq_busy,
  input  logic                          seq_done,
  input  logic                          seq_error,
  // counters
  output logic                          ctr_snap,
  output logic [$clog2(N_COUNTERS)-1:0] ctr_idx,
  input  logic [COUNTER_BITS-1:0]       ctr_data,
  // board display
  output logic [15:0]                   disp_util,
  output logic [7:0]                    disp_mode,
  output logic                          disp_valid,
  // self-test engine
  output logic                          st_start,
  input  logic                          st_busy,
  input  logic                          st_done,
  input  logic                          st_pass_full,
  input  logic                          st_pass_strip,
  input  logic [31:0]                   st_fl_full,
  input  logic [31:0]                   st_fl_strip,
  // observability
  output logic [4:0]                    dbg_state
);

  // ------------------------------------------------------------ constants
  localparam int unsigned TIMEOUT = RX_TIMEOUT_CYC;
  localparam int unsigned TO_W    = $clog2(TIMEOUT + 1);
  localparam int unsigned RW      = $clog2(N_REGIONS);
  localparam int unsigned BA_W    = $clog2(WBUF_BYTES) + 1;   // byte address within a region (+1 for end)
  localparam int unsigned PB_W    = $clog2(MAX_PAYLOAD);      // payload buffer index
  localparam int unsigned LANES   = WBUF_WORD_BYTES;
  localparam int unsigned LN_W    = $clog2(LANES);
  localparam int unsigned CI_W    = $clog2(N_COUNTERS);
  localparam int unsigned CB_W    = $clog2(COUNTER_BYTES);
  localparam int unsigned WR_OFF  = 5;                        // WRITE_MEM data starts at payload byte 5
  localparam int unsigned SMALL_N = 10;                       // largest fixed payload (SELFTEST)
  localparam int unsigned PING_LEN     = 9;
  localparam int unsigned POLL_LEN     = 1;
  localparam int unsigned SELFTEST_LEN = 10;
  localparam int unsigned CTR_LEN      = N_COUNTERS * COUNTER_BYTES;
  localparam int unsigned FW_W    = $clog2(L_RD + 1);

  // Elaboration checks: the bus is sized by WBUF, which must be the widest/largest.
  if (WBUF_WORD_BYTES < ABUF_WORD_BYTES || WBUF_WORD_BYTES < OBUF_WORD_BYTES ||
      WBUF_WORD_BYTES < RQ_WORD_BYTES || WBUF_BYTES < ABUF_BYTES ||
      WBUF_BYTES < OBUF_BYTES || WBUF_BYTES < RQ_BYTES ||
      WBUF_AW < ABUF_AW || WBUF_AW < OBUF_AW || WBUF_AW < RQ_AW) begin : g_chk_wbuf
    $error("packet: WBUF must be the widest and largest region");
  end
  if (L_RD < 1) begin : g_chk_lrd
    $error("packet: L_RD must be >= 1");
  end
  if (N_REGIONS != 4) begin : g_chk_regions
    $error("packet: region table below assumes 4 regions");
  end
  if (TIMEOUT < 2) begin : g_chk_timeout
    $error("packet: RX_TIMEOUT_CYC must be >= 2");
  end

  // --------------------------------------------------------------- helpers
  function automatic logic [7:0] crc8_byte(input logic [7:0] crc, input logic [7:0] d);
    logic [7:0] c;
    c = crc ^ d;
    for (int i = 0; i < 8; i++) c = c[7] ? ((c << 1) ^ CRC8_POLY) : (c << 1);
    return c;
  endfunction

  function automatic logic [BA_W-1:0] region_bytes(input logic [RW-1:0] r);
    case (r)
      RW'(REGION_WBUF): return BA_W'(WBUF_BYTES);
      RW'(REGION_ABUF): return BA_W'(ABUF_BYTES);
      RW'(REGION_OBUF): return BA_W'(OBUF_BYTES);
      default:          return BA_W'(RQ_BYTES);
    endcase
  endfunction

  // log2 of the region's word size in bytes
  function automatic logic [2:0] region_shift(input logic [RW-1:0] r);
    case (r)
      RW'(REGION_WBUF): return 3'($clog2(WBUF_WORD_BYTES));
      RW'(REGION_ABUF): return 3'($clog2(ABUF_WORD_BYTES));
      RW'(REGION_OBUF): return 3'($clog2(OBUF_WORD_BYTES));
      default:          return 3'($clog2(RQ_WORD_BYTES));
    endcase
  endfunction

  // ------------------------------------------------------------ state
  typedef enum logic [4:0] {
    S_SYNC, S_OP, S_LEN0, S_LEN1, S_PAY, S_CRC,      // receive
    S_DROP,                                           // oversize frame: drop until idle
    S_CALC1, S_CALC2, S_DECIDE,                       // decode
    S_WR_RD, S_WR_ACC,                                // WRITE_MEM loop
    S_RUN, S_SELFTEST,                                // wait on other blocks
    S_FETCH, S_FWAIT, S_TX                            // respond
  } state_t;

  typedef enum logic [2:0] {PH_SYNC, PH_STAT, PH_LEN0, PH_LEN1, PH_PAY, PH_CRC} phase_t;
  typedef enum logic [1:0] {SRC_SMALL, SRC_MEM, SRC_CTR} src_t;

  state_t state;
  phase_t tx_phase;
  src_t   src;

  assign dbg_state = state;

  // receive-side registers
  logic [7:0]      op;
  logic [15:0]     len;
  logic [15:0]     idx;            // payload bytes received so far
  logic [7:0]      rx_crc;
  logic [7:0]      pl [DESC_BYTES]; // first DESC_BYTES payload bytes
  logic [TO_W-1:0] to_cnt;

  // decoded payload fields (combinational views of pl)
  logic [7:0]  f_region;
  logic [31:0] f_addr;
  logic [15:0] f_rdlen;
  logic [15:0] f_m, f_k, f_n, f_rq_base, f_resv;
  logic [31:0] f_a_base, f_w_base, f_o_base;
  logic [7:0]  f_mode, f_flags;

  assign f_region  = pl[0];
  assign f_addr    = {pl[4], pl[3], pl[2], pl[1]};
  assign f_rdlen   = {pl[6], pl[5]};
  assign f_mode    = pl[DESC_OFF_MODE];
  assign f_flags   = pl[DESC_OFF_FLAGS];
  assign f_m       = {pl[DESC_OFF_M+1],        pl[DESC_OFF_M]};
  assign f_k       = {pl[DESC_OFF_K+1],        pl[DESC_OFF_K]};
  assign f_n       = {pl[DESC_OFF_N+1],        pl[DESC_OFF_N]};
  assign f_a_base  = {pl[DESC_OFF_A_BASE+3],   pl[DESC_OFF_A_BASE+2],
                      pl[DESC_OFF_A_BASE+1],   pl[DESC_OFF_A_BASE]};
  assign f_w_base  = {pl[DESC_OFF_W_BASE+3],   pl[DESC_OFF_W_BASE+2],
                      pl[DESC_OFF_W_BASE+1],   pl[DESC_OFF_W_BASE]};
  assign f_o_base  = {pl[DESC_OFF_OUT_BASE+3], pl[DESC_OFF_OUT_BASE+2],
                      pl[DESC_OFF_OUT_BASE+1], pl[DESC_OFF_OUT_BASE]};
  assign f_rq_base = {pl[DESC_OFF_RQ_BASE+1],  pl[DESC_OFF_RQ_BASE]};
  assign f_resv    = {pl[DESC_OFF_RESERVED+1], pl[DESC_OFF_RESERVED]};

  // range-check pipeline registers
  logic        c_mode_ok, c_flags_ok, c_dims_ok, c_resv_ok, c_region_ok;
  logic [12:0] c_pa;              // ceil(M/8)*K            <= 8*256
  logic [13:0] c_pw;              // ceil(N/16)*K           <= 16*256
  logic [13:0] c_po;              // NB*M_pad*(1|INT32_WORDS_PER_ROW) <= 16*64*4
  logic [8:0]  c_prq;             // N                       <= 256
  logic [15:0] c_cnt;             // WRITE_MEM data bytes / READ_MEM len
  logic [32:0] c_a_end, c_w_end, c_o_end, c_rq_end, c_mem_end;

  // execute / respond registers
  logic [7:0]      status;
  logic [15:0]     rsp_len;
  logic [15:0]     pay_idx;
  logic [7:0]      spay [SMALL_N]; // fixed-size response payload (PING, POLL, SELFTEST)
  logic [BA_W-1:0] cur_addr;       // running byte address within mem_region
  logic [15:0]     wcnt;           // WRITE_MEM data bytes done
  logic [15:0]     dlen;           // WRITE_MEM data bytes total
  logic [WBUF_WORD_BITS-1:0]  wacc;
  logic [WBUF_WORD_BYTES-1:0] wbe;
  logic [CI_W-1:0] cidx;
  logic [CB_W-1:0] cbyte;
  logic [FW_W-1:0] fwait;
  logic [7:0]      tx_byte;
  logic [7:0]      tx_crc;

  // ------------------------------------------------------- payload buffer
  // Simple dual-port block RAM: written while receiving, read by WRITE_MEM.
  (* ram_style = "block" *) logic [7:0] pbuf [MAX_PAYLOAD];
  logic [PB_W-1:0] pb_raddr;
  logic [7:0]      pb_q;
  logic            pb_we;

  assign pb_we    = (state == S_PAY) && rx_valid && (idx < 16'(MAX_PAYLOAD));
  assign pb_raddr = PB_W'(WR_OFF) + wcnt[PB_W-1:0];

  always_ff @(posedge clk) begin
    if (pb_we) pbuf[idx[PB_W-1:0]] <= rx_data;
  end

  always_ff @(posedge clk) begin
    if (rst) pb_q <= '0;
    else     pb_q <= pbuf[pb_raddr];
  end

  // ------------------------------------------------- combinational helpers
  logic [7:0]                 crc_rx_upd, crc_tx_upd;
  logic [RW-1:0]              cur_region;
  logic [2:0]                 cur_shift;
  logic [LN_W-1:0]            lane_mask, lane;
  logic [WBUF_WORD_BITS-1:0]  wacc_n;
  logic [WBUF_WORD_BYTES-1:0] wbe_n;
  logic [7:0]                 fetched;
  logic [7:0]                 poll_byte;

  assign crc_rx_upd = crc8_byte(rx_crc, rx_data);
  assign crc_tx_upd = crc8_byte(tx_crc, tx_byte);
  assign cur_region = mem_region;
  assign cur_shift  = region_shift(cur_region);
  assign lane_mask  = LN_W'((1 << cur_shift) - 1);
  assign lane       = cur_addr[LN_W-1:0] & lane_mask;

  always_comb begin
    wacc_n                = wacc;
    wacc_n[8*lane +: 8]   = pb_q;
    wbe_n                 = wbe;
    wbe_n[lane]           = 1'b1;
  end

  always_comb begin
    case (src)
      SRC_MEM: fetched = mem_rdata[8*lane +: 8];
      SRC_CTR: fetched = ctr_data[8*cbyte +: 8];
      default: fetched = (pay_idx < 16'(SMALL_N)) ? spay[pay_idx[3:0]] : 8'h00;
    endcase
  end

  // BUSY applies to RUN_GEMM, SELFTEST, WRITE_MEM, READ_MEM (D-010)
  logic busy;
  assign busy = seq_busy || st_busy || run_valid;

  always_comb begin
    if      (seq_busy || st_busy) poll_byte = POLL_BUSY;
    else if (seq_error) poll_byte = POLL_ERROR;
    else if (seq_done)  poll_byte = POLL_DONE;
    else                poll_byte = POLL_IDLE;
  end

  assign tx_valid = (state == S_TX);
  assign tx_data  = tx_byte;

  // receive states in which the inter-byte timeout applies
  logic in_frame;
  assign in_frame = (state == S_OP) || (state == S_LEN0) || (state == S_LEN1) ||
                    (state == S_PAY) || (state == S_CRC) || (state == S_DROP);

  // ----------------------------------------------------- inter-byte timeout
  always_ff @(posedge clk) begin
    if (rst || !in_frame || rx_valid) to_cnt <= '0;
    else                              to_cnt <= to_cnt + TO_W'(1);
  end

  // ------------------------------------------------------- range-check calc
  // S_CALC1: small products and flag checks. S_CALC2: 33-bit end addresses.
  logic [3:0] mb;     // ceil(M/8), valid when M <= 64
  logic [4:0] nb;     // ceil(N/16), valid when N <= 256
  assign mb = 4'((7'(f_m[6:0]) + 7'd7) >> 3);
  assign nb = 5'((9'(f_n[8:0]) + 9'd15) >> 4);

  always_ff @(posedge clk) begin
    if (rst) begin
      c_mode_ok <= 1'b0; c_flags_ok <= 1'b0; c_dims_ok <= 1'b0; c_resv_ok <= 1'b0;
      c_region_ok <= 1'b0; c_pa <= '0; c_pw <= '0; c_po <= '0; c_prq <= '0; c_cnt <= '0;
    end else if (state == S_CALC1) begin
      c_mode_ok   <= (f_mode == MODE_FULL) || (STRIP_EN && (f_mode == MODE_STRIP));
      c_flags_ok  <= (f_flags == FLAGS_INT8) || (f_flags == FLAGS_INT32);
      c_dims_ok   <= (f_m >= 16'(M_MIN)) && (f_m <= 16'(M_MAX)) &&
                     (f_k >= 16'(K_MIN)) && (f_k <= 16'(K_MAX)) &&
                     (f_n >= 16'(N_MIN)) && (f_n <= 16'(N_MAX));
      c_resv_ok   <= (f_resv == 16'd0);
      c_region_ok <= (f_region < 8'(N_REGIONS));
      c_pa        <= 13'(mb) * 13'(f_k[8:0]);
      c_pw        <= 14'(nb) * 14'(f_k[8:0]);
      c_po        <= 14'(nb) * 14'({mb, 3'b000}) *
                     ((f_flags == FLAGS_INT32) ? 14'(INT32_WORDS_PER_ROW) : 14'd1);
      c_prq       <= f_n[8:0];
      c_cnt       <= (op == OP_WRITE_MEM) ? (len - 16'(WR_OFF)) : f_rdlen;
    end
  end

  always_ff @(posedge clk) begin
    if (rst) begin
      c_a_end <= '0; c_w_end <= '0; c_o_end <= '0; c_rq_end <= '0; c_mem_end <= '0;
    end else if (state == S_CALC2) begin
      c_a_end   <= 33'(f_a_base)  + 33'(c_pa);
      c_w_end   <= 33'(f_w_base)  + 33'(c_pw);
      c_o_end   <= 33'(f_o_base)  + 33'(c_po);
      c_rq_end  <= 33'(f_rq_base) + 33'(c_prq);
      c_mem_end <= 33'(f_addr)    + 33'(c_cnt);
    end
  end

  logic desc_ok, mem_ok;
  assign desc_ok = c_mode_ok && c_flags_ok && c_dims_ok && c_resv_ok &&
                   (c_a_end  <= 33'(ABUF_WORDS)) && (c_w_end  <= 33'(WBUF_WORDS)) &&
                   (c_o_end  <= 33'(OBUF_WORDS)) && (c_rq_end <= 33'(RQ_ENTRIES));
  assign mem_ok  = c_region_ok &&
                   (c_mem_end <= 33'(region_bytes(f_region[RW-1:0])));

  // ------------------------------------------------------------ main FSM
  task automatic respond(input logic [7:0] st, input logic [15:0] n, input src_t s);
    status   <= st;
    rsp_len  <= n;
    src      <= s;
    pay_idx  <= '0;
    tx_crc   <= CRC8_INIT;
    tx_byte  <= SYNC_RSP;
    tx_phase <= PH_SYNC;
    state    <= S_TX;
  endtask

  always_ff @(posedge clk) begin
    if (rst) begin
      state      <= S_SYNC;
      tx_phase   <= PH_SYNC;
      src        <= SRC_SMALL;
      op         <= '0;
      len        <= '0;
      idx        <= '0;
      rx_crc     <= '0;
      for (int i = 0; i < DESC_BYTES; i++) pl[i] <= '0;
      for (int i = 0; i < SMALL_N; i++)    spay[i] <= '0;
      status     <= '0;
      rsp_len    <= '0;
      pay_idx    <= '0;
      cur_addr   <= '0;
      wcnt       <= '0;
      dlen       <= '0;
      wacc       <= '0;
      wbe        <= '0;
      cidx       <= '0;
      cbyte      <= '0;
      fwait      <= '0;
      tx_byte    <= '0;
      tx_crc     <= '0;
      mem_region <= '0;
      mem_addr   <= '0;
      mem_wdata  <= '0;
      mem_be     <= '0;
      mem_we     <= 1'b0;
      mem_re     <= 1'b0;
      run_desc   <= '0;
      run_valid  <= 1'b0;
      ctr_snap   <= 1'b0;
      ctr_idx    <= '0;
      disp_util  <= '0;
      disp_mode  <= '0;
      disp_valid <= 1'b0;
      st_start   <= 1'b0;
    end else begin
      // single-cycle strobes default low
      mem_we     <= 1'b0;
      mem_re     <= 1'b0;
      ctr_snap   <= 1'b0;
      disp_valid <= 1'b0;
      st_start   <= 1'b0;

      if (in_frame && !rx_valid && (to_cnt == TO_W'(TIMEOUT - 1))) begin
        // inter-byte timeout: drop the frame silently, or answer an oversize one
        if (state == S_DROP) respond(ST_NACK_RANGE, 16'd0, SRC_SMALL);
        else                 state <= S_SYNC;
      end else begin
        case (state)
          // ------------------------------------------------------ receive
          S_SYNC: if (rx_valid && rx_data == SYNC_REQ) begin
            rx_crc <= CRC8_INIT;
            state  <= S_OP;
          end
          S_OP: if (rx_valid) begin
            op     <= rx_data;
            rx_crc <= crc_rx_upd;
            state  <= S_LEN0;
          end
          S_LEN0: if (rx_valid) begin
            len[7:0] <= rx_data;
            rx_crc   <= crc_rx_upd;
            state    <= S_LEN1;
          end
          S_LEN1: if (rx_valid) begin
            len[15:8] <= rx_data;
            rx_crc    <= crc_rx_upd;
            idx       <= '0;
            for (int i = 0; i < DESC_BYTES; i++) pl[i] <= '0;
            if ({rx_data, len[7:0]} > 16'(MAX_PAYLOAD)) state <= S_DROP;
            else if ({rx_data, len[7:0]} == 16'd0)      state <= S_CRC;
            else                                         state <= S_PAY;
          end
          S_PAY: if (rx_valid) begin
            if (idx < 16'(DESC_BYTES)) pl[idx[$clog2(DESC_BYTES)-1:0]] <= rx_data;
            rx_crc <= crc_rx_upd;
            idx    <= idx + 16'd1;
            if (idx == len - 16'd1) state <= S_CRC;
          end
          S_CRC: if (rx_valid) begin
            if (rx_data == rx_crc) state <= S_CALC1;
            else                   respond(ST_NACK_CRC, 16'd0, SRC_SMALL);
          end
          S_DROP: ;                              // left only through the timeout above

          // ------------------------------------------------------- decode
          // Precedence: (CRC already checked) OPCODE > BUSY > RANGE (length first).
          S_CALC1: state <= S_CALC2;
          S_CALC2: state <= S_DECIDE;
          S_DECIDE: begin
            case (op)
              OP_PING: begin
                if (len != 16'd0) respond(ST_NACK_RANGE, 16'd0, SRC_SMALL);
                else begin
                  for (int i = 0; i < 4; i++) spay[i]     <= PING_MAGIC_LE[8*i +: 8];
                  spay[4] <= 8'(PROTO_VERSION);
                  for (int i = 0; i < 4; i++) spay[5 + i] <= BUILD_ID[8*i +: 8];
                  respond(ST_ACK, 16'(PING_LEN), SRC_SMALL);
                end
              end
              OP_WRITE_MEM: begin
                if (busy)                           respond(ST_BUSY, 16'd0, SRC_SMALL);
                else if (len <= 16'(WR_OFF) || !mem_ok)
                                                    respond(ST_NACK_RANGE, 16'd0, SRC_SMALL);
                else begin
                  mem_region <= f_region[RW-1:0];
                  cur_addr   <= f_addr[BA_W-1:0];
                  dlen       <= c_cnt;
                  wcnt       <= '0;
                  wacc       <= '0;
                  wbe        <= '0;
                  state      <= S_WR_RD;
                end
              end
              OP_READ_MEM: begin
                if (busy)                           respond(ST_BUSY, 16'd0, SRC_SMALL);
                else if (len != 16'd7 || !mem_ok || f_rdlen == 16'd0 ||
                         f_rdlen > 16'(MAX_PAYLOAD)) respond(ST_NACK_RANGE, 16'd0, SRC_SMALL);
                else begin
                  mem_region <= f_region[RW-1:0];
                  cur_addr   <= f_addr[BA_W-1:0];
                  respond(ST_ACK, f_rdlen, SRC_MEM);
                end
              end
              OP_RUN_GEMM: begin
                if (busy)                                     respond(ST_BUSY, 16'd0, SRC_SMALL);
                else if (len != 16'(DESC_BYTES) || !desc_ok)  respond(ST_NACK_RANGE, 16'd0, SRC_SMALL);
                else begin
                  for (int i = 0; i < DESC_BYTES; i++) run_desc[8*i +: 8] <= pl[i];
                  run_valid <= 1'b1;
                  state     <= S_RUN;
                end
              end
              OP_POLL: begin
                if (len != 16'd0) respond(ST_NACK_RANGE, 16'd0, SRC_SMALL);
                else begin
                  spay[0] <= poll_byte;
                  respond(ST_ACK, 16'(POLL_LEN), SRC_SMALL);
                end
              end
              OP_READ_COUNTERS: begin
                if (len != 16'd0) respond(ST_NACK_RANGE, 16'd0, SRC_SMALL);
                else begin
                  ctr_snap <= 1'b1;
                  cidx     <= '0;
                  cbyte    <= '0;
                  respond(ST_ACK, 16'(CTR_LEN), SRC_CTR);
                end
              end
              OP_SELFTEST: begin
                if (busy)                 respond(ST_BUSY, 16'd0, SRC_SMALL);
                else if (len != 16'd0)    respond(ST_NACK_RANGE, 16'd0, SRC_SMALL);
                else begin
                  st_start <= 1'b1;
                  state    <= S_SELFTEST;
                end
              end
              OP_SET_DISPLAY: begin
                if (len != 16'd3 || {pl[1], pl[0]} > 16'd1000 || pl[2] > 8'd1)
                  respond(ST_NACK_RANGE, 16'd0, SRC_SMALL);
                else begin
                  disp_util  <= {pl[1], pl[0]};
                  disp_mode  <= pl[2];
                  disp_valid <= 1'b1;
                  respond(ST_ACK, 16'd0, SRC_SMALL);
                end
              end
              default: respond(ST_NACK_OPCODE, 16'd0, SRC_SMALL);
            endcase
          end

          // ------------------------------------------------ WRITE_MEM loop
          // S_WR_RD presents pb_raddr = WR_OFF + wcnt; pb_q holds the byte in
          // S_WR_ACC. Bytes collect into wacc/wbe; a word is written when its
          // last lane is filled or the data ends (byte enables cover partials).
          S_WR_RD: state <= S_WR_ACC;
          S_WR_ACC: begin
            if ((lane == lane_mask) || (wcnt == dlen - 16'd1)) begin
              mem_we    <= 1'b1;
              mem_addr  <= WBUF_AW'(cur_addr >> cur_shift);
              mem_wdata <= wacc_n;
              mem_be    <= wbe_n;
              wacc      <= '0;
              wbe       <= '0;
            end else begin
              wacc <= wacc_n;
              wbe  <= wbe_n;
            end
            cur_addr <= cur_addr + BA_W'(1);
            wcnt     <= wcnt + 16'd1;
            if (wcnt == dlen - 16'd1) respond(ST_ACK, 16'd0, SRC_SMALL);
            else                      state <= S_WR_RD;
          end

          // ------------------------------------------- wait on other blocks
          S_RUN: if (run_accept) begin
            run_valid <= 1'b0;
            respond(ST_ACK, 16'd0, SRC_SMALL);
          end
          S_SELFTEST: if (st_done) begin
            spay[0] <= {7'd0, st_pass_full};
            spay[1] <= {7'd0, st_pass_strip};
            for (int i = 0; i < 4; i++) spay[2 + i] <= st_fl_full[8*i +: 8];
            for (int i = 0; i < 4; i++) spay[6 + i] <= st_fl_strip[8*i +: 8];
            respond(ST_ACK, 16'(SELFTEST_LEN), SRC_SMALL);
          end

          // ------------------------------------------------------ respond
          S_FETCH: begin
            if (src == SRC_MEM) begin
              mem_re   <= 1'b1;
              mem_addr <= WBUF_AW'(cur_addr >> cur_shift);
            end
            ctr_idx <= cidx;
            fwait   <= FW_W'(L_RD);
            state   <= S_FWAIT;
          end
          S_FWAIT: begin
            if (fwait == '0) begin
              tx_byte  <= fetched;
              tx_phase <= PH_PAY;
              state    <= S_TX;
            end else begin
              fwait <= fwait - FW_W'(1);
            end
          end
          S_TX: if (tx_ready) begin
            case (tx_phase)
              PH_SYNC: begin
                tx_byte  <= status;
                tx_phase <= PH_STAT;
              end
              PH_STAT: begin
                tx_crc   <= crc_tx_upd;
                tx_byte  <= rsp_len[7:0];
                tx_phase <= PH_LEN0;
              end
              PH_LEN0: begin
                tx_crc   <= crc_tx_upd;
                tx_byte  <= rsp_len[15:8];
                tx_phase <= PH_LEN1;
              end
              PH_LEN1: begin
                tx_crc <= crc_tx_upd;
                if (rsp_len == 16'd0) begin
                  tx_byte  <= crc_tx_upd;
                  tx_phase <= PH_CRC;
                end else begin
                  state <= S_FETCH;
                end
              end
              PH_PAY: begin
                tx_crc   <= crc_tx_upd;
                pay_idx  <= pay_idx + 16'd1;
                cur_addr <= cur_addr + BA_W'(1);
                if (cbyte == CB_W'(COUNTER_BYTES - 1)) begin
                  cbyte <= '0;
                  cidx  <= cidx + CI_W'(1);
                end else begin
                  cbyte <= cbyte + CB_W'(1);
                end
                if (pay_idx == rsp_len - 16'd1) begin
                  tx_byte  <= crc_tx_upd;
                  tx_phase <= PH_CRC;
                end else begin
                  state <= S_FETCH;
                end
              end
              default: begin                    // PH_CRC sent: frame complete
                tx_phase <= PH_SYNC;
                state    <= S_SYNC;
              end
            endcase
          end
          default: state <= S_SYNC;
        endcase
      end
    end
  end

endmodule
