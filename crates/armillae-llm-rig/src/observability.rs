use std::time::Instant;

use armillae_core::{AssistantContent, CompletionEvent, CompletionResponse, TokenUsage};
use armillae_llm::{
    BridgeError, CompatibilityAction, CompletionStream, ErrorMetadata, ProjectionReport,
};
use futures_util::{StreamExt, stream};
use tracing::{Span, field};

pub(crate) fn record_projection(report: &ProjectionReport) {
    for fact in &report.facts {
        let action = match fact.action {
            CompatibilityAction::NotForwarded => "not_forwarded",
            _ => "unknown",
        };
        tracing::info!(
            target: "armillae::llm",
            source_provider = %fact.source_provider,
            target_provider = %fact.target_provider,
            provider_data_kind = %fact.kind,
            compatibility_action = action,
            lossy = fact.lossy,
            message_index = fact.location.message_index,
            content_index = fact.location.content_index,
            "LLM request compatibility action"
        );
    }
}

pub(crate) struct InvocationObservation {
    span: Span,
    started: Instant,
    first_output_recorded: bool,
    finished: bool,
}

impl InvocationObservation {
    pub(crate) fn new(
        provider: &str,
        model: &str,
        streaming: bool,
        tool_definition_count: usize,
    ) -> Self {
        let span = tracing::info_span!(
            target: "armillae::llm",
            "llm.bridge.call",
            adapter = "rig",
            provider,
            model,
            request_id = field::Empty,
            streaming,
            tool_definition_count,
            tool_call_count = field::Empty,
            input_tokens = field::Empty,
            output_tokens = field::Empty,
            total_tokens = field::Empty,
            cached_input_tokens = field::Empty,
            total_latency_ms = field::Empty,
            first_token_latency_ms = field::Empty,
            error_category = field::Empty,
        );
        Self {
            span,
            started: Instant::now(),
            first_output_recorded: false,
            finished: false,
        }
    }

    pub(crate) fn finish_completion(&mut self, result: &Result<CompletionResponse, BridgeError>) {
        match result {
            Ok(response) => self.finish_success(response),
            Err(error) => self.finish_error(error),
        }
    }

    fn observe_stream_event(&mut self, event: &CompletionEvent) {
        if !self.first_output_recorded
            && matches!(
                event,
                CompletionEvent::TextDelta { .. }
                    | CompletionEvent::ToolCallStarted { .. }
                    | CompletionEvent::ProviderEvent { .. }
            )
        {
            self.first_output_recorded = true;
            self.span
                .record("first_token_latency_ms", elapsed_millis(self.started));
        }

        match event {
            CompletionEvent::ResponseStarted {
                id: Some(request_id),
                ..
            } => {
                self.span.record("request_id", request_id.as_str());
            }
            CompletionEvent::ResponseCompleted { response } => self.finish_success(response),
            _ => {}
        }
    }

    fn finish_success(&mut self, response: &CompletionResponse) {
        if self.finished {
            return;
        }
        if let Some(request_id) = &response.id {
            self.span.record("request_id", request_id.as_str());
        }
        self.span.record(
            "tool_call_count",
            response
                .content
                .iter()
                .filter(|content| matches!(content, AssistantContent::ToolCall(_)))
                .count(),
        );
        record_usage(&self.span, response.usage.as_ref());
        let total_latency_ms = elapsed_millis(self.started);
        self.span.record("total_latency_ms", total_latency_ms);
        tracing::info!(
            target: "armillae::llm",
            parent: &self.span,
            status = "ok",
            total_latency_ms,
            "LLM Bridge call completed"
        );
        self.finished = true;
    }

    pub(crate) fn finish_error(&mut self, error: &BridgeError) {
        if self.finished {
            return;
        }
        if let Some(request_id) =
            error_metadata(error).and_then(|metadata| metadata.request_id.as_ref())
        {
            self.span.record("request_id", request_id.as_str());
        }
        let category = error_category(error);
        let total_latency_ms = elapsed_millis(self.started);
        self.span.record("error_category", category);
        self.span.record("total_latency_ms", total_latency_ms);
        tracing::warn!(
            target: "armillae::llm",
            parent: &self.span,
            status = "error",
            error_category = category,
            total_latency_ms,
            "LLM Bridge call failed"
        );
        self.finished = true;
    }

