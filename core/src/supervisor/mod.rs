//! The supervisor (hub): launches nodes, routes frames, monitors health (docs/01).

pub mod proc;
pub mod queue;
pub mod router;
pub mod run;
pub mod session;
pub mod signal;
pub mod tool;

pub use run::{run, LogLevel, RunOptions};
