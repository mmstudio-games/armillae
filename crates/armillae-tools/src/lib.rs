//! Type-safe Tool authoring and single-call execution for Armillae.

mod context;
mod dyn_tool;
mod error;
mod executor;
mod output;
mod registry;
mod tool;

pub use context::ToolContext;
pub use dyn_tool::{BoxFuture, DynTool};
pub use error::{ToolExecutionError, ToolRegistryError};
pub use executor::ToolExecutor;
pub use output::{IntoToolOutput, ToolOutput};
pub use registry::{ToolRegistry, ToolRegistryBuilder};
pub use tool::Tool;

#[doc(hidden)]
pub mod __private {
    pub use schemars;
    pub use serde;
}
