//! SplitSys contract constants.
//!
//! The constants at the crate root (`splitsys_spec::ROWS`, `L_PE`, `OP_PING`,
//! `COUNTER_NAMES`, ...) are generated from `spec/spec.toml` into
//! `src/generated.rs` by `cargo run -p splitsys-spec --bin gen`. Never edit
//! `generated.rs` by hand. The parsed, validated spec is also available at run
//! time through [`model::Spec`].

#![forbid(unsafe_code)]

mod generated;
pub mod model;

pub use generated::*;
pub use model::{crc8, fletcher32, Spec, SpecError};
