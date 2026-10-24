// =============================================================================
// heartbeat_uart_echo.sv - bring-up stage 1 top (owner: fpga-integrator)
// -----------------------------------------------------------------------------
// Glue only. Production modules used: rtl/uart_rx.sv, rtl/uart_tx.sv.
// Pins: fpga/xdc/urbana_splitsys.xdc (port names = master XDC names).
//
// What the board shows (docs/HW_CHECKLIST.md H-1, H-2):
//   LED15      heartbeat: toggles every CLK_HZ/2 cycles -> 1 Hz blink
//   LED14      constant 1 ("bitstream loaded", also proves LED polarity)
//   LED3       UART activity (rx frame in progress or tx busy), stretched 50 ms
//   LED5       sticky: echo FIFO overflowed (a byte was dropped)
//   LED4       sticky: RX frame error (stop bit low) seen
//   LED8..11   debounced level of BTN0..3 (measures button polarity)
//   other LEDs 0
//   Any change of the debounced BTN1 level clears the two sticky flags.
//   7-segment: lamp test (all segments + DP on all 8 digits) for 2 s after
//   configuration, then BUILD_ID in hex: D1 shows BUILD_ID[31:16], D0 shows
//   BUILD_ID[15:0]; digit AN[k] shows nibble k of its half (AN[3] = most
//   significant). The DP of D0 AN[0] is lit as an orientation marker.
//   UART: every received byte is echoed (8N1, BAUD from the package) through
//   a 64-byte FIFO, which absorbs the host/board baud mismatch.
//
// Reset: power-on reset only (POR_CYC cycles after configuration). No button
// is used as reset because the button active level is not yet known.
//
// Parameters whose defaults are the hardware values; testbenches may shorten
// them. Hardware builds (fpga/tcl/build.tcl) never override them, and
// build.tcl refuses any nonzero DIV_OVERRIDE.
// =============================================================================
module heartbeat_uart_echo
  import splitsys_pkg::*;
