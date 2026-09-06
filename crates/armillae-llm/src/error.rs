use std::time::Duration;

use serde::{Deserialize, Serialize};
use thiserror::Error;

/// Fixed diagnostic categories; never contains a raw client error or URL.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum TransportErrorKind {
    Timeout,
    Connect,
    ConnectionRefused,
    ConnectionReset,
    ConnectionAborted,
    NotConnected,
    HostUnreachable,
    NetworkUnreachable,
    PermissionDenied,
    BrokenPipe,
    UnexpectedEof,
    Body,
    Decode,
    Redirect,
    Request,
    Protocol,
    Unknown,
}

impl TransportErrorKind {
    pub const fn code(self) -> &'static str {
        match self {
            Self::Timeout => "timeout",
            Self::Connect => "connect",
            Self::ConnectionRefused => "connection_refused",
            Self::ConnectionReset => "connection_reset",
            Self::ConnectionAborted => "connection_aborted",
            Self::NotConnected => "not_connected",
            Self::HostUnreachable => "host_unreachable",
            Self::NetworkUnreachable => "network_unreachable",
            Self::PermissionDenied => "permission_denied",
            Self::BrokenPipe => "broken_pipe",
            Self::UnexpectedEof => "unexpected_eof",
            Self::Body => "body",
            Self::Decode => "decode",
            Self::Redirect => "redirect",
            Self::Request => "request",
            Self::Protocol => "protocol",
            Self::Unknown => "unknown",
        }
    }
}

/// Safe, normalized facts attached to a Provider failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ErrorMetadata {
    pub provider: String,
    pub http_status: Option<u16>,
    pub request_id: Option<String>,
    pub transport_kind: Option<TransportErrorKind>,
    pub os_error: Option<i32>,
}

impl ErrorMetadata {
    pub fn new(provider: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
            http_status: None,
            request_id: None,
            transport_kind: None,
            os_error: None,
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

    #[error(
        "request content at message {message_index}, content {content_index} is incompatible with {target_provider} projection: {kind}"
    )]
    ProjectionIncompatible {
        target_provider: String,
        message_index: usize,
        content_index: usize,
        kind: String,
    },

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

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    #[test]
    fn transport_kind_serde_uses_the_stable_diagnostic_code() {
        for kind in [
            TransportErrorKind::Timeout,
            TransportErrorKind::ConnectionRefused,
            TransportErrorKind::ConnectionReset,
            TransportErrorKind::Protocol,
            TransportErrorKind::Unknown,
        ] {
            let encoded = serde_json::to_string(&kind).expect("encode");
            assert_eq!(encoded, format!("\"{}\"", kind.code()));
            assert_eq!(
                serde_json::from_str::<TransportErrorKind>(&encoded).expect("decode"),
                kind
            );
        }
    }
}
