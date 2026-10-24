// Testbench wrapper (owner: verification-engineer). Instantiates rtl/skew.sv in
// every configuration the design uses, plus the single 16-lane W-skew form that
// SPEC v1.1 sec. 3.3 describes, all driven from one shared 16-lane input bus.
//   u_a   : A skew            LANES=8,  BASE=0,                 FLAGS=1  (array u_askew)
//   u_wt  : W skew, top edge  LANES=8,  BASE=0,                 FLAGS=0  (array u_wskew_top)
//   u_wm  : W skew, row-4     LANES=8,  BASE=W_SKEW_BASE_STRIP[127:64], FLAGS=0 (array u_wskew_mid)
//   u_w16 : SPEC sec. 3.3 single W unit LANES=16, BASE=W_SKEW_BASE_STRIP, FLAGS=1
module skew_tb (
    input  logic              clk,
    input  logic              rst,
    input  logic [15:0][7:0]  in_data,
    input  logic [15:0]       in_vld,
    input  logic [15:0]       in_first,
    output logic [7:0][7:0]   a_data,
    output logic [7:0]        a_vld,
    output logic [7:0]        a_first,
    output logic [7:0][7:0]   wt_data,
    output logic [7:0]        wt_vld,
    output logic [7:0]        wt_first,
    output logic [7:0][7:0]   wm_data,
    output logic [7:0]        wm_vld,
    output logic [7:0]        wm_first,
    output logic [15:0][7:0]  w16_data,
    output logic [15:0]       w16_vld,
    output logic [15:0]       w16_first
);
    localparam logic [63:0] MID_BASE = splitsys_pkg::W_SKEW_BASE_STRIP[127:64];

    skew #(.LANES(8), .WIDTH(8), .BASE('0), .FLAGS(1'b1)) u_a (
        .clk, .rst, .in_data(in_data[7:0]), .in_vld(in_vld[7:0]), .in_first(in_first[7:0]),
        .out_data(a_data), .out_vld(a_vld), .out_first(a_first));

    skew #(.LANES(8), .WIDTH(8), .BASE('0), .FLAGS(1'b0)) u_wt (
        .clk, .rst, .in_data(in_data[7:0]), .in_vld(in_vld[7:0]), .in_first(in_first[7:0]),
        .out_data(wt_data), .out_vld(wt_vld), .out_first(wt_first));

    skew #(.LANES(8), .WIDTH(8), .BASE(MID_BASE), .FLAGS(1'b0)) u_wm (
        .clk, .rst, .in_data(in_data[15:8]), .in_vld(in_vld[15:8]), .in_first(in_first[15:8]),
        .out_data(wm_data), .out_vld(wm_vld), .out_first(wm_first));

    skew #(.LANES(16), .WIDTH(8), .BASE(splitsys_pkg::W_SKEW_BASE_STRIP), .FLAGS(1'b1)) u_w16 (
        .clk, .rst, .in_data(in_data), .in_vld(in_vld), .in_first(in_first),
        .out_data(w16_data), .out_vld(w16_vld), .out_first(w16_first));
endmodule