#(
  parameter logic [31:0] BUILD_ID     = 32'h0000_0000,
  parameter int unsigned DIV_OVERRIDE = 0,               // SIM ONLY; 0 in hardware
  parameter int unsigned HB_HALF_CYC  = CLK_HZ / 2,      // heartbeat half period
  parameter int unsigned DEB_CYC      = CLK_HZ / 100,    // 10 ms debounce
  parameter int unsigned ACT_CYC      = CLK_HZ / 20,     // 50 ms activity stretch
  parameter int unsigned SCAN_CYC     = CLK_HZ / 1000,   // 1 ms per digit
  parameter int unsigned LAMP_CYC     = 2 * CLK_HZ,      // 2 s lamp test
  parameter int unsigned POR_CYC      = 16
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

  logic clk;
  assign clk = CLK_100MHZ;

  // ------------------------------------------------------------ power-on reset
  // The counter's initial value comes from the bitstream (INIT); it is the only
  // flop without a synchronous reset in this top.
  localparam int unsigned PW = $clog2(POR_CYC + 1);
  /* verilator lint_off PROCASSINIT */  // intentional: bitstream INIT is the POR
  logic [PW-1:0] por_cnt = '0;
  logic          rst     = 1'b1;
  /* verilator lint_on PROCASSINIT */
  always_ff @(posedge clk) begin
    if (por_cnt != PW'(POR_CYC)) begin
      por_cnt <= por_cnt + PW'(1);
      rst     <= 1'b1;
    end else begin
      rst     <= 1'b0;
    end
  end

  // ---------------------------------------------------------------- heartbeat
  localparam int unsigned HW = $clog2(HB_HALF_CYC);
  logic [HW-1:0] hb_cnt;
  logic          hb;
  always_ff @(posedge clk) begin
    if (rst) begin
      hb_cnt <= '0;
      hb     <= 1'b0;
    end else if (hb_cnt == HW'(HB_HALF_CYC - 1)) begin
      hb_cnt <= '0;
      hb     <= ~hb;
    end else begin
      hb_cnt <= hb_cnt + HW'(1);
    end
  end

  // ---------------------------------------------------------- button debounce
  localparam int unsigned DBW = $clog2(DEB_CYC + 1);
  (* ASYNC_REG = "TRUE" *) logic [3:0] btn_meta, btn_sync;
  logic [3:0]     btn_db;
  logic [DBW-1:0] deb_cnt [4];

  always_ff @(posedge clk) begin
    if (rst) begin
      btn_meta <= '0;
      btn_sync <= '0;
    end else begin
      btn_meta <= BTN;
      btn_sync <= btn_meta;
    end
  end

  for (genvar b = 0; b < 4; b++) begin : g_deb
    always_ff @(posedge clk) begin
      if (rst) begin
        deb_cnt[b] <= '0;
        btn_db[b]  <= 1'b0;
      end else if (btn_sync[b] == btn_db[b]) begin
        deb_cnt[b] <= '0;
      end else if (deb_cnt[b] == DBW'(DEB_CYC - 1)) begin
        deb_cnt[b] <= '0;
        btn_db[b]  <= btn_sync[b];
      end else begin
        deb_cnt[b] <= deb_cnt[b] + DBW'(1);
      end
    end
  end

  logic btn1_prev;
  logic clear_flags;
  always_ff @(posedge clk) begin
    if (rst) btn1_prev <= 1'b0;
    else     btn1_prev <= btn_db[1];
  end
  assign clear_flags = (btn1_prev != btn_db[1]);

  // ---------------------------------------------------------------- UART echo
  logic [7:0] rx_data;
  logic       rx_valid, rx_frame_err, rx_active;
  logic [7:0] tx_data;
  logic       tx_valid, tx_ready, tx_busy;

  uart_rx #(.DIV_OVERRIDE(DIV_OVERRIDE)) u_rx (
    .clk          (clk),
    .rst          (rst),
    .rxd          (UART_RXD),
    .rx_data      (rx_data),
    .rx_valid     (rx_valid),
    .rx_frame_err (rx_frame_err),
    .rx_active    (rx_active)
  );

  uart_tx #(.DIV_OVERRIDE(DIV_OVERRIDE)) u_tx (
    .clk      (clk),
    .rst      (rst),
    .tx_data  (tx_data),
    .tx_valid (tx_valid),
    .tx_ready (tx_ready),
    .txd      (UART_TXD),
    .tx_busy  (tx_busy)
  );

  // 64 x 8 FIFO (distributed RAM), first-word fall-through.
  localparam int unsigned FD  = 64;
  localparam int unsigned FAW = $clog2(FD);
  localparam int unsigned FPW = FAW + 1;
  logic [7:0]   fifo_mem [FD];
  logic [FAW:0] wr_ptr, rd_ptr;
  logic         fifo_empty, fifo_full, push, pop;
  logic         ovf_flag, ferr_flag;

  assign fifo_empty = (wr_ptr == rd_ptr);
  assign fifo_full  = (wr_ptr[FAW] != rd_ptr[FAW]) && (wr_ptr[FAW-1:0] == rd_ptr[FAW-1:0]);
  assign push       = rx_valid && !fifo_full;
  assign tx_valid   = !fifo_empty;
  assign tx_data    = fifo_mem[rd_ptr[FAW-1:0]];
  assign pop        = tx_valid && tx_ready;

  always_ff @(posedge clk) begin
    if (push) fifo_mem[wr_ptr[FAW-1:0]] <= rx_data;
  end

  always_ff @(posedge clk) begin
    if (rst) begin
      wr_ptr    <= '0;
      rd_ptr    <= '0;
      ovf_flag  <= 1'b0;
      ferr_flag <= 1'b0;
    end else begin
      if (push) wr_ptr <= wr_ptr + FPW'(1);
      if (pop)  rd_ptr <= rd_ptr + FPW'(1);
      if (clear_flags) begin
        ovf_flag  <= 1'b0;
        ferr_flag <= 1'b0;
      end else begin
        if (rx_valid && fifo_full) ovf_flag  <= 1'b1;
        if (rx_frame_err)          ferr_flag <= 1'b1;
      end
    end
  end

  // ------------------------------------------------- UART activity stretcher
  localparam int unsigned AW = $clog2(ACT_CYC + 1);
  logic [AW-1:0] act_cnt;
  always_ff @(posedge clk) begin
    if (rst)                       act_cnt <= '0;
    else if (rx_active || tx_busy) act_cnt <= AW'(ACT_CYC);
    else if (act_cnt != '0)        act_cnt <= act_cnt - AW'(1);
  end

  // ------------------------------------------------------------- 7-segment
  localparam int unsigned SW  = $clog2(SCAN_CYC);
  localparam int unsigned LW  = $clog2(LAMP_CYC + 1);
  logic [SW-1:0] scan_cnt;
  logic [1:0]    digit;
  logic [LW-1:0] lamp_cnt;
  logic          lamp;

  always_ff @(posedge clk) begin
    if (rst) begin
      scan_cnt <= '0;
      digit    <= '0;
      lamp_cnt <= '0;
    end else begin
      if (scan_cnt == SW'(SCAN_CYC - 1)) begin
        scan_cnt <= '0;
        digit    <= digit + 2'd1;
      end else begin
        scan_cnt <= scan_cnt + SW'(1);
      end
      if (lamp_cnt != LW'(LAMP_CYC)) lamp_cnt <= lamp_cnt + LW'(1);
    end
  end
  assign lamp = (lamp_cnt != LW'(LAMP_CYC));

  // segment pattern, bit 0 = CA ... bit 6 = CG (active high here)
  function automatic logic [6:0] hex7(input logic [3:0] v);
    case (v)
      4'h0: hex7 = 7'h3F;  4'h1: hex7 = 7'h06;  4'h2: hex7 = 7'h5B;  4'h3: hex7 = 7'h4F;
      4'h4: hex7 = 7'h66;  4'h5: hex7 = 7'h6D;  4'h6: hex7 = 7'h7D;  4'h7: hex7 = 7'h07;
      4'h8: hex7 = 7'h7F;  4'h9: hex7 = 7'h6F;  4'hA: hex7 = 7'h77;  4'hB: hex7 = 7'h7C;
      4'hC: hex7 = 7'h39;  4'hD: hex7 = 7'h5E;  4'hE: hex7 = 7'h79;  default: hex7 = 7'h71;
    endcase
  endfunction

  logic [3:0] nib0, nib1;
  logic [7:0] seg0_on, seg1_on;     // active-high segment patterns incl. DP
  assign nib0 = BUILD_ID[{1'b0, digit, 2'b00} +: 4];
  assign nib1 = BUILD_ID[{1'b1, digit, 2'b00} +: 4];
  assign seg0_on = lamp ? 8'hFF : {(digit == 2'd0), hex7(nib0)};
  assign seg1_on = lamp ? 8'hFF : {1'b0,            hex7(nib1)};

  // registered, active-low outputs
  always_ff @(posedge clk) begin
    if (rst) begin
      D0_AN  <= 4'hF;
      D1_AN  <= 4'hF;
      D0_SEG <= 8'hFF;
      D1_SEG <= 8'hFF;
    end else begin
      D0_AN  <= ~(4'b0001 << digit);
      D1_AN  <= ~(4'b0001 << digit);
      D0_SEG <= ~seg0_on;
      D1_SEG <= ~seg1_on;
    end
  end

  // ---------------------------------------------------------------- LEDs
  always_ff @(posedge clk) begin
    if (rst) begin
      LED <= '0;
    end else begin
      LED        <= '0;
      LED[15]    <= hb;
      LED[14]    <= 1'b1;
      LED[11:8]  <= btn_db;
      LED[5]     <= ovf_flag;
      LED[4]     <= ferr_flag;
      LED[3]     <= (act_cnt != '0);
    end
  end

endmodule
