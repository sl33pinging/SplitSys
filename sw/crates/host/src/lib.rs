//! `splitsys_host`: SplitSys host library — framing (SPEC §3.8), transports
//! (serial / tcp co-sim / in-process mock), protocol driver with CRC retry, mock
//! board, GEMM runner with bit-exact golden checking, run records (DEMO.md §4),
//! and the dashboard server.

#![forbid(unsafe_code)]

pub mod client;
pub mod frame;
pub mod mock;
pub mod record;
pub mod runner;
pub mod transport;
pub mod server;
