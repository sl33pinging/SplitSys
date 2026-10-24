// =============================================================================
// tb_packet_mem.sv - smoke test for the stage-2 bring-up top (owner:
// fpga-integrator). Run: bash fpga/bringup/packet_mem/sim/run.sh
//
// Drives the real UART pin of packet_mem (DIV_OVERRIDE=2 -> 32 clocks/bit) with
// framed requests and parses the framed responses from the TX pin. Checks:
//   * TB CRC-8 self-check against CRC8_CHECK ("123456789").
//   * PING -> ACK, len 9, "SPLT", PROTO_VERSION, BUILD_ID (LE).
//   * For each region (WBUF, ABUF, OBUF, RQPARAM): WRITE_MEM of random bytes at
//     an unaligned address, READ_MEM of a window 3 bytes wider on each side ->
//     written bytes match and neighbours are still 0 (byte enables); a write
//     ending exactly at the region end -> ACK and read back; a write one byte
//     past the end -> NACK_RANGE.
//   * Bad CRC -> NACK_CRC; unknown opcode -> NACK_OPCODE.
//   * POLL -> idle; SET_DISPLAY -> ACK; RUN_GEMM is NOT exercised (stubbed).
// Every response's CRC is checked.
// =============================================================================
`timescale 1ns/1ps
module tb_packet_mem;
  import splitsys_pkg::*;

  localparam int unsigned DIV  = 2;
  localparam int unsigned BITC = DIV * 16;
  localparam logic [31:0] BID  = 32'hC0DE_1234;

  logic        clk = 1'b0;
  logic [3:0]  btn = 4'b0000;
  logic        rxd = 1'b1;
  logic [15:0] led;
  logic [3:0]  d0_an, d1_an;
  logic [7:0]  d0_seg, d1_seg;
  logic        txd;

  always #5 clk = ~clk;

  packet_mem #(
    .BUILD_ID(BID), .DIV_OVERRIDE(DIV), .RX_TIMEOUT_CYC(20 * BITC),
    .DEB_CYC(8), .HB_HALF_CYC(200), .ACT_CYC(20), .SCAN_CYC(16)
  ) dut (
    .CLK_100MHZ(clk), .BTN(btn), .LED(led),
    .D0_AN(d0_an), .D0_SEG(d0_seg), .D1_AN(d1_an), .D1_SEG(d1_seg),
    .UART_RXD(rxd), .UART_TXD(txd)
  );

  int errors = 0;
  int checks = 0;
  task automatic fail(input string msg);
    errors++;
    if (errors <= 30) $display("ERROR @%0t: %s", $time, msg);
  endtask
  task automatic expect_eq(input int got, input int exp, input string what);
    checks++;
    if (got != exp) fail($sformatf("%s: got %0d (0x%0h) expected %0d (0x%0h)", what, got, got, exp, exp));
  endtask

  // ----------------------------------------------------------------- CRC-8
  function automatic logic [7:0] crc8_byte(input logic [7:0] crc, input logic [7:0] d);
    logic [7:0] c;
    c = crc ^ d;
    for (int i = 0; i < 8; i++) c = c[7] ? ((c << 1) ^ CRC8_POLY) : (c << 1);
    return c;
  endfunction

  // ------------------------------------------------------------ UART driver
  task automatic send_byte(input logic [7:0] b);
    rxd = 1'b0; repeat (BITC) @(posedge clk);
    for (int i = 0; i < 8; i++) begin
      rxd = b[i]; repeat (BITC) @(posedge clk);
    end
    rxd = 1'b1; repeat (BITC) @(posedge clk);
  endtask

  // received byte stream from TXD
  logic [7:0] rxq [$];
  initial begin : tx_mon
    logic [7:0] b;
    forever begin
      @(negedge txd);
      repeat (BITC / 2) @(posedge clk);
      if (txd !== 1'b0) fail("TX start bit not low");
      for (int i = 0; i < 8; i++) begin
        repeat (BITC) @(posedge clk);
        b[i] = txd;
      end
      repeat (BITC) @(posedge clk);
      if (txd !== 1'b1) fail("TX stop bit not high");
      rxq.push_back(b);
    end
  end

  task automatic get_byte(output logic [7:0] b);
    int t;
    t = 0;
    while (rxq.size() == 0 && t <= 400 * BITC * 10) begin
      @(posedge clk);
      t++;
    end
    if (rxq.size() == 0) begin
      fail("timeout waiting for a response byte");
      b = 8'h00;
    end else begin
      b = rxq.pop_front();
    end
  endtask

  // ---------------------------------------------------- request / response
  logic [7:0] req [$];
  logic [7:0] rsp_payload [$];
  logic [7:0] rsp_status;

  task automatic send_request(input logic [7:0] op, input bit corrupt_crc = 0);
    logic [7:0] crc;
    int n;
    n = req.size();
    crc = CRC8_INIT;
    send_byte(SYNC_REQ);
    send_byte(op);                 crc = crc8_byte(crc, op);
    send_byte(n[7:0]);             crc = crc8_byte(crc, n[7:0]);
    send_byte(n[15:8]);            crc = crc8_byte(crc, n[15:8]);
    foreach (req[i]) begin
      send_byte(req[i]);           crc = crc8_byte(crc, req[i]);
    end
    send_byte(corrupt_crc ? ~crc : crc);
  endtask

  task automatic recv_response();
    logic [7:0] b, crc;
    int len;
    rsp_payload.delete();
    get_byte(b);
    expect_eq(b, SYNC_RSP, "response sync");
    crc = CRC8_INIT;
    get_byte(rsp_status);  crc = crc8_byte(crc, rsp_status);
    get_byte(b);           crc = crc8_byte(crc, b); len = b;
    get_byte(b);           crc = crc8_byte(crc, b); len |= int'(b) << 8;
    for (int i = 0; i < len; i++) begin
      get_byte(b); crc = crc8_byte(crc, b);
      rsp_payload.push_back(b);
    end
    get_byte(b);
    expect_eq(b, crc, "response crc");
  endtask

  task automatic put_u32(input logic [31:0] v);
    for (int i = 0; i < 4; i++) req.push_back(v[8*i +: 8]);
  endtask

  task automatic write_mem(input int region, input int addr, input logic [7:0] data [$],
                           input int exp_status);
    req.delete();
    req.push_back(8'(region));
    put_u32(addr);
    foreach (data[i]) req.push_back(data[i]);
    send_request(OP_WRITE_MEM);
    recv_response();
    expect_eq(rsp_status, exp_status, $sformatf("WRITE_MEM r%0d @%0d n%0d status", region, addr, data.size()));
    expect_eq(rsp_payload.size(), 0, "WRITE_MEM response len");
  endtask

  task automatic read_mem(input int region, input int addr, input int n, input int exp_status);
    req.delete();
    req.push_back(8'(region));
    put_u32(addr);
    req.push_back(8'(n));
    req.push_back(8'(n >> 8));
    send_request(OP_READ_MEM);
    recv_response();
    expect_eq(rsp_status, exp_status, $sformatf("READ_MEM r%0d @%0d n%0d status", region, addr, n));
    if (exp_status == ST_ACK) expect_eq(rsp_payload.size(), n, "READ_MEM response len");
  endtask

  // heartbeat must toggle
  int hb_edges = 0;
  logic hb_prev = 1'b0;
  always @(posedge clk) begin
    hb_prev <= led[15];
    if (led[15] !== hb_prev) hb_edges++;
  end


  // ------------------------------------------------------------------ main
  int region_bytes [4];
  initial begin
    logic [7:0] data [$];
    logic [7:0] chk;
    int base, n;

    region_bytes[REGION_WBUF]    = WBUF_BYTES;
    region_bytes[REGION_ABUF]    = ABUF_BYTES;
    region_bytes[REGION_OBUF]    = OBUF_BYTES;
    region_bytes[REGION_RQPARAM] = RQ_BYTES;

    // TB CRC self-check
    chk = CRC8_INIT;   // "123456789" = 0x31..0x39
    for (int i = 0; i < 9; i++) chk = crc8_byte(chk, 8'h31 + 8'(i));
    expect_eq(chk, CRC8_CHECK, "TB crc8 check value");

    repeat (100) @(posedge clk);

    // ---- PING
    req.delete();
    send_request(OP_PING);
    recv_response();
    expect_eq(rsp_status, ST_ACK, "PING status");
    expect_eq(rsp_payload.size(), 9, "PING len");
    if (rsp_payload.size() == 9) begin
      for (int i = 0; i < 4; i++) expect_eq(rsp_payload[i], PING_MAGIC_LE[8*i +: 8], "PING magic");
      expect_eq(rsp_payload[4], PROTO_VERSION, "PING version");
      for (int i = 0; i < 4; i++) expect_eq(rsp_payload[5+i], BID[8*i +: 8], "PING build_id");
    end

    // ---- memory round trips on every region
    for (int r = 0; r < N_REGIONS; r++) begin
      // unaligned write in the middle, read with a 3-byte margin on both sides
      base = (r == REGION_RQPARAM) ? 101 : 1003;
      n    = (r == REGION_RQPARAM) ? 29 : 45;
      data.delete();
      for (int i = 0; i < n; i++) data.push_back($urandom);
      write_mem(r, base, data, ST_ACK);
      read_mem(r, base - 3, n + 6, ST_ACK);
      if (rsp_payload.size() == n + 6) begin
        for (int i = 0; i < 3; i++) begin
          expect_eq(rsp_payload[i], 0, $sformatf("r%0d guard byte before", r));
          expect_eq(rsp_payload[n + 3 + i], 0, $sformatf("r%0d guard byte after", r));
        end
        for (int i = 0; i < n; i++)
          expect_eq(rsp_payload[3 + i], data[i], $sformatf("r%0d data byte %0d", r, i));
      end
      // write ending exactly at the region end
      data.delete();
      for (int i = 0; i < 5; i++) data.push_back($urandom);
      write_mem(r, region_bytes[r] - 5, data, ST_ACK);
      read_mem(r, region_bytes[r] - 5, 5, ST_ACK);
      if (rsp_payload.size() == 5)
        for (int i = 0; i < 5; i++) expect_eq(rsp_payload[i], data[i], $sformatf("r%0d end byte %0d", r, i));
      // one byte past the end
      write_mem(r, region_bytes[r] - 4, data, ST_NACK_RANGE);
      read_mem(r, region_bytes[r] - 4, 5, ST_NACK_RANGE);
      $display("region %0d ok-so-far (errors=%0d)", r, errors);
    end

    // ---- error paths
    req.delete();
    send_request(OP_PING, 1);
    recv_response();
    expect_eq(rsp_status, ST_NACK_CRC, "bad CRC status");
    req.delete();
    send_request(8'h7E);
    recv_response();
    expect_eq(rsp_status, ST_NACK_OPCODE, "unknown opcode status");

    // ---- POLL and SET_DISPLAY
    req.delete();
    send_request(OP_POLL);
    recv_response();
    expect_eq(rsp_status, ST_ACK, "POLL status");
    if (rsp_payload.size() == 1) expect_eq(rsp_payload[0], POLL_IDLE, "POLL state");
    else fail("POLL len");
    req.delete();
    req.push_back(8'd93); req.push_back(8'd0); req.push_back(8'd0);
    send_request(OP_SET_DISPLAY);
    recv_response();
    expect_eq(rsp_status, ST_ACK, "SET_DISPLAY status");

    // ---- no stray bytes; board_io heartbeat alive
    repeat (20 * BITC) @(posedge clk);
    expect_eq(rxq.size(), 0, "no stray response bytes");
    if (hb_edges < 4) fail($sformatf("heartbeat edges %0d", hb_edges));

    $display("checks=%0d errors=%0d", checks, errors);
    if (errors == 0 && checks > 300) $display("SMOKE PASS");
    else                             $display("SMOKE FAIL errors=%0d checks=%0d", errors, checks);
    $finish;
  end

endmodule
