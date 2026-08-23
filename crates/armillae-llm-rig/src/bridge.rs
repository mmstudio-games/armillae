use std::sync::Arc;

use armillae_core::{CompletionRequest, CompletionResponse, GenerationOptions};
use armillae_llm::{BoxFuture, BridgeCapabilities, BridgeError, CompletionStream, LlmBridge};
use rig_core::completion::CompletionModel;

use crate::{
    request::RigRequestMapper,
    response::{self, RigResponseNormalizer},
};

pub struct RigBridge<M>
where
    M: CompletionModel,
{
    model: M,
    capabilities: BridgeCapabilities,
    defaults: GenerationOptions,
    request_mapper: Arc<dyn RigRequestMapper>,
    normalizer: Arc<dyn RigResponseNormalizer<M::Response>>,
}

impl<M> RigBridge<M>
where
    M: CompletionModel,
{
    pub(crate) fn new(
        model: M,
        capabilities: BridgeCapabilities,
        defaults: GenerationOptions,
        request_mapper: Arc<dyn RigRequestMapper>,
        normalizer: Arc<dyn RigResponseNormalizer<M::Response>>,
    ) -> Result<Self, BridgeError> {
        capabilities.validate()?;
        if capabilities.streaming {
            return Err(BridgeError::InvalidConfiguration {
                message: "non-streaming RigBridge cannot advertise streaming capability".to_owned(),
            });
        }

        Ok(Self {
            model,
            capabilities,
            defaults,
            request_mapper,
            normalizer,
        })
    }
}

