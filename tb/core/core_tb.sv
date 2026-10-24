// Testbench wrapper (owner: verification-engineer): rtl/core.sv + rtl/counters.sv wired
// exactly as rtl/top.sv wires them (start = ctr_start, done = ctr_done, taps 1:1), so
// the level-2 suite reads real 48-bit counters without the UART/packet layer.
module core_tb
    import splitsys_pkg::*;
#(
    parameter bit STRIP_EN = 1'b1
) (
    input  logic                          clk,
    input  logic                          rst,
    input  logic [$clog2(N_REGIONS)-1:0]  mem_region,
    input  logic [WBUF_AW-1:0]            mem_addr,
    input  logic [WBUF_WORD_BITS-1:0]     mem_wdata,
    input  logic [WBUF_WORD_BYTES-1:0]    mem_be,
    input  logic                          mem_we,
    input  logic                          mem_re,
    output logic [WBUF_WORD_BITS-1:0]     mem_rdata,
    input  logic [DESC_BYTES*8-1:0]       run_desc,
    input  logic                          run_valid,
    output logic                          run_accept,
    output logic                          seq_busy,
    output logic                          seq_done,
    output logic                          seq_error,
    input  logic                          ctr_snap,
    input  logic [$clog2(N_COUNTERS)-1:0] ctr_idx,
    output logic [COUNTER_BITS-1:0]       ctr_data,
    output logic                          ctr_running,
    output logic                          ctr_start,
    output logic                          ctr_done,
    output logic [2:0]                    dbg_state,
    output logic                          dbg_strip
);
    logic tap_setup, tap_stream, tap_flush, tap_drain, tap_stall, tile_done;
    logic [$clog2(N_PES+1)-1:0] mac_cnt;
    logic [N_QUADS*5-1:0]       mac_q_cnt;

    core #(.STRIP_EN(STRIP_EN)) u_core (
        .clk, .rst, .mem_region, .mem_addr, .mem_wdata, .mem_be, .mem_we, .mem_re, .mem_rdata,
        .run_desc, .run_valid, .run_accept, .seq_busy, .seq_done, .seq_error,
        .ctr_start, .ctr_done, .tap_setup, .tap_stream, .tap_flush, .tap_drain, .tap_stall,
        .mac_cnt, .mac_q_cnt, .tile_done, .dbg_state, .dbg_strip);

    counters u_counters (
        .clk, .rst, .start(ctr_start), .done(ctr_done),
        .tap_setup, .tap_stream, .tap_flush, .tap_drain, .tap_stall,
        .mac_cnt, .mac_q_cnt, .tile_done,
        .snap(ctr_snap), .rd_idx(ctr_idx), .rd_data(ctr_data), .running(ctr_running));
endmodule
