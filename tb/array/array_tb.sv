// Testbench wrapper (owner: verification-engineer): rtl/array.sv built twice from
// the same inputs, ENABLE_STRIP = 1 (es1_*) and ENABLE_STRIP = 0 (es0_*, the
// cost-of-splitting build, which must behave as FULL whatever mode_strip says).
module array_tb (
    input  logic        clk,
    input  logic        rst,
    input  logic        mode_strip,
    input  logic [splitsys_pkg::ROWS-1:0][7:0]          a_in,
    input  logic        a_vld,
    input  logic        a_first,
    input  logic [splitsys_pkg::W_LANES-1:0][7:0]       w_in,
    input  logic [splitsys_pkg::ROWS-1:0]               row_inb,
    input  logic [splitsys_pkg::STRIP_TILE_N-1:0]       col_inb,
    input  logic [$clog2(splitsys_pkg::ROWS)-1:0]       drain_row,
    output logic [splitsys_pkg::STRIP_TILE_N-1:0][31:0] es1_drain_data,
    output logic [splitsys_pkg::N_QUADS-1:0][4:0]       es1_mac_cnt_q,
    output logic [6:0]                                  es1_mac_cnt,
    output logic [splitsys_pkg::STRIP_TILE_N-1:0][31:0] es0_drain_data,
    output logic [splitsys_pkg::N_QUADS-1:0][4:0]       es0_mac_cnt_q,
    output logic [6:0]                                  es0_mac_cnt
);
    array #(.ENABLE_STRIP(1'b1)) u_es1 (
        .clk, .rst, .mode_strip, .a_in, .a_vld, .a_first, .w_in, .row_inb, .col_inb, .drain_row,
        .drain_data(es1_drain_data), .mac_cnt_q(es1_mac_cnt_q), .mac_cnt(es1_mac_cnt));
    array #(.ENABLE_STRIP(1'b0)) u_es0 (
        .clk, .rst, .mode_strip, .a_in, .a_vld, .a_first, .w_in, .row_inb, .col_inb, .drain_row,
        .drain_data(es0_drain_data), .mac_cnt_q(es0_mac_cnt_q), .mac_cnt(es0_mac_cnt));
endmodule
