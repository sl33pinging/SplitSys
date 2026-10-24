// =============================================================================
// selftest.sv — on-chip self-test engine (SPEC §3.9, D-001; ROM format:
//               sw/crates/vec/README.md "Self-test ROM")
// -----------------------------------------------------------------------------
// On `start`, the engine:
//   1. checks ROM word 0 == MAGIC (else both tests fail immediately),
//   2. performs every load record: ROM word src+k -> region bytes dst+4k..dst+4k+3
//      (little-endian), one 4-byte byte-enabled write per word on the memory bus,
//   3. for job j = 0 (FULL) and 1 (STRIP): presents the 6-word descriptor on the
//      run interface, waits for accept, waits for the job to finish, then reads
//      CHECK_WORDS OBUF words from desc.out_base and computes Fletcher-32 (D-001)
//      over their bytes in address order, compares with the stored value,
//   4. latches pass_full/pass_strip and fl_full/fl_strip, pulses `done`.
//
// Ports
//   start          1-cycle pulse (top ORs board_io.selftest_req and packet.st_start).
//                  Ignored while busy or while seq_busy (a host job is running).
//   busy           high from the accepted start until (and excluding) the done cycle.
//                  top gives the selftest the memory bus and the run interface
//                  while busy; packet answers BUSY to memory/run opcodes then.
//   done           1-cycle pulse; pass_* / fl_* are valid in that cycle and held
//                  until the next start (cleared to 0 at start).
//   Memory bus: identical protocol to packet.sv (see its header): registered
//     mem_region / mem_addr (word address) / mem_wdata (lane i at [8i+:8]) /
//     mem_be / mem_we / mem_re pulses; mem_rdata valid exactly L_RD cycles
//     after the mem_re cycle. Writes here are always 4 aligned bytes.
//   Run interface: identical to packet.sv: run_desc stable while run_valid,
//     run_valid held until run_accept. Completion = seq_busy observed high after
//     the accept, then low again (seq_error at that point = failure).
//     The sequencer must therefore raise seq_busy at some point after accept.
//
// ROM: ROM_WORDS x 32 inferred block ROM, initialised by $readmemh(ROM_FILE),
//   synchronous read (1 cycle). ROM_FILE is relative to the simulator / Vivado
//   working directory; fpga-integrator should pass an absolute path.
//
// Timing (MVP; not on any performance path): 3 cycles per ROM field read,
//   3 cycles per loaded word, per OBUF word (2 + L_RD) + 8 cycles of checksum.
//   The current ROM (464 words) completes in roughly 2k cycles plus the two GEMMs.
//
// Fletcher-32 datapath: sum1 is updated with one 16-bit word per cycle; sum2 adds
//   the PREVIOUS sum1 in the same cycle (one-cycle lag), plus one flush step at
//   the end, so each cycle is a single 17-bit add + conditional subtract per sum.
//   mod 65535 maps an input word 0xFFFF to "add 0", as D-001 requires.
//
// dbg_state exposes the FSM for testbenches.
// =============================================================================
module selftest
  import splitsys_pkg::*;
