use std::{
    any::{Any, TypeId},
    collections::HashMap,
    fmt,
    sync::Arc,
};

type ExtensionValue = Arc<dyn Any + Send + Sync>;

/// Host-only, type-safe data passed to a Tool invocation.
#[derive(Clone, Default)]
pub struct ToolContext {
    extensions: HashMap<TypeId, ExtensionValue>,
}

impl ToolContext {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_extension<T>(mut self, value: T) -> Self
    where
        T: Send + Sync + 'static,
    {
        self.insert(value);
        self
    }

    pub fn insert<T>(&mut self, value: T) -> Option<Arc<T>>
    where
        T: Send + Sync + 'static,
    {
        self.extensions
            .insert(TypeId::of::<T>(), Arc::new(value))
            .and_then(|previous| previous.downcast::<T>().ok())
    }

    pub fn get<T>(&self) -> Option<&T>
    where
        T: Send + Sync + 'static,
    {
        self.extensions
            .get(&TypeId::of::<T>())
            .and_then(|value| value.downcast_ref::<T>())
    }

    pub fn get_arc<T>(&self) -> Option<Arc<T>>
    where
        T: Send + Sync + 'static,
    {
        self.extensions
            .get(&TypeId::of::<T>())
            .cloned()
            .and_then(|value| value.downcast::<T>().ok())
    }

    pub fn contains<T>(&self) -> bool
    where
        T: Send + Sync + 'static,
    {
        self.extensions.contains_key(&TypeId::of::<T>())
    }

    pub fn remove<T>(&mut self) -> Option<Arc<T>>
    where
        T: Send + Sync + 'static,
    {
        self.extensions
            .remove(&TypeId::of::<T>())
            .and_then(|value| value.downcast::<T>().ok())
    }
}

impl fmt::Debug for ToolContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ToolContext")
            .field("extension_count", &self.extensions.len())
            .finish_non_exhaustive()
    }
}
