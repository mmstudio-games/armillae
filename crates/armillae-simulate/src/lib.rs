//! Backend-neutral simulation protocols and runtime contracts for Armillae.
//!
//! This crate owns one-call execution, clock advancement, module descriptors,
//! lifecycle, and structured errors. It does not own a main loop, agent/tool
//! scheduling, persistence, or a backend-native world.

mod clock;
mod error;
mod id;
mod protocol;
mod simulation;
mod version;

#[cfg(feature = "testing")]
pub mod testing;

pub use clock::*;
pub use error::*;
pub use id::*;
pub use protocol::*;
pub use simulation::*;
pub use version::*;