#(
  parameter string       ROM_FILE  = "sw/crates/vec/out/selftest_rom.mem",
  parameter int unsigned ROM_WORDS = 1024
) (
  input  logic                          clk,
  input  logic                          rst,
  input  logic                          start,
  output logic                          busy,
  output logic                          done,
  output logic                          pass_full,
  output logic                          pass_strip,
  output logic [31:0]                   fl_full,
  output logic [31:0]                   fl_strip,
  // memory bus (same protocol as packet.sv)
  output logic [$clog2(N_REGIONS)-1:0]  mem_region,
  output logic [WBUF_AW-1:0]            mem_addr,
  output logic [WBUF_WORD_BITS-1:0]     mem_wdata,
  output logic [WBUF_WORD_BYTES-1:0]    mem_be,
  output logic                          mem_we,
  output logic                          mem_re,
  input  logic [WBUF_WORD_BITS-1:0]     mem_rdata,
  // run interface (same protocol as packet.sv)
  output logic [DESC_BYTES*8-1:0]       run_desc,
  output logic                          run_valid,
  input  logic                          run_accept,
  input  logic                          seq_busy,
  input  logic                          seq_error,
  output logic [4:0]                    dbg_state
);

  localparam logic [31:0] MAGIC   = 32'h5353_5431;
  localparam int unsigned RA_W    = $clog2(ROM_WORDS);
  localparam int unsigned RW      = $clog2(N_REGIONS);
  localparam int unsigned BA_W    = $clog2(WBUF_BYTES) + 1;
  localparam int unsigned LN_W    = $clog2(WBUF_WORD_BYTES);
  localparam int unsigned FW_W    = $clog2(L_RD + 1);
  localparam int unsigned N_JOBS  = 2;

  if (DESC_BYTES != 24 || WBUF_WORD_BYTES != 16 || OBUF_WORD_BYTES != 16 || L_RD < 1)
  begin : g_chk_cfg
    $error("selftest: assumes 24-byte descriptors, 16-byte WBUF/OBUF words, L_RD >= 1");
  end

  // ------------------------------------------------------------------ ROM
  (* rom_style = "block" *) logic [31:0] rom [ROM_WORDS];
  initial $readmemh(ROM_FILE, rom);

  logic [RA_W-1:0] rom_addr;
  logic [31:0]     rom_q;

  always_ff @(posedge clk) begin
    if (rst) rom_q <= '0;
    else     rom_q <= rom[rom_addr];
  end

  // ------------------------------------------------------------ helpers
  function automatic logic [15:0] mod65535_add(input logic [15:0] a, input logic [15:0] b);
    logic [16:0] s;
    s = {1'b0, a} + {1'b0, b};
    return (s >= 17'd65535) ? 16'(s - 17'd65535) : s[15:0];
  endfunction

  function automatic logic [2:0] region_shift(input logic [RW-1:0] r);
    case (r)
      RW'(REGION_WBUF): return 3'($clog2(WBUF_WORD_BYTES));
      RW'(REGION_ABUF): return 3'($clog2(ABUF_WORD_BYTES));
      RW'(REGION_OBUF): return 3'($clog2(OBUF_WORD_BYTES));
      default:          return 3'($clog2(RQ_WORD_BYTES));
    endcase
  endfunction

  // ------------------------------------------------------------ state
  typedef enum logic [4:0] {
    S_IDLE, S_RWAIT,
    S_MAGIC, S_NLOADS, S_JOBTBL, S_LOADTBL,
    S_LREC, S_LREG, S_LDST, S_LLEN, S_LSRC, S_LWORD, S_LWR,
    S_JREC, S_JW, S_RUN, S_JWAIT,
    S_CRD, S_CWAIT, S_CSUM, S_CFLUSH, S_CCMP,
    S_FINISH
  } state_t;

  state_t state, ret;
  assign dbg_state = state;

  logic            rw;            // S_RWAIT phase
  logic [31:0]     n_loads, job_tbl, load_tbl;
  logic [31:0]     li, k;
  logic [RW-1:0]   l_region;
  logic [31:0]     l_dst, l_len, l_src;
  logic            ji;            // job index 0/1
  logic [2:0]      jw;            // job record word index 0..7
  logic [31:0]     jrec [8];
  logic            seen_busy;
  logic [31:0]     ci;            // OBUF words checked so far
  logic [FW_W-1:0] fwait;
  logic [127:0]    cword;
  logic [2:0]      hw;            // half-word index within cword
  logic [15:0]     sum1, sum2;

  // common: issue a ROM read of `a`, continue in `nxt` with rom_q valid
  task automatic rom_read(input logic [RA_W-1:0] a, input state_t nxt);
    rom_addr <= a;
    ret      <= nxt;
    rw       <= 1'b1;
    state    <= S_RWAIT;
  endtask

  logic [BA_W-1:0] l_byte;
  logic [2:0]      l_shift;
  logic [LN_W-1:0] l_lane;
  assign l_byte  = BA_W'(l_dst + (k << 2));
  assign l_shift = region_shift(l_region);
  assign l_lane  = l_byte[LN_W-1:0] & LN_W'((1 << l_shift) - 1);

  always_ff @(posedge clk) begin
    if (rst) begin
      state      <= S_IDLE;
      ret        <= S_IDLE;
      rw         <= 1'b0;
      rom_addr   <= '0;
      busy       <= 1'b0;
      done       <= 1'b0;
      pass_full  <= 1'b0;
      pass_strip <= 1'b0;
      fl_full    <= '0;
      fl_strip   <= '0;
      mem_region <= '0;
      mem_addr   <= '0;
      mem_wdata  <= '0;
      mem_be     <= '0;
      mem_we     <= 1'b0;
      mem_re     <= 1'b0;
      run_desc   <= '0;
      run_valid  <= 1'b0;
      n_loads    <= '0;
      job_tbl    <= '0;
      load_tbl   <= '0;
      li         <= '0;
      k          <= '0;
      l_region   <= '0;
      l_dst      <= '0;
      l_len      <= '0;
      l_src      <= '0;
      ji         <= 1'b0;
      jw         <= '0;
      for (int i = 0; i < 8; i++) jrec[i] <= '0;
      seen_busy  <= 1'b0;
      ci         <= '0;
      fwait      <= '0;
      cword      <= '0;
      hw         <= '0;
      sum1       <= '0;
      sum2       <= '0;
    end else begin
      done   <= 1'b0;
      mem_we <= 1'b0;
      mem_re <= 1'b0;

      case (state)
        S_IDLE: if (start && !seq_busy) begin
          busy       <= 1'b1;
          pass_full  <= 1'b0;
          pass_strip <= 1'b0;
          fl_full    <= '0;
          fl_strip   <= '0;
          rom_read(RA_W'(32'd0), S_MAGIC);
        end

        S_RWAIT: begin                      // rom_addr -> rom_q takes 1 cycle
          if (rw) rw <= 1'b0;
          else    state <= ret;
        end

        // ---------------------------------------------------------- header
        S_MAGIC:   if (rom_q != MAGIC) state <= S_FINISH;
                   else rom_read(RA_W'(32'd3), S_NLOADS);
        S_NLOADS:  begin n_loads  <= rom_q; rom_read(RA_W'(32'd5), S_JOBTBL);  end
        S_JOBTBL:  begin job_tbl  <= rom_q; rom_read(RA_W'(32'd6), S_LOADTBL); end
        S_LOADTBL: begin load_tbl <= rom_q; li <= '0; state <= S_LREC;  end

        // ----------------------------------------------------------- loads
        S_LREC: begin
          if (li == n_loads) begin
            ji    <= 1'b0;
            state <= S_JREC;
          end else begin
            rom_read(RA_W'(load_tbl + (li << 2) + 32'd0), S_LREG);
          end
        end
        S_LREG: begin l_region <= rom_q[RW-1:0]; rom_read(RA_W'(load_tbl + (li << 2) + 32'd1), S_LDST); end
        S_LDST: begin l_dst    <= rom_q;         rom_read(RA_W'(load_tbl + (li << 2) + 32'd2), S_LLEN); end
        S_LLEN: begin l_len    <= rom_q;         rom_read(RA_W'(load_tbl + (li << 2) + 32'd3), S_LSRC); end
        S_LSRC: begin l_src    <= rom_q; k <= '0; state <= S_LWORD; end
        S_LWORD: begin
          if ((k << 2) >= l_len) begin
            li    <= li + 32'd1;
            state <= S_LREC;
          end else begin
            rom_read(RA_W'(l_src + k), S_LWR);
          end
        end
        S_LWR: begin
          mem_region <= l_region;
          mem_addr   <= WBUF_AW'(l_byte >> l_shift);
          mem_wdata  <= WBUF_WORD_BITS'(rom_q) << (8 * l_lane);
          mem_be     <= WBUF_WORD_BYTES'(4'hF) << l_lane;
          mem_we     <= 1'b1;
          k          <= k + 32'd1;
          state      <= S_LWORD;
        end

        // ------------------------------------------------------------ jobs
        S_JREC: begin
          jw <= '0;
          rom_read(RA_W'(job_tbl + 32'({ji, 3'b000})), S_JW);
        end
        S_JW: begin
          jrec[jw] <= rom_q;
          jw       <= jw + 3'd1;
          if (jw == 3'd7) state <= S_RUN;
          else            rom_read(RA_W'(job_tbl + 32'({ji, 3'b000}) + 32'(jw) + 32'd1), S_JW);
        end
        S_RUN: begin
          run_desc  <= {jrec[5], jrec[4], jrec[3], jrec[2], jrec[1], jrec[0]};
          run_valid <= 1'b1;
          seen_busy <= 1'b0;
          state     <= S_JWAIT;
        end
        S_JWAIT: begin
          if (run_valid) begin
            if (run_accept) run_valid <= 1'b0;
          end else if (seq_busy) begin
            seen_busy <= 1'b1;
          end else if (seen_busy) begin
            ci    <= '0;
            sum1  <= '0;
            sum2  <= '0;
            state <= seq_error ? S_CCMP : S_CRD;   // error -> compare fails
          end
        end

        // -------------------------------------------------------- checksum
        S_CRD: begin
          if (ci == jrec[7]) begin
            state <= S_CFLUSH;
          end else begin
            mem_region <= RW'(REGION_OBUF);
            mem_addr   <= WBUF_AW'(jrec[4] + ci);       // desc.out_base + ci
            mem_re     <= 1'b1;
            fwait      <= FW_W'(L_RD);
            state      <= S_CWAIT;
          end
        end
        S_CWAIT: begin
          if (fwait == '0) begin
            cword <= mem_rdata;
            hw    <= '0;
            state <= S_CSUM;
          end else begin
            fwait <= fwait - FW_W'(1);
          end
        end
        S_CSUM: begin
          sum1 <= mod65535_add(sum1, cword[16*hw +: 16]);
          sum2 <= mod65535_add(sum2, sum1);              // sum1 lags one word
          hw   <= hw + 3'd1;
          if (hw == 3'd7) begin
            ci    <= ci + 32'd1;
            state <= S_CRD;
          end
        end
        S_CFLUSH: begin
          sum2  <= mod65535_add(sum2, sum1);             // add the final sum1
          state <= S_CCMP;
        end
        S_CCMP: begin
          if (!ji) begin
            fl_full   <= {sum2, sum1};
            pass_full <= !seq_error && ({sum2, sum1} == jrec[6]);
          end else begin
            fl_strip   <= {sum2, sum1};
            pass_strip <= !seq_error && ({sum2, sum1} == jrec[6]);
          end
          if (32'(ji) == 32'(N_JOBS - 1)) begin
            state <= S_FINISH;
          end else begin
            ji    <= 1'b1;
            state <= S_JREC;
          end
        end

        S_FINISH: begin
          busy  <= 1'b0;
          done  <= 1'b1;
          state <= S_IDLE;
        end

        default: state <= S_IDLE;
      endcase
    end
  end

endmodule
