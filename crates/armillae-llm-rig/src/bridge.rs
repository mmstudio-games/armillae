use std::sync::Arc;

use armillae_core::{CompletionRequest, CompletionResponse, GenerationOptions};
use armillae_llm::{
    BoxFuture, BridgeCapabilities, BridgeError, CompletionStream, LlmBridge, ProjectionReport,
};
use rig_core::completion::CompletionModel;

use crate::{
    observability::{self, InvocationObservation},
    request::RigRequestMapper,
    response::{
        self, NoopStreamingResponseNormalizer, RigResponseNormalizer,
        RigStreamingResponseNormalizer,
    },
    stream,
};

pub struct RigBridge<M>
where
    M: CompletionModel,
{
    model: M,
    model_name: String,
    capabilities: BridgeCapabilities,
    defaults: GenerationOptions,
    request_mapper: Arc<dyn RigRequestMapper>,
    normalizer: Arc<dyn RigResponseNormalizer<M::Response>>,
    streaming_normalizer: Arc<dyn RigStreamingResponseNormalizer<M::StreamingResponse>>,
}

impl<M> RigBridge<M>
where
    M: CompletionModel,
{
    pub(crate) fn new(
        model: M,
        model_name: impl Into<String>,
        capabilities: BridgeCapabilities,
        defaults: GenerationOptions,
        request_mapper: Arc<dyn RigRequestMapper>,
        normalizer: Arc<dyn RigResponseNormalizer<M::Response>>,
    ) -> Result<Self, BridgeError> {
        Self::new_with_streaming_normalizer(
            model,
            model_name,
            capabilities,
            defaults,
            request_mapper,
            normalizer,
            Arc::new(NoopStreamingResponseNormalizer),
        )
    }

    pub(crate) fn new_with_streaming_normalizer(
        model: M,
        model_name: impl Into<String>,
        capabilities: BridgeCapabilities,
        defaults: GenerationOptions,
        request_mapper: Arc<dyn RigRequestMapper>,
        normalizer: Arc<dyn RigResponseNormalizer<M::Response>>,
        streaming_normalizer: Arc<dyn RigStreamingResponseNormalizer<M::StreamingResponse>>,
    ) -> Result<Self, BridgeError> {
        capabilities.validate()?;
        Ok(Self {
            model,
            model_name: model_name.into(),
            capabilities,
            defaults,
            request_mapper,
            normalizer,
            streaming_normalizer,
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

    fn project(&self, request: &CompletionRequest) -> Result<ProjectionReport, BridgeError> {
        self.capabilities.validate_request(request)?;
        self.request_mapper
            .map_request(request.clone(), &self.defaults)
            .map(|projection| projection.report)
    }

    fn complete<'a>(
        &'a self,
        request: CompletionRequest,
    ) -> BoxFuture<'a, Result<CompletionResponse, BridgeError>> {
        Box::pin(async move {
            let mut observation = InvocationObservation::new(
                self.normalizer.provider(),
                &self.model_name,
                false,
                request.tools.len(),
            );
            let result = async {
                self.capabilities.validate_request(&request)?;
                let projection = self.request_mapper.map_request(request, &self.defaults)?;
                observability::record_projection(&projection.report);
                let request = projection.request;
                let response = self
                    .model
                    .completion(request)
                    .await
                    .map_err(|error| self.normalizer.normalize_error(error))?;
                response::response_from_rig(response, self.normalizer.as_ref())
            }
            .await;
            observation.finish_completion(&result);
            result
        })
    }

    fn stream<'a>(
        &'a self,
        request: CompletionRequest,
    ) -> BoxFuture<'a, Result<CompletionStream, BridgeError>> {
        Box::pin(async move {
            let mut observation = InvocationObservation::new(
                self.normalizer.provider(),
                &self.model_name,
                true,
                request.tools.len(),
            );
            let result = async {
                self.capabilities.validate_streaming_request(&request)?;
                let projection = self.request_mapper.map_request(request, &self.defaults)?;
                observability::record_projection(&projection.report);
                let request = projection.request;
                let response = self
                    .model
                    .stream(request)
                    .await
                    .map_err(|error| self.normalizer.normalize_error(error))?;
                Ok(stream::completion_stream_with_normalizer(
                    response,
                    self.normalizer.provider(),
                    self.streaming_normalizer.clone(),
                ))
            }
            .await;
            match result {
                Ok(stream) => Ok(observability::observe_stream(stream, observation)),
                Err(error) => {
                    observation.finish_error(&error);
                    Err(error)
                }
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use armillae_core::{
        AssistantContent, CompletionRequest, ContentPart, FinishReason, GenerationOptions, Message,
        ProviderData, TextContent, TokenUsage,
    };
    use armillae_llm::{
        BridgeCapabilities, BridgeError, CompatibilityAction, LlmBridge, OutputFormatCapabilities,
        ToolChoiceCapabilities,
        mock::contract::{verify_completion, verify_stream},
    };
    use futures::stream;
    use rig_core::{
        OneOrMany,
        completion::{
            CompletionError, CompletionModel, CompletionRequest as RigCompletionRequest,
            CompletionResponse as RigCompletionResponse, GetTokenUsage, Usage,
        },
        streaming::{RawStreamingChoice, StreamingCompletionResponse, StreamingResult},
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
            request: RigCompletionRequest,
        ) -> Result<StreamingCompletionResponse<Self::StreamingResponse>, CompletionError> {
            self.requests
                .lock()
                .expect("the probe request lock must not be poisoned")
                .push(request);
            let inner: StreamingResult<ProbeResponse> = Box::pin(stream::iter([
                Ok(RawStreamingChoice::Message("hello".to_owned())),
                Ok(RawStreamingChoice::FinalResponse(ProbeResponse)),
            ]));
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
            streaming: true,
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
            "probe-model",
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
    fn projection_reports_foreign_data_without_invoking_or_mutating_request() {
        let model = ProbeModel::default();
        let bridge = bridge(model.clone());
        let request = CompletionRequest {
            messages: vec![Message::assistant(vec![
                ContentPart::text("visible"),
                ContentPart::ProviderData(ProviderData {
                    provider: "deepseek".to_owned(),
                    kind: "reasoning".to_owned(),
                    value: json!({ "opaque": true }),
                }),
            ])],
            ..CompletionRequest::default()
        };
        let original = request.clone();

        let report = bridge
            .project(&request)
            .expect("foreign ProviderData must not block projection");

        assert_eq!(request, original);
        assert_eq!(report.target_provider, "openai");
        assert!(matches!(
            report.facts.as_slice(),
            [fact]
                if fact.source_provider == "deepseek"
                    && fact.target_provider == "openai"
                    && fact.kind == "reasoning"
                    && fact.action == CompatibilityAction::NotForwarded
        ));
        assert!(
            model
                .requests
                .lock()
                .expect("the probe request lock must not be poisoned")
                .is_empty()
        );
    }

    #[test]
    fn p5_bridge_maps_and_executes_one_streaming_model_call() {
        futures::executor::block_on(async {
            let model = ProbeModel::default();
            let bridge = bridge(model.clone());
            let request = CompletionRequest {
                messages: vec![Message::user("hello")],
                ..CompletionRequest::default()
            };
            let expected = armillae_core::CompletionResponse {
                id: None,
                model: None,
                content: vec![AssistantContent::Text(TextContent::new("hello"))],
                finish_reason: None,
                usage: None,
                provider_metadata: json!({}),
            };

            verify_stream(&bridge, request, &expected)
                .await
                .expect("RigBridge must satisfy the shared streaming contract");

            let requests = model
                .requests
                .lock()
                .expect("the probe request lock must not be poisoned");
            assert_eq!(requests.len(), 1);
            assert_eq!(requests[0].temperature, Some(0.25));
        });
    }
}
