use std::sync::Arc;

use crate::{BoxFuture, BridgeError, LlmBridge, ResolvedBridgeConfig};

/// Constructs a runtime-selected Bridge from validated, resolved configuration.
pub trait BridgeFactory: Send + Sync {
    fn driver(&self) -> &'static str;

    fn create<'a>(
        &'a self,
        config: ResolvedBridgeConfig,
    ) -> BoxFuture<'a, Result<Arc<dyn LlmBridge>, BridgeError>>;
}
