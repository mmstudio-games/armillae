use armillae_core::{
    AssistantContent, CompletionResponse as ArmillaeCompletionResponse, FinishReason,
};
use armillae_llm::{BridgeError, ErrorMetadata, TransportErrorKind};
use rig_core::{
    completion::{CompletionError, CompletionResponse as RigCompletionResponse},
    providers::openai,
};
use serde_json::{Map, Value};

use crate::convert;

#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NormalizedResponseFacts {
    pub(crate) id: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) finish_reason: Option<FinishReason>,
    pub(crate) provider_metadata: Value,
}

pub(crate) trait RigResponseNormalizer<R>: Send + Sync {
    fn provider(&self) -> &str;

    fn normalize(&self, raw_response: &R) -> Result<NormalizedResponseFacts, BridgeError>;

    fn normalize_content(
        &self,
        content: Vec<AssistantContent>,
    ) -> Result<Vec<AssistantContent>, BridgeError> {
        Ok(content)
    }

    fn normalize_error(&self, error: CompletionError) -> BridgeError {
        normalize_completion_error(self.provider(), error)
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NormalizedStreamingResponseFacts {
    pub(crate) finish_reason: Option<FinishReason>,
    pub(crate) provider_metadata: Value,
}

pub(crate) trait RigStreamingResponseNormalizer<R>: Send + Sync {
    fn normalize(&self, raw_response: &R) -> Result<NormalizedStreamingResponseFacts, ()>;
}

#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct NoopStreamingResponseNormalizer;

impl<R> RigStreamingResponseNormalizer<R> for NoopStreamingResponseNormalizer {
    fn normalize(&self, _raw_response: &R) -> Result<NormalizedStreamingResponseFacts, ()> {
        Ok(NormalizedStreamingResponseFacts {
            finish_reason: None,
            provider_metadata: Value::Object(Map::new()),
        })
    }
}

#[derive(Clone, Debug)]
pub(crate) struct OpenAiResponseNormalizer {
    provider: String,
}

impl OpenAiResponseNormalizer {
    pub(crate) fn new(provider: impl Into<String>) -> Self {
        Self {
            provider: provider.into(),
        }
    }
}

impl RigResponseNormalizer<openai::completion::CompletionResponse> for OpenAiResponseNormalizer {
    fn provider(&self) -> &str {
        &self.provider
    }

    fn normalize(
        &self,
        raw_response: &openai::completion::CompletionResponse,
    ) -> Result<NormalizedResponseFacts, BridgeError> {
        if raw_response.id.trim().is_empty() {
            return invalid_provider_response(&self.provider, "OpenAI response id is empty");
        }
        if raw_response.model.trim().is_empty() {
            return invalid_provider_response(&self.provider, "OpenAI response model is empty");
        }
        let choice =
            raw_response
                .choices
                .first()
                .ok_or_else(|| BridgeError::InvalidProviderResponse {
                    message: "OpenAI response contained no choices".to_owned(),
                    metadata: ErrorMetadata::new(&self.provider),
                })?;

        let mut metadata = Map::new();
        if let Some(fingerprint) = &raw_response.system_fingerprint {
            metadata.insert(
                "system_fingerprint".to_owned(),
                Value::String(fingerprint.clone()),
            );
        }

        Ok(NormalizedResponseFacts {
            id: Some(raw_response.id.clone()),
            model: Some(raw_response.model.clone()),
            finish_reason: Some(openai_finish_reason(&choice.finish_reason)),
            provider_metadata: Value::Object(metadata),
        })
    }
}

pub(crate) fn response_from_rig<R>(
    response: RigCompletionResponse<R>,
    normalizer: &dyn RigResponseNormalizer<R>,
) -> Result<ArmillaeCompletionResponse, BridgeError> {
    let facts = normalizer.normalize(&response.raw_response)?;
    let content = convert::assistant_content_from_rig(response.choice, normalizer.provider())?;
    let content = normalizer.normalize_content(content)?;

    Ok(ArmillaeCompletionResponse {
        id: facts.id,
        model: facts.model,
        content,
        finish_reason: facts.finish_reason,
        usage: convert::usage_from_rig(response.usage),
        provider_metadata: facts.provider_metadata,
    })
}

fn openai_finish_reason(reason: &str) -> FinishReason {
    match reason {
        "stop" => FinishReason::Stop,
        "length" | "max_tokens" => FinishReason::Length,
        "tool_calls" | "function_call" => FinishReason::ToolCall,
        "content_filter" => FinishReason::ContentFilter,
        "cancelled" => FinishReason::Cancelled,
        other => FinishReason::Unknown(other.to_owned()),
    }
}

fn normalize_completion_error(provider: &str, error: CompletionError) -> BridgeError {
    let mut facts = ErrorMetadata::new(provider);
    facts.http_status = error
        .provider_response_status()
        .map(|status| status.as_u16());
    if let CompletionError::HttpError(http_error) = &error {
        classify_http_error(http_error, &mut facts);
    }
    let http_status = facts.http_status;
    let metadata = || facts.clone();

    match http_status {
        Some(401) => BridgeError::Authentication {
            metadata: metadata(),
        },
        Some(403) => BridgeError::PermissionDenied {
            metadata: metadata(),
        },
        Some(408 | 504) => BridgeError::Timeout {
            metadata: metadata(),
        },
        Some(429) => BridgeError::RateLimited {
            retry_after: None,
            metadata: metadata(),
        },
        Some(status) if (500..=599).contains(&status) => BridgeError::Transport {
            retryable: true,
            metadata: metadata(),
        },
        Some(_) => BridgeError::ProviderRejected {
            code: None,
            message: "provider rejected the completion request".to_owned(),
            metadata: metadata(),
        },
        None => match error {
            CompletionError::ResponseError(_) | CompletionError::JsonError(_) => {
                BridgeError::InvalidProviderResponse {
                    message: "provider returned an invalid completion response".to_owned(),
                    metadata: metadata(),
                }
            }
            CompletionError::UrlError(_) => BridgeError::InvalidConfiguration {
                message: "provider endpoint URL is invalid".to_owned(),
            },
            CompletionError::RequestError(_) => BridgeError::InvalidRequest {
                message: "provider request could not be constructed".to_owned(),
            },
            CompletionError::HttpError(_)
                if facts.transport_kind == Some(TransportErrorKind::Timeout) =>
            {
                BridgeError::Timeout {
                    metadata: metadata(),
                }
            }
            CompletionError::HttpError(_) => BridgeError::Transport {
                retryable: true,
                metadata: metadata(),
            },
            CompletionError::ProviderError(_) | CompletionError::ProviderResponse(_) => {
                BridgeError::ProviderRejected {
                    code: None,
                    message: "provider returned an error".to_owned(),
                    metadata: metadata(),
                }
            }
            _ => BridgeError::Transport {
                retryable: false,
                metadata: metadata(),
            },
        },
    }
}

fn classify_http_error(error: &rig_core::http_client::Error, metadata: &mut ErrorMetadata) {
    use rig_core::http_client::Error;
    match error {
        Error::InvalidStatusCode(status) | Error::InvalidStatusCodeWithMessage(status, _) => {
            metadata.http_status = Some(status.as_u16());
        }
        Error::Instance(source) => {
            let mut current: Option<&(dyn std::error::Error + 'static)> = Some(source.as_ref());
            let mut kind = TransportErrorKind::Unknown;
            let mut io_kind = None;
            let mut timed_out = false;
            // Bound traversal even for a custom client's cyclic error chain.
            for _ in 0..16 {
                let Some(error) = current else {
                    break;
                };
                if let Some(error) = error.downcast_ref::<reqwest::Error>() {
                    metadata.http_status = metadata
                        .http_status
                        .or(error.status().map(|status| status.as_u16()));
                    timed_out |= error.is_timeout();
                    kind = if error.is_connect() {
                        TransportErrorKind::Connect
                    } else if error.is_body() {
                        TransportErrorKind::Body
                    } else if error.is_decode() {
                        TransportErrorKind::Decode
                    } else if error.is_redirect() {
                        TransportErrorKind::Redirect
                    } else if error.is_request() {
                        TransportErrorKind::Request
                    } else {
                        TransportErrorKind::Unknown
                    };
                }
                if let Some(error) = error.downcast_ref::<std::io::Error>() {
                    use std::io::ErrorKind;
                    metadata.os_error = metadata.os_error.or(error.raw_os_error());
                    timed_out |= error.kind() == ErrorKind::TimedOut;
                    let classified = match error.kind() {
                        ErrorKind::ConnectionRefused => Some(TransportErrorKind::ConnectionRefused),
                        ErrorKind::ConnectionReset => Some(TransportErrorKind::ConnectionReset),
                        ErrorKind::ConnectionAborted => Some(TransportErrorKind::ConnectionAborted),
                        ErrorKind::NotConnected => Some(TransportErrorKind::NotConnected),
                        ErrorKind::HostUnreachable => Some(TransportErrorKind::HostUnreachable),
                        ErrorKind::NetworkUnreachable => {
                            Some(TransportErrorKind::NetworkUnreachable)
                        }
                        ErrorKind::PermissionDenied => Some(TransportErrorKind::PermissionDenied),
                        ErrorKind::BrokenPipe => Some(TransportErrorKind::BrokenPipe),
                        ErrorKind::UnexpectedEof => Some(TransportErrorKind::UnexpectedEof),
                        _ => None,
                    };
                    io_kind = classified.or(io_kind);
                }
                current = error.source();
            }
            let kind = if timed_out {
                TransportErrorKind::Timeout
            } else {
                io_kind.unwrap_or(kind)
            };
            if metadata.http_status.is_none() || kind != TransportErrorKind::Unknown {
                metadata.transport_kind = Some(kind);
            }
        }
        Error::Protocol(_) | Error::InvalidHeaderValue(_) | Error::NoHeaders => {
            metadata.transport_kind = Some(TransportErrorKind::Protocol);
        }
        Error::StreamEnded => metadata.transport_kind = Some(TransportErrorKind::UnexpectedEof),
        Error::InvalidContentType(_) => metadata.transport_kind = Some(TransportErrorKind::Decode),
    }
}

fn invalid_provider_response<T>(
    provider: &str,
    message: impl Into<String>,
) -> Result<T, BridgeError> {
    Err(BridgeError::InvalidProviderResponse {
        message: message.into(),
        metadata: ErrorMetadata::new(provider),
    })
}

#[cfg(test)]
mod tests {
    use armillae_core::{AssistantContent, FinishReason};
    use armillae_llm::{BridgeError, ErrorMetadata};
    use rig_core::{
        OneOrMany,
        completion::{CompletionError, CompletionResponse as RigCompletionResponse, Usage},
        providers::openai,
    };
    use serde_json::json;

    use super::{OpenAiResponseNormalizer, RigResponseNormalizer, response_from_rig};

    fn raw_response(finish_reason: &str) -> openai::completion::CompletionResponse {
        serde_json::from_value(json!({
            "id": "chatcmpl-1",
            "object": "chat.completion",
            "created": 1,
            "model": "gpt-test",
            "system_fingerprint": "fp-test",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": "hello"
                },
                "logprobs": null,
                "finish_reason": finish_reason
            }],
            "usage": {
                "prompt_tokens": 3,
                "completion_tokens": 2,
                "total_tokens": 5
            }
        }))
        .expect("OpenAI response fixture must deserialize")
    }