    fn finish_cancelled(&mut self) {
        if self.finished {
            return;
        }
        let total_latency_ms = elapsed_millis(self.started);
        self.span.record("error_category", "cancelled");
        self.span.record("total_latency_ms", total_latency_ms);
        tracing::info!(
            target: "armillae::llm",
            parent: &self.span,
            status = "cancelled",
            error_category = "cancelled",
            total_latency_ms,
            "LLM Bridge call cancelled"
        );
        self.finished = true;
    }
}

impl Drop for InvocationObservation {
    fn drop(&mut self) {
        self.finish_cancelled();
    }
}

pub(crate) fn observe_stream(
    inner: CompletionStream,
    observation: InvocationObservation,
) -> CompletionStream {
    Box::pin(stream::unfold(
        (inner, observation),
        |(mut inner, mut observation)| async move {
            let item = inner.next().await;
            match &item {
                Some(Ok(event)) => observation.observe_stream_event(event),
                Some(Err(error)) => observation.finish_error(error),
                None => observation.finish_cancelled(),
            }
            item.map(|item| (item, (inner, observation)))
        },
    ))
}

fn record_usage(span: &Span, usage: Option<&TokenUsage>) {
    let Some(usage) = usage else {
        return;
    };
    if let Some(value) = usage.input_tokens {
        span.record("input_tokens", value);
    }
    if let Some(value) = usage.output_tokens {
        span.record("output_tokens", value);
    }
    if let Some(value) = usage.total_tokens {
        span.record("total_tokens", value);
    }
    if let Some(value) = usage.cached_input_tokens {
        span.record("cached_input_tokens", value);
    }
}

fn error_category(error: &BridgeError) -> &'static str {
    match error {
        BridgeError::InvalidConfiguration { .. } => "invalid_configuration",
        BridgeError::UnsupportedCapability { .. } => "unsupported_capability",
        BridgeError::InvalidRequest { .. } => "invalid_request",
        BridgeError::ProjectionIncompatible { .. } => "projection_incompatible",
        BridgeError::Authentication { .. } => "authentication",
        BridgeError::PermissionDenied { .. } => "permission_denied",
        BridgeError::RateLimited { .. } => "rate_limited",
        BridgeError::Timeout { .. } => "timeout",
        BridgeError::Cancelled => "cancelled",
        BridgeError::Transport { .. } => "transport",
        BridgeError::ProviderRejected { .. } => "provider_rejected",
        BridgeError::InvalidProviderResponse { .. } => "invalid_provider_response",
        BridgeError::StreamInterrupted { .. } => "stream_interrupted",
        _ => "unknown",
    }
}

fn error_metadata(error: &BridgeError) -> Option<&ErrorMetadata> {
    match error {
        BridgeError::Authentication { metadata }
        | BridgeError::PermissionDenied { metadata }
        | BridgeError::Timeout { metadata }
        | BridgeError::Transport { metadata, .. }
        | BridgeError::ProviderRejected { metadata, .. }
        | BridgeError::InvalidProviderResponse { metadata, .. }
        | BridgeError::StreamInterrupted { metadata }
        | BridgeError::RateLimited { metadata, .. } => Some(metadata),
        BridgeError::InvalidConfiguration { .. }
        | BridgeError::UnsupportedCapability { .. }
        | BridgeError::InvalidRequest { .. }
        | BridgeError::ProjectionIncompatible { .. }
        | BridgeError::Cancelled => None,
        _ => None,
    }
}

