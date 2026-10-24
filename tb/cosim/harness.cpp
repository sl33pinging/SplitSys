// =============================================================================
// tb/cosim/harness.cpp -- Verilator harness: simulated `top` behind a TCP socket
// (owner: verification-engineer). Used by BOTH tb/cosim (real splitsys-host over
// tcp:) and tb/system (Python scoreboard over the same socket).
//
// Data port (--port, 0 = ephemeral):
//   bytes received over TCP are serialized onto top.UART_RXD as 8N1 frames at
//   BIT_CYC = UART_DIV_OVERRIDE * OVERSAMPLE clocks per bit (back-to-back, no
//   idle gap between bytes); top.UART_TXD is decoded (mid-bit sampling, stop bit
//   checked) and every byte is sent back over TCP. Only the real pins are used.
//   One client at a time; state persists across connections (like a board).
// Control port (--ctrl-port, 0 = ephemeral), line protocol, one reply line each:
//   cycle            -> "ok <cycle>"
//   led              -> "ok <LED hex>"
//   seg              -> "ok <D0_AN> <D0_SEG> <D1_AN> <D1_SEG>" (hex)
//   btn <mask>       -> "ok"   logical press mask for BTN[3:0] (polarity applied here)
//   step <n>         -> "ok <cycle>"   advance n clocks (UART engines keep running)
//   idle             -> "ok <cycle>"   advance until quiescent (see `active()`)
//   stats            -> "ok <rx_bytes> <tx_bytes> <frame_errors> <cycle>"
//   quit             -> "ok", then exit
// Time only advances while something is in flight (RX queue/driver, TX decoder,
// LED2 = busy, or within LINGER clocks of the last UART byte). While idle the
// simulation is frozen, so wall-clock gaps between host bytes never become
// simulated idle time (no spurious RX_TIMEOUT inside a split TCP frame).
//
// Build-time defines (tb/cosim/Makefile, must match the -G overrides):
//   UART_DIV, OVERSAMPLE_N, BTN_ACTIVE_LOW_V, TRACE (optional FST: --trace f.fst)
// =============================================================================
#include <arpa/inet.h>
#include <fcntl.h>
#include <netinet/in.h>
#include <netinet/tcp.h>
#include <poll.h>
#include <signal.h>
#include <sys/socket.h>
#include <unistd.h>

#include <cerrno>
#include <cstdint>
#include <cstdio>
#include <cstdlib>
#include <cstring>
#include <deque>
#include <memory>
#include <string>
#include <vector>

#include "Vtop.h"
#include "verilated.h"
#ifdef TRACE
#include "verilated_fst_c.h"
#endif

#ifndef UART_DIV
#define UART_DIV 1
#endif
#ifndef OVERSAMPLE_N
#define OVERSAMPLE_N 16
#endif
#ifndef BTN_ACTIVE_LOW_V
#define BTN_ACTIVE_LOW_V 0
#endif

static volatile sig_atomic_t g_stop = 0;
static void on_signal(int) { g_stop = 1; }

static const uint64_t BIT_CYC = (uint64_t)UART_DIV * OVERSAMPLE_N;

struct Harness {
    std::unique_ptr<VerilatedContext> ctx;
    std::unique_ptr<Vtop> top;
#ifdef TRACE
    std::unique_ptr<VerilatedFstC> tfp;
#endif
    uint64_t cyc = 0;
    uint64_t linger_cyc = 0, linger = 0;
    uint8_t btn_mask = 0;

    // RX driver (host -> FPGA)
    std::deque<uint8_t> rxq;
    bool drv_busy = false;
    uint16_t drv_shift = 0;  // 10 bits: start(0), d0..d7, stop(1)
    int drv_bit = 0;
    uint64_t drv_cnt = 0;

    // TX decoder (FPGA -> host)
    bool mon_busy = false;
    uint64_t mon_cnt = 0;
    int mon_bit = 0;
    uint8_t mon_byte = 0;
    int prev_txd = 1;
    std::vector<uint8_t> txout;

    uint64_t n_rx = 0, n_tx = 0, n_ferr = 0;

