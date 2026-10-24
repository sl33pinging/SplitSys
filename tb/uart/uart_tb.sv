// Testbench wrapper (owner: verification-engineer) for rtl/uart_tx.sv + rtl/uart_rx.sv.
//   f_* : fast sim divisor DIV_F (DIV_OVERRIDE, sim only). f_rx input = loop ? f_txd : f_rxd_ext
//   s_* : spec divisor (DIV_OVERRIDE = 0 -> splitsys_pkg::BAUD_DIV), tx looped into rx
module uart_tb #(
    parameter int unsigned DIV_F = 2
) (
    input  logic       clk,
    input  logic       rst,
    // fast pair
    input  logic [7:0] f_tx_data,
    input  logic       f_tx_valid,
    output logic       f_tx_ready,
    output logic       f_txd,
    output logic       f_tx_busy,
    input  logic       f_loop,
    input  logic       f_rxd_ext,
    output logic [7:0] f_rx_data,
    output logic       f_rx_valid,
    output logic       f_rx_frame_err,
    output logic       f_rx_active,
    // spec-divisor pair (looped)
    input  logic [7:0] s_tx_data,
    input  logic       s_tx_valid,
    output logic       s_tx_ready,
    output logic       s_txd,
    output logic [7:0] s_rx_data,
    output logic       s_rx_valid,
    output logic       s_rx_frame_err
);
    logic f_rxd;
    assign f_rxd = f_loop ? f_txd : f_rxd_ext;

    uart_tx #(.DIV_OVERRIDE(DIV_F)) u_ftx (
        .clk, .rst, .tx_data(f_tx_data), .tx_valid(f_tx_valid), .tx_ready(f_tx_ready),
        .txd(f_txd), .tx_busy(f_tx_busy));
    uart_rx #(.DIV_OVERRIDE(DIV_F)) u_frx (
        .clk, .rst, .rxd(f_rxd), .rx_data(f_rx_data), .rx_valid(f_rx_valid),
        .rx_frame_err(f_rx_frame_err), .rx_active(f_rx_active));

    /* verilator lint_off PINCONNECTEMPTY */
    uart_tx #(.DIV_OVERRIDE(0)) u_stx (
        .clk, .rst, .tx_data(s_tx_data), .tx_valid(s_tx_valid), .tx_ready(s_tx_ready),
        .txd(s_txd), .tx_busy());
    uart_rx #(.DIV_OVERRIDE(0)) u_srx (
        .clk, .rst, .rxd(s_txd), .rx_data(s_rx_data), .rx_valid(s_rx_valid),
        .rx_frame_err(s_rx_frame_err), .rx_active());
    /* verilator lint_on PINCONNECTEMPTY */
endmodule