fn elapsed_millis(started: Instant) -> u64 {
    u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::{
        io::{self, Write},
        sync::{Arc, Mutex},
    };

    use armillae_core::{
        AssistantContent, CompletionEvent, CompletionResponse, ProviderData, TextContent,
        TokenUsage,
    };
    use armillae_llm::{
        BridgeError, CompatibilityAction, CompatibilityFact, ErrorMetadata, MessageContentLocation,
        ProjectionReport,
    };
    use futures::{StreamExt, executor::block_on, stream};
    use serde_json::json;
    use tracing_subscriber::fmt::format::FmtSpan;

    use super::{InvocationObservation, observe_stream, record_projection};

    #[derive(Clone, Default)]
    struct SharedWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    struct BufferWriter {
        bytes: Arc<Mutex<Vec<u8>>>,
    }

    impl<'writer> tracing_subscriber::fmt::MakeWriter<'writer> for SharedWriter {
        type Writer = BufferWriter;

        fn make_writer(&'writer self) -> Self::Writer {
            BufferWriter {
                bytes: self.bytes.clone(),
            }
        }
    }

    impl Write for BufferWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.bytes
                .lock()
                .map_err(|_| io::Error::other("trace buffer lock poisoned"))?
                .extend_from_slice(buffer);
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    impl SharedWriter {
        fn output(&self) -> String {
            let bytes = self
                .bytes
                .lock()
                .expect("trace buffer lock must not be poisoned")
                .clone();
            String::from_utf8(bytes).expect("tracing output must be UTF-8")
        }
    }

    #[test]
    fn structured_facts_are_recorded_without_content_or_error_bodies() {
        let writer = SharedWriter::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(writer.clone())
            .with_ansi(false)
            .without_time()
            .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            record_projection(&ProjectionReport {
                target_provider: "anthropic".to_owned(),
                facts: vec![CompatibilityFact {
                    location: MessageContentLocation {
                        message_index: 1,
                        content_index: 2,
                    },
                    source_provider: "deepseek".to_owned(),
                    target_provider: "anthropic".to_owned(),
                    kind: "reasoning".to_owned(),
                    action: CompatibilityAction::NotForwarded,
                    lossy: true,
                }],
            });
            let mut successful = InvocationObservation::new("openai", "gpt-test", false, 2);
            successful.finish_completion(&Ok(CompletionResponse {
                id: Some("request-1".to_owned()),
                model: Some("provider-model".to_owned()),
                content: vec![
                    AssistantContent::Text(TextContent::new("response-secret-marker")),
                    AssistantContent::ProviderData(ProviderData {
                        provider: "openai".to_owned(),
                        kind: "secret".to_owned(),
                        value: json!({ "token": "provider-data-secret-marker" }),
                    }),
                ],
                finish_reason: None,
                usage: Some(TokenUsage {
                    input_tokens: Some(3),
                    output_tokens: Some(2),
                    total_tokens: Some(5),
                    cached_input_tokens: Some(1),
                }),
                provider_metadata: json!({ "raw": "metadata-secret-marker" }),
            }));

            let mut failed = InvocationObservation::new("openai", "gpt-test", false, 0);
            failed.finish_completion(&Err(BridgeError::ProviderRejected {
                code: None,
                message: "provider-error-secret-marker".to_owned(),
                metadata: ErrorMetadata::new("openai").with_http_status(400),
            }));
        });

        let output = writer.output();
        assert!(output.contains("llm.bridge.call"));
        assert!(output.contains("adapter=\"rig\""));
        assert!(output.contains("provider=\"openai\""));
        assert!(output.contains("model=\"gpt-test\""));
        assert!(output.contains("tool_definition_count=2"));
        assert!(output.contains("input_tokens=3"));
        assert!(output.contains("error_category=\"provider_rejected\""));
        assert!(output.contains("compatibility_action=\"not_forwarded\""));
        assert!(output.contains("source_provider=deepseek"));
        assert!(output.contains("target_provider=anthropic"));
        for secret in [
            "response-secret-marker",
            "provider-data-secret-marker",
            "metadata-secret-marker",
            "provider-error-secret-marker",
        ] {
            assert!(!output.contains(secret));
        }
    }

    #[test]
    fn streaming_records_first_output_and_drop_cancellation() {
        let writer = SharedWriter::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(writer.clone())
            .with_ansi(false)
            .without_time()
            .with_span_events(FmtSpan::NEW | FmtSpan::CLOSE)
            .finish();

        tracing::subscriber::with_default(subscriber, || {
            block_on(async {
                let inner = Box::pin(stream::iter([Ok(CompletionEvent::TextDelta {
                    index: 0,
                    text: "stream-secret-marker".to_owned(),
                })]));
                let mut observed = observe_stream(
                    inner,
                    InvocationObservation::new("ollama", "qwen-test", true, 0),
                );
                observed
                    .next()
                    .await
                    .expect("one stream item must exist")
                    .expect("fixture stream item must succeed");
                drop(observed);
            });
        });

        let output = writer.output();
        assert!(output.contains("first_token_latency_ms"));
        assert!(output.contains("status=\"cancelled\""));
        assert!(!output.contains("stream-secret-marker"));
    }
}
