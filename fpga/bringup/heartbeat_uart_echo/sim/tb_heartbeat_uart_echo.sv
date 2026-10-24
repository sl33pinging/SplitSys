// =============================================================================
// tb_heartbeat_uart_echo.sv - smoke test for the stage-1 bring-up top
// (owner: fpga-integrator). Run: fpga/bringup/heartbeat_uart_echo/sim/run.sh
//
// Uses shortened, sim-only parameters (DIV_OVERRIDE=2 -> 32 clocks per bit,
// fast heartbeat/debounce/scan/lamp). Checks:
//   1. UART echo: 200 random bytes back to back at the DUT bit rate, then
//      300 bytes from a host 3.1% fast (31 clk/bit) -> every byte echoed in
//      order, no overflow / frame-error flags.
//   2. LED15 toggles every HB_HALF_CYC clocks; LED14 = 1; LED3 shows activity
//      during traffic and clears when idle; unused LEDs stay 0.
//   3. BTN0 glitches shorter than DEB_CYC are ignored; a held level reaches LED8.
//   4. 7-segment: lamp test (all segments low) then BUILD_ID nibbles with the
//      DP marker on D0 AN[0]; exactly one anode low per display.
// =============================================================================
`timescale 1ns/1ps
module tb_heartbeat_uart_echo;

  localparam int unsigned DIV      = 2;
  localparam int unsigned BITC     = DIV * 16;
  localparam int unsigned HB_HALF  = 50;
  localparam int unsigned DEB      = 8;
  localparam int unsigned ACT      = 20;
  localparam int unsigned SCAN     = 4;
  localparam int unsigned LAMP     = 100;
  localparam logic [31:0] BID      = 32'h1234_ABCD;
  localparam int unsigned N1       = 200;
  localparam int unsigned N2       = 300;
  localparam int unsigned NTOT     = N1 + N2;

  logic        clk = 1'b0;
  logic [3:0]  btn = 4'b0000;
  logic        rxd = 1'b1;
  logic [15:0] led;
  logic [3:0]  d0_an, d1_an;
  logic [7:0]  d0_seg, d1_seg;
  logic        txd;

  always #5 clk = ~clk;

  heartbeat_uart_echo #(
    .BUILD_ID(BID), .DIV_OVERRIDE(DIV), .HB_HALF_CYC(HB_HALF), .DEB_CYC(DEB),
    .ACT_CYC(ACT), .SCAN_CYC(SCAN), .LAMP_CYC(LAMP)
  ) dut (
    .CLK_100MHZ(clk), .BTN(btn), .LED(led),
    .D0_AN(d0_an), .D0_SEG(d0_seg), .D1_AN(d1_an), .D1_SEG(d1_seg),
    .UART_RXD(rxd), .UART_TXD(txd)
  );

  int errors = 0;
  task automatic fail(input string msg);
    errors++;
    if (errors <= 20) $display("ERROR @%0t: %s", $time, msg);
  endtask

  // ------------------------------------------------------------ stimulus data
  logic [7:0] sent [NTOT];
  logic [7:0] got  [NTOT];
  int         n_got = 0;

  task automatic send_byte(input logic [7:0] b, input int unsigned bitc);
    rxd = 1'b0; repeat (bitc) @(posedge clk);
    for (int i = 0; i < 8; i++) begin
      rxd = b[i]; repeat (bitc) @(posedge clk);
    end
    rxd = 1'b1; repeat (bitc) @(posedge clk);
  endtask

  // ------------------------------------------------------------- TX monitor
  initial begin : tx_mon
    logic [7:0] b;
    forever begin
      @(negedge txd);
      repeat (BITC / 2) @(posedge clk);
      if (txd !== 1'b0) fail("TX start bit not low at mid-bit");
      for (int i = 0; i < 8; i++) begin
        repeat (BITC) @(posedge clk);
        b[i] = txd;
      end
      repeat (BITC) @(posedge clk);
      if (txd !== 1'b1) fail("TX stop bit not high");
      if (n_got < NTOT) got[n_got] = b;
      n_got++;
    end
  end

  // ------------------------------------------------------- heartbeat monitor
  longint unsigned cyc = 0;
  longint unsigned last_hb_edge = 0;
  int              hb_edges = 0;
  logic            hb_prev = 1'b0;
  always @(posedge clk) begin
    cyc <= cyc + 1;
    hb_prev <= led[15];
    if (led[15] !== hb_prev && cyc > 20) begin
      if (hb_edges > 0 && (cyc - last_hb_edge) != HB_HALF)
        fail($sformatf("heartbeat half period %0d != %0d", cyc - last_hb_edge, HB_HALF));
      last_hb_edge <= cyc;
      hb_edges     <= hb_edges + 1;
    end
  end

  // ------------------------------------------------------ 7-segment monitor
  function automatic logic [6:0] hex7(input logic [3:0] v);
    // independent table (bit 0 = segment a), written as a packed constant
    logic [111:0] t;
    t = {7'h71, 7'h79, 7'h5E, 7'h39, 7'h7C, 7'h77, 7'h6F, 7'h7F,
         7'h07, 7'h7D, 7'h6D, 7'h66, 7'h4F, 7'h5B, 7'h06, 7'h3F};
    return t[7*v +: 7];
  endfunction

  int lamp_checks = 0, id_checks = 0;
  always @(posedge clk) begin
    if (cyc > 30) begin
      // exactly one anode low on each display, and the same index
      if (!(d0_an == 4'b1110 || d0_an == 4'b1101 || d0_an == 4'b1011 || d0_an == 4'b0111) || d0_an !== d1_an)
        fail($sformatf("bad anodes d0=%b d1=%b", d0_an, d1_an));
      if (cyc < LAMP) begin
        if (d0_seg !== 8'h00 || d1_seg !== 8'h00) fail("lamp test: segments not all on");
        lamp_checks++;
      end else if (cyc > LAMP + 30) begin  // DUT lamp counter starts after POR (~18 cycles)
        for (int k = 0; k < 4; k++) begin
          if (d0_an === ~(4'b0001 << k)) begin
            if (d0_seg !== ~{(k == 0), hex7(BID[4*k +: 4])})
              fail($sformatf("D0 digit %0d seg %h", k, d0_seg));
            if (d1_seg !== ~{1'b0, hex7(BID[16 + 4*k +: 4])})
              fail($sformatf("D1 digit %0d seg %h", k, d1_seg));
            id_checks++;
          end
        end
      end
    end
  end

  // ----------------------------------------------------------- main sequence
  int act_seen = 0;
  always @(posedge clk) if (led[3]) act_seen++;

  initial begin
    $display("tb_heartbeat_uart_echo: DIV=%0d BITC=%0d", DIV, BITC);
    repeat (40) @(posedge clk);
    if (led[14] !== 1'b1) fail("LED14 not on");
    if (led[3] !== 1'b0)  fail("LED3 on while idle");

    // ---- button debounce: glitches then a held level
    for (int g = 0; g < 5; g++) begin
      btn[0] = 1'b1; repeat (DEB / 2) @(posedge clk);
      btn[0] = 1'b0; repeat (3) @(posedge clk);
    end
    repeat (DEB + 6) @(posedge clk);
    if (led[8] !== 1'b0) fail("LED8 followed a glitch");
    btn[0] = 1'b1;
    repeat (DEB + 6) @(posedge clk);
    if (led[8] !== 1'b1) fail("LED8 did not follow held BTN0");
    btn[0] = 1'b0;
    repeat (DEB + 6) @(posedge clk);
    if (led[8] !== 1'b0) fail("LED8 did not release");

    // ---- echo phase 1: exact rate, back to back
    for (int i = 0; i < N1; i++) begin
      sent[i] = $urandom;
      send_byte(sent[i], BITC);
    end
    // ---- echo phase 2: host 3.1% fast, back to back
    for (int i = N1; i < NTOT; i++) begin
      sent[i] = $urandom;
      send_byte(sent[i], BITC - 1);
    end
    // wait for the FIFO to drain
    fork
      wait (n_got >= NTOT);
      repeat (80 * BITC * 11) @(posedge clk);
    join_any
    disable fork;
    repeat (ACT + 4 * BITC) @(posedge clk);

    if (n_got != NTOT) fail($sformatf("echoed %0d of %0d bytes", n_got, NTOT));
    for (int i = 0; i < NTOT && i < n_got; i++)
      if (got[i] !== sent[i]) fail($sformatf("byte %0d: sent %h got %h", i, sent[i], got[i]));
    if (led[5] !== 1'b0) fail("FIFO overflow flag set");
    if (led[4] !== 1'b0) fail("frame error flag set");
    if (act_seen == 0)   fail("LED3 never showed activity");
    if (led[3] !== 1'b0) fail("LED3 still on after idle");
    if ((led & 16'b0011_0000_1100_0111) !== 16'h0000) fail($sformatf("unused LEDs on: %b", led));
    if (hb_edges < 10)   fail($sformatf("only %0d heartbeat edges", hb_edges));
    if (lamp_checks == 0 || id_checks < 100) fail("7-seg checks did not run");

    $display("echoed=%0d hb_edges=%0d lamp_checks=%0d id_checks=%0d act_cycles=%0d",
             n_got, hb_edges, lamp_checks, id_checks, act_seen);
    if (errors == 0) $display("SMOKE PASS");
    else             $display("SMOKE FAIL errors=%0d", errors);
    $finish;
  end

endmodule
