// Testbench wrapper (owner: verification-engineer): rtl/buffers.sv in all four
// region geometries from the package. Ports of instance X are prefixed X_.
`define BUF_PORTS(P, WORDS, WB) \
    input  logic                     P``_a_en, \
    input  logic [WB-1:0]            P``_a_we, \
    input  logic [$clog2(WORDS)-1:0] P``_a_addr, \
    input  logic [8*WB-1:0]          P``_a_wdata, \
    output logic [8*WB-1:0]          P``_a_rdata, \
    output logic                     P``_a_rvalid, \
    input  logic                     P``_b_en, \
    input  logic [WB-1:0]            P``_b_we, \
    input  logic [$clog2(WORDS)-1:0] P``_b_addr, \
    input  logic [8*WB-1:0]          P``_b_wdata, \
    output logic [8*WB-1:0]          P``_b_rdata
`define BUF_INST(P, NW, NB) \
    buffers #(.WORDS(NW), .WORD_BYTES(NB)) u_``P ( \
        .clk, .rst, \
        .a_en(P``_a_en), .a_we(P``_a_we), .a_addr(P``_a_addr), .a_wdata(P``_a_wdata), \
        .a_rdata(P``_a_rdata), .a_rvalid(P``_a_rvalid), \
        .b_en(P``_b_en), .b_we(P``_b_we), .b_addr(P``_b_addr), .b_wdata(P``_b_wdata), \
        .b_rdata(P``_b_rdata));
module buffers_tb (
    input  logic clk,
    input  logic rst,
    `BUF_PORTS(w, splitsys_pkg::WBUF_WORDS, splitsys_pkg::WBUF_WORD_BYTES),
    `BUF_PORTS(a, splitsys_pkg::ABUF_WORDS, splitsys_pkg::ABUF_WORD_BYTES),
    `BUF_PORTS(o, splitsys_pkg::OBUF_WORDS, splitsys_pkg::OBUF_WORD_BYTES),
    `BUF_PORTS(r, splitsys_pkg::RQ_ENTRIES, splitsys_pkg::RQ_WORD_BYTES)
);
    `BUF_INST(w, splitsys_pkg::WBUF_WORDS, splitsys_pkg::WBUF_WORD_BYTES)
    `BUF_INST(a, splitsys_pkg::ABUF_WORDS, splitsys_pkg::ABUF_WORD_BYTES)
    `BUF_INST(o, splitsys_pkg::OBUF_WORDS, splitsys_pkg::OBUF_WORD_BYTES)
    `BUF_INST(r, splitsys_pkg::RQ_ENTRIES, splitsys_pkg::RQ_WORD_BYTES)
endmodule
