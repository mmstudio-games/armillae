use std::sync::Arc;

use crate::driver::RigDriver;
use armillae_core::{CompletionRequest, CompletionResponse, GenerationOptions};
use armillae_llm::{
    BoxFuture, BridgeCapabilities, BridgeError, CompletionStream, LlmBridge, OutputValidation,
    ProjectionReport,
};

use crate::{
    observability::{self, InvocationObservation},
    request::RigRequestMapper,
    response::{self, RigResponseNormalizer},
    stream,
};

// Construction is factory-only; the sealed driver never crosses the public boundary.
#[allow(private_bounds)]
pub struct RigBridge<M>
where
    M: RigDriver,
{
    model: M,
    model_name: String,
    capabilities: BridgeCapabilities,
    defaults: GenerationOptions,
    request_mapper: Arc<dyn RigRequestMapper>,
    normalizer: Arc<dyn RigResponseNormalizer<M::Response>>,
}

#[allow(private_bounds)]
impl<M> RigBridge<M>
where
    M: RigDriver,
{
    pub(crate) fn new(
        model: M,
        model_name: impl Into<String>,
        capabilities: BridgeCapabilities,
        defaults: GenerationOptions,
        request_mapper: Arc<dyn RigRequestMapper>,
        normalizer: Arc<dyn RigResponseNormalizer<M::Response>>,
    ) -> Result<Self, BridgeError> {
        let model_name = model_name.into();
        let mut capabilities = capabilities;
        capabilities.reasoning = crate::reasoning::capabilities(normalizer.provider(), &model_name);
        capabilities.validate()?;
        crate::reasoning::parameters(normalizer.provider(), &model_name, &defaults, None).map_err(
            |error| BridgeError::InvalidConfiguration {
                message: error.to_string(),
            },
        )?;
        Ok(Self {
            model,
            model_name,
            capabilities,
            defaults,
            request_mapper,
            normalizer,
        })
    }
    fn prepare(
        &self,
        mut request: CompletionRequest,
        streaming: bool,
    ) -> Result<crate::request::RigRequestProjection, BridgeError> {
        request.generation =
            crate::convert::merge_generation_options(&self.defaults, request.generation);
        if streaming {
            self.capabilities.validate_streaming_request(&request)?;
        } else {
            self.capabilities.validate_request(&request)?;
        }
        let controls = crate::reasoning::parameters(
            self.normalizer.provider(),
            &self.model_name,
            &request.generation,
            request.tool_choice.as_ref(),
        )?;
        let mut projection = self
            .request_mapper
            .map_request(request, &GenerationOptions::default())?;
        crate::reasoning::apply(&mut projection.request, controls)?;
        Ok(projection)
    }
}

impl<M> LlmBridge for RigBridge<M>
where
    M: RigDriver + Send + Sync + 'static,
    M::Response: Send + Sync,
{
    fn capabilities(&self) -> BridgeCapabilities {
        self.capabilities
    }

    fn project(&self, request: &CompletionRequest) -> Result<ProjectionReport, BridgeError> {
        OutputValidation::prepare(request.output_format.as_ref())?;
        self.prepare(request.clone(), false)
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
                let validation = OutputValidation::prepare(request.output_format.as_ref())?;
                let projection = self.prepare(request, false)?;
                observability::record_projection(&projection.report);
                let request = projection.request;
                let response = self
                    .model
                    .completion(request)
                    .await
                    .map_err(|error| self.normalizer.normalize_error(error))?;
                let response = response::response_from_rig(response, self.normalizer.as_ref())?;
                validation.validate(&response)?;
                Ok(response)
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
                let validation = OutputValidation::prepare(request.output_format.as_ref())?;
                let projection = self.prepare(request, true)?;
                observability::record_projection(&projection.report);
                let request = projection.request;
                let response = self
                    .model
                    .stream(request)
                    .await
                    .map_err(|error| self.normalizer.normalize_error(error))?;
                let stream = stream::completion_stream(response, self.normalizer.provider());
                Ok(validation.stream(stream, self.normalizer.provider()))
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
        completion::{
            CompletionError, CompletionRequest as RigCompletionRequest,
            CompletionResponse as RigCompletionResponse, Usage,
        },
        streaming::RawStreamingChoice,
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

    impl crate::driver::NativeResponse for ProbeResponse {
        fn normalize_native(
            self,
            provider: &str,
        ) -> Result<RigCompletionResponse, CompletionError> {
            Ok(RigCompletionResponse::new(
                vec![rig_core::message::AssistantContent::text("hello")],
                Usage {
                    input_tokens: 3,
                    output_tokens: 2,
                    total_tokens: 5,
                    ..Usage::default()
                },
                provider,
            ))
        }
    }
    #[derive(Clone, Default)]
    struct ProbeModel {
        requests: Arc<Mutex<Vec<RigCompletionRequest>>>,
    }
    impl crate::driver::RigDriver for ProbeModel {
        type Response = ProbeResponse;
        async fn completion(
            &self,
            request: RigCompletionRequest,
        ) -> Result<ProbeResponse, CompletionError> {
            self.requests.lock().expect("probe lock").push(request);
            Ok(ProbeResponse)
        }
        async fn stream(
            &self,
            request: RigCompletionRequest,
        ) -> Result<rig_core::streaming::RawStreamingResult<crate::driver::Terminal>, CompletionError>
        {
            self.requests.lock().expect("probe lock").push(request);
            Ok(Box::pin(stream::iter([
                Ok(RawStreamingChoice::Message("hello".to_owned())),
                Ok(RawStreamingChoice::FinalResponse(
                    crate::driver::Terminal::default(),
                )),
            ])))
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
            reasoning: armillae_llm::ReasoningCapabilities::NONE,
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
