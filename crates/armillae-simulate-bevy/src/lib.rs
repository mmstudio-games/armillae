#![allow(clippy::result_large_err)]

//! Bevy ECS simulation backend for Armillae.
//!
//! This crate adapts native Rust modules and clocks to the backend-neutral
//! contracts owned by `armillae-simulate`. It intentionally exposes neither a
//! main loop nor raw schedule ownership.
//!
//! Native panics are converted to structured simulation failures when the
//! process uses unwind panics. A binary built with `panic = "abort"` cannot
//! recover at these boundaries.
//!
//! World access is closure-scoped, so a borrow cannot escape the operation:
//!
//! ```compile_fail
//! use armillae_simulate_bevy::BevySimulation;
//! use bevy_ecs::world::World;
//!
//! fn leak_world(simulation: &BevySimulation) -> &World {
//!     simulation
//!         .inspect_world(|world| world)
//!         .expect("inspection succeeds")
//! }
//! ```
//!
//! ```compile_fail
//! use armillae_simulate_bevy::BevySimulation;
//! use bevy_ecs::world::World;
//!
//! fn leak_world_mut(simulation: &mut BevySimulation) -> &mut World {
//!     simulation
//!         .write_world(|world| world)
//!         .expect("write succeeds")
//! }
//! ```
//!
//! A World borrow also cannot be held across an async suspension point:
//!
//! ```compile_fail
//! use armillae_simulate_bevy::BevySimulation;
//!
//! async fn hold_across_await(simulation: &BevySimulation) {
//!     simulation
//!         .inspect_world(|world| async move {
//!             std::future::ready(()).await;
//!             world.entities().len()
//!         })
//!         .expect("inspection succeeds")
//!         .await;
//! }
//! ```

mod builder;
mod context;
mod runtime;
mod support;

pub use builder::{BevyModule, BevyModuleRegistrar, BevySimulationBuilder};
pub use context::{AdvanceContext, ClockComponent, ExecuteContext, ExecuteOutputError};
pub use runtime::BevySimulation;
pub use support::{BEVY_BACKEND_ID, BEVY_ENGINE_NAME};
