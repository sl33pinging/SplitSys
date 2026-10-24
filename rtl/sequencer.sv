// =============================================================================
// sequencer.sv -- descriptor -> tile loop -> STREAM / FLUSH / DRAIN / WRITEBACK
//                 (SPEC sec. 3.5, 3.6, 4)
// -----------------------------------------------------------------------------
// FSM (state lengths are exact; they are what the counters measure):
//
//   state       cycles per tile          tap          what happens
//   IDLE        -                        -            run_accept = 1; run_valid
//                                                     -> latch descriptor, ctr_start
//   ACCEPT      L_ACCEPT = 1 (per job)   (total only) derive tile counts, M_pad
//   TILE_SETUP  L_SETUP  = 16            tap_setup    issue 16 RQPARAM reads
//                                                     (rq_base + col0 + j); each
//                                                     lands in the requant bank
//                                                     L_RD cycles later (the last
//                                                     two in STREAM cycles 0-1)
//   STREAM      L_RD + K                 tap_stream   issue K reads of ABUF/WBUF
//   FLUSH       14 + L_PE = 17           tap_flush    wait for PE(7,7)
//   DRAIN       T_DRAIN(mode, flags)     tap_drain    one drain_row per cycle
//   WRITEBACK   L_RQ + L_WB = 8          tap_drain    requant + last OBUF write
//   DONE        L_DONE = 1 (per job)     (total only) ctr_done pulse -> IDLE
//
//   Tile order: column block outer, row block inner. FULL tiles are 8x8
//   (row block = 8 rows, column block = 8 columns = half of a 16-col W/C
//   block); STRIP tiles are 4x16 (row block = 4 rows = half of an 8-row A
//   block). tiles(FULL) = ceil(M/8)*ceil(N/8), tiles(STRIP) = ceil(M/4)*ceil(N/16).
//
// Data movement (SPEC sec. 1.3; bases are word indices)
//   A word   a_base + (row0/8)*K + k      STRIP uses lanes 4h..4h+3, h = row0[2]
//   W word   w_base + (col0/16)*K + k     FULL uses lanes 8p..8p+7, p = col0[3]
//   int8 C   out_base + (col0/16)*M_pad + m            (FULL: byte enables 8p..)
//   int32 C  out_base + ((col0/16)*M_pad + m)*4 + q    (FULL q = 2p+g, g = 0..1;
//                                                       STRIP q = g = 0..3)
//   RQPARAM  rq_base + col0 + j, j = 0..15 (mod RQ_ENTRIES; lanes past the
//            tile's real columns load unused values, SPEC D-006)
//   All offsets are kept incrementally (no multipliers).
//
// Timing chain (cycle numbers relative to the first STREAM cycle s):
//   read k issued s+k -> BRAM data s+k+L_RD -> lane-select register ->
//   array port s+k+L_RD+1. The last word reaches the array in the last
//   STREAM cycle + 1 = first FLUSH cycle; PE(7,7) is drainable 17 cycles
//   later = first DRAIN cycle (FLUSH = 17 exactly, no slack).
//   DRAIN cycle d: drain_row -> array drain register (d+1) -> requant (6
//   stages) -> out valid d+7 = d+L_RQ -> registered OBUF write presented in
//   d+8 (L_WB = 1). The write of the last DRAIN cycle completes at the end of
//   the last WRITEBACK cycle.
//   int32: drain_row is held for 2 (FULL) or 4 (STRIP) cycles; the requant
//   pipeline re-processes the same row and word group g selects the lanes.
//
// Operand arrival offset (for the cycle model): word k reaches the ARRAY
//   INPUT PORT in cycle s + L_RD + 1 + k = s + 3 + k (read issued s+k, BRAM
//   data s+k+2, lane-select register s+k+3), and the port of PE(r,c) in
//   cycle s + 3 + k + r + c. STREAM covers s .. s+K+1, so the last word is at
//   the array port in the first FLUSH cycle f0.
// Drain boundary (red-team SEV-3 item): PE(r,c)'s final acc is readable from
//   f0 + L_PE + r + c; DRAIN starts at f0 + 17. Row slots are read in order,
//   one slot per cycle (int8) or per 2/4 cycles (int32), and a slot is the
//   physical row r (FULL) or the row pair i / 4+i (STRIP). So PE(7,7) is
//   read in DRAIN cycle 7 (FULL) or 3 (STRIP), never first. Worst margin:
//     FULL  int8: PE(r,7) read in DRAIN cycle r, 7 cycles of slack
//     STRIP int8: PE(4+i,7) read in DRAIN cycle i, 3 cycles of slack
//   Measured by mutation: FLUSH shortened by 3 still passes every job;
//   shortened by 4 corrupts exactly STRIP int8 lane 15 (= PE(4+i,7)).
//
// Counter taps (to counters.sv): exactly one of tap_setup/stream/flush/drain
//   is high in every running cycle except the ACCEPT and DONE cycles;
//   tap_stall = 0 (no stalls in the MVP). ctr_start = the accept cycle,
//   ctr_done = the DONE cycle, tile_done = last WRITEBACK cycle of each tile.
//   So cycles_total = L_ACCEPT + sum(states) + L_DONE.
//
// Interface notes
//   run_desc     descriptor bytes, byte i at [8*i +: 8], already validated by
//                packet.sv (SPEC sec. 3.5 rules 1-8). Sampled in the accept cycle.
//   seq_busy     state != IDLE.  seq_done: set in DONE, cleared at the next
//                accept.  seq_error: always 0 (MVP).
//   dbg_state    current state (state_e encoding below), for testbenches.
//
// Critical paths (estimates): incremental address adders (<= 12 bits, 1-2
//   CARRY4) feeding BRAM address registers; mask compares (7/9-bit) -> FF;
//   drain_row = static-mode bit select of a counter (1 LUT) -> array drain
//   mux select (high fanout, replicated by synthesis); OBUF write-data
//   formatting (4:1 int32 group mux / byte placement, 1-2 LUT) -> FF.
// =============================================================================
module sequencer
    import splitsys_pkg::*;