    #[test]
    fn openai_normalizer_uses_raw_response_facts() {
        let response = RigCompletionResponse {
            choice: OneOrMany::one(rig_core::message::AssistantContent::text("hello")),
            usage: Usage {
                input_tokens: 3,
                output_tokens: 2,
                total_tokens: 5,
                cached_input_tokens: 1,
                ..Usage::default()
            },
            raw_response: raw_response("tool_calls"),
            message_id: None,
        };

        let normalized = response_from_rig(response, &OpenAiResponseNormalizer::new("openai"))
            .expect("valid OpenAI response must normalize");

        assert_eq!(normalized.id.as_deref(), Some("chatcmpl-1"));
        assert_eq!(normalized.model.as_deref(), Some("gpt-test"));
        assert_eq!(normalized.finish_reason, Some(FinishReason::ToolCall));
        assert_eq!(
            normalized.provider_metadata["system_fingerprint"],
            "fp-test"
        );
        assert_eq!(
            normalized.usage.expect("usage must exist").total_tokens,
            Some(5)
        );
        assert!(
            matches!(&normalized.content[0], AssistantContent::Text(text) if text.text == "hello")
        );
    }

    #[test]
    fn unknown_finish_reason_is_preserved_without_guessing() {
        let facts = OpenAiResponseNormalizer::new("openai")
            .normalize(&raw_response("future_reason"))
            .expect("unknown finish reason remains a valid response");

        assert_eq!(
            facts.finish_reason,
            Some(FinishReason::Unknown("future_reason".to_owned()))
        );
    }

