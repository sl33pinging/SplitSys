// =============================================================================
// board_io.sv — reset generation, buttons, LEDs, 7-segment display (SPEC §3.9)
// -----------------------------------------------------------------------------
// Board facts (fpga/xdc/urbana_master.xdc, RealDigital Urbana V2I1):
//   * two 4-digit 7-segment displays, D0 and D1, each with its own anodes
//     D*_AN[3:0] ("Active LOW") and segments D*_SEG[7:0] = {DP, G, F, E, D, C, B, A}
//     (SEG[0] = CA ... SEG[6] = CG, SEG[7] = CDP, all "Active LOW").
//   * BTN[3:0] (LVCMOS25), LED[15:0] (LVCMOS33).
//   NOT in the XDC, so parameters here (defaults are assumptions to confirm in
//   Phase H): which display is physically on the left (LEFT_IS_D1), whether
//   AN[0] is the rightmost digit of its display (AN0_IS_RIGHT), and the button
//   polarity (BTN_ACTIVE_LOW). Segment/anode polarity defaults follow the XDC.
//
// Reset (this module is the reset root of the design)
//   rst = registered OR of: power-on reset (POR_CYC cycles after configuration),
//         debounced BTN1, and ext_rst_req (e.g. !mmcm_locked) through a 2-FF
//         synchronizer. rst is synchronous to clk, active high, and is held for
//         at least POR_CYC cycles and for as long as BTN1 is (debounced) pressed.
//   top must use `rst` as the single synchronous reset of every module, and
//   board_io uses it internally for everything except the reset generator.
//   Exception to "every flop reset": the reset generator's own flops (POR
//   counter, BTN1 synchronizer/debouncer, ext synchronizer, rst register) start
//   from FPGA configuration INIT values; they cannot be reset by the reset they
//   produce.
//
// Ports
//   btn[1:0]        raw buttons: BTN0 = self-test, BTN1 = reset
//   ext_rst_req     optional asynchronous reset request (tie 0 if unused)
//   rst             system reset out
//   selftest_req    1-cycle pulse on each debounced BTN0 press
//   pass_full, pass_strip, busy        levels -> LED0, LED1, LED2
//   uart_act        level, high while UART RX or TX is active -> LED3, stretched
//                   so every activity burst is visible for >= ACT_CYC cycles
//   led[15:0]       LED15 = 1 Hz heartbeat (HB_HALF_CYC high, HB_HALF_CYC low),
//                   LED4..14 off
//   disp_util, disp_mode, disp_valid   from packet.sv (SET_DISPLAY) or from a
//                   finished run (top muxes); captured on disp_valid
//   d0_an, d0_seg, d1_an, d1_seg       display pins (registered)
//   seg_img[63:0]   observability: the 8 characters currently shown, active-high
//                   {DP,G..A}; char k at [8k +: 8], k = 0 rightmost, 7 leftmost
//
// Display format (8 characters, left to right; position 7 is leftmost)
//   [7] mode letter: 'F' (mode 0), 'S' (mode 1), '-' otherwise
//   [6..4] blank
//   [3..0] utilization in percent with one decimal, right-aligned:
//          p = min(disp_util, 1000) permille; [1] = (p/10)%10 with DP lit,
//          [0] = p%10, [2] = (p/100)%10 only if p >= 100, [3] = 1 only if p = 1000.
//          So the left display shows the mode, the right one the value:
//          p=0 -> "F      0.0", p=93 -> "F    9.3" (shown "F   " | " 9.3"),
//          p=214 -> "S   " | "21.4", p=999 -> "99.9", p=1000 -> "100.0".
//   Before the first update every character shows '-'.
//
// Timing
//   Binary-to-BCD is a sequential double-dabble: the new value appears on
//   seg_img 11 cycles after disp_valid. Each display is scanned one digit at a
//   time for SCAN_CYC cycles (default 1 ms, 250 Hz frame), with anodes off for
//   the first SCAN_CYC/8 cycles of each slot to avoid ghosting.
//   Buttons: 2-FF synchronizer, then the debounced state follows the input after
//   it has been stable for DEB_CYC consecutive cycles (default 10 ms).
//
// Critical path: double-dabble step (4 nibble add-3 + shift) and the 8:1
// character mux into the segment register; both a few LUT levels.
// =============================================================================
module board_io
  import splitsys_pkg::*;