impl<M> LlmBridge for RigBridge<M>
where
    M: CompletionModel + Send + Sync + 'static,
    M::Response: Send + Sync,
    M::StreamingResponse: Send + Sync,
{
    fn capabilities(&self) -> BridgeCapabilities {
        self.capabilities
    }

    fn complete<'a>(
        &'a self,
        request: CompletionRequest,
    ) -> BoxFuture<'a, Result<CompletionResponse, BridgeError>> {
        Box::pin(async move {
            self.capabilities.validate_request(&request)?;
            let request = self.request_mapper.map_request(request, &self.defaults)?;
            let response = self
                .model
                .completion(request)
                .await
                .map_err(|error| self.normalizer.normalize_error(error))?;
            response::response_from_rig(response, self.normalizer.as_ref())
        })
    }

    fn stream<'a>(
        &'a self,
        request: CompletionRequest,
    ) -> BoxFuture<'a, Result<CompletionStream, BridgeError>> {
        Box::pin(async move {
            self.capabilities.validate_streaming_request(&request)?;
            Err(BridgeError::UnsupportedCapability {
                capability: "streaming".to_owned(),
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use armillae_core::{
        AssistantContent, CompletionRequest, FinishReason, GenerationOptions, Message, TextContent,
        TokenUsage,
    };
    use armillae_llm::{
        BridgeCapabilities, BridgeError, LlmBridge, OutputFormatCapabilities,
        ToolChoiceCapabilities, mock::contract::verify_completion,
    };
    use futures::stream;
    use rig_core::{
        OneOrMany,
        completion::{
            CompletionError, CompletionModel, CompletionRequest as RigCompletionRequest,
            CompletionResponse as RigCompletionResponse, GetTokenUsage, Usage,
        },
        streaming::{StreamingCompletionResponse, StreamingResult},
    };
    use serde::{Deserialize, Serialize};
    use serde_json::{Value, json};

    use crate::{
        RigBridge,
        request::OpenAiRequestMapper,
        response::{NormalizedResponseFacts, RigResponseNormalizer},
    };

    #[derive(Clone, Debug, Deserialize, Serialize)]
    struct ProbeResponse;

    impl GetTokenUsage for ProbeResponse {
        fn token_usage(&self) -> Usage {
            Usage::default()
        }
    }

    #[derive(Clone, Default)]
    struct ProbeModel {
        requests: Arc<Mutex<Vec<RigCompletionRequest>>>,
    }

    impl CompletionModel for ProbeModel {
        type Response = ProbeResponse;
        type StreamingResponse = ProbeResponse;
        type Client = ();

        fn make(_client: &Self::Client, _model: impl Into<String>) -> Self {
            Self::default()
        }

        async fn completion(
            &self,
            request: RigCompletionRequest,
        ) -> Result<RigCompletionResponse<Self::Response>, CompletionError> {
            self.requests
                .lock()
                .expect("the probe request lock must not be poisoned")
                .push(request);
            Ok(RigCompletionResponse {
                choice: OneOrMany::one(rig_core::message::AssistantContent::text("hello")),
                usage: Usage {
                    input_tokens: 3,
                    output_tokens: 2,
                    total_tokens: 5,
                    ..Usage::default()
                },
                raw_response: ProbeResponse,
                message_id: None,
            })
        }

        async fn stream(
            &self,
            _request: RigCompletionRequest,
        ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
            let inner: StreamingResult<ProbeResponse> = Box::pin(stream::empty());
            Ok(StreamingCompletionResponse::stream(inner))
        }
    }

    struct ProbeNormalizer;

    impl RigResponseNormalizer<ProbeResponse> for ProbeNormalizer {
        fn provider(&self) -> &str {
            "probe"
        }

        fn normalize(
            &self,
            _raw_response: &ProbeResponse,
        ) -> Result<NormalizedResponseFacts, BridgeError> {
            Ok(NormalizedResponseFacts {
                id: Some("response-1".to_owned()),
                model: Some("probe-model".to_owned()),
                finish_reason: Some(FinishReason::Stop),
                provider_metadata: Value::Object(Default::default()),
            })
        }
    }

    fn capabilities() -> BridgeCapabilities {
        BridgeCapabilities {
            streaming: false,
            tool_calling: true,
            parallel_tool_calls: true,
            tool_choice: ToolChoiceCapabilities::all(),
            output_format: OutputFormatCapabilities::all(),
            system_message: true,
            developer_message: false,
        }
    }

    fn bridge(model: ProbeModel) -> RigBridge<ProbeModel> {
        RigBridge::new(
            model,
            capabilities(),
            GenerationOptions {
                temperature: Some(0.25),
                ..GenerationOptions::default()
            },
            Arc::new(OpenAiRequestMapper::default()),
            Arc::new(ProbeNormalizer),
        )
        .expect("valid probe bridge must construct")
    }

    #[test]
    fn completion_uses_mapper_model_and_normalizer() {
        futures::executor::block_on(async {
            let model = ProbeModel::default();
            let bridge = bridge(model.clone());
            let request = CompletionRequest {
                messages: vec![Message::user("hello")],
                ..CompletionRequest::default()
            };
            let expected = armillae_core::CompletionResponse {
                id: Some("response-1".to_owned()),
                model: Some("probe-model".to_owned()),
                content: vec![AssistantContent::Text(TextContent::new("hello"))],
                finish_reason: Some(FinishReason::Stop),
                usage: Some(TokenUsage {
                    input_tokens: Some(3),
                    output_tokens: Some(2),
                    total_tokens: Some(5),
                    cached_input_tokens: Some(0),
                }),
                provider_metadata: json!({}),
            };

            verify_completion(&bridge, request, &expected)
                .await
                .expect("RigBridge must satisfy the shared completion contract");

            let requests = model
                .requests
                .lock()
                .expect("the probe request lock must not be poisoned");
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].temperature, Some(0.25));
        });
    }

    #[test]
    fn capability_preflight_runs_before_model_invocation() {
        futures::executor::block_on(async {
            let model = ProbeModel::default();
            let bridge = bridge(model.clone());
            let request = CompletionRequest {
                messages: vec![Message::new(
                    armillae_core::Role::Developer,
                    vec![armillae_core::ContentPart::text("hidden")],
                )],
                ..CompletionRequest::default()
            };

            assert_eq!(
                bridge
                    .complete(request)
                    .await
                    .expect_err("Developer role must fail preflight"),
                BridgeError::UnsupportedCapability {
                    capability: "role.developer".to_owned(),
                }
            );
            assert!(
                model
                    .requests
                    .lock()
                    .expect("the probe request lock must not be poisoned")
                    .is_empty()
            );
        });
    }

    #[test]
    fn p4_bridge_cannot_advertise_or_fake_streaming() {
        let model = ProbeModel::default();
        let mut invalid_capabilities = capabilities();
        invalid_capabilities.streaming = true;

        assert!(matches!(
            RigBridge::new(
                model.clone(),
                invalid_capabilities,
                GenerationOptions::default(),
                Arc::new(OpenAiRequestMapper::default()),
                Arc::new(ProbeNormalizer),
            ),
            Err(BridgeError::InvalidConfiguration { .. })
        ));

        let bridge = bridge(model);
        let result = futures::executor::block_on(bridge.stream(CompletionRequest {
            messages: vec![Message::user("hello")],
            ..CompletionRequest::default()
        }));
        let error = match result {
            Ok(_) => panic!("P4 must not synthesize a stream"),
            Err(error) => error,
        };
        assert_eq!(
            error,
            BridgeError::UnsupportedCapability {
                capability: "streaming".to_owned(),
            }
        );
    }
}
