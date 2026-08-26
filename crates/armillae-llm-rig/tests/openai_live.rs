use std::{env, error::Error, io};

use armillae_core::{
    AssistantContent, CompletionRequest, ContentPart, Message, OutputFormat, Role, ToolChoice,
    ToolDefinition, ToolResult, ToolResultContent,
};
use armillae_llm::{
    BridgeConfig, BridgeError, BridgeFactory, CredentialRef, LlmBridge,
    mock::contract::validate_stream_events,
};
use armillae_llm_rig::RigBridgeFactory;
use futures::StreamExt;
use serde_json::json;

const LIVE_PROVIDER_ENV: &str = "ARMILLAE_LIVE_PROVIDER";
const LIVE_MODEL_ENV: &str = "ARMILLAE_LIVE_MODEL";
const LIVE_ENDPOINT_ENV: &str = "ARMILLAE_LIVE_ENDPOINT";

struct LiveTarget {
    provider: String,
    model: String,
    credential_env: String,
    endpoint: Option<String>,
}

impl LiveTarget {
    fn from_environment() -> Result<Self, Box<dyn Error>> {
        let provider = env::var(LIVE_PROVIDER_ENV).map_err(|_| {
            io::Error::other(format!(
                "{LIVE_PROVIDER_ENV} must select one frozen Live Provider"
            ))
        })?;
        let (default_model, credential_env) = match provider.as_str() {
            "openai" => ("gpt-4.1-mini", "OPENAI_API_KEY"),
            "deepseek" => ("deepseek-v4-flash", "DEEPSEEK_API_KEY"),
            "minimax" => ("MiniMax-M2", "MINIMAX_API_KEY"),
            "moonshot" => ("kimi-k2", "MOONSHOT_API_KEY"),
            _ => {
                return Err(io::Error::other(
                    "Live Provider must be openai, deepseek, minimax, or moonshot",
                )
                .into());
            }
        };

        Ok(Self {
            provider,
            model: env::var(LIVE_MODEL_ENV).unwrap_or_else(|_| default_model.to_owned()),
            credential_env: credential_env.to_owned(),
            endpoint: env::var(LIVE_ENDPOINT_ENV).ok(),
        })
    }

    fn config(&self) -> Result<BridgeConfig, Box<dyn Error>> {
        let mut builder = BridgeConfig::builder(&self.provider, &self.model).credential(
            CredentialRef::Environment {
                name: self.credential_env.clone(),
            },
        );
        if let Some(endpoint) = &self.endpoint {
            builder = builder.endpoint(endpoint.parse()?);
        }
        Ok(builder.build()?)
    }

    async fn bridge(&self) -> Result<std::sync::Arc<dyn LlmBridge>, Box<dyn Error>> {
        let resolved = self.config()?.resolve().await?;
        Ok(RigBridgeFactory.create(resolved).await?)
    }
}

fn response_text(response: &armillae_core::CompletionResponse) -> String {
    response
        .content
        .iter()
        .filter_map(|content| match content {
            AssistantContent::Text(text) => Some(text.text.as_str()),
            _ => None,
        })
        .collect()
}

fn lookup_tool() -> ToolDefinition {
    ToolDefinition {
        name: "lookup".to_owned(),
        description: "Return the exact value associated with a key".to_owned(),
        input_schema: json!({
            "type": "object",
            "properties": { "key": { "type": "string" } },
            "required": ["key"],
            "additionalProperties": false
        }),
    }
}

fn tool_choice_for_single(bridge: &dyn LlmBridge) -> Option<ToolChoice> {
    let capabilities = bridge.capabilities().tool_choice;
    if capabilities.specific {
        Some(ToolChoice::Specific {
            name: "lookup".to_owned(),
        })
    } else if capabilities.required {
        Some(ToolChoice::Required)
    } else if capabilities.auto {
        Some(ToolChoice::Auto)
    } else {
        None
    }
}