#(
  parameter int unsigned DEB_CYC        = CLK_HZ / 100,   // 10 ms button debounce
  parameter int unsigned HB_HALF_CYC    = CLK_HZ / 2,     // heartbeat half period: 1 Hz
  parameter int unsigned ACT_CYC        = CLK_HZ / 20,    // 50 ms UART-activity stretch
  parameter int unsigned SCAN_CYC       = CLK_HZ / 1000,  // 1 ms per digit slot
  parameter int unsigned POR_CYC        = 16,             // power-on reset length
  parameter bit          BTN_ACTIVE_LOW = 1'b0,           // ASSUMPTION: confirm on board
  parameter bit          SEG_ACTIVE_LOW = 1'b1,           // XDC: "Active LOW"
  parameter bit          AN_ACTIVE_LOW  = 1'b1,           // XDC: "Active LOW"
  parameter bit          LEFT_IS_D1     = 1'b1,           // ASSUMPTION: D1 is the left display
  parameter bit          AN0_IS_RIGHT   = 1'b1            // ASSUMPTION: AN[0] is the rightmost digit
) (
  input  logic        clk,
  input  logic [1:0]  btn,
  input  logic        ext_rst_req,
  output logic        rst,
  output logic        selftest_req,
  input  logic        pass_full,
  input  logic        pass_strip,
  input  logic        busy,
  input  logic        uart_act,
  output logic [15:0] led,
  input  logic [15:0] disp_util,
  input  logic [7:0]  disp_mode,
  input  logic        disp_valid,
  output logic [3:0]  d0_an,
  output logic [7:0]  d0_seg,
  output logic [3:0]  d1_an,
  output logic [7:0]  d1_seg,
  output logic [63:0] seg_img
);

  localparam int unsigned DEB_W = (DEB_CYC     > 1) ? $clog2(DEB_CYC)     : 1;
  localparam int unsigned HB_W  = (HB_HALF_CYC > 1) ? $clog2(HB_HALF_CYC) : 1;
  localparam int unsigned ACT_W = $clog2(ACT_CYC + 1);
  localparam int unsigned SC_W  = (SCAN_CYC    > 1) ? $clog2(SCAN_CYC)    : 1;
  localparam int unsigned POR_W = $clog2(POR_CYC + 1);

  if (DEB_CYC < 1 || HB_HALF_CYC < 1 || ACT_CYC < 1 || SCAN_CYC < 8 || POR_CYC < 1)
  begin : g_chk_params
    $error("board_io: DEB_CYC, HB_HALF_CYC, ACT_CYC, POR_CYC >= 1 and SCAN_CYC >= 8 required");
  end

  // ===================================================== reset generator
  // These flops use configuration INIT values (no reset input; see header).
  // Waiver: PROCASSINIT is intended here. The declaration initializer IS the
  // power-up value (Vivado maps it to the flop INIT); nothing else can reset
  // the reset root.
  /* verilator lint_off PROCASSINIT */
  logic [POR_W-1:0] por_cnt  = '0;
  logic             por_done = 1'b0;
  (* ASYNC_REG = "TRUE" *) logic b1_meta = 1'b0, b1_sync = 1'b0;
  (* ASYNC_REG = "TRUE" *) logic ext_meta = 1'b1, ext_sync = 1'b1;
  logic             b1_state = 1'b0;
  logic [DEB_W-1:0] b1_cnt   = '0;
  logic             rst_r    = 1'b1;
  /* verilator lint_on PROCASSINIT */

  always_ff @(posedge clk) begin
    if (!por_done) begin
      por_cnt  <= por_cnt + POR_W'(1);
      por_done <= (por_cnt == POR_W'(POR_CYC - 1));
    end
  end

  always_ff @(posedge clk) begin
    b1_meta  <= btn[1] ^ BTN_ACTIVE_LOW;
    b1_sync  <= b1_meta;
    ext_meta <= ext_rst_req;
    ext_sync <= ext_meta;
  end

  always_ff @(posedge clk) begin
    if (b1_sync == b1_state) begin
      b1_cnt <= '0;
    end else if (b1_cnt == DEB_W'(DEB_CYC - 1)) begin
      b1_cnt   <= '0;
      b1_state <= b1_sync;
    end else begin
      b1_cnt <= b1_cnt + DEB_W'(1);
    end
  end

  always_ff @(posedge clk) begin
    rst_r <= !por_done || b1_state || ext_sync;
  end

  assign rst = rst_r;

  // ============================================================ BTN0 path
  (* ASYNC_REG = "TRUE" *) logic b0_meta, b0_sync;
  logic             b0_state;
  logic [DEB_W-1:0] b0_cnt;

  always_ff @(posedge clk) begin
    if (rst) begin
      b0_meta <= 1'b0;
      b0_sync <= 1'b0;
    end else begin
      b0_meta <= btn[0] ^ BTN_ACTIVE_LOW;
      b0_sync <= b0_meta;
    end
  end

  always_ff @(posedge clk) begin
    if (rst) begin
      b0_state     <= 1'b0;
      b0_cnt       <= '0;
      selftest_req <= 1'b0;
    end else begin
      selftest_req <= 1'b0;
      if (b0_sync == b0_state) begin
        b0_cnt <= '0;
      end else if (b0_cnt == DEB_W'(DEB_CYC - 1)) begin
        b0_cnt       <= '0;
        b0_state     <= b0_sync;
        selftest_req <= b0_sync;          // pulse on the debounced press edge only
      end else begin
        b0_cnt <= b0_cnt + DEB_W'(1);
      end
    end
  end

  // ================================================================ LEDs
  logic [HB_W-1:0]  hb_cnt;
  logic             hb;
  logic [ACT_W-1:0] act_cnt;

  always_ff @(posedge clk) begin
    if (rst) begin
      hb_cnt <= '0;
      hb     <= 1'b0;
    end else if (hb_cnt == HB_W'(HB_HALF_CYC - 1)) begin
      hb_cnt <= '0;
      hb     <= !hb;
    end else begin
      hb_cnt <= hb_cnt + HB_W'(1);
    end
  end

  always_ff @(posedge clk) begin
    if (rst)                act_cnt <= '0;
    else if (uart_act)      act_cnt <= ACT_W'(ACT_CYC);
    else if (act_cnt != '0) act_cnt <= act_cnt - ACT_W'(1);
  end

  always_ff @(posedge clk) begin
    if (rst) led <= '0;
    else     led <= {hb, 11'd0, (act_cnt != '0), busy, pass_strip, pass_full};
  end

  // =========================================================== 7-segment
  // segment patterns, active high, bit order {DP, G, F, E, D, C, B, A}
  localparam logic [7:0] SEG_BLANK = 8'h00;
  localparam logic [7:0] SEG_DASH  = 8'h40;
  localparam logic [7:0] SEG_F     = 8'h71;
  localparam logic [7:0] SEG_S     = 8'h6D;
  localparam logic [7:0] SEG_DP    = 8'h80;

  function automatic logic [7:0] seg_digit(input logic [3:0] d);
    case (d)
      4'd0: return 8'h3F;
      4'd1: return 8'h06;
      4'd2: return 8'h5B;
      4'd3: return 8'h4F;
      4'd4: return 8'h66;
      4'd5: return 8'h6D;
      4'd6: return 8'h7D;
      4'd7: return 8'h07;
      4'd8: return 8'h7F;
      4'd9: return 8'h6F;
      default: return SEG_DASH;
    endcase
  endfunction

  // ---- capture + sequential double-dabble (10 input bits -> 4 BCD digits)
  logic        shown;          // a value has been displayed at least once
  logic [7:0]  mode_q;
  logic [9:0]  bin_sh;
  logic [15:0] bcd;
  logic [3:0]  dd_cnt;
  logic        dd_run;
  // Waiver: bcd_adj[15] is never used: the thousands digit is at most 1 (p <= 1000),
  // so its add-3 result never reaches bit 3 and is shifted out.
  /* verilator lint_off UNUSEDSIGNAL */
  logic [15:0] bcd_adj;
  /* verilator lint_on UNUSEDSIGNAL */
  logic [15:0] bcd_q;          // converted digits {thousands, hundreds, tens, ones}
  logic [9:0]  p_q;            // converted value (for blanking decisions)
  logic [9:0]  p_in;

  assign p_in = (disp_util > 16'd1000) ? 10'd1000 : disp_util[9:0];

  always_comb begin
    for (int n = 0; n < 4; n++) begin
      bcd_adj[4*n +: 4] = (bcd[4*n +: 4] >= 4'd5) ? bcd[4*n +: 4] + 4'd3 : bcd[4*n +: 4];
    end
  end

  always_ff @(posedge clk) begin
    if (rst) begin
      shown  <= 1'b0;
      mode_q <= '0;
      bin_sh <= '0;
      bcd    <= '0;
      dd_cnt <= '0;
      dd_run <= 1'b0;
      bcd_q  <= '0;
      p_q    <= '0;
    end else if (disp_valid) begin
      mode_q <= disp_mode;
      bin_sh <= p_in;
      p_q    <= p_in;
      bcd    <= '0;
      dd_cnt <= '0;
      dd_run <= 1'b1;
    end else if (dd_run) begin
      {bcd, bin_sh} <= {bcd_adj[14:0], bin_sh, 1'b0};
      dd_cnt        <= dd_cnt + 4'd1;
      if (dd_cnt == 4'd9) begin
        dd_run <= 1'b0;
        bcd_q  <= {bcd_adj[14:0], bin_sh[9]};
        shown  <= 1'b1;
      end
    end
  end

  // ---- character image (registered)
  always_ff @(posedge clk) begin
    if (rst) begin
      seg_img <= {8{SEG_DASH}};
    end else if (!dd_run) begin
      if (!shown) begin
        seg_img <= {8{SEG_DASH}};
      end else begin
        seg_img[8*7 +: 8] <= (mode_q == 8'(MODE_FULL))  ? SEG_F :
                             (mode_q == 8'(MODE_STRIP)) ? SEG_S : SEG_DASH;
        seg_img[8*4 +: 24] <= {3{SEG_BLANK}};
        seg_img[8*3 +: 8]  <= (p_q >= 10'd1000) ? seg_digit(bcd_q[15:12]) : SEG_BLANK;
        seg_img[8*2 +: 8]  <= (p_q >= 10'd100)  ? seg_digit(bcd_q[11:8])  : SEG_BLANK;
        seg_img[8*1 +: 8]  <= seg_digit(bcd_q[7:4]) | SEG_DP;
        seg_img[8*0 +: 8]  <= seg_digit(bcd_q[3:0]);
      end
    end
  end

  // ---- scan: both displays light digit `slot` at the same time
  logic [SC_W-1:0] sc_cnt;
  logic [1:0]      slot;

  always_ff @(posedge clk) begin
    if (rst) begin
      sc_cnt <= '0;
      slot   <= '0;
    end else if (sc_cnt == SC_W'(SCAN_CYC - 1)) begin
      sc_cnt <= '0;
      slot   <= slot + 2'd1;
    end else begin
      sc_cnt <= sc_cnt + SC_W'(1);
    end
  end

  logic       lit;               // anodes enabled (after the anti-ghosting gap)
  logic [2:0] pos_r, pos_l;      // character index shown on right / left display
  logic [3:0] an_onehot;
  assign lit       = (sc_cnt >= SC_W'(SCAN_CYC / 8));
  assign pos_r     = AN0_IS_RIGHT ? {1'b0, slot} : {1'b0, 2'd3 - slot};
  assign pos_l     = pos_r + 3'd4;
  assign an_onehot = 4'b0001 << slot;

  logic [7:0] seg_r, seg_l;
  logic [3:0] an_on;
  assign seg_r = seg_img[8*pos_r +: 8];
  assign seg_l = seg_img[8*pos_l +: 8];
  assign an_on = lit ? an_onehot : 4'b0000;

  always_ff @(posedge clk) begin
    if (rst) begin
      d0_an  <= {4{AN_ACTIVE_LOW}};
      d1_an  <= {4{AN_ACTIVE_LOW}};
      d0_seg <= {8{SEG_ACTIVE_LOW}};
      d1_seg <= {8{SEG_ACTIVE_LOW}};
    end else begin
      d0_an  <= an_on ^ {4{AN_ACTIVE_LOW}};
      d1_an  <= an_on ^ {4{AN_ACTIVE_LOW}};
      d0_seg <= (LEFT_IS_D1 ? seg_r : seg_l) ^ {8{SEG_ACTIVE_LOW}};
      d1_seg <= (LEFT_IS_D1 ? seg_l : seg_r) ^ {8{SEG_ACTIVE_LOW}};
    end
  end

endmodule
