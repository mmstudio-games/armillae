use armillae_core::ToolDefinition;
pub use futures_util::future::BoxFuture;

use crate::{IntoToolOutput, Tool, ToolContext, ToolExecutionError, ToolOutput};

/// The object-safe Tool boundary stored by registries.
pub trait DynTool: Send + Sync {
    fn definition(&self) -> ToolDefinition;

    fn call_json<'a>(
        &'a self,
        context: ToolContext,
        arguments: serde_json::Value,
    ) -> BoxFuture<'a, Result<ToolOutput, ToolExecutionError>>;
}

impl<T> DynTool for T
where
    T: Tool,
{
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: T::NAME.to_owned(),
            description: self.description().into_owned(),
            input_schema: schemars::schema_for!(T::Args).to_value(),
        }
    }

    fn call_json<'a>(
        &'a self,
        context: ToolContext,
        arguments: serde_json::Value,
    ) -> BoxFuture<'a, Result<ToolOutput, ToolExecutionError>> {
        Box::pin(async move {
            let args = serde_json::from_value(arguments).map_err(|error| {
                ToolExecutionError::InvalidArguments {
                    name: T::NAME.to_owned(),
                    message: error.to_string(),
                }
            })?;
            let output = self.call(context, args).await.map_err(|error| {
                ToolExecutionError::ExecutionFailed {
                    name: T::NAME.to_owned(),
                    message: error.to_string(),
                }
            })?;
            output.into_tool_output()
        })
    }
}
