use std::{collections::HashMap, fmt, sync::Arc};

use armillae_core::{ToolCall, ToolDefinition, ToolResult};

use crate::{BoxFuture, DynTool, ToolContext, ToolExecutionError, ToolExecutor, ToolRegistryError};

/// A mutable collection of uniquely named, dynamically dispatched Tools.
#[derive(Default)]
pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn DynTool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn builder() -> ToolRegistryBuilder {
        ToolRegistryBuilder::default()
    }

    pub fn register<T>(&mut self, tool: T) -> Result<(), ToolRegistryError>
    where
        T: DynTool + 'static,
    {
        self.register_arc(Arc::new(tool))
    }

    pub fn register_arc(&mut self, tool: Arc<dyn DynTool>) -> Result<(), ToolRegistryError> {
        let name = tool.definition().name;
        if self.tools.contains_key(&name) {
            return Err(ToolRegistryError::DuplicateTool { name });
        }
        self.tools.insert(name, tool);
        Ok(())
    }

    pub fn unregister(&mut self, name: &str) -> Option<Arc<dyn DynTool>> {
        self.tools.remove(name)
    }

    pub fn get(&self, name: &str) -> Option<&Arc<dyn DynTool>> {
        self.tools.get(name)
    }

    pub fn contains(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    pub fn len(&self) -> usize {
        self.tools.len()
    }

    pub fn is_empty(&self) -> bool {
        self.tools.is_empty()
    }
}

impl ToolExecutor for ToolRegistry {
    fn definitions(&self) -> Vec<ToolDefinition> {
        let mut definitions = self
            .tools
            .values()
            .map(|tool| tool.definition())
            .collect::<Vec<_>>();
        definitions.sort_by(|left, right| left.name.cmp(&right.name));
        definitions
    }

    fn execute<'a>(
        &'a self,
        context: ToolContext,
        call: ToolCall,
    ) -> BoxFuture<'a, Result<ToolResult, ToolExecutionError>> {
        Box::pin(async move {
            let tool =
                self.tools
                    .get(&call.name)
                    .ok_or_else(|| ToolExecutionError::UnknownTool {
                        name: call.name.clone(),
                    })?;
            let output = tool.call_json(context, call.arguments).await?;
            Ok(ToolResult {
                call_id: call.id,
                content: output.content,
                is_error: false,
            })
        })
    }
}

impl fmt::Debug for ToolRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut names = self.tools.keys().collect::<Vec<_>>();
        names.sort();
        formatter
            .debug_struct("ToolRegistry")
            .field("tool_names", &names)
            .finish()
    }
}

/// A fluent builder that rejects duplicate Tool names.
#[derive(Default)]
pub struct ToolRegistryBuilder {
    registry: ToolRegistry,
}

impl ToolRegistryBuilder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn register<T>(mut self, tool: T) -> Result<Self, ToolRegistryError>
    where
        T: DynTool + 'static,
    {
        self.registry.register(tool)?;
        Ok(self)
    }

    pub fn register_arc(mut self, tool: Arc<dyn DynTool>) -> Result<Self, ToolRegistryError> {
        self.registry.register_arc(tool)?;
        Ok(self)
    }

    pub fn build(self) -> ToolRegistry {
        self.registry
    }
}