    void apply_btn() {
        uint8_t v = btn_mask & 0xF;
        if (BTN_ACTIVE_LOW_V) v = (~v) & 0xF;
        top->BTN = v;
    }

    void tick() {
        // inputs for this cycle are set before the rising edge
        // RX driver
        int line = 1;
        if (!drv_busy && !rxq.empty()) {
            uint8_t b = rxq.front();
            rxq.pop_front();
            drv_shift = (uint16_t)((1u << 9) | ((uint16_t)b << 1));  // stop | data | start
            drv_bit = 0;
            drv_cnt = 0;
            drv_busy = true;
            n_rx++;
        }
        if (drv_busy) line = (drv_shift >> drv_bit) & 1;
        top->UART_RXD = line;
        apply_btn();

        top->CLK_100MHZ = 0;
        top->eval();
#ifdef TRACE
        if (tfp) tfp->dump(ctx->time());
#endif
        ctx->timeInc(5);
        top->CLK_100MHZ = 1;
        top->eval();
#ifdef TRACE
        if (tfp) tfp->dump(ctx->time());
#endif
        ctx->timeInc(5);
        cyc++;

        if (drv_busy) {
            if (++drv_cnt == BIT_CYC) {
                drv_cnt = 0;
                if (++drv_bit == 10) {
                    drv_busy = false;
                    linger = linger_cyc;
                }
            }
        }

        // TX decoder: sample the registered output after the edge
        int txd = top->UART_TXD & 1;
        if (!mon_busy) {
            if (prev_txd == 1 && txd == 0) {
                mon_busy = true;
                mon_cnt = 0;
                mon_bit = 0;
                mon_byte = 0;
            }
        } else {
            mon_cnt++;
            // start edge seen in cycle 0; sample bit i at BIT_CYC/2 + i*BIT_CYC
            if (mon_cnt == BIT_CYC / 2 + (uint64_t)mon_bit * BIT_CYC) {
                if (mon_bit == 0) {
                    if (txd != 0) {  // glitch, not a start bit
                        fprintf(stderr, "[harness] cycle %llu: TX start-bit glitch\n",
                                (unsigned long long)cyc);
                        n_ferr++;
                        mon_busy = false;
                    }
                } else if (mon_bit <= 8) {
                    mon_byte |= (uint8_t)(txd << (mon_bit - 1));
                } else {
                    if (txd != 1) {
                        fprintf(stderr, "[harness] cycle %llu: TX framing error (stop bit low)\n",
                                (unsigned long long)cyc);
                        n_ferr++;
                    } else {
                        txout.push_back(mon_byte);
                        n_tx++;
                    }
                    mon_busy = false;  // stop bit sampled at mid-bit; hunt for next start
                    linger = linger_cyc;
                }
                mon_bit++;
            }
        }
        prev_txd = txd;
        if (linger) linger--;
    }

    bool busy_led() const { return (top->LED >> 2) & 1; }
    bool active() const {
        return !rxq.empty() || drv_busy || mon_busy || busy_led() || linger != 0;
    }
};

// ------------------------------------------------------------------ sockets
static int listen_on(int port) {
    int s = socket(AF_INET, SOCK_STREAM, 0);
    if (s < 0) { perror("socket"); exit(1); }
    int one = 1;
    setsockopt(s, SOL_SOCKET, SO_REUSEADDR, &one, sizeof one);
    sockaddr_in a{};
    a.sin_family = AF_INET;
    a.sin_addr.s_addr = htonl(INADDR_LOOPBACK);
    a.sin_port = htons((uint16_t)port);
    if (bind(s, (sockaddr*)&a, sizeof a) < 0) { perror("bind"); exit(1); }
    if (listen(s, 4) < 0) { perror("listen"); exit(1); }
    fcntl(s, F_SETFL, O_NONBLOCK);
    return s;
}

static int port_of(int s) {
    sockaddr_in a{};
    socklen_t l = sizeof a;
    getsockname(s, (sockaddr*)&a, &l);
    return ntohs(a.sin_port);
}

static int accept_nb(int ls) {
    int c = accept(ls, nullptr, nullptr);
    if (c < 0) return -1;
    fcntl(c, F_SETFL, O_NONBLOCK);
    int one = 1;
    setsockopt(c, IPPROTO_TCP, TCP_NODELAY, &one, sizeof one);
    return c;
}