    #[test]
    fn empty_required_openai_facts_are_invalid_provider_responses() {
        let mut raw = raw_response("stop");
        raw.id.clear();

        assert!(matches!(
            OpenAiResponseNormalizer::new("openai").normalize(&raw),
            Err(BridgeError::InvalidProviderResponse { .. })
        ));
    }

    #[test]
    fn completion_errors_are_classified_without_raw_provider_text() {
        let normalizer = OpenAiResponseNormalizer::new("openai");
        let error = normalizer.normalize_error(CompletionError::ProviderError(
            "secret response body".to_owned(),
        ));

        assert_eq!(
            error,
            BridgeError::ProviderRejected {
                code: None,
                message: "provider returned an error".to_owned(),
                metadata: ErrorMetadata::new("openai"),
            }
        );
        assert!(!error.to_string().contains("secret response body"));
        assert!(!format!("{error:?}").contains("secret response body"));
    }
}

#[cfg(test)]
mod transport_diagnostic_tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        time::Duration,
    };

    fn assert_safe(error: &BridgeError) {
        let text = format!("{error:?} {error}");
        assert!(!text.contains("private-response-marker"));
        assert!(!text.contains("private-url-marker"));
        assert!(!text.contains("private-source-marker"));
    }

    #[tokio::test]
    async fn reqwest_status_errors_retain_status_without_url_or_body() {
        for status in [400, 401, 403, 408, 429, 500, 503, 504] {
            let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
            let address = listener.local_addr().expect("address");
            let server = std::thread::spawn(move || {
                let (mut stream, _) = listener.accept().expect("accept");
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .expect("timeout");
                let mut buffer = [0; 4096];
                assert!(stream.read(&mut buffer).expect("request") > 0);
                let body = "private-response-marker";
                write!(stream, "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).expect("response");
            });
            let response = reqwest::Client::builder()
                .no_proxy()
                .build()
                .expect("client")
                .get(format!("http://{address}/private-url-marker"))
                .send()
                .await
                .expect("response");
            let error = response.error_for_status().expect_err("HTTP error");
            let error = normalize_completion_error(
                "deepseek",
                CompletionError::HttpError(rig_core::http_client::Error::Instance(Box::new(error))),
            );
            server.join().expect("server");
            assert_safe(&error);
            let metadata = match (&error, status) {
                (BridgeError::Authentication { metadata }, 401)
                | (BridgeError::PermissionDenied { metadata }, 403)
                | (BridgeError::RateLimited { metadata, .. }, 429)
                | (BridgeError::Timeout { metadata }, 408 | 504)
                | (
                    BridgeError::Transport {
                        metadata,
                        retryable: true,
                    },
                    500 | 503,
                )
                | (BridgeError::ProviderRejected { metadata, .. }, 400) => metadata,
                _ => panic!("incorrect category: {error:?}"),
            };
            assert_eq!(metadata.http_status, Some(status));
        }
    }

    #[tokio::test]
    async fn refused_connection_keeps_typed_cause_without_url() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        drop(listener);
        let error = reqwest::Client::builder()
            .no_proxy()
            .build()
            .expect("client")
            .get(format!("http://{address}/private-url-marker"))
            .send()
            .await
            .expect_err("connection refused");
        let error = normalize_completion_error(
            "deepseek",
            CompletionError::HttpError(rig_core::http_client::Error::Instance(Box::new(error))),
        );
        assert_safe(&error);
        let BridgeError::Transport { metadata, .. } = error else {
            panic!("transport");
        };
        assert_eq!(metadata.http_status, None);
        assert_eq!(
            metadata.transport_kind,
            Some(TransportErrorKind::ConnectionRefused)
        );
        assert!(metadata.os_error.is_some());
    }

    #[tokio::test]
    async fn client_timeout_is_classified_without_inventing_http_status() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("listener");
        let address = listener.local_addr().expect("address");
        // The listener stays alive but never returns an HTTP response.
        let error = reqwest::Client::builder()
            .no_proxy()
            .timeout(Duration::from_millis(50))
            .build()
            .expect("client")
            .get(format!("http://{address}/private-url-marker"))
            .send()
            .await
            .expect_err("timeout");
        let error = normalize_completion_error(
            "deepseek",
            CompletionError::HttpError(rig_core::http_client::Error::Instance(Box::new(error))),
        );
        assert_safe(&error);
        let BridgeError::Timeout { metadata } = error else {
            panic!("timeout");
        };
        assert_eq!(metadata.http_status, None);
        assert_eq!(metadata.transport_kind, Some(TransportErrorKind::Timeout));
    }

    #[test]
    fn typed_io_error_is_safe_and_unknown_sources_are_not_guessed() {
        for (kind, expected) in [
            (
                std::io::ErrorKind::ConnectionReset,
                TransportErrorKind::ConnectionReset,
            ),
            (std::io::ErrorKind::Other, TransportErrorKind::Unknown),
        ] {
            let error = normalize_completion_error(
                "deepseek",
                CompletionError::HttpError(rig_core::http_client::Error::Instance(Box::new(
                    std::io::Error::new(kind, "private-source-marker"),
                ))),
            );
            assert_safe(&error);
            let BridgeError::Transport { metadata, .. } = error else {
                panic!("transport");
            };
            assert_eq!(metadata.http_status, None);
            assert_eq!(metadata.transport_kind, Some(expected));
        }
    }
}