fn tool_choice_for_multiple(bridge: &dyn LlmBridge) -> Option<ToolChoice> {
    let capabilities = bridge.capabilities().tool_choice;
    if capabilities.required {
        Some(ToolChoice::Required)
    } else if capabilities.auto {
        Some(ToolChoice::Auto)
    } else {
        None
    }
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires an explicitly selected Provider and real credential"]
async fn live_text_stream_system_history_usage_and_response_facts() -> Result<(), Box<dyn Error>> {
    let target = LiveTarget::from_environment()?;
    let bridge = target.bridge().await?;
    let response = bridge
        .complete(CompletionRequest {
            messages: vec![
                Message::new(Role::System, vec![ContentPart::text("Answer concisely.")]),
                Message::user("Remember the word amber."),
                Message::assistant(vec![ContentPart::text("I will remember amber.")]),
                Message::user("Which word did I ask you to remember?"),
            ],
            ..CompletionRequest::default()
        })
        .await?;
    assert!(!response_text(&response).trim().is_empty());
    assert!(response.id.as_deref().is_some_and(|id| !id.is_empty()));
    assert!(
        response
            .model
            .as_deref()
            .is_some_and(|model| !model.is_empty())
    );
    assert!(response.finish_reason.is_some());
    assert!(
        response
            .usage
            .as_ref()
            .is_some_and(|usage| { usage.total_tokens.is_some_and(|tokens| tokens > 0) })
    );

    let mut stream = bridge
        .stream(CompletionRequest {
            messages: vec![Message::user("Reply with one short greeting.")],
            ..CompletionRequest::default()
        })
        .await?;
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event?);
    }
    let streamed = validate_stream_events(&events)?;
    assert!(!response_text(streamed).trim().is_empty());
    assert!(
        streamed
            .usage
            .as_ref()
            .is_some_and(|usage| { usage.total_tokens.is_some_and(|tokens| tokens > 0) })
    );
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires an explicitly selected Provider and real credential"]
async fn live_supported_structured_output_is_valid_json() -> Result<(), Box<dyn Error>> {
    let target = LiveTarget::from_environment()?;
    let bridge = target.bridge().await?;
    let output_format = if bridge.capabilities().output_format.json_schema {
        OutputFormat::JsonSchema {
            name: "answer".to_owned(),
            schema: json!({
                "type": "object",
                "properties": { "answer": { "type": "string" } },
                "required": ["answer"],
                "additionalProperties": false
            }),
            strict: true,
        }
    } else {
        OutputFormat::JsonObject
    };
    let response = bridge
        .complete(CompletionRequest {
            messages: vec![Message::user(
                "Return a JSON object whose answer field is the string armillae.",
            )],
            output_format: Some(output_format),
            ..CompletionRequest::default()
        })
        .await?;
    let value: serde_json::Value = serde_json::from_str(&response_text(&response))?;
    assert_eq!(value["answer"], "armillae");
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires an explicitly selected Provider and real credential"]
async fn live_single_tool_and_manual_tool_result_round_trip() -> Result<(), Box<dyn Error>> {
    let target = LiveTarget::from_environment()?;
    let bridge = target.bridge().await?;
    let user = Message::user("Call lookup once with key armillae. Do not invent its result.");
    let first = bridge
        .complete(CompletionRequest {
            messages: vec![user.clone()],
            tools: vec![lookup_tool()],
            tool_choice: tool_choice_for_single(bridge.as_ref()),
            ..CompletionRequest::default()
        })
        .await?;
    let calls = first.tool_calls().cloned().collect::<Vec<_>>();
    assert_eq!(calls.len(), 1);

    let mut history = vec![user, first.as_assistant_message()];
    history.push(Message::tool_result(ToolResult {
        call_id: calls[0].id.clone(),
        content: vec![ToolResultContent::Json {
            value: json!({ "value": "verified-result" }),
        }],
        is_error: false,
    }));
    let second = bridge
        .complete(CompletionRequest {
            messages: history,
            tools: vec![lookup_tool()],
            tool_choice: bridge
                .capabilities()
                .tool_choice
                .none
                .then_some(ToolChoice::None),
            ..CompletionRequest::default()
        })
        .await?;
    assert!(!response_text(&second).trim().is_empty());
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires an explicitly selected Provider and real credential"]
async fn live_streaming_multiple_tool_calls_reassemble() -> Result<(), Box<dyn Error>> {
    let target = LiveTarget::from_environment()?;
    let bridge = target.bridge().await?;
    let mut stream = bridge
        .stream(CompletionRequest {
            messages: vec![Message::user(
                "Call lookup twice in parallel: once with key alpha and once with key beta.",
            )],
            tools: vec![lookup_tool()],
            tool_choice: tool_choice_for_multiple(bridge.as_ref()),
            ..CompletionRequest::default()
        })
        .await?;
    let mut events = Vec::new();
    while let Some(event) = stream.next().await {
        events.push(event?);
    }
    let response = validate_stream_events(&events)?;
    let calls = response.tool_calls().collect::<Vec<_>>();
    assert_eq!(calls.len(), 2);
    assert_ne!(calls[0].id, calls[1].id);
    assert!(calls.iter().all(|call| call.name == "lookup"));
    Ok(())
}

#[tokio::test(flavor = "current_thread")]
#[ignore = "requires an explicitly selected Provider and real credential"]
async fn live_local_preflight_and_remote_rejection_are_classified() -> Result<(), Box<dyn Error>> {
    let target = LiveTarget::from_environment()?;
    let bridge = target.bridge().await?;
    let local = bridge
        .complete(CompletionRequest {
            messages: vec![Message::new(
                Role::Developer,
                vec![ContentPart::text("unsupported")],
            )],
            ..CompletionRequest::default()
        })
        .await
        .expect_err("Developer role must fail locally for the frozen Provider matrix");
    assert!(matches!(local, BridgeError::UnsupportedCapability { .. }));

    let invalid_config =
        BridgeConfig::builder(&target.provider, "armillae-intentionally-invalid-model")
            .credential(CredentialRef::Environment {
                name: target.credential_env,
            })
            .build()?;
    let invalid_bridge = RigBridgeFactory
        .create(invalid_config.resolve().await?)
        .await?;
    let remote = invalid_bridge
        .complete(CompletionRequest {
            messages: vec![Message::user("hello")],
            ..CompletionRequest::default()
        })
        .await
        .expect_err("an intentionally invalid model must be rejected remotely");
    assert!(matches!(remote, BridgeError::ProviderRejected { .. }));
    Ok(())
}