static void send_all(int fd, const void* p, size_t n) {
    const uint8_t* b = (const uint8_t*)p;
    while (n) {
        ssize_t w = send(fd, b, n, MSG_NOSIGNAL);
        if (w < 0) {
            if (errno == EAGAIN || errno == EWOULDBLOCK) {
                pollfd pf{fd, POLLOUT, 0};
                poll(&pf, 1, 100);
                continue;
            }
            return;  // peer gone; reader side will notice
        }
        b += w;
        n -= (size_t)w;
    }
}

int main(int argc, char** argv) {
    int port = 5555, cport = 0;
    uint64_t max_cycles = 0;
    const char* trace = nullptr;
    uint64_t linger_cyc = 64 * BIT_CYC + 4096;
    std::vector<char*> vargs{argv[0]};
    for (int i = 1; i < argc; i++) {
        std::string a = argv[i];
        if (a == "--port" && i + 1 < argc) port = atoi(argv[++i]);
        else if (a == "--ctrl-port" && i + 1 < argc) cport = atoi(argv[++i]);
        else if (a == "--max-cycles" && i + 1 < argc) max_cycles = strtoull(argv[++i], nullptr, 0);
        else if (a == "--linger" && i + 1 < argc) linger_cyc = strtoull(argv[++i], nullptr, 0);
        else if (a == "--trace" && i + 1 < argc) trace = argv[++i];
        else vargs.push_back(argv[i]);  // +verilator+... plusargs
    }
    signal(SIGPIPE, SIG_IGN);
    signal(SIGINT, on_signal);
    signal(SIGTERM, on_signal);

    Harness h;
    h.linger_cyc = linger_cyc;
    h.ctx = std::make_unique<VerilatedContext>();
    h.ctx->commandArgs((int)vargs.size(), vargs.data());
    h.top = std::make_unique<Vtop>(h.ctx.get(), "top");
#ifdef TRACE
    if (trace) {
        h.ctx->traceEverOn(true);
        h.tfp = std::make_unique<VerilatedFstC>();
        h.top->trace(h.tfp.get(), 99);
        h.tfp->open(trace);
    }
#else
    if (trace) fprintf(stderr, "[harness] --trace ignored: build with WAVES=1\n");
#endif
    h.top->CLK_100MHZ = 0;
    h.top->UART_RXD = 1;
    h.apply_btn();
    h.top->eval();
    // power-on: let board_io's POR finish and the line settle
    for (int i = 0; i < 256; i++) h.tick();

    int ls = listen_on(port), cls = listen_on(cport);
    printf("COSIM_PORT %d\nCOSIM_CTRL_PORT %d\nCOSIM_BIT_CYC %llu\n", port_of(ls), port_of(cls),
           (unsigned long long)BIT_CYC);
    printf("cosim: simulated top listening on tcp:127.0.0.1:%d  (splitsys-host --transport tcp:127.0.0.1:%d ...)\n",
           port_of(ls), port_of(ls));
    fflush(stdout);

    int cfd = -1, kfd = -1;
    std::string kbuf;
    bool quit = false;
    const uint64_t CHUNK = 2048;

    while (!quit && !g_stop && !Verilated::gotFinish()) {
        bool act = h.active();
        pollfd fds[4];
        int nf = 0;
        fds[nf++] = {ls, POLLIN, 0};
        fds[nf++] = {cls, POLLIN, 0};
        int ci = -1, ki = -1;
        if (cfd >= 0) { ci = nf; fds[nf++] = {cfd, POLLIN, 0}; }
        if (kfd >= 0) { ki = nf; fds[nf++] = {kfd, POLLIN, 0}; }
        poll(fds, nf, act ? 0 : 20);

        if (fds[0].revents & POLLIN) {
            int c = accept_nb(ls);
            if (c >= 0) {
                if (cfd >= 0) close(cfd);  // newest client wins (host reconnects per command)
                cfd = c;
                ci = -1;
            }
        }
        if (fds[1].revents & POLLIN) {
            int c = accept_nb(cls);
            if (c >= 0) {
                if (kfd >= 0) close(kfd);
                kfd = c;
                ki = -1;
                kbuf.clear();
            }
        }
        if (ci >= 0 && (fds[ci].revents & (POLLIN | POLLHUP | POLLERR))) {
            uint8_t buf[65536];
            ssize_t r = recv(cfd, buf, sizeof buf, 0);
            if (r > 0) h.rxq.insert(h.rxq.end(), buf, buf + r);
            else if (r == 0 || (errno != EAGAIN && errno != EWOULDBLOCK)) { close(cfd); cfd = -1; }
        }
        if (ki >= 0 && (fds[ki].revents & (POLLIN | POLLHUP | POLLERR))) {
            char buf[4096];
            ssize_t r = recv(kfd, buf, sizeof buf, 0);
            if (r > 0) kbuf.append(buf, (size_t)r);
            else if (r == 0 || (errno != EAGAIN && errno != EWOULDBLOCK)) { close(kfd); kfd = -1; kbuf.clear(); }
        }
        // control commands
        size_t nl;
        while (kfd >= 0 && (nl = kbuf.find('\n')) != std::string::npos) {
            std::string line = kbuf.substr(0, nl);
            kbuf.erase(0, nl + 1);
            char cmd[32] = {0};
            unsigned long long arg = 0;
            int na = sscanf(line.c_str(), "%31s %lli", cmd, (long long*)&arg);
            char out[256];
            std::string c = cmd;
            if (c == "cycle") snprintf(out, sizeof out, "ok %llu\n", (unsigned long long)h.cyc);
            else if (c == "led") snprintf(out, sizeof out, "ok %04x\n", (unsigned)h.top->LED);
            else if (c == "seg")
                snprintf(out, sizeof out, "ok %x %02x %x %02x\n", (unsigned)h.top->D0_AN, (unsigned)h.top->D0_SEG,
                         (unsigned)h.top->D1_AN, (unsigned)h.top->D1_SEG);
            else if (c == "btn" && na == 2) { h.btn_mask = (uint8_t)arg; snprintf(out, sizeof out, "ok\n"); }
            else if (c == "step" && na == 2) {
                for (unsigned long long i = 0; i < arg; i++) h.tick();
                snprintf(out, sizeof out, "ok %llu\n", (unsigned long long)h.cyc);
            } else if (c == "idle") {
                while (h.active()) h.tick();
                snprintf(out, sizeof out, "ok %llu\n", (unsigned long long)h.cyc);
            } else if (c == "stats")
                snprintf(out, sizeof out, "ok %llu %llu %llu %llu\n", (unsigned long long)h.n_rx,
                         (unsigned long long)h.n_tx, (unsigned long long)h.n_ferr, (unsigned long long)h.cyc);
            else if (c == "quit") { snprintf(out, sizeof out, "ok\n"); quit = true; }
            else snprintf(out, sizeof out, "err unknown command '%s'\n", line.c_str());
            send_all(kfd, out, strlen(out));
            if (!h.txout.empty() && cfd >= 0) { send_all(cfd, h.txout.data(), h.txout.size()); h.txout.clear(); }
        }

        if (h.active()) {
            for (uint64_t i = 0; i < CHUNK && h.active(); i++) h.tick();
        }
        if (!h.txout.empty()) {
            if (cfd >= 0) send_all(cfd, h.txout.data(), h.txout.size());
            else fprintf(stderr, "[harness] dropped %zu TX bytes (no client)\n", h.txout.size());
            h.txout.clear();
        }
        if (max_cycles && h.cyc >= max_cycles) {
            fprintf(stderr, "[harness] --max-cycles %llu reached\n", (unsigned long long)max_cycles);
            break;
        }
    }
    printf("cosim: exit at cycle %llu (rx %llu B, tx %llu B, frame errors %llu)\n", (unsigned long long)h.cyc,
           (unsigned long long)h.n_rx, (unsigned long long)h.n_tx, (unsigned long long)h.n_ferr);
    fflush(stdout);
#ifdef TRACE
    if (h.tfp) h.tfp->close();
#endif
    h.top->final();
    if (cfd >= 0) close(cfd);
    if (kfd >= 0) close(kfd);
    return h.n_ferr ? 3 : 0;
}
