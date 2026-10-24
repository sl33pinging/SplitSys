//! Byte transports: `serial:<port>` (hardware), `tcp:<host:port>` (co-sim bridge),
//! `mock` (in-process mock board). The protocol layer above is identical for all.

use crate::mock::{MockBoard, MockConfig};
use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// Where the numbers come from; written as `source` in every run record.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Hw,
    Sim,
    Model,
}

impl Source {
    pub fn as_str(self) -> &'static str {
        match self {
            Source::Hw => "hw",
            Source::Sim => "sim",
            Source::Model => "model",
        }
    }
}

/// A raw byte pipe to a board (real, simulated, or mock).
pub trait Transport: Send {
    fn send(&mut self, data: &[u8]) -> io::Result<()>;
    /// Read whatever is available, waiting up to `timeout`. `Ok(0)` = timed out.
    fn recv(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize>;
    fn source(&self) -> Source;
    fn describe(&self) -> String;
    /// Multiplier on per-frame timeouts (the RTL co-sim is far slower than hw).
    fn timeout_scale(&self) -> u32 {
        1
    }
    /// Line rate in bytes/s, if the transport has one (timeouts scale with it).
    fn bytes_per_sec(&self) -> Option<f64> {
        None
    }
}

/// A byte stream whose read timeout can be set (serial ports, sockets, fakes).
pub trait TimedIo: Read + Write + Send {
    fn set_read_timeout(&mut self, t: Duration) -> io::Result<()>;
}

impl TimedIo for TcpStream {
    fn set_read_timeout(&mut self, t: Duration) -> io::Result<()> {
        TcpStream::set_read_timeout(self, Some(t.max(Duration::from_millis(1))))
    }
}

impl TimedIo for Box<dyn serialport::SerialPort> {
    fn set_read_timeout(&mut self, t: Duration) -> io::Result<()> {
        self.set_timeout(t).map_err(io::Error::other)
    }
}

/// Generic stream transport used by both serial and TCP.
pub struct StreamTransport<S: TimedIo> {
    io: S,
    source: Source,
    name: String,
    scale: u32,
    bps: Option<f64>,
}

impl<S: TimedIo> StreamTransport<S> {
    pub fn new(io: S, source: Source, name: String, scale: u32, bps: Option<f64>) -> Self {
        Self {
            io,
            source,
            name,
            scale,
            bps,
        }
    }
}

impl<S: TimedIo> Transport for StreamTransport<S> {
    fn send(&mut self, data: &[u8]) -> io::Result<()> {
        self.io.write_all(data)?;
        self.io.flush()
    }
    fn recv(&mut self, buf: &mut [u8], timeout: Duration) -> io::Result<usize> {
        self.io.set_read_timeout(timeout)?;
        match self.io.read(buf) {
            Ok(0) if self.source == Source::Sim => Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "co-sim bridge closed the connection",
            )),
            Ok(n) => Ok(n),
            Err(e) if matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock) => {
                Ok(0)
            }
            Err(e) => Err(e),
        }
    }
    fn source(&self) -> Source {
        self.source
    }
    fn describe(&self) -> String {
        self.name.clone()
    }
    fn timeout_scale(&self) -> u32 {
        self.scale
    }
    fn bytes_per_sec(&self) -> Option<f64> {
        self.bps
    }
}

/// In-process mock board transport.
pub struct MockTransport {
    pub board: MockBoard,
    out: VecDeque<u8>,
}

impl MockTransport {
    pub fn new(cfg: MockConfig) -> Self {
        Self {
            board: MockBoard::new(cfg),
            out: VecDeque::new(),
        }
    }
}

impl Transport for MockTransport {
    fn send(&mut self, data: &[u8]) -> io::Result<()> {
        let r = self.board.rx(data);
        self.out.extend(r);
        Ok(())
    }
    fn recv(&mut self, buf: &mut [u8], _timeout: Duration) -> io::Result<usize> {
        let n = buf.len().min(self.out.len());
        for (i, b) in self.out.drain(..n).enumerate() {
            buf[i] = b;
        }
        Ok(n)
    }
    fn source(&self) -> Source {
        Source::Model
    }
    fn describe(&self) -> String {
        "mock (in-process model board)".into()
    }
}

/// Parsed `--transport` value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TransportSpec {
    Mock,
    Serial(String),
    Tcp(String),
}

