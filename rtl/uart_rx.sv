// =============================================================================
// uart_rx.sv — 8N1 UART receiver, 16x oversampling (SPEC §3.8)
// -----------------------------------------------------------------------------
// Ports
//   clk, rst          single clock, synchronous active-high reset
//   rxd               asynchronous serial input (idle high); 2-FF synchronized here
//   rx_data[7:0]      received byte, valid only in the rx_valid cycle (held after)
//   rx_valid          1-cycle pulse: a byte with a good stop bit was received
//   rx_frame_err      1-cycle pulse: the stop bit sampled low (byte discarded)
//   rx_active         high while a frame is being received (start..stop), for
//                     the UART-activity LED and for testbench probing
//
// Timing
//   A free-running divider makes one oversample tick every DIV clocks:
//     DIV = DIV_OVERRIDE != 0 ? DIV_OVERRIDE : BAUD_DIV (= round(CLK_HZ / (BAUD*OVERSAMPLE)), package)
//   DIV_OVERRIDE is a SIMULATION-ONLY speed-up and must be 0 in hardware builds.
//   A low level seen on a tick starts a frame; the start bit is re-checked
//   OVERSAMPLE/2 ticks later (mid-bit), which rejects glitches shorter than
//   half a bit. Each data bit and the stop bit are then sampled every
//   OVERSAMPLE ticks (near mid-bit), LSB first. The receiver returns to idle
//   at mid-stop-bit, so back-to-back frames with no idle time are received.
//   Latency: rx_valid rises 2 (synchronizer) + ~9.5 bit-times after the start
//   edge. Tolerates roughly +/-4% total baud mismatch.
//
// Invariants
//   rx_valid and rx_frame_err are never high in the same cycle.
//   Every flop is reset; no latches.
// =============================================================================
module uart_rx
  import splitsys_pkg::*;
#(
  parameter int unsigned DIV_OVERRIDE = 0   // 0 = spec divisor; nonzero = sim only
) (
  input  logic       clk,
  input  logic       rst,
  input  logic       rxd,
  output logic [7:0] rx_data,
  output logic       rx_valid,
  output logic       rx_frame_err,
  output logic       rx_active
);

  // ---------------------------------------------------------------- divider
  localparam int unsigned DIV  = (DIV_OVERRIDE != 0) ? DIV_OVERRIDE : BAUD_DIV;
  localparam int unsigned DW   = (DIV > 1) ? $clog2(DIV) : 1;
  localparam int unsigned OSW  = $clog2(OVERSAMPLE);
  localparam logic [OSW-1:0] OS_LAST = OSW'(OVERSAMPLE - 1);
  localparam logic [OSW-1:0] OS_MID  = OSW'(OVERSAMPLE / 2 - 1);

  // synthesis-time sanity checks (elaboration error if violated)
  if (DIV < 1)                 $error("uart_rx: divisor must be >= 1");
  if (OVERSAMPLE < 4)          $error("uart_rx: OVERSAMPLE must be >= 4");
  if ((1 << OSW) != OVERSAMPLE) $error("uart_rx: OVERSAMPLE must be a power of 2");

  logic [DW-1:0] div_cnt;
  logic          tick;

  always_ff @(posedge clk) begin
    if (rst) begin
      div_cnt <= '0;
      tick    <= 1'b0;
    end else if (div_cnt == DW'(DIV - 1)) begin
      div_cnt <= '0;
      tick    <= 1'b1;
    end else begin
      div_cnt <= div_cnt + DW'(1);
      tick    <= 1'b0;
    end
  end

  // ------------------------------------------------------------ synchronizer
  (* ASYNC_REG = "TRUE" *) logic rxd_meta, rxd_sync;

  always_ff @(posedge clk) begin
    if (rst) begin
      rxd_meta <= 1'b1;
      rxd_sync <= 1'b1;
    end else begin
      rxd_meta <= rxd;
      rxd_sync <= rxd_meta;
    end
  end

  // ---------------------------------------------------------------- receiver
  typedef enum logic [1:0] {S_IDLE, S_START, S_DATA, S_STOP} state_t;
  state_t         state;
  logic [OSW-1:0] os_cnt;
  logic [2:0]     bit_cnt;
  logic [7:0]     shreg;

  always_ff @(posedge clk) begin
    if (rst) begin
      state        <= S_IDLE;
      os_cnt       <= '0;
      bit_cnt      <= '0;
      shreg        <= '0;
      rx_data      <= '0;
      rx_valid     <= 1'b0;
      rx_frame_err <= 1'b0;
    end else begin
      rx_valid     <= 1'b0;
      rx_frame_err <= 1'b0;
      if (tick) begin
        case (state)
          S_IDLE: begin
            os_cnt <= '0;
            if (!rxd_sync) state <= S_START;
          end
          S_START: begin
            if (os_cnt == OS_MID) begin
              os_cnt  <= '0;
              bit_cnt <= '0;
              state   <= rxd_sync ? S_IDLE : S_DATA;   // glitch -> back to idle
            end else begin
              os_cnt <= os_cnt + OSW'(1);
            end
          end
          S_DATA: begin
            if (os_cnt == OS_LAST) begin
              os_cnt  <= '0;
              shreg   <= {rxd_sync, shreg[7:1]};      // LSB first
              bit_cnt <= bit_cnt + 3'd1;
              if (bit_cnt == 3'd7) state <= S_STOP;
            end else begin
              os_cnt <= os_cnt + OSW'(1);
            end
          end
          S_STOP: begin
            if (os_cnt == OS_LAST) begin
              os_cnt <= '0;
              state  <= S_IDLE;
              if (rxd_sync) begin
                rx_data  <= shreg;
                rx_valid <= 1'b1;
              end else begin
                rx_frame_err <= 1'b1;
              end
            end else begin
              os_cnt <= os_cnt + OSW'(1);
            end
          end
          default: state <= S_IDLE;
        endcase
      end
    end
  end

  assign rx_active = (state != S_IDLE);

endmodule
