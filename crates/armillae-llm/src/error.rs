use std::time::Duration;

use thiserror::Error;

/// Safe, normalized facts attached to a Provider failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErrorMetadata {
    pub provider: String,
    pub http_status: Option<u16>,
    pub request_id: Option<String>,
}

impl ErrorMetadata {
    pub fn new(provider: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            http_status: None,
            request_id: None,
        }
    }

    pub fn with_http_status(mut self, http_status: u16) -> Self {
        self.http_status = Some(http_status);
        self
    }

    pub fn with_request_id(mut self, request_id: impl Into<String>) -> Self {
        self.request_id = Some(request_id.into());
        self
    }
}

/// A normalized failure from Bridge construction or one model call.
#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[non_exhaustive]
pub enum BridgeError {
    #[error("invalid bridge configuration: {message}")]
    InvalidConfiguration { message: String },

    #[error("unsupported capability: {capability}")]
    UnsupportedCapability { capability: String },

    #[error("invalid request: {message}")]
    InvalidRequest { message: String },

    #[error("authentication failed")]
    Authentication { metadata: ErrorMetadata },

    #[error("permission denied")]
    PermissionDenied { metadata: ErrorMetadata },

    #[error("rate limited")]
    RateLimited {
        retry_after: Option<Duration>,
        metadata: ErrorMetadata,
    },

    #[error("request timed out")]
    Timeout { metadata: ErrorMetadata },

    #[error("request cancelled")]
    Cancelled,

    #[error("transport error")]
    Transport {
        retryable: bool,
        metadata: ErrorMetadata,
    },

    #[error("provider rejected request: {message}")]
    ProviderRejected {
        code: Option<String>,
        message: String,
        metadata: ErrorMetadata,
    },

    #[error("invalid provider response: {message}")]
    InvalidProviderResponse {
        message: String,
        metadata: ErrorMetadata,
    },

    #[error("stream interrupted")]
    StreamInterrupted { metadata: ErrorMetadata },
}