#(
    parameter bit STRIP_EN = splitsys_pkg::ENABLE_STRIP   // 0: STRIP compiled out (cost build)
) (
    input  logic                         clk,
    input  logic                         rst,
    // run interface (packet.sv / selftest via top)
    input  logic [DESC_BYTES*8-1:0]      run_desc,
    input  logic                         run_valid,
    output logic                         run_accept,
    output logic                         seq_busy,
    output logic                         seq_done,
    output logic                         seq_error,
    // counter taps
    output logic                         ctr_start,
    output logic                         ctr_done,
    output logic                         tap_setup,
    output logic                         tap_stream,
    output logic                         tap_flush,
    output logic                         tap_drain,
    output logic                         tap_stall,
    output logic                         tile_done,
    // ABUF / WBUF / RQPARAM port B (reads)
    output logic                         ab_en,
    output logic [ABUF_AW-1:0]           ab_addr,
    input  logic [ABUF_WORD_BITS-1:0]    ab_rdata,
    output logic                         wb_en,
    output logic [WBUF_AW-1:0]           wb_addr,
    input  logic [WBUF_WORD_BITS-1:0]    wb_rdata,
    output logic                         rq_en,
    output logic [RQ_AW-1:0]             rq_addr,
    input  logic [RQ_WORD_BITS-1:0]      rq_rdata,
    // OBUF port B (writes)
    output logic                         ob_en,
    output logic [OBUF_WORD_BYTES-1:0]   ob_we,
    output logic [OBUF_AW-1:0]           ob_addr,
    output logic [OBUF_WORD_BITS-1:0]    ob_wdata,
    // array
    output logic                         arr_mode_strip,
    output logic [ROWS-1:0][7:0]         arr_a,
    output logic                         arr_a_vld,
    output logic                         arr_a_first,
    output logic [W_LANES-1:0][7:0]      arr_w,
    output logic [ROWS-1:0]              arr_row_inb,
    output logic [STRIP_TILE_N-1:0]      arr_col_inb,
    output logic [$clog2(ROWS)-1:0]      arr_drain_row,
    // requant
    output logic                         rq_prm_we,
    output logic [3:0]                   rq_prm_lane,
    output logic [31:0]                  rq_prm_word,
    output logic                         rq_bypass,
    output logic                         rq_in_vld,
    input  logic                         rq_out_vld,
    input  logic [STRIP_TILE_N-1:0][31:0] rq_out,
    // observability
    output logic [2:0]                   dbg_state
);
    // ------------------------------------------------------------------ consts
    localparam int DRAIN_MUX_LAT = 1;    // array.sv drain read register
    localparam int RQ_PIPE_LAT   = 6;    // requant.sv pipeline
    localparam int RQ_LOADS      = STRIP_TILE_N;          // 16 params per tile
    localparam int FLUSH_LEN     = (ROWS-1) + (COLS-1) + L_PE;
    localparam int WB_LEN        = L_RQ + L_WB;
    localparam int CW            = 10;   // state cycle counter width (max L_RD+K = 258)

    // The FSM implements these latencies structurally; refuse to elaborate if
    // the package disagrees (the cycle model would silently diverge).
    if (L_RQ != DRAIN_MUX_LAT + RQ_PIPE_LAT || L_WB != 1 || L_ACCEPT != 1 ||
        L_DONE != 1 || L_SETUP < RQ_LOADS || L_RD < 1) begin : g_bad_latency
        $error("sequencer.sv: package latencies do not match the RTL structure");
    end

    typedef enum logic [2:0] {
        S_IDLE   = 3'd0,
        S_ACCEPT = 3'd1,
        S_SETUP  = 3'd2,
        S_STREAM = 3'd3,
        S_FLUSH  = 3'd4,
        S_DRAIN  = 3'd5,
        S_WB     = 3'd6,
        S_DONE   = 3'd7
    } state_e;

    state_e         state;
    logic [CW-1:0]  cnt;          // cycle index within the current state
    logic           last;         // last cycle of the current state
    logic           start;

    assign start      = (state == S_IDLE) && run_valid;
    assign run_accept = (state == S_IDLE);

    // ---------------------------------------------------- descriptor registers
    function automatic logic [7:0] dbyte(input int unsigned off);
        return run_desc[8*off +: 8];
    endfunction

    logic                strip_q, int32_q;
    logic [6:0]          m_q;            // 1..64
    logic [8:0]          k_q;            // 1..256
    logic [8:0]          n_q;            // 1..256
    logic [ABUF_AW-1:0]  a_base_q;
    logic [WBUF_AW-1:0]  w_base_q;
    logic [OBUF_AW-1:0]  o_base_q;
    logic [RQ_AW-1:0]    rq_base_q;

    always_ff @(posedge clk) begin
        if (rst) begin
            strip_q   <= 1'b0;
            int32_q   <= 1'b0;
            m_q       <= '0;
            k_q       <= '0;
            n_q       <= '0;
            a_base_q  <= '0;
            w_base_q  <= '0;
            o_base_q  <= '0;
            rq_base_q <= '0;
        end else if (start) begin
            strip_q   <= STRIP_EN && (dbyte(DESC_OFF_MODE) == MODE_STRIP);
            int32_q   <= (dbyte(DESC_OFF_FLAGS) == FLAGS_INT32);
            // Validated ranges (packet.sv): M <= 64, K, N <= 256, bases inside
            // their buffers, so only the low bits of each field are kept.
            m_q       <= run_desc[8*DESC_OFF_M +: 7];
            k_q       <= run_desc[8*DESC_OFF_K +: 9];
            n_q       <= run_desc[8*DESC_OFF_N +: 9];
            a_base_q  <= run_desc[8*DESC_OFF_A_BASE   +: ABUF_AW];
            w_base_q  <= run_desc[8*DESC_OFF_W_BASE   +: WBUF_AW];
            o_base_q  <= run_desc[8*DESC_OFF_OUT_BASE +: OBUF_AW];
            rq_base_q <= run_desc[8*DESC_OFF_RQ_BASE  +: RQ_AW];
        end
    end

    /* verilator lint_off UNUSEDSIGNAL */
    logic unused_desc;   // validated upper bytes (packet.sv guarantees ranges)
    assign unused_desc = ^run_desc;
    /* verilator lint_on UNUSEDSIGNAL */

    // ------------------------------------------- per-job derived (ACCEPT state)
    logic [4:0] rbn_q;       // row blocks:    FULL ceil(M/8) <= 8,  STRIP ceil(M/4) <= 16
    logic [5:0] cbn_q;       // column blocks: FULL ceil(N/8) <= 32, STRIP ceil(N/16) <= 16
    logic [6:0] mpad_q;      // M_pad = 8*ceil(M/8) <= 64
    logic [4:0] tdrain_q;    // DRAIN length
    logic [7:0] m_p7;
    logic [9:0] n_p7, n_p15;

    assign m_p7  = {1'b0, m_q} + 8'd7;
    assign n_p7  = {1'b0, n_q} + 10'd7;
    assign n_p15 = {1'b0, n_q} + 10'd15;

    always_ff @(posedge clk) begin
        if (rst) begin
            rbn_q    <= '0;
            cbn_q    <= '0;
            mpad_q   <= '0;
            tdrain_q <= '0;
        end else if (state == S_ACCEPT) begin
            mpad_q <= {m_p7[6:3], 3'b000};
            if (strip_q) begin
                rbn_q    <= 5'(({1'b0, m_q} + 8'd3) >> 2);
                cbn_q    <= 6'(n_p15 >> 4);
                tdrain_q <= int32_q ? 5'(T_DRAIN_STRIP_INT32) : 5'(T_DRAIN_STRIP);
            end else begin
                rbn_q    <= 5'(m_p7 >> 3);
                cbn_q    <= 6'(n_p7 >> 3);
                tdrain_q <= int32_q ? 5'(T_DRAIN_FULL_INT32) : 5'(T_DRAIN_FULL);
            end
        end
    end

    // ------------------------------------------------------------ tile state
    logic [4:0]           rb;      // row-block index
    logic [5:0]           cb;      // column-block index
    logic [6:0]           row0;    // first tile row (m)
    logic [8:0]           col0;    // first tile column (n)
    logic [ABUF_AW-1:0]   aoff;    // (row0/8) * K
    logic [WBUF_AW-1:0]   woff;    // (col0/16) * K
    logic [OBUF_AW-1:0]   ooff;    // (col0/16) * M_pad
    logic                 last_tile;

    assign last_tile = (rb == rbn_q - 5'd1) && (cb == cbn_q - 6'd1);

    always_ff @(posedge clk) begin
        if (rst || state == S_ACCEPT) begin
            rb   <= '0;
            cb   <= '0;
            row0 <= '0;
            col0 <= '0;
            aoff <= '0;
            woff <= '0;
            ooff <= '0;
        end else if (state == S_WB && last) begin
            if (rb == rbn_q - 5'd1) begin                 // next column block
                rb   <= '0;
                row0 <= '0;
                aoff <= '0;
                cb   <= cb + 6'd1;
                col0 <= col0 + (strip_q ? 9'd16 : 9'd8);
                if (strip_q || col0[3]) begin             // crossed a 16-col block
                    woff <= woff + WBUF_AW'(k_q);
                    ooff <= ooff + OBUF_AW'(mpad_q);
                end
            end else begin                                // next row block
                rb   <= rb + 5'd1;
                row0 <= row0 + (strip_q ? 7'd4 : 7'd8);
                if (!strip_q || row0[2]) aoff <= aoff + ABUF_AW'(k_q);
            end
        end
    end

    // ------------------------------------------------------------- FSM
    logic [CW-1:0] len;
    always_comb begin
        case (state)
            S_SETUP:  len = CW'(L_SETUP);
            S_STREAM: len = CW'(L_RD) + CW'(k_q);
            S_FLUSH:  len = CW'(FLUSH_LEN);
            S_DRAIN:  len = CW'(tdrain_q);
            S_WB:     len = CW'(WB_LEN);
            default:  len = CW'(1);                        // ACCEPT, DONE
        endcase
    end
    assign last = (cnt == len - CW'(1));

    always_ff @(posedge clk) begin
        if (rst) begin
            state <= S_IDLE;
            cnt   <= '0;
        end else begin
            case (state)
                S_IDLE:   if (start) state <= S_ACCEPT;
                S_ACCEPT: state <= S_SETUP;
                S_SETUP:  if (last) state <= S_STREAM;
                S_STREAM: if (last) state <= S_FLUSH;
                S_FLUSH:  if (last) state <= S_DRAIN;
                S_DRAIN:  if (last) state <= S_WB;
                S_WB:     if (last) state <= last_tile ? S_DONE : S_SETUP;
                S_DONE:   state <= S_IDLE;
                default:  state <= S_IDLE;
            endcase
            cnt <= (last || state == S_IDLE) ? '0 : cnt + CW'(1);
        end
    end

    always_ff @(posedge clk) begin
        if (rst || start)          seq_done <= 1'b0;
        else if (state == S_DONE)  seq_done <= 1'b1;
    end

    assign seq_busy   = (state != S_IDLE);
    assign seq_error  = 1'b0;
    assign ctr_start  = start;
    assign ctr_done   = (state == S_DONE);
    assign tap_setup  = (state == S_SETUP);
    assign tap_stream = (state == S_STREAM);
    assign tap_flush  = (state == S_FLUSH);
    assign tap_drain  = (state == S_DRAIN) || (state == S_WB);
    assign tap_stall  = 1'b0;
    assign tile_done  = (state == S_WB) && last;
    assign dbg_state  = state;

    // ------------------------------------------- RQPARAM load (TILE_SETUP)
    logic [L_RD-1:0]      rqv_p;          // read in flight
    logic [L_RD-1:0][3:0] rql_p;          // its lane

    // SETUP cycle j (j < 16) decides read j: address rq_base + col0 + j
    // (wraps mod RQ_ENTRIES), issued from registers in the next cycle.
    always_ff @(posedge clk) begin
        if (rst) begin
            rq_en   <= 1'b0;
            rq_addr <= '0;
        end else begin
            rq_en   <= (state == S_SETUP) && (cnt < CW'(RQ_LOADS));
            rq_addr <= rq_base_q + col0[RQ_AW-1:0] + RQ_AW'(cnt);
        end
    end

    // rq_en/rq_addr are registered (issued one cycle after the SETUP cycle
    // that decided them); the lane index travels with the read.
    logic [3:0] rq_lane_iss;
    always_ff @(posedge clk) begin
        if (rst) begin
            rq_lane_iss <= '0;
            rqv_p       <= '0;
            rql_p       <= '0;
        end else begin
            rq_lane_iss <= cnt[3:0];
            rqv_p       <= {rqv_p[L_RD-2:0], rq_en};
            rql_p       <= {rql_p[L_RD-2:0], rq_lane_iss};
        end
    end

    assign rq_prm_we   = rqv_p[L_RD-1];
    assign rq_prm_lane = rql_p[L_RD-1];
    assign rq_prm_word = rq_rdata;
    assign rq_bypass   = int32_q;

    // ------------------------------------------------ STREAM: buffer reads
    logic issue;
    assign issue = (state == S_STREAM) && (cnt < CW'(k_q));

    always_ff @(posedge clk) begin
        if (rst) begin
            ab_addr <= '0;
            wb_addr <= '0;
        end else if (state == S_SETUP) begin
            ab_addr <= a_base_q + aoff;
            wb_addr <= w_base_q + woff;
        end else if (issue) begin
            ab_addr <= ab_addr + ABUF_AW'(1);
            wb_addr <= wb_addr + WBUF_AW'(1);
        end
    end
    // address is presented combinationally with en in the issue cycle
    assign ab_en = issue;
    assign wb_en = issue;

    // vld/first travel with the read through L_RD cycles
    logic [L_RD-1:0] sv_p, sf_p;
    always_ff @(posedge clk) begin
        if (rst) begin
            sv_p <= '0;
            sf_p <= '0;
        end else begin
            sv_p <= {sv_p[L_RD-2:0], issue};
            sf_p <= {sf_p[L_RD-2:0], issue && (cnt == '0)};
        end
    end

    // lane-select register in front of the array (static selects per tile)
    logic half_a, half_w;
    assign half_a = row0[2];   // STRIP: rows 4h..4h+3 of the 8-row A block
    assign half_w = col0[3];   // FULL: columns 8p..8p+7 of the 16-col W block

    always_ff @(posedge clk) begin
        if (rst) begin
            arr_a       <= '0;
            arr_w       <= '0;
            arr_a_vld   <= 1'b0;
            arr_a_first <= 1'b0;
        end else begin
            arr_a_vld   <= sv_p[L_RD-1];
            arr_a_first <= sf_p[L_RD-1];
            for (int r = 0; r < ROWS; r++) begin
                if (strip_q) arr_a[r] <= ab_rdata[8*(4*int'(half_a) + (r % 4)) +: 8];
                else         arr_a[r] <= ab_rdata[8*r +: 8];
            end
            for (int c = 0; c < W_LANES; c++) begin
                if (strip_q) arr_w[c] <= wb_rdata[8*c +: 8];
                else         arr_w[c] <= wb_rdata[8*(8*int'(half_w) + (c % 8)) +: 8];
            end
        end
    end

    // ------------------------------------------------- array config + masks
    always_ff @(posedge clk) begin
        if (rst) begin
            arr_mode_strip <= 1'b0;
            arr_row_inb    <= '0;
            arr_col_inb    <= '0;
        end else begin
            arr_mode_strip <= strip_q;
            for (int i = 0; i < ROWS; i++)
                arr_row_inb[i] <= ({1'b0, row0} + 8'(i)) < {1'b0, m_q};
            for (int c = 0; c < STRIP_TILE_N; c++)
                arr_col_inb[c] <= ({1'b0, col0} + 10'(c)) < {1'b0, n_q};
        end
    end

    // ------------------------------------------------------ DRAIN + writeback
    // Word group g and drained row r from the DRAIN cycle counter.
    logic [2:0] d_row;
    logic [1:0] d_grp;
    always_comb begin
        if (!int32_q)     begin d_row = cnt[2:0];          d_grp = 2'd0;          end
        else if (strip_q) begin d_row = {1'b0, cnt[3:2]};  d_grp = cnt[1:0];      end
        else              begin d_row = cnt[3:1];          d_grp = {1'b0, cnt[0]}; end
    end
    assign arr_drain_row = d_row;

    always_ff @(posedge clk) begin
        if (rst) rq_in_vld <= 1'b0;
        else     rq_in_vld <= (state == S_DRAIN);       // drain register: +1 cycle
    end

    // Write context of a DRAIN cycle, delayed L_RQ cycles to meet the requant
    // output: valid, word address, byte enables, int32 word group.
    typedef struct packed {
        logic                        vld;
        logic [OBUF_AW-1:0]          addr;
        logic [OBUF_WORD_BYTES-1:0]  be;
        logic [1:0]                  grp;
    } wctx_t;

    wctx_t ctx_d;
    wctx_t ctx_p [L_RQ];
    logic [OBUF_AW-1:0] orow;      // (col0/16)*M_pad + m

    assign orow = ooff + OBUF_AW'(row0) + OBUF_AW'(d_row);

    always_comb begin
        ctx_d.vld = (state == S_DRAIN);
        ctx_d.grp = d_grp;
        if (int32_q) begin
            ctx_d.addr = o_base_q + {orow[OBUF_AW-3:0], 2'b00}
                       + OBUF_AW'(strip_q ? d_grp : {half_w, d_grp[0]});
            ctx_d.be   = '1;
        end else begin
            ctx_d.addr = o_base_q + orow;
            ctx_d.be   = strip_q ? '1 : (half_w ? 16'hFF00 : 16'h00FF);
        end
    end

    always_ff @(posedge clk) begin
        if (rst) begin
            for (int i = 0; i < L_RQ; i++) ctx_p[i] <= '0;
        end else begin
            ctx_p[0] <= ctx_d;
            for (int i = 1; i < L_RQ; i++) ctx_p[i] <= ctx_p[i-1];
        end
    end

    wctx_t ctx_o;
    assign ctx_o = ctx_p[L_RQ-1];

    // Format the OBUF word from the requant output and register it (L_WB = 1).
    logic [OBUF_WORD_BITS-1:0] wdata_d;
    logic [STRIP_TILE_N*32-1:0] rq_flat;     // lane j at [32*j +: 32]
    assign rq_flat = rq_out;
    always_comb begin
        if (int32_q) begin
            wdata_d = rq_flat[128*ctx_o.grp +: 128];       // lanes 4g..4g+3
        end else begin
            for (int c = 0; c < OBUF_WORD_BYTES; c++)
                wdata_d[8*c +: 8] = strip_q ? rq_flat[32*c +: 8] : rq_flat[32*(c % 8) +: 8];
        end
    end

    always_ff @(posedge clk) begin
        if (rst) begin
            ob_en    <= 1'b0;
            ob_we    <= '0;
            ob_addr  <= '0;
            ob_wdata <= '0;
        end else begin
            ob_en    <= ctx_o.vld;
            ob_we    <= ctx_o.vld ? ctx_o.be : '0;
            ob_addr  <= ctx_o.addr;
            ob_wdata <= wdata_d;
        end
    end

    /* verilator lint_off UNUSEDSIGNAL */
    logic unused_rq;   // out_vld is redundant with the context pipeline (checked in sim)
    assign unused_rq = rq_out_vld;
    /* verilator lint_on UNUSEDSIGNAL */

`ifndef SYNTHESIS
    // The requant output valid must coincide with the write context.
    always @(posedge clk) begin
        if (!rst && (rq_out_vld !== ctx_o.vld))
            $error("sequencer: requant out_vld %b != write context %b", rq_out_vld, ctx_o.vld);
    end
`endif

endmodule
