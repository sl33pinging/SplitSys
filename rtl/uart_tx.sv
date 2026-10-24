// =============================================================================
// uart_tx.sv — 8N1 UART transmitter with a one-byte holding register (SPEC §3.8)
// -----------------------------------------------------------------------------
// Ports
//   clk, rst          single clock, synchronous active-high reset
//   tx_data[7:0]      byte to send
//   tx_valid          request; the byte is taken in a cycle with tx_valid && tx_ready
//   tx_ready          high when the holding register is empty. It rises again as
//                     soon as the shifter has taken the held byte, so a producer
//                     has a whole frame time to fetch the next byte: frames go
//                     out back to back without idle gaps.
//   txd               serial output (idle high), driven from a flop
//   tx_busy           high while a frame is shifting or a byte is held
//
// Timing
//   One bit lasts BIT_CYC = DIV * OVERSAMPLE clocks, with
//     DIV = DIV_OVERRIDE != 0 ? DIV_OVERRIDE : BAUD_DIV (= round(CLK_HZ / (BAUD*OVERSAMPLE)), package),
//   i.e. the same bit period uart_rx assumes. DIV_OVERRIDE is SIMULATION-ONLY
//   and must be 0 in hardware builds.
//   Frame = start(0), d0..d7 (LSB first), stop(1) = 10 bit times, plus two idle
//   clocks between frames. txd falls 2 clocks after an accepted byte when idle.
//
// Invariants
//   txd is high whenever the transmitter is idle and during reset.
//   A byte is accepted only when tx_ready, and is transmitted exactly once.
// =============================================================================
module uart_tx
  import splitsys_pkg::*;
#(
  parameter int unsigned DIV_OVERRIDE = 0   // 0 = spec divisor; nonzero = sim only
) (
  input  logic       clk,
  input  logic       rst,
  input  logic [7:0] tx_data,
  input  logic       tx_valid,
  output logic       tx_ready,
  output logic       txd,
  output logic       tx_busy
);

  localparam int unsigned DIV     = (DIV_OVERRIDE != 0) ? DIV_OVERRIDE : BAUD_DIV;
  localparam int unsigned BIT_CYC = DIV * OVERSAMPLE;
  localparam int unsigned BW      = $clog2(BIT_CYC);

  if (DIV < 1)     $error("uart_tx: divisor must be >= 1");
  if (BIT_CYC < 2) $error("uart_tx: bit period must be >= 2 clocks");

  // ---------------------------------------------------------- holding register
  logic       hold_v;
  logic [7:0] hold_d;
  logic       load;          // shifter takes the held byte this cycle

  assign tx_ready = !hold_v;

  always_ff @(posedge clk) begin
    if (rst) begin
      hold_v <= 1'b0;
      hold_d <= '0;
    end else if (tx_valid && tx_ready) begin
      hold_v <= 1'b1;
      hold_d <= tx_data;
    end else if (load) begin
      hold_v <= 1'b0;
    end
  end

  // ------------------------------------------------------------------ shifter
  logic          shifting;
  logic [9:0]    shreg;      // {stop, d7..d0, start}; bit 0 is on the line
  logic [3:0]    bit_idx;    // 0..9
  logic [BW-1:0] cyc;

  assign load = !shifting && hold_v;

  always_ff @(posedge clk) begin
    if (rst) begin
      shifting <= 1'b0;
      shreg    <= '1;
      bit_idx  <= '0;
      cyc      <= '0;
    end else if (load) begin
      shifting <= 1'b1;
      shreg    <= {1'b1, hold_d, 1'b0};
      bit_idx  <= '0;
      cyc      <= '0;
    end else if (shifting) begin
      if (cyc == BW'(BIT_CYC - 1)) begin
        cyc     <= '0;
        shreg   <= {1'b1, shreg[9:1]};
        bit_idx <= bit_idx + 4'd1;
        if (bit_idx == 4'd9) shifting <= 1'b0;
      end else begin
        cyc <= cyc + BW'(1);
      end
    end
  end

  always_ff @(posedge clk) begin
    if (rst) txd <= 1'b1;
    else     txd <= shifting ? shreg[0] : 1'b1;
  end

  assign tx_busy = shifting || hold_v;

endmodule
