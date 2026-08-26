use armillae_core::{CompletionEvent, CompletionRequest, CompletionResponse};
pub use futures_util::future::BoxFuture;
use futures_util::stream::Stream;

use crate::{BridgeCapabilities, BridgeError, ProjectionReport};

/// A boxed semantic completion-event stream.
pub type CompletionStream =
    std::pin::Pin<Box<dyn Stream<Item = Result<CompletionEvent, BridgeError>> + Send>>;

/// A runtime-selected Bridge that performs exactly one model call.
pub trait LlmBridge: Send + Sync {
    fn capabilities(&self) -> BridgeCapabilities;

    /// Validate and project a canonical request without performing network I/O.
    fn project(&self, request: &CompletionRequest) -> Result<ProjectionReport, BridgeError>;

    fn complete<'a>(
        &'a self,
        request: CompletionRequest,
    ) -> BoxFuture<'a, Result<CompletionResponse, BridgeError>>;

    fn stream<'a>(
        &'a self,
        request: CompletionRequest,
    ) -> BoxFuture<'a, Result<CompletionStream, BridgeError>>;
}