impl std::str::FromStr for TransportSpec {
    type Err = String;
    fn from_str(s: &str) -> Result<Self, String> {
        if s == "mock" {
            Ok(Self::Mock)
        } else if let Some(p) = s.strip_prefix("serial:") {
            if p.is_empty() {
                Err("serial: needs a port, e.g. serial:COM5, serial:/dev/ttyUSB1, serial:auto".into())
            } else {
                Ok(Self::Serial(p.to_string()))
            }
        } else if let Some(a) = s.strip_prefix("tcp:") {
            if a.rsplit_once(':').is_some_and(|(_, port)| port.parse::<u16>().is_ok()) {
                Ok(Self::Tcp(a.to_string()))
            } else {
                Err(format!("tcp:{a}: expected tcp:HOST:PORT, e.g. tcp:127.0.0.1:5555"))
            }
        } else {
            Err(format!(
                "unknown transport '{s}': use mock | serial:COM5 | serial:/dev/ttyUSB1 | serial:auto | tcp:127.0.0.1:PORT"
            ))
        }
    }
}

/// Serial ports visible to the OS, for error messages and `serial:auto`.
pub fn list_serial_ports() -> Vec<String> {
    match serialport::available_ports() {
        Ok(v) => v
            .into_iter()
            .map(|p| match p.port_type {
                serialport::SerialPortType::UsbPort(u) => format!(
                    "{} (USB {:04x}:{:04x} {})",
                    p.port_name,
                    u.vid,
                    u.pid,
                    u.product.unwrap_or_default()
                ),
                _ => p.port_name,
            })
            .collect(),
        Err(_) => vec![],
    }
}

fn ports_hint() -> String {
    let ports = list_serial_ports();
    if ports.is_empty() {
        "no serial ports found (is the board plugged in / attached? see docs/ENV.md USB strategy)".into()
    } else {
        format!("available serial ports: {}", ports.join(", "))
    }
}

/// Default TCP timeout multiplier: RTL co-simulation runs far below real time.
pub const SIM_TIMEOUT_SCALE: u32 = 200;

/// Open a transport. `timeout_scale` overrides the per-transport default.
pub fn open(
    spec: &TransportSpec,
    baud: u32,
    timeout_scale: Option<u32>,
    mock_cfg: MockConfig,
) -> Result<Box<dyn Transport>, String> {
    match spec {
        TransportSpec::Mock => Ok(Box::new(MockTransport::new(mock_cfg))),
        TransportSpec::Tcp(addr) => {
            let s = TcpStream::connect(addr).map_err(|e| {
                format!("cannot connect to co-sim bridge at {addr}: {e} (start it with `make -C tb cosim`, which prints the port)")
            })?;
            s.set_nodelay(true).ok();
            Ok(Box::new(StreamTransport::new(
                s,
                Source::Sim,
                format!("tcp:{addr} (co-sim)"),
                timeout_scale.unwrap_or(SIM_TIMEOUT_SCALE),
                None,
            )))
        }
        TransportSpec::Serial(p) => {
            let port = if p == "auto" {
                let ports: Vec<String> = serialport::available_ports()
                    .map_err(|e| format!("cannot list serial ports: {e}"))?
                    .into_iter()
                    .map(|p| p.port_name)
                    .collect();
                match ports.as_slice() {
                    [one] => one.clone(),
                    [] => return Err(format!("serial:auto: {}", ports_hint())),
                    _ => {
                        return Err(format!(
                            "serial:auto: more than one port, pick one with --transport serial:<port>; {}",
                            ports_hint()
                        ))
                    }
                }
            } else {
                p.clone()
            };
            let sp = serialport::new(&port, baud)
                .data_bits(serialport::DataBits::Eight)
                .parity(serialport::Parity::None)
                .stop_bits(serialport::StopBits::One)
                .flow_control(serialport::FlowControl::None)
                .timeout(Duration::from_millis(100))
                .open()
                .map_err(|e| format!("cannot open serial port {port} at {baud} baud: {e}; {}", ports_hint()))?;
            Ok(Box::new(StreamTransport::new(
                sp,
                Source::Hw,
                format!("serial:{port}@{baud}"),
                timeout_scale.unwrap_or(1),
                Some(f64::from(baud) / 10.0),
            )))
        }
    }
}

#[cfg(test)]
pub mod fake {
    //! In-process fakes: a socket-backed "co-sim bridge" and a serial-like pipe,
    //! both running a [`MockBoard`] on a thread. They exercise the exact
    //! `StreamTransport` code the serial and TCP transports use.
    use super::*;
    use std::net::TcpListener;
    use std::sync::mpsc;
    use std::thread;

    /// Spawn a TCP server that speaks the protocol through a MockBoard. Returns its address.
    pub fn spawn_tcp_bridge(cfg: MockConfig, chunk: usize) -> String {
        let l = TcpListener::bind("127.0.0.1:0").expect("bind");
        let addr = l.local_addr().expect("addr").to_string();
        thread::spawn(move || {
            let (mut s, _) = l.accept().expect("accept");
            let mut board = MockBoard::new(cfg);
            let mut buf = [0u8; 4096];
            loop {
                match s.read(&mut buf) {
                    Ok(0) | Err(_) => return,
                    Ok(n) => {
                        let out = board.rx(&buf[..n]);
                        // Trickle the response out in small writes, like a UART.
                        for c in out.chunks(chunk.max(1)) {
                            if s.write_all(c).is_err() {
                                return;
                            }
                            s.flush().ok();
                        }
                    }
                }
            }
        });
        addr
    }

