//! PSL core library: parser, checker, protocol and supervisor.
//!
//! The crate has no dependencies by design (ADR 0004).

pub mod check;
pub mod cli;
pub mod diag;
pub mod embedded;
pub mod json;
pub mod model;
pub mod protocol;
pub mod supervisor;
pub mod syntax;
pub mod util;

/// Version of the `psl` binary.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Wire protocol version this core speaks (docs/03).
pub const PROTOCOL_VERSION: u8 = 1;
/// Grammar version (docs/04).
pub const GRAMMAR_VERSION: u32 = 1;
/// Manifest schema version (docs/04).
pub const MANIFEST_VERSION: u32 = 1;
/// Recording format version (docs/03).
pub const RECORDING_VERSION: u32 = 1;
