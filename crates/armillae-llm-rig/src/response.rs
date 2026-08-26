use armillae_core::{
    AssistantContent, CompletionResponse as ArmillaeCompletionResponse, FinishReason,
};
use armillae_llm::{BridgeError, ErrorMetadata};
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
    let http_status = error
        .provider_response_status()
        .map(|status| status.as_u16());
    let metadata = || {
        let metadata = ErrorMetadata::new(provider);
        http_status.map_or(metadata.clone(), |status| metadata.with_http_status(status))
    };

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