    /// A serial-port-like fake: host writes go to a MockBoard thread, responses
    /// come back one byte at a time through a channel, with real read timeouts.
    pub struct FakeSerial {
        tx: mpsc::Sender<Vec<u8>>,
        rx: mpsc::Receiver<u8>,
        timeout: Duration,
    }

    impl FakeSerial {
        pub fn new(cfg: MockConfig) -> Self {
            let (htx, brx) = mpsc::channel::<Vec<u8>>();
            let (btx, hrx) = mpsc::channel::<u8>();
            thread::spawn(move || {
                let mut board = MockBoard::new(cfg);
                while let Ok(v) = brx.recv() {
                    for b in board.rx(&v) {
                        if btx.send(b).is_err() {
                            return;
                        }
                    }
                }
            });
            Self {
                tx: htx,
                rx: hrx,
                timeout: Duration::from_millis(100),
            }
        }
    }

    impl Read for FakeSerial {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            if buf.is_empty() {
                return Ok(0);
            }
            match self.rx.recv_timeout(self.timeout) {
                Ok(b) => {
                    buf[0] = b;
                    let mut n = 1;
                    while n < buf.len() {
                        match self.rx.try_recv() {
                            Ok(b) => {
                                buf[n] = b;
                                n += 1;
                            }
                            Err(_) => break,
                        }
                    }
                    Ok(n)
                }
                Err(_) => Err(io::Error::new(io::ErrorKind::TimedOut, "timeout")),
            }
        }
    }
    impl Write for FakeSerial {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.tx
                .send(buf.to_vec())
                .map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "board gone"))?;
            Ok(buf.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl TimedIo for FakeSerial {
        fn set_read_timeout(&mut self, t: Duration) -> io::Result<()> {
            self.timeout = t;
            Ok(())
        }
    }

    /// A byte-echo line (the `heartbeat_uart_echo` bitstream), for `echo` tests.
    #[derive(Default)]
    pub struct EchoLine {
        buf: VecDeque<u8>,
    }
    impl Read for EchoLine {
        fn read(&mut self, out: &mut [u8]) -> io::Result<usize> {
            if self.buf.is_empty() {
                return Err(io::Error::new(io::ErrorKind::TimedOut, "timeout"));
            }
            let n = out.len().min(self.buf.len());
            for (i, b) in self.buf.drain(..n).enumerate() {
                out[i] = b;
            }
            Ok(n)
        }
    }
    impl Write for EchoLine {
        fn write(&mut self, b: &[u8]) -> io::Result<usize> {
            self.buf.extend(b);
            Ok(b.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl TimedIo for EchoLine {
        fn set_read_timeout(&mut self, _t: Duration) -> io::Result<()> {
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_transport_specs() {
        assert_eq!("mock".parse::<TransportSpec>(), Ok(TransportSpec::Mock));
        assert_eq!(
            "serial:COM5".parse::<TransportSpec>(),
            Ok(TransportSpec::Serial("COM5".into()))
        );
        assert_eq!(
            "serial:/dev/ttyUSB1".parse::<TransportSpec>(),
            Ok(TransportSpec::Serial("/dev/ttyUSB1".into()))
        );
        assert_eq!(
            "tcp:127.0.0.1:5555".parse::<TransportSpec>(),
            Ok(TransportSpec::Tcp("127.0.0.1:5555".into()))
        );
        assert!("tcp:127.0.0.1".parse::<TransportSpec>().is_err());
        assert!("serial:".parse::<TransportSpec>().is_err());
        assert!("usb".parse::<TransportSpec>().is_err());
    }

    #[test]
    fn missing_serial_port_error_is_clear() {
        let e = open(
            &TransportSpec::Serial("/dev/splitsys-does-not-exist".into()),
            115200,
            None,
            MockConfig::default(),
        )
        .err()
        .expect("must fail");
        assert!(e.contains("cannot open serial port"), "{e}");
        assert!(e.contains("serial port"), "{e}");
    }

    #[test]
    fn tcp_connect_refused_is_clear() {
        // Bind then drop to get a port that is (almost certainly) closed.
        let port = std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let e = open(
            &TransportSpec::Tcp(format!("127.0.0.1:{port}")),
            115200,
            None,
            MockConfig::default(),
        )
        .err()
        .expect("must fail");
        assert!(e.contains("co-sim bridge"), "{e}");
    }
}
